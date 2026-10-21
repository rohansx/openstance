//! The [`Graph`] side of embedding: every write the facade does (import,
//! edit, merge, split, review, rebuild) refreshes the vectors of the people
//! it touched once its transaction has committed.

use super::{index, status, EmbedReport, IndexOptions, Progress, Status, CHANGED_WARNING};
use crate::error::Result;
use crate::{Graph, PersonId};

/// Per-graph embedding state: an optional progress callback (the CLI's
/// progress line) and the report of the last automatic refresh.
#[derive(Default)]
pub struct Hook {
    progress: Option<Box<dyn FnMut(Progress) + Send>>,
    last: Option<EmbedReport>,
}

impl std::fmt::Debug for Hook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hook")
            .field("progress", &self.progress.is_some())
            .field("last", &self.last)
            .finish()
    }
}

impl Graph {
    /// Report embedding progress (import, rebuild) to `f`.
    pub fn on_embed_progress(&mut self, f: impl FnMut(Progress) + Send + 'static) {
        self.embed.progress = Some(Box::new(f));
    }

    /// The report of the last automatic embedding refresh, if one ran.
    pub fn last_embed_report(&self) -> Option<&EmbedReport> {
        self.embed.last.as_ref()
    }

    /// Stored vectors vs the configured embedder.
    pub fn embedding_status(&self) -> Result<Status> {
        status(&self.conn, self.providers.models.embedder.as_ref())
    }

    /// The warning to print before any command when the configured embedder
    /// differs from the stored one (P2-MODEL-02).
    pub fn embedding_warning(&self) -> Option<String> {
        match self.embedding_status() {
            Ok(Status::Changed { .. }) => Some(CHANGED_WARNING.to_string()),
            _ => None,
        }
    }

    /// Run an embedding pass (`rebuild --embeddings` passes `force`).
    pub fn embed(&mut self, opts: &IndexOptions) -> Result<EmbedReport> {
        let Graph {
            conn,
            providers,
            embed,
            ..
        } = self;
        let mut noop = |_: Progress| {};
        let progress: &mut dyn FnMut(Progress) = match embed.progress.as_mut() {
            Some(p) => p.as_mut(),
            None => &mut noop,
        };
        index(conn, providers.models.embedder.as_ref(), opts, progress)
    }

    /// The automatic refresh after a write: `people = None` scans everyone
    /// (only changed hashes are embedded). Never fails the write: a model
    /// problem lands in the report and a log line.
    pub(crate) fn refresh_embeddings(&mut self, people: Option<&[PersonId]>) -> Result<()> {
        if self.providers.models.embedder.dims() == 0 {
            return Ok(());
        }
        let opts = IndexOptions {
            people: people.map(<[PersonId]>::to_vec),
            ..IndexOptions::default()
        };
        let report = self.embed(&opts)?;
        if let Some(w) = report.warning() {
            tracing::warn!(warning = %w, "embeddings");
        }
        self.embed.last = Some(report);
        Ok(())
    }
}
