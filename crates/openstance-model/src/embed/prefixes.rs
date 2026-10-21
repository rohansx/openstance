//! Query and passage forms per model family (models.md section 3.2).

use crate::EmbedKind;

/// The text prepended to an input of `kind` for the embedder `id`
/// (`backend:model`). Unknown models get no prefix.
pub fn query_prefix(id: &str, kind: EmbedKind) -> &'static str {
    let model = id.split_once(':').map_or(id, |(_, m)| m);
    if model.starts_with("bge-") && model.contains("-en-v1.5") {
        return match kind {
            EmbedKind::Query => "Represent this sentence for searching relevant passages: ",
            EmbedKind::Passage => "",
        };
    }
    if model.starts_with("nomic-embed-text") {
        return match kind {
            EmbedKind::Query => "search_query: ",
            EmbedKind::Passage => "search_document: ",
        };
    }
    ""
}
