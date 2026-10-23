//! Human-readable output (the default when `--json` is not given).
//! Plain columns, no color, no truncation, so piping keeps every value.

use std::collections::BTreeMap;
use std::path::Path;

use openstance_core::config::{Config, Origin};
use openstance_core::json::{
    ymd, Evidence, FlaggedMerge, ImportResponse, ReviewQueue, SearchMode, SearchResponse,
};
use openstance_core::overrides::OverrideRow;
use openstance_core::stats::GraphCounts;
use openstance_core::Query;

fn day(iso: &Option<String>) -> String {
    iso.as_deref()
        .map(|s| s.chars().take(10).collect())
        .unwrap_or_else(|| "-".to_string())
}

fn evidence_line(e: &Evidence) -> Option<&str> {
    match e {
        Evidence::Interaction { line, .. } => Some(line),
        _ => None,
    }
}

fn title_company(title: &Option<String>, company: &Option<String>) -> String {
    match (title, company) {
        (Some(t), Some(c)) => format!("{t}, {c}"),
        (Some(t), None) => t.clone(),
        (None, Some(c)) => c.clone(),
        (None, None) => String::new(),
    }
}

fn list(v: &[String]) -> String {
    format!("[{}]", v.join(", "))
}

fn date(ts: i64) -> String {
    ymd(ts)
}

/// `title=[founder] met_in_person=true since=2025-01-01`.
pub fn query_summary(q: &Query) -> String {
    let mut parts = Vec::new();
    if let Some(t) = &q.text {
        parts.push(format!("text={t:?}"));
    }
    for (name, v) in [
        ("company", &q.company),
        ("title", &q.title),
        ("location", &q.location),
        ("industry", &q.industry),
        ("tags", &q.tags),
    ] {
        if !v.is_empty() {
            parts.push(format!("{name}={}", list(v)));
        }
    }
    for (name, v) in [
        ("since", q.since),
        ("until", q.until),
        ("last_contact_before", q.last_contact_before),
    ] {
        if let Some(t) = v {
            parts.push(format!("{name}={}", date(t)));
        }
    }
    for (name, v) in [
        ("met_in_person", q.met_in_person),
        ("replied", q.replied),
        ("lost_touch", q.lost_touch),
    ] {
        if let Some(b) = v {
            parts.push(format!("{name}={b}"));
        }
    }
    if let Some(m) = q.min_score {
        parts.push(format!("min_score={m}"));
    }
    if q.include_bulk {
        parts.push("include_bulk=true".into());
    }
    parts.push(format!("limit={}", q.limit));
    parts.join(" ")
}

/// The search result table, then filters and the rerun command.
pub fn search_table(r: &SearchResponse, explain: bool) {
    if explain {
        println!(
            "query: {}  (parser: {})",
            query_summary(&r.query),
            r.query_source.parser
        );
    }
    if r.results.is_empty() {
        println!("no results");
        for s in &r.suggestions {
            println!("  try: {s}");
        }
    } else {
        let name_w = r
            .results
            .iter()
            .map(|x| {
                x.person
                    .display_name
                    .as_deref()
                    .unwrap_or("-")
                    .chars()
                    .count()
            })
            .max()
            .unwrap_or(6)
            .max(6);
        let tc_w = r
            .results
            .iter()
            .map(|x| {
                title_company(&x.person.title, &x.person.company)
                    .chars()
                    .count()
            })
            .max()
            .unwrap_or(15)
            .max(15);
        println!(
            "{:>3}  {:<name_w$}  {:<tc_w$}  {:>5}  last contact",
            "#", "person", "title / company", "score"
        );
        for x in &r.results {
            let p = &x.person;
            println!(
                "{:>3}  {:<name_w$}  {:<tc_w$}  {:>5.2}  {}",
                x.rank,
                p.display_name.as_deref().unwrap_or("-"),
                title_company(&p.title, &p.company),
                p.score,
                day(&p.last_contact)
            );
            for e in &x.evidence {
                if let Some(l) = evidence_line(e) {
                    println!("       {l}");
                }
            }
            if explain {
                if let Some(rel) = &x.relevance {
                    println!(
                        "       relevance: fts {:.3}  fused {:.3}  final {:.3}",
                        rel.fts, rel.fused, rel.final_
                    );
                }
                let matched: Vec<String> = x
                    .matched
                    .iter()
                    .map(|m| match (&m.matched, &m.via) {
                        (Some(t), Some(v)) => format!("{}:{t} ({v})", m.field),
                        (Some(t), None) => format!("{}:{t}", m.field),
                        (None, Some(v)) => format!("{} ({v})", m.field),
                        (None, None) => m.field.clone(),
                    })
                    .collect();
                if !matched.is_empty() {
                    println!("       matched: {}", matched.join(", "));
                }
            }
        }
    }
    let noun = if r.total == 1 { "result" } else { "results" };
    let mode = match r.mode {
        SearchMode::Structured => "by score",
        SearchMode::Text => "by relevance",
    };
    println!(
        "{} {noun} ({} shown, {mode}) in {} ms.",
        r.total,
        r.results.len(),
        r.took_ms
    );
    if let Some(t) = &r.timings_ms {
        println!(
            "timings: parse {} ms, filter {} ms, candidates {} ms, fuse {} ms, rerank {} ms, evidence {} ms",
            t.parse, t.filter, t.candidates, t.fuse, t.rerank, t.evidence
        );
    }
    println!(
        "filters: {}  (parser: {})",
        query_summary(&r.query),
        r.query_source.parser
    );
    println!("rerun:   {}", r.rerun);
}

/// Per-file import lines plus graph totals.
/// A `kind:value` self handle as shown to the user (emails bare).
fn handle_text(h: &str) -> &str {
    h.strip_prefix("email:").unwrap_or(h)
}

pub fn import_summary(r: &ImportResponse, c: &GraphCounts, preset: &str, config_file: &Path) {
    let has_self = c.persons > c.persons_excluding_self;
    for s in &r.sources {
        let name = std::path::Path::new(&s.path)
            .file_name()
            .map_or_else(|| s.path.clone(), |n| n.to_string_lossy().into_owned());
        if s.skipped_duplicate_file {
            println!(
                "{:<13} {name}  already imported as source #{}",
                s.kind, s.id
            );
            continue;
        }
        let st = &s.stats;
        let rate = if st.elapsed_ms > 0 {
            format!(
                "  {:.0} records/s",
                st.records_read as f64 * 1000.0 / st.elapsed_ms as f64
            )
        } else {
            String::new()
        };
        let skipped = if st.skipped.is_empty() {
            String::new()
        } else {
            let parts: Vec<String> = st.skipped.iter().map(|(k, v)| format!("{k} {v}")).collect();
            format!("  skipped: {}", parts.join(", "))
        };
        println!(
            "{:<13} {name}  {} records  {} identities  {} new interactions  {} ms{rate}{skipped}",
            s.kind, st.records_read, s.identities, s.interactions, st.elapsed_ms
        );
        if !s.self_adopted.is_empty() {
            let handles: Vec<&str> = s.self_adopted.iter().map(|h| handle_text(h)).collect();
            println!(
                "self          treating {} as you; to change it, set [me] identities in {}",
                handles.join(", "),
                config_file.display()
            );
        } else if !has_self && !st.self_candidates.is_empty() {
            let handles: Vec<&str> = st.self_candidates.iter().map(|h| handle_text(h)).collect();
            println!(
                "self          is this you? {} (not adopted: not unambiguous); add it to [me] identities in {}",
                handles.join(", "),
                config_file.display()
            );
        }
    }
    println!(
        "graph         {} people, {} identities, {} interactions",
        c.persons_excluding_self, c.identities, c.interactions
    );
    let res = &r.resolution;
    let review = if res.flagged > 0 {
        " -> openstance review"
    } else {
        ""
    };
    println!(
        "resolution    {} auto-merged, {} flagged{review}",
        res.auto_merged, res.flagged
    );
    println!(
        "scoring       preset {preset}, {} bulk senders excluded",
        res.bulk
    );
    println!("took          {} ms", r.took_ms);
}

fn idents(v: &[openstance_core::json::Identity]) -> String {
    v.iter()
        .map(|i| format!("{}:{}", i.kind, i.value))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One flagged merge, as shown by interactive review.
pub fn flagged_block(n: usize, total: usize, f: &FlaggedMerge) -> String {
    format!(
        "[{n}/{total}] FLAGGED  {}  {:.2}\n  person:  {} (#{})\n  flagged: {}\n  rest:    {}\n  why:     {}",
        f.rule,
        f.confidence,
        f.person.display_name.as_deref().unwrap_or("-"),
        f.person.id,
        idents(&f.flagged_identities),
        idents(&f.other_identities),
        f.why
    )
}

/// `review --list`.
pub fn review_list(q: &ReviewQueue) {
    if q.flagged.is_empty() && q.suggestions.is_empty() {
        println!("nothing to review");
    }
    if !q.flagged.is_empty() {
        println!("flagged merges ({}):", q.flagged.len());
        println!(" conf  rule          person");
        for f in &q.flagged {
            println!(
                " {:.2}  {:<12}  {} (#{})  {}  +  {}",
                f.confidence,
                f.rule,
                f.person.display_name.as_deref().unwrap_or("-"),
                f.person.id,
                idents(&f.other_identities),
                idents(&f.flagged_identities)
            );
            println!("       why: {}", f.why);
        }
    }
    if !q.suggestions.is_empty() {
        println!(
            "suggestions ({}), never merged automatically:",
            q.suggestions.len()
        );
        for s in &q.suggestions {
            println!(
                " {:.2}  {:<12}  {} (#{})  ~  {} (#{})",
                s.confidence,
                s.rule,
                s.a.display_name.as_deref().unwrap_or("-"),
                s.a.id,
                s.b.display_name.as_deref().unwrap_or("-"),
                s.b.id
            );
            println!("       why: {}", s.why);
        }
    }
    for i in &q.info {
        println!("info: {}", i.message);
    }
    if !q.flagged.is_empty() {
        println!(
            "confirm: openstance review --confirm <id>   split: openstance review --split <id>"
        );
    }
}

/// `edit --history`.
pub fn history(person: i64, rows: &[OverrideRow]) {
    if rows.is_empty() {
        println!("person {person} has no edits");
        return;
    }
    for r in rows {
        println!(
            "{}  #{:<5} {:<13} {}  (on identity {})",
            day(&Some(openstance_core::json::iso(r.set_at))),
            r.id,
            r.field.as_str(),
            r.value.as_deref().unwrap_or("(unset)"),
            r.person_id
        );
    }
}

/// The effective config as TOML, each key annotated with its origin.
pub fn annotated_config(cfg: &Config, origins: &BTreeMap<String, Origin>) -> String {
    let text = cfg.to_toml();
    let mut section = String::new();
    let mut out = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = t.trim_matches(|c| c == '[' || c == ']').to_string();
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if let Some((k, _)) = t.split_once(" = ") {
            let key = if section.is_empty() {
                k.trim().to_string()
            } else {
                format!("{section}.{}", k.trim())
            };
            let origin = origins.get(&key).copied().unwrap_or(Origin::Default);
            let o = match origin {
                Origin::Default => "default",
                Origin::File => "file",
                Origin::Env => "env",
                Origin::Flag => "flag",
            };
            out.push_str(&format!("{line}  # {o}\n"));
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
