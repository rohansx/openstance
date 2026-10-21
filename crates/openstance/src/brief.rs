//! `openstance brief <person>` (P3-CLI-02): the 10-line intro brief as
//! markdown on stdout, so `ost brief "Maya Chen" > ask.md` works. The plan,
//! the prompt and notes go to stderr. `--json` prints the `Outcome` the MCP
//! `brief` tool returns.

use anyhow::Result;
use clap::Args;
use openstance_core::playbook::Outcome;
use openstance_core::Graph;
use serde_json::{json, Value};

use crate::playbook::{self, EgressArgs};

/// `openstance brief`.
#[derive(Debug, Args)]
pub struct BriefArgs {
    /// An id, email, LinkedIn URL or slug, @handle, or a name in your graph.
    pub person: String,
    /// Add one web search for recent public posts (1 paid request).
    #[arg(long)]
    pub web: bool,
    /// Egress approval and budget.
    #[command(flatten)]
    pub egress: EgressArgs,
}

impl BriefArgs {
    /// The playbook input (the MCP `brief` fields).
    pub fn input(&self) -> Value {
        json!({ "person": self.person, "web": self.web })
    }
}

/// Run `brief` against an open graph.
pub fn run(g: &Graph, json: bool, a: BriefArgs) -> Result<()> {
    let show = |o: &Outcome| eprint!("{}", playbook::render_plan(&o.plan));
    let out = playbook::run(g, "brief", &a.input(), &a.egress, json, &show)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        if a.egress.plan || !out.plan.quote.lines.is_empty() {
            eprint!("{}", playbook::render_plan(&out.plan));
        }
        if let Some(md) = out.result.as_ref().and_then(|r| r["markdown"].as_str()) {
            print!("{md}");
        }
    }
    playbook::finish(&out, a.egress.plan)
}
