//! `watch` (P4-CORE-03): job changes, from the LinkedIn exports you already
//! have and, if asked and paid for, from fresh profiles. Spec:
//! `docs/tech-spec/playbooks.md` section 5. There is no daemon: run it from
//! cron or launchd.
//!
//! | # | step | egress |
//! |---|---|---|
//! | 1 | diff the latest two `Connections.csv` exports: company and title changes, new and removed connections | - |
//! | 2 | `check_profiles`: one `profile` for each of the top `top_n` by score (not `no_enrich`, not already moved) | `top_n` `profile` |
//! | 3 | flag moves into a company on `targets` | - |

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::accounts::{export_rows, linkedin_exports, Export, ExportRow};
use super::refresh::{linkedin_of, profile_job, Candidate};
use super::{
    egress_status, egress_step, local_step, parse_input, quote_notes, CompanyTarget, EgressMax,
    Outcome, Plan, Playbook, RunCtx, StepDecision,
};
use crate::egress::{EgressGate, Executed};
use crate::enrich::EnrichData;
use crate::error::Result;
use crate::json::{iso, round3, V};
use crate::resolve::normalize::company_key;
use crate::{Graph, PersonId};

/// Most profile checks per run, whatever the input says.
pub const MAX_TOP_N: u32 = 1000;

const STEP_CHECK: u8 = 2;

/// `watch` input; missing fields come from `[playbooks.watch]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchInput {
    /// Companies (names or domains) whose incoming moves are highlighted.
    #[serde(default)]
    pub targets: Option<Vec<String>>,
    /// Add the paid profile checks (step 2).
    #[serde(default)]
    pub check_profiles: bool,
    /// Profiles checked in step 2.
    #[serde(default)]
    pub top_n: Option<u32>,
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

/// The `watch` playbook.
#[derive(Debug, Clone, Copy, Default)]
pub struct Watch;

/// A company change.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Move {
    /// Person id.
    pub id: PersonId,
    /// Display name.
    pub display_name: Option<String>,
    /// Old company.
    pub from: String,
    /// New company.
    pub to: String,
    /// Old title, if known.
    pub from_title: Option<String>,
    /// New title, if known.
    pub to_title: Option<String>,
    /// `linkedin_diff` or `enrich:<backend>`.
    pub detected_by: String,
    /// `[previous export, latest export]` import times (diff only).
    pub seen_between: Option<[String; 2]>,
    /// The new company is on the target list.
    pub target_hit: bool,
    /// Relationship score.
    pub score: f64,
    /// The `enrich_log` row (profile checks only).
    pub enrich_log_id: Option<i64>,
    /// The export rows the diff read (diff only).
    pub evidence: Vec<Value>,
}

/// A title change at the same company.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TitleChange {
    /// Person id.
    pub id: PersonId,
    /// Display name.
    pub display_name: Option<String>,
    /// Company.
    pub company: Option<String>,
    /// Old title.
    pub from: Option<String>,
    /// New title.
    pub to: Option<String>,
    /// The export rows the diff read.
    pub evidence: Vec<Value>,
}

/// A connection present in only one of the two exports.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConnectionChange {
    /// Person id.
    pub id: PersonId,
    /// Display name.
    pub display_name: Option<String>,
    /// Company in that export.
    pub company: Option<String>,
    /// Title in that export.
    pub title: Option<String>,
    /// The export row.
    pub interaction_id: i64,
}

fn person(conn: &Connection, id: PersonId) -> Result<(Option<String>, f64)> {
    Ok(conn
        .prepare_cached("SELECT display_name, score FROM person WHERE id = ?1")?
        .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))?)
}

fn evidence(e: &Export, r: &ExportRow) -> Value {
    json!({
        "kind": "linkedin_export",
        "source_id": e.id,
        "label": e.label,
        "imported_at": iso(e.imported_at),
        "interaction_id": r.interaction_id,
        "company": r.company,
        "title": r.title,
    })
}

fn nonempty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

/// Step 1 output.
#[derive(Default)]
struct Diff {
    exports: Option<(Export, Export)>,
    n_exports: usize,
    moves: Vec<Move>,
    titles: Vec<TitleChange>,
    added: Vec<ConnectionChange>,
    removed: Vec<ConnectionChange>,
}

fn diff(conn: &Connection) -> Result<Diff> {
    let exports = linkedin_exports(conn)?;
    let n = exports.len();
    let mut d = Diff {
        n_exports: n,
        ..Diff::default()
    };
    if n < 2 {
        return Ok(d);
    }
    let (prev, latest) = (exports[n - 2].clone(), exports[n - 1].clone());
    let a = export_rows(conn, prev.id)?;
    let b = export_rows(conn, latest.id)?;
    let between = [iso(prev.imported_at), iso(latest.imported_at)];
    for (id, rb) in &b {
        let (name, score) = person(conn, *id)?;
        let Some(ra) = a.get(id) else {
            d.added.push(ConnectionChange {
                id: *id,
                display_name: name,
                company: rb.company.clone(),
                title: rb.title.clone(),
                interaction_id: rb.interaction_id,
            });
            continue;
        };
        let ev = vec![evidence(&prev, ra), evidence(&latest, rb)];
        let (ca, cb) = (
            nonempty(ra.company.as_deref()),
            nonempty(rb.company.as_deref()),
        );
        let (ta, tb) = (nonempty(ra.title.as_deref()), nonempty(rb.title.as_deref()));
        match (ca, cb) {
            (Some(x), Some(y)) if company_key(x) != company_key(y) => d.moves.push(Move {
                id: *id,
                display_name: name,
                from: x.to_string(),
                to: y.to_string(),
                from_title: ta.map(str::to_string),
                to_title: tb.map(str::to_string),
                detected_by: "linkedin_diff".into(),
                seen_between: Some(between.clone()),
                target_hit: false,
                score: round3(score),
                enrich_log_id: None,
                evidence: ev,
            }),
            _ => {
                let norm = |t: Option<&str>| t.map(str::to_lowercase);
                if norm(ta) != norm(tb) {
                    d.titles.push(TitleChange {
                        id: *id,
                        display_name: name,
                        company: cb.or(ca).map(str::to_string),
                        from: ta.map(str::to_string),
                        to: tb.map(str::to_string),
                        evidence: ev,
                    });
                }
            }
        }
    }
    for (id, ra) in &a {
        if !b.contains_key(id) {
            let (name, _) = person(conn, *id)?;
            d.removed.push(ConnectionChange {
                id: *id,
                display_name: name,
                company: ra.company.clone(),
                title: ra.title.clone(),
                interaction_id: ra.interaction_id,
            });
        }
    }
    d.exports = Some((prev, latest));
    Ok(d)
}

/// Step 2 candidates: top `top_n` by score, not `no_enrich`, not moved.
fn check_candidates(conn: &Connection, top_n: u32, moved: &[PersonId]) -> Result<Vec<Candidate>> {
    let mut st = conn.prepare_cached(
        "SELECT id, display_name, title, company FROM person
         WHERE is_self = 0 AND is_bulk = 0 AND no_enrich = 0 AND score > 0
           AND display_name IS NOT NULL AND TRIM(display_name) <> ''
         ORDER BY score DESC, id",
    )?;
    let rows: Vec<(PersonId, String, Option<String>, Option<String>)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::new();
    for (id, name, title, company) in rows {
        if out.len() >= top_n as usize {
            break;
        }
        if moved.contains(&id) {
            continue;
        }
        out.push(Candidate {
            id,
            name,
            title,
            company,
            linkedin: linkedin_of(conn, id)?,
        });
    }
    Ok(out)
}

impl Watch {
    fn top_n(g: &Graph, inp: &WatchInput) -> u32 {
        inp.top_n
            .unwrap_or(g.config().playbooks.watch.top_n)
            .clamp(1, MAX_TOP_N)
    }

    fn jobs(g: &Graph, inp: &WatchInput, d: &Diff) -> Result<Vec<Candidate>> {
        if !inp.check_profiles {
            return Ok(Vec::new());
        }
        let moved: Vec<PersonId> = d.moves.iter().map(|m| m.id).collect();
        check_candidates(g.conn(), Self::top_n(g, inp), &moved)
    }
}

impl Playbook for Watch {
    fn name(&self) -> &'static str {
        "watch"
    }

    fn egress_max(&self) -> Vec<EgressMax> {
        vec![EgressMax {
            op: "profile".into(),
            max: crate::config::WatchPlaybook::default().top_n,
        }]
    }

    fn plan(&self, g: &Graph, gate: &mut EgressGate<'_>, input: &Value) -> Result<Plan> {
        let inp: WatchInput = parse_input("watch", input)?;
        let d = diff(g.conn())?;
        let cands = Self::jobs(g, &inp, &d)?;
        let quote = gate.quote(cands.iter().map(profile_job).collect())?;
        let mut steps = vec![
            local_step(1, "diff the latest two LinkedIn exports"),
            egress_step(STEP_CHECK, "check top profiles", "profile", &quote),
            local_step(3, "flag moves into target companies"),
        ];
        if !inp.check_profiles {
            steps[1].will_run = StepDecision::SkipNotRequested;
        }
        Ok(Plan {
            playbook: self.name().into(),
            run_id: gate.run_id().to_string(),
            steps,
            quote,
            egress_max: vec![EgressMax {
                op: "profile".into(),
                max: if inp.check_profiles {
                    Self::top_n(g, &inp)
                } else {
                    0
                },
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
        let inp: WatchInput = parse_input("watch", input)?;
        let mut plan = self.plan(g, gate, input)?;
        let conn = g.conn();
        let mut d = diff(conn)?;
        let mut notes = Vec::new();
        if d.exports.is_none() {
            notes.push(format!(
                "the LinkedIn diff needs two Connections.csv exports (found {}); import a newer export to see changes",
                d.n_exports
            ));
        }
        if inp.check_profiles {
            notes.extend(quote_notes(&plan, ctx.allow_egress));
        }
        let cands = Self::jobs(g, &inp, &d)?;
        let mut checked = 0u32;
        let mut partial = false;
        let quoted = !plan.quote.lines.is_empty();
        if quoted && !ctx.allow_egress {
            plan.mark_egress(StepDecision::SkipNoEgress);
        } else if quoted {
            gate.approve(&plan.quote.clone())?;
            let source = format!("enrich:{}", gate.backend());
            let jobs: Vec<_> = plan.quote.lines.iter().map(|l| l.job.clone()).collect();
            for job in jobs {
                let Some(c) = cands.iter().find(|c| Some(c.id) == job.person_id) else {
                    continue;
                };
                match gate.execute(&job)? {
                    Executed::Done { log_id, result } => {
                        checked += 1;
                        let EnrichData::Profile(p) = result.data else {
                            continue;
                        };
                        let (from, to) = (
                            nonempty(c.company.as_deref()),
                            nonempty(p.company.as_deref()),
                        );
                        if let (Some(x), Some(y)) = (from, to) {
                            if company_key(x) != company_key(y) {
                                let (name, score) = person(conn, c.id)?;
                                d.moves.push(Move {
                                    id: c.id,
                                    display_name: name,
                                    from: x.to_string(),
                                    to: y.to_string(),
                                    from_title: c.title.clone(),
                                    to_title: p.title.clone(),
                                    detected_by: source.clone(),
                                    seen_between: None,
                                    target_hit: false,
                                    score: round3(score),
                                    enrich_log_id: Some(log_id),
                                    evidence: Vec::new(),
                                });
                            }
                        }
                    }
                    Executed::Blocked { .. } => partial = true,
                    Executed::Failed { error, .. } => {
                        partial = true;
                        notes.push(format!("profile check for #{} failed: {error}", c.id));
                    }
                }
            }
            gate.commit()?;
        }
        // Step 3: target hits first, then by score.
        let targets: Vec<CompanyTarget> = inp
            .targets
            .clone()
            .unwrap_or_else(|| g.config().playbooks.watch.targets.clone())
            .iter()
            .filter_map(|t| CompanyTarget::parse(t))
            .collect();
        for m in &mut d.moves {
            m.target_hit = targets.iter().any(|t| t.matches(&m.to));
        }
        d.moves.sort_by(|a, b| {
            b.target_hit
                .cmp(&a.target_hit)
                .then(b.score.total_cmp(&a.score))
                .then(a.id.cmp(&b.id))
        });
        let sources = match &d.exports {
            Some((p, l)) => {
                let s = |e: &Export| json!({"id": e.id, "label": e.label, "imported_at": iso(e.imported_at)});
                json!({ "previous": s(p), "latest": s(l) })
            }
            None => Value::Null,
        };
        let result = json!({
            "moves": d.moves,
            "title_changes": d.titles,
            "new_connections": d.added,
            "removed_connections": d.removed,
            "checked": checked,
            "targets": targets.iter().map(|t| t.input.clone()).collect::<Vec<_>>(),
            "sources": sources,
        });
        Ok(Outcome {
            v: V,
            playbook: self.name().into(),
            status: egress_status(&plan, ctx.allow_egress, partial),
            receipt: gate.receipt(),
            plan,
            result: Some(result),
            notes,
        })
    }
}
