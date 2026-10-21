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

/// `$0.0100` from micro-USD.
fn usd(v: &Value) -> String {
    let m = v.as_u64().unwrap_or(0);
    format!("${}.{:04}", m / 1_000_000, (m % 1_000_000) / 100)
}

/// The quote part shared by `path_to`, `brief` and `quote_enrich`.
fn quote_summary(out: &mut String, q: &Value) {
    let lines = q["lines"].as_array().map_or(&[][..], Vec::as_slice);
    if lines.is_empty() {
        out.push_str("No paid request is needed; nothing leaves this machine.\n");
    } else {
        let _ = writeln!(
            out,
            "quote: {} request{} for {} via {} (budget {}{})",
            lines.len(),
            if lines.len() == 1 { "" } else { "s" },
            usd(&q["total_micro_usd"]),
            s(&q["backend"]).unwrap_or("?"),
            usd(&q["budget_micro_usd"]),
            if q["within_budget"] == Value::Bool(false) {
                ", over budget"
            } else {
                ""
            },
        );
        for l in lines {
            let req = &l["job"]["request"];
            let sent: Vec<String> = req
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, v)| *k != "kind" && !v.is_null())
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or(&v.to_string())))
                .collect();
            let _ = writeln!(
                out,
                "  - {} {} (sends {})",
                s(&req["kind"]).unwrap_or("?"),
                usd(&l["micro_usd"]),
                if sent.is_empty() {
                    "nothing".to_string()
                } else {
                    sent.join(", ")
                }
            );
        }
    }
    for sk in q["skipped"].as_array().into_iter().flatten() {
        let _ = writeln!(
            out,
            "  skipped {}{}: {}",
            s(&sk["op"]).unwrap_or("?"),
            sk["person_id"]
                .as_i64()
                .map_or(String::new(), |p| format!(" for person {p}")),
            s(&sk["reason"]).unwrap_or("?"),
        );
    }
}

/// Status, quote, receipt and notes of an `Outcome`.
fn outcome_head(out: &mut String, v: &Value) {
    let _ = writeln!(
        out,
        "{}: {}",
        s(&v["playbook"]).unwrap_or("playbook"),
        s(&v["status"]).unwrap_or("?")
    );
    quote_summary(out, &v["plan"]["quote"]);
    let r = &v["receipt"];
    if r["requests_sent"].as_u64().unwrap_or(0) > 0 {
        let _ = writeln!(
            out,
            "spent {} on {} request(s) (run {})",
            usd(&r["spent_micro_usd"]),
            r["requests_sent"],
            s(&r["run_id"]).unwrap_or("?")
        );
    }
    for n in v["notes"].as_array().into_iter().flatten() {
        if let Some(n) = s(n) {
            let _ = writeln!(out, "note: {n}");
        }
    }
    if v["status"] == "quote" {
        out.push_str(
            "Nothing was sent. Show the user this quote; call again with allow_egress=true only if they agree.\n",
        );
    }
}

/// `path_to`.
pub fn path_to(v: &Value) -> String {
    let mut out = String::new();
    outcome_head(&mut out, v);
    let r = &v["result"];
    if r.is_null() {
        return out;
    }
    let t = &r["target"];
    let _ = writeln!(
        out,
        "target: {}{}",
        headline(t),
        if t["in_graph"] == Value::Bool(true) {
            " (in the graph)"
        } else {
            " (not in the graph)"
        }
    );
    let paths = r["paths"].as_array().map_or(&[][..], Vec::as_slice);
    if paths.is_empty() {
        out.push_str("No path found through the user's graph.\n");
    }
    for (i, p) in paths.iter().enumerate() {
        let _ = writeln!(
            out,
            "{:>2}. via {} [id {}]: strength {} ({})",
            i + 1,
            headline(&p["via"]),
            p["via"]["id"],
            num(&p["strength"]),
            s(&p["reason"]).unwrap_or("?"),
        );
        lines(&mut out, &p["evidence"], MAX_LINES);
    }
    if let Some(e) = s(&r["email"]["address"]) {
        let _ = writeln!(
            out,
            "email: {e} ({})",
            s(&r["email"]["status"]).unwrap_or("unverified")
        );
    }
    out
}

/// `brief`: the markdown itself, after the status lines.
pub fn brief(v: &Value) -> String {
    let mut out = String::new();
    outcome_head(&mut out, v);
    if let Some(md) = s(&v["result"]["markdown"]) {
        out.push('\n');
        out.push_str(md);
    }
    out
}

/// `quote_enrich`.
pub fn quote(v: &Value) -> String {
    let mut out = String::new();
    quote_summary(&mut out, v);
    out.push_str("This is a price only; nothing was sent.\n");
    out
}

/// `list_views`.
pub fn views(v: &Value) -> String {
    let all = v["views"].as_array().map_or(&[][..], Vec::as_slice);
    if all.is_empty() {
        return "No views.\n".into();
    }
    let mut out = String::new();
    for w in all {
        let _ = writeln!(
            out,
            "{} view {} [{}]: {} people, expires {}{}",
            s(&w["direction"]).unwrap_or("?"),
            s(&w["name"]).unwrap_or("?"),
            s(&w["id"]).unwrap_or("not exported"),
            w["rows"],
            day(&w["expires_at"]).unwrap_or("never"),
            s(&w["owner"]["label"])
                .filter(|_| w["direction"] == "imported")
                .map_or(String::new(), |l| format!(", from {l} (claimed)")),
        );
    }
    out
}

/// `search_view`.
pub fn view_search(v: &Value) -> String {
    let all = v["results"].as_array().map_or(&[][..], Vec::as_slice);
    if all.is_empty() {
        return "No matching people in the imported views.\n".into();
    }
    let mut out = String::new();
    for (i, r) in all.iter().take(MAX_RESULTS).enumerate() {
        let p = &r["person"];
        let _ = writeln!(
            out,
            "{:>2}. {} ({}, last contact {}) from view {} shared by {}",
            i + 1,
            headline(p),
            s(&p["score_bucket"]).unwrap_or("?"),
            s(&p["last_contact_month"]).unwrap_or("-"),
            s(&r["view"]["name"]).unwrap_or("?"),
            s(&r["view"]["owner"]["label"]).unwrap_or("unknown"),
        );
    }
    if all.len() > MAX_RESULTS {
        let _ = writeln!(
            out,
            "... {} more in the structured result",
            all.len() - MAX_RESULTS
        );
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
