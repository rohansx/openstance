//! `openstance who <person>` (P2-CLI-01): one person, with the score taken
//! apart component by component (`weight x component = contribution`, then
//! the sum), so the output answers "why 0.81" on its own. `--json` prints
//! the `PersonDetail` shape (`get_person`, interfaces.md 1.8).

use std::fmt::Write as _;

use anyhow::Result;
use openstance_core::json::{Evidence, PersonDetail};
use openstance_core::resolve::lookup::PersonKey;
use openstance_core::search::person::DetailOptions;
use openstance_core::Graph;

use crate::WhoArgs;

/// Mutuals shown in the human view (the JSON keeps the `get_person` default).
const MUTUALS_SHOWN: usize = 5;

/// Run `who` against an open graph.
pub fn run(g: &Graph, json: bool, a: WhoArgs) -> Result<()> {
    let opts = DetailOptions {
        interactions: a.interactions,
        ..DetailOptions::default()
    };
    let d = g.get_person(&PersonKey::parse(&a.person), opts)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&d)?);
    } else {
        print!("{}", render(&d));
    }
    Ok(())
}

fn day(iso: &str) -> &str {
    iso.get(..10).unwrap_or(iso)
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The human view.
pub fn render(d: &PersonDetail) -> String {
    let mut s = String::new();
    let p = &d.person;
    let c = &p.components;
    let w = &c.weights;
    let mut head = p
        .display_name
        .clone()
        .unwrap_or_else(|| format!("#{}", p.id));
    let tc = match (&p.title, &p.company) {
        (Some(t), Some(co)) => format!("{t}, {co}"),
        (Some(t), None) => t.clone(),
        (None, Some(co)) => co.clone(),
        (None, None) => String::new(),
    };
    if !tc.is_empty() {
        head.push_str(&format!(" - {tc}"));
    }
    if let Some(l) = &p.location {
        head.push_str(&format!(" - {l}"));
    }
    let _ = writeln!(
        s,
        "{head}    score {:.2} (preset {}, #{})",
        p.score, c.preset, p.id
    );
    if let Some(from) = d.resolved_from {
        let _ = writeln!(s, "  (resolved from merged id {from})");
    }

    let raw = c.raw.as_ref();
    let why_r = raw.and_then(|r| r.days_since_last).map_or_else(
        || "no contact yet".to_string(),
        |d| {
            format!(
                "{d:.0} days since last contact (half-life {})",
                c.half_life_days
            )
        },
    );
    let why_f = raw.map_or_else(String::new, |r| {
        plural(r.n_interactions, "interaction", "interactions")
    });
    let why_c = raw.map_or_else(String::new, |r| {
        format!(
            "{} + {} started (x2) / {} sent",
            plural(r.replies_from_them, "reply", "replies"),
            plural(r.threads_they_started, "thread", "threads"),
            r.messages_from_you
        )
    });
    let why_m = raw.map_or_else(String::new, |r| plural(r.n_meetings, "meeting", "meetings"));
    let why_b = raw.map_or_else(String::new, |r| r.channels.join(", "));
    let rows = [
        ("recency", c.recency, w.recency, why_r),
        ("frequency", c.frequency, w.frequency, why_f),
        ("reciprocity", c.reciprocity, w.reciprocity, why_c),
        ("meetings", c.meetings, w.meetings, why_m),
        ("breadth", c.breadth, w.breadth, why_b),
    ];
    let mut sum = 0.0;
    for (name, value, weight, why) in &rows {
        let contribution = value * weight;
        sum += contribution;
        let _ = writeln!(
            s,
            "  {name:<12} {value:.3} x {weight:.2} = {contribution:.3}   {why}"
        );
    }
    let _ = writeln!(s, "  {:<12}              = {sum:.3}", "sum");

    let mut flags = Vec::new();
    if p.is_bulk {
        flags.push("bulk sender (score 0, hidden from search)");
    }
    if p.do_not_share {
        flags.push("do not share (never in views or remote results)");
    }
    if p.no_enrich {
        flags.push("no enrich");
    }
    if !flags.is_empty() {
        let _ = writeln!(s, "flags       {}", flags.join(", "));
    }

    let ids: Vec<String> = d
        .identities
        .iter()
        .map(|i| {
            let conf = i
                .confidence
                .map_or_else(|| "-".to_string(), |c| format!("{c:.2}"));
            let rule = i.match_rule.as_deref().unwrap_or("-");
            format!("{}:{} ({}, {conf} {rule})", i.kind, i.value, i.source.kind)
        })
        .collect();
    let _ = writeln!(s, "identities  {}", ids.join("  "));
    if !p.tags.is_empty() {
        let _ = writeln!(s, "tags        {}", p.tags.join(", "));
    }

    let mut first = true;
    for e in &d.recent_interactions {
        if let Evidence::Interaction { line, .. } = e {
            let label = if first { "recent" } else { "" };
            let _ = writeln!(s, "{label:<11} {line}");
            first = false;
        }
    }
    if !d.mutuals.is_empty() {
        let m: Vec<String> = d
            .mutuals
            .iter()
            .take(MUTUALS_SHOWN)
            .map(|m| {
                format!(
                    "{} ({}, {} shared, tie {:.2})",
                    m.person.display_name.as_deref().unwrap_or("-"),
                    m.tie_kind,
                    m.n_shared,
                    m.weight
                )
            })
            .collect();
        let _ = writeln!(s, "mutuals     {}", m.join("  "));
    }

    let fs: Vec<String> = p
        .field_sources
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}: {v}")))
        .collect();
    if !fs.is_empty() {
        let _ = writeln!(s, "fields from {}", fs.join("  "));
    }
    for o in &d.overrides {
        let _ = writeln!(
            s,
            "{:<11} {} = {:?} (override {})",
            "edited",
            o.field,
            o.value.as_deref().unwrap_or(""),
            day(&o.set_at)
        );
    }
    for n in &d.notes {
        let _ = writeln!(s, "note        {} ({})", n.value, day(&n.set_at));
    }
    if d.merge.flagged || !d.merge.merged_from.is_empty() {
        let mut parts = Vec::new();
        if d.merge.flagged {
            parts.push(format!(
                "flagged for review (confidence {:.2}; openstance review)",
                d.merge.confidence
            ));
        }
        if !d.merge.merged_from.is_empty() {
            let ids: Vec<String> = d
                .merge
                .merged_from
                .iter()
                .map(|i| format!("#{i}"))
                .collect();
            parts.push(format!("merged from {}", ids.join(", ")));
        }
        let _ = writeln!(s, "merge       {}", parts.join("; "));
    }
    s
}
