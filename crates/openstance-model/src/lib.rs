//! Model seams for openstance: the [`Query`] type every search surface shares,
//! the [`Embedder`], [`QueryParser`] and [`Reranker`] traits, and the `Null`
//! implementations that let the binary work with nothing installed.
//!
//! Authoritative specs: `docs/tech-spec/search.md` section 2 (Query) and
//! `docs/tech-spec/models.md` section 2 (traits). Concrete backends
//! (fastembed, ollama, openai, anthropic) arrive behind cargo features in
//! later phases; core depends on this crate with `default-features = false`.
#![warn(missing_docs)]

pub mod embed;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Default `Query.limit`.
pub const DEFAULT_LIMIT: usize = 30;
/// Hard cap on `Query.limit`.
pub const MAX_LIMIT: usize = 200;

/// Result type of model calls.
pub type Result<T> = std::result::Result<T, ModelError>;

/// Errors from model backends.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// Not downloaded, server down, or the Null implementation.
    #[error("model not available: {0}")]
    Unavailable(String),
    /// The call took too long.
    #[error("timeout after {0:?}")]
    Timeout(std::time::Duration),
    /// JSON / schema / grounding failure.
    #[error("invalid model output: {0}")]
    InvalidOutput(String),
    /// HTTP failure from a remote backend.
    #[error("http {status}: {body}")]
    Http {
        /// Status code.
        status: u16,
        /// Response body (never contains user data we sent).
        body: String,
    },
    /// The config does not allow egress for this backend.
    #[error("egress not allowed for {0}")]
    EgressDenied(String),
    /// Anything else.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

/// A parsed search: free text plus structured filters. Values inside one
/// field are OR-ed; fields are AND-ed. Serialized as-is into `view.query_json`
/// and echoed in every `SearchResponse` so it can be edited and rerun.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Query {
    /// Free text for FTS5 and the embedder. `None` = pure structured query.
    #[serde(default)]
    pub text: Option<String>,
    /// Hard filter on company (current company or work email domain).
    #[serde(default)]
    pub company: Vec<String>,
    /// Hard filter on title.
    #[serde(default)]
    pub title: Vec<String>,
    /// Hard filter on location.
    #[serde(default)]
    pub location: Vec<String>,
    /// Soft: boosts text relevance and matches tags; never excludes.
    #[serde(default)]
    pub industry: Vec<String>,
    /// Exact tag match against `override(field='tag')`.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Interaction window start (unix seconds): at least one contact
    /// interaction in `[since, until)`.
    #[serde(default)]
    pub since: Option<i64>,
    /// Interaction window end (exclusive, unix seconds).
    #[serde(default)]
    pub until: Option<i64>,
    /// Last contact strictly before this instant.
    #[serde(default)]
    pub last_contact_before: Option<i64>,
    /// `true`: has a meeting interaction (inside the window if set);
    /// `false`: has none.
    #[serde(default)]
    pub met_in_person: Option<bool>,
    /// `true`: reciprocity > 0; `false`: reciprocity = 0.
    #[serde(default)]
    pub replied: Option<bool>,
    /// Minimum relationship score.
    #[serde(default)]
    pub min_score: Option<f32>,
    /// High frequency, low recency: `score_f >= 0.5 AND score_r < 0.2`.
    #[serde(default)]
    pub lost_touch: Option<bool>,
    /// Include `is_bulk = 1` people. Default false.
    #[serde(default)]
    pub include_bulk: bool,
    /// Default 30, max 200.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    DEFAULT_LIMIT
}

impl Default for Query {
    fn default() -> Self {
        Query {
            text: None,
            company: Vec::new(),
            title: Vec::new(),
            location: Vec::new(),
            industry: Vec::new(),
            tags: Vec::new(),
            since: None,
            until: None,
            last_contact_before: None,
            met_in_person: None,
            replied: None,
            min_score: None,
            lost_touch: None,
            include_bulk: false,
            limit: DEFAULT_LIMIT,
        }
    }
}

impl Query {
    /// A text-only query.
    pub fn text(text: impl Into<String>) -> Self {
        Query {
            text: Some(text.into()),
            ..Query::default()
        }
    }

    /// `limit` clamped to `1..=MAX_LIMIT`.
    pub fn effective_limit(&self) -> usize {
        self.limit.clamp(1, MAX_LIMIT)
    }

    /// True when no structured filter is set (text, `include_bulk` and
    /// `limit` do not count).
    pub fn has_structured_filters(&self) -> bool {
        !(self.company.is_empty()
            && self.title.is_empty()
            && self.location.is_empty()
            && self.tags.is_empty()
            && self.since.is_none()
            && self.until.is_none()
            && self.last_contact_before.is_none()
            && self.met_in_person.is_none()
            && self.replied.is_none()
            && self.min_score.is_none()
            && self.lost_touch.is_none())
    }
}

/// Explicit filters (CLI flags, MCP / HTTP fields) laid over a parsed
/// [`Query`] field by field. Every field is optional; a set field always wins.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryPatch {
    /// Overrides `text` (use `Some(None)` to clear it).
    #[serde(default)]
    pub text: Option<Option<String>>,
    /// Overrides `company`.
    #[serde(default)]
    pub company: Option<Vec<String>>,
    /// Overrides `title`.
    #[serde(default)]
    pub title: Option<Vec<String>>,
    /// Overrides `location`.
    #[serde(default)]
    pub location: Option<Vec<String>>,
    /// Overrides `industry`.
    #[serde(default)]
    pub industry: Option<Vec<String>>,
    /// Overrides `tags`.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Overrides `since`.
    #[serde(default)]
    pub since: Option<i64>,
    /// Overrides `until`.
    #[serde(default)]
    pub until: Option<i64>,
    /// Overrides `last_contact_before`.
    #[serde(default)]
    pub last_contact_before: Option<i64>,
    /// Overrides `met_in_person`.
    #[serde(default)]
    pub met_in_person: Option<bool>,
    /// Overrides `replied`.
    #[serde(default)]
    pub replied: Option<bool>,
    /// Overrides `min_score`.
    #[serde(default)]
    pub min_score: Option<f32>,
    /// Overrides `lost_touch`.
    #[serde(default)]
    pub lost_touch: Option<bool>,
    /// Overrides `include_bulk`.
    #[serde(default)]
    pub include_bulk: Option<bool>,
    /// Overrides `limit`.
    #[serde(default)]
    pub limit: Option<usize>,
}

impl QueryPatch {
    /// Lay this patch over `q`. Returns the names of the fields it set, in
    /// declaration order (reported as `query_source.explicit`).
    pub fn apply(&self, q: &mut Query) -> Vec<&'static str> {
        let mut set = Vec::new();
        macro_rules! over {
            ($field:ident) => {
                if let Some(v) = &self.$field {
                    q.$field = v.clone();
                    set.push(stringify!($field));
                }
            };
            ($field:ident, wrap) => {
                if let Some(v) = &self.$field {
                    q.$field = Some(v.clone());
                    set.push(stringify!($field));
                }
            };
        }
        over!(text);
        over!(company);
        over!(title);
        over!(location);
        over!(industry);
        over!(tags);
        over!(since, wrap);
        over!(until, wrap);
        over!(last_contact_before, wrap);
        over!(met_in_person, wrap);
        over!(replied, wrap);
        over!(min_score, wrap);
        over!(lost_touch, wrap);
        over!(include_bulk);
        over!(limit);
        set
    }

    /// True when nothing is set.
    pub fn is_empty(&self) -> bool {
        *self == QueryPatch::default()
    }
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Whether a text is embedded as a search query or as a stored document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedKind {
    /// The user's query.
    Query,
    /// A person's embedding text.
    Passage,
}

/// Turns text into vectors.
pub trait Embedder: Send + Sync {
    /// Stable id stored in the DB, e.g. `"fastembed:bge-small-en-v1.5"`.
    fn id(&self) -> &str;
    /// Vector length; 0 disables the vector half of search.
    fn dims(&self) -> usize;
    /// True when calls leave the machine.
    fn is_remote(&self) -> bool;
    /// Embed a batch.
    fn embed(&self, texts: &[&str], kind: EmbedKind) -> Result<Vec<Vec<f32>>>;
}

/// Turns free text into a [`Query`].
pub trait QueryParser: Send + Sync {
    /// `"rules"`, `"passthrough"`, `"ollama:qwen2.5:7b"`, ...
    fn id(&self) -> &str;
    /// True when calls leave the machine.
    fn is_remote(&self) -> bool;
    /// `today` is unix seconds at local midnight; the timezone is fixed at
    /// construction.
    fn parse(&self, text: &str, today: i64) -> Result<Query>;
}

/// What a reranker may see about one candidate.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CandidateProfile {
    /// Title.
    pub title: Option<String>,
    /// Company.
    pub company: Option<String>,
    /// Location.
    pub location: Option<String>,
    /// User tags.
    pub tags: Vec<String>,
    /// Local backends only.
    pub display_name: Option<String>,
    /// Local backends only, top 5.
    pub subjects: Vec<String>,
}

/// One fused search candidate handed to a [`Reranker`].
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// `person.id`; never sent to remote backends.
    pub id: i64,
    /// Profile fields.
    pub profile: CandidateProfile,
    /// Fused relevance.
    pub fused: f32,
    /// Relationship score.
    pub score: f32,
    /// `no_enrich` people are never sent to a remote model.
    pub no_enrich: bool,
}

/// A reranked candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    /// `person.id`.
    pub id: i64,
    /// Reranker score (higher is better).
    pub rank_score: f32,
    /// Optional one-line reason (API models only).
    pub reason: Option<String>,
}

/// Reorders the top fused candidates.
pub trait Reranker: Send + Sync {
    /// `"none"`, `"fastembed:bge-reranker-base"`, ...
    fn id(&self) -> &str;
    /// True when calls leave the machine.
    fn is_remote(&self) -> bool;
    /// Return a new order for `cands`.
    fn rerank(&self, q: &Query, cands: Vec<Candidate>) -> Result<Vec<Ranked>>;
}

/// The three model seams, as built by the binary from config.
pub struct Models {
    /// Embedder.
    pub embedder: Box<dyn Embedder>,
    /// Query parser.
    pub parser: Box<dyn QueryParser>,
    /// Reranker.
    pub reranker: Box<dyn Reranker>,
}

impl Models {
    /// No embedder, passthrough parser, no reranker. Core swaps in its
    /// `RuleParser` when `model.parser = "rules"`.
    pub fn null() -> Self {
        Models {
            embedder: Box::new(NullEmbedder),
            parser: Box::new(PassthroughParser),
            reranker: Box::new(NullReranker),
        }
    }
}

impl std::fmt::Debug for Models {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Models")
            .field("embedder", &self.embedder.id())
            .field("parser", &self.parser.id())
            .field("reranker", &self.reranker.id())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Null implementations
// ---------------------------------------------------------------------------

/// No embedder: `dims() == 0` tells core to skip the vector scan.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullEmbedder;

impl Embedder for NullEmbedder {
    fn id(&self) -> &str {
        "none"
    }
    fn dims(&self) -> usize {
        0
    }
    fn is_remote(&self) -> bool {
        false
    }
    fn embed(&self, _texts: &[&str], _kind: EmbedKind) -> Result<Vec<Vec<f32>>> {
        Err(ModelError::Unavailable("no embedder configured".into()))
    }
}

/// The whole input becomes `Query.text` (used by `--no-parse`).
#[derive(Debug, Clone, Copy, Default)]
pub struct PassthroughParser;

impl QueryParser for PassthroughParser {
    fn id(&self) -> &str {
        "passthrough"
    }
    fn is_remote(&self) -> bool {
        false
    }
    fn parse(&self, text: &str, _today: i64) -> Result<Query> {
        let t = text.trim();
        Ok(Query {
            text: (!t.is_empty()).then(|| t.to_string()),
            ..Query::default()
        })
    }
}

/// Keeps the fused order, writes no reasons.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullReranker;

impl Reranker for NullReranker {
    fn id(&self) -> &str {
        "none"
    }
    fn is_remote(&self) -> bool {
        false
    }
    fn rerank(&self, _q: &Query, cands: Vec<Candidate>) -> Result<Vec<Ranked>> {
        Ok(cands
            .into_iter()
            .map(|c| Ranked {
                id: c.id,
                rank_score: c.fused,
                reason: None,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_query() -> Query {
        Query {
            text: Some("devtools dinner".into()),
            company: vec!["stripe".into(), "jane street".into()],
            title: vec!["founder".into()],
            location: vec!["san francisco".into()],
            industry: vec!["fintech".into()],
            tags: vec!["angels".into()],
            since: Some(1_735_689_600),
            until: Some(1_767_225_600),
            last_contact_before: Some(1_704_067_200),
            met_in_person: Some(true),
            replied: Some(false),
            min_score: Some(0.4),
            lost_touch: Some(true),
            include_bulk: true,
            limit: 10,
        }
    }

    #[test]
    fn query_round_trips_through_json_without_loss() {
        let q = full_query();
        let s = serde_json::to_string(&q).unwrap();
        let back: Query = serde_json::from_str(&s).unwrap();
        assert_eq!(back, q);
        let d = Query::default();
        let back: Query = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn query_json_field_names_are_the_documented_ones() {
        let v = serde_json::to_value(Query::default()).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "text",
                "company",
                "title",
                "location",
                "industry",
                "tags",
                "since",
                "until",
                "last_contact_before",
                "met_in_person",
                "replied",
                "min_score",
                "lost_touch",
                "include_bulk",
                "limit"
            ]
        );
        assert_eq!(v["limit"], 30);
        assert_eq!(v["include_bulk"], false);
        assert!(v["company"].as_array().unwrap().is_empty());
        assert!(v["text"].is_null());
    }

    #[test]
    fn query_missing_fields_take_defaults_and_unknown_fields_are_rejected() {
        let q: Query = serde_json::from_str(r#"{"company":["acme"]}"#).unwrap();
        assert_eq!(q.limit, DEFAULT_LIMIT);
        assert_eq!(q.company, vec!["acme".to_string()]);
        assert!(serde_json::from_str::<Query>(r#"{"compnay":["acme"]}"#).is_err());
    }

    #[test]
    fn effective_limit_is_clamped() {
        let q = Query {
            limit: 0,
            ..Query::default()
        };
        assert_eq!(q.effective_limit(), 1);
        let q = Query {
            limit: 5000,
            ..Query::default()
        };
        assert_eq!(q.effective_limit(), MAX_LIMIT);
    }

    #[test]
    fn structured_filter_detection() {
        assert!(!Query::text("x").has_structured_filters());
        let q = Query {
            replied: Some(true),
            ..Query::default()
        };
        assert!(q.has_structured_filters());
    }

    #[test]
    fn patch_overrides_only_set_fields_and_reports_them() {
        let mut q = full_query();
        let patch = QueryPatch {
            company: Some(vec!["acme".into()]),
            met_in_person: Some(false),
            text: Some(None),
            ..QueryPatch::default()
        };
        let set = patch.apply(&mut q);
        assert_eq!(set, ["text", "company", "met_in_person"]);
        assert_eq!(q.company, vec!["acme".to_string()]);
        assert_eq!(q.met_in_person, Some(false));
        assert_eq!(q.text, None);
        assert_eq!(q.title, vec!["founder".to_string()]);
        assert!(QueryPatch::default().is_empty());
        assert!(!patch.is_empty());
    }

    #[test]
    fn null_embedder_disables_vectors() {
        let e = NullEmbedder;
        assert_eq!(e.dims(), 0);
        assert_eq!(e.id(), "none");
        assert!(matches!(
            e.embed(&["x"], EmbedKind::Query),
            Err(ModelError::Unavailable(_))
        ));
    }

    #[test]
    fn null_reranker_keeps_fused_order() {
        let cands: Vec<Candidate> = [(3, 0.9), (1, 0.5), (2, 0.1)]
            .into_iter()
            .map(|(id, fused)| Candidate {
                id,
                profile: CandidateProfile::default(),
                fused,
                score: 0.0,
                no_enrich: false,
            })
            .collect();
        let out = NullReranker.rerank(&Query::default(), cands).unwrap();
        assert_eq!(out.iter().map(|r| r.id).collect::<Vec<_>>(), [3, 1, 2]);
        assert!(out.iter().all(|r| r.reason.is_none()));
    }

    #[test]
    fn passthrough_parser_puts_everything_in_text() {
        let q = PassthroughParser.parse("  fintech founders  ", 0).unwrap();
        assert_eq!(q.text.as_deref(), Some("fintech founders"));
        assert!(!q.has_structured_filters());
        assert_eq!(PassthroughParser.parse("   ", 0).unwrap().text, None);
    }

    #[test]
    fn query_has_a_json_schema() {
        let schema = schemars::schema_for!(Query);
        let v = serde_json::to_value(&schema).unwrap();
        assert!(v["properties"]["company"].is_object());
    }
}
