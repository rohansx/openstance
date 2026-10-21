//! Embedder backends and the config factory (P2-MODEL-01..03).
//! Spec: `docs/tech-spec/models.md` sections 2.2, 3 and 7.
//!
//! - [`embedder_from_spec`] maps a `model.embedder` string to a
//!   `Box<dyn Embedder>` (`none` -> [`NullEmbedder`]);
//! - [`HashEmbedder`]: deterministic token-hash vectors for CI and tests (no
//!   model, no network);
//! - `FastEmbedder` (feature `fastembed`): bge-small-en-v1.5 via ONNX
//!   Runtime, loaded lazily on the first `embed` call;
//! - [`UnavailableEmbedder`]: keeps the configured id and dims (so re-index
//!   detection still works) when a backend cannot run in this build.

use std::path::PathBuf;

use crate::{EmbedKind, Embedder, ModelError, NullEmbedder, Result};

mod hash;
mod prefixes;

#[cfg(feature = "fastembed")]
mod fastembed;

#[cfg(feature = "fastembed")]
pub use self::fastembed::FastEmbedder;
pub use hash::HashEmbedder;
pub use prefixes::query_prefix;

/// The default embedder id.
pub const DEFAULT_EMBEDDER: &str = "fastembed:bge-small-en-v1.5";

/// The hint every "model missing" error carries (P2-MODEL-01).
pub const OFFLINE_HINT: &str = "run once with network or set `model.embedder = \"none\"`";

/// Default texts per ONNX batch (P2-MODEL-03).
pub const DEFAULT_BATCH_SIZE: usize = 64;

/// How backends are constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedderOptions {
    /// Model cache root (`~/.openstance/models`); fastembed uses `fastembed/`
    /// under it.
    pub cache_dir: PathBuf,
    /// Never download (`OPENSTANCE_OFFLINE=1` / `HF_HUB_OFFLINE=1`).
    pub offline: bool,
    /// Show a download progress bar on stderr.
    pub show_progress: bool,
    /// Texts per inference batch.
    pub batch_size: usize,
    /// ONNX Runtime intra-op threads (`OPENSTANCE_EMBED_THREADS`); `None`
    /// uses every core.
    pub threads: Option<usize>,
}

impl Default for EmbedderOptions {
    fn default() -> Self {
        EmbedderOptions {
            cache_dir: PathBuf::from("models"),
            offline: false,
            show_progress: false,
            batch_size: DEFAULT_BATCH_SIZE,
            threads: None,
        }
    }
}

impl EmbedderOptions {
    /// Options from the process environment: `OPENSTANCE_MODEL_DIR`
    /// overrides `cache_dir`, `OPENSTANCE_OFFLINE` / `HF_HUB_OFFLINE` forbid
    /// downloads, `OPENSTANCE_EMBED_THREADS` and `OPENSTANCE_EMBED_BATCH`
    /// tune throughput.
    pub fn from_env(cache_dir: PathBuf) -> Self {
        let flag = |k: &str| {
            std::env::var(k)
                .map(|v| matches!(v.trim(), "1" | "true" | "yes"))
                .unwrap_or(false)
        };
        let num = |k: &str| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|n| *n > 0)
        };
        EmbedderOptions {
            cache_dir: std::env::var_os("OPENSTANCE_MODEL_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or(cache_dir),
            offline: flag("OPENSTANCE_OFFLINE") || flag("HF_HUB_OFFLINE"),
            show_progress: false,
            batch_size: num("OPENSTANCE_EMBED_BATCH").unwrap_or(DEFAULT_BATCH_SIZE),
            threads: num("OPENSTANCE_EMBED_THREADS"),
        }
    }
}

/// Scale `v` to unit length in place (a zero vector stays zero).
pub fn l2_normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 && n.is_finite() {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
}

/// Map a `model.embedder` spec (`backend:model`) to an embedder. Nothing is
/// loaded here: model files are read on the first `embed` call.
pub fn embedder_from_spec(spec: &str, opts: &EmbedderOptions) -> Result<Box<dyn Embedder>> {
    let spec = spec.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("none") {
        return Ok(Box::new(NullEmbedder));
    }
    let (backend, model) = spec
        .split_once(':')
        .ok_or_else(|| invalid(format!("embedder {spec:?}: expected backend:model or none")))?;
    match backend {
        "hash" => {
            let dims: usize = model
                .parse()
                .ok()
                .filter(|d| (1..=4096).contains(d))
                .ok_or_else(|| invalid(format!("embedder {spec:?}: hash:<dims>")))?;
            Ok(Box::new(HashEmbedder::new(dims)))
        }
        "fastembed" => fastembed_backend(spec, model, opts),
        "ollama" | "openai" => Ok(Box::new(UnavailableEmbedder {
            id: spec.to_string(),
            dims: 0,
            reason: format!("the {backend} embedder is not available in this version"),
        })),
        "anthropic" => Err(invalid(
            "Anthropic has no embeddings endpoint; use fastembed, ollama or openai".into(),
        )),
        _ => Err(invalid(format!("unknown embedder backend {backend:?}"))),
    }
}

/// Dims of a known fastembed model.
fn fastembed_dims(model: &str) -> Option<usize> {
    match model {
        "bge-small-en-v1.5" => Some(384),
        _ => None,
    }
}

fn fastembed_backend(spec: &str, model: &str, opts: &EmbedderOptions) -> Result<Box<dyn Embedder>> {
    let dims = fastembed_dims(model).ok_or_else(|| {
        invalid(format!(
            "unknown fastembed model {model:?}; supported: bge-small-en-v1.5"
        ))
    })?;
    #[cfg(feature = "fastembed")]
    {
        let _ = (dims, spec);
        Ok(Box::new(FastEmbedder::new(model, opts)?))
    }
    #[cfg(not(feature = "fastembed"))]
    {
        let _ = opts;
        Ok(Box::new(UnavailableEmbedder {
            id: spec.to_string(),
            dims,
            reason: format!("this build has no fastembed support; {OFFLINE_HINT}"),
        }))
    }
}

fn invalid(msg: String) -> ModelError {
    ModelError::Other(anyhow::anyhow!(msg))
}

/// A configured backend that cannot run here. Keeps the id and dims so the
/// stored-vs-configured check still works; every call returns
/// [`ModelError::Unavailable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableEmbedder {
    /// The configured id.
    pub id: String,
    /// The dims the backend would have (0 when unknown).
    pub dims: usize,
    /// Why it cannot run.
    pub reason: String,
}

impl Embedder for UnavailableEmbedder {
    fn id(&self) -> &str {
        &self.id
    }
    fn dims(&self) -> usize {
        self.dims
    }
    fn is_remote(&self) -> bool {
        false
    }
    fn embed(&self, _texts: &[&str], _kind: EmbedKind) -> Result<Vec<Vec<f32>>> {
        Err(ModelError::Unavailable(self.reason.clone()))
    }
}
