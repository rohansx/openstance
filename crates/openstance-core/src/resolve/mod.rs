//! Entity resolution: identities -> people. Conservative by design: a wrong
//! merge is worse than a missed one. Spec: `docs/tech-spec/entity-resolution.md`.
//!
//! Owned by the "resolve" stream (P1-CORE-06, -07, -12 and the review /
//! merge / split commands). Module map:
//! - [`normalize`]: storage normalization and match keys (section 2);
//! - [`rules`]: data files, common names, rule confidences (section 3);
//! - [`clusters`]: union-find, rules 1-4, dormant filter (steps 1-13);
//! - [`fields`]: person fields from hints + overrides (step 15);
//! - [`review`]: the review queue and its decisions (section 8);
//! - [`commands`]: merge / split (section 9);
//! - [`lookup`]: person references (id, email, slug, name).

use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::config::Config;
use crate::error::Result;
use crate::ingest::Touched;
use crate::{IdentityId, PersonId};

pub mod clusters;
pub mod commands;
pub mod fields;
pub mod lookup;
pub mod normalize;
pub mod review;
pub mod rules;

/// Which rule attached an identity to its person (`identity.match_rule`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MatchRule {
    /// The cluster's anchor (minimum identity id).
    Anchor,
    /// Rule 1: same handle or source-asserted co-reference.
    Exact,
    /// Rule 2: Gmail / plus aliasing.
    GmailAlias,
    /// Rule 3: name + company (flagged).
    NameCompany,
    /// Rule 4: name + mutual thread (flagged).
    NameThread,
    /// A `merged_into` override or `[me]` must-link.
    Override,
    /// An enrichment identity (Phase 3).
    Enrich,
}

impl MatchRule {
    /// The string stored in `identity.match_rule`.
    pub fn as_str(self) -> &'static str {
        match self {
            MatchRule::Anchor => "anchor",
            MatchRule::Exact => "exact",
            MatchRule::GmailAlias => "gmail_alias",
            MatchRule::NameCompany => "name_company",
            MatchRule::NameThread => "name_thread",
            MatchRule::Override => "override",
            MatchRule::Enrich => "enrich",
        }
    }

    /// Parse a stored `match_rule`.
    pub fn parse(s: &str) -> Option<MatchRule> {
        [
            MatchRule::Anchor,
            MatchRule::Exact,
            MatchRule::GmailAlias,
            MatchRule::NameCompany,
            MatchRule::NameThread,
            MatchRule::Override,
            MatchRule::Enrich,
        ]
        .into_iter()
        .find(|r| r.as_str() == s)
    }

    /// Fixed confidence of the rule (enrich is `<= 0.9`, reported per edge).
    pub fn confidence(self) -> f64 {
        match self {
            MatchRule::Anchor | MatchRule::Exact | MatchRule::Override => 1.0,
            MatchRule::GmailAlias => 0.98,
            MatchRule::NameCompany => 0.85,
            MatchRule::NameThread => 0.75,
            MatchRule::Enrich => 0.9,
        }
    }

    /// Merged but listed in `review` until confirmed or split.
    pub fn is_flagged(self) -> bool {
        matches!(self, MatchRule::NameCompany | MatchRule::NameThread)
    }
}

/// Where one identity landed after resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Assignment {
    /// The person (anchor = minimum identity id of the cluster).
    pub person_id: PersonId,
    /// Min confidence over the union-tree path to the anchor.
    pub confidence: f64,
    /// Rule of the weakest edge on that path.
    pub rule: MatchRule,
}

/// The in-memory result of steps 1-13: a pure function of raw rows,
/// overrides, config and data files.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolution {
    /// Every identity that belongs to a person. Dormant identities are absent.
    pub assignments: BTreeMap<IdentityId, Assignment>,
    /// The person containing a `[me]` identity, if any.
    pub self_person: Option<PersonId>,
    /// Unions skipped because of a `split_from`.
    pub blocked_by_override: u64,
    /// `override.id`s of must-links skipped as conflicting.
    pub conflicting_overrides: Vec<i64>,
}

impl Resolution {
    /// Distinct person ids, ascending.
    pub fn people(&self) -> Vec<PersonId> {
        let mut v: Vec<PersonId> = self.assignments.values().map(|a| a.person_id).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// What changed when a [`Resolution`] was written to the database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Materialized {
    /// People whose identity set or interactions changed (fields, score,
    /// ties and FTS must be recomputed for these).
    pub affected: Vec<PersonId>,
    /// Person ids that no longer exist (their anchors were absorbed).
    pub removed: Vec<PersonId>,
}

/// Steps 1-13 of the rebuild algorithm, in memory, over all identities.
pub fn resolve(conn: &Connection, cfg: &Config) -> Result<Resolution> {
    clusters::resolve(conn, cfg)
}

/// Step 14, full: clear derived assignments and write `person` rows,
/// `identity.person_id/confidence/match_rule`, `interaction_person.person_id`,
/// `person.is_self` and `interaction.direction` for everyone.
pub fn materialize_all(conn: &Connection, res: &Resolution) -> Result<Materialized> {
    let _ = (conn, res);
    todo!("resolve stream: P1-CORE-07/09 materialize_all (step 14)")
}

/// Step 14, incremental: diff `res` against the stored assignment and write
/// only what changed. People touched by `touched` count as affected even if
/// their identity set is unchanged. Must leave the database exactly as
/// [`materialize_all`] would (incremental equals full).
pub fn materialize_diff(
    conn: &Connection,
    res: &Resolution,
    touched: &Touched,
) -> Result<Materialized> {
    let _ = (conn, res, touched);
    todo!("resolve stream: P1-CORE-09 materialize_diff")
}
