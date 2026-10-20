//! The single JSON shape shared by the CLI (`--json`), MCP
//! `structuredContent` and the HTTP API. Authoritative definitions:
//! `docs/tech-spec/interfaces.md` section 1 (and `search.md` section 9 for
//! search-only fields).
//!
//! Conventions enforced here:
//! - every top-level response carries `"v": 1` ([`V`]);
//! - timestamps are ISO 8601 UTC strings ([`iso`]); the database stores unix
//!   seconds and conversion happens only at the surface;
//! - scores and components are rounded to 3 decimals ([`round3`]);
//! - `null` means unknown; arrays are always present, never omitted.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use openstance_model::Query;

/// JSON schema version carried by every top-level response.
pub const V: u32 = 1;

/// Unix seconds -> `"2026-09-14T16:02:00Z"`.
pub fn iso(ts: i64) -> String {
    let dt =
        time::OffsetDateTime::from_unix_timestamp(ts).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let f = time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    dt.format(&f).unwrap_or_default()
}

/// Optional variant of [`iso`].
pub fn iso_opt(ts: Option<i64>) -> Option<String> {
    ts.map(iso)
}

/// Unix seconds -> `"2026-09-14"` (UTC date, used in evidence lines).
pub fn ymd(ts: i64) -> String {
    let dt =
        time::OffsetDateTime::from_unix_timestamp(ts).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let f = time::macros::format_description!("[year]-[month]-[day]");
    dt.format(&f).unwrap_or_default()
}

/// Round a score or component to 3 decimals for output.
pub fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

// ---------------------------------------------------------------------------
// People
// ---------------------------------------------------------------------------

/// Compact person reference used inside other shapes (1.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonRef {
    /// Anchor person id.
    pub id: i64,
    /// Display name.
    pub display_name: Option<String>,
    /// Title.
    pub title: Option<String>,
    /// Company.
    pub company: Option<String>,
    /// Relationship score.
    pub score: f64,
}

/// The five weights, keyed by component name.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WeightsJson {
    /// R weight.
    pub recency: f64,
    /// F weight.
    pub frequency: f64,
    /// C weight.
    pub reciprocity: f64,
    /// M weight.
    pub meetings: f64,
    /// B weight.
    pub breadth: f64,
}

/// Raw counts behind the components (computed on demand for `who`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreRaw {
    /// Days since last contact (fractional days, clamped at 0).
    pub days_since_last: Option<f64>,
    /// Contact-group interactions.
    pub n_interactions: u32,
    /// Outbound email/dm messages to them.
    pub messages_from_you: u32,
    /// Their replies.
    pub replies_from_them: u32,
    /// Threads they started.
    pub threads_they_started: u32,
    /// Past meetings.
    pub n_meetings: u32,
    /// Channels seen, sorted.
    pub channels: Vec<String>,
}

/// Score components plus the config that produced them (1.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreComponents {
    /// R.
    pub recency: f64,
    /// F.
    pub frequency: f64,
    /// C.
    pub reciprocity: f64,
    /// M.
    pub meetings: f64,
    /// B.
    pub breadth: f64,
    /// Effective weights.
    pub weights: WeightsJson,
    /// Preset name, or `"custom"` for explicit weights.
    pub preset: String,
    /// Recency half-life.
    pub half_life_days: f64,
    /// Raw inputs; `null` in search results, filled by `who` / `get_person`.
    pub raw: Option<ScoreRaw>,
}

/// A person (1.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    /// Anchor person id.
    pub id: i64,
    /// Display name.
    pub display_name: Option<String>,
    /// Title.
    pub title: Option<String>,
    /// Company.
    pub company: Option<String>,
    /// Location.
    pub location: Option<String>,
    /// Bio.
    pub bio: Option<String>,
    /// User tags.
    pub tags: Vec<String>,
    /// ISO time of the earliest non-contact interaction.
    pub first_seen: Option<String>,
    /// ISO time of the last contact.
    pub last_contact: Option<String>,
    /// Relationship score.
    pub score: f64,
    /// Components.
    pub components: ScoreComponents,
    /// Bulk sender.
    pub is_bulk: bool,
    /// Never shared in views.
    pub do_not_share: bool,
    /// Never sent to enrichment.
    pub no_enrich: bool,
    /// Where each displayed scalar came from: `"override"`,
    /// `"<source.kind>:<source.label>"`, or `null`.
    pub field_sources: BTreeMap<String, Option<String>>,
}

/// A source reference inside identities and evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRefJson {
    /// `source.id`.
    pub id: i64,
    /// `source.kind`.
    pub kind: String,
    /// `source.label`.
    pub label: Option<String>,
}

/// Email verification state (Phase 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    /// `deliverable | undeliverable | risky | unknown`.
    pub status: String,
    /// ISO time.
    pub at: String,
    /// e.g. `"enrich:treg"`.
    pub via: String,
}

/// One handle of a person (1.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    /// `identity.id`.
    pub id: i64,
    /// `identity.kind`.
    pub kind: String,
    /// Normalized value.
    pub value: String,
    /// Confidence of the rule that attached it.
    pub confidence: Option<f64>,
    /// Rule that attached it (`anchor`, `exact`, `gmail_alias`, ...).
    pub match_rule: Option<String>,
    /// First source that produced it.
    pub source: SourceRefJson,
    /// Present only for verified emails.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub verification: Option<Verification>,
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

/// The other side of a tie in tie evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TieWith {
    /// Person id.
    pub id: i64,
    /// Display name.
    pub display_name: Option<String>,
}

/// Evidence for a claim (1.5). Tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Evidence {
    /// An interaction (subject only, never a body).
    Interaction {
        /// `interaction.id`.
        interaction_id: i64,
        /// `email | meeting | dm | mention | follow | connection`.
        #[serde(rename = "type")]
        kind: String,
        /// `in | out | mutual | none`.
        direction: Option<String>,
        /// ISO time.
        at: String,
        /// Subject or title.
        subject: Option<String>,
        /// The person's role in it.
        role: Option<String>,
        /// Where it came from.
        source: SourceRefJson,
        /// Rendered evidence line (`{YYYY-MM-DD}  {kind} {direction}  "{subject}"  ({source.label})`).
        line: String,
    },
    /// A matched field.
    Field {
        /// Field name.
        field: String,
        /// Field value.
        value: String,
        /// The query term it matched.
        matched: String,
    },
    /// A tie to another person.
    Tie {
        /// The other person.
        with: TieWith,
        /// `email | meeting | dm`.
        tie_kind: String,
        /// Shared interactions.
        n_shared: u32,
        /// Tie weight.
        weight: f64,
        /// ISO time of the latest shared interaction.
        last_at: String,
    },
    /// An enrichment fact (Phase 3).
    Enrich {
        /// Op.
        op: String,
        /// Field.
        field: String,
        /// Value.
        value: String,
        /// Backend.
        backend: String,
        /// ISO time.
        at: String,
        /// `enrich_log.id`.
        enrich_log_id: i64,
    },
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Relevance breakdown of a text-mode result. `null` in structured mode.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Relevance {
    /// Min-max normalized bm25 (0 when not an FTS hit).
    pub fts: f64,
    /// Min-max normalized cosine (`null` when no vector half).
    pub vector: Option<f64>,
    /// Fused relevance.
    pub fused: f64,
    /// `fused * (0.5 + 0.5 * score)`.
    #[serde(rename = "final")]
    pub final_: f64,
}

/// One matched field of a result (search.md section 8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Matched {
    /// Field name (`company`, `title`, `subjects`, `semantic`, `met_in_person`, ...).
    pub field: String,
    /// The person's value (string, bool or number).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<serde_json::Value>,
    /// The query term that matched.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub matched: Option<String>,
    /// How it matched (`person.company`, `email domain stripe.com`, `4 meetings`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub via: Option<String>,
    /// FTS5 snippet.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub snippet: Option<String>,
    /// Cosine similarity for semantic-only hits.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub similarity: Option<f64>,
}

/// One search result (1.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    /// 1-based rank.
    pub rank: usize,
    /// The person.
    pub person: Person,
    /// `null` in structured mode.
    pub relevance: Option<Relevance>,
    /// Matched fields.
    pub matched: Vec<Matched>,
    /// Up to `evidence_per_result` most recent interactions.
    pub evidence: Vec<Evidence>,
    /// Reranker prose (only with a reason-writing reranker).
    pub reason: Option<String>,
    /// `"model:<id>"` when `reason` is set.
    pub reason_source: Option<String>,
}

/// Where the parsed query came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySource {
    /// Parser id (`rules`, `passthrough`, `ollama:...`).
    pub parser: String,
    /// Fallback reason when a model parser failed.
    pub fallback: Option<String>,
    /// Fields set explicitly by flags / JSON filters.
    pub explicit: Vec<String>,
}

/// Model ids used for a search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelsUsed {
    /// Parser id.
    pub parser: String,
    /// Embedder id (`none` when disabled).
    pub embedder: String,
    /// Reranker id.
    pub reranker: String,
}

/// Per-step timings (only with `--explain`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Timings {
    /// Step 1.
    pub parse: u64,
    /// Step 2.
    pub filter: u64,
    /// Step 3.
    pub candidates: u64,
    /// Step 4.
    pub fuse: u64,
    /// Step 5.
    pub rerank: u64,
    /// Step 6.
    pub evidence: u64,
    /// Total.
    pub total: u64,
}

/// `structured` (no text) or `text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    /// Ordered by score.
    Structured,
    /// Ordered by fused relevance.
    Text,
}

/// The `search_people` payload (1.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    /// Schema version.
    pub v: u32,
    /// The parsed query (editable, rerunnable).
    pub query: Query,
    /// Parser provenance.
    pub query_source: QuerySource,
    /// CLI command reproducing `query` with no model call.
    pub rerun: String,
    /// Mode.
    pub mode: SearchMode,
    /// Models used.
    pub models: ModelsUsed,
    /// Results before `limit`.
    pub total: usize,
    /// Wall time.
    pub took_ms: u64,
    /// Results.
    pub results: Vec<SearchResult>,
    /// Degradation warnings.
    pub warnings: Vec<String>,
    /// Zero-result widening hints (`drop met_in_person -> 12 results`).
    pub suggestions: Vec<String>,
    /// Only with `--explain`.
    pub timings_ms: Option<Timings>,
}

// ---------------------------------------------------------------------------
// Person detail, ties, review
// ---------------------------------------------------------------------------

/// A mutual connection in `PersonDetail`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mutual {
    /// The other person.
    pub person: PersonRef,
    /// Tie kind.
    pub tie_kind: String,
    /// Shared interactions.
    pub n_shared: u32,
    /// Tie weight.
    pub weight: f64,
}

/// One override row as shown to the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverrideJson {
    /// Field.
    pub field: String,
    /// Value (`null` = tombstone).
    pub value: Option<String>,
    /// ISO time.
    pub set_at: String,
}

/// A note override.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteJson {
    /// Text.
    pub value: String,
    /// ISO time.
    pub set_at: String,
}

/// A suggested field value from enrichment (Phase 3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestedValue {
    /// Value.
    pub value: String,
    /// e.g. `enrich:treg`.
    pub from: String,
    /// ISO time.
    pub at: String,
}

/// Merge provenance of a person.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergeInfo {
    /// Former anchor ids now inside this person.
    pub merged_from: Vec<i64>,
    /// Contains flagged identities awaiting review.
    pub flagged: bool,
    /// Lowest identity confidence.
    pub confidence: f64,
}

/// `get_person` / `who` / `GET /people/{id}` (1.8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonDetail {
    /// Schema version.
    pub v: u32,
    /// Set when the requested id was merged away into this person.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resolved_from: Option<i64>,
    /// The person (with `components.raw` filled).
    pub person: Person,
    /// Every handle.
    pub identities: Vec<Identity>,
    /// Most recent interactions (`Evidence::Interaction`).
    pub recent_interactions: Vec<Evidence>,
    /// Strongest ties.
    pub mutuals: Vec<Mutual>,
    /// Scalar overrides in effect.
    pub overrides: Vec<OverrideJson>,
    /// Notes, newest first.
    pub notes: Vec<NoteJson>,
    /// Enrichment suggestions (Phase 3).
    pub suggested: BTreeMap<String, SuggestedValue>,
    /// Merge provenance.
    pub merge: MergeInfo,
}

/// Direct relationship block of `HowConnected`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Direct {
    /// Score of the target.
    pub score: f64,
    /// ISO time.
    pub last_contact: Option<String>,
}

/// One intro path (1.7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathJson {
    /// The mutual.
    pub via: PersonRef,
    /// `my_score * link_weight`.
    pub strength: f64,
    /// `tie | same_company | past_company | school`.
    pub reason: String,
    /// Score of the mutual.
    pub my_score: f64,
    /// Tie weight or affinity.
    pub link_weight: f64,
    /// Evidence.
    pub evidence: Vec<Evidence>,
}

/// `how_connected` (1.7, Phase 2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HowConnected {
    /// Schema version.
    pub v: u32,
    /// Target.
    pub target: PersonRef,
    /// `null` when there is no interaction with the target.
    pub direct: Option<Direct>,
    /// Paths, strongest first.
    pub paths: Vec<PathJson>,
}

/// A flagged merge in the review queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlaggedMerge {
    /// The merged person.
    pub person: PersonRef,
    /// `name_company` or `name_thread`.
    pub rule: String,
    /// Rule confidence.
    pub confidence: f64,
    /// The identities attached by the flagged rule.
    pub flagged_identities: Vec<Identity>,
    /// The rest of the person.
    pub other_identities: Vec<Identity>,
    /// Human explanation (`same name "jane doe"; company "stripe"`).
    pub why: String,
}

/// A merge suggestion (never auto-merged).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    /// One side.
    pub a: PersonRef,
    /// Other side.
    pub b: PersonRef,
    /// `rare_name | jaro_winkler | large_company | enrich_collision`.
    pub rule: String,
    /// Suggestion confidence.
    pub confidence: f64,
    /// Human explanation.
    pub why: String,
}

/// An informational review row (shared handles, conflicting overrides).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewInfo {
    /// `shared_handle | conflicting_override`.
    pub kind: String,
    /// Message (handles shown are the user's own data, printed locally only).
    pub message: String,
}

/// `openstance review --json` / `GET /review`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewQueue {
    /// Schema version.
    pub v: u32,
    /// Section 1.
    pub flagged: Vec<FlaggedMerge>,
    /// Section 2.
    pub suggestions: Vec<Suggestion>,
    /// Section 3.
    pub info: Vec<ReviewInfo>,
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// One imported file in `ImportResponse`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceImport {
    /// `source.id` (the existing one when already imported).
    pub id: i64,
    /// Importer kind.
    pub kind: String,
    /// Label.
    pub label: Option<String>,
    /// Path as given.
    pub path: String,
    /// Identity observations.
    pub identities: u64,
    /// New interactions.
    pub interactions: u64,
    /// True when the file hash was already imported (nothing done).
    pub skipped_duplicate_file: bool,
    /// Full importer stats.
    pub stats: openstance_import::ImportStats,
}

/// Resolution summary after an import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResolutionSummary {
    /// Persons now in the graph (excluding self).
    pub people: u64,
    /// New person rows.
    pub people_created: u64,
    /// Identities attached by rules 1-2 and overrides.
    pub auto_merged: u64,
    /// Persons with flagged identities.
    pub flagged: u64,
    /// Bulk senders.
    pub bulk: u64,
}

/// `openstance import --json` / `POST /import`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportResponse {
    /// Schema version.
    pub v: u32,
    /// One entry per file, in order.
    pub sources: Vec<SourceImport>,
    /// After the last file.
    pub resolution: ResolutionSummary,
    /// Wall time.
    pub took_ms: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Stable error codes (1.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Invalid input or flags.
    BadRequest,
    /// Missing or wrong bearer token.
    Unauthorized,
    /// No such person, view or playbook.
    NotFound,
    /// The person reference matched several people.
    Ambiguous,
    /// The view is past `expires_at`.
    ViewExpired,
    /// The quote exceeds the budget.
    BudgetExceeded,
    /// Needs an enricher and `enrich.backend = "none"`.
    EgressDisabled,
    /// Locked by another writer.
    DbBusy,
    /// Written by a newer openstance.
    SchemaTooNew,
    /// Bug.
    Internal,
}

impl ErrorCode {
    /// CLI exit code (interfaces.md section 14).
    pub fn exit_code(self) -> u8 {
        match self {
            ErrorCode::BadRequest => 2,
            ErrorCode::Unauthorized => 1,
            ErrorCode::NotFound => 5,
            ErrorCode::Ambiguous => 6,
            ErrorCode::ViewExpired => 7,
            ErrorCode::BudgetExceeded => 4,
            ErrorCode::EgressDisabled => 3,
            ErrorCode::DbBusy | ErrorCode::SchemaTooNew => 10,
            ErrorCode::Internal => 1,
        }
    }

    /// HTTP status.
    pub fn http_status(self) -> u16 {
        match self {
            ErrorCode::BadRequest => 400,
            ErrorCode::Unauthorized => 401,
            ErrorCode::NotFound => 404,
            ErrorCode::Ambiguous => 409,
            ErrorCode::ViewExpired => 410,
            ErrorCode::BudgetExceeded => 402,
            ErrorCode::EgressDisabled => 409,
            ErrorCode::DbBusy | ErrorCode::SchemaTooNew => 503,
            ErrorCode::Internal => 500,
        }
    }
}

/// A candidate listed in an `ambiguous` error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// Person id.
    pub id: i64,
    /// Display name.
    pub display_name: String,
}

/// The `error` object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// Code.
    pub code: ErrorCode,
    /// Message, safe to show.
    pub message: String,
    /// For `ambiguous` only.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub candidates: Vec<Candidate>,
}

/// `{"v": 1, "error": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Schema version.
    pub v: u32,
    /// The error.
    pub error: ErrorBody,
}

impl From<&crate::Error> for ErrorResponse {
    fn from(e: &crate::Error) -> Self {
        let candidates = match e {
            crate::Error::Ambiguous { candidates, .. } => candidates
                .iter()
                .map(|(id, name)| Candidate {
                    id: *id,
                    display_name: name.clone(),
                })
                .collect(),
            _ => Vec::new(),
        };
        ErrorResponse {
            v: V,
            error: ErrorBody {
                code: e.code(),
                message: e.to_string(),
                candidates,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn components() -> ScoreComponents {
        ScoreComponents {
            recency: 0.87,
            frequency: 0.683,
            reciprocity: 0.765,
            meetings: 0.736,
            breadth: 0.75,
            weights: WeightsJson {
                recency: 0.35,
                frequency: 0.2,
                reciprocity: 0.25,
                meetings: 0.15,
                breadth: 0.05,
            },
            preset: "default".into(),
            half_life_days: 180.0,
            raw: None,
        }
    }

    fn person() -> Person {
        Person {
            id: 812,
            display_name: Some("Tom Example".into()),
            title: Some("Engineering Manager".into()),
            company: Some("Northwind".into()),
            location: None,
            bio: None,
            tags: vec![],
            first_seen: Some(iso(1_706_951_520)),
            last_contact: None,
            score: 0.78,
            components: components(),
            is_bulk: false,
            do_not_share: false,
            no_enrich: false,
            field_sources: BTreeMap::from([("title".to_string(), Some("override".to_string()))]),
        }
    }

    #[test]
    fn iso_and_ymd_format_utc() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_789_401_720), "2026-09-14T16:02:00Z");
        assert_eq!(ymd(1_789_401_720), "2026-09-14");
        assert_eq!(iso_opt(None), None);
    }

    #[test]
    fn round3_rounds_half_away() {
        assert_eq!(round3(0.88531), 0.885);
        assert_eq!(round3(0.8855), 0.886);
        assert_eq!(round3(1.0), 1.0);
    }

    #[test]
    fn person_json_has_documented_fields_and_empty_arrays() {
        let v = serde_json::to_value(person()).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "id",
                "display_name",
                "title",
                "company",
                "location",
                "bio",
                "tags",
                "first_seen",
                "last_contact",
                "score",
                "components",
                "is_bulk",
                "do_not_share",
                "no_enrich",
                "field_sources"
            ]
        );
        assert_eq!(v["tags"], json!([]));
        assert_eq!(v["first_seen"], "2024-02-03T09:12:00Z");
        assert!(v["components"]["raw"].is_null());
        assert_eq!(v["components"]["weights"]["recency"], 0.35);
    }

    #[test]
    fn evidence_is_tagged_by_kind_with_type_field() {
        let e = Evidence::Interaction {
            interaction_id: 99120,
            kind: "meeting".into(),
            direction: Some("mutual".into()),
            at: iso(1_789_401_720),
            subject: Some("devtools dinner".into()),
            role: Some("attendee".into()),
            source: SourceRefJson {
                id: 4,
                kind: "ics".into(),
                label: Some("work calendar".into()),
            },
            line: "2026-09-14  meeting mutual  \"devtools dinner\"  (work calendar)".into(),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "interaction");
        assert_eq!(v["type"], "meeting");
        assert_eq!(v["source"]["label"], "work calendar");
        let back: Evidence = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
        let t = serde_json::to_value(Evidence::Tie {
            with: TieWith {
                id: 77,
                display_name: None,
            },
            tie_kind: "email".into(),
            n_shared: 9,
            weight: 0.52,
            last_at: iso(0),
        })
        .unwrap();
        assert_eq!(t["kind"], "tie");
    }

    #[test]
    fn search_response_shape() {
        let r = SearchResponse {
            v: V,
            query: Query::text("devtools dinner"),
            query_source: QuerySource {
                parser: "rules".into(),
                fallback: None,
                explicit: vec![],
            },
            rerun: "openstance search \"devtools dinner\" --no-parse".into(),
            mode: SearchMode::Text,
            models: ModelsUsed {
                parser: "rules".into(),
                embedder: "none".into(),
                reranker: "none".into(),
            },
            total: 1,
            took_ms: 3,
            results: vec![SearchResult {
                rank: 1,
                person: person(),
                relevance: Some(Relevance {
                    fts: 1.0,
                    vector: None,
                    fused: 1.0,
                    final_: 0.89,
                }),
                matched: vec![Matched {
                    field: "subjects".into(),
                    matched: Some("devtools dinner".into()),
                    snippet: Some("devtools dinner".into()),
                    ..Matched::default()
                }],
                evidence: vec![],
                reason: None,
                reason_source: None,
            }],
            warnings: vec![],
            suggestions: vec![],
            timings_ms: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "v",
                "query",
                "query_source",
                "rerun",
                "mode",
                "models",
                "total",
                "took_ms",
                "results",
                "warnings",
                "suggestions",
                "timings_ms"
            ]
        );
        assert_eq!(v["v"], 1);
        assert_eq!(v["mode"], "text");
        assert_eq!(v["results"][0]["relevance"]["final"], 0.89);
        assert_eq!(
            v["results"][0]["matched"][0],
            json!({"field": "subjects", "matched": "devtools dinner", "snippet": "devtools dinner"})
        );
        assert!(v["timings_ms"].is_null());
        let back: SearchResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn error_envelope_lists_ambiguous_candidates() {
        let e = crate::Error::Ambiguous {
            reference: "Sam".into(),
            candidates: vec![(3, "Sam Ito".into()), (9, "Sam Park".into())],
        };
        let v = serde_json::to_value(ErrorResponse::from(&e)).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["error"]["code"], "ambiguous");
        assert_eq!(v["error"]["candidates"][1]["id"], 9);
        let nf = serde_json::to_value(ErrorResponse::from(&crate::Error::NotFound(
            "person 4".into(),
        )))
        .unwrap();
        assert!(nf["error"].get("candidates").is_none());
    }

    #[test]
    fn exit_and_http_codes_match_interfaces_doc() {
        let table = [
            (ErrorCode::BadRequest, 400, 2),
            (ErrorCode::NotFound, 404, 5),
            (ErrorCode::Ambiguous, 409, 6),
            (ErrorCode::ViewExpired, 410, 7),
            (ErrorCode::BudgetExceeded, 402, 4),
            (ErrorCode::EgressDisabled, 409, 3),
            (ErrorCode::DbBusy, 503, 10),
            (ErrorCode::SchemaTooNew, 503, 10),
            (ErrorCode::Internal, 500, 1),
        ];
        for (code, http, exit) in table {
            assert_eq!(code.http_status(), http, "{code:?}");
            assert_eq!(code.exit_code(), exit, "{code:?}");
        }
    }
}
