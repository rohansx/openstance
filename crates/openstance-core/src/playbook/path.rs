//! `path` (P3-CORE-04): the best warm route to a person, in the graph or
//! not. Spec: `docs/tech-spec/playbooks.md` section 2.
//!
//! | # | step | egress |
//! |---|---|---|
//! | 1 | look the target up in the graph; if found, answer with `how_connected` | - |
//! | 2 | enrich the stranger's profile (employers, school) | 1 `profile` |
//! | 3 | intersect employers and school with the graph | - |
//! | 4 | rank mutuals by `my_score x affinity` | - |
//! | 5 | find the target's email at the company domain (optional) | 1 `find_email` |
//! | 6 | verify that email (optional) | 1 `verify_email` |
//!
//! Affinities (interfaces.md 1.7): same current company 1.0, a past
//! employer in common 0.6, the school named in bio or title 0.4.
//! Without egress, step 3 still runs on the company the caller gave.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    egress_step, local_step, parse_input, quote_notes, EgressMax, Outcome, Plan, Playbook, RunCtx,
    RunStatus, StepDecision,
};
use crate::egress::{EgressGate, Executed};
use crate::enrich::{EnrichData, EnrichJob, EnrichRequest, ProfileData, SkipReason};
use crate::error::{Error, Result};
use crate::graphq::{how_connected, HowConnectedResponse, HowOptions};
use crate::json::{iso, round3, Evidence, PathJson, V};
use crate::resolve::lookup::{resolve_person, PersonKey};
use crate::resolve::normalize::company_key;
use crate::search::evidence::recent;
use crate::search::person::person_ref;
use crate::{Graph, PersonId};

/// Paths returned by default.
pub const DEFAULT_LIMIT: usize = 3;
/// Most paths a caller can ask for.
pub const MAX_LIMIT: usize = 10;

/// Note when the target is unknown and enrichment is off (enrichment.md 3.1).
pub const NO_ENRICHER_NOTE: &str = "target not in graph; enable an enricher to look up employers";

const AFFINITY_SAME: f64 = 1.0;
const AFFINITY_PAST: f64 = 0.6;
const AFFINITY_SCHOOL: f64 = 0.4;

/// `path` input (MCP `path_to`; the run options are read by the surface).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathInput {
    /// Name, email, LinkedIn URL or person id. `"Name, Title at Company"`
    /// also works when `company` is not given.
    pub target: String,
    /// Disambiguates and seeds egress.
    #[serde(default)]
    pub company: Option<String>,
    /// LinkedIn URL or slug.
    #[serde(default)]
    pub linkedin: Option<String>,
    /// Add the find + verify email steps.
    #[serde(default)]
    pub find_email: bool,
    /// Paths to return (1..=10, default 3).
    #[serde(default)]
    pub limit: Option<usize>,
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

impl PathInput {
    /// `(name, company)`: `company` wins; otherwise a trailing
    /// `" at <Company>"` in the target text is used.
    fn name_and_company(&self) -> (String, Option<String>) {
        let t = self.target.trim();
        let given = self
            .company
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string);
        let (name, parsed) = match t.rsplit_once(" at ") {
            Some((head, co)) if !co.trim().is_empty() => {
                let name = head.split(',').next().unwrap_or(head).trim();
                (name.to_string(), Some(co.trim().to_string()))
            }
            _ => (t.split(',').next().unwrap_or(t).trim().to_string(), None),
        };
        let name = if name.is_empty() { t.to_string() } else { name };
        (name, given.or(parsed))
    }

    fn limit(&self) -> usize {
        self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
    }
}

/// The `path` playbook.
#[derive(Debug, Clone, Copy, Default)]
pub struct PathPlaybook;

/// Step numbers.
const STEP_LOOKUP: u8 = 1;
const STEP_PROFILE: u8 = 2;
const STEP_FIND: u8 = 5;
const STEP_VERIFY: u8 = 6;

impl Playbook for PathPlaybook {
    fn name(&self) -> &'static str {
        "path"
    }

    fn egress_max(&self) -> Vec<EgressMax> {
        ["profile", "find_email", "verify_email"]
            .iter()
            .map(|op| EgressMax {
                op: (*op).to_string(),
                max: 1,
            })
            .collect()
    }

    fn plan(&self, g: &Graph, gate: &mut EgressGate<'_>, input: &Value) -> Result<Plan> {
        let inp: PathInput = parse_input("path", input)?;
        let (name, company) = inp.name_and_company();
        let found = lookup(g.conn(), &name, company.as_deref(), inp.linkedin.as_deref())?;
        let jobs = if found.is_some() {
            Vec::new()
        } else {
            let mut jobs = vec![EnrichJob {
                person_id: None,
                request: EnrichRequest::Profile {
                    name: name.clone(),
                    company: company.clone(),
                    linkedin: inp.linkedin.clone(),
                },
            }];
            if inp.find_email {
                // The domain and the address come from earlier steps; the
                // quote prices the op only and records them as empty.
                jobs.push(EnrichJob {
                    person_id: None,
                    request: EnrichRequest::FindEmail {
                        name: name.clone(),
                        domain: String::new(),
                    },
                });
                jobs.push(EnrichJob {
                    person_id: None,
                    request: EnrichRequest::VerifyEmail {
                        email: String::new(),
                    },
                });
            }
            jobs
        };
        let quote = gate.quote(jobs)?;
        let mut steps = vec![
            local_step(STEP_LOOKUP, "lookup target in graph"),
            egress_step(STEP_PROFILE, "enrich profile", "profile", &quote),
            local_step(3, "intersect employers/school"),
            local_step(4, "rank mutuals"),
            egress_step(STEP_FIND, "find email", "find_email", &quote),
            egress_step(STEP_VERIFY, "verify email", "verify_email", &quote),
        ];
        for s in &mut steps {
            let email = matches!(s.n, STEP_FIND | STEP_VERIFY);
            if s.egress.is_some() && found.is_some() {
                s.will_run = StepDecision::SkipLocalSufficient;
            } else if email && !inp.find_email {
                s.will_run = StepDecision::SkipNotRequested;
            }
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
        let inp: PathInput = parse_input("path", input)?;
        let (name, company) = inp.name_and_company();
        let mut plan = self.plan(g, gate, input)?;
        let conn = g.conn();
        let limit = inp.limit();

        // Step 1: in the graph -> how_connected, no egress at all.
        if let Some(p) = lookup(conn, &name, company.as_deref(), inp.linkedin.as_deref())? {
            let how = how_connected(conn, &PersonKey::Id(p), HowOptions { limit }, ctx.now)?;
            let HowConnectedResponse::Found(h) = how else {
                return Err(Error::Internal(format!("person {p} vanished")));
            };
            let result = json!({
                "target": {
                    "person_id": h.target.id,
                    "display_name": h.target.display_name,
                    "company": h.target.company,
                    "in_graph": true,
                    "profile_source": Value::Null,
                },
                "direct": h.direct,
                "paths": h.paths,
                "email": Value::Null,
            });
            return Ok(Outcome {
                v: V,
                playbook: self.name().into(),
                status: RunStatus::Done,
                receipt: gate.receipt(),
                plan,
                result: Some(result),
                notes: vec![],
            });
        }

        let mut notes = quote_notes(&plan, ctx.allow_egress);
        if plan
            .quote
            .skipped
            .iter()
            .any(|s| s.reason == SkipReason::BackendNone)
        {
            notes.push(NO_ENRICHER_NOTE.into());
        }
        let egress = ctx.allow_egress && !plan.quote.lines.is_empty();
        if !plan.quote.lines.is_empty() && !ctx.allow_egress {
            plan.mark_egress(StepDecision::SkipNoEgress);
        }
        let mut partial = false;
        let mut target = Target {
            company: company.clone(),
            ..Target::default()
        };
        let mut profile_source = Value::Null;
        let mut email = Value::Null;

        if egress {
            gate.approve(&plan.quote.clone())?;
            // Step 2: profile.
            let job = plan
                .quote
                .lines
                .iter()
                .find(|l| l.job.request.op() == "profile")
                .map(|l| l.job.clone());
            match job
                .map(|j| gate.execute(&j))
                .transpose()?
                .unwrap_or(Executed::Blocked {
                    log_id: None,
                    reason: SkipReason::Unsupported,
                }) {
                Executed::Done { log_id, result } => match result.data {
                    EnrichData::Profile(p) => {
                        profile_source = json!(format!("enrich:{}", gate.backend()));
                        target.absorb(&p, log_id, result.fetched_at, gate.backend());
                    }
                    _ => notes.push("profile: no public profile found".into()),
                },
                other => {
                    partial = true;
                    note_unsent(&mut plan, STEP_PROFILE, &other, &mut notes);
                }
            }
            // Steps 5 and 6: email.
            if inp.find_email {
                email = find_and_verify(gate, &mut plan, &name, &target, &mut notes, &mut partial)?;
            }
        }

        // Steps 3 and 4: intersect and rank (local, also without egress).
        let paths = intersect(conn, &target, limit, ctx.now)?;
        let result = json!({
            "target": {
                "person_id": Value::Null,
                "display_name": name,
                "company": target.company,
                "in_graph": false,
                "profile_source": profile_source,
            },
            "direct": Value::Null,
            "paths": paths,
            "email": email,
        });
        let status = if !plan.quote.lines.is_empty() && !ctx.allow_egress {
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

/// Steps 5 and 6. Returns the `email` result (or `null`).
fn find_and_verify(
    gate: &mut EgressGate<'_>,
    plan: &mut Plan,
    name: &str,
    target: &Target,
    notes: &mut Vec<String>,
    partial: &mut bool,
) -> Result<Value> {
    let has = |op: &str| plan.quote.lines.iter().any(|l| l.job.request.op() == op);
    if !has("find_email") {
        return Ok(Value::Null);
    }
    let Some(domain) = target.domain.clone() else {
        notes.push("email: no company domain for the target; email steps skipped".into());
        plan.mark(STEP_FIND, StepDecision::SkipNoInput);
        plan.mark(STEP_VERIFY, StepDecision::SkipNoInput);
        return Ok(Value::Null);
    };
    let source = format!("enrich:{}", gate.backend());
    let find = EnrichJob {
        person_id: None,
        request: EnrichRequest::FindEmail {
            name: name.to_string(),
            domain,
        },
    };
    let address = match gate.execute(&find)? {
        Executed::Done { result, .. } => match result.data {
            EnrichData::Email { email, .. } => email,
            _ => {
                notes.push("email: no address found".into());
                plan.mark(STEP_VERIFY, StepDecision::SkipNoInput);
                return Ok(Value::Null);
            }
        },
        other => {
            *partial = true;
            note_unsent(plan, STEP_FIND, &other, notes);
            plan.mark(STEP_VERIFY, StepDecision::SkipNoInput);
            return Ok(Value::Null);
        }
    };
    let mut status = json!("unverified");
    if has("verify_email") {
        let verify = EnrichJob {
            person_id: None,
            request: EnrichRequest::VerifyEmail {
                email: address.clone(),
            },
        };
        match gate.execute(&verify)? {
            Executed::Done { result, .. } => {
                if let EnrichData::Verification { status: s, .. } = result.data {
                    status = serde_json::to_value(s)?;
                }
            }
            other => {
                *partial = true;
                note_unsent(plan, STEP_VERIFY, &other, notes);
            }
        }
    }
    Ok(json!({ "address": address, "status": status, "source": source }))
}

/// Record why an egress step did not produce a result.
fn note_unsent(plan: &mut Plan, step: u8, e: &Executed, notes: &mut Vec<String>) {
    let what = plan
        .steps
        .iter()
        .find(|s| s.n == step)
        .map_or_else(String::new, |s| s.description.clone());
    match e {
        Executed::Blocked { reason, .. } => {
            let d = match reason {
                SkipReason::OverBudget => StepDecision::SkipOverBudget,
                SkipReason::NoEnrich | SkipReason::DoNotShare => StepDecision::SkipPrivacy,
                _ => StepDecision::SkipNoEnricher,
            };
            plan.mark(step, d);
            let r = serde_json::to_value(reason)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            notes.push(format!("{what}: not sent ({r})"));
        }
        Executed::Failed { error, .. } => notes.push(format!("{what}: failed: {error}")),
        Executed::Done { .. } => {}
    }
}

/// What we know about the target for step 3.
#[derive(Debug, Default)]
struct Target {
    company: Option<String>,
    domain: Option<String>,
    past: Vec<String>,
    schools: Vec<String>,
    /// `(log id, fetched_at, backend)` of the profile answer.
    profile: Option<(i64, i64, String)>,
}

impl Target {
    fn absorb(&mut self, p: &ProfileData, log_id: i64, at: i64, backend: &str) {
        if let Some(c) = p.company.as_deref().filter(|c| !c.trim().is_empty()) {
            self.company = Some(c.trim().to_string());
        }
        self.domain = p.company_domain.clone().filter(|d| !d.trim().is_empty());
        self.past = p.past_companies.clone();
        self.schools = p.schools.clone();
        self.profile = Some((log_id, at, backend.to_string()));
    }

    fn enrich_evidence(&self, field: &str, value: &str) -> Option<Evidence> {
        self.profile
            .as_ref()
            .map(|(id, at, backend)| Evidence::Enrich {
                op: "profile".into(),
                field: field.into(),
                value: value.into(),
                backend: backend.clone(),
                at: iso(*at),
                enrich_log_id: *id,
            })
    }
}

/// Step 1: the target's person id, if it is in the graph. Identity first
/// (LinkedIn, then the reference itself), then name (+ company when given).
fn lookup(
    conn: &Connection,
    name: &str,
    company: Option<&str>,
    linkedin: Option<&str>,
) -> Result<Option<PersonId>> {
    if let Some(l) = linkedin.filter(|l| !l.trim().is_empty()) {
        if let Some(p) = found(conn, &PersonKey::parse(&format!("linkedin_url:{l}")))? {
            return Ok(Some(p));
        }
    }
    let key = PersonKey::parse(name);
    let candidates: Vec<PersonId> = match resolve_person(conn, &key) {
        Ok(r) => vec![r.person],
        Err(Error::NotFound(_)) => return Ok(None),
        Err(Error::Ambiguous { candidates, .. }) if company.is_some() => {
            candidates.into_iter().map(|(id, _)| id).collect()
        }
        Err(e) => return Err(e),
    };
    let by_handle = !matches!(key, PersonKey::Name(_));
    let mut matched = Vec::new();
    for p in candidates {
        let (co, is_self): (Option<String>, bool) = conn.query_row(
            "SELECT company, is_self FROM person WHERE id = ?1",
            [p],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
        )?;
        if is_self {
            return Err(Error::BadRequest(
                "that is you: path looks for routes to other people".into(),
            ));
        }
        let ok = by_handle
            || match (company, co.as_deref()) {
                (Some(want), Some(have)) => company_key(want) == company_key(have),
                _ => true,
            };
        if ok {
            matched.push(p);
        }
    }
    match matched.len() {
        0 => Ok(None),
        1 => Ok(Some(matched[0])),
        _ => resolve_person(conn, &key).map(|r| Some(r.person)),
    }
}

fn found(conn: &Connection, key: &PersonKey) -> Result<Option<PersonId>> {
    match resolve_person(conn, key) {
        Ok(r) => Ok(Some(r.person)),
        Err(Error::NotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// `" a b "`-style token string for whole-word matching.
fn words(s: &str) -> String {
    format!(" {} ", company_key(s))
}

/// Steps 3 and 4: people linked to the target by company or school,
/// strongest first.
fn intersect(conn: &Connection, t: &Target, limit: usize, now: i64) -> Result<Vec<PathJson>> {
    let tkey = t
        .company
        .as_deref()
        .map(company_key)
        .filter(|k| !k.is_empty());
    let past: BTreeMap<String, &str> = t
        .past
        .iter()
        .map(|c| (company_key(c), c.as_str()))
        .filter(|(k, _)| !k.is_empty())
        .collect();
    let schools: Vec<(&str, String)> = t
        .schools
        .iter()
        .map(|s| (s.as_str(), company_key(s)))
        .filter(|(_, k)| !k.is_empty())
        .collect();
    if tkey.is_none() && past.is_empty() && schools.is_empty() {
        return Ok(Vec::new());
    }
    // Companies each person was ever seen at (LinkedIn history, signatures).
    let mut history: BTreeMap<PersonId, BTreeSet<(String, String)>> = BTreeMap::new();
    let mut st = conn.prepare_cached(
        "SELECT DISTINCT person_id, company_hint FROM interaction_person
         WHERE person_id IS NOT NULL AND company_hint IS NOT NULL",
    )?;
    for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
        let (p, c) = row?;
        let k = company_key(&c);
        if !k.is_empty() {
            history.entry(p).or_default().insert((k, c));
        }
    }
    let mut st = conn.prepare_cached(
        "SELECT id, company, title, bio, score FROM person
         WHERE is_self = 0 AND is_bulk = 0 AND score > 0",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, PersonId>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, f64>(4)?,
        ))
    })?;
    let mut hits: Vec<(PersonId, f64, f64, &'static str, Vec<Evidence>)> = Vec::new();
    for row in rows {
        let (id, company, title, bio, score) = row?;
        let ckey = company.as_deref().map(company_key).unwrap_or_default();
        let hist = history.get(&id);
        let field = |field: &str, value: &str, matched: &str| Evidence::Field {
            field: field.into(),
            value: value.into(),
            matched: matched.into(),
        };
        let mut link: Option<(f64, &'static str, Vec<Evidence>)> = None;
        if let (Some(tk), Some(c)) = (tkey.as_deref(), company.as_deref()) {
            if ckey == tk {
                let mut ev = vec![field("company", c, "target.company")];
                ev.extend(t.enrich_evidence("company", c));
                link = Some((AFFINITY_SAME, "same_company", ev));
            }
        }
        if link.is_none() {
            // A past employer of the target that this person works at or
            // worked at, or the target's company in this person's history.
            let current = company
                .as_deref()
                .filter(|_| past.contains_key(&ckey))
                .map(|c| (c.to_string(), past[&ckey]));
            let earlier = hist.and_then(|h| {
                h.iter().find_map(|(k, c)| {
                    past.get(k).map(|p| (c.clone(), *p)).or_else(|| {
                        (Some(k.as_str()) == tkey.as_deref())
                            .then(|| (c.clone(), t.company.as_deref().unwrap_or_default()))
                    })
                })
            });
            if let Some((theirs, target_side)) = current.or(earlier) {
                let mut ev = vec![field("company", &theirs, "target.past_company")];
                ev.extend(t.enrich_evidence("past_companies", target_side));
                link = Some((AFFINITY_PAST, "past_company", ev));
            }
        }
        if link.is_none() {
            let text = words(&format!(
                "{} {}",
                bio.as_deref().unwrap_or(""),
                title.as_deref().unwrap_or("")
            ));
            if let Some((school, _)) = schools
                .iter()
                .find(|(_, k)| text.contains(&format!(" {k} ")))
            {
                let mut ev = vec![field("bio", school, "target.school")];
                ev.extend(t.enrich_evidence("schools", school));
                link = Some((AFFINITY_SCHOOL, "school", ev));
            }
        }
        if let Some((affinity, reason, ev)) = link {
            let strength = score * affinity;
            if strength > 0.0 {
                hits.push((id, score, affinity, reason, ev));
            }
        }
    }
    hits.sort_by(|a, b| {
        (b.1 * b.2)
            .total_cmp(&(a.1 * a.2))
            .then(b.1.total_cmp(&a.1))
            .then(a.0.cmp(&b.0))
    });
    let mut out = Vec::new();
    for (id, score, affinity, reason, mut evidence) in hits.into_iter().take(limit) {
        let Some(via) = person_ref(conn, id)? else {
            continue;
        };
        if let Some(mut mine) = recent(conn, &[id], 1, now)?.remove(&id) {
            evidence.append(&mut mine);
        }
        out.push(PathJson {
            via,
            strength: round3(score * affinity),
            reason: reason.into(),
            my_score: round3(score),
            link_weight: affinity,
            evidence,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(target: &str, company: Option<&str>) -> PathInput {
        PathInput {
            target: target.into(),
            company: company.map(str::to_string),
            linkedin: None,
            find_email: false,
            limit: None,
            allow_egress: None,
            max_usd: None,
            plan_only: None,
        }
    }

    #[test]
    fn target_text_splits_name_and_company() {
        assert_eq!(
            input("Dana Whitfield, CTO at Lumen Pay", None).name_and_company(),
            ("Dana Whitfield".into(), Some("Lumen Pay".into()))
        );
        assert_eq!(
            input("Dana Whitfield at Lumen Pay", Some("Orbit")).name_and_company(),
            ("Dana Whitfield".into(), Some("Orbit".into()))
        );
        assert_eq!(
            input("Dana Whitfield", None).name_and_company(),
            ("Dana Whitfield".into(), None)
        );
    }

    #[test]
    fn limit_is_clamped() {
        let mut i = input("x", None);
        assert_eq!(i.limit(), 3);
        i.limit = Some(50);
        assert_eq!(i.limit(), 10);
        i.limit = Some(0);
        assert_eq!(i.limit(), 1);
    }
}
