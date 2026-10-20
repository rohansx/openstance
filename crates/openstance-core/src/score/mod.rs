//! Relationship scoring: components R F C M B, presets, bulk-sender
//! detection and tie derivation. Deterministic and explainable; no model.
//! Spec: `docs/tech-spec/scoring.md` (P1-CORE-10, -11, -13).
//!
//! Owned by the "score" stream. `now` is always passed in; never read the
//! clock here. All SQL lives in loaders; [`components::components`] and
//! [`components::total`] are pure.
//!
//! Called by `rebuild` (never by `ingest`):
//! - [`recompute_all`]: bulk detection, then scores, then ties, for everyone;
//! - [`recompute_people`]: the same for affected people (and their ties);
//! - [`refresh_recency`]: R and S only, when `scored_at` is older than 24 h.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::config::ScoreSection;
use crate::error::Result;
use crate::json;
use crate::PersonId;

pub mod bulk;
pub mod components;
pub mod presets;
pub mod ties;

/// Score weights (non-negative, normalized to sum to 1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Weights {
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
}

impl From<Weights> for json::WeightsJson {
    fn from(w: Weights) -> Self {
        json::WeightsJson {
            recency: w.recency,
            frequency: w.frequency,
            reciprocity: w.reciprocity,
            meetings: w.meetings,
            breadth: w.breadth,
        }
    }
}

/// Effective scoring configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreConfig {
    /// Resolved from the preset or explicit table, normalized.
    pub weights: Weights,
    /// `None` when explicit weights were given (shown as `custom`).
    pub preset: Option<String>,
    /// R = 0.5 at this many days.
    pub half_life_days: f64,
    /// F saturation (20).
    pub frequency_scale: f64,
    /// M saturation (3).
    pub meetings_scale: f64,
    /// C thread-start multiplier (2).
    pub thread_start_multiplier: f64,
    /// Tie saturation (3).
    pub tie_scale: f64,
    /// No ties from interactions with more participants (20).
    pub tie_max_participants: usize,
    /// Bulk rule 1 threshold (50).
    pub bulk_min_messages: u32,
    /// Bulk rule 2 ratio (0.5).
    pub bulk_list_unsubscribe_ratio: f64,
}

impl ScoreConfig {
    /// Resolve `[score]`: preset or explicit weights (normalized, with a
    /// warning when the input sum is off by more than 1e-6).
    pub fn from_config(s: &ScoreSection) -> Result<(ScoreConfig, Vec<String>)> {
        let _ = s;
        todo!("score stream: P1-CORE-11 ScoreConfig::from_config")
    }

    /// blake3 of the effective config, stored as `meta.score_config_hash`.
    pub fn hash(&self) -> String {
        todo!("score stream: ScoreConfig::hash")
    }
}

/// The five components, each in `[0, 1]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Components {
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
}

/// A breadth channel, derived from `source.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// mbox, imap.
    Email,
    /// ics.
    Calendar,
    /// linkedin_csv.
    Linkedin,
    /// x_archive, whatsapp, slack, ...
    Social,
}

impl Channel {
    /// JSON / display name.
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Email => "email",
            Channel::Calendar => "calendar",
            Channel::Linkedin => "linkedin",
            Channel::Social => "social",
        }
    }

    /// Channel of a source kind; `None` for address books and lookups
    /// (`vcf`, `csv`, `enrich:*`).
    pub fn from_source_kind(kind: &str) -> Option<Channel> {
        let _ = kind;
        todo!("score stream: Channel::from_source_kind")
    }
}

/// Raw per-person counts behind the components.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ScoreInputs {
    /// Latest contact-group interaction at or before `now`.
    pub last_contact: Option<i64>,
    /// Fractional days since `last_contact`, clamped at 0.
    pub days_since_last: Option<f64>,
    /// Contact-group interactions.
    pub n_interactions: u32,
    /// Outbound email/dm to them.
    pub n_out: u32,
    /// Their replies.
    pub n_replies: u32,
    /// Threads they started.
    pub n_threads_started: u32,
    /// Past meetings.
    pub n_meetings: u32,
    /// Channels seen, sorted and deduped.
    pub channels: Vec<Channel>,
}

/// What a scoring pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ScoreReport {
    /// People scored.
    pub scored: u64,
    /// People flagged bulk.
    pub bulk: u64,
    /// Tie rows written.
    pub ties: u64,
}

/// Bulk detection, scoring and ties for every person; writes
/// `meta.scored_at` and `meta.score_config_hash`.
pub fn recompute_all(conn: &Connection, cfg: &ScoreConfig, now: i64) -> Result<ScoreReport> {
    let _ = (conn, cfg, now);
    todo!("score stream: P1-CORE-10/11/13 recompute_all")
}

/// The same for `people` only (and every tie touching them).
pub fn recompute_people(
    conn: &Connection,
    cfg: &ScoreConfig,
    people: &[PersonId],
    now: i64,
) -> Result<ScoreReport> {
    let _ = (conn, cfg, people, now);
    todo!("score stream: recompute_people")
}

/// Recompute R and S for everyone from stored components and
/// `last_contact` (one UPDATE pass).
pub fn refresh_recency(conn: &Connection, cfg: &ScoreConfig, now: i64) -> Result<u64> {
    let _ = (conn, cfg, now);
    todo!("score stream: refresh_recency")
}

/// Load the raw inputs for one person (used by `who` and tests).
pub fn inputs(conn: &Connection, person: PersonId, now: i64) -> Result<ScoreInputs> {
    let _ = (conn, person, now);
    todo!("score stream: inputs")
}

/// The full `ScoreComponents` JSON for one person, with `raw` filled.
pub fn detail(
    conn: &Connection,
    cfg: &ScoreConfig,
    person: PersonId,
    now: i64,
) -> Result<json::ScoreComponents> {
    let _ = (conn, cfg, person, now);
    todo!("score stream: detail")
}
