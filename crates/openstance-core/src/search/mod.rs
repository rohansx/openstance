//! Search: parse -> structured filter (SQL) -> FTS5 candidates -> fusion ->
//! optional rerank -> evidence. One function serves the CLI, MCP and HTTP.
//! Spec: `docs/tech-spec/search.md` (P1-CORE-14..17, P1-MODEL-02).
//!
//! Owned by the "search" stream. Module map:
//! - [`rules`]: `RuleParser` (the default `QueryParser`) and its lexicon;
//! - [`filters`]: structured filters to SQL with bound parameters only;
//! - [`fts`]: `person_fts` maintenance, `escape_query`, bm25 candidates;
//! - [`fuse`]: min-max normalization and the score lift;
//! - [`evidence`]: recent interactions, evidence lines, matched fields;
//! - [`person`]: the `Person` / `PersonDetail` JSON builders (`who`).

use std::collections::BTreeMap;
use std::time::Instant;

use chrono::TimeZone;
use openstance_model::{Candidate, CandidateProfile, Models, Query, QueryPatch, DEFAULT_LIMIT};
use rusqlite::Connection;

use crate::config::Config;
use crate::error::Result;
use crate::json::{
    round3, Matched, ModelsUsed, QuerySource, Relevance, SearchMode, SearchResponse, SearchResult,
    Timings, V,
};
use crate::PersonId;

pub mod evidence;
pub mod filters;
pub mod fts;
pub mod fuse;
pub mod person;
pub mod rules;

/// Inputs to [`search`] and their provenance.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchRequest {
    /// Raw user text, parsed in step 1 unless `no_parse`.
    pub text: Option<String>,
    /// Explicit flags / JSON filters; always win over parsed values.
    pub filters: QueryPatch,
    /// Skip step 1: text goes straight to `Query.text`.
    pub no_parse: bool,
    /// Include timings (`--explain`).
    pub explain: bool,
    /// Unix seconds at local midnight, for relative dates.
    pub today: i64,
    /// Unix seconds, for evidence (`at <= now`).
    pub now: i64,
}

/// The timezone for local dates: `[import] timezone`, else UTC.
pub fn timezone(cfg: &Config) -> chrono_tz::Tz {
    cfg.import.timezone.trim().parse().unwrap_or(chrono_tz::UTC)
}

/// Unix seconds at local midnight of the day containing `now`.
pub fn today_start(now: i64, tz: chrono_tz::Tz) -> i64 {
    let date = tz
        .timestamp_opt(now, 0)
        .single()
        .map(|t| t.date_naive())
        .unwrap_or_default();
    let midnight = date.and_time(chrono::NaiveTime::MIN);
    tz.from_local_datetime(&midnight)
        .earliest()
        .map_or(now - now.rem_euclid(86_400), |t| t.timestamp())
}

fn ms(t: Instant) -> u64 {
    u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Step 1: parse, then lay the explicit filters over the result.
fn parse_step(
    conn: &Connection,
    cfg: &Config,
    models: &Models,
    req: &SearchRequest,
    warnings: &mut Vec<String>,
) -> Result<(Query, QuerySource)> {
    let text = req.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let mut source = QuerySource {
        parser: "rules".into(),
        fallback: None,
        explicit: Vec::new(),
    };
    let mut q = match text {
        None => Query::default(),
        Some(t) if req.no_parse => {
            source.parser = "passthrough".into();
            Query::text(t)
        }
        Some(t) => {
            let rules = || -> Result<rules::Parsed> {
                let p = rules::RuleParser::new(rules::Lexicon::load(conn)?, timezone(cfg));
                Ok(p.parse_detailed(t, req.today))
            };
            let use_model = cfg.model.parser != "rules" && models.parser.id() != "passthrough";
            let model_out = use_model.then(|| models.parser.parse(t, req.today));
            match model_out {
                Some(Ok(q)) => {
                    source.parser = models.parser.id().to_string();
                    q
                }
                Some(Err(e)) => {
                    let reason = e.to_string();
                    warnings.push(format!("parser fallback: {reason}"));
                    source.fallback = Some(reason);
                    let p = rules()?;
                    warnings.extend(p.warnings);
                    p.query
                }
                None => {
                    let p = rules()?;
                    warnings.extend(p.warnings);
                    p.query
                }
            }
        }
    };
    if q.limit == DEFAULT_LIMIT {
        q.limit = usize::try_from(cfg.search.default_limit).unwrap_or(DEFAULT_LIMIT);
    }
    if cfg.search.include_bulk {
        q.include_bulk = true;
    }
    source.explicit = req
        .filters
        .apply(&mut q)
        .into_iter()
        .map(str::to_string)
        .collect();
    q.limit = q.effective_limit();
    Ok((q, source))
}

/// FTS text: the query text plus industry terms (industry is soft).
fn fts_text(q: &Query) -> Option<String> {
    let text = q.text.as_deref()?.trim();
    let mut parts = vec![text.to_string()];
    parts.extend(rules::text_alias_expansions(text));
    parts.extend(q.industry.iter().cloned());
    Some(parts.join(" "))
}

fn scores(conn: &Connection, ids: &[PersonId]) -> Result<BTreeMap<PersonId, f64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, score FROM person WHERE id IN (SELECT value FROM json_each(?1))",
    )?;
    let m = stmt
        .query_map([fts::json_ids(ids)], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    Ok(m)
}

/// Text candidates fused and filtered; `None` when there is no text.
fn text_candidates(
    conn: &Connection,
    cfg: &Config,
    q: &Query,
    allowed: &[PersonId],
) -> Result<Option<Vec<fuse::Fused>>> {
    let Some(text) = fts_text(q) else {
        return Ok(None);
    };
    let restrict = q.has_structured_filters().then_some(allowed);
    let limit = usize::try_from(cfg.search.candidates_fts).unwrap_or(200);
    let hits = fts::candidates(conn, &text, restrict, limit)?;
    let ids: Vec<PersonId> = hits.iter().map(|h| h.0).collect();
    let sc = scores(conn, &ids)?;
    let fused = fuse::fuse(
        &hits,
        None,
        cfg.search.text_weight,
        cfg.search.relationship_lift,
        &|id| sc.get(&id).copied().unwrap_or(0.0),
    );
    Ok(Some(fused))
}

/// Keep non-zero fused hits; zero ones only to fill up to `limit`.
fn drop_zero(fused: Vec<fuse::Fused>, limit: usize) -> Vec<fuse::Fused> {
    let (mut keep, zero): (Vec<_>, Vec<_>) = fused.into_iter().partition(|f| f.final_score > 0.0);
    let room = limit.saturating_sub(keep.len());
    keep.extend(zero.into_iter().take(room));
    keep
}

/// Count of results a query would return (used for suggestions).
fn count(conn: &Connection, cfg: &Config, q: &Query) -> Result<usize> {
    let allowed = filters::allowed_ids(conn, q)?;
    Ok(match text_candidates(conn, cfg, q, &allowed)? {
        Some(f) if !f.is_empty() => f.len(),
        Some(_) if q.has_structured_filters() => allowed.len(),
        Some(_) => 0,
        None => allowed.len(),
    })
}

/// Zero-result hints: each filter that alone would widen the result.
fn suggestions(conn: &Connection, cfg: &Config, q: &Query) -> Result<Vec<String>> {
    type Drop = (&'static str, fn(&mut Query));
    let drops: &[Drop] = &[
        ("company", |q| q.company.clear()),
        ("title", |q| q.title.clear()),
        ("location", |q| q.location.clear()),
        ("tags", |q| q.tags.clear()),
        ("since", |q| q.since = None),
        ("until", |q| q.until = None),
        ("last_contact_before", |q| q.last_contact_before = None),
        ("met_in_person", |q| q.met_in_person = None),
        ("replied", |q| q.replied = None),
        ("min_score", |q| q.min_score = None),
        ("lost_touch", |q| q.lost_touch = None),
    ];
    let mut out = Vec::new();
    for (name, clear) in drops {
        let mut wider = q.clone();
        clear(&mut wider);
        if wider == *q {
            continue;
        }
        let n = count(conn, cfg, &wider)?;
        if n > 0 {
            let noun = if n == 1 { "result" } else { "results" };
            out.push(format!("drop {name} -> {n} {noun}"));
        }
    }
    Ok(out)
}

/// Text-mode matched entries: person columns and FTS `subjects` snippets.
fn text_matches(conn: &Connection, q: &Query, p: &crate::json::Person) -> Result<Vec<Matched>> {
    let Some(text) = fts_text(q) else {
        return Ok(Vec::new());
    };
    let terms = fts::terms(&text);
    // A term matches a word it prefixes (FTS prefix semantics).
    let hits_in = |value: &str| -> Vec<&str> {
        let words: Vec<String> = value
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        let mut out: Vec<&str> = Vec::new();
        for t in &terms {
            let t_words = fts::terms(&t.replace(|c: char| !c.is_alphanumeric(), " "));
            let all = !t_words.is_empty()
                && t_words.iter().all(|tw| {
                    words
                        .iter()
                        .any(|w| w == tw || (tw.len() >= 4 && w.starts_with(tw.as_str())))
                });
            if all && !out.contains(&t.as_str()) {
                out.push(t.as_str());
            }
        }
        out
    };
    let mut out = Vec::new();
    let cols = [
        ("display_name", &p.display_name),
        ("title", &p.title),
        ("company", &p.company),
        ("location", &p.location),
        ("bio", &p.bio),
    ];
    for (name, value) in cols {
        let Some(v) = value else { continue };
        let hit = hits_in(v);
        if !hit.is_empty() {
            out.push(Matched {
                field: name.into(),
                value: Some(serde_json::json!(v)),
                matched: Some(hit.join(" ")),
                ..Matched::default()
            });
        }
    }
    let row: Option<String> = conn
        .prepare_cached("SELECT subjects FROM person_fts WHERE rowid = ?1")?
        .query_row([p.id], |r| r.get(0))
        .ok();
    if let Some(subjects) = row {
        let hit = hits_in(&subjects);
        if !hit.is_empty() {
            let snippet: Option<String> = conn
                .prepare_cached(
                    "SELECT snippet(person_fts, 7, '', '', '…', 10) FROM person_fts
                     WHERE person_fts MATCH ?1 AND rowid = ?2",
                )?
                .query_row(
                    rusqlite::params![fts::escape_query(&hit.join(" ")), p.id],
                    |r| r.get(0),
                )
                .ok();
            out.push(Matched {
                field: "subjects".into(),
                matched: Some(hit.join(" ")),
                snippet,
                ..Matched::default()
            });
        }
    }
    Ok(out)
}

/// Optional step 5. `none` keeps the fused order.
fn rerank(
    conn: &Connection,
    cfg: &Config,
    models: &Models,
    q: &Query,
    fused: Vec<fuse::Fused>,
    warnings: &mut Vec<String>,
) -> Result<(Vec<fuse::Fused>, BTreeMap<PersonId, String>)> {
    let mut reasons = BTreeMap::new();
    if models.reranker.id() == "none" || fused.is_empty() {
        return Ok((fused, reasons));
    }
    let top_n = usize::try_from(cfg.model.rerank_top_n)
        .unwrap_or(50)
        .min(fused.len());
    let (head, tail) = fused.split_at(top_n);
    let ids: Vec<PersonId> = head.iter().map(|f| f.id).collect();
    let people = person::people(conn, cfg, &ids)?;
    let cands: Vec<Candidate> = head
        .iter()
        .zip(&people)
        .map(|(f, p)| Candidate {
            id: f.id,
            profile: CandidateProfile {
                title: p.title.clone(),
                company: p.company.clone(),
                location: p.location.clone(),
                tags: p.tags.clone(),
                display_name: (!models.reranker.is_remote())
                    .then(|| p.display_name.clone())
                    .flatten(),
                subjects: Vec::new(),
            },
            fused: f.final_score as f32,
            score: p.score as f32,
            no_enrich: p.no_enrich,
        })
        .filter(|c| !(models.reranker.is_remote() && c.no_enrich))
        .collect();
    match models.reranker.rerank(q, cands) {
        Ok(ranked) => {
            let mut out = Vec::with_capacity(fused.len());
            for r in &ranked {
                if let Some(f) = head.iter().find(|f| f.id == r.id) {
                    if !out.iter().any(|o: &fuse::Fused| o.id == r.id) {
                        out.push(*f);
                        if let Some(reason) = &r.reason {
                            reasons.insert(r.id, reason.clone());
                        }
                    }
                }
            }
            for f in head {
                if !out.iter().any(|o| o.id == f.id) {
                    out.push(*f);
                }
            }
            out.extend_from_slice(tail);
            Ok((out, reasons))
        }
        Err(e) => {
            warnings.push(format!("rerank skipped: {e}"));
            Ok((fused, BTreeMap::new()))
        }
    }
}

/// Run a search. `models.parser` is used for step 1 unless the config says
/// `rules` (then core's `RuleParser` runs, with a lexicon from the graph).
pub fn search(
    conn: &Connection,
    cfg: &Config,
    models: &Models,
    req: &SearchRequest,
) -> Result<SearchResponse> {
    let started = Instant::now();
    let mut timings = Timings::default();
    let mut warnings = Vec::new();

    let t = Instant::now();
    let (q, query_source) = parse_step(conn, cfg, models, req, &mut warnings)?;
    timings.parse = ms(t);

    let t = Instant::now();
    let allowed = filters::allowed_ids(conn, &q)?;
    timings.filter = ms(t);

    let limit = q.effective_limit();
    if models.embedder.dims() > 0 && q.text.is_some() {
        warnings.push(
            "vector search is not available in this version; using full-text search only".into(),
        );
    }

    let t = Instant::now();
    let candidates = text_candidates(conn, cfg, &q, &allowed)?;
    timings.candidates = ms(t);

    let t = Instant::now();
    let (mode, ranked, total, relevance): (
        SearchMode,
        Vec<PersonId>,
        usize,
        BTreeMap<PersonId, fuse::Fused>,
    ) = match candidates {
        Some(fused) if !fused.is_empty() => {
            let kept = drop_zero(fused, limit);
            let total = kept.len();
            timings.fuse = ms(t);
            let t2 = Instant::now();
            let (order, reasons) = rerank(conn, cfg, models, &q, kept, &mut warnings)?;
            timings.rerank = ms(t2);
            let rel = order.iter().map(|f| (f.id, *f)).collect();
            let ids = order.iter().map(|f| f.id).collect();
            return finish(
                conn,
                cfg,
                models,
                req,
                q,
                query_source,
                SearchMode::Text,
                ids,
                total,
                rel,
                reasons,
                warnings,
                timings,
                started,
            );
        }
        Some(_) if q.has_structured_filters() => {
            warnings.push("no text matches; showing filter matches by score".into());
            let total = allowed.len();
            (SearchMode::Structured, allowed, total, BTreeMap::new())
        }
        Some(_) => (SearchMode::Text, Vec::new(), 0, BTreeMap::new()),
        None => {
            let total = allowed.len();
            (SearchMode::Structured, allowed, total, BTreeMap::new())
        }
    };
    timings.fuse = ms(t);
    finish(
        conn,
        cfg,
        models,
        req,
        q,
        query_source,
        mode,
        ranked,
        total,
        relevance,
        BTreeMap::new(),
        warnings,
        timings,
        started,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish(
    conn: &Connection,
    cfg: &Config,
    models: &Models,
    req: &SearchRequest,
    q: Query,
    query_source: QuerySource,
    mode: SearchMode,
    ranked: Vec<PersonId>,
    total: usize,
    relevance: BTreeMap<PersonId, fuse::Fused>,
    reasons: BTreeMap<PersonId, String>,
    warnings: Vec<String>,
    mut timings: Timings,
    started: Instant,
) -> Result<SearchResponse> {
    let t = Instant::now();
    let limit = q.effective_limit();
    let page: Vec<PersonId> = ranked.into_iter().take(limit).collect();
    let people = person::people(conn, cfg, &page)?;
    let n_evidence = usize::try_from(cfg.search.evidence_per_result).unwrap_or(3);
    let mut ev = evidence::recent(conn, &page, n_evidence, req.now)?;
    let mut results = Vec::with_capacity(people.len());
    for (i, p) in people.into_iter().enumerate() {
        let mut matched = evidence::matched_fields(&q, &p);
        let unmatched: Vec<String> = q
            .company
            .iter()
            .filter(|c| {
                !matched.iter().any(|m| {
                    m.field == "company" && m.matched.as_deref() == Some(&c.to_lowercase())
                })
            })
            .cloned()
            .collect();
        if !unmatched.is_empty() {
            let extra = evidence::company_domain_matches(conn, p.id, &unmatched)?;
            let at = matched
                .iter()
                .position(|m| m.field != "company")
                .unwrap_or(matched.len());
            for (k, m) in extra.into_iter().enumerate() {
                matched.insert(at + k, m);
            }
        }
        for m in matched.iter_mut() {
            if m.field == "met_in_person" && m.value == Some(serde_json::json!(true)) {
                m.via = Some(evidence::meetings_via(conn, p.id, req.now)?);
            }
        }
        matched.extend(text_matches(conn, &q, &p)?);
        let rel = relevance.get(&p.id).map(|f| Relevance {
            fts: round3(f.fts),
            vector: f.vector.map(round3),
            fused: round3(f.relevance),
            final_: round3(f.final_score),
        });
        let reason = reasons.get(&p.id).cloned();
        results.push(SearchResult {
            rank: i + 1,
            evidence: ev.remove(&p.id).unwrap_or_default(),
            reason_source: reason
                .as_ref()
                .map(|_| format!("model:{}", models.reranker.id())),
            reason,
            relevance: rel,
            matched,
            person: p,
        });
    }
    let suggestions = if results.is_empty() && (q.has_structured_filters()) {
        suggestions(conn, cfg, &q)?
    } else {
        Vec::new()
    };
    timings.evidence = ms(t);
    let took_ms = ms(started);
    timings.total = took_ms;
    let tz = timezone(cfg);
    Ok(SearchResponse {
        v: V,
        rerun: rerun_command_in(&q, tz),
        models: ModelsUsed {
            parser: query_source.parser.clone(),
            embedder: models.embedder.id().to_string(),
            reranker: models.reranker.id().to_string(),
        },
        query: q,
        query_source,
        mode,
        total,
        took_ms,
        results,
        warnings,
        suggestions,
        timings_ms: req.explain.then_some(timings),
    })
}

/// Quote a value for a POSIX shell: bare when safe, double quotes when
/// only spaces need protecting, single quotes otherwise.
pub fn shell_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "._@:/+-,=%".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_string();
    }
    if !s.chars().any(|c| "\"$\\`!'".contains(c)) {
        return format!("\"{s}\"");
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn date_arg(ts: i64, tz: chrono_tz::Tz) -> String {
    match tz.timestamp_opt(ts, 0).single() {
        Some(t) if t.time() == chrono::NaiveTime::MIN => t.format("%Y-%m-%d").to_string(),
        _ => format!("@{ts}"),
    }
}

/// The CLI command that reproduces `q` with no model call
/// (`openstance search --company anthropic --lost-touch`). Dates are
/// written in UTC; see [`rerun_command_in`].
pub fn rerun_command(q: &Query) -> String {
    rerun_command_in(q, chrono_tz::UTC)
}

/// [`rerun_command`] with dates written in `tz` (local midnight ->
/// `YYYY-MM-DD`, anything else -> `@unix`).
pub fn rerun_command_in(q: &Query, tz: chrono_tz::Tz) -> String {
    let mut parts = vec!["openstance search".to_string()];
    if let Some(t) = &q.text {
        parts.push(shell_quote(t));
    }
    let multi = [
        ("--company", &q.company),
        ("--title", &q.title),
        ("--location", &q.location),
        ("--industry", &q.industry),
        ("--tag", &q.tags),
    ];
    for (flag, values) in multi {
        for v in values {
            parts.push(format!("{flag} {}", shell_quote(v)));
        }
    }
    for (flag, v) in [
        ("--since", q.since),
        ("--until", q.until),
        ("--before", q.last_contact_before),
    ] {
        if let Some(ts) = v {
            parts.push(format!("{flag} {}", date_arg(ts, tz)));
        }
    }
    for (yes, no, v) in [
        ("--met", "--not-met", q.met_in_person),
        ("--replied", "--not-replied", q.replied),
    ] {
        match v {
            Some(true) => parts.push(yes.into()),
            Some(false) => parts.push(no.into()),
            None => {}
        }
    }
    if let Some(m) = q.min_score {
        parts.push(format!("--min-score {m}"));
    }
    match q.lost_touch {
        Some(true) => parts.push("--lost-touch".into()),
        Some(false) => parts.push("--not-lost-touch".into()),
        None => {}
    }
    if q.include_bulk {
        parts.push("--include-bulk".into());
    }
    if q.limit != DEFAULT_LIMIT {
        parts.push(format!("--limit {}", q.limit));
    }
    if q.text.is_some() {
        parts.push("--no-parse".into());
    }
    parts.join(" ")
}

#[cfg(test)]
pub(crate) mod testkit;

#[cfg(test)]
#[path = "search_tests.rs"]
mod search_tests;
