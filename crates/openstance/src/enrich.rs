//! `openstance enrich log` (P3-CLI-04): every request that was priced or
//! sent, exactly what it carried, and what it cost. Spec: interfaces.md
//! section 8 and enrichment.md section 6.

use std::fmt::Write as _;

use anyhow::Result;
use clap::{Args, Subcommand};
use openstance_core::egress::log::{EnrichLogResponse, LogFilter};
use openstance_core::playbook::usd;
use openstance_core::resolve::lookup::{resolve_person, PersonKey};
use openstance_core::search;
use openstance_core::search::rules::parse_date_arg;

use crate::commands::Ctx;
use crate::UsageError;

/// `openstance enrich`.
#[derive(Debug, Args)]
pub struct EnrichArgs {
    /// Sub-action.
    #[command(subcommand)]
    pub action: EnrichAction,
}

/// `openstance enrich <action>`.
#[derive(Debug, Subcommand)]
pub enum EnrichAction {
    /// List what left the machine (and what was only priced), with costs.
    Log(LogArgs),
}

/// `openstance enrich log`.
#[derive(Debug, Args)]
pub struct LogArgs {
    /// Only this run id.
    #[arg(long, value_name = "RUN_ID")]
    pub run: Option<String>,
    /// Only rows since (12h, 7d, 6m, 1y, YYYY, YYYY-MM, YYYY-MM-DD).
    #[arg(long, value_name = "WHEN")]
    pub since: Option<String>,
    /// Only rows about this person.
    #[arg(long, value_name = "PERSON")]
    pub person: Option<String>,
}

/// Run `enrich <action>`.
pub fn run(ctx: &Ctx, a: EnrichArgs) -> Result<()> {
    match a.action {
        EnrichAction::Log(a) => log(ctx, a),
    }
}

/// `--since`: hours (`12h`) or any `parse_date_arg` form.
fn since(ctx: &Ctx, s: &str) -> Result<i64> {
    let t = s.trim().to_ascii_lowercase();
    if let Some(h) = t.strip_suffix('h').and_then(|n| n.parse::<i64>().ok()) {
        return Ok(ctx.now - h * 3600);
    }
    let tz = search::timezone(&ctx.loaded.config);
    let today = search::today_start(ctx.now, tz);
    parse_date_arg(&t, today, tz)
        .ok_or_else(|| anyhow::Error::new(UsageError(format!("could not parse --since {s:?}"))))
}

fn log(ctx: &Ctx, a: LogArgs) -> Result<()> {
    let g = ctx.open_default()?;
    let person = match &a.person {
        Some(p) => Some(resolve_person(g.conn(), &PersonKey::parse(p))?.person),
        None => None,
    };
    let filter = LogFilter {
        run: a.run.clone(),
        since: a.since.as_deref().map(|s| since(ctx, s)).transpose()?,
        person,
    };
    let r = g.enrich_log(&filter)?;
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&r)?);
    } else {
        print!("{}", render(&r, a.since.as_deref()));
    }
    Ok(())
}

/// The human table.
pub fn render(r: &EnrichLogResponse, since: Option<&str>) -> String {
    let mut s = String::new();
    if r.rows.is_empty() {
        let _ = writeln!(
            s,
            "no enrichment requests logged; nothing has left this machine"
        );
        return s;
    }
    let _ = writeln!(
        s,
        "{:<26}  {:<16}  {:<8}  {:<12}  {:<24}  {:>9}  status",
        "run", "at", "playbook", "op", "sent", "cost"
    );
    for row in &r.rows {
        let at = row.at.replace('T', " ");
        let at = at.get(..16).unwrap_or(&at);
        let about = row
            .subject()
            .map(|x| format!("({x})"))
            .unwrap_or_else(|| "-".into());
        let cost = match row.cost_micro_usd {
            Some(c) => usd(c),
            None => format!("~{}", usd(row.quoted_micro_usd)),
        };
        let status = match &row.error {
            Some(e) if row.status != "ok" => format!("{} ({e})", row.status),
            _ => row.status.clone(),
        };
        let _ = writeln!(
            s,
            "{:<26}  {:<16}  {:<8}  {:<12}  {:<24}  {:>9}  {status}",
            row.run_id,
            at,
            row.playbook.as_deref().unwrap_or("-"),
            row.op,
            about,
            cost,
        );
    }
    let t = &r.totals;
    let span = since.map(|x| format!(" since {x}")).unwrap_or_default();
    let _ = writeln!(
        s,
        "total{span}: {} request{} sent, {} spent ({} quoted only, {} blocked)",
        t.requests_sent,
        if t.requests_sent == 1 { "" } else { "s" },
        usd(t.spent_micro_usd),
        t.quoted_only,
        t.blocked
    );
    s
}
