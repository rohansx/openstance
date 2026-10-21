//! `openstance path "<target>"` (P3-CLI-01): a warm route to anyone, in
//! the graph or not. Prints the plan with per-step costs and the total;
//! spends only with `--allow-egress` (or a "y" at the prompt). `--json`
//! prints the same `Outcome` the MCP `path_to` tool returns.

use std::fmt::Write as _;

use anyhow::Result;
use clap::Args;
use openstance_core::json::{Evidence, PathJson};
use openstance_core::playbook::{Outcome, RunStatus};
use openstance_core::Graph;
use serde_json::{json, Value};

use crate::playbook::{self, EgressArgs};

/// `openstance path`.
#[derive(Debug, Args)]
pub struct PathArgs {
    /// Name, email, LinkedIn URL or person id ("Name, Title at Company" works too).
    pub target: String,
    /// The target's company (disambiguates and seeds the lookup).
    #[arg(long)]
    pub company: Option<String>,
    /// The target's LinkedIn URL or slug.
    #[arg(long)]
    pub linkedin: Option<String>,
    /// Also find and verify the target's work email (2 paid requests).
    #[arg(long)]
    pub find_email: bool,
    /// Paths to show (1 to 10).
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u16).range(1..=10))]
    pub limit: u16,
    /// Egress approval and budget.
    #[command(flatten)]
    pub egress: EgressArgs,
}

impl PathArgs {
    /// The playbook input (the MCP `path_to` fields).
    pub fn input(&self) -> Value {
        json!({
            "target": self.target,
            "company": self.company,
            "linkedin": self.linkedin,
            "find_email": self.find_email,
            "limit": self.limit,
        })
    }
}

/// Run `path` against an open graph.
pub fn run(g: &Graph, json: bool, a: PathArgs) -> Result<()> {
    let show = |o: &Outcome| print!("{}", playbook::render_plan(&o.plan));
    let out = playbook::run(g, "path", &a.input(), &a.egress, json, &show)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print!("{}", playbook::render_plan(&out.plan));
        if out.result.is_some() {
            print!("\n{}", render(&out));
        }
    }
    playbook::finish(&out, a.egress.plan)
}

fn day(iso: &str) -> &str {
    iso.get(..10).unwrap_or(iso)
}

/// The human view of a `path` result.
pub fn render(out: &Outcome) -> String {
    let mut s = String::new();
    let Some(r) = &out.result else { return s };
    let t = &r["target"];
    let name = t["display_name"].as_str().unwrap_or("?");
    let mut head = name.to_string();
    if let Some(c) = t["company"].as_str() {
        let _ = write!(head, " - {c}");
    }
    if t["in_graph"] == true {
        head.push_str(" (in your graph)");
    } else if let Some(src) = t["profile_source"].as_str() {
        let _ = write!(head, " ({src})");
    } else {
        head.push_str(" (not in your graph)");
    }
    let _ = writeln!(s, "{head}");
    let paths: Vec<PathJson> = serde_json::from_value(r["paths"].clone()).unwrap_or_default();
    if paths.is_empty() {
        let _ = writeln!(
            s,
            "  no warm paths found{}",
            if out.status == RunStatus::Quote {
                " yet (the profile lookup needs --allow-egress)"
            } else {
                ""
            }
        );
    }
    for (i, p) in paths.iter().enumerate() {
        let via = p
            .via
            .display_name
            .clone()
            .unwrap_or_else(|| format!("#{}", p.via.id));
        let co = p
            .via
            .company
            .as_deref()
            .map(|c| format!(" ({c})"))
            .unwrap_or_default();
        let why = match p.reason.as_str() {
            "same_company" => "same company".to_string(),
            "past_company" => "shared employer".to_string(),
            "school" => "same school".to_string(),
            _ => "you both know them".to_string(),
        };
        let _ = writeln!(
            s,
            "{:>3}. {via}{co}  strength {:.2} = {:.2} x {:.2}  {why}",
            i + 1,
            p.strength,
            p.my_score,
            p.link_weight
        );
        for e in &p.evidence {
            match e {
                Evidence::Interaction { line, .. } => {
                    let _ = writeln!(s, "       {line}");
                }
                Evidence::Tie {
                    tie_kind,
                    n_shared,
                    last_at,
                    ..
                } => {
                    let _ = writeln!(
                        s,
                        "       {n_shared} shared {tie_kind}, last {}",
                        day(last_at)
                    );
                }
                Evidence::Field {
                    field,
                    value,
                    matched,
                } => {
                    let _ = writeln!(s, "       {field} {value:?} matches {matched}");
                }
                Evidence::Enrich { .. } => {}
            }
        }
    }
    if let Some(e) = r["email"].as_object() {
        let _ = writeln!(
            s,
            "  email: {} ({})",
            e.get("address").and_then(Value::as_str).unwrap_or("?"),
            e.get("status").and_then(Value::as_str).unwrap_or("?")
        );
    }
    s
}
