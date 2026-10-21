//! `reconnect` (P4-CORE-01): people you used to talk to a lot and have lost
//! touch with. Spec: `docs/tech-spec/playbooks.md` section 7. No egress.
//!
//! | # | step | egress |
//! |---|---|---|
//! | 1 | people with `F > min_frequency` and `R < max_recency`, not bulk | - |
//! | 2 | sort by past score (the score with `R = 1`) | - |
//! | 3 | top `limit`, each with the last subject and date | - |
//!
//! The components are the stored ones (the same numbers `who` and search
//! show). Merged-away people have no `person` row, so they never appear.

use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    local_outcome, local_plan, local_step, parse_input, EgressMax, Outcome, Plan, Playbook, RunCtx,
};
use crate::egress::EgressGate;
use crate::error::{Error, Result};
use crate::json::{iso_opt, round3};
use crate::score::{components, Components, ScoreConfig};
use crate::{Graph, PersonId};

/// Most people a caller can ask for.
pub const MAX_LIMIT: u32 = 100;

/// `reconnect` input; missing fields come from `[playbooks.reconnect]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconnectInput {
    /// People returned.
    #[serde(default)]
    pub limit: Option<u32>,
    /// `F` must be above this.
    #[serde(default)]
    pub min_frequency: Option<f64>,
    /// `R` must be below this.
    #[serde(default)]
    pub max_recency: Option<f64>,
    /// Accepted for symmetry with the other playbooks; never used.
    #[serde(default)]
    pub allow_egress: Option<bool>,
    /// Accepted; never used.
    #[serde(default)]
    pub max_usd: Option<f64>,
    /// Read by the surface.
    #[serde(default)]
    pub plan_only: Option<bool>,
}

/// The `reconnect` playbook.
#[derive(Debug, Clone, Copy, Default)]
pub struct Reconnect;

impl Playbook for Reconnect {
    fn name(&self) -> &'static str {
        "reconnect"
    }

    fn egress_max(&self) -> Vec<EgressMax> {
        Vec::new()
    }

    fn plan(&self, _g: &Graph, gate: &mut EgressGate<'_>, input: &Value) -> Result<Plan> {
        let _: ReconnectInput = parse_input("reconnect", input)?;
        local_plan(
            self,
            gate,
            vec![
                local_step(1, "select lapsed people (F high, R low)"),
                local_step(2, "rank by past score"),
                local_step(3, "attach last subject"),
            ],
        )
    }

    fn run(
        &self,
        g: &Graph,
        gate: &mut EgressGate<'_>,
        input: &Value,
        ctx: &RunCtx,
    ) -> Result<Outcome> {
        let inp: ReconnectInput = parse_input("reconnect", input)?;
        let plan = self.plan(g, gate, input)?;
        let cfg = &g.config().playbooks.reconnect;
        let limit = inp.limit.unwrap_or(cfg.limit).clamp(1, MAX_LIMIT);
        let min_f = inp.min_frequency.unwrap_or(cfg.min_frequency);
        let max_r = inp.max_recency.unwrap_or(cfg.max_recency);
        if !min_f.is_finite() || !max_r.is_finite() {
            return Err(Error::BadRequest(
                "min_frequency and max_recency must be numbers".into(),
            ));
        }
        let (score_cfg, _) = ScoreConfig::from_config(&g.config().score)?;
        let people = lapsed(g.conn(), &score_cfg, min_f, max_r, limit, ctx.now)?;
        Ok(local_outcome(
            self,
            gate,
            plan,
            json!({ "people": people }),
            Vec::new(),
        ))
    }
}

/// One `reconnect` row.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Lapsed {
    /// Person id.
    pub id: PersonId,
    /// Display name.
    pub display_name: Option<String>,
    /// Title.
    pub title: Option<String>,
    /// Company.
    pub company: Option<String>,
    /// The score evaluated with `R = 1`.
    pub past_score: f64,
    /// The current score.
    pub score: f64,
    /// Last contact (ISO 8601).
    pub last_contact: Option<String>,
    /// Subject of the latest past interaction.
    pub last_subject: Option<String>,
    /// That interaction (evidence).
    pub last_interaction_id: Option<i64>,
}

fn lapsed(
    conn: &rusqlite::Connection,
    cfg: &ScoreConfig,
    min_f: f64,
    max_r: f64,
    limit: u32,
    now: i64,
) -> Result<Vec<Lapsed>> {
    let mut st = conn.prepare(
        "SELECT id, display_name, title, company, score, score_f, score_c, score_m, score_b,
                last_contact
         FROM person
         WHERE is_self = 0 AND is_bulk = 0 AND score_f > ?1 AND score_r < ?2",
    )?;
    let mut rows: Vec<Lapsed> = st
        .query_map(rusqlite::params![min_f, max_r], |r| {
            let comps = Components {
                recency: 1.0,
                frequency: r.get(5)?,
                reciprocity: r.get(6)?,
                meetings: r.get(7)?,
                breadth: r.get(8)?,
            };
            Ok((
                Lapsed {
                    id: r.get(0)?,
                    display_name: r.get(1)?,
                    title: r.get(2)?,
                    company: r.get(3)?,
                    past_score: 0.0,
                    score: round3(r.get(4)?),
                    last_contact: iso_opt(r.get(9)?),
                    last_subject: None,
                    last_interaction_id: None,
                },
                comps,
            ))
        })?
        .map(|row| {
            row.map(|(mut l, comps)| {
                l.past_score = round3(components::total(&comps, &cfg.weights));
                l
            })
        })
        .collect::<rusqlite::Result<_>>()?;
    rows.sort_by(|a, b| {
        b.past_score
            .total_cmp(&a.past_score)
            .then(b.score.total_cmp(&a.score))
            .then(a.id.cmp(&b.id))
    });
    rows.truncate(limit as usize);
    let mut last = conn.prepare_cached(
        "SELECT i.id, i.subject FROM interaction i
         JOIN interaction_person ip ON ip.interaction_id = i.id
         WHERE ip.person_id = ?1 AND i.kind <> 'contact' AND i.at <= ?2
         ORDER BY i.at DESC, i.id DESC LIMIT 1",
    )?;
    for l in &mut rows {
        if let Some((id, subject)) = last
            .query_row(rusqlite::params![l.id, now], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .optional()?
        {
            l.last_interaction_id = Some(id);
            l.last_subject = subject;
        }
    }
    Ok(rows)
}
