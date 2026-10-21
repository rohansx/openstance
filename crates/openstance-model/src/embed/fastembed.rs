//! `FastEmbedder`: bge-small-en-v1.5 (int8-quantized ONNX, 384 dims) via the
//! `fastembed` crate, in-process on CPU (P2-MODEL-01, P2-MODEL-03).
//!
//! The model is loaded lazily on the first `embed` call from
//! `<cache_dir>/fastembed/`, downloading it once (about 33 MB) unless
//! offline. Inference is batched (`batch_size`, default 64) and ONNX
//! Runtime uses every core unless `threads` caps it.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

use super::{l2_normalize, query_prefix, EmbedderOptions, OFFLINE_HINT};
use crate::{EmbedKind, Embedder, ModelError, Result};

/// Hugging Face repo of the quantized model (fastembed's `BGESmallENV15Q`).
const BGE_SMALL_Q_REPO: &str = "Qdrant/bge-small-en-v1.5-onnx-Q";

/// The fastembed backend. `Send + Sync`; the session sits behind a mutex
/// because `TextEmbedding::embed` takes `&mut self`.
pub struct FastEmbedder {
    id: String,
    dims: usize,
    model: EmbeddingModel,
    repo: &'static str,
    opts: EmbedderOptions,
    session: OnceLock<std::result::Result<Mutex<TextEmbedding>, String>>,
}

impl std::fmt::Debug for FastEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FastEmbedder")
            .field("id", &self.id)
            .field("dims", &self.dims)
            .field("loaded", &self.session.get().is_some())
            .finish()
    }
}

impl FastEmbedder {
    /// A lazy embedder for `model` (currently `bge-small-en-v1.5`).
    pub fn new(model: &str, opts: &EmbedderOptions) -> Result<Self> {
        let (m, dims, repo) = match model {
            "bge-small-en-v1.5" => (EmbeddingModel::BGESmallENV15Q, 384, BGE_SMALL_Q_REPO),
            other => {
                return Err(ModelError::Other(anyhow::anyhow!(
                    "unknown fastembed model {other:?}"
                )))
            }
        };
        Ok(FastEmbedder {
            id: format!("fastembed:{model}"),
            dims,
            model: m,
            repo,
            opts: opts.clone(),
            session: OnceLock::new(),
        })
    }

    /// `<cache_dir>/fastembed`.
    pub fn model_dir(&self) -> PathBuf {
        self.opts.cache_dir.join("fastembed")
    }

    fn is_cached(dir: &Path, repo: &str) -> bool {
        dir.join(format!("models--{}", repo.replace('/', "--")))
            .join("snapshots")
            .is_dir()
    }

    fn load(&self) -> std::result::Result<Mutex<TextEmbedding>, String> {
        let dir = self.model_dir();
        if self.opts.offline && !Self::is_cached(&dir, self.repo) {
            return Err(format!(
                "{} is not downloaded to {} and downloads are disabled (offline); {OFFLINE_HINT}",
                self.id,
                dir.display()
            ));
        }
        let mut init = TextInitOptions::new(self.model.clone())
            .with_cache_dir(dir.clone())
            .with_show_download_progress(self.opts.show_progress);
        if let Some(t) = self.opts.threads {
            init = init.with_intra_threads(t);
        }
        TextEmbedding::try_new(init).map(Mutex::new).map_err(|e| {
            format!(
                "could not load {} from {}: {e}; {OFFLINE_HINT}",
                self.id,
                dir.display()
            )
        })
    }

    fn session(&self) -> Result<&Mutex<TextEmbedding>> {
        self.session
            .get_or_init(|| self.load())
            .as_ref()
            .map_err(|e| ModelError::Unavailable(e.clone()))
    }
}

impl Embedder for FastEmbedder {
    fn id(&self) -> &str {
        &self.id
    }
    fn dims(&self) -> usize {
        self.dims
    }
    fn is_remote(&self) -> bool {
        false
    }
    fn embed(&self, texts: &[&str], kind: EmbedKind) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let prefix = query_prefix(&self.id, kind);
        let inputs: Vec<String> = texts.iter().map(|t| format!("{prefix}{t}")).collect();
        let session = self.session()?;
        let mut model = session
            .lock()
            .map_err(|_| ModelError::Unavailable("embedder session poisoned".into()))?;
        let mut out = model
            .embed(&inputs, Some(self.opts.batch_size.max(1)))
            .map_err(|e| ModelError::Other(anyhow::anyhow!("fastembed: {e}")))?;
        for v in &mut out {
            if v.len() != self.dims {
                return Err(ModelError::InvalidOutput(format!(
                    "expected {} dims, got {}",
                    self.dims,
                    v.len()
                )));
            }
            l2_normalize(v);
        }
        Ok(out)
    }
}
