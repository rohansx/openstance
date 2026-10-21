//! `openstance view create|list|show|export|import|search|rm` and
//! `openstance keys show` (P3-CLI-03). Spec: interfaces.md section 9 and
//! sharing-and-views.md.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use openstance_core::keys;
use openstance_core::search::rules::{Lexicon, RuleParser};
use openstance_core::search::{self};
use openstance_core::view::inbox::{self, ViewQuery};
use openstance_core::view::ost::ExportOptions;
use openstance_core::view::{
    self, parse_ttl, CreateView, Direction, ShareDirs, Ttl, ViewList, ViewOwner, ViewSummary,
};
use openstance_core::{Graph, Query, QueryPatch};

use crate::commands::Ctx;
use crate::FilterArgs;

/// `openstance keys`.
#[derive(Debug, Args)]
pub struct KeysArgs {
    /// Sub-action.
    #[command(subcommand)]
    pub action: KeysAction,
}

/// `openstance keys <action>`.
#[derive(Debug, Subcommand)]
pub enum KeysAction {
    /// Print my age public key (created on first use) to share with people
    /// who will send me views.
    Show,
}

/// `openstance view`.
#[derive(Debug, Args)]
pub struct ViewArgs {
    /// Sub-action.
    #[command(subcommand)]
    pub action: ViewAction,
}

/// `openstance view <action>`.
#[derive(Debug, Subcommand)]
pub enum ViewAction {
    /// Save a query as a shareable view.
    Create(Box<CreateArgs>),
    /// List own and imported views.
    List(ListArgs),
    /// Show a view definition and how many people an export writes now.
    Show(NameArg),
    /// Write an encrypted .ost file of a view (stdout when not a TTY).
    Export(ExportArgs),
    /// Import a .ost file someone sent you (never merged into your graph).
    Import(ImportArgs),
    /// Search imported views (`*` searches all of them).
    Search(SearchArgs),
    /// Delete an own view definition or an imported view's local copy.
    Rm(NameArg),
}

/// `view create`.
#[derive(Debug, Args)]
pub struct CreateArgs {
    /// View name.
    pub name: String,
    /// Search text (parsed like `search`) or a full Query as JSON.
    #[arg(long, value_name = "TEXT|JSON")]
    pub query: Option<String>,
    /// Structured filters (win over parsed values).
    #[command(flatten)]
    pub filters: FilterArgs,
    /// Expiry: Nd, Nw, Nm, Ny or never (default share.default_ttl).
    #[arg(long)]
    pub ttl: Option<String>,
    /// Recipient: age1... key, `self`, or a [share.recipients] alias (repeatable).
    #[arg(long, value_name = "RECIPIENT")]
    pub to: Vec<String>,
    /// Replace an existing view and allow --ttl never.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

/// `view list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    /// own, imported or all.
    #[arg(long, default_value = "all")]
    pub direction: String,
}

/// A view name.
#[derive(Debug, Args)]
pub struct NameArg {
    /// View name (or id for imported views).
    pub name: String,
}

/// `view export`.
#[derive(Debug, Args)]
pub struct ExportArgs {
    /// View name.
    pub name: String,
    /// Output file (default stdout when it is not a terminal).
    #[arg(short = 'o', long = "out", value_name = "FILE")]
    pub out: Option<PathBuf>,
    /// ASCII-armored output for pasting into text channels.
    #[arg(long)]
    pub armor: bool,
}

/// `view import`.
#[derive(Debug, Args)]
pub struct ImportArgs {
    /// The .ost file.
    pub file: PathBuf,
}

/// `view search`.
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// View name or id, or `*`.
    pub view: String,
    /// Free text.
    pub text: Option<String>,
    /// Company contains (repeatable).
    #[arg(long)]
    pub company: Vec<String>,
    /// Title contains (repeatable).
    #[arg(long)]
    pub title: Vec<String>,
    /// Maximum results.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
}

#[derive(serde::Serialize)]
struct KeysShow {
    v: u32,
    public_key: String,
    path: PathBuf,
    created: bool,
}

fn dirs(ctx: &Ctx) -> ShareDirs {
    let user_home = std::env::var_os("HOME").map(PathBuf::from);
    ShareDirs::new(&ctx.loaded.config, &ctx.loaded.paths, user_home.as_deref())
}

/// Load (or create) my keypair.
pub(crate) fn ensure_key(ctx: &Ctx) -> Result<keys::KeyPair> {
    let path = dirs(ctx).key;
    keys::ensure(&path, ctx.now).with_context(|| format!("age key {}", path.display()))
}

/// `openstance keys show`.
pub fn keys(ctx: &Ctx, a: KeysArgs) -> Result<()> {
    match a.action {
        KeysAction::Show => {
            let k = ensure_key(ctx)?;
            if ctx.json {
                return crate::commands::print_json(&KeysShow {
                    v: openstance_core::json::V,
                    public_key: k.public(),
                    path: k.path.clone(),
                    created: k.created,
                });
            }
            if k.created {
                eprintln!("created {}", k.path.display());
            }
            println!("{}", k.public());
            Ok(())
        }
    }
}

/// `openstance view ...`.
pub fn run(ctx: &Ctx, a: ViewArgs) -> Result<()> {
    let d = dirs(ctx);
    // Cooperative expiry: purge expired imported views on every view command.
    for name in inbox::purge_expired(&d.views, ctx.now)? {
        eprintln!("deleted expired imported view {name}");
    }
    match a.action {
        ViewAction::Create(a) => create(ctx, *a),
        ViewAction::List(a) => list(ctx, &d, a),
        ViewAction::Show(a) => show(ctx, a),
        ViewAction::Export(a) => export(ctx, a),
        ViewAction::Import(a) => import(ctx, &d, a),
        ViewAction::Search(a) => search_views(ctx, &d, a),
        ViewAction::Rm(a) => rm(ctx, &d, a),
    }
}

fn owner(ctx: &Ctx) -> Result<ViewOwner> {
    let user = std::env::var("USER").ok();
    Ok(ViewOwner {
        pubkey: Some(ensure_key(ctx)?.public()),
        label: Some(view::owner_label(&ctx.loaded.config, user.as_deref())),
    })
}

fn summary_of(g: &Graph, ctx: &Ctx, name: &str) -> Result<ViewSummary> {
    g.view_summaries(&owner(ctx)?)?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| anyhow::anyhow!("view {name:?} vanished"))
}

fn parse_query(ctx: &Ctx, g: &Graph, text: Option<&str>) -> Result<Query> {
    match text.map(str::trim) {
        Some(t) if t.starts_with('{') => serde_json::from_str(t)
            .map_err(|e| crate::commands::usage(format!("--query is not a valid Query: {e}"))),
        Some(t) if !t.is_empty() => {
            let parser = RuleParser::new(
                Lexicon::load(g.conn())?,
                search::timezone(&ctx.loaded.config),
            );
            let p = parser.parse_detailed(t, ctx.today());
            for w in &p.warnings {
                eprintln!("warning: {w}");
            }
            Ok(p.query)
        }
        _ => Ok(Query::default()),
    }
}

fn create(ctx: &Ctx, a: CreateArgs) -> Result<()> {
    let ttl_text = a
        .ttl
        .clone()
        .unwrap_or_else(|| ctx.loaded.config.share.default_ttl.clone());
    let ttl = parse_ttl(&ttl_text)?;
    if ttl == Ttl::Never {
        if !a.yes {
            return Err(crate::commands::usage(
                "--ttl never shares the view with no expiry; pass --yes to confirm",
            ));
        }
        eprintln!("warning: this view never expires");
    }
    let mut g = ctx.open_default()?;
    let mut q = parse_query(ctx, &g, a.query.as_deref())?;
    let mut patch = QueryPatch::default();
    crate::commands::apply_filters(ctx, &a.filters, &mut patch)?;
    patch.apply(&mut q);
    let self_pk = if a.to.iter().any(|t| t == "self") {
        Some(ensure_key(ctx)?.public())
    } else {
        None
    };
    let recipients = view::resolve_recipients(&ctx.loaded.config, &a.to, self_pk.as_deref())?;
    let replace = a.yes;
    let def = g.view_create(&CreateView {
        name: a.name.clone(),
        query: q,
        ttl,
        recipients,
        replace,
    })?;
    let s = summary_of(&g, ctx, &def.name)?;
    if ctx.json {
        return crate::commands::print_json(&with_v(&s)?);
    }
    let expires = s
        .expires_at
        .as_deref()
        .map_or("never".to_string(), |e| e.chars().take(10).collect());
    println!(
        "view {}: {} people ({} excluded: do_not_share), expires {}",
        s.name,
        s.rows,
        s.excluded_do_not_share.unwrap_or(0),
        expires
    );
    if def.recipients.is_empty() {
        eprintln!("no --to given: exports are encrypted to your own key");
    }
    Ok(())
}

fn with_v<T: serde::Serialize>(v: &T) -> Result<serde_json::Value> {
    let mut out = serde_json::Map::new();
    out.insert("v".into(), serde_json::json!(openstance_core::json::V));
    if let serde_json::Value::Object(m) = serde_json::to_value(v)? {
        out.extend(m);
    }
    Ok(serde_json::Value::Object(out))
}

fn list(ctx: &Ctx, d: &ShareDirs, a: ListArgs) -> Result<()> {
    let dir = Direction::parse(&a.direction)
        .ok_or_else(|| crate::commands::usage("--direction must be own, imported or all"))?;
    let mut views = Vec::new();
    if dir != Direction::Imported {
        let g = ctx.open_default()?;
        views.extend(g.view_summaries(&owner(ctx)?)?);
    }
    if dir != Direction::Own {
        views.extend(inbox::list(&d.views, ctx.now)?);
    }
    if ctx.json {
        return crate::commands::print_json(&ViewList {
            v: openstance_core::json::V,
            views,
        });
    }
    if views.is_empty() {
        println!("no views");
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    for v in &views {
        let expires = v
            .expires_at
            .as_deref()
            .map_or("never".to_string(), |e| e.chars().take(10).collect());
        let who = match v.direction.as_str() {
            "imported" => format!(
                "from {} (claimed)",
                v.owner.label.as_deref().unwrap_or("unknown")
            ),
            _ => format!("to {} recipient(s)", v.recipients.len().max(1)),
        };
        writeln!(
            out,
            "{:<9} {:<28} {:>5} people  expires {}  {}",
            v.direction, v.name, v.rows, expires, who
        )?;
    }
    Ok(())
}

fn show(ctx: &Ctx, a: NameArg) -> Result<()> {
    let g = ctx.open_default()?;
    g.view_get(&a.name)?;
    let s = summary_of(&g, ctx, &a.name)?;
    if ctx.json {
        return crate::commands::print_json(&with_v(&s)?);
    }
    println!("view       {}", s.name);
    println!(
        "id         {}",
        s.id.as_deref().unwrap_or("(not exported yet)")
    );
    println!("created    {}", s.created_at);
    println!("expires    {}", s.expires_at.as_deref().unwrap_or("never"));
    println!(
        "people     {} (excluded: {} do_not_share)",
        s.rows,
        s.excluded_do_not_share.unwrap_or(0)
    );
    if s.recipients.is_empty() {
        println!("recipients self");
    } else {
        for r in &s.recipients {
            println!("recipient  {r}");
        }
    }
    println!("query      {}", serde_json::to_string(&s.query)?);
    Ok(())
}

fn export(ctx: &Ctx, a: ExportArgs) -> Result<()> {
    if a.out.is_none() && std::io::stdout().is_terminal() {
        return Err(crate::commands::usage(
            "refusing to write an encrypted view to a terminal; pass -o FILE or redirect stdout",
        ));
    }
    let key = ensure_key(ctx)?;
    let pk = key.public();
    let user = std::env::var("USER").ok();
    let label = view::owner_label(&ctx.loaded.config, user.as_deref());
    let mut g = ctx.open_default()?;
    let out = g.view_export(
        &a.name,
        &ExportOptions {
            owner_pubkey: &pk,
            owner_label: &label,
            armor: a.armor || ctx.loaded.config.share.armor,
            tmp_parent: None,
            version: env!("CARGO_PKG_VERSION"),
        },
    )?;
    match &a.out {
        Some(path) => {
            std::fs::write(path, &out.bytes)
                .with_context(|| format!("writing {}", path.display()))?;
        }
        None => {
            let mut so = std::io::stdout().lock();
            so.write_all(&out.bytes)?;
            so.flush()?;
        }
    }
    let r = &out.report;
    let first = r.recipients.first().map(String::as_str).unwrap_or("");
    eprintln!(
        "wrote {} people, {} identities, encrypted to {} recipient{} ({}{})",
        r.people,
        r.identities,
        r.recipients.len(),
        if r.recipients.len() == 1 { "" } else { "s" },
        short_key(first),
        if r.recipients.len() > 1 { ", ..." } else { "" }
    );
    if r.excluded_do_not_share > 0 {
        eprintln!("excluded {} do-not-share people", r.excluded_do_not_share);
    }
    eprintln!("note: expiry is advisory for offline files");
    Ok(())
}

fn short_key(k: &str) -> String {
    if k.len() > 16 {
        format!("{}...{}", &k[..10], &k[k.len() - 4..])
    } else {
        k.to_string()
    }
}

fn import(ctx: &Ctx, d: &ShareDirs, a: ImportArgs) -> Result<()> {
    let bytes = std::fs::read(&a.file).with_context(|| format!("reading {}", a.file.display()))?;
    let key = ensure_key(ctx)?;
    let r = inbox::import(
        &d.views,
        &key.identity,
        &bytes,
        inbox::max_bytes(ctx.loaded.config.share.max_import_mb),
        ctx.now,
    )?;
    if ctx.json {
        return crate::commands::print_json(&r);
    }
    let what = match r.status {
        inbox::ImportStatus::Imported => "imported",
        inbox::ImportStatus::Replaced => "replaced older copy of",
        inbox::ImportStatus::KeptNewer => "kept the newer copy already imported of",
    };
    println!(
        "{what} view {} ({} people), claimed owner: {} ({})",
        r.view.name,
        r.view.rows,
        r.view.owner.label.as_deref().unwrap_or("unknown"),
        short_key(r.view.owner.pubkey.as_deref().unwrap_or(""))
    );
    println!(
        "expires {}",
        r.view.expires_at.as_deref().unwrap_or("never")
    );
    Ok(())
}

fn search_views(ctx: &Ctx, d: &ShareDirs, a: SearchArgs) -> Result<()> {
    let res = inbox::search(
        &d.views,
        &ViewQuery {
            view: a.view,
            text: a.text,
            company: a.company,
            title: a.title,
            limit: Some(a.limit),
        },
        ctx.now,
    )?;
    if ctx.json {
        return crate::commands::print_json(&res);
    }
    if res.results.is_empty() {
        println!("no matches");
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    for r in &res.results {
        let p = &r.person;
        let role = [p.title.as_deref(), p.company.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
        let mut via = vec![r.view.name.clone()];
        via.extend(r.also_in.iter().map(|v| v.name.clone()));
        writeln!(
            out,
            "{:<24} {:<40} {:<6} {}  via {}",
            p.display_name.as_deref().unwrap_or("?"),
            role,
            p.score_bucket,
            p.last_contact_month.as_deref().unwrap_or("-"),
            via.join(", ")
        )?;
        let mut handles = p.emails.clone();
        handles.extend(p.linkedin.iter().map(|l| format!("linkedin.com/in/{l}")));
        if !handles.is_empty() {
            writeln!(out, "    {}", handles.join("  "))?;
        }
    }
    Ok(())
}

fn rm(ctx: &Ctx, d: &ShareDirs, a: NameArg) -> Result<()> {
    let mut g = ctx.open_default()?;
    if g.view_get(&a.name).is_ok() {
        g.view_remove(&a.name)?;
        println!(
            "deleted view {} (files already sent are unaffected)",
            a.name
        );
        return Ok(());
    }
    inbox::remove(&d.views, &a.name, ctx.now)?;
    println!("deleted the local copy of imported view {}", a.name);
    Ok(())
}
