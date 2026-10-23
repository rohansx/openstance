//! Playbooks (P3-CORE-03): small, fixed step lists over the local graph
//! that may include metered egress through the [`EgressGate`]. Spec:
//! `docs/tech-spec/playbooks.md` sections 1, 2 and 6, and
//! `docs/tech-spec/interfaces.md` 1.9 (the `Outcome` JSON).
//!
//! There is no agent loop: every playbook declares its steps and its egress
//! ceiling up front ([`Playbook::plan`]), and the only branch allowed is
//! "local data is enough, skip the egress step". `plan` prices egress through
//! [`EgressGate::quote`] and never sends anything. The CLI, the MCP server
//! and (later) the HTTP API all call [`Graph::run_playbook`].

pub mod accounts;
pub mod brief;
pub mod path;
pub mod reconnect;
pub mod refresh;
pub mod verify;
pub mod watch;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::egress::{effective_budget, new_run_id, EgressGate, Receipt, Surface};
use crate::enrich::{Enricher, MicroUsd, Quote, SkipReason};
use crate::error::{Error, Result};
use crate::json::{iso, V};
use crate::Graph;

pub use accounts::Accounts;
pub use brief::Brief;
pub use path::PathPlaybook;
pub use reconnect::Reconnect;
pub use refresh::Refresh;
pub use verify::Verify;
pub use watch::Watch;

/// What a step will do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepDecision {
    /// The step runs.
    Run,
    /// The graph already has the answer; no egress.
    SkipLocalSufficient,
    /// `enrich.backend = "none"` (or the backend cannot do this op).
    SkipNoEnricher,
    /// Egress was not allowed for this run.
    SkipNoEgress,
    /// The caller did not ask for this optional step.
    SkipNotRequested,
    /// The person is `no_enrich` (or `do_not_share`).
    SkipPrivacy,
    /// The step's price does not fit in the budget.
    SkipOverBudget,
    /// An earlier step did not produce what this one needs.
    SkipNoInput,
}

/// One planned step.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanStep {
    /// 1-based step number.
    pub n: u8,
    /// "lookup target in graph".
    pub description: String,
    /// The op this step may send (`None` for local steps).
    pub egress: Option<String>,
    /// Price of this step in the quote (0 for local steps).
    pub quoted_micro_usd: MicroUsd,
    /// What the step will do.
    pub will_run: StepDecision,
}

/// A static egress ceiling of a playbook (playbooks.md section 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EgressMax {
    /// Op name.
    pub op: String,
    /// Most calls of this op per run.
    pub max: u32,
}

/// Steps and quoted cost, before running.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    /// Playbook name.
    pub playbook: String,
    /// Run id (shared with the `enrich_log` rows).
    pub run_id: String,
    /// Fixed steps.
    pub steps: Vec<PlanStep>,
    /// Price of the egress steps (empty lines when none is planned).
    #[serde(serialize_with = "quote_json")]
    pub quote: Quote,
    /// Static ceiling.
    pub egress_max: Vec<EgressMax>,
}

impl Plan {
    /// Set the decision of every egress step that is still `run`.
    pub fn mark_egress(&mut self, d: StepDecision) {
        for s in &mut self.steps {
            if s.egress.is_some() && s.will_run == StepDecision::Run {
                s.will_run = d;
            }
        }
    }

    /// Set one step's decision.
    pub fn mark(&mut self, n: u8, d: StepDecision) {
        if let Some(s) = self.steps.iter_mut().find(|s| s.n == n) {
            s.will_run = d;
        }
    }

    /// Number of quoted requests.
    pub fn calls(&self) -> usize {
        self.quote.lines.len()
    }
}

/// `Quote` with `quoted_at` as ISO 8601 (interfaces.md 1.9).
fn quote_json<S: Serializer>(q: &Quote, s: S) -> std::result::Result<S::Ok, S::Error> {
    let mut v = serde_json::to_value(q).map_err(serde::ser::Error::custom)?;
    if let Some(m) = v.as_object_mut() {
        m.insert("quoted_at".into(), Value::String(iso(q.quoted_at)));
    }
    v.serialize(s)
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Every step that could run, ran.
    Done,
    /// Some egress was blocked (budget) or failed.
    Partial,
    /// Egress is needed but was not allowed: here is the price.
    Quote,
    /// Nothing to do (e.g. `plan_only` with no egress).
    NothingToDo,
}

/// The result of a playbook run (interfaces.md 1.9).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outcome {
    /// Schema version.
    pub v: u32,
    /// Playbook name.
    pub playbook: String,
    /// How it ended.
    pub status: RunStatus,
    /// The plan, with decisions updated by the run.
    pub plan: Plan,
    /// Playbook-specific result (partial when egress was not allowed).
    pub result: Option<Value>,
    /// What it cost.
    pub receipt: Receipt,
    /// Human-readable notes ("target not in graph; enricher is none").
    pub notes: Vec<String>,
}

/// Run context, resolved by [`Graph::run_playbook`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCtx {
    /// The caller allowed egress for whatever quote this run produces.
    pub allow_egress: bool,
    /// Effective budget (enrichment.md section 5).
    pub budget_micro_usd: MicroUsd,
    /// Injected "now".
    pub now: i64,
    /// Run id.
    pub run_id: String,
}

/// A playbook. Implementations are stateless.
pub trait Playbook: Send + Sync {
    /// `path`, `brief`, ...
    fn name(&self) -> &'static str;
    /// Static egress ceiling.
    fn egress_max(&self) -> Vec<EgressMax>;
    /// Steps and quote. Local reads plus [`EgressGate::quote`]; never egress.
    fn plan(&self, g: &Graph, gate: &mut EgressGate<'_>, input: &Value) -> Result<Plan>;
    /// Execute. Egress only through `gate`, and only if `ctx.allow_egress`.
    fn run(
        &self,
        g: &Graph,
        gate: &mut EgressGate<'_>,
        input: &Value,
        ctx: &RunCtx,
    ) -> Result<Outcome>;
}

/// Every playbook name, in the order of playbooks.md.
pub const NAMES: [&str; 7] = [
    "path",
    "brief",
    "accounts",
    "refresh",
    "watch",
    "reconnect",
    "verify",
];

/// The playbooks shipped so far.
pub fn by_name(name: &str) -> Option<&'static dyn Playbook> {
    match name {
        "path" => Some(&PathPlaybook),
        "brief" => Some(&Brief),
        "reconnect" => Some(&Reconnect),
        "accounts" => Some(&Accounts),
        "refresh" => Some(&Refresh),
        "verify" => Some(&Verify),
        "watch" => Some(&Watch),
        _ => None,
    }
}

/// How a surface runs a playbook.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOptions {
    /// Approve whatever quote this run produces (`--yes`, `allow_egress`).
    pub allow_egress: bool,
    /// Budget for this run in USD (see [`effective_budget`]).
    pub max_usd: Option<f64>,
    /// Who is asking (the budget rule differs).
    pub surface: Surface,
    /// Return the plan and quote only; do not run local steps.
    pub plan_only: bool,
    /// Fixed run id (tests); a new ULID otherwise.
    pub run_id: Option<String>,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            allow_egress: false,
            max_usd: None,
            surface: Surface::Agent,
            plan_only: false,
            run_id: None,
        }
    }
}

impl Graph {
    /// Run (or only plan) the playbook `name` with the configured enricher.
    pub fn run_playbook(&self, name: &str, input: &Value, opts: &RunOptions) -> Result<Outcome> {
        self.run_playbook_with(self.providers.enricher.as_ref(), name, input, opts)
    }

    /// [`Graph::run_playbook`] with an explicit enricher (tests, MCP).
    pub fn run_playbook_with(
        &self,
        enricher: &dyn Enricher,
        name: &str,
        input: &Value,
        opts: &RunOptions,
    ) -> Result<Outcome> {
        let pb = by_name(name).ok_or_else(|| Error::NotFound(format!("playbook {name:?}")))?;
        let now = self.now();
        let ctx = RunCtx {
            allow_egress: opts.allow_egress,
            budget_micro_usd: effective_budget(&self.cfg.enrich, opts.max_usd, opts.surface),
            now,
            run_id: opts.run_id.clone().unwrap_or_else(|| new_run_id(now)),
        };
        let mut gate =
            EgressGate::new(self, enricher, pb.name(), &ctx.run_id, ctx.budget_micro_usd);
        if opts.plan_only {
            let plan = pb.plan(self, &mut gate, input)?;
            let status = if plan.calls() == 0 {
                RunStatus::NothingToDo
            } else {
                RunStatus::Quote
            };
            let notes = quote_notes(&plan, false);
            return Ok(Outcome {
                v: V,
                playbook: pb.name().to_string(),
                status,
                receipt: gate.receipt(),
                plan,
                result: None,
                notes,
            });
        }
        pb.run(self, &mut gate, input, &ctx)
    }
}

/// `$0.0100` style money.
pub fn usd(micro: MicroUsd) -> String {
    format!("${}.{:04}", micro / 1_000_000, (micro % 1_000_000) / 100)
}

/// Notes that explain the quote: what egress would cost, or why it is off.
pub fn quote_notes(plan: &Plan, allow_egress: bool) -> Vec<String> {
    let mut notes = Vec::new();
    let q = &plan.quote;
    if q.skipped
        .iter()
        .any(|s| s.reason == SkipReason::BackendNone)
    {
        notes.push("enrichment is off (enrich.backend = \"none\"): egress steps skipped".into());
    }
    if q.skipped
        .iter()
        .any(|s| matches!(s.reason, SkipReason::NoEnrich | SkipReason::DoNotShare))
    {
        notes.push("skipped: this person is marked no_enrich or do_not_share".into());
    }
    if !q.lines.is_empty() {
        if !q.within_budget {
            notes.push(format!(
                "quote {} is over the budget {}: only requests that fit will be sent",
                usd(q.total_micro_usd),
                usd(q.budget_micro_usd)
            ));
        }
        if !allow_egress {
            notes.push(format!(
                "egress not allowed: pass allow_egress=true to send {} request{} for {}",
                q.lines.len(),
                if q.lines.len() == 1 { "" } else { "s" },
                usd(q.total_micro_usd)
            ));
        }
    }
    notes
}

/// Decision for an egress step of `op` given the quote.
pub(crate) fn decision_for(q: &Quote, op: &str) -> StepDecision {
    if q.lines.iter().any(|l| l.job.request.op() == op) {
        return StepDecision::Run;
    }
    match q.skipped.iter().find(|s| s.op == op).map(|s| s.reason) {
        Some(SkipReason::NoEnrich | SkipReason::DoNotShare) => StepDecision::SkipPrivacy,
        Some(SkipReason::OverBudget) => StepDecision::SkipOverBudget,
        Some(SkipReason::LocalDataSufficient) => StepDecision::SkipLocalSufficient,
        Some(SkipReason::BackendNone | SkipReason::Unsupported) | None => {
            StepDecision::SkipNoEnricher
        }
    }
}

/// Price of the quote lines of `op`.
pub(crate) fn price_of(q: &Quote, op: &str) -> MicroUsd {
    q.lines
        .iter()
        .filter(|l| l.job.request.op() == op)
        .map(|l| l.micro_usd)
        .sum()
}

/// A local step.
pub(crate) fn local_step(n: u8, description: &str) -> PlanStep {
    PlanStep {
        n,
        description: description.into(),
        egress: None,
        quoted_micro_usd: 0,
        will_run: StepDecision::Run,
    }
}

/// An egress step priced from the quote.
pub(crate) fn egress_step(n: u8, description: &str, op: &str, q: &Quote) -> PlanStep {
    PlanStep {
        n,
        description: description.into(),
        egress: Some(op.into()),
        quoted_micro_usd: price_of(q, op),
        will_run: decision_for(q, op),
    }
}

/// The plan of a playbook whose steps are all local: an empty quote (which
/// writes no `enrich_log` row) and the playbook's egress ceiling.
pub(crate) fn local_plan(
    pb: &dyn Playbook,
    gate: &mut EgressGate<'_>,
    steps: Vec<PlanStep>,
) -> Result<Plan> {
    let quote = gate.quote(Vec::new())?;
    Ok(Plan {
        playbook: pb.name().into(),
        run_id: gate.run_id().to_string(),
        steps,
        quote,
        egress_max: pb.egress_max(),
    })
}

/// A finished run with no egress.
pub(crate) fn local_outcome(
    pb: &dyn Playbook,
    gate: &EgressGate<'_>,
    plan: Plan,
    result: Value,
    notes: Vec<String>,
) -> Outcome {
    Outcome {
        v: V,
        playbook: pb.name().into(),
        status: RunStatus::Done,
        receipt: gate.receipt(),
        plan,
        result: Some(result),
        notes,
    }
}

/// How a run that may have quoted egress ended: `quote` when egress was
/// needed but not allowed, `partial` when something was not sent or failed.
pub(crate) fn egress_status(plan: &Plan, allow_egress: bool, partial: bool) -> RunStatus {
    if !plan.quote.lines.is_empty() && !allow_egress {
        RunStatus::Quote
    } else if partial {
        RunStatus::Partial
    } else {
        RunStatus::Done
    }
}

/// A company named by the caller (`accounts`, `watch.targets`): a name
/// (`"Stripe"`) or a domain (`"stripe.com"`, `"www.stripe.com"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompanyTarget {
    /// As given, trimmed.
    pub input: String,
    /// `company_key` of the name, or of the domain's first label.
    pub key: String,
    /// The domain, lowercase, when the input was one.
    pub domain: Option<String>,
}

impl CompanyTarget {
    pub(crate) fn parse(s: &str) -> Option<CompanyTarget> {
        let input = s.trim();
        if input.is_empty() {
            return None;
        }
        let lower = input.to_ascii_lowercase();
        let looks_like_domain = !lower.contains(char::is_whitespace)
            && lower.contains('.')
            && lower
                .rsplit('.')
                .next()
                .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
        let (key, domain) = if looks_like_domain {
            let d = lower
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches("www.")
                .trim_end_matches('/')
                .to_string();
            let label = d.split('.').next().unwrap_or(&d).to_string();
            (crate::resolve::normalize::company_key(&label), Some(d))
        } else {
            (crate::resolve::normalize::company_key(input), None)
        };
        (!key.is_empty()).then(|| CompanyTarget {
            input: input.to_string(),
            key,
            domain,
        })
    }

    /// Does a company field value name this company?
    pub(crate) fn matches(&self, company: &str) -> bool {
        crate::resolve::normalize::company_key(company) == self.key
    }
}

/// Parse a playbook input object.
pub(crate) fn parse_input<T: serde::de::DeserializeOwned>(name: &str, input: &Value) -> Result<T> {
    serde_json::from_value(input.clone())
        .map_err(|e| Error::BadRequest(format!("invalid {name} input: {e}")))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_p4;

#[cfg(test)]
mod target_tests {
    use super::CompanyTarget;

    #[test]
    fn names_and_domains() {
        let t = CompanyTarget::parse(" Stripe, Inc. ").unwrap();
        assert_eq!((t.key.as_str(), t.domain), ("stripe", None));
        let t = CompanyTarget::parse("www.Stripe.com").unwrap();
        assert_eq!(t.key, "stripe");
        assert_eq!(t.domain.as_deref(), Some("stripe.com"));
        assert!(t.matches("Stripe"));
        let t = CompanyTarget::parse("Acme Co.").unwrap();
        assert_eq!(t.domain, None, "a trailing dot is not a domain");
        assert!(CompanyTarget::parse("   ").is_none());
    }
}
