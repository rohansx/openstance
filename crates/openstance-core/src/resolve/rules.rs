//! Resolution data files and rule predicates.
//! Spec: `docs/tech-spec/entity-resolution.md` sections 2.3, 2.5 and 3.
//!
//! Owned by the "resolve" stream. Data files live in
//! `crates/openstance-core/data/` (`freemail.txt`, `domains.toml`,
//! `given_names.txt`, `surnames.txt`, `common_cjk.txt`) and are embedded with
//! `include_str!`; config adds `freemail_extra`, `company_aliases`,
//! `role_localparts_extra` and `gmail_domains`.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::ResolutionSection;

/// Rule 1 confidence.
pub const CONF_EXACT: f64 = 1.0;
/// Rule 2 confidence.
pub const CONF_GMAIL_ALIAS: f64 = 0.98;
/// Rule 3 confidence.
pub const CONF_NAME_COMPANY: f64 = 0.85;
/// Rule 4 confidence.
pub const CONF_NAME_THREAD: f64 = 0.75;
/// Rule 5 suggestion confidence.
pub const CONF_RARE_NAME: f64 = 0.5;
/// Large-company guard suggestion confidence.
pub const CONF_LARGE_COMPANY: f64 = 0.6;

/// Bundled data files plus config additions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataFiles {
    /// Consumer mail domains (never a company signal).
    pub freemail: BTreeSet<String>,
    /// Domain -> company key.
    pub domains: BTreeMap<String, String>,
    /// Top given names (name keys).
    pub given_names: BTreeSet<String>,
    /// Top surnames (name keys).
    pub surnames: BTreeSet<String>,
    /// Common full names in space-less scripts.
    pub common_cjk: BTreeSet<String>,
    /// Role local parts (`info`, `sales`, ...).
    pub role_localparts: BTreeSet<String>,
    /// Company alias map (config).
    pub company_aliases: BTreeMap<String, String>,
    /// Dot/plus-insensitive domains; the first is canonical.
    pub gmail_domains: Vec<String>,
    /// Large-company guard threshold.
    pub large_company_threshold: u32,
}

impl DataFiles {
    /// The bundled files merged with `[resolution]` config.
    pub fn load(cfg: &ResolutionSection) -> Self {
        let _ = cfg;
        todo!("resolve stream: P1-CORE-07 DataFiles::load")
    }

    /// First token in `given_names` and last token in `surnames` (or the
    /// whole key in `common_cjk`).
    pub fn is_common_name(&self, name_key: &str) -> bool {
        let _ = name_key;
        todo!("resolve stream: P1-CORE-07 is_common_name")
    }

    /// Company keys of an email's registrable domain (none for freemail).
    pub fn email_company_key(&self, email: &str) -> Option<String> {
        let _ = email;
        todo!("resolve stream: P1-CORE-07 email_company_key")
    }

    /// Mapped company key of a company hint.
    pub fn company_hint_key(&self, company: &str) -> Option<String> {
        let _ = company;
        todo!("resolve stream: P1-CORE-07 company_hint_key")
    }

    /// Role address local part (`info@`, `sales@`, ...).
    pub fn is_role_address(&self, email: &str) -> bool {
        let _ = email;
        todo!("resolve stream: P1-CORE-07 is_role_address")
    }
}
