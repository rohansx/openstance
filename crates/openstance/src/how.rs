//! `openstance how <target>` (alias `how-connected`, P2-CLI-02): the CLI
//! face of `how_connected`. Prints the direct relationship, then the top
//! paths `you -> Mutual (score) -> Target (tie)` with their evidence lines.
//! `--json` prints exactly what the MCP `how_connected` tool returns as
//! `structuredContent` (interfaces.md 1.7).

use std::fmt::Write as _;

use anyhow::Result;
use clap::Args;
use openstance_core::graphq::{HowConnectedResponse, HowOptions};
use openstance_core::json::{Evidence, HowConnected};
use openstance_core::resolve::lookup::PersonKey;
use openstance_core::Graph;

/// `openstance how`.
#[derive(Debug, Args)]
pub struct HowArgs {
    /// The target: an id, email, LinkedIn URL or slug, @handle, or a name.
    pub target: String,
    /// Paths to show (1 to 10).
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u16).range(1..=10))]
    pub limit: u16,
}

/// Run `how` against an open graph.
pub fn run(g: &Graph, json: bool, a: HowArgs) -> Result<()> {
    let r = g.how_connected(
        &PersonKey::parse(&a.target),
        HowOptions {
            limit: usize::from(a.limit),
        },
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
    } else {
        print!("{}", render(&r, &a.target));
    }
    Ok(())
}

fn day(iso: &str) -> &str {
    iso.get(..10).unwrap_or(iso)
}

/// The human view.
pub fn render(r: &HowConnectedResponse, asked: &str) -> String {
    match r {
        HowConnectedResponse::NotFound(n) => {
            format!("{asked:?} is not in your graph. {}\n", n.hint)
        }
        HowConnectedResponse::Found(h) => render_found(h),
    }
}

fn render_found(h: &HowConnected) -> String {
    let mut s = String::new();
    let target = h
        .target
        .display_name
        .clone()
        .unwrap_or_else(|| format!("#{}", h.target.id));
    match &h.direct {
        Some(d) => {
            let _ = writeln!(
                s,
                "direct: score {:.2}, last contact {}",
                d.score,
                d.last_contact.as_deref().map_or("-", day)
            );
        }
        None => {
            let _ = writeln!(s, "direct: none (no interactions with {target})");
        }
    }
    if h.paths.is_empty() {
        let _ = writeln!(
            s,
            "no warm paths to {target}: nobody you know shares a thread or meeting with them"
        );
        return s;
    }
    for (n, p) in h.paths.iter().enumerate() {
        let via = p
            .via
            .display_name
            .clone()
            .unwrap_or_else(|| format!("#{}", p.via.id));
        let _ = writeln!(
            s,
            "{:>2}  you -> {via} (score {:.2}) -> {target} (tie {:.2})   strength {:.3} = {:.2} x {:.2}",
            n + 1,
            p.my_score,
            p.link_weight,
            p.strength,
            p.my_score,
            p.link_weight
        );
        let interactions: Vec<&str> = p
            .evidence
            .iter()
            .filter_map(|e| match e {
                Evidence::Interaction { line, .. } => Some(line.as_str()),
                _ => None,
            })
            .collect();
        for e in &p.evidence {
            if let Evidence::Tie {
                tie_kind,
                n_shared,
                weight,
                last_at,
                ..
            } = e
            {
                let _ = writeln!(
                    s,
                    "      {n_shared} shared {tie_kind} (tie {weight:.2}), last {}",
                    day(last_at)
                );
            }
        }
        // Core contract: the last interaction item is my own last
        // interaction with the mutual; the ones before it are shared.
        if let Some((mine, shared)) = interactions.split_last() {
            for l in shared {
                let _ = writeln!(s, "      {l}");
            }
            let _ = writeln!(s, "      you and {via}: {mine}");
        }
    }
    s
}
