//! Command handlers: load config, open the graph, call the facade, render.

use std::collections::BTreeSet;
use std::io::{BufRead, IsTerminal, Read, Write};

use anyhow::{Context, Result};
use openstance_core::config::{self, LoadOptions, LoadedConfig};
use openstance_core::export::{ExportFormat, ExportOptions};
use openstance_core::ingest::{ImportRequest, Touched};
use openstance_core::json::V;
use openstance_core::overrides::{self, Edit, OverrideField};
use openstance_core::resolve::lookup::{resolve_person, PersonKey};
use openstance_core::search::rules::{parse_date_arg, Lexicon, RuleParser};
use openstance_core::search::{self, SearchRequest};
use openstance_core::{
    Clock, Config, FixedClock, Graph, InitOptions, Providers, Query, QueryPatch, SystemClock,
};
use openstance_import::ImportOptions;

use crate::render;
use crate::{
    Cli, Command, ConfigAction, ConfigArgs, EditArgs, ExportArgs, FilterArgs, Format, ImportArgs,
    InitArgs, MergeArgs, ReviewArgs, SearchArgs, SplitArgs, UsageError,
};

/// Global state for one invocation.
pub(crate) struct Ctx {
    pub(crate) loaded: LoadedConfig,
    pub(crate) json: bool,
    pub(crate) now: i64,
    fixed_now: bool,
}

impl Ctx {
    fn clock(&self) -> Box<dyn Clock> {
        if self.fixed_now {
            Box::new(FixedClock(self.now))
        } else {
            Box::new(SystemClock)
        }
    }

    fn open(&self, cfg: Config, providers: Providers) -> Result<Graph> {
        let db = &self.loaded.paths.db;
        let g = Graph::open(cfg, db, providers, self.clock())
            .with_context(|| format!("opening {}", db.display()))?;
        if let Some(w) = g.embedding_warning() {
            eprintln!("warning: {w}");
        }
        Ok(g)
    }

    /// Providers for `cfg`: the configured embedder (loaded lazily) plus
    /// `importers`.
    pub(crate) fn providers(
        &self,
        cfg: &Config,
        importers: Vec<Box<dyn openstance_import::Importer>>,
    ) -> Result<Providers> {
        let mut p = Providers::with_importers(importers);
        p.models.embedder = crate::embeddings::embedder(cfg, &self.loaded.paths)?;
        // `enrich.backend` (default "none": no socket is ever opened).
        p.enricher = crate::playbook::enricher(cfg);
        Ok(p)
    }

    pub(crate) fn open_default(&self) -> Result<Graph> {
        let cfg = self.loaded.config.clone();
        let providers = self.providers(&cfg, Vec::new())?;
        self.open(cfg, providers)
    }

    /// `--now` was given (tests, reproducible reports).
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    pub(crate) fn fixed_now(&self) -> bool {
        self.fixed_now
    }

    pub(crate) fn today(&self) -> i64 {
        search::today_start(self.now, search::timezone(&self.loaded.config))
    }
}

pub(crate) fn usage(msg: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(UsageError(msg.into()))
}

/// Parse `--now`: unix seconds, `YYYY-MM-DD` (UTC midnight) or RFC 3339.
pub fn parse_now(s: &str) -> Result<i64> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i64>() {
        return Ok(n);
    }
    if let Ok(t) = time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339) {
        return Ok(t.unix_timestamp());
    }
    let f = time::macros::format_description!("[year]-[month]-[day]");
    if let Ok(d) = time::Date::parse(s, &f) {
        return Ok(d.midnight().assume_utc().unix_timestamp());
    }
    Err(usage(format!("could not parse --now {s:?}")))
}

pub fn dispatch(cli: Cli) -> Result<()> {
    let mut opts = LoadOptions::from_process_env();
    opts.config_path = cli.config.clone();
    opts.db_path = cli.db.clone();
    let loaded = config::load(&opts)?;
    for w in &loaded.warnings {
        eprintln!("warning: {w}");
    }
    let (now, fixed_now) = match &cli.now {
        Some(s) => (parse_now(s)?, true),
        None => (SystemClock.now(), false),
    };
    let ctx = Ctx {
        loaded,
        json: cli.json,
        now,
        fixed_now,
    };
    match cli.command {
        Command::Init(a) => init(&ctx, a),
        Command::Import(a) => import(&ctx, a),
        Command::Rebuild(a) => rebuild(&ctx, a),
        Command::Review(a) => review(&ctx, a),
        Command::Search(a) => search_cmd(&ctx, a),
        Command::Who(a) => crate::who::run(&ctx.open_default()?, ctx.json, a),
        Command::How(a) => crate::how::run(&ctx.open_default()?, ctx.json, a),
        Command::Edit(a) => edit(&ctx, a),
        Command::Merge(a) => merge(&ctx, a),
        Command::Split(a) => split(&ctx, a),
        Command::Export(a) => export(&ctx, a),
        Command::Config(a) => config_cmd(&ctx, a),
        Command::View(a) => crate::view::run(&ctx, a),
        Command::Keys(a) => crate::view::keys(&ctx, a),
        Command::Path(a) => crate::path::run(&ctx.open_default()?, ctx.json, a),
        Command::Brief(a) => crate::brief::run(&ctx.open_default()?, ctx.json, a),
        Command::Enrich(a) => crate::enrich::run(&ctx, a),
        Command::Serve(a) => crate::serve::run(&ctx, a),
        Command::Mcp(a) => crate::mcp::run(
            ctx.loaded.config.clone(),
            &ctx.loaded.paths.db,
            ctx.providers(&ctx.loaded.config, Vec::new())?,
            ctx.clock(),
            crate::view::dirs(&ctx),
            a,
        ),
    }
}

pub(crate) fn print_json<T: serde::Serialize>(v: &T) -> Result<()> {
    let s = serde_json::to_string_pretty(v)?;
    let mut out = std::io::stdout().lock();
    writeln!(out, "{s}")?;
    Ok(())
}

/// Serialize `v` and add `"v": 1` in front when it has no version field.
fn with_v<T: serde::Serialize>(v: &T) -> Result<serde_json::Value> {
    let inner = serde_json::to_value(v)?;
    let mut out = serde_json::Map::new();
    out.insert("v".into(), serde_json::json!(V));
    if let serde_json::Value::Object(m) = inner {
        for (k, val) in m {
            if k != "v" {
                out.insert(k, val);
            }
        }
    }
    Ok(serde_json::Value::Object(out))
}

// ---------------------------------------------------------------------------
// init / import / rebuild
// ---------------------------------------------------------------------------

fn init(ctx: &Ctx, a: InitArgs) -> Result<()> {
    let r = Graph::init(
        &ctx.loaded.config,
        &ctx.loaded.paths,
        &InitOptions { me: a.me },
        ctx.now,
    )?;
    // P3-CORE-09: new installs get their age key for views right away.
    let key = crate::view::ensure_key(ctx)?;
    // P4-API-02: new installs get their api token for `serve` right away.
    let token_created = crate::serve::ensure_token(ctx)?;
    if ctx.json {
        return print_json(&with_v(&r)?);
    }
    if token_created {
        println!(
            "created {} (api token for openstance serve)",
            crate::serve::token_path(ctx).display()
        );
    }
    if key.created {
        println!(
            "created {} (share: openstance keys show)",
            key.path.display()
        );
    }
    let state = |created: bool| if created { "created" } else { "exists " };
    println!(
        "{} {} (schema {})",
        state(r.created_db),
        r.db.display(),
        r.schema_version
    );
    println!("{} {}", state(r.created_config), r.config_file.display());
    if r.created_db {
        println!("next: openstance import ~/Downloads/takeout/*.mbox ~/Downloads/Connections.csv");
    }
    Ok(())
}

/// The importers compiled into this binary, configured from `cfg`.
#[cfg_attr(not(feature = "server"), allow(dead_code))]
pub(crate) fn importers(cfg: &Config) -> Vec<Box<dyn openstance_import::Importer>> {
    openstance_import::registry(&import_options(cfg, false))
}

fn import_options(cfg: &Config, bodies: bool) -> ImportOptions {
    ImportOptions {
        email_bodies: bodies || cfg.import.email_bodies,
        default_region: Some(cfg.import.default_region.trim().to_string())
            .filter(|r| !r.is_empty()),
        tz: search::timezone(cfg),
        max_message_bytes: u64::from(cfg.import.max_message_mb) * 1024 * 1024,
        ics_max_attendees: cfg.import.ics_max_attendees,
        me_identities: cfg.me.identities.clone(),
        ..ImportOptions::default()
    }
}

fn import(ctx: &Ctx, a: ImportArgs) -> Result<()> {
    let cfg = ctx.loaded.config.clone();
    let opts = import_options(&cfg, a.bodies);
    let providers = ctx.providers(&cfg, openstance_import::registry(&opts))?;
    let mut g = ctx.open(cfg, providers)?;
    crate::embeddings::show_progress(&mut g, ctx.json);
    let req = ImportRequest {
        kind: a.kind,
        label: a.label,
        replace: a.replace,
        no_resolve: a.no_resolve,
    };
    if !ctx.json && std::io::stderr().is_terminal() {
        eprintln!("importing {} file(s)...", a.files.len());
    }
    let resp = g.import(&a.files, &req).context("importing")?;
    crate::embeddings::print_refresh(&g, ctx.json);
    if ctx.json {
        return print_json(&resp);
    }
    let counts = g.counts()?;
    let preset = openstance_core::score::ScoreConfig::from_config(&g.config().score)
        .map(|(s, _)| s.preset_name().to_string())
        .unwrap_or_else(|_| "default".into());
    render::import_summary(&resp, &counts, &preset);
    Ok(())
}

fn rebuild(ctx: &Ctx, a: crate::embeddings::RebuildArgs) -> Result<()> {
    let mut g = ctx.open_default()?;
    if a.embeddings {
        return crate::embeddings::run(&mut g, ctx.json);
    }
    crate::embeddings::show_progress(&mut g, ctx.json);
    let r = g.rebuild()?;
    crate::embeddings::print_refresh(&g, ctx.json);
    if ctx.json {
        return print_json(&with_v(&r)?);
    }
    println!(
        "rebuilt {} people ({} flagged, {} bulk, {} ties) in {} ms",
        r.people, r.flagged, r.bulk, r.ties, r.took_ms
    );
    if r.blocked_by_override > 0 {
        println!("{} merges blocked by your edits", r.blocked_by_override);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// review
// ---------------------------------------------------------------------------

fn person_id(g: &Graph, s: &str) -> Result<i64> {
    Ok(resolve_person(g.conn(), &PersonKey::parse(s))?.person)
}

/// An anchor id as given (for undo commands), else the resolved person.
fn anchor_id(g: &Graph, s: &str) -> Result<i64> {
    match s.trim().parse::<i64>() {
        Ok(n) => Ok(n),
        Err(_) => person_id(g, s),
    }
}

fn review(ctx: &Ctx, a: ReviewArgs) -> Result<()> {
    let mut g = ctx.open_default()?;
    let now = ctx.now;
    let min_conf = a.min_confidence.or(a.accept_all_above);
    if a.confirm_all || a.accept_all_above.is_some() {
        let filter = openstance_core::resolve::review::ConfirmFilter {
            rule: a.rule.clone(),
            min_confidence: min_conf,
        };
        let n = g.write(|tx, cfg| {
            let n = openstance_core::resolve::review::confirm_all(tx, cfg, &filter, now)?;
            openstance_core::rebuild::rebuild(tx, cfg, now)?;
            Ok(n)
        })?;
        if ctx.json {
            return print_json(&serde_json::json!({"v": V, "confirmed": n}));
        }
        println!("confirmed {n} flagged merge(s)");
        return Ok(());
    }
    if let Some(p) = &a.confirm {
        let ids = g.review_confirm(&PersonKey::parse(p))?;
        if ctx.json {
            return print_json(&serde_json::json!({"v": V, "overrides": ids}));
        }
        println!("confirmed ({} override row(s) written)", ids.len());
        return Ok(());
    }
    if let Some(p) = &a.split {
        let id = person_id(&g, p)?;
        let out = split_flagged(&mut g, id)?;
        if ctx.json {
            return print_json(&serde_json::json!({"v": V, "kept": out.0, "moved": out.1}));
        }
        println!(
            "split: kept {} and moved the flagged identities to {}",
            out.0, out.1
        );
        return Ok(());
    }
    if let [x, y] = a.reject.as_slice() {
        let id = g.review_reject(&PersonKey::parse(x), &PersonKey::parse(y))?;
        if ctx.json {
            return print_json(&serde_json::json!({"v": V, "override": id}));
        }
        println!("kept apart (override #{id})");
        return Ok(());
    }

    let mut q = g.review()?;
    if let Some(m) = min_conf {
        q.flagged.retain(|f| f.confidence >= m - 1e-9);
        q.suggestions.retain(|s| s.confidence >= m - 1e-9);
    }
    let interactive =
        !ctx.json && !a.list && std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if ctx.json {
        return print_json(&q);
    }
    if !interactive {
        render::review_list(&q);
        return Ok(());
    }
    review_interactive(&mut g, &q)
}

fn split_flagged(g: &mut Graph, person: i64) -> Result<(i64, i64)> {
    let q = g.review()?;
    let f = q
        .flagged
        .iter()
        .find(|f| f.person.id == person)
        .ok_or_else(|| usage(format!("person {person} has no flagged merge")))?;
    let ids: Vec<i64> = f.flagged_identities.iter().map(|i| i.id).collect();
    let out = g.split(&PersonKey::Id(person), &ids)?;
    Ok((out.kept, out.moved))
}

fn review_interactive(g: &mut Graph, q: &openstance_core::json::ReviewQueue) -> Result<()> {
    let total = q.flagged.len();
    let stdin = std::io::stdin();
    for (i, f) in q.flagged.iter().enumerate() {
        eprintln!("{}", render::flagged_block(i + 1, total, f));
        loop {
            eprint!("[a]ccept  [s]plit  [k]skip  [q]uit > ");
            std::io::stderr().flush().ok();
            let mut line = String::new();
            if stdin.lock().read_line(&mut line)? == 0 {
                return Ok(());
            }
            match line.trim() {
                "a" => {
                    g.review_confirm(&PersonKey::Id(f.person.id))?;
                    eprintln!("accepted");
                    break;
                }
                "s" => {
                    split_flagged(g, f.person.id)?;
                    eprintln!("split");
                    break;
                }
                "k" | "" => break,
                "q" => return Ok(()),
                _ => continue,
            }
        }
    }
    if !q.suggestions.is_empty() {
        eprintln!(
            "{} suggestion(s); keep pairs apart with `openstance review --reject <a> <b>` or merge them with `openstance merge <a> <b>`",
            q.suggestions.len()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// search / who
// ---------------------------------------------------------------------------

fn date_flag(name: &str, v: &Option<String>, today: i64, ctx: &Ctx) -> Result<Option<i64>> {
    match v {
        None => Ok(None),
        Some(s) => parse_date_arg(s, today, search::timezone(&ctx.loaded.config))
            .map(Some)
            .ok_or_else(|| usage(format!("--{name}: could not parse date: '{s}'"))),
    }
}

/// Lay the flags over `patch` (flags always win).
pub(crate) fn apply_filters(ctx: &Ctx, f: &FilterArgs, patch: &mut QueryPatch) -> Result<()> {
    let today = ctx.today();
    let vec = |v: &Vec<String>| (!v.is_empty()).then(|| v.clone());
    if let Some(v) = vec(&f.company) {
        patch.company = Some(v);
    }
    if let Some(v) = vec(&f.title) {
        patch.title = Some(v);
    }
    if let Some(v) = vec(&f.location) {
        patch.location = Some(v);
    }
    if let Some(v) = vec(&f.industry) {
        patch.industry = Some(v);
    }
    if let Some(v) = vec(&f.tag) {
        patch.tags = Some(v);
    }
    if let Some(t) = date_flag("since", &f.since, today, ctx)? {
        patch.since = Some(t);
    }
    if let Some(t) = date_flag("until", &f.until, today, ctx)? {
        patch.until = Some(t);
    }
    if let Some(t) = date_flag("before", &f.before, today, ctx)? {
        patch.last_contact_before = Some(t);
    }
    if f.met || f.not_met {
        patch.met_in_person = Some(f.met);
    }
    if f.replied || f.not_replied {
        patch.replied = Some(f.replied);
    }
    if let Some(m) = f.min_score {
        if !(0.0..=1.0).contains(&m) {
            return Err(usage("--min-score must be between 0 and 1"));
        }
        patch.min_score = Some(m);
    }
    if f.lost_touch || f.not_lost_touch {
        patch.lost_touch = Some(f.lost_touch);
    }
    if f.include_bulk {
        patch.include_bulk = Some(true);
    }
    Ok(())
}

/// Every field of `q` as an explicit patch (`--query-json`).
fn full_patch(q: &Query) -> QueryPatch {
    QueryPatch {
        text: Some(q.text.clone()),
        company: Some(q.company.clone()),
        title: Some(q.title.clone()),
        location: Some(q.location.clone()),
        industry: Some(q.industry.clone()),
        tags: Some(q.tags.clone()),
        since: q.since,
        until: q.until,
        last_contact_before: q.last_contact_before,
        met_in_person: q.met_in_person,
        replied: q.replied,
        min_score: q.min_score,
        lost_touch: q.lost_touch,
        include_bulk: Some(q.include_bulk),
        limit: Some(q.limit),
    }
}

fn search_cmd(ctx: &Ctx, a: SearchArgs) -> Result<()> {
    let mut cfg = ctx.loaded.config.clone();
    if a.no_model {
        cfg.model.parser = "rules".into();
        cfg.model.embedder = "none".into();
        cfg.model.reranker = "none".into();
    }
    let providers = ctx.providers(&cfg, Vec::new())?;
    let g = ctx.open(cfg, providers)?;
    let mut patch = QueryPatch::default();
    let mut text = a.text.clone();
    let mut no_parse = a.no_parse;
    if let Some(qj) = &a.query_json {
        let raw = if qj == "-" {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        } else {
            qj.clone()
        };
        let q: Query =
            serde_json::from_str(&raw).map_err(|e| usage(format!("--query-json: {e}")))?;
        patch = full_patch(&q);
        if text.is_none() {
            text = q.text.clone();
        }
        no_parse = true;
    }
    apply_filters(ctx, &a.filters, &mut patch)?;
    if let Some(l) = a.limit {
        patch.limit = Some(l);
    }
    let req = SearchRequest {
        text,
        filters: patch,
        no_parse,
        explain: a.explain,
        today: ctx.today(),
        now: ctx.now,
    };
    let resp = g.search(&req)?;
    for w in &resp.warnings {
        eprintln!("warning: {w}");
    }
    if ctx.json {
        return print_json(&resp);
    }
    render::search_table(&resp, a.explain);
    Ok(())
}

// ---------------------------------------------------------------------------
// edit / merge / split
// ---------------------------------------------------------------------------

fn edit(ctx: &Ctx, a: EditArgs) -> Result<()> {
    let mut g = ctx.open_default()?;
    let key = PersonKey::parse(&a.person);
    if a.history {
        let p = resolve_person(g.conn(), &key)?.person;
        let cluster = overrides::cluster(g.conn(), p)?;
        let rows = overrides::history(g.conn(), &cluster)?;
        if ctx.json {
            let v: Vec<_> = rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id, "person_id": r.person_id, "field": r.field.as_str(),
                        "value": r.value, "set_at": openstance_core::json::iso(r.set_at),
                    })
                })
                .collect();
            return print_json(&serde_json::json!({"v": V, "person": p, "overrides": v}));
        }
        render::history(p, &rows);
        return Ok(());
    }
    let mut e = Edit::default();
    for (field, v) in [
        (OverrideField::DisplayName, &a.name),
        (OverrideField::Title, &a.title),
        (OverrideField::Company, &a.company),
        (OverrideField::Location, &a.location),
        (OverrideField::Bio, &a.bio),
    ] {
        if let Some(v) = v {
            e.set.push((field, v.clone()));
        }
    }
    for u in &a.unset {
        let f = OverrideField::parse(u.trim())
            .filter(|f| f.is_scalar())
            .ok_or_else(|| usage(format!("--unset: unknown field {u:?}")))?;
        e.unset.push(f);
    }
    e.add_tags = a.tag.clone();
    e.remove_tags = a.untag.clone();
    e.add_notes = a.note.clone();
    e.do_not_share = (a.no_share || a.share).then_some(a.no_share);
    e.no_enrich = (a.no_enrich || a.enrich).then_some(a.no_enrich);
    e.is_bulk = (a.bulk || a.not_bulk).then_some(a.bulk);
    if e == Edit::default() {
        return Err(usage("nothing to change; see `openstance edit --help`"));
    }
    let id = g.edit(&key, &e)?;
    if ctx.json {
        return print_json(&g.who(&PersonKey::Id(id))?);
    }
    println!("updated person {id}");
    Ok(())
}

fn merge(ctx: &Ctx, a: MergeArgs) -> Result<()> {
    let mut g = ctx.open_default()?;
    let now = ctx.now;
    if a.undo {
        let (x, y) = (anchor_id(&g, &a.a)?, anchor_id(&g, &a.b)?);
        let n = g.write(|tx, cfg| {
            let n = openstance_core::resolve::commands::unmerge(tx, x, y)?;
            openstance_core::rebuild::incremental(tx, cfg, &touched(&[x, y]), now)?;
            Ok(n)
        })?;
        if ctx.json {
            return print_json(&serde_json::json!({"v": V, "removed": n}));
        }
        println!("removed {n} merge row(s) between {x} and {y}");
        return Ok(());
    }
    let out = g.merge(&[PersonKey::parse(&a.a), PersonKey::parse(&a.b)])?;
    if ctx.json {
        return print_json(&g.who(&PersonKey::Id(out.person))?);
    }
    println!("merged into person {}", out.person);
    Ok(())
}

fn touched(ids: &[i64]) -> Touched {
    Touched {
        identities: Default::default(),
        people: ids.iter().copied().collect::<BTreeSet<_>>(),
    }
}

/// `kind:value` or an identity id -> identity id inside `person`.
fn identity_id(g: &Graph, person: i64, s: &str) -> Result<i64> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i64>() {
        return Ok(n);
    }
    let (kind, value) = s
        .split_once(':')
        .ok_or_else(|| usage(format!("--identity {s:?}: expected kind:value or an id")))?;
    let (kind, v) = (kind.trim(), value.trim());
    let idents = openstance_core::search::person::identities(g.conn(), person)?;
    let hit = idents
        .iter()
        .find(|i| i.kind == kind && i.value == v)
        .or_else(|| {
            idents
                .iter()
                .find(|i| i.kind == kind && i.value.eq_ignore_ascii_case(v))
        });
    if let Some(i) = hit {
        return Ok(i.id);
    }
    Err(openstance_core::Error::NotFound(format!("identity {kind}:… in person {person}")).into())
}

fn split(ctx: &Ctx, a: SplitArgs) -> Result<()> {
    let mut g = ctx.open_default()?;
    let now = ctx.now;
    if a.undo {
        let other = a.other.as_deref().unwrap_or_default();
        let (x, y) = (person_id(&g, &a.person)?, person_id(&g, other)?);
        let n = g.write(|tx, cfg| {
            let n = openstance_core::resolve::commands::unsplit(tx, x, y, true)?;
            openstance_core::rebuild::incremental(tx, cfg, &touched(&[x, y]), now)?;
            Ok(n)
        })?;
        if ctx.json {
            return print_json(&serde_json::json!({"v": V, "removed": n}));
        }
        println!("removed {n} split row(s) between {x} and {y}");
        return Ok(());
    }
    let p = person_id(&g, &a.person)?;
    let mut ids = Vec::new();
    for s in &a.identity {
        ids.push(identity_id(&g, p, s)?);
    }
    if ids.is_empty() {
        if ctx.json || !std::io::stdin().is_terminal() {
            return Err(usage("split needs --identity (or run it in a terminal)"));
        }
        ids = pick_identities(&g, p)?;
        if ids.is_empty() {
            return Ok(());
        }
    }
    let out = g.split(&PersonKey::Id(p), &ids)?;
    if ctx.json {
        return print_json(&serde_json::json!({
            "v": V, "kept": out.kept, "moved": out.moved, "swapped": out.swapped
        }));
    }
    println!("kept person {}; new person {}", out.kept, out.moved);
    Ok(())
}

fn pick_identities(g: &Graph, p: i64) -> Result<Vec<i64>> {
    let idents = openstance_core::search::person::identities(g.conn(), p)?;
    for (i, id) in idents.iter().enumerate() {
        eprintln!("  [{}] {}:{}", i + 1, id.kind, id.value);
    }
    eprint!("identities to move (numbers, space-separated; empty to cancel) > ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let mut out = Vec::new();
    for tok in line.split_whitespace() {
        let n: usize = tok
            .parse()
            .map_err(|_| usage(format!("not a number: {tok}")))?;
        let id = idents
            .get(n.wrapping_sub(1))
            .ok_or_else(|| usage(format!("no identity [{n}]")))?;
        out.push(id.id);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// export / config
// ---------------------------------------------------------------------------

fn export(ctx: &Ctx, a: ExportArgs) -> Result<()> {
    let g = ctx.open_default()?;
    let mut q = match &a.query {
        Some(text) => {
            let parser = RuleParser::new(
                Lexicon::load(g.conn())?,
                search::timezone(&ctx.loaded.config),
            );
            let p = parser.parse_detailed(text, ctx.today());
            for w in &p.warnings {
                eprintln!("warning: {w}");
            }
            p.query
        }
        None => Query::default(),
    };
    let mut patch = QueryPatch::default();
    apply_filters(ctx, &a.filters, &mut patch)?;
    patch.apply(&mut q);
    let opts = ExportOptions {
        include_bulk: a.filters.include_bulk,
        include_do_not_share: a.include_do_not_share,
        query: Some(q),
    };
    let format = match a.format {
        Format::Csv => ExportFormat::Csv,
        Format::Json => ExportFormat::Json,
        Format::Vcf => ExportFormat::Vcf,
    };
    let report = match &a.out {
        Some(path) => {
            let mut f = std::fs::File::create(path)
                .with_context(|| format!("creating {}", path.display()))?;
            let r = g.export(format, &opts, &mut f)?;
            f.flush()?;
            r
        }
        None => {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            let r = g.export(format, &opts, &mut lock)?;
            lock.flush()?;
            r
        }
    };
    eprintln!(
        "exported {} people ({} excluded as do-not-share)",
        report.people, report.excluded_do_not_share
    );
    Ok(())
}

fn config_cmd(ctx: &Ctx, a: ConfigArgs) -> Result<()> {
    let l = &ctx.loaded;
    match a.action {
        Some(ConfigAction::Path) => {
            if ctx.json {
                return print_json(&serde_json::json!({
                    "v": V,
                    "config_file": l.paths.config_file,
                    "db": l.paths.db,
                    "home": l.paths.home,
                    "file_found": l.file_found,
                }));
            }
            println!("config  {}", l.paths.config_file.display());
            println!("db      {}", l.paths.db.display());
            println!("home    {}", l.paths.home.display());
            Ok(())
        }
        Some(ConfigAction::Get { key }) => {
            let v = toml::Value::try_from(&l.config)?;
            let mut cur = &v;
            for part in key.split('.') {
                cur = cur
                    .get(part)
                    .ok_or_else(|| openstance_core::Error::NotFound(format!("config key {key}")))?;
            }
            if ctx.json {
                return print_json(&serde_json::json!({"v": V, "key": key, "value": cur}));
            }
            match cur {
                toml::Value::String(s) => println!("{s}"),
                toml::Value::Table(t) => print!("{}", toml::to_string_pretty(t)?),
                other => println!("{other}"),
            }
            Ok(())
        }
        None => {
            if ctx.json {
                return print_json(&config_json(l));
            }
            print!("{}", render::annotated_config(&l.config, &l.origins));
            Ok(())
        }
    }
}

/// `openstance config --json` (also `GET /config`).
pub(crate) fn config_json(l: &LoadedConfig) -> serde_json::Value {
    serde_json::json!({
        "v": V,
        "config": l.config,
        "origins": l.origins,
        "warnings": l.warnings,
        "config_file": l.paths.config_file,
        "file_found": l.file_found,
        // Resolved locations (`$OPENSTANCE_HOME`, `--db`), unlike the
        // `config.db.path` literal.
        "home": l.paths.home,
        "db_path": l.paths.db,
    })
}
