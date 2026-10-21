//! `brief` (P3-CORE-05): a deterministic intro brief of at most 10 lines.
//! Spec: `docs/tech-spec/playbooks.md` section 6.
//!
//! | # | step | egress |
//! |---|---|---|
//! | 1 | the last 5 interactions | - |
//! | 2 | mutuals ranked by `my_score x tie weight` (top 3, via `how_connected`) | - |
//! | 3 | one web search for recent public posts (optional, `web: true`) | 1 `web_search` |
//! | 4 | render the markdown | - |
//!
//! Sections, always in this order and each optional: who, how you know
//! them, last contact, best mutual, recent public activity, suggested
//! opener. Every line except "who" carries the interaction ids or
//! `enrich_log` ids it was built from (`result.facts`). No model writes
//! prose here.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    egress_step, local_step, parse_input, quote_notes, EgressMax, Outcome, Plan, Playbook, RunCtx,
    RunStatus, StepDecision,
};
use crate::egress::{EgressGate, Executed};
use crate::enrich::{EnrichData, EnrichJob, EnrichRequest, SkipReason, WebHit};
use crate::error::{Error, Result};
use crate::graphq::{get_person, how_connected, HowConnectedResponse, HowOptions};
use crate::json::{Evidence, PathJson, PersonDetail, V};
use crate::resolve::lookup::{resolve_person, PersonKey};
use crate::search::person::DetailOptions;
use crate::{Graph, PersonId};

/// Interactions read in step 1.
pub const LAST_INTERACTIONS: usize = 5;
/// Mutuals ranked in step 2.
pub const MUTUALS: usize = 3;
/// Web hits shown.
pub const WEB_HITS_SHOWN: usize = 2;
/// Hard cap on markdown lines.
pub const MAX_LINES: usize = 10;

const STEP_WEB: u8 = 3;

/// `brief` input (MCP `brief`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BriefInput {
    /// Id, email, LinkedIn, @handle or name. Must be in the graph.
    pub person: String,
    /// One web search for recent public posts (costs money).
    #[serde(default)]
    pub web: bool,
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

/// One markdown line and the evidence it was built from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fact {
    /// `who | how_you_know | last_contact | best_mutual |
    /// recent_public_activity | opener`.
    pub section: &'static str,
    /// 1-based markdown line.
    pub line: usize,
    /// The line text.
    pub text: String,
    /// Interactions behind it.
    pub interaction_ids: Vec<i64>,
    /// `enrich_log` rows behind it.
    pub enrich_log_ids: Vec<i64>,
    /// People it names.
    pub person_ids: Vec<PersonId>,
}

/// A web hit with the `enrich_log` row it came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WebFact {
    /// The hit.
    #[serde(flatten)]
    pub hit: WebHit,
    /// `enrich_log.id`.
    pub enrich_log_id: i64,
}

/// The `brief` playbook.
#[derive(Debug, Clone, Copy, Default)]
pub struct Brief;

fn query_for(name: &str, company: Option<&str>) -> String {
    match company.filter(|c| !c.trim().is_empty()) {
        Some(c) => format!("\"{name}\" \"{}\"", c.trim()),
        None => format!("\"{name}\""),
    }
}

/// The person (must be in the graph, not the owner).
fn target(g: &Graph, person: &str) -> Result<(PersonId, Option<String>, Option<String>)> {
    let p = resolve_person(g.conn(), &PersonKey::parse(person))?.person;
    let (name, company, is_self): (Option<String>, Option<String>, bool) = g.conn().query_row(
        "SELECT display_name, company, is_self FROM person WHERE id = ?1",
        [p],
        |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)),
    )?;
    if is_self {
        return Err(Error::BadRequest(
            "that is you: brief describes other people".into(),
        ));
    }
    Ok((p, name, company))
}

impl Playbook for Brief {
    fn name(&self) -> &'static str {
        "brief"
    }

    fn egress_max(&self) -> Vec<EgressMax> {
        vec![EgressMax {
            op: "web_search".into(),
            max: 1,
        }]
    }

    fn plan(&self, g: &Graph, gate: &mut EgressGate<'_>, input: &Value) -> Result<Plan> {
        let inp: BriefInput = parse_input("brief", input)?;
        let (p, name, company) = target(g, &inp.person)?;
        let mut jobs = Vec::new();
        if inp.web {
            let name = name.unwrap_or_else(|| format!("#{p}"));
            jobs.push(EnrichJob {
                person_id: Some(p),
                request: EnrichRequest::WebSearch {
                    query: query_for(&name, company.as_deref()),
                },
            });
        }
        let quote = gate.quote(jobs)?;
        let mut steps = vec![
            local_step(1, "last 5 interactions"),
            local_step(2, "rank mutuals"),
            egress_step(
                STEP_WEB,
                "web search for recent posts",
                "web_search",
                &quote,
            ),
            local_step(4, "render brief"),
        ];
        if !inp.web {
            steps[2].will_run = StepDecision::SkipNotRequested;
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
        let inp: BriefInput = parse_input("brief", input)?;
        let mut plan = self.plan(g, gate, input)?;
        let (p, _, _) = target(g, &inp.person)?;
        let detail = get_person(
            g.conn(),
            g.config(),
            &PersonKey::Id(p),
            DetailOptions {
                interactions: LAST_INTERACTIONS,
                mutuals: MUTUALS,
            },
            ctx.now,
        )?;
        let mutuals = match how_connected(
            g.conn(),
            &PersonKey::Id(p),
            HowOptions { limit: MUTUALS },
            ctx.now,
        )? {
            HowConnectedResponse::Found(h) => h.paths,
            HowConnectedResponse::NotFound(_) => Vec::new(),
        };
        let mut notes = Vec::new();
        if inp.web {
            notes = quote_notes(&plan, ctx.allow_egress);
            if plan
                .quote
                .skipped
                .iter()
                .any(|s| matches!(s.reason, SkipReason::NoEnrich | SkipReason::DoNotShare))
            {
                notes.push("web search skipped: this person is no_enrich".into());
            }
        }
        let quoted = !plan.quote.lines.is_empty();
        let mut web = Vec::new();
        let mut partial = false;
        if quoted && !ctx.allow_egress {
            plan.mark_egress(StepDecision::SkipNoEgress);
        } else if quoted {
            gate.approve(&plan.quote.clone())?;
            let job = plan.quote.lines[0].job.clone();
            match gate.execute(&job)? {
                Executed::Done { log_id, result } => {
                    if let EnrichData::WebHits { hits } = result.data {
                        web = hits
                            .into_iter()
                            .map(|hit| WebFact {
                                hit,
                                enrich_log_id: log_id,
                            })
                            .collect();
                    }
                }
                Executed::Blocked { reason, .. } => {
                    partial = true;
                    plan.mark(
                        STEP_WEB,
                        if reason == SkipReason::OverBudget {
                            StepDecision::SkipOverBudget
                        } else {
                            StepDecision::SkipPrivacy
                        },
                    );
                    notes.push("web search: not sent".into());
                }
                Executed::Failed { error, .. } => {
                    partial = true;
                    notes.push(format!("web search failed: {error}"));
                }
            }
        }
        let (markdown, facts) = render(&detail, &mutuals, &web);
        let result = json!({
            "markdown": markdown,
            "person": detail.person,
            "interactions": detail.recent_interactions,
            "mutuals": mutuals,
            "web": web,
            "facts": facts,
        });
        let status = if quoted && !ctx.allow_egress {
            RunStatus::Quote
        } else if partial {
            RunStatus::Partial
        } else {
            RunStatus::Done
        };
        Ok(Outcome {
            v: V,
            playbook: self.name().into(),
            status,
            receipt: gate.receipt(),
            plan,
            result: Some(result),
            notes,
        })
    }
}

/// One interaction, compact: `2026-10-15 email in "Re: seed round notes"`.
fn short(e: &Evidence) -> Option<(i64, String, String)> {
    match e {
        Evidence::Interaction {
            interaction_id,
            kind,
            direction,
            at,
            subject,
            ..
        } => {
            let day = at.get(..10).unwrap_or(at).to_string();
            let mut s = format!("{day} {kind}");
            if let Some(d) = direction
                .as_deref()
                .filter(|d| !d.is_empty() && *d != "none")
            {
                s.push(' ');
                s.push_str(d);
            }
            if let Some(sub) = subject.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
                s.push_str(&format!(" \"{sub}\""));
            }
            Some((*interaction_id, s, day))
        }
        _ => None,
    }
}

fn interaction_ids(ev: &[Evidence]) -> Vec<i64> {
    ev.iter()
        .filter_map(|e| match e {
            Evidence::Interaction { interaction_id, .. } => Some(*interaction_id),
            _ => None,
        })
        .collect()
}

/// The markdown (at most [`MAX_LINES`] lines) and its facts.
pub fn render(d: &PersonDetail, mutuals: &[PathJson], web: &[WebFact]) -> (String, Vec<Fact>) {
    let p = &d.person;
    let mut facts: Vec<Fact> = Vec::new();
    let mut push = |section, text: String, ix: Vec<i64>, enr: Vec<i64>, people: Vec<PersonId>| {
        facts.push(Fact {
            section,
            line: facts.len() + 1,
            text,
            interaction_ids: ix,
            enrich_log_ids: enr,
            person_ids: people,
        });
    };

    // Who.
    let name = p
        .display_name
        .clone()
        .unwrap_or_else(|| format!("#{}", p.id));
    let role: Vec<&str> = [
        p.title.as_deref(),
        p.company.as_deref(),
        p.location.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .collect();
    let who = if role.is_empty() {
        format!("**{name}** (score {:.2})", p.score)
    } else {
        format!("**{name}** - {} (score {:.2})", role.join(", "), p.score)
    };
    push("who", who, vec![], vec![], vec![p.id]);

    // How you know them.
    let recent: Vec<(i64, String, String)> =
        d.recent_interactions.iter().filter_map(short).collect();
    if !recent.is_empty() {
        let raw = p.components.raw.as_ref();
        let n = raw.map_or(recent.len() as u32, |r| {
            r.n_interactions.max(recent.len() as u32)
        });
        let mut s = format!(
            "- How you know them: {n} interaction{}",
            if n == 1 { "" } else { "s" }
        );
        if let Some(first) = p.first_seen.as_deref() {
            s.push_str(&format!(" since {}", first.get(..10).unwrap_or(first)));
        }
        if let Some(ch) = raw.map(|r| &r.channels).filter(|c| !c.is_empty()) {
            s.push_str(&format!(" ({})", ch.join(", ")));
        }
        s.push('.');
        push(
            "how_you_know",
            s,
            recent.iter().map(|r| r.0).collect(),
            vec![],
            vec![],
        );
        if recent.len() > 1 {
            let earlier: Vec<&str> = recent[1..].iter().map(|r| r.1.as_str()).collect();
            push(
                "how_you_know",
                format!("- Earlier: {}.", earlier.join("; ")),
                recent[1..].iter().map(|r| r.0).collect(),
                vec![],
                vec![],
            );
        }
        // Last contact.
        push(
            "last_contact",
            format!("- Last contact: {}.", recent[0].1),
            vec![recent[0].0],
            vec![],
            vec![],
        );
    }

    // Best mutual.
    let best = mutuals
        .first()
        .filter(|m| !interaction_ids(&m.evidence).is_empty());
    if let Some(m) = best {
        let via = m
            .via
            .display_name
            .clone()
            .unwrap_or_else(|| format!("#{}", m.via.id));
        let ties: Vec<String> = m
            .evidence
            .iter()
            .filter_map(|e| match e {
                Evidence::Tie {
                    tie_kind, n_shared, ..
                } => Some(format!("{n_shared} shared {tie_kind}")),
                _ => None,
            })
            .collect();
        let mut s = format!("- Best mutual: {via} (score {:.2}", m.my_score);
        if !ties.is_empty() {
            s.push_str(&format!(", {}", ties.join(", ")));
        }
        s.push_str(&format!(", strength {:.2}).", m.strength));
        push(
            "best_mutual",
            s,
            interaction_ids(&m.evidence),
            vec![],
            vec![m.via.id],
        );
    }

    // Recent public activity.
    for w in web.iter().take(WEB_HITS_SHOWN) {
        let mut s = format!("- Recent public activity: \"{}\"", w.hit.title.trim());
        if let Some(day) = w.hit.published.as_deref() {
            s.push_str(&format!(" ({})", day.get(..10).unwrap_or(day)));
        }
        s.push_str(&format!(" {}", w.hit.url));
        push(
            "recent_public_activity",
            s,
            vec![],
            vec![w.enrich_log_id],
            vec![],
        );
    }

    // Suggested opener, from facts only.
    if let Some((id, _, day)) = recent.first() {
        let subject = match &d.recent_interactions[0] {
            Evidence::Interaction { subject, .. } => subject.clone(),
            _ => None,
        };
        let mut s = match subject.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
            Some(sub) => format!("- Suggested opener: follow up on \"{sub}\" from {day}"),
            None => format!("- Suggested opener: follow up on your last contact on {day}"),
        };
        let mut enr = vec![];
        if let Some(w) = web.first() {
            s.push_str(&format!(", and mention \"{}\"", w.hit.title.trim()));
            enr.push(w.enrich_log_id);
        }
        s.push('.');
        push("opener", s, vec![*id], enr, vec![]);
    } else if let Some(m) = best {
        let via = m.via.display_name.clone().unwrap_or_default();
        push(
            "opener",
            format!("- Suggested opener: ask {via} for an introduction."),
            interaction_ids(&m.evidence),
            vec![],
            vec![m.via.id],
        );
    }

    facts.truncate(MAX_LINES);
    let mut md = String::new();
    for f in &facts {
        md.push_str(&f.text);
        md.push('\n');
    }
    (md, facts)
}
