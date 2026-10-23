//! `accounts` (P4-CORE-02): warm coverage of a list of target companies.
//! Spec: `docs/tech-spec/playbooks.md` section 3. No egress.
//!
//! | # | step | egress |
//! |---|---|---|
//! | 1 | current: `company` matches, or no company and a work address at the company domain | - |
//! | 2 | alumni: an older LinkedIn export, an enrichment profile's past employers, or an old work address | - |
//! | 3 | one hop: ties with `n_shared >= 2` into 1 or 2, by `my_score x tie.weight` | - |
//! | 4 | coverage table | - |
//!
//! A name target (`"Contoso"`) learns its domains from the work addresses
//! of the people found in step 1 (freemail never counts); a domain target
//! (`"stripe.com"`) matches the company whose key is the domain's first
//! label (or the bundled domain map).

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    local_outcome, local_plan, local_step, parse_input, CompanyTarget, EgressMax, Outcome, Plan,
    Playbook, RunCtx,
};
use crate::egress::EgressGate;
use crate::enrich::EnrichData;
use crate::error::{Error, Result};
use crate::json::round3;
use crate::resolve::rules::DataFiles;
use crate::{Graph, PersonId};

/// Rows per list by default.
pub const DEFAULT_LIMIT: usize = 10;
/// Most companies per run.
pub const MAX_COMPANIES: usize = 500;
/// `warm` when a current contact scores at least this.
pub const WARM_SCORE: f64 = 0.6;
/// One-hop ties need this many shared interactions.
pub const MIN_SHARED: i64 = 2;

/// `accounts` input.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountsInput {
    /// Company names or domains.
    pub companies: Vec<String>,
    /// Add people one tie away (default true).
    #[serde(default)]
    pub one_hop: Option<bool>,
    /// Rows per list (default 10).
    #[serde(default)]
    pub limit_per_company: Option<usize>,
    /// Accepted; never used (no egress).
    #[serde(default)]
    pub allow_egress: Option<bool>,
    /// Accepted; never used.
    #[serde(default)]
    pub max_usd: Option<f64>,
    /// Read by the surface.
    #[serde(default)]
    pub plan_only: Option<bool>,
}

/// The `accounts` playbook.
#[derive(Debug, Clone, Copy, Default)]
pub struct Accounts;

impl Playbook for Accounts {
    fn name(&self) -> &'static str {
        "accounts"
    }

    fn egress_max(&self) -> Vec<EgressMax> {
        Vec::new()
    }

    fn plan(&self, _g: &Graph, gate: &mut EgressGate<'_>, input: &Value) -> Result<Plan> {
        targets(&parse_input::<AccountsInput>("accounts", input)?)?;
        local_plan(
            self,
            gate,
            vec![
                local_step(1, "current people at each company"),
                local_step(2, "alumni (LinkedIn history, enrichment, old addresses)"),
                local_step(3, "one hop through ties"),
                local_step(4, "coverage table"),
            ],
        )
    }

    fn run(
        &self,
        g: &Graph,
        gate: &mut EgressGate<'_>,
        input: &Value,
        _ctx: &RunCtx,
    ) -> Result<Outcome> {
        let inp: AccountsInput = parse_input("accounts", input)?;
        let plan = self.plan(g, gate, input)?;
        let targets = targets(&inp)?;
        let data = Data::load(g)?;
        let limit = inp
            .limit_per_company
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(1, 1000);
        let one_hop = inp.one_hop.unwrap_or(true);
        let mut rows = Vec::new();
        for t in &targets {
            rows.push(data.company(g.conn(), t, one_hop, limit)?);
        }
        Ok(local_outcome(
            self,
            gate,
            plan,
            json!({ "companies": rows }),
            Vec::new(),
        ))
    }
}

fn targets(inp: &AccountsInput) -> Result<Vec<CompanyTarget>> {
    let t: Vec<CompanyTarget> = inp
        .companies
        .iter()
        .filter_map(|c| CompanyTarget::parse(c))
        .collect();
    if t.is_empty() {
        return Err(Error::BadRequest(
            "accounts needs at least one company name or domain".into(),
        ));
    }
    if t.len() > MAX_COMPANIES {
        return Err(Error::BadRequest(format!(
            "accounts takes at most {MAX_COMPANIES} companies per run"
        )));
    }
    Ok(t)
}

/// One person in a coverage list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Current {
    /// Person id.
    pub id: PersonId,
    /// Display name.
    pub display_name: Option<String>,
    /// Title.
    pub title: Option<String>,
    /// Company.
    pub company: Option<String>,
    /// Relationship score.
    pub score: f64,
}

/// Someone who used to work at the company.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Alumnus {
    /// Person id.
    pub id: PersonId,
    /// Display name.
    pub display_name: Option<String>,
    /// Where they are now.
    pub company: Option<String>,
    /// `YYYY-MM` of the first LinkedIn export that showed them elsewhere.
    pub left_seen: Option<String>,
    /// `linkedin_csv | enrich | email_domain`.
    pub seen_in: &'static str,
    /// Relationship score.
    pub score: f64,
}

/// Someone one tie away from a current or former employee.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OneHop {
    /// Person id.
    pub id: PersonId,
    /// Display name.
    pub display_name: Option<String>,
    /// The insider they are tied to.
    pub via: PersonId,
    /// `my_score x tie.weight`.
    pub strength: f64,
    /// Relationship score.
    pub score: f64,
}

/// One coverage row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompanyRow {
    /// Display name (the most common company spelling among current people).
    pub company: String,
    /// The domain given or learned from work addresses.
    pub domain: Option<String>,
    /// Current people, by score.
    pub current: Vec<Current>,
    /// Alumni, by score.
    pub alumni: Vec<Alumnus>,
    /// One hop, by strength.
    pub one_hop: Vec<OneHop>,
    /// Totals before `limit_per_company`.
    pub counts: Counts,
    /// `warm | known | none`.
    pub coverage: &'static str,
    /// Best contact.
    pub best: Option<PersonId>,
    /// Their score (or one-hop strength).
    pub best_score: Option<f64>,
}

/// List sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Counts {
    /// Current people.
    pub current: usize,
    /// Alumni.
    pub alumni: usize,
    /// One hop.
    pub one_hop: usize,
}

#[derive(Debug, Clone)]
struct Person {
    name: Option<String>,
    title: Option<String>,
    company: Option<String>,
    score: f64,
}

/// One LinkedIn `Connections.csv` export.
#[derive(Debug, Clone)]
pub(crate) struct Export {
    pub id: i64,
    pub label: Option<String>,
    pub imported_at: i64,
}

/// Every `Connections.csv` export, oldest first (by import time, then id).
pub(crate) fn linkedin_exports(conn: &Connection) -> Result<Vec<Export>> {
    let mut st = conn.prepare_cached(
        "SELECT s.id, s.label, s.imported_at FROM source s
         WHERE s.kind = 'linkedin_csv' AND EXISTS (
           SELECT 1 FROM interaction i
           WHERE i.source_id = s.id AND i.kind = 'connection' AND i.subject IS NULL)
         ORDER BY s.imported_at, s.id",
    )?;
    let rows = st.query_map([], |r| {
        Ok(Export {
            id: r.get(0)?,
            label: r.get(1)?,
            imported_at: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// One person's row in one export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExportRow {
    pub interaction_id: i64,
    pub company: Option<String>,
    pub title: Option<String>,
}

/// person -> their row in export `source`.
pub(crate) fn export_rows(conn: &Connection, source: i64) -> Result<BTreeMap<PersonId, ExportRow>> {
    let mut st = conn.prepare_cached(
        "SELECT ip.person_id, i.id, ip.company_hint, ip.title_hint
         FROM interaction i JOIN interaction_person ip ON ip.interaction_id = i.id
         WHERE i.source_id = ?1 AND i.kind = 'connection' AND i.subject IS NULL
           AND ip.role = 'subject' AND ip.person_id IS NOT NULL
         ORDER BY i.id",
    )?;
    let mut out = BTreeMap::new();
    for row in st.query_map([source], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            ExportRow {
                interaction_id: r.get(1)?,
                company: r.get(2)?,
                title: r.get(3)?,
            },
        ))
    })? {
        let (p, e) = row?;
        out.entry(p).or_insert(e);
    }
    Ok(out)
}

/// `YYYY-MM` of a unix time.
pub(crate) fn month(ts: i64) -> String {
    crate::json::iso(ts).chars().take(7).collect()
}

/// Everything `accounts` reads, loaded once per run.
struct Data {
    people: BTreeMap<PersonId, Person>,
    /// person -> non-freemail email domains.
    domains: BTreeMap<PersonId, BTreeSet<String>>,
    /// Exports, oldest first, with each person's row.
    history: Vec<(Export, BTreeMap<PersonId, ExportRow>)>,
    /// person -> past employers from committed enrichment profiles.
    past: BTreeMap<PersonId, Vec<String>>,
    files: DataFiles,
}

impl Data {
    fn load(g: &Graph) -> Result<Data> {
        let conn = g.conn();
        let files = DataFiles::load(&g.config().resolution);
        let mut people = BTreeMap::new();
        let mut st = conn.prepare(
            "SELECT id, display_name, title, company, score FROM person
             WHERE is_self = 0 AND is_bulk = 0",
        )?;
        for row in st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Person {
                    name: r.get(1)?,
                    title: r.get(2)?,
                    company: r.get(3)?,
                    score: r.get(4)?,
                },
            ))
        })? {
            let (id, p) = row?;
            people.insert(id, p);
        }
        let mut domains: BTreeMap<PersonId, BTreeSet<String>> = BTreeMap::new();
        let mut st = conn.prepare(
            "SELECT person_id, value FROM identity WHERE kind = 'email' AND person_id IS NOT NULL",
        )?;
        for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (p, email) = row?;
            if let Some((_, d)) = email.rsplit_once('@') {
                let d = d.to_ascii_lowercase();
                if !files.freemail.contains(&d) {
                    domains.entry(p).or_default().insert(d);
                }
            }
        }
        let mut history = Vec::new();
        for e in linkedin_exports(conn)? {
            let rows = export_rows(conn, e.id)?;
            history.push((e, rows));
        }
        let mut past: BTreeMap<PersonId, Vec<String>> = BTreeMap::new();
        let mut st = conn.prepare(
            "SELECT person_id, response_json FROM enrich_log
             WHERE status = 'ok' AND op = 'profile' AND source_id IS NOT NULL
               AND person_id IS NOT NULL AND response_json IS NOT NULL
             ORDER BY id",
        )?;
        for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (p, json) = row?;
            if let Ok(EnrichData::Profile(prof)) = serde_json::from_str::<EnrichData>(&json) {
                let p = crate::resolve::lookup::resolve_person(
                    conn,
                    &crate::resolve::lookup::PersonKey::Id(p),
                )
                .map_or(p, |r| r.person);
                past.entry(p).or_default().extend(prof.past_companies);
            }
        }
        Ok(Data {
            people,
            domains,
            history,
            past,
            files,
        })
    }

    fn company_matches(&self, t: &CompanyTarget, company: Option<&str>) -> bool {
        company.is_some_and(|c| !c.trim().is_empty() && t.matches(c))
    }

    fn company(
        &self,
        conn: &Connection,
        t: &CompanyTarget,
        one_hop: bool,
        limit: usize,
    ) -> Result<CompanyRow> {
        let mut t = t.clone();
        if let Some(d) = &t.domain {
            if let Some(k) = self.files.domains.get(d) {
                t.key.clone_from(k);
            }
        }
        // Step 1: current, by company field.
        let mut current: BTreeSet<PersonId> = self
            .people
            .iter()
            .filter(|(_, p)| self.company_matches(&t, p.company.as_deref()))
            .map(|(id, _)| *id)
            .collect();
        // The company's domains: given, or learned from the people found.
        let mut domain_votes: BTreeMap<String, usize> = BTreeMap::new();
        if let Some(d) = &t.domain {
            domain_votes.insert(d.clone(), usize::MAX);
        }
        for id in &current {
            for d in self.domains.get(id).into_iter().flatten() {
                let v = domain_votes.entry(d.clone()).or_insert(0);
                *v = v.saturating_add(1);
            }
        }
        let domains: BTreeSet<&String> = domain_votes.keys().collect();
        let at_domain = |id: &PersonId| {
            self.domains
                .get(id)
                .is_some_and(|ds| ds.iter().any(|d| domains.contains(d)))
        };
        for (id, p) in &self.people {
            let no_company = p.company.as_deref().is_none_or(|c| c.trim().is_empty());
            if no_company && at_domain(id) {
                current.insert(*id);
            }
        }

        // Step 2: alumni.
        let mut alumni: BTreeMap<PersonId, (Option<String>, &'static str)> = BTreeMap::new();
        for (id, _) in self.people.iter().filter(|(id, _)| !current.contains(id)) {
            // LinkedIn history: matched in an export, elsewhere later (or now).
            let mut last_match: Option<usize> = None;
            for (i, (_, rows)) in self.history.iter().enumerate() {
                if rows
                    .get(id)
                    .is_some_and(|r| self.company_matches(&t, r.company.as_deref()))
                {
                    last_match = Some(i);
                }
            }
            if let Some(i) = last_match {
                let left = self.history[i + 1..]
                    .iter()
                    .find(|(_, rows)| {
                        rows.get(id)
                            .is_some_and(|r| !self.company_matches(&t, r.company.as_deref()))
                    })
                    .map(|(e, _)| month(e.imported_at));
                alumni.insert(*id, (left, "linkedin_csv"));
                continue;
            }
            if self
                .past
                .get(id)
                .is_some_and(|cs| cs.iter().any(|c| t.matches(c)))
            {
                alumni.insert(*id, (None, "enrich"));
                continue;
            }
            if at_domain(id) {
                alumni.insert(*id, (None, "email_domain"));
            }
        }

        // Step 3: one hop.
        let mut hops: BTreeMap<PersonId, (PersonId, f64)> = BTreeMap::new();
        if one_hop {
            let inside: BTreeSet<PersonId> = current.iter().chain(alumni.keys()).copied().collect();
            let mut st = conn.prepare_cached(
                "SELECT a, b, weight FROM tie WHERE n_shared >= ?1 ORDER BY a, b, kind",
            )?;
            for row in st.query_map([MIN_SHARED], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, f64>(2)?,
                ))
            })? {
                let (a, b, w) = row?;
                let (m, via) = match (inside.contains(&a), inside.contains(&b)) {
                    (true, false) => (b, a),
                    (false, true) => (a, b),
                    _ => continue,
                };
                let Some(p) = self.people.get(&m) else {
                    continue;
                };
                let strength = p.score * w;
                if strength <= 0.0 {
                    continue;
                }
                let e = hops.entry(m).or_insert((via, strength));
                if strength > e.1 || (strength == e.1 && via < e.0) {
                    *e = (via, strength);
                }
            }
        }

        // Step 4: rows.
        let score = |id: &PersonId| self.people.get(id).map_or(0.0, |p| p.score);
        let by_score = |a: &PersonId, b: &PersonId| score(b).total_cmp(&score(a)).then(a.cmp(b));
        let mut cur: Vec<PersonId> = current.iter().copied().collect();
        cur.sort_by(by_score);
        let mut alu: Vec<PersonId> = alumni.keys().copied().collect();
        alu.sort_by(by_score);
        let mut hop: Vec<(PersonId, PersonId, f64)> =
            hops.iter().map(|(m, (v, s))| (*m, *v, *s)).collect();
        hop.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));

        let counts = Counts {
            current: cur.len(),
            alumni: alu.len(),
            one_hop: hop.len(),
        };
        let coverage = if cur.iter().any(|id| score(id) >= WARM_SCORE) {
            "warm"
        } else if cur.is_empty() && alu.is_empty() && hop.is_empty() {
            "none"
        } else {
            "known"
        };
        let (best, best_score) = match (cur.first(), alu.first(), hop.first()) {
            (Some(id), _, _) | (None, Some(id), _) => (Some(*id), Some(round3(score(id)))),
            (None, None, Some((m, _, s))) => (Some(*m), Some(round3(*s))),
            _ => (None, None),
        };
        let mut spellings: BTreeMap<&str, usize> = BTreeMap::new();
        for id in &cur {
            if let Some(c) = self.people[id].company.as_deref().filter(|c| t.matches(c)) {
                *spellings.entry(c.trim()).or_insert(0) += 1;
            }
        }
        let company = spellings
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map_or_else(|| t.input.clone(), |(c, _)| (*c).to_string());
        let domain = domain_votes
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(d, _)| d.clone());
        Ok(CompanyRow {
            company,
            domain,
            current: cur
                .iter()
                .take(limit)
                .map(|id| {
                    let p = &self.people[id];
                    Current {
                        id: *id,
                        display_name: p.name.clone(),
                        title: p.title.clone(),
                        company: p.company.clone(),
                        score: round3(p.score),
                    }
                })
                .collect(),
            alumni: alu
                .iter()
                .take(limit)
                .map(|id| {
                    let p = &self.people[id];
                    let (left, seen_in) = alumni[id].clone();
                    Alumnus {
                        id: *id,
                        display_name: p.name.clone(),
                        company: p.company.clone(),
                        left_seen: left,
                        seen_in,
                        score: round3(p.score),
                    }
                })
                .collect(),
            one_hop: hop
                .iter()
                .take(limit)
                .map(|(m, via, s)| OneHop {
                    id: *m,
                    display_name: self.people[m].name.clone(),
                    via: *via,
                    strength: round3(*s),
                    score: round3(self.people[m].score),
                })
                .collect(),
            counts,
            coverage,
            best,
            best_score,
        })
    }
}
