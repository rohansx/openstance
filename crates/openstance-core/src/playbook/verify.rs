//! `verify` (P4-CORE-05): check an address before you send, and name the
//! freshest mutual who replies. Spec: `docs/tech-spec/playbooks.md`
//! section 8.
//!
//! | # | step | egress |
//! |---|---|---|
//! | 1 | best email: confidence, then most recent use, then not `enrich:`-sourced | - |
//! | 2 | verify it, unless verified within `playbooks.verify.fresh_days` | 1 `verify_email` |
//! | 3 | the mutual with `C > 0` and the most recent tie | - |
//!
//! Without egress the result is the candidate address, `status: "unverified"`.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    egress_status, egress_step, local_step, parse_input, quote_notes, EgressMax, Outcome, Plan,
    Playbook, RunCtx, RunStatus, StepDecision,
};
use crate::egress::facts::verifications;
use crate::egress::{EgressGate, Executed};
use crate::enrich::{EnrichData, EnrichJob, EnrichRequest};
use crate::error::{Error, Result};
use crate::json::{iso, V};
use crate::resolve::lookup::{resolve_person, PersonKey};
use crate::{Graph, PersonId};

/// The note when a person has no email identity.
pub const NO_EMAIL_NOTE: &str = "no email in graph; try path --find-email";

const STEP_VERIFY: u8 = 2;
const DAY: i64 = 86_400;

/// `verify` input.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyInput {
    /// Id, email, LinkedIn, @handle or name. Must be in the graph.
    pub person: String,
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

/// The `verify` playbook.
#[derive(Debug, Clone, Copy, Default)]
pub struct Verify;

/// A recent enough verdict already in the graph.
#[derive(Debug, Clone)]
struct Fresh {
    status: Value,
    at: i64,
    via: String,
}

/// What the plan found locally.
struct Local {
    person: PersonId,
    email: Option<String>,
    fresh: Option<Fresh>,
}

fn target(conn: &Connection, person: &str) -> Result<PersonId> {
    let p = resolve_person(conn, &PersonKey::parse(person))?.person;
    let is_self: bool = conn.query_row("SELECT is_self FROM person WHERE id = ?1", [p], |r| {
        Ok(r.get::<_, i64>(0)? != 0)
    })?;
    if is_self {
        return Err(Error::BadRequest(
            "that is you: verify checks other people's addresses".into(),
        ));
    }
    Ok(p)
}

/// Step 1.
fn best_email(conn: &Connection, p: PersonId) -> Result<Option<String>> {
    let mut st = conn.prepare_cached(
        "SELECT i.id, i.value, COALESCE(i.confidence, 0), s.kind LIKE 'enrich:%',
                (SELECT MAX(x.at) FROM interaction_person ip
                 JOIN interaction x ON x.id = ip.interaction_id
                 WHERE ip.identity_id = i.id)
         FROM identity i JOIN source s ON s.id = i.source_id
         WHERE i.kind = 'email' AND i.person_id = ?1",
    )?;
    let mut rows: Vec<(i64, String, f64, bool, Option<i64>)> = st
        .query_map([p], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.sort_by(|a, b| {
        b.2.total_cmp(&a.2)
            .then(b.4.cmp(&a.4))
            .then(a.3.cmp(&b.3))
            .then(a.0.cmp(&b.0))
    });
    Ok(rows.into_iter().next().map(|r| r.1))
}

/// The freshest mutual who replied.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Mutual {
    /// Person id.
    pub id: PersonId,
    /// Display name.
    pub display_name: Option<String>,
    /// Their latest message to you (or the tie's last shared interaction).
    pub last_reply: String,
    /// Its subject.
    pub last_subject: Option<String>,
    /// Its interaction id (evidence).
    pub interaction_id: Option<i64>,
}

/// Step 3.
fn mutual(conn: &Connection, p: PersonId, now: i64) -> Result<Option<Mutual>> {
    let row: Option<(PersonId, Option<String>, i64)> = conn
        .prepare_cached(
            "SELECT m.id, m.display_name, t.last_at
             FROM tie t JOIN person m ON m.id = CASE WHEN t.a = ?1 THEN t.b ELSE t.a END
             WHERE (t.a = ?1 OR t.b = ?1) AND m.is_self = 0 AND m.is_bulk = 0 AND m.score_c > 0
             ORDER BY t.last_at DESC, m.id
             LIMIT 1",
        )?
        .query_row([p], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    let Some((id, name, tie_at)) = row else {
        return Ok(None);
    };
    let reply: Option<(i64, i64, Option<String>)> = conn
        .prepare_cached(
            "SELECT i.id, i.at, i.subject FROM interaction i
             JOIN interaction_person ip ON ip.interaction_id = i.id
             WHERE ip.person_id = ?1 AND ip.role IN ('from', 'sender')
               AND i.direction = 'in' AND i.at <= ?2
             ORDER BY i.at DESC, i.id DESC LIMIT 1",
        )?
        .query_row(rusqlite::params![id, now], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    Ok(Some(match reply {
        Some((iid, at, subject)) => Mutual {
            id,
            display_name: name,
            last_reply: iso(at),
            last_subject: subject,
            interaction_id: Some(iid),
        },
        None => Mutual {
            id,
            display_name: name,
            last_reply: iso(tie_at),
            last_subject: None,
            interaction_id: None,
        },
    }))
}

impl Verify {
    fn local(g: &Graph, inp: &VerifyInput) -> Result<Local> {
        let conn = g.conn();
        let person = target(conn, &inp.person)?;
        let email = best_email(conn, person)?;
        let fresh = match &email {
            Some(e) => {
                let window = i64::from(g.config().playbooks.verify.fresh_days) * DAY;
                verifications(conn, &[e.as_str()])?
                    .remove(e)
                    .filter(|f| g.now() - f.at <= window)
                    .map(|f| Fresh {
                        status: serde_json::to_value(f.status).unwrap_or(Value::Null),
                        at: f.at,
                        via: f.via,
                    })
            }
            None => None,
        };
        Ok(Local {
            person,
            email,
            fresh,
        })
    }
}

impl Playbook for Verify {
    fn name(&self) -> &'static str {
        "verify"
    }

    fn egress_max(&self) -> Vec<EgressMax> {
        vec![EgressMax {
            op: "verify_email".into(),
            max: 1,
        }]
    }

    fn plan(&self, g: &Graph, gate: &mut EgressGate<'_>, input: &Value) -> Result<Plan> {
        let inp: VerifyInput = parse_input("verify", input)?;
        let local = Self::local(g, &inp)?;
        let jobs = match (&local.email, &local.fresh) {
            (Some(e), None) => vec![EnrichJob {
                person_id: Some(local.person),
                request: EnrichRequest::VerifyEmail { email: e.clone() },
            }],
            _ => Vec::new(),
        };
        let quote = gate.quote(jobs)?;
        let mut steps = vec![
            local_step(1, "pick the best email"),
            egress_step(STEP_VERIFY, "verify email", "verify_email", &quote),
            local_step(3, "freshest mutual who replied"),
        ];
        if local.email.is_none() {
            steps[1].will_run = StepDecision::SkipNoInput;
        } else if local.fresh.is_some() {
            steps[1].will_run = StepDecision::SkipLocalSufficient;
        }
        Ok(Plan {
            playbook: self.name().into(),
            run_id: gate.run_id().to_string(),
            steps,
            quote,
            egress_max: self.egress_max(),
        })
    }

    fn run(
        &self,
        g: &Graph,
        gate: &mut EgressGate<'_>,
        input: &Value,
        ctx: &RunCtx,
    ) -> Result<Outcome> {
        let inp: VerifyInput = parse_input("verify", input)?;
        let mut plan = self.plan(g, gate, input)?;
        let local = Self::local(g, &inp)?;
        let mutual = mutual(g.conn(), local.person, ctx.now)?;
        let Some(email) = local.email.clone() else {
            return Ok(Outcome {
                v: V,
                playbook: self.name().into(),
                status: RunStatus::NothingToDo,
                receipt: gate.receipt(),
                plan,
                result: Some(json!({
                    "person_id": local.person,
                    "email": Value::Null,
                    "status": Value::Null,
                    "verified_at": Value::Null,
                    "source": Value::Null,
                    "mutual": mutual,
                })),
                notes: vec![NO_EMAIL_NOTE.into()],
            });
        };
        let mut notes = quote_notes(&plan, ctx.allow_egress);
        let mut status = json!("unverified");
        let mut verified_at = Value::Null;
        let mut source = Value::Null;
        let mut log_id = Value::Null;
        if let Some(f) = &local.fresh {
            status = f.status.clone();
            verified_at = json!(iso(f.at));
            source = json!(f.via);
            notes.push(format!(
                "verified on {} (within playbooks.verify.fresh_days); not re-checked",
                iso(f.at)
            ));
        }
        let mut partial = false;
        let quoted = !plan.quote.lines.is_empty();
        if quoted && !ctx.allow_egress {
            plan.mark_egress(StepDecision::SkipNoEgress);
        } else if quoted {
            gate.approve(&plan.quote.clone())?;
            let job = plan.quote.lines[0].job.clone();
            match gate.execute(&job)? {
                Executed::Done { log_id: id, result } => {
                    if let EnrichData::Verification { status: s, .. } = result.data {
                        status = serde_json::to_value(s)?;
                    } else {
                        status = json!("unknown");
                    }
                    verified_at = json!(iso(result.fetched_at));
                    source = json!(format!("enrich:{}", gate.backend()));
                    log_id = json!(id);
                }
                Executed::Blocked { reason, .. } => {
                    partial = true;
                    plan.mark(
                        STEP_VERIFY,
                        if reason == crate::enrich::SkipReason::OverBudget {
                            StepDecision::SkipOverBudget
                        } else {
                            StepDecision::SkipPrivacy
                        },
                    );
                    notes.push("verify email: not sent".into());
                }
                Executed::Failed { error, .. } => {
                    partial = true;
                    notes.push(format!("verify email failed: {error}"));
                }
            }
            gate.commit()?;
        }
        let result = json!({
            "person_id": local.person,
            "email": email,
            "status": status,
            "verified_at": verified_at,
            "source": source,
            "enrich_log_id": log_id,
            "mutual": mutual,
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
