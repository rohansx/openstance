//! The Phase 4 playbook commands (P4-CLI-01): `reconnect`, `accounts`,
//! `refresh`, `verify` and `watch`. Each is the shared core playbook plus a
//! table; `--json` prints the `Outcome` that `POST /playbooks/{name}`
//! returns. Approval works as for `path` (interfaces.md section 7): the plan
//! goes to stderr, `--yes` approves, no TTY or `--json` without `--yes`
//! exits 3 with the quote. The plan and notes go to stderr, results to
//! stdout.

use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use openstance_core::playbook::Outcome;
use openstance_core::Graph;
use serde_json::{json, Value};

use crate::playbook::{self, EgressArgs};

/// `openstance reconnect`.
#[derive(Debug, Args)]
pub struct ReconnectArgs {
    /// People listed (default playbooks.reconnect.limit).
    #[arg(long)]
    pub limit: Option<u32>,
    /// Egress approval and budget (reconnect never spends).
    #[command(flatten)]
    pub egress: EgressArgs,
}

/// `openstance accounts <companies.txt>`.
#[derive(Debug, Args)]
pub struct AccountsArgs {
    /// One company name or domain per line (`#` comments and blank lines are skipped).
    pub file: PathBuf,
    /// Skip people one tie away.
    #[arg(long)]
    pub no_one_hop: bool,
    /// Rows per list in --json (default 10).
    #[arg(long)]
    pub limit_per_company: Option<usize>,
    /// Print the coverage table as CSV.
    #[arg(long)]
    pub csv: bool,
    /// Egress approval and budget (accounts never spends).
    #[command(flatten)]
    pub egress: EgressArgs,
}

/// `openstance refresh`.
#[derive(Debug, Args)]
pub struct RefreshArgs {
    /// Only people with at least this many interactions (default 5).
    #[arg(long)]
    pub min_interactions: Option<u32>,
    /// Most people per run (default 50; the budget may stop it earlier).
    #[arg(long)]
    pub limit: Option<u32>,
    /// Egress approval and budget.
    #[command(flatten)]
    pub egress: EgressArgs,
}

/// `openstance verify <person>`.
#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// An id, email, LinkedIn URL or slug, @handle, or a name in your graph.
    pub person: String,
    /// Egress approval and budget.
    #[command(flatten)]
    pub egress: EgressArgs,
}

/// `openstance watch`.
#[derive(Debug, Args)]
pub struct WatchArgs {
    /// Companies whose incoming moves are highlighted: a file (one per line)
    /// or a comma-separated list (default playbooks.watch.targets).
    #[arg(long)]
    pub targets: Option<String>,
    /// Also check the top profiles through the enricher (paid, quote first).
    #[arg(long)]
    pub check_profiles: bool,
    /// Profiles checked with --check-profiles (default 100).
    #[arg(long = "top", visible_alias = "top-n")]
    pub top: Option<u32>,
    /// Egress approval and budget.
    #[command(flatten)]
    pub egress: EgressArgs,
}

/// Lines of a list file: trimmed, no blanks, no `#` comments.
fn list_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn date(v: &Value) -> String {
    s(v).chars().take(10).collect()
}

fn score(v: &Value) -> String {
    v.as_f64().map_or_else(String::new, |f| format!("{f:.2}"))
}

/// Run the playbook with the CLI approval flow, print, finish.
fn go(
    g: &Graph,
    name: &str,
    input: &Value,
    a: &EgressArgs,
    json: bool,
    render: &dyn Fn(&Outcome) -> String,
) -> Result<()> {
    let show = |o: &Outcome| eprint!("{}", playbook::render_plan(&o.plan));
    let out = playbook::run(g, name, input, a, json, &show)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        if a.plan || !out.plan.quote.lines.is_empty() {
            eprint!("{}", playbook::render_plan(&out.plan));
        }
        if out.result.is_some() {
            print!("{}", render(&out));
        }
    }
    playbook::finish(&out, a.plan)
}

/// `openstance reconnect`.
pub fn reconnect(g: &Graph, json: bool, a: ReconnectArgs) -> Result<()> {
    let mut input = json!({});
    if let Some(l) = a.limit {
        input["limit"] = json!(l);
    }
    go(g, "reconnect", &input, &a.egress, json, &|o| {
        let people = o
            .result
            .as_ref()
            .map(|r| r["people"].clone())
            .unwrap_or_default();
        let rows = people.as_array().cloned().unwrap_or_default();
        if rows.is_empty() {
            return "Nobody to reconnect with: no one you talked to often has gone quiet.\n".into();
        }
        let mut t = String::new();
        let _ = writeln!(
            t,
            "{:>2}  {:<24}  {:>4}  {:>4}  {:<12}  last subject",
            "#", "person", "was", "now", "last contact"
        );
        for (i, p) in rows.iter().enumerate() {
            let _ = writeln!(
                t,
                "{:>2}  {:<24}  {:>4}  {:>4}  {:<12}  {}",
                i + 1,
                s(&p["display_name"]),
                score(&p["past_score"]),
                score(&p["score"]),
                date(&p["last_contact"]),
                s(&p["last_subject"])
            );
        }
        t
    })
}

/// The best contact of a coverage row, labelled.
fn best_label(row: &Value) -> (String, String) {
    let Some(best) = row["best"].as_i64() else {
        return (String::new(), "-".into());
    };
    let find = |list: &str| {
        row[list]
            .as_array()
            .and_then(|l| l.iter().find(|p| p["id"].as_i64() == Some(best)).cloned())
    };
    let sc = score(&row["best_score"]);
    if let Some(p) = find("current") {
        let n = s(&p["display_name"]);
        (n.clone(), format!("{n} ({sc})"))
    } else if let Some(p) = find("alumni") {
        let n = s(&p["display_name"]);
        (n.clone(), format!("{n} ({sc}, alumni)"))
    } else if let Some(p) = find("one_hop") {
        let n = s(&p["display_name"]);
        (n.clone(), format!("{n} ({sc}, one hop)"))
    } else {
        (String::new(), format!("#{best}"))
    }
}

fn csv_field(v: &str) -> String {
    if v.contains([',', '"', '\n']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

/// `openstance accounts`.
pub fn accounts(g: &Graph, json: bool, a: AccountsArgs) -> Result<()> {
    let text = std::fs::read_to_string(&a.file)
        .with_context(|| format!("cannot read {}", a.file.display()))?;
    let mut input = json!({ "companies": list_lines(&text), "one_hop": !a.no_one_hop });
    if let Some(l) = a.limit_per_company {
        input["limit_per_company"] = json!(l);
    }
    let csv = a.csv;
    go(g, "accounts", &input, &a.egress, json, &|o| {
        let rows = o
            .result
            .as_ref()
            .and_then(|r| r["companies"].as_array().cloned())
            .unwrap_or_default();
        let mut t = String::new();
        if csv {
            let _ = writeln!(
                t,
                "company,domain,current,alumni,one_hop,coverage,best_id,best_contact,best_score"
            );
            for r in &rows {
                let (name, _) = best_label(r);
                let fields = [
                    s(&r["company"]),
                    s(&r["domain"]),
                    s(&r["counts"]["current"]),
                    s(&r["counts"]["alumni"]),
                    s(&r["counts"]["one_hop"]),
                    s(&r["coverage"]),
                    s(&r["best"]),
                    name,
                    s(&r["best_score"]),
                ];
                let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
                let _ = writeln!(t, "{}", line.join(","));
            }
            return t;
        }
        let width = rows
            .iter()
            .map(|r| s(&r["company"]).chars().count())
            .max()
            .unwrap_or(0)
            .max(7);
        let _ = writeln!(
            t,
            "{:<width$}  current  alumni  one-hop  coverage  best contact",
            "company"
        );
        for r in &rows {
            let _ = writeln!(
                t,
                "{:<width$}  {:>7}  {:>6}  {:>7}  {:<8}  {}",
                s(&r["company"]),
                s(&r["counts"]["current"]),
                s(&r["counts"]["alumni"]),
                s(&r["counts"]["one_hop"]),
                s(&r["coverage"]),
                best_label(r).1
            );
        }
        t
    })
}

/// `openstance refresh`.
pub fn refresh(g: &Graph, json: bool, a: RefreshArgs) -> Result<()> {
    let mut input = json!({});
    if let Some(m) = a.min_interactions {
        input["min_interactions"] = json!(m);
    }
    if let Some(l) = a.limit {
        input["limit"] = json!(l);
    }
    go(g, "refresh", &input, &a.egress, json, &|o| {
        let r = o.result.clone().unwrap_or_default();
        let mut t = String::new();
        for u in r["updated"].as_array().into_iter().flatten() {
            let fields: Vec<String> = u["fields"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| format!("{k} = {}", s(v)))
                .collect();
            let _ = writeln!(t, "#{}  {}", s(&u["id"]), fields.join(", "));
        }
        let n = |k: &str| r[k].as_array().map_or(0, Vec::len);
        let _ = writeln!(
            t,
            "updated {}, unchanged {}, not found {}, failed {}, skipped {}",
            n("updated"),
            n("unchanged"),
            n("not_found"),
            n("failed"),
            n("skipped")
        );
        t
    })
}

/// `openstance verify`.
pub fn verify(g: &Graph, json: bool, a: VerifyArgs) -> Result<()> {
    let input = json!({ "person": a.person });
    go(g, "verify", &input, &a.egress, json, &|o| {
        let r = o.result.clone().unwrap_or_default();
        let mut t = String::new();
        if r["email"].is_null() {
            return t;
        }
        let via = if r["source"].is_null() {
            String::new()
        } else if o.receipt.requests_sent > 0 {
            format!(
                " ({}, {})",
                s(&r["source"]),
                openstance_core::playbook::usd(o.receipt.spent_micro_usd)
            )
        } else {
            format!(" ({}, {})", s(&r["source"]), date(&r["verified_at"]))
        };
        let _ = writeln!(t, "{}  {}{via}", s(&r["email"]), s(&r["status"]));
        match r["mutual"].as_object() {
            Some(m) => {
                let subject = match m.get("last_subject") {
                    Some(Value::String(x)) => format!(" \"{x}\""),
                    _ => String::new(),
                };
                let _ = writeln!(
                    t,
                    "freshest mutual who replied: {}, {}{subject}",
                    s(&m["display_name"]),
                    date(&m["last_reply"])
                );
            }
            None => {
                let _ = writeln!(t, "no mutual who replied");
            }
        }
        t
    })
}

/// `openstance watch`.
pub fn watch(g: &Graph, json: bool, a: WatchArgs) -> Result<()> {
    let mut input = json!({ "check_profiles": a.check_profiles });
    if let Some(t) = &a.targets {
        let path = std::path::Path::new(t);
        let list = if path.is_file() {
            list_lines(
                &std::fs::read_to_string(path)
                    .with_context(|| format!("cannot read {}", path.display()))?,
            )
        } else {
            t.split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect()
        };
        input["targets"] = json!(list);
    }
    if let Some(n) = a.top {
        input["top_n"] = json!(n);
    }
    go(g, "watch", &input, &a.egress, json, &|o| {
        let r = o.result.clone().unwrap_or_default();
        let list = |k: &str| r[k].as_array().cloned().unwrap_or_default();
        let (moves, titles, added, removed) = (
            list("moves"),
            list("title_changes"),
            list("new_connections"),
            list("removed_connections"),
        );
        let mut t = String::new();
        if moves.is_empty() && titles.is_empty() && added.is_empty() && removed.is_empty() {
            let _ = writeln!(t, "No changes found.");
            return t;
        }
        for m in &moves {
            let _ = writeln!(
                t,
                "{}move  {}: {} -> {}  ({})",
                if m["target_hit"] == json!(true) {
                    "* "
                } else {
                    "  "
                },
                s(&m["display_name"]),
                s(&m["from"]),
                s(&m["to"]),
                s(&m["detected_by"])
            );
        }
        for c in &titles {
            let _ = writeln!(
                t,
                "  title {}: {} -> {}",
                s(&c["display_name"]),
                s(&c["from"]),
                s(&c["to"])
            );
        }
        let _ = writeln!(
            t,
            "{} moves ({} into targets), {} title changes, {} new and {} removed connections",
            moves.len(),
            moves
                .iter()
                .filter(|m| m["target_hit"] == json!(true))
                .count(),
            titles.len(),
            added.len(),
            removed.len()
        );
        t
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_files_skip_comments_and_blanks() {
        assert_eq!(
            list_lines("# t\nStripe\n\n  ramp.com  \n#x\n"),
            ["Stripe", "ramp.com"]
        );
    }

    #[test]
    fn csv_quotes_only_when_needed() {
        assert_eq!(csv_field("Halden & Rowe"), "Halden & Rowe");
        assert_eq!(csv_field("Acme, Inc."), "\"Acme, Inc.\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
