//! `RuleParser`: regex and lexicon rules for company, title, industry,
//! location, tags, dates and flags; whatever is not recognized becomes
//! `text`. Never fails. Spec: `docs/tech-spec/search.md` section 4
//! (P1-MODEL-02; it lives in core per `crates-and-dependencies.md`).
//!
//! Owned by the "search" stream. Golden: `"fintech founders I've met since
//! 2025"` -> `title=[founder], industry=[fintech], met_in_person=true,
//! since=2025-01-01`.

use std::collections::BTreeSet;

use openstance_model::{Query, QueryParser};
use rusqlite::Connection;

use crate::error::Result;

/// Graph-derived vocabularies the parser matches against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lexicon {
    /// Normalized companies (from `person.company` and email domains).
    pub companies: BTreeSet<String>,
    /// Normalized locations.
    pub locations: BTreeSet<String>,
    /// Existing tags.
    pub tags: BTreeSet<String>,
}

impl Lexicon {
    /// Build from the current graph.
    pub fn load(conn: &Connection) -> Result<Lexicon> {
        let _ = conn;
        todo!("search stream: Lexicon::load")
    }
}

/// The always-available rules parser.
#[derive(Debug, Clone)]
pub struct RuleParser {
    #[allow(dead_code)]
    lexicon: Lexicon,
    #[allow(dead_code)]
    tz: chrono_tz::Tz,
}

impl RuleParser {
    /// A parser for one process; `tz` decides local dates.
    pub fn new(lexicon: Lexicon, tz: chrono_tz::Tz) -> Self {
        RuleParser { lexicon, tz }
    }
}

impl QueryParser for RuleParser {
    fn id(&self) -> &str {
        "rules"
    }

    fn is_remote(&self) -> bool {
        false
    }

    fn parse(&self, text: &str, today: i64) -> openstance_model::Result<Query> {
        let _ = (text, today);
        todo!("search stream: P1-MODEL-02 RuleParser::parse")
    }
}
