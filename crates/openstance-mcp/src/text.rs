//! The plain-text `content` block of every tool result: a short rendering of
//! the structured JSON for clients that ignore `structuredContent`.
//!
//! Rendered from the JSON value (not the Rust types) so the summary always
//! describes exactly what was sent. Evidence lines are copied verbatim:
//! agents are told to quote them, not paraphrase them.

use std::fmt::Write as _;

use openstance_core::json::{ErrorCode, ErrorResponse};
use serde_json::Value;

/// Results listed in the search summary (the JSON has all of them).
const MAX_RESULTS: usize = 10;
/// Evidence lines per result / path.
const MAX_LINES: usize = 3;

fn s(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.is_empty())
}

fn day(v: &Value) -> Option<&str> {
    s(v).map(|t| t.get(..10).unwrap_or(t))
}

fn num(v: &Value) -> String {
    v.as_f64().map_or_else(|| "-".into(), |x| format!("{x:.2}"))
}

fn name(p: &Value) -> String {
    s(&p["display_name"]).map_or_else(
        || format!("person {}", p["id"]),
        std::string::ToString::to_string,
    )
}

/// `Name - Title, Company - Location`, skipping what is unknown.
fn headline(p: &Value) -> String {
    let mut out = name(p);
    let role: Vec<&str> = [s(&p["title"]), s(&p["company"])]
        .into_iter()
        .flatten()
        .collect();
    if !role.is_empty() {
        let _ = write!(out, " - {}", role.join(", "));
    }
    if let Some(l) = s(&p["location"]) {
        let _ = write!(out, " - {l}");
    }
    out
}

/// One evidence item as a single line (the core `line` when there is one).
fn evidence_line(e: &Value) -> Option<String> {
    if let Some(l) = s(&e["line"]) {
        return Some(l.to_string());
    }
    match s(&e["kind"])? {
        "tie" => Some(format!(
            "{} tie with {}: {} shared, weight {}, last {}",
            s(&e["tie_kind"]).unwrap_or("?"),
            name(&e["with"]),
            e["n_shared"],
            num(&e["weight"]),
            day(&e["last_at"]).unwrap_or("?"),
        )),
        "field" => Some(format!(
            "{} = \"{}\" (matched \"{}\")",
            s(&e["field"]).unwrap_or("?"),
            s(&e["value"]).unwrap_or(""),
            s(&e["matched"]).unwrap_or(""),
        )),
        "enrich" => Some(format!(
            "{} {} = \"{}\" ({}, {})",
            s(&e["op"]).unwrap_or("?"),
            s(&e["field"]).unwrap_or("?"),
            s(&e["value"]).unwrap_or(""),
            s(&e["backend"]).unwrap_or("?"),
            day(&e["at"]).unwrap_or("?"),
        )),
        _ => None,
    }
}

fn lines(out: &mut String, items: &Value, max: usize) {
    for line in items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(evidence_line)
        .take(max)
    {
        let _ = writeln!(out, "     {line}");
    }
}

fn list(out: &mut String, label: &str, items: &Value) {
    let items: Vec<&str> = items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(s)
        .collect();
    if !items.is_empty() {
        let _ = writeln!(out, "{label}: {}", items.join("; "));
    }
}

/// `search_people`.
pub fn search(v: &Value) -> String {
    let mut out = String::new();
    let results = v["results"].as_array().map_or(&[][..], Vec::as_slice);
    let total = v["total"].as_u64().unwrap_or(results.len() as u64);
    if results.is_empty() {
        out.push_str("No matching people in the graph.\n");
    } else {
        let _ = writeln!(
            out,
            "{} of {total} matching people (most relevant first):",
            results.len()
        );
    }
    for r in results.iter().take(MAX_RESULTS) {
        let p = &r["person"];
        let _ = write!(out, "{:>2}. {} [id {}]", r["rank"], headline(p), p["id"]);
        let _ = write!(out, " score {}", num(&p["score"]));
        if let Some(d) = day(&p["last_contact"]) {
            let _ = write!(out, ", last contact {d}");
        }
        out.push('\n');
        lines(&mut out, &r["evidence"], MAX_LINES);
    }
    if results.len() > MAX_RESULTS {
        let _ = writeln!(
            out,
            "... {} more in the structured result.",
            results.len() - MAX_RESULTS
        );
    }
    list(&mut out, "warnings", &v["warnings"]);
    list(&mut out, "to widen", &v["suggestions"]);
    if let Some(r) = s(&v["rerun"]) {
        let _ = writeln!(out, "same query from the CLI: {r}");
    }
    out
}

/// `get_person`.
pub fn person(v: &Value) -> String {
    let mut out = String::new();
    let p = &v["person"];
    let c = &p["components"];
    let _ = writeln!(out, "{} [id {}]", headline(p), p["id"]);
    let _ = writeln!(
        out,
        "score {} (preset {}): recency {}, frequency {}, reciprocity {}, meetings {}, breadth {}",
        num(&p["score"]),
        s(&c["preset"]).unwrap_or("?"),
        num(&c["recency"]),
        num(&c["frequency"]),
        num(&c["reciprocity"]),
        num(&c["meetings"]),
        num(&c["breadth"]),
    );
    let mut flags = Vec::new();
    for (key, label) in [
        ("is_bulk", "bulk sender"),
        ("do_not_share", "do not share"),
        ("no_enrich", "no enrichment"),
    ] {
        if p[key] == Value::Bool(true) {
            flags.push(label);
        }
    }
    if !flags.is_empty() {
        let _ = writeln!(out, "flags: {}", flags.join(", "));
    }
    let ids: Vec<String> = v["identities"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            format!(
                "{} {}",
                s(&i["kind"]).unwrap_or("?"),
                s(&i["value"]).unwrap_or("?")
            )
        })
        .collect();
    if !ids.is_empty() {
        let _ = writeln!(out, "identities: {}", ids.join("; "));
    }
    if v["recent_interactions"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
    {
        out.push_str("recent:\n");
        lines(&mut out, &v["recent_interactions"], usize::MAX);
    }
    let mutuals: Vec<String> = v["mutuals"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            format!(
                "{} [id {}] ({} shared, {})",
                name(&m["person"]),
                m["person"]["id"],
                m["n_shared"],
                s(&m["tie_kind"]).unwrap_or("?")
            )
        })
        .collect();
    if !mutuals.is_empty() {
        let _ = writeln!(out, "mutuals: {}", mutuals.join("; "));
    }
    let notes: Vec<&str> = v["notes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| s(&n["value"]))
        .collect();
    if !notes.is_empty() {
        let _ = writeln!(out, "notes: {}", notes.join("; "));
    }
    out
}

/// `how_connected`.
pub fn how(v: &Value) -> String {
    let mut out = String::new();
    if v["found"] == Value::Bool(false) {
        let _ = writeln!(
            out,
            "The target is not in the user's graph. {}",
            s(&v["hint"]).unwrap_or("")
        );
        return out;
    }
    let t = &v["target"];
    let _ = writeln!(out, "target: {} [id {}]", headline(t), t["id"]);
    match v["direct"].as_object() {
        Some(_) => {
            let d = &v["direct"];
            let _ = write!(
                out,
                "direct: the user knows them (score {}",
                num(&d["score"])
            );
            if let Some(day) = day(&d["last_contact"]) {
                let _ = write!(out, ", last contact {day}");
            }
            out.push_str(")\n");
        }
        None => out.push_str("direct: no interaction with the target\n"),
    }
    let paths = v["paths"].as_array().map_or(&[][..], Vec::as_slice);
    if paths.is_empty() {
        out.push_str("No mutual connection has a tie to the target.\n");
    }
    for (i, p) in paths.iter().enumerate() {
        let _ = writeln!(
            out,
            "{:>2}. via {} [id {}]: strength {} = {} x {} ({})",
            i + 1,
            headline(&p["via"]),
            p["via"]["id"],
            num(&p["strength"]),
            num(&p["my_score"]),
            num(&p["link_weight"]),
            s(&p["reason"]).unwrap_or("?"),
        );
        lines(&mut out, &p["evidence"], MAX_LINES + 2);
    }
    out
}

/// An error envelope.
pub fn error(resp: &ErrorResponse) -> String {
    let e = &resp.error;
    let mut out = format!("error ({}): {}\n", code_str(e.code), e.message);
    if !e.candidates.is_empty() {
        out.push_str("candidates:\n");
        for c in &e.candidates {
            let _ = writeln!(out, "  id {}  {}", c.id, c.display_name);
        }
        out.push_str("Ask the user which one they mean, then call again with the id.\n");
    }
    out
}

fn code_str(c: ErrorCode) -> String {
    serde_json::to_value(c)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{c:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn search_summary_quotes_lines_and_handles_empty() {
        let v = json!({
            "total": 1, "rerun": "openstance search devtools --no-parse",
            "warnings": [], "suggestions": ["drop met_in_person -> 3 results"],
            "results": [{ "rank": 1,
                "person": { "id": 7, "display_name": "Ravi Iyer", "title": null, "company": null,
                            "location": null, "score": 0.4, "last_contact": "2026-09-14T16:02:00Z" },
                "evidence": [ { "kind": "interaction", "line": "2026-09-14  meeting  \"devtools dinner\"  (cal)" } ] }]
        });
        let t = search(&v);
        assert!(
            t.contains("Ravi Iyer [id 7] score 0.40, last contact 2026-09-14"),
            "{t}"
        );
        assert!(t.contains("\"devtools dinner\""), "{t}");
        assert!(t.contains("drop met_in_person"), "{t}");
        assert!(search(&json!({"results": [], "total": 0})).starts_with("No matching"));
    }

    #[test]
    fn how_summary_covers_not_found_and_paths() {
        let t = how(&json!({"v": 1, "found": false, "hint": "try later"}));
        assert!(t.contains("not in the user's graph") && t.contains("try later"));
        let t = how(&json!({
            "target": {"id": 5, "display_name": "Sam Ito"}, "direct": null,
            "paths": [{"via": {"id": 2, "display_name": "Priya Raman"}, "strength": 0.4,
                       "my_score": 0.8, "link_weight": 0.5, "reason": "tie",
                       "evidence": [{"kind": "tie", "with": {"id": 5, "display_name": "Sam Ito"},
                                     "tie_kind": "email", "n_shared": 3, "weight": 0.5,
                                     "last_at": "2026-01-02T00:00:00Z"}]}]
        }));
        assert!(
            t.contains("via Priya Raman [id 2]: strength 0.40 = 0.80 x 0.50 (tie)"),
            "{t}"
        );
        assert!(t.contains("email tie with Sam Ito: 3 shared"), "{t}");
        assert!(t.contains("no interaction with the target"), "{t}");
    }
}
