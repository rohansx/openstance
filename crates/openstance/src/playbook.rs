//! Shared plumbing of the playbook commands (`path`, `brief`): the
//! configured enricher, the egress approval flow, plan rendering and exit
//! codes. Spec: `docs/tech-spec/interfaces.md` sections 7 and 14,
//! `enrichment.md` section 4.1 (approval semantics per surface).
//!
//! Approval: `--allow-egress` (alias `--yes`) approves whatever quote the
//! run produces, within the budget. Without it the playbook runs its free
//! local steps; if egress would be needed, an interactive terminal is asked
//! `Send N requests for $X? [y/N]`, and anything else (a pipe, `--json`)
//! gets the plan and exit code 3. Both runs share one run id, so the audit
//! log holds one row per request.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal, Write as _};

use anyhow::Result;
use clap::Args;
use openstance_core::egress::Surface;
use openstance_core::enrich::{Enricher, NullEnricher, SkipReason};
use openstance_core::playbook::{usd, Outcome, Plan, RunOptions, RunStatus, StepDecision};
use openstance_core::{Config, Graph};
use serde_json::Value;

/// Egress flags shared by every playbook command.
#[derive(Debug, Args, Clone, Default)]
pub struct EgressArgs {
    /// Approve the quote and send the requests (alias: --yes).
    #[arg(long = "allow-egress", visible_alias = "yes", short = 'y')]
    pub allow_egress: bool,
    /// Budget for this run in USD (default enrich.max_usd_per_run; may raise it).
    #[arg(long, value_name = "USD")]
    pub max_usd: Option<f64>,
    /// Print the plan and quote, then exit without running local steps.
    #[arg(long)]
    pub plan: bool,
}

/// Exit with this code after the output was printed (no error message).
#[derive(Debug)]
pub struct Exit(pub u8);

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit {}", self.0)
    }
}

impl std::error::Error for Exit {}

/// The configured enrichment backend (`enrich.backend`). `none` (the
/// default) never opens a socket.
pub fn enricher(cfg: &Config) -> Box<dyn Enricher> {
    match cfg.enrich.backend.as_str() {
        #[cfg(feature = "enrich-treg")]
        "treg" => Box::new(openstance_enrich::treg::TregEnricher::from_env(
            &cfg.enrich.treg.endpoint,
            &cfg.enrich.treg.token_env,
            std::time::Duration::from_secs(u64::from(cfg.enrich.timeout_secs)),
        )),
        _ => Box::new(NullEnricher),
    }
}

fn interactive(json: bool) -> bool {
    !json && std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Run `name` with the CLI approval flow. `show_quote` prints the plan
/// before the prompt. Returns the final outcome.
pub fn run(
    g: &Graph,
    name: &str,
    input: &Value,
    a: &EgressArgs,
    json: bool,
    show_quote: &dyn Fn(&Outcome),
) -> Result<Outcome> {
    let enricher = enricher(g.config());
    let opts = RunOptions {
        allow_egress: a.allow_egress,
        max_usd: a.max_usd,
        surface: Surface::Cli,
        plan_only: a.plan,
        run_id: None,
    };
    let out = g.run_playbook_with(enricher.as_ref(), name, input, &opts)?;
    if out.status != RunStatus::Quote || a.plan || a.allow_egress || !interactive(json) {
        return Ok(out);
    }
    show_quote(&out);
    let q = &out.plan.quote;
    eprint!(
        "Send {} request{} for {}? [y/N] ",
        q.lines.len(),
        if q.lines.len() == 1 { "" } else { "s" },
        usd(q.total_micro_usd)
    );
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if !matches!(answer.trim(), "y" | "Y" | "yes" | "YES" | "Yes") {
        return Ok(out);
    }
    let again = RunOptions {
        allow_egress: true,
        run_id: Some(out.plan.run_id.clone()),
        ..opts
    };
    Ok(g.run_playbook_with(enricher.as_ref(), name, input, &again)?)
}

/// Exit code of an outcome (interfaces.md section 14).
pub fn exit_code(out: &Outcome, plan_only: bool) -> u8 {
    match out.status {
        RunStatus::Quote if !plan_only => 3,
        RunStatus::Partial
            if out
                .receipt
                .skipped
                .iter()
                .any(|s| s.reason == SkipReason::OverBudget) =>
        {
            4
        }
        _ => 0,
    }
}

/// Finish a command: notes and the "re-run" hint on stderr, then the exit
/// code as an [`Exit`] error when it is not 0.
pub fn finish(out: &Outcome, plan_only: bool) -> Result<()> {
    for n in &out.notes {
        if !n.starts_with("egress not allowed") {
            eprintln!("note: {n}");
        }
    }
    let q = &out.plan.quote;
    if out.status == RunStatus::Quote && !q.lines.is_empty() {
        eprintln!(
            "re-run with --allow-egress to spend {} ({} request{})",
            usd(q.total_micro_usd),
            q.lines.len(),
            if q.lines.len() == 1 { "" } else { "s" }
        );
    }
    if out.receipt.requests_sent > 0 {
        eprintln!(
            "Spent {} of {}. Log: openstance enrich log --run {}",
            usd(out.receipt.spent_micro_usd),
            usd(out.receipt.budget_micro_usd),
            out.receipt.run_id
        );
    }
    match exit_code(out, plan_only) {
        0 => Ok(()),
        c => Err(anyhow::Error::new(Exit(c))),
    }
}

fn decision(d: StepDecision) -> &'static str {
    match d {
        StepDecision::Run => "",
        StepDecision::SkipLocalSufficient => "skipped: local data is enough",
        StepDecision::SkipNoEnricher => "skipped: no enricher",
        StepDecision::SkipNoEgress => "needs --allow-egress",
        StepDecision::SkipNotRequested => "skipped: not requested",
        StepDecision::SkipPrivacy => "skipped: no_enrich",
        StepDecision::SkipOverBudget => "skipped: over budget",
        StepDecision::SkipNoInput => "skipped: nothing to look up",
    }
}

/// The plan with per-step costs and the total.
pub fn render_plan(plan: &Plan) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "Plan for {} (run {})", plan.playbook, plan.run_id);
    for st in &plan.steps {
        let label = format!("{} ", st.description);
        let what = match (&st.egress, st.will_run) {
            (None, _) => "local".to_string(),
            (Some(op), StepDecision::Run) => format!("{op} x1  {}", usd(st.quoted_micro_usd)),
            (Some(op), StepDecision::SkipNoEgress) => {
                format!(
                    "{op} x1  {}  ({})",
                    usd(st.quoted_micro_usd),
                    decision(st.will_run)
                )
            }
            (Some(_), d) => decision(d).to_string(),
        };
        let _ = writeln!(s, "  {} {label:.<36} {what}", st.n);
    }
    let q = &plan.quote;
    if q.lines.is_empty() {
        let _ = writeln!(s, "  No egress: nothing leaves this machine.");
    } else {
        let sends: BTreeSet<&str> = q
            .lines
            .iter()
            .flat_map(|l| l.job.request.outbound_fields())
            .map(|(k, _)| k)
            .collect();
        let _ = writeln!(
            s,
            "  Total {} (budget {}) via {}. Sends: {}.",
            usd(q.total_micro_usd),
            usd(q.budget_micro_usd),
            q.backend,
            sends.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_four_decimals() {
        assert_eq!(usd(16_000), "$0.0160");
        assert_eq!(usd(500_000), "$0.5000");
        assert_eq!(usd(2_500_000), "$2.5000");
    }

    #[test]
    fn default_config_gets_the_null_enricher() {
        assert_eq!(enricher(&Config::default()).name(), "none");
    }
}
