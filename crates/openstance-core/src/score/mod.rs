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
use crate::error::{Error, Result};
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
        let mut warnings = Vec::new();
        positive("score.half_life_days", s.half_life_days)?;
        positive("score.frequency_scale", s.frequency_scale)?;
        positive("score.meetings_scale", s.meetings_scale)?;
        positive("score.tie_scale", s.tie_scale)?;
        if !(s.thread_start_multiplier.is_finite() && s.thread_start_multiplier >= 1.0) {
            return Err(config_err("score.thread_start_multiplier", "must be >= 1"));
        }
        if s.tie_max_participants < 2 {
            return Err(config_err("score.tie_max_participants", "must be >= 2"));
        }
        if s.bulk.min_messages < 1 {
            return Err(config_err("score.bulk.min_messages", "must be >= 1"));
        }
        let ratio = s.bulk.list_unsubscribe_ratio;
        if !(ratio.is_finite() && (0.0..=1.0).contains(&ratio)) {
            return Err(config_err(
                "score.bulk.list_unsubscribe_ratio",
                "must be between 0 and 1",
            ));
        }

        let (weights, preset) = match (&s.preset, &s.weights) {
            (Some(_), Some(_)) => {
                return Err(config_err(
                    "score.weights",
                    "set either score.preset or score.weights, not both",
                ))
            }
            (Some(name), None) => {
                let w = presets::preset(name).ok_or_else(|| {
                    config_err(
                        "score.preset",
                        &format!(
                            "unknown preset; expected one of {}",
                            presets::PRESET_NAMES.join(", ")
                        ),
                    )
                })?;
                (w, Some(name.clone()))
            }
            (None, None) => (
                presets::preset("default").expect("default preset exists"),
                Some("default".to_string()),
            ),
            (None, Some(w)) => {
                let all = [w.recency, w.frequency, w.reciprocity, w.meetings, w.breadth];
                if all.iter().any(|x| !x.is_finite() || *x < 0.0) {
                    return Err(config_err("score.weights", "every weight must be >= 0"));
                }
                let sum: f64 = all.iter().sum();
                if sum <= 0.0 {
                    return Err(config_err(
                        "score.weights",
                        "weights must have a positive sum",
                    ));
                }
                let n = Weights {
                    recency: w.recency / sum,
                    frequency: w.frequency / sum,
                    reciprocity: w.reciprocity / sum,
                    meetings: w.meetings / sum,
                    breadth: w.breadth / sum,
                };
                if (sum - 1.0).abs() > 1e-6 {
                    let msg = format!(
                        "score.weights sum to {sum}; normalized to recency {:.4}, frequency {:.4}, reciprocity {:.4}, meetings {:.4}, breadth {:.4}",
                        n.recency, n.frequency, n.reciprocity, n.meetings, n.breadth
                    );
                    tracing::warn!("{msg}");
                    warnings.push(msg);
                }
                (n, None)
            }
        };

        Ok((
            ScoreConfig {
                weights,
                preset,
                half_life_days: s.half_life_days,
                frequency_scale: s.frequency_scale,
                meetings_scale: s.meetings_scale,
                thread_start_multiplier: s.thread_start_multiplier,
                tie_scale: s.tie_scale,
                tie_max_participants: s.tie_max_participants as usize,
                bulk_min_messages: s.bulk.min_messages,
                bulk_list_unsubscribe_ratio: ratio,
            },
            warnings,
        ))
    }

    /// blake3 of the effective config, stored as `meta.score_config_hash`.
    pub fn hash(&self) -> String {
        // Floats are hashed by their exact bit pattern so any change counts.
        let w = &self.weights;
        let canonical = format!(
            "score-config-v1|preset={}|w={:016x},{:016x},{:016x},{:016x},{:016x}|half_life={:016x}|fscale={:016x}|mscale={:016x}|tsm={:016x}|tie_scale={:016x}|tie_max={}|bulk_min={}|bulk_ratio={:016x}",
            self.preset.as_deref().unwrap_or("custom"),
            w.recency.to_bits(),
            w.frequency.to_bits(),
            w.reciprocity.to_bits(),
            w.meetings.to_bits(),
            w.breadth.to_bits(),
            self.half_life_days.to_bits(),
            self.frequency_scale.to_bits(),
            self.meetings_scale.to_bits(),
            self.thread_start_multiplier.to_bits(),
            self.tie_scale.to_bits(),
            self.tie_max_participants,
            self.bulk_min_messages,
            self.bulk_list_unsubscribe_ratio.to_bits(),
        );
        blake3::hash(canonical.as_bytes()).to_hex().to_string()
    }

    /// Preset name for display: the preset, or `"custom"`.
    pub fn preset_name(&self) -> &str {
        self.preset.as_deref().unwrap_or("custom")
    }
}

fn config_err(key: &str, message: &str) -> Error {
    Error::Config {
        key: key.to_string(),
        message: message.to_string(),
    }
}

fn positive(key: &str, v: f64) -> Result<()> {
    if v.is_finite() && v > 0.0 {
        Ok(())
    } else {
        Err(config_err(key, "must be > 0"))
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
        match kind {
            "mbox" | "imap" => Some(Channel::Email),
            "ics" => Some(Channel::Calendar),
            "linkedin_csv" => Some(Channel::Linkedin),
            "x_archive" | "whatsapp" | "slack" | "discord" | "telegram" => Some(Channel::Social),
            _ => None,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ScoreSection, WeightsInput};

    #[test]
    fn empty_config_uses_default_preset() {
        let (c, warnings) = ScoreConfig::from_config(&ScoreSection::default()).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(c.preset.as_deref(), Some("default"));
        assert_eq!(c.weights, presets::preset("default").unwrap());
        assert_eq!(c.half_life_days, 180.0);
        assert_eq!(c.frequency_scale, 20.0);
        assert_eq!(c.meetings_scale, 3.0);
        assert_eq!(c.thread_start_multiplier, 2.0);
        assert_eq!(c.tie_scale, 3.0);
        assert_eq!(c.tie_max_participants, 20);
        assert_eq!(c.bulk_min_messages, 50);
        assert_eq!(c.bulk_list_unsubscribe_ratio, 0.5);
    }

    #[test]
    fn named_preset_is_used() {
        let s = ScoreSection {
            preset: Some("founder".into()),
            ..ScoreSection::default()
        };
        let (c, _) = ScoreConfig::from_config(&s).unwrap();
        assert_eq!(c.preset.as_deref(), Some("founder"));
        assert_eq!(c.weights, presets::preset("founder").unwrap());
    }

    #[test]
    fn unknown_preset_is_a_config_error() {
        let s = ScoreSection {
            preset: Some("vip".into()),
            ..ScoreSection::default()
        };
        let err = ScoreConfig::from_config(&s).unwrap_err();
        assert!(err.to_string().contains("score.preset"), "{err}");
    }

    #[test]
    fn explicit_weights_override_and_normalize_with_warning() {
        let s = ScoreSection {
            weights: Some(WeightsInput {
                recency: 2.0,
                frequency: 1.0,
                reciprocity: 1.0,
                meetings: 0.0,
                breadth: 0.0,
            }),
            ..ScoreSection::default()
        };
        let (c, warnings) = ScoreConfig::from_config(&s).unwrap();
        assert_eq!(c.preset, None);
        assert_eq!(c.weights.recency, 0.5);
        assert_eq!(c.weights.frequency, 0.25);
        assert_eq!(c.weights.meetings, 0.0);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("score.weights"));
    }

    #[test]
    fn explicit_weights_summing_to_one_do_not_warn() {
        let s = ScoreSection {
            weights: Some(WeightsInput {
                recency: 0.30,
                frequency: 0.15,
                reciprocity: 0.30,
                meetings: 0.20,
                breadth: 0.05,
            }),
            ..ScoreSection::default()
        };
        let (c, warnings) = ScoreConfig::from_config(&s).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!((c.weights.reciprocity - 0.30).abs() < 1e-12);
    }

    #[test]
    fn preset_and_weights_together_is_an_error() {
        let s = ScoreSection {
            preset: Some("default".into()),
            weights: Some(WeightsInput {
                recency: 1.0,
                frequency: 0.0,
                reciprocity: 0.0,
                meetings: 0.0,
                breadth: 0.0,
            }),
            ..ScoreSection::default()
        };
        assert!(ScoreConfig::from_config(&s).is_err());
    }

    #[test]
    fn invalid_values_are_errors() {
        let bad = [
            ScoreSection {
                half_life_days: 0.0,
                ..ScoreSection::default()
            },
            ScoreSection {
                tie_scale: -1.0,
                ..ScoreSection::default()
            },
            ScoreSection {
                tie_max_participants: 1,
                ..ScoreSection::default()
            },
            ScoreSection {
                weights: Some(WeightsInput {
                    recency: 0.0,
                    frequency: 0.0,
                    reciprocity: 0.0,
                    meetings: 0.0,
                    breadth: 0.0,
                }),
                ..ScoreSection::default()
            },
            ScoreSection {
                weights: Some(WeightsInput {
                    recency: -0.1,
                    frequency: 1.0,
                    reciprocity: 0.0,
                    meetings: 0.0,
                    breadth: 0.0,
                }),
                ..ScoreSection::default()
            },
        ];
        for s in bad {
            assert!(ScoreConfig::from_config(&s).is_err(), "{s:?}");
        }
    }

    #[test]
    fn hash_is_stable_and_changes_with_any_key() {
        let base = ScoreConfig::from_config(&ScoreSection::default())
            .unwrap()
            .0;
        assert_eq!(base.hash(), base.clone().hash());
        assert_eq!(base.hash().len(), 64);
        let mut other = base.clone();
        other.half_life_days = 120.0;
        assert_ne!(base.hash(), other.hash());
        let founder = ScoreConfig::from_config(&ScoreSection {
            preset: Some("founder".into()),
            ..ScoreSection::default()
        })
        .unwrap()
        .0;
        assert_ne!(base.hash(), founder.hash());
        let mut bulk = base.clone();
        bulk.bulk_min_messages = 10;
        assert_ne!(base.hash(), bulk.hash());
    }

    #[test]
    fn channels_from_source_kinds() {
        assert_eq!(Channel::from_source_kind("mbox"), Some(Channel::Email));
        assert_eq!(Channel::from_source_kind("imap"), Some(Channel::Email));
        assert_eq!(Channel::from_source_kind("ics"), Some(Channel::Calendar));
        assert_eq!(
            Channel::from_source_kind("linkedin_csv"),
            Some(Channel::Linkedin)
        );
        for k in ["x_archive", "whatsapp", "slack", "discord", "telegram"] {
            assert_eq!(Channel::from_source_kind(k), Some(Channel::Social), "{k}");
        }
        for k in ["vcf", "csv", "enrich:treg", "enrich:mock", "unknown"] {
            assert_eq!(Channel::from_source_kind(k), None, "{k}");
        }
    }
}
