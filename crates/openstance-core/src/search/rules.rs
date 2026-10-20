//! `RuleParser`: regex and lexicon rules for company, title, industry,
//! location, tags, dates and flags; whatever is not recognized becomes
//! `text`. Never fails. Spec: `docs/tech-spec/search.md` section 4
//! (P1-MODEL-02; it lives in core per `crates-and-dependencies.md`).
//!
//! Owned by the "search" stream. Golden: `"fintech founders I've met since
//! 2025"` -> `title=[founder], industry=[fintech], met_in_person=true,
//! since=2025-01-01`.
//!
//! Pipeline (section 4.2): normalize (NFKC, curly quotes), extract explicit
//! `key:value` syntax and `#tags`, tokenize, then run the date rules, the
//! flag rules and the entity rules in that order. Each match consumes its
//! tokens; at every position the longest phrase wins. Remaining tokens minus
//! stopwords (with the v0.1 alias map) become `text`.
//!
//! Choices the spec leaves open, made here:
//! - `ai`, `ml`, `machine learning` and `devtools` are left as text (the
//!   section 4.7 examples `ml researchers in sf` and `devtools dinner` keep
//!   them as text); industry is soft anyway, so the FTS sees them either way.
//! - The capitalized-word fallback for companies applies after `at`, `@`,
//!   `now at`, `works at`; after `from` only a lexicon company is accepted
//!   (`emails from Priya` must not become a company).
//! - Explicit `location:` values are canonicalized through the alias table
//!   (`location:sf` -> `san francisco`); other explicit values are taken
//!   literally (lowercased).

use std::collections::BTreeSet;
use std::sync::OnceLock;

use chrono::{Datelike, Duration, Months, NaiveDate, TimeZone};
use openstance_model::{Query, QueryParser, MAX_LIMIT};
use regex::Regex;
use rusqlite::Connection;
use unicode_normalization::UnicodeNormalization;

use crate::error::Result;

// ---------------------------------------------------------------------------
// Static tables
// ---------------------------------------------------------------------------

/// Role words -> canonical title (section 4.4).
pub const TITLES: &[(&str, &str)] = &[
    ("founder", "founder"),
    ("founders", "founder"),
    ("co-founder", "founder"),
    ("co-founders", "founder"),
    ("co founder", "founder"),
    ("co founders", "founder"),
    ("cofounder", "founder"),
    ("cofounders", "founder"),
    ("founding", "founder"),
    ("ceo", "ceo"),
    ("ceos", "ceo"),
    ("cto", "cto"),
    ("ctos", "cto"),
    ("cfo", "cfo"),
    ("cfos", "cfo"),
    ("coo", "coo"),
    ("coos", "coo"),
    ("vp", "vp"),
    ("vps", "vp"),
    ("vice president", "vp"),
    ("vice presidents", "vp"),
    ("head of", "head of"),
    ("heads of", "head of"),
    ("director", "director"),
    ("directors", "director"),
    ("engineer", "engineer"),
    ("engineers", "engineer"),
    ("eng", "engineer"),
    ("swe", "engineer"),
    ("swes", "engineer"),
    ("developer", "engineer"),
    ("developers", "engineer"),
    ("dev", "engineer"),
    ("devs", "engineer"),
    ("pm", "pm"),
    ("pms", "pm"),
    ("product manager", "pm"),
    ("product managers", "pm"),
    ("designer", "designer"),
    ("designers", "designer"),
    ("recruiter", "recruiter"),
    ("recruiters", "recruiter"),
    ("talent", "recruiter"),
    ("investor", "investor"),
    ("investors", "investor"),
    ("vc", "investor"),
    ("vcs", "investor"),
    ("angel", "investor"),
    ("angels", "investor"),
    ("partner at a fund", "investor"),
    ("researcher", "researcher"),
    ("researchers", "researcher"),
    ("sales", "sales"),
    ("ae", "sales"),
    ("aes", "sales"),
    ("account executive", "sales"),
    ("account executives", "sales"),
    ("seller", "sales"),
    ("sellers", "sales"),
    ("marketing", "marketing"),
    ("marketer", "marketing"),
    ("marketers", "marketing"),
    ("advisor", "advisor"),
    ("advisors", "advisor"),
    ("adviser", "advisor"),
    ("advisers", "advisor"),
];

/// Terms a canonical title matches in `person.title` (filters expand these).
pub fn title_terms(canonical: &str) -> Vec<String> {
    let extra: &[&str] = match canonical {
        "founder" => &["founder", "cofounder", "founding"],
        "engineer" => &["engineer", "engineering", "swe", "developer"],
        "pm" => &["pm", "product manager"],
        "vp" => &["vp", "vice president"],
        "investor" => &["investor", "vc", "angel", "venture"],
        "recruiter" => &["recruiter", "recruiting", "talent"],
        "sales" => &["sales", "account executive"],
        "advisor" => &["advisor", "adviser"],
        "marketing" => &["marketing", "marketer"],
        _ => &[],
    };
    let mut out = vec![canonical.to_string()];
    for e in extra {
        if !out.iter().any(|o| o == e) {
            out.push((*e).to_string());
        }
    }
    out
}

/// Industry words -> canonical industry (section 4.4).
pub const INDUSTRIES: &[(&str, &str)] = &[
    ("fintech", "fintech"),
    ("fin tech", "fintech"),
    ("crypto", "crypto"),
    ("web3", "crypto"),
    ("healthtech", "healthtech"),
    ("health tech", "healthtech"),
    ("health", "healthtech"),
    ("climate", "climate"),
    ("climatetech", "climate"),
    ("climate tech", "climate"),
    ("security", "security"),
    ("infosec", "security"),
    ("cybersecurity", "security"),
    ("saas", "saas"),
    ("b2b", "b2b"),
    ("consumer", "consumer"),
    ("gaming", "gaming"),
    ("edtech", "edtech"),
    ("biotech", "biotech"),
    ("robotics", "robotics"),
    ("hardware", "hardware"),
];

/// Location aliases (alias -> canonical); canonical names map to themselves.
pub const LOCATION_ALIASES: &[(&str, &str)] = &[
    ("sf", "san francisco"),
    ("san francisco", "san francisco"),
    ("bay area", "san francisco"),
    ("nyc", "new york"),
    ("new york city", "new york"),
    ("new york", "new york"),
    ("la", "los angeles"),
    ("los angeles", "los angeles"),
    ("bangalore", "bengaluru"),
    ("bengaluru", "bengaluru"),
    ("bombay", "mumbai"),
    ("mumbai", "mumbai"),
    ("new delhi", "delhi"),
    ("delhi", "delhi"),
    ("london", "london"),
    ("berlin", "berlin"),
    ("paris", "paris"),
    ("seattle", "seattle"),
    ("boston", "boston"),
    ("austin", "austin"),
    ("toronto", "toronto"),
    ("singapore", "singapore"),
    ("tokyo", "tokyo"),
    ("amsterdam", "amsterdam"),
    ("dublin", "dublin"),
    ("chicago", "chicago"),
    ("tel aviv", "tel aviv"),
    ("sydney", "sydney"),
];

/// Every spelling that means the same place as `value` (for filters).
pub fn location_terms(value: &str) -> Vec<String> {
    let v = value.trim().to_lowercase();
    let canonical = canonical_location(&v).unwrap_or_else(|| v.clone());
    let mut out = vec![canonical.clone()];
    for (alias, c) in LOCATION_ALIASES {
        if *c == canonical && !out.iter().any(|o| o == alias) {
            out.push((*alias).to_string());
        }
    }
    if !out.contains(&v) {
        out.push(v);
    }
    out
}

fn canonical_location(v: &str) -> Option<String> {
    LOCATION_ALIASES
        .iter()
        .find(|(a, _)| *a == v)
        .map(|(_, c)| (*c).to_string())
}

/// Aliases applied to leftover text tokens (the v0.1 synonym map).
const TEXT_ALIASES: &[(&str, &str)] = &[
    ("devtools", "developer tools"),
    ("sf", "san francisco"),
    ("bangalore", "bengaluru"),
    ("nyc", "new york"),
];

/// Words that never become `text` (the v0.1 `STOP` list, extended).
const STOPWORDS: &[&str] = &[
    "a",
    "an",
    "the",
    "and",
    "or",
    "of",
    "to",
    "in",
    "on",
    "at",
    "for",
    "with",
    "from",
    "by",
    "about",
    "as",
    "into",
    "who",
    "whom",
    "whose",
    "which",
    "that",
    "what",
    "where",
    "when",
    "how",
    "do",
    "does",
    "did",
    "i",
    "i'm",
    "im",
    "i've",
    "ive",
    "i'd",
    "me",
    "my",
    "mine",
    "we",
    "us",
    "our",
    "you",
    "your",
    "they",
    "them",
    "their",
    "he",
    "she",
    "his",
    "her",
    "it",
    "its",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "am",
    "have",
    "has",
    "had",
    "know",
    "knows",
    "knew",
    "known",
    "people",
    "person",
    "persons",
    "folks",
    "someone",
    "somebody",
    "anyone",
    "anybody",
    "everyone",
    "any",
    "all",
    "some",
    "now",
    "currently",
    "works",
    "work",
    "working",
    "worked",
    "find",
    "show",
    "list",
    "get",
    "give",
    "contacts",
    "guys",
    "there",
    "here",
    "this",
    "these",
    "those",
    "since",
    "until",
    "after",
    "before",
    "during",
    "near",
    "based",
    "just",
    "also",
    "very",
    "really",
    "please",
    "can",
    "could",
    "would",
    "should",
    "will",
    "still",
    "ones",
    "like",
    "than",
    "then",
    "so",
    "but",
    "not",
    "more",
    "most",
    "other",
    "else",
    "actually",
    "ever",
    "via",
    "@",
    "who's",
    "whos",
    "theyre",
    "they're",
    "'s",
];

/// Dictionary words that never count as a bare company mention.
const COMMON_WORDS: &[&str] = &[
    "notion", "stripe", "square", "apple", "slack", "box", "ramp", "plaid", "amazon", "oracle",
    "shell", "target", "gap", "mail", "email", "meta", "block", "cash", "signal", "discord",
    "zoom", "linear", "loom", "monday", "asana", "ripple", "coinbase", "dinner", "lunch", "coffee",
    "meeting", "intro", "chat", "friends", "friend", "team", "group", "company", "startup",
    "product", "design", "data", "cloud", "labs", "health", "people",
];

/// Personal mail domain labels that never yield a company.
const FREEMAIL_LABELS: &[&str] = &[
    "gmail",
    "googlemail",
    "outlook",
    "hotmail",
    "live",
    "msn",
    "yahoo",
    "ymail",
    "icloud",
    "me",
    "mac",
    "proton",
    "protonmail",
    "aol",
    "gmx",
    "yandex",
    "qq",
    "163",
    "rediffmail",
    "fastmail",
    "hey",
    "zoho",
    "mail",
    "email",
    "example",
];

const NUMBER_WORDS: &[(&str, u32)] = &[
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
];

const MONTHS: &[(&str, u32)] = &[
    ("january", 1),
    ("jan", 1),
    ("february", 2),
    ("feb", 2),
    ("march", 3),
    ("mar", 3),
    ("april", 4),
    ("apr", 4),
    ("may", 5),
    ("june", 6),
    ("jun", 6),
    ("july", 7),
    ("jul", 7),
    ("august", 8),
    ("aug", 8),
    ("september", 9),
    ("sep", 9),
    ("sept", 9),
    ("october", 10),
    ("oct", 10),
    ("november", 11),
    ("nov", 11),
    ("december", 12),
    ("dec", 12),
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Flag {
    Met(bool),
    Replied(bool),
    LostTouch,
    Close,
}

/// Flag phrases (section 4.6); negative forms are listed so that the
/// longest-match rule picks them over the positive ones.
const FLAGS: &[(&str, Flag)] = &[
    ("never met", Flag::Met(false)),
    ("haven't met", Flag::Met(false)),
    ("havent met", Flag::Met(false)),
    ("have not met", Flag::Met(false)),
    ("not met", Flag::Met(false)),
    ("never meet", Flag::Met(false)),
    ("met in person", Flag::Met(true)),
    ("in person", Flag::Met(true)),
    ("i've met", Flag::Met(true)),
    ("ive met", Flag::Met(true)),
    ("i have met", Flag::Met(true)),
    ("have met", Flag::Met(true)),
    ("actually met", Flag::Met(true)),
    ("had a meeting with", Flag::Met(true)),
    ("i met", Flag::Met(true)),
    ("met", Flag::Met(true)),
    ("never replied", Flag::Replied(false)),
    ("never responded", Flag::Replied(false)),
    ("didn't reply", Flag::Replied(false)),
    ("didnt reply", Flag::Replied(false)),
    ("did not reply", Flag::Replied(false)),
    ("ghosted", Flag::Replied(false)),
    ("no reply", Flag::Replied(false)),
    ("who replied", Flag::Replied(true)),
    ("replied", Flag::Replied(true)),
    ("responded", Flag::Replied(true)),
    ("wrote back", Flag::Replied(true)),
    ("actually talk to", Flag::Replied(true)),
    ("talk to", Flag::Replied(true)),
    ("lost touch with", Flag::LostTouch),
    ("lost touch", Flag::LostTouch),
    ("lost-touch", Flag::LostTouch),
    ("drifted", Flag::LostTouch),
    ("haven't talked to in a while", Flag::LostTouch),
    ("havent talked to in a while", Flag::LostTouch),
    ("haven't talked in a while", Flag::LostTouch),
    ("fell out of touch", Flag::LostTouch),
    ("should reconnect", Flag::LostTouch),
    ("close", Flag::Close),
    ("warm", Flag::Close),
    ("strong", Flag::Close),
    ("know well", Flag::Close),
];

/// `min_score` set by "close", "warm", "strong", "know well".
pub const CLOSE_MIN_SCORE: f32 = 0.4;

const LOCATION_TRIGGERS: &[&str] = &["based in", "located in", "living in", "in", "near", "from"];
const COMPANY_TRIGGERS: &[&str] = &[
    "who knows someone at",
    "who knows anyone at",
    "knows someone at",
    "someone at",
    "anyone at",
    "works at",
    "working at",
    "work at",
    "now at",
    "at",
    "@",
    "from",
];
const NOT_SINCE: &[&str] = &[
    "haven't talked to since",
    "havent talked to since",
    "haven't spoken to since",
    "havent spoken to since",
    "haven't talked since",
    "haven't spoken since",
    "haven't heard from since",
    "not talked to since",
    "not spoken to since",
    "not since",
];

// ---------------------------------------------------------------------------
// Lexicon
// ---------------------------------------------------------------------------

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
        let mut lex = Lexicon::default();
        let mut stmt =
            conn.prepare("SELECT DISTINCT company FROM person WHERE company IS NOT NULL")?;
        for c in stmt.query_map([], |r| r.get::<_, String>(0))? {
            let k = company_norm(&c?);
            if !k.is_empty() {
                lex.companies.insert(k);
            }
        }
        let mut stmt = conn.prepare("SELECT value FROM identity WHERE kind = 'email'")?;
        for e in stmt.query_map([], |r| r.get::<_, String>(0))? {
            if let Some(label) = email_company_label(&e?) {
                lex.companies.insert(label);
            }
        }
        let mut stmt =
            conn.prepare("SELECT DISTINCT location FROM person WHERE location IS NOT NULL")?;
        for l in stmt.query_map([], |r| r.get::<_, String>(0))? {
            let l = l?;
            let full = simple_norm(&l.replace(',', " "));
            if !full.is_empty() {
                lex.locations.insert(full);
            }
            if let Some(first) = l.split(',').next() {
                let first = simple_norm(first);
                if !first.is_empty() {
                    lex.locations.insert(first);
                }
            }
        }
        let mut stmt = conn.prepare(
            "SELECT DISTINCT lower(trim(value)) FROM override
             WHERE field = 'tag' AND value IS NOT NULL AND trim(value) <> ''",
        )?;
        for t in stmt.query_map([], |r| r.get::<_, String>(0))? {
            lex.tags.insert(t?);
        }
        Ok(lex)
    }
}

/// NFKC, lowercase, punctuation (except `& ' -`) to spaces, collapse spaces.
fn simple_norm(s: &str) -> String {
    let s: String = s
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '&' | '\'' | '-') {
                c
            } else {
                ' '
            }
        })
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

const LEGAL_SUFFIXES: &[&str] = &[
    "inc",
    "incorporated",
    "llc",
    "ltd",
    "limited",
    "pvt",
    "private",
    "plc",
    "gmbh",
    "ag",
    "sa",
    "sas",
    "srl",
    "bv",
    "nv",
    "co",
    "corp",
    "corporation",
    "company",
    "pte",
    "pty",
    "oy",
    "ab",
    "kk",
];

/// Normalized company name: [`simple_norm`], drop a leading `the`, drop
/// trailing legal-form tokens.
pub fn company_norm(s: &str) -> String {
    let n = simple_norm(&s.replace(['.', ','], " "));
    let mut toks: Vec<&str> = n.split(' ').filter(|t| !t.is_empty()).collect();
    if toks.len() > 1 && toks[0] == "the" {
        toks.remove(0);
    }
    while toks.len() > 1 && toks.last().is_some_and(|t| LEGAL_SUFFIXES.contains(t)) {
        toks.pop();
    }
    toks.join(" ")
}

/// The company label of a work email (`a@mail.stripe.com` -> `stripe`), or
/// `None` for personal mail domains.
pub fn email_company_label(email: &str) -> Option<String> {
    let domain = email.rsplit_once('@')?.1.trim().to_lowercase();
    let registrable = psl::domain_str(&domain).unwrap_or(&domain).to_string();
    let label = registrable.split('.').next()?.to_string();
    if label.len() < 2 || FREEMAIL_LABELS.contains(&label.as_str()) {
        return None;
    }
    Some(label)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// The always-available rules parser.
#[derive(Debug, Clone)]
pub struct RuleParser {
    lexicon: Lexicon,
    tz: chrono_tz::Tz,
}

/// Result of [`RuleParser::parse_detailed`].
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// The query.
    pub query: Query,
    /// Non-fatal problems (`could not parse date: 'Q5'`).
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct Tok {
    raw: String,
    norm: String,
    used: bool,
}

/// Values from `key:value` syntax; they win over phrase rules.
#[derive(Debug, Default)]
struct Explicit {
    company: Vec<String>,
    title: Vec<String>,
    location: Vec<String>,
    industry: Vec<String>,
    tags: Vec<String>,
    since: Option<i64>,
    until: Option<i64>,
    last_contact_before: Option<i64>,
    met: Option<bool>,
    replied: Option<bool>,
    lost_touch: Option<bool>,
    min_score: Option<f32>,
    limit: Option<usize>,
    include_bulk: Option<bool>,
}

enum DateEffect {
    Window(Option<i64>, Option<i64>),
    LastContactBefore(i64),
}

fn explicit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)(?:^|\s)(company|at|title|role|location|in|industry|tag|since|until|before|met|replied|min-score|limit|bulk):(?:"([^"]*)"?|(\S*))"#,
        )
        .unwrap_or_else(|_| unreachable!("static regex"))
    })
}

fn hashtag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:^|\s)#([\p{L}\p{N}][\p{L}\p{N}_-]*)")
            .unwrap_or_else(|_| unreachable!("static regex"))
    })
}

fn push_unique(v: &mut Vec<String>, s: impl Into<String>) {
    let s = s.into();
    if !s.is_empty() && !v.contains(&s) {
        v.push(s);
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "" | "yes" | "y" | "true" | "1" => Some(true),
        "no" | "n" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn number(tok: &str) -> Option<u32> {
    if let Ok(n) = tok.parse::<u32>() {
        return (n > 0 && n <= 100_000).then_some(n);
    }
    NUMBER_WORDS
        .iter()
        .find(|(w, _)| *w == tok)
        .map(|(_, n)| *n)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Unit {
    Day,
    Week,
    Month,
    Year,
}

fn unit(tok: &str) -> Option<Unit> {
    match tok {
        "day" | "days" => Some(Unit::Day),
        "week" | "weeks" => Some(Unit::Week),
        "month" | "months" => Some(Unit::Month),
        "year" | "years" => Some(Unit::Year),
        _ => None,
    }
}

fn sub_units(d: NaiveDate, n: u32, u: Unit) -> Option<NaiveDate> {
    match u {
        Unit::Day => d.checked_sub_signed(Duration::days(i64::from(n))),
        Unit::Week => d.checked_sub_signed(Duration::weeks(i64::from(n))),
        Unit::Month => d.checked_sub_months(Months::new(n)),
        Unit::Year => d.checked_sub_months(Months::new(n.checked_mul(12)?)),
    }
}

fn month_start(y: i32, m: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(y, m, 1)
}

fn next_month(d: NaiveDate) -> Option<NaiveDate> {
    d.checked_add_months(Months::new(1))
}

fn local_date(today: i64, tz: chrono_tz::Tz) -> NaiveDate {
    tz.timestamp_opt(today, 0)
        .single()
        .map(|t| t.date_naive())
        .unwrap_or_else(|| {
            chrono::DateTime::from_timestamp(today, 0)
                .unwrap_or_default()
                .date_naive()
        })
}

/// Unix seconds at local midnight of `d` in `tz` (DST gaps move forward).
fn start_of(d: NaiveDate, tz: chrono_tz::Tz) -> i64 {
    let mut ndt = d.and_time(chrono::NaiveTime::MIN);
    for _ in 0..4 {
        if let Some(t) = tz.from_local_datetime(&ndt).earliest() {
            return t.timestamp();
        }
        ndt += Duration::hours(1);
    }
    ndt.and_utc().timestamp()
}

/// `YYYY`, `YYYY-MM`, `YYYY-MM-DD` -> `[start, end)` dates.
fn iso_period(tok: &str) -> Option<(NaiveDate, NaiveDate)> {
    let parts: Vec<&str> = tok.split('-').collect();
    let year = |s: &str| -> Option<i32> {
        (s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
            .filter(|y| (1900..=2200).contains(y))
    };
    let two = |s: &str| -> Option<u32> {
        (s.len() <= 2 && !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    match parts.as_slice() {
        [y] => {
            let y = year(y)?;
            Some((month_start(y, 1)?, month_start(y + 1, 1)?))
        }
        [y, m] => {
            let s = month_start(year(y)?, two(m)?)?;
            Some((s, next_month(s)?))
        }
        [y, m, d] => {
            let s = NaiveDate::from_ymd_opt(year(y)?, two(m)?, two(d)?)?;
            Some((s, s.succ_opt()?))
        }
        _ => None,
    }
}

/// Parse a CLI date argument: `YYYY`, `YYYY-MM`, `YYYY-MM-DD`, or relative
/// `90d`, `2w`, `6m`, `1y` (that long before `today`). Returns the start of
/// the period in unix seconds (local midnight in `tz`).
pub fn parse_date_arg(s: &str, today: i64, tz: chrono_tz::Tz) -> Option<i64> {
    let s = s.trim().to_lowercase();
    if let Some((start, _)) = iso_period(&s) {
        return Some(start_of(start, tz));
    }
    let (num, suffix) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: u32 = num.parse().ok()?;
    let u = match suffix {
        "d" => Unit::Day,
        "w" => Unit::Week,
        "m" => Unit::Month,
        "y" => Unit::Year,
        _ => return None,
    };
    Some(start_of(sub_units(local_date(today, tz), n, u)?, tz))
}

fn looks_like_date(tok: &str) -> bool {
    tok.chars().any(|c| c.is_ascii_digit())
}

impl RuleParser {
    /// A parser for one process; `tz` decides local dates.
    pub fn new(lexicon: Lexicon, tz: chrono_tz::Tz) -> Self {
        RuleParser { lexicon, tz }
    }

    /// The lexicon in use.
    pub fn lexicon(&self) -> &Lexicon {
        &self.lexicon
    }

    /// The timezone in use.
    pub fn tz(&self) -> chrono_tz::Tz {
        self.tz
    }

    /// Parse and also return warnings. Never fails.
    pub fn parse_detailed(&self, text: &str, today: i64) -> Parsed {
        let mut warnings = Vec::new();
        let normalized: String = text
            .nfkc()
            .map(|c| match c {
                '\u{2018}' | '\u{2019}' | '\u{02bc}' => '\'',
                '\u{201c}' | '\u{201d}' => '"',
                c => c,
            })
            .collect();
        let (rest, explicit) = self.extract_explicit(&normalized, today, &mut warnings);
        let mut toks = tokenize(&rest);
        let mut q = Query::default();
        let today_d = local_date(today, self.tz);

        self.date_pass(&mut toks, today_d, &mut q, &mut warnings);
        flag_pass(&mut toks, &mut q);
        self.entity_pass(&mut toks, &mut q);

        let text_toks: Vec<String> = toks
            .iter()
            .filter(|t| !t.used)
            .map(|t| t.norm.as_str())
            .filter(|n| n.chars().any(char::is_alphanumeric) && !STOPWORDS.contains(n))
            .map(|n| {
                TEXT_ALIASES
                    .iter()
                    .find(|(a, _)| *a == n)
                    .map_or_else(|| n.to_string(), |(_, c)| (*c).to_string())
            })
            .collect();
        q.text = (!text_toks.is_empty()).then(|| text_toks.join(" "));

        apply_explicit(&mut q, explicit);
        q.limit = q.limit.clamp(1, MAX_LIMIT);
        Parsed { query: q, warnings }
    }

    fn extract_explicit(
        &self,
        text: &str,
        today: i64,
        warnings: &mut Vec<String>,
    ) -> (String, Explicit) {
        let mut ex = Explicit::default();
        let mut rest = String::with_capacity(text.len());
        let mut last = 0;
        for cap in explicit_re().captures_iter(text) {
            let Some(m) = cap.get(0) else { continue };
            rest.push_str(&text[last..m.start()]);
            rest.push(' ');
            last = m.end();
            let key = cap.get(1).map_or("", |k| k.as_str()).to_lowercase();
            let raw = cap
                .get(2)
                .or_else(|| cap.get(3))
                .map_or("", |v| v.as_str())
                .trim();
            let val = raw.to_lowercase();
            let date = |w: &mut Vec<String>| -> Option<i64> {
                if raw.is_empty() {
                    return None;
                }
                let d = parse_date_arg(raw, today, self.tz);
                if d.is_none() {
                    w.push(format!("could not parse date: '{raw}'"));
                }
                d
            };
            match key.as_str() {
                "company" | "at" => push_unique(&mut ex.company, company_norm(&val)),
                "title" | "role" => push_unique(&mut ex.title, simple_norm(&val)),
                "location" | "in" => {
                    let v = simple_norm(&val);
                    push_unique(&mut ex.location, canonical_location(&v).unwrap_or(v));
                }
                "industry" => push_unique(&mut ex.industry, simple_norm(&val)),
                "tag" => push_unique(&mut ex.tags, val.trim().to_string()),
                "since" => ex.since = date(warnings).or(ex.since),
                "until" => ex.until = date(warnings).or(ex.until),
                "before" => ex.last_contact_before = date(warnings).or(ex.last_contact_before),
                "met" => ex.met = parse_bool(&val).or(ex.met),
                "replied" => ex.replied = parse_bool(&val).or(ex.replied),
                "bulk" => ex.include_bulk = parse_bool(&val).or(ex.include_bulk),
                "min-score" => {
                    if let Ok(v) = val.parse::<f32>() {
                        if v.is_finite() {
                            ex.min_score = Some(v.clamp(0.0, 1.0));
                        }
                    }
                }
                "limit" => {
                    if let Ok(v) = val.parse::<usize>() {
                        ex.limit = Some(v.clamp(1, MAX_LIMIT));
                    }
                }
                _ => {}
            }
        }
        rest.push_str(&text[last..]);

        let mut out = String::with_capacity(rest.len());
        let mut last = 0;
        for cap in hashtag_re().captures_iter(&rest) {
            let (Some(m), Some(tag)) = (cap.get(0), cap.get(1)) else {
                continue;
            };
            out.push_str(&rest[last..m.start()]);
            out.push(' ');
            last = m.end();
            push_unique(&mut ex.tags, tag.as_str().to_lowercase());
        }
        out.push_str(&rest[last..]);
        (out, ex)
    }

    // -- dates ---------------------------------------------------------------

    /// A date expression at `j`: `(tokens, start, end)`.
    fn date_expr(
        &self,
        toks: &[Tok],
        j: usize,
        today: NaiveDate,
    ) -> Option<(usize, NaiveDate, NaiveDate)> {
        let t = toks.get(j).filter(|t| !t.used)?;
        if let Some((s, e)) = iso_period(&t.norm) {
            return Some((1, s, e));
        }
        if let Some(&(_, m)) = MONTHS.iter().find(|(name, _)| *name == t.norm) {
            if let Some(y) = toks
                .get(j + 1)
                .filter(|n| !n.used)
                .and_then(|n| iso_period(&n.norm).filter(|_| n.norm.len() == 4))
                .map(|(s, _)| s.year())
            {
                let s = month_start(y, m)?;
                return Some((2, s, next_month(s)?));
            }
            let y = if m <= today.month() {
                today.year()
            } else {
                today.year() - 1
            };
            let s = month_start(y, m)?;
            return Some((1, s, next_month(s)?));
        }
        if let Some((s, e, n)) = self.period_at(toks, j, today) {
            return Some((n, s, e.unwrap_or(today.succ_opt()?)));
        }
        None
    }

    /// `today`, `yesterday`, `this|last week|month|year`.
    fn period_at(
        &self,
        toks: &[Tok],
        j: usize,
        today: NaiveDate,
    ) -> Option<(NaiveDate, Option<NaiveDate>, usize)> {
        let t = toks.get(j).filter(|t| !t.used)?;
        match t.norm.as_str() {
            "today" => return Some((today, today.succ_opt(), 1)),
            "yesterday" => return Some((today.pred_opt()?, Some(today), 1)),
            "this" | "last" => {}
            _ => return None,
        }
        let u = toks
            .get(j + 1)
            .filter(|n| !n.used)
            .and_then(|n| unit(&n.norm))?;
        let this_start = match u {
            Unit::Day => return None,
            Unit::Week => today.checked_sub_signed(Duration::days(i64::from(
                today.weekday().num_days_from_monday(),
            )))?,
            Unit::Month => month_start(today.year(), today.month())?,
            Unit::Year => month_start(today.year(), 1)?,
        };
        if t.norm == "this" {
            return Some((this_start, None, 2));
        }
        let prev = match u {
            Unit::Week => this_start.checked_sub_signed(Duration::weeks(1))?,
            Unit::Month => this_start.checked_sub_months(Months::new(1))?,
            _ => month_start(today.year() - 1, 1)?,
        };
        Some((prev, Some(this_start), 2))
    }

    /// `[in|over|within|during|for] [the] last|past [N] unit` -> since.
    fn relative_at(&self, toks: &[Tok], i: usize, today: NaiveDate) -> Option<(usize, NaiveDate)> {
        let word = |k: usize| toks.get(k).filter(|t| !t.used).map(|t| t.norm.as_str());
        let mut k = i;
        let mut prefixed = false;
        if matches!(word(k), Some("in" | "over" | "within" | "during" | "for")) {
            k += 1;
            prefixed = true;
        }
        if word(k) == Some("the") {
            k += 1;
            prefixed = true;
        }
        let lp = word(k)?;
        if lp != "last" && lp != "past" {
            return None;
        }
        k += 1;
        if let Some(n) = word(k).and_then(number) {
            let u = word(k + 1).and_then(unit)?;
            return Some((k + 2 - i, sub_units(today, n, u)?));
        }
        let u = word(k).and_then(unit)?;
        if !prefixed && lp == "last" {
            return None; // "last week" is the calendar period
        }
        Some((k + 1 - i, sub_units(today, 1, u)?))
    }

    fn date_at(
        &self,
        toks: &[Tok],
        i: usize,
        today: NaiveDate,
        warnings: &mut Vec<String>,
    ) -> Option<(usize, DateEffect)> {
        let tz = self.tz;
        for p in NOT_SINCE {
            let words: Vec<&str> = p.split(' ').collect();
            if phrase_at(toks, i, &words) {
                if let Some((n, s, _)) = self.date_expr(toks, i + words.len(), today) {
                    return Some((
                        words.len() + n,
                        DateEffect::LastContactBefore(start_of(s, tz)),
                    ));
                }
            }
        }
        let t = &toks[i];
        match t.norm.as_str() {
            "since" | "after" | "before" | "until" | "till" => {
                if let Some((n, s, _)) = self.date_expr(toks, i + 1, today) {
                    let at = Some(start_of(s, tz));
                    let eff = if matches!(t.norm.as_str(), "since" | "after") {
                        DateEffect::Window(at, None)
                    } else {
                        DateEffect::Window(None, at)
                    };
                    return Some((n + 1, eff));
                }
                if let Some(next) = toks.get(i + 1).filter(|n| !n.used) {
                    if looks_like_date(&next.norm) {
                        warnings.push(format!("could not parse date: '{}'", next.raw));
                    }
                }
                return None;
            }
            _ => {}
        }
        if let Some((n, since)) = self.relative_at(toks, i, today) {
            return Some((n, DateEffect::Window(Some(start_of(since, tz)), None)));
        }
        if matches!(t.norm.as_str(), "in" | "during") {
            let next = toks.get(i + 1).filter(|n| !n.used)?;
            let is_calendar = iso_period(&next.norm).is_some()
                || MONTHS.iter().any(|(m, _)| *m == next.norm && *m != "may")
                || (next.norm == "may"
                    && toks
                        .get(i + 2)
                        .is_some_and(|y| iso_period(&y.norm).is_some()));
            if is_calendar {
                let (n, s, e) = self.date_expr(toks, i + 1, today)?;
                return Some((
                    n + 1,
                    DateEffect::Window(Some(start_of(s, tz)), Some(start_of(e, tz))),
                ));
            }
            return None;
        }
        match t.norm.as_str() {
            "recently" | "lately" => {
                let s = today.checked_sub_signed(Duration::days(90))?;
                Some((1, DateEffect::Window(Some(start_of(s, tz)), None)))
            }
            _ => {
                let (s, e, n) = self.period_at(toks, i, today)?;
                Some((
                    n,
                    DateEffect::Window(Some(start_of(s, tz)), e.map(|e| start_of(e, tz))),
                ))
            }
        }
    }

    fn date_pass(
        &self,
        toks: &mut [Tok],
        today: NaiveDate,
        q: &mut Query,
        warnings: &mut Vec<String>,
    ) {
        let mut i = 0;
        while i < toks.len() {
            if toks[i].used {
                i += 1;
                continue;
            }
            match self.date_at(toks, i, today, warnings) {
                Some((n, eff)) => {
                    consume(toks, i, n);
                    match eff {
                        DateEffect::Window(s, u) => {
                            if s.is_some() {
                                q.since = s;
                            }
                            if u.is_some() {
                                q.until = u;
                            }
                        }
                        DateEffect::LastContactBefore(t) => q.last_contact_before = Some(t),
                    }
                    i += n;
                }
                None => i += 1,
            }
        }
    }

    // -- entities ------------------------------------------------------------

    /// Longest location (alias table or lexicon) of up to 4 tokens at `j`.
    fn location_at(&self, toks: &[Tok], j: usize) -> Option<(usize, String)> {
        for len in (1..=4).rev() {
            let Some(phrase) = joined(toks, j, len) else {
                continue;
            };
            if let Some(c) = canonical_location(&phrase) {
                return Some((len, c));
            }
            if self.lexicon.locations.contains(&phrase) {
                return Some((len, phrase));
            }
        }
        None
    }

    /// Longest lexicon company of up to 4 tokens at `j`.
    fn lexicon_company_at(&self, toks: &[Tok], j: usize) -> Option<(usize, String)> {
        for len in (1..=4).rev() {
            let Some(phrase) = joined(toks, j, len) else {
                continue;
            };
            let key = company_norm(&phrase);
            if self.lexicon.companies.contains(&key) {
                return Some((len, key));
            }
        }
        None
    }

    fn company_after(&self, toks: &[Tok], j: usize, allow_caps: bool) -> Option<(usize, String)> {
        if let Some(hit) = self.lexicon_company_at(toks, j) {
            return Some(hit);
        }
        if !allow_caps {
            return None;
        }
        let mut words = Vec::new();
        for t in toks.iter().skip(j).take(3) {
            let capitalized = t.raw.chars().next().is_some_and(char::is_uppercase);
            if t.used
                || !capitalized
                || STOPWORDS.contains(&t.norm.as_str())
                || COMPANY_TRIGGERS.contains(&t.norm.as_str())
            {
                break;
            }
            words.push(t.norm.clone());
        }
        if words.is_empty() {
            return None;
        }
        let key = company_norm(&words.join(" "));
        (!key.is_empty()).then_some((words.len(), key))
    }

    fn entity_pass(&self, toks: &mut [Tok], q: &mut Query) {
        // Location triggers first, so `in sf` is a location and `at sf` is not.
        let mut i = 0;
        while i < toks.len() {
            let mut advanced = false;
            for trig in LOCATION_TRIGGERS {
                let words: Vec<&str> = trig.split(' ').collect();
                if !phrase_at(toks, i, &words) {
                    continue;
                }
                if let Some((n, loc)) = self.location_at(toks, i + words.len()) {
                    consume(toks, i, words.len() + n);
                    push_unique(&mut q.location, loc);
                    i += words.len() + n;
                    advanced = true;
                    break;
                }
            }
            if !advanced {
                i += 1;
            }
        }

        // Company triggers.
        let mut i = 0;
        while i < toks.len() {
            if toks[i].used {
                i += 1;
                continue;
            }
            let mut advanced = false;
            for trig in COMPANY_TRIGGERS {
                let words: Vec<&str> = trig.split(' ').collect();
                if !phrase_at(toks, i, &words) {
                    continue;
                }
                let allow_caps = *trig != "from";
                if let Some((n, c)) = self.company_after(toks, i + words.len(), allow_caps) {
                    consume(toks, i, words.len() + n);
                    push_unique(&mut q.company, c);
                    i += words.len() + n;
                    advanced = true;
                    break;
                }
            }
            if !advanced {
                // `@stripe`
                let t = &toks[i];
                if let Some(rest) = t
                    .norm
                    .strip_prefix('@')
                    .filter(|r| !r.is_empty() && !r.contains('@'))
                {
                    let key = company_norm(rest);
                    if !key.is_empty() {
                        push_unique(&mut q.company, key);
                        toks[i].used = true;
                    }
                }
                i += 1;
            }
        }

        table_pass(toks, TITLES, &mut q.title);
        table_pass(toks, INDUSTRIES, &mut q.industry);

        // Tags: a token equal to an existing tag.
        for t in toks.iter_mut().filter(|t| !t.used) {
            if self.lexicon.tags.contains(&t.norm) {
                push_unique(&mut q.tags, t.norm.clone());
                t.used = true;
            }
        }

        // Bare company mentions.
        let mut i = 0;
        while i < toks.len() {
            if let Some((n, c)) = self.lexicon_company_at(toks, i) {
                let plain = c.chars().count() >= 4
                    && !COMMON_WORDS.contains(&c.as_str())
                    && !STOPWORDS.contains(&c.as_str());
                if plain {
                    consume(toks, i, n);
                    push_unique(&mut q.company, c);
                    i += n;
                    continue;
                }
            }
            i += 1;
        }
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
        Ok(self.parse_detailed(text, today).query)
    }
}

fn tokenize(text: &str) -> Vec<Tok> {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() || "@.:-'\"#_+/&".contains(c) {
                c
            } else {
                ' '
            }
        })
        .collect();
    cleaned
        .split_whitespace()
        .filter_map(|piece| {
            let raw =
                piece.trim_matches(|c: char| matches!(c, '.' | ':' | '\'' | '"' | '-' | '/' | '+'));
            if raw.is_empty() {
                return None;
            }
            Some(Tok {
                raw: raw.to_string(),
                norm: raw.to_lowercase(),
                used: false,
            })
        })
        .collect()
}

fn phrase_at(toks: &[Tok], i: usize, words: &[&str]) -> bool {
    words
        .iter()
        .enumerate()
        .all(|(k, w)| toks.get(i + k).is_some_and(|t| !t.used && t.norm == *w))
}

fn joined(toks: &[Tok], i: usize, len: usize) -> Option<String> {
    let slice = toks.get(i..i + len)?;
    if slice.iter().any(|t| t.used) {
        return None;
    }
    Some(
        slice
            .iter()
            .map(|t| t.norm.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn consume(toks: &mut [Tok], i: usize, n: usize) {
    for t in toks.iter_mut().skip(i).take(n) {
        t.used = true;
    }
}

fn table_pass(toks: &mut [Tok], table: &[(&str, &str)], out: &mut Vec<String>) {
    let mut i = 0;
    'outer: while i < toks.len() {
        for len in (1..=4).rev() {
            let Some(phrase) = joined(toks, i, len) else {
                continue;
            };
            if let Some((_, c)) = table.iter().find(|(p, _)| *p == phrase) {
                consume(toks, i, len);
                push_unique(out, *c);
                i += len;
                continue 'outer;
            }
        }
        i += 1;
    }
}

fn flag_pass(toks: &mut [Tok], q: &mut Query) {
    let mut flags: Vec<(Vec<&str>, Flag)> = FLAGS
        .iter()
        .map(|(p, f)| (p.split(' ').collect::<Vec<_>>(), *f))
        .collect();
    flags.sort_by_key(|(w, _)| std::cmp::Reverse(w.len()));
    let mut i = 0;
    'outer: while i < toks.len() {
        if toks[i].used {
            i += 1;
            continue;
        }
        // Limits: `top N`, `first N`, `N people`.
        let word = |k: usize| toks.get(k).filter(|t| !t.used).map(|t| t.norm.as_str());
        if matches!(word(i), Some("top" | "first")) {
            if let Some(n) = word(i + 1).and_then(number) {
                q.limit = (n as usize).min(MAX_LIMIT);
                consume(toks, i, 2);
                i += 2;
                continue;
            }
        }
        if let Some(n) = word(i).and_then(number) {
            if matches!(
                word(i + 1),
                Some("people" | "persons" | "contacts" | "folks")
            ) {
                q.limit = (n as usize).min(MAX_LIMIT);
                consume(toks, i, 2);
                i += 2;
                continue;
            }
        }
        for (words, flag) in &flags {
            if phrase_at(toks, i, words) {
                match *flag {
                    Flag::Met(v) => q.met_in_person = Some(v),
                    Flag::Replied(v) => q.replied = Some(v),
                    Flag::LostTouch => q.lost_touch = Some(true),
                    Flag::Close => q.min_score = Some(CLOSE_MIN_SCORE),
                }
                consume(toks, i, words.len());
                i += words.len();
                continue 'outer;
            }
        }
        i += 1;
    }
}

fn apply_explicit(q: &mut Query, ex: Explicit) {
    if !ex.company.is_empty() {
        q.company = ex.company;
    }
    if !ex.title.is_empty() {
        q.title = ex.title;
    }
    if !ex.location.is_empty() {
        q.location = ex.location;
    }
    if !ex.industry.is_empty() {
        q.industry = ex.industry;
    }
    if !ex.tags.is_empty() {
        for t in ex.tags {
            push_unique(&mut q.tags, t);
        }
    }
    if ex.since.is_some() {
        q.since = ex.since;
    }
    if ex.until.is_some() {
        q.until = ex.until;
    }
    if ex.last_contact_before.is_some() {
        q.last_contact_before = ex.last_contact_before;
    }
    if ex.met.is_some() {
        q.met_in_person = ex.met;
    }
    if ex.replied.is_some() {
        q.replied = ex.replied;
    }
    if ex.lost_touch.is_some() {
        q.lost_touch = ex.lost_touch;
    }
    if ex.min_score.is_some() {
        q.min_score = ex.min_score;
    }
    if let Some(l) = ex.limit {
        q.limit = l;
    }
    if let Some(b) = ex.include_bulk {
        q.include_bulk = b;
    }
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod tests;
