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
use crate::resolve::normalize::{company_key, name_key};

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

const FREEMAIL: &str = include_str!("../../data/freemail.txt");
const DOMAINS: &str = include_str!("../../data/domains.toml");
const GIVEN_NAMES: &str = include_str!("../../data/given_names.txt");
const SURNAMES: &str = include_str!("../../data/surnames.txt");
const COMMON_CJK: &str = include_str!("../../data/common_cjk.txt");

/// Role local parts (entity-resolution.md section 2.5).
pub const ROLE_LOCALPARTS: &[&str] = &[
    "info", "team", "hello", "contact", "sales", "support", "admin", "office", "hr", "jobs",
    "careers", "billing", "founders", "partners", "press", "accounts", "finance", "legal",
];

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
}

fn local_part(email: &str) -> Option<&str> {
    let (local, _) = email.rsplit_once('@')?;
    Some(match local.find('+') {
        Some(p) if p > 0 => &local[..p],
        _ => local,
    })
}

impl DataFiles {
    /// The bundled files merged with `[resolution]` config.
    pub fn load(cfg: &ResolutionSection) -> Self {
        let top_n = usize::try_from(cfg.common_names_top_n).unwrap_or(usize::MAX);
        let mut freemail: BTreeSet<String> = lines(FREEMAIL).map(str::to_ascii_lowercase).collect();
        freemail.extend(
            cfg.freemail_extra
                .iter()
                .map(|d| d.trim().to_ascii_lowercase()),
        );
        freemail.extend(
            cfg.gmail_domains
                .iter()
                .map(|d| d.trim().to_ascii_lowercase()),
        );
        let domains: BTreeMap<String, String> = toml::from_str::<BTreeMap<String, String>>(DOMAINS)
            .unwrap_or_default()
            .into_iter()
            .map(|(d, c)| (d.to_ascii_lowercase(), company_key(&c)))
            .collect();
        let names = |text: &str| -> BTreeSet<String> {
            lines(text)
                .take(top_n)
                .map(name_key)
                .filter(|k| !k.is_empty())
                .collect()
        };
        let mut role_localparts: BTreeSet<String> =
            ROLE_LOCALPARTS.iter().map(|s| s.to_string()).collect();
        role_localparts.extend(
            cfg.role_localparts_extra
                .iter()
                .map(|s| s.trim().to_lowercase()),
        );
        let company_aliases = cfg
            .company_aliases
            .iter()
            .map(|(a, c)| (company_key(a), company_key(c)))
            .filter(|(a, c)| !a.is_empty() && !c.is_empty())
            .collect();
        DataFiles {
            freemail,
            domains,
            given_names: names(GIVEN_NAMES),
            surnames: names(SURNAMES),
            common_cjk: lines(COMMON_CJK).map(name_key).collect(),
            role_localparts,
            company_aliases,
            gmail_domains: cfg
                .gmail_domains
                .iter()
                .map(|d| d.trim().to_ascii_lowercase())
                .collect(),
            large_company_threshold: cfg.large_company_threshold,
        }
    }

    /// First token in `given_names` and last token in `surnames` (or the
    /// whole key in `common_cjk`).
    pub fn is_common_name(&self, name_key: &str) -> bool {
        let toks: Vec<&str> = name_key.split_whitespace().collect();
        match toks.as_slice() {
            [] => false,
            [one] => self.common_cjk.contains(*one),
            [first, .., last] => self.given_names.contains(*first) && self.surnames.contains(*last),
        }
    }

    fn is_freemail(&self, domain: &str) -> bool {
        if self.freemail.contains(domain) {
            return true;
        }
        // A subdomain of a listed domain (`eu.mail.example` under `mail.example`).
        psl::domain_str(domain).is_some_and(|reg| self.freemail.contains(reg))
    }

    /// Company key of an email's registrable domain (none for freemail).
    pub fn email_company_key(&self, email: &str) -> Option<String> {
        let (_, domain) = email.rsplit_once('@')?;
        let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
        if domain.is_empty() || self.is_freemail(&domain) {
            return None;
        }
        if let Some(k) = self.domains.get(&domain) {
            return Some(k.clone());
        }
        let reg = psl::domain_str(&domain)?.to_string();
        if self.is_freemail(&reg) {
            return None;
        }
        if let Some(k) = self.domains.get(&reg) {
            return Some(k.clone());
        }
        let first = reg.split('.').next()?;
        let k = company_key(first);
        (!k.is_empty()).then(|| self.alias(k))
    }

    fn alias(&self, key: String) -> String {
        self.company_aliases.get(&key).cloned().unwrap_or(key)
    }

    /// Mapped company key of a company hint.
    pub fn company_hint_key(&self, company: &str) -> Option<String> {
        let t = company.trim().to_ascii_lowercase();
        // A hint written as a domain ("stripe.com") maps like an email domain.
        if !t.contains(' ') && t.contains('.') && t.split('.').all(|p| !p.is_empty()) {
            if let Some(k) = self.email_company_key(&format!("x@{t}")) {
                return Some(k);
            }
        }
        let k = company_key(company);
        if k.is_empty() {
            return None;
        }
        let k = self.alias(k);
        // A hint naming a mapped company ("Alphabet") uses the mapped key.
        let via_domain = self
            .domains
            .iter()
            .find(|(d, _)| d.split('.').next() == Some(k.as_str()))
            .map(|(_, c)| c.clone());
        Some(via_domain.unwrap_or(k))
    }

    /// Role address local part (`info@`, `sales@`, ...).
    pub fn is_role_address(&self, email: &str) -> bool {
        local_part(&email.to_lowercase()).is_some_and(|l| self.role_localparts.contains(l))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> DataFiles {
        DataFiles::load(&ResolutionSection::default())
    }

    #[test]
    fn bundled_lists_are_loaded_and_normalized() {
        let d = data();
        assert!(d.given_names.len() >= 300, "{}", d.given_names.len());
        assert!(d.surnames.len() >= 300, "{}", d.surnames.len());
        assert!(d.freemail.contains("gmail.com"));
        assert!(d.freemail.contains("googlemail.com"));
        assert!(d.role_localparts.contains("info"));
        assert_eq!(d.gmail_domains, ["gmail.com", "googlemail.com"]);
        assert_eq!(d.large_company_threshold, 50);
        for n in d.given_names.iter().chain(&d.surnames) {
            assert_eq!(
                &crate::resolve::normalize::name_key(n),
                n,
                "not a name key: {n}"
            );
        }
    }

    #[test]
    fn common_names() {
        let d = data();
        for n in [
            "priya sharma",
            "john smith",
            "wei zhang",
            "alex kim",
            "maria garcia",
        ] {
            assert!(d.is_common_name(n), "{n}");
        }
        for n in [
            "rohan quillfeather",
            "ines quillfeather",
            "jon quillby",
            "jane",
            "",
        ] {
            assert!(!d.is_common_name(n), "{n}");
        }
        assert!(d.is_common_name("王伟"));
        assert!(!d.is_common_name("王小明"));
    }

    #[test]
    fn top_n_truncates_the_lists() {
        let d = DataFiles::load(&ResolutionSection {
            common_names_top_n: 10,
            ..ResolutionSection::default()
        });
        assert_eq!(d.given_names.len(), 10);
        assert_eq!(d.surnames.len(), 10);
    }

    #[test]
    fn email_company_keys() {
        let d = data();
        assert_eq!(
            d.email_company_key("jane@mail.stripe.com").as_deref(),
            Some("stripe")
        );
        assert_eq!(
            d.email_company_key("sam.ito@northwind.example").as_deref(),
            Some("northwind")
        );
        assert_eq!(
            d.email_company_key("a@alphabet.com").as_deref(),
            Some("google")
        );
        assert_eq!(d.email_company_key("a@fb.com").as_deref(), Some("meta"));
        assert_eq!(d.email_company_key("a@gmail.com"), None);
        assert_eq!(d.email_company_key("a@hotmail.co.uk"), None);
        assert_eq!(d.email_company_key("not-an-email"), None);
    }

    #[test]
    fn config_extends_the_data_files() {
        let d = DataFiles::load(&ResolutionSection {
            freemail_extra: vec!["mail.test".into()],
            company_aliases: [("Stripe Payments".to_string(), "Stripe".to_string())].into(),
            role_localparts_extra: vec!["concierge".into()],
            gmail_domains: vec!["gmail.test".into(), "googlemail.test".into()],
            ..ResolutionSection::default()
        });
        assert_eq!(d.email_company_key("lena.park@mail.test"), None);
        assert_eq!(
            d.company_hint_key("Stripe Payments, Inc.").as_deref(),
            Some("stripe")
        );
        assert!(d.is_role_address("concierge@hotel.example"));
        assert_eq!(d.gmail_domains[0], "gmail.test");
    }

    #[test]
    fn company_hints() {
        let d = data();
        assert_eq!(
            d.company_hint_key("Northwind").as_deref(),
            Some("northwind")
        );
        assert_eq!(
            d.company_hint_key("Stripe, Inc.").as_deref(),
            Some("stripe")
        );
        assert_eq!(d.company_hint_key("stripe.com").as_deref(), Some("stripe"));
        assert_eq!(
            d.company_hint_key("Alphabet Inc.").as_deref(),
            Some("google")
        );
        assert_eq!(d.company_hint_key("  "), None);
        assert_eq!(d.company_hint_key("--"), None);
    }

    #[test]
    fn role_addresses() {
        let d = data();
        assert!(d.is_role_address("info@startup.example"));
        assert!(d.is_role_address("Sales+eu@acme.example"));
        assert!(!d.is_role_address("jane@acme.example"));
        assert!(!d.is_role_address("information"));
    }
}
