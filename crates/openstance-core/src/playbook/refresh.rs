//! `refresh` (P4-CORE-04): fill empty titles and companies of people you
//! talk to, in score order, until the budget runs out. Spec:
//! `docs/tech-spec/playbooks.md` section 4.
//!
//! | # | step | egress |
//! |---|---|---|
//! | 1 | select: empty title or company, `>= min_interactions`, not bulk, not `no_enrich`, no override on those fields | - |
//! | 2 | quote one `profile` each | - |
//! | 3 | enrich in score order until the budget is used up | `limit` `profile` |
//! | 4 | commit under `source.kind = 'enrich:<backend>'` (overrides still win) | - |

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{
    egress_status, egress_step, local_step, parse_input, quote_notes, EgressMax, Outcome, Plan,
    Playbook, RunCtx, StepDecision,
};
use crate::egress::{EgressGate, Executed};
use crate::enrich::{EnrichData, EnrichJob, EnrichRequest};
use crate::error::Result;
use crate::json::V;
use crate::{Graph, PersonId};

/// Most people per run, whatever the input says.
pub const MAX_LIMIT: u32 = 1000;

const STEP_ENRICH: u8 = 3;

/// `refresh` input; missing fields come from `[playbooks.refresh]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshInput {
    /// Minimum interactions.
    #[serde(default)]
    pub min_interactions: Option<u32>,
    /// Most people per run.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Read by the surface ([`super::RunOptions`]).
    #[serde(default)]
    pub allow_egress: Option<bool>,
    /// Read by the surface.
    #[serde(default)]
    pub max_usd: Option<f64>,
    /// Read by the surface.
    #[serde(default)]
    pub plan_only: Option<bool>,
}

/// The `refresh` playbook.
#[derive(Debug, Clone, Copy, Default)]
pub struct Refresh;

/// One person to refresh.
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub id: PersonId,
    pub name: String,
    pub title: Option<String>,
    pub company: Option<String>,
    pub linkedin: Option<String>,
}

/// The `profile` job for a person: name, company and LinkedIn slug, the
/// only fields that leave the machine.
pub(crate) fn profile_job(c: &Candidate) -> EnrichJob {
    EnrichJob {
        person_id: Some(c.id),
        request: EnrichRequest::Profile {
            name: c.name.clone(),
            company: c.company.clone().filter(|s| !s.trim().is_empty()),
            linkedin: c.linkedin.clone(),
        },
    }
}

/// The first LinkedIn slug of a person.
pub(crate) fn linkedin_of(conn: &Connection, p: PersonId) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached(
            "SELECT value FROM identity WHERE person_id = ?1 AND kind = 'linkedin_url'
             ORDER BY COALESCE(confidence, 0) DESC, id LIMIT 1",
        )?
        .query_row([p], |r| r.get(0))
        .optional()?)
}

fn empty(s: Option<&str>) -> bool {
    s.is_none_or(|s| s.trim().is_empty())
}

const SELECT: &str = "SELECT p.id, p.display_name, p.title, p.company FROM person p
     WHERE p.is_self = 0 AND p.is_bulk = 0 AND p.no_enrich = ?3
       AND p.display_name IS NOT NULL AND TRIM(p.display_name) <> ''
       AND (COALESCE(TRIM(p.title), '') = '' OR COALESCE(TRIM(p.company), '') = '')
       AND p.n_interactions >= ?1
       AND NOT EXISTS (
         SELECT 1 FROM override o JOIN identity i ON i.id = o.person_id
         WHERE i.person_id = p.id AND o.field IN ('title', 'company'))
     ORDER BY p.score DESC, p.id
     LIMIT ?2";

fn candidates(conn: &Connection, min: u32, limit: u32) -> Result<(Vec<Candidate>, usize)> {
    let mut st = conn.prepare_cached(SELECT)?;
    let rows: Vec<(PersonId, String, Option<String>, Option<String>)> = st
        .query_map(rusqlite::params![min, limit, 0], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::new();
    for (id, name, title, company) in rows {
        out.push(Candidate {
            id,
            name,
            title,
            company,
            linkedin: linkedin_of(conn, id)?,
        });
    }
    let no_enrich = st
        .query_map(rusqlite::params![min, limit, 1], |r| r.get::<_, i64>(0))?
        .count();
    Ok((out, no_enrich))
}

/// One person `refresh` filled.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Updated {
    /// Person id.
    pub id: PersonId,
    /// The previously empty fields the profile filled.
    pub fields: Map<String, Value>,
    /// `enrich_log.id`.
    pub enrich_log_id: i64,
}

impl Refresh {
    fn limits(g: &Graph, inp: &RefreshInput) -> (u32, u32) {
        let cfg = &g.config().playbooks.refresh;
        (
            inp.min_interactions.unwrap_or(cfg.min_interactions),
            inp.limit.unwrap_or(cfg.limit).clamp(1, MAX_LIMIT),
        )
    }
}

impl Playbook for Refresh {
    fn name(&self) -> &'static str {
        "refresh"
    }

    fn egress_max(&self) -> Vec<EgressMax> {
        vec![EgressMax {
            op: "profile".into(),
            max: crate::config::RefreshPlaybook::default().limit,
        }]
    }

    fn plan(&self, g: &Graph, gate: &mut EgressGate<'_>, input: &Value) -> Result<Plan> {
        let inp: RefreshInput = parse_input("refresh", input)?;
        let (min, limit) = Self::limits(g, &inp);
        let (cands, _) = candidates(g.conn(), min, limit)?;
        let quote = gate.quote(cands.iter().map(profile_job).collect())?;
        let mut steps = vec![
            local_step(1, "select people missing title/company"),
            local_step(2, "quote one profile each"),
            egress_step(STEP_ENRICH, "enrich in score order", "profile", &quote),
            local_step(4, "commit as enrich source"),
        ];
        if cands.is_empty() {
            steps[2].will_run = StepDecision::SkipNoInput;
        }
        Ok(Plan {
            playbook: self.name().into(),
            run_id: gate.run_id().to_string(),
            steps,
            quote,
            egress_max: vec![EgressMax {
                op: "profile".into(),
                max: limit,
            }],
        })
    }

    fn run(
        &self,
        g: &Graph,
        gate: &mut EgressGate<'_>,
        input: &Value,
        ctx: &RunCtx,
    ) -> Result<Outcome> {
        let inp: RefreshInput = parse_input("refresh", input)?;
        let (min, limit) = Self::limits(g, &inp);
        let mut plan = self.plan(g, gate, input)?;
        let (cands, no_enrich) = candidates(g.conn(), min, limit)?;
        let mut notes = Vec::new();
        notes.push(format!(
            "{} {} no title/company and {min}+ interactions ({no_enrich} skipped: no_enrich)",
            cands.len(),
            if cands.len() == 1 {
                "person has"
            } else {
                "people have"
            },
        ));
        notes.extend(quote_notes(&plan, ctx.allow_egress));
        let mut updated = Vec::new();
        let mut unchanged = Vec::new();
        let mut not_found = Vec::new();
        let mut failed = Vec::new();
        let mut partial = false;
        let quoted = !plan.quote.lines.is_empty();
        if quoted && !ctx.allow_egress {
            plan.mark_egress(StepDecision::SkipNoEgress);
        } else if quoted {
            gate.approve(&plan.quote.clone())?;
            let jobs: Vec<EnrichJob> = plan.quote.lines.iter().map(|l| l.job.clone()).collect();
            for job in jobs {
                let Some(c) = cands.iter().find(|c| Some(c.id) == job.person_id) else {
                    continue;
                };
                match gate.execute(&job)? {
                    Executed::Done { log_id, result } => match result.data {
                        EnrichData::Profile(p) => {
                            let mut fields = Map::new();
                            if empty(c.title.as_deref()) && !empty(p.title.as_deref()) {
                                fields.insert("title".into(), json!(p.title));
                            }
                            if empty(c.company.as_deref()) && !empty(p.company.as_deref()) {
                                fields.insert("company".into(), json!(p.company));
                            }
                            if fields.is_empty() {
                                unchanged.push(c.id);
                            } else {
                                updated.push(Updated {
                                    id: c.id,
                                    fields,
                                    enrich_log_id: log_id,
                                });
                            }
                        }
                        _ => not_found.push(c.id),
                    },
                    Executed::Blocked { .. } => partial = true,
                    Executed::Failed { error, .. } => {
                        partial = true;
                        failed.push(json!({ "id": c.id, "error": error }));
                    }
                }
            }
            gate.commit()?;
        }
        let receipt = gate.receipt();
        let result = json!({
            "updated": updated,
            "unchanged": unchanged,
            "not_found": not_found,
            "failed": failed,
            "skipped": receipt.skipped,
        });
        let status = if cands.is_empty() {
            super::RunStatus::NothingToDo
        } else {
            egress_status(&plan, ctx.allow_egress, partial)
        };
        Ok(Outcome {
            v: V,
            playbook: self.name().into(),
            status,
            receipt,
            plan,
            result: Some(result),
            notes,
        })
    }
}
