//! Embedder wiring and `openstance rebuild --embeddings` (P2-MODEL-02,
//! P2-CLI-03).
//!
//! - [`embedder`]: `model.embedder` -> `Box<dyn Embedder>` through the
//!   model factory (lazy: nothing loads until the first embed call);
//! - [`run`]: the forced embedding pass behind `rebuild --embeddings`;
//! - [`show_progress`]: the `embedding 120/612` line on stderr during
//!   `import` and `rebuild` (TTY only).

use std::io::{IsTerminal, Write};

use anyhow::{bail, Result};
use clap::Args;
use openstance_core::config::{expand_path, Paths};
use openstance_core::embed::{EmbedReport, IndexOptions};
use openstance_core::json::V;
use openstance_core::{Config, Graph};
use openstance_model::embed::{embedder_from_spec, EmbedderOptions};
use openstance_model::Embedder;

/// `openstance rebuild` flags.
#[derive(Debug, Args, Default, Clone)]
pub struct RebuildArgs {
    /// Re-embed every person with the configured embedder (recreates the
    /// vector table when the embedder or its dims changed).
    #[arg(long)]
    pub embeddings: bool,
}

/// The configured embedder. `model.cache_dir` (or `OPENSTANCE_MODEL_DIR`)
/// is the model cache; `OPENSTANCE_OFFLINE=1` forbids downloads.
pub fn embedder(cfg: &Config, paths: &Paths) -> Result<Box<dyn Embedder>> {
    let user_home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let cache = expand_path(&cfg.model.cache_dir, &paths.home, user_home.as_deref());
    let mut opts = EmbedderOptions::from_env(cache);
    opts.show_progress = std::io::stderr().is_terminal();
    embedder_from_spec(&cfg.model.embedder, &opts)
        .map_err(|e| crate::UsageError(format!("model.embedder: {e}")).into())
}

/// Print `embedding done/total` on stderr while a pass runs (TTY only).
pub fn show_progress(g: &mut Graph, json: bool) {
    if json || !std::io::stderr().is_terminal() {
        return;
    }
    g.on_embed_progress(|p| {
        let mut e = std::io::stderr().lock();
        let _ = write!(e, "\rembedding {}/{} people", p.done, p.total);
        if p.done == p.total {
            let _ = writeln!(e);
        }
        let _ = e.flush();
    });
}

/// One line about an automatic refresh (after `import` / `rebuild`).
pub fn print_refresh(g: &Graph, json: bool) {
    let Some(r) = g.last_embed_report() else {
        return;
    };
    if let Some(w) = r.warning() {
        eprintln!("warning: {w}");
    }
    if !json && r.embedded > 0 {
        eprintln!(
            "embedded {} people with {} in {} ms",
            r.embedded, r.embedder, r.took_ms
        );
    }
}

/// `openstance rebuild --embeddings`.
pub fn run(g: &mut Graph, json: bool) -> Result<()> {
    if g.providers().models.embedder.dims() == 0 {
        if json {
            let r = EmbedReport {
                embedder: "none".into(),
                skipped: Some("no embedder configured".into()),
                ..EmbedReport::default()
            };
            return print_json(&with_v(&r)?);
        }
        println!("model.embedder = \"none\": nothing to embed (search uses full-text only)");
        return Ok(());
    }
    show_progress(g, json);
    let r = g.embed(&IndexOptions {
        force: true,
        ..IndexOptions::default()
    })?;
    if let Some(e) = &r.error {
        bail!("embedding failed after {} people: {e}", r.embedded);
    }
    if json {
        return print_json(&with_v(&r)?);
    }
    println!(
        "embedded {} people with {} in {} ms{}",
        r.embedded,
        r.embedder,
        r.took_ms,
        if r.recreated {
            " (vector table recreated)"
        } else {
            ""
        }
    );
    if r.removed > 0 {
        println!("removed {} stale vectors", r.removed);
    }
    Ok(())
}

fn print_json(v: &serde_json::Value) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", serde_json::to_string_pretty(v)?)?;
    Ok(())
}

fn with_v(r: &EmbedReport) -> Result<serde_json::Value> {
    let mut v = serde_json::json!({ "v": V });
    if let (Some(m), serde_json::Value::Object(inner)) =
        (v.as_object_mut(), serde_json::to_value(r)?)
    {
        m.extend(inner);
    }
    Ok(v)
}
