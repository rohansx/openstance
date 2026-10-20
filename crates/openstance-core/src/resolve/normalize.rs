//! Storage normalization (in the Sink, versioned by
//! `meta.normalizer_version`) and match keys (recomputed on every run).
//! Spec: `docs/tech-spec/entity-resolution.md` section 2 (P1-CORE-06).
//!
//! Owned by the "resolve" stream. Every function is pure, never panics on
//! arbitrary UTF-8, and is idempotent (`f(f(x)) == f(x)`).

use openstance_import::HandleKind;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Bumped when storage normalization changes (requires re-import).
pub const NORMALIZER_VERSION: u32 = 1;

/// Context for storage normalization.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormalizeContext {
    /// ISO 3166 alpha-2 region for phones without `+`.
    pub default_region: Option<String>,
    /// Importer kind, used as the `name_local` scope (`"vcf|jane doe"`).
    pub importer_kind: String,
}

/// Normalize a raw handle into `identity.value`; `None` = invalid (dropped).
pub fn normalize_handle(kind: HandleKind, raw: &str, ctx: &NormalizeContext) -> Option<String> {
    match kind {
        HandleKind::Email => normalize_email(raw),
        HandleKind::Phone => normalize_phone(raw, ctx.default_region.as_deref()),
        HandleKind::LinkedinUrl => normalize_linkedin(raw),
        HandleKind::XHandle => normalize_x_handle(raw),
        HandleKind::XUserId => {
            let t = raw.trim();
            (!t.is_empty() && t.len() <= 20 && t.bytes().all(|b| b.is_ascii_digit()))
                .then(|| t.to_string())
        }
        HandleKind::IgHandle => normalize_ig_handle(raw),
        HandleKind::NameCompany => {
            let (name, company) = raw.split_once('|')?;
            let n = name_key(name);
            let c = company_key(company);
            (n.split_whitespace().count() >= 2 && !c.is_empty()).then(|| format!("{n}|{c}"))
        }
        HandleKind::NameLocal => {
            let (scope, name) = match raw.split_once('|') {
                Some((s, n)) if is_scope(s) => (s.to_string(), n),
                _ => (scope_of(&ctx.importer_kind), raw),
            };
            let n = name_key(name);
            (!n.is_empty() && !scope.is_empty()).then(|| format!("{scope}|{n}"))
        }
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> &'a str {
    match s.get(..prefix.len()) {
        Some(head) if head.eq_ignore_ascii_case(prefix) => &s[prefix.len()..],
        _ => s,
    }
}

fn is_scope(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'_' || b.is_ascii_digit())
}

fn scope_of(kind: &str) -> String {
    kind.trim().to_ascii_lowercase()
}

/// Email: trim, strip `<>`/`mailto:`, NFKC, lowercase, IDNA domain, strip a
/// trailing dot. Rejects no `@`, empty local part, dotless domain, > 254
/// chars, `undisclosed-recipients`. Gmail aliasing is NOT applied here.
pub fn normalize_email(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    s = s.strip_prefix('<').unwrap_or(s);
    s = s.strip_suffix('>').unwrap_or(s);
    s = s.trim();
    s = strip_prefix_ci(s, "mailto:");
    let s: String = s.trim().nfkc().collect::<String>().to_lowercase();
    if s.contains("undisclosed-recipients")
        || s.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    let mut parts = s.split('@');
    let local = parts.next()?;
    let domain = parts.next()?;
    if parts.next().is_some() || local.is_empty() {
        return None;
    }
    let domain = domain.strip_suffix('.').unwrap_or(domain);
    if domain.is_empty() || domain.split('.').any(str::is_empty) {
        return None;
    }
    let domain = idna::domain_to_ascii(domain).ok()?.to_lowercase();
    if !domain.contains('.') || domain.split('.').any(str::is_empty) {
        return None;
    }
    let out = format!("{local}@{domain}");
    (out.len() <= 254).then_some(out)
}

/// Phone to E.164 with the `phonenumber` crate; extensions dropped.
pub fn normalize_phone(raw: &str, default_region: Option<&str>) -> Option<String> {
    let mut s = raw.trim();
    s = strip_prefix_ci(s, "tel:");
    // Drop URI parameters (`;ext=12`) and written extensions (`x12`, `ext. 12`).
    let s = s.split(';').next().unwrap_or("");
    let lower = s.to_ascii_lowercase();
    let cut = ["ext", "x", "#"]
        .iter()
        .filter_map(|m| lower.find(m))
        .min()
        .unwrap_or(lower.len());
    let s = &s[..cut];
    let cleaned: String = s
        .chars()
        .filter(|c| !matches!(c, ' ' | '.' | '-' | '(' | ')' | '/' | '\u{a0}'))
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let (plus, digits) = match cleaned.strip_prefix('+') {
        Some(d) => (true, d),
        None => (false, cleaned.as_str()),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.len() > 17 {
        return None;
    }
    let region = if plus {
        None
    } else {
        let r = default_region?.trim().to_ascii_uppercase();
        Some(r.parse::<phonenumber::country::Id>().ok()?)
    };
    let text = if plus {
        format!("+{digits}")
    } else {
        digits.to_string()
    };
    let n = phonenumber::parse(region, &text).ok()?;
    if !phonenumber::is_valid(&n) {
        return None;
    }
    Some(
        phonenumber::format(&n)
            .mode(phonenumber::Mode::E164)
            .to_string(),
    )
}

/// LinkedIn URL to the lowercase slug (`/in/<slug>`; legacy `/pub/...` ->
/// `pub:...`). Company / school / post URLs are rejected.
pub fn normalize_linkedin(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_lowercase();
    // Already-normalized values: a bare slug or a legacy `pub:` value.
    if let Some(rest) = lower.strip_prefix("pub:") {
        return valid_pub(rest).then(|| format!("pub:{rest}"));
    }
    if !s.contains('/') && !s.contains('.') && !s.contains(':') {
        return valid_slug(&lower).then_some(lower);
    }
    let with_scheme = if lower.starts_with("http://") || lower.starts_with("https://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    let url = url::Url::parse(&with_scheme).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    if host != "linkedin.com" && !host.ends_with(".linkedin.com") {
        return None;
    }
    let segs: Vec<String> = url
        .path_segments()?
        .filter(|p| !p.is_empty())
        .map(|p| percent_decode(p).to_lowercase())
        .collect();
    match segs.first().map(String::as_str) {
        Some("in") => {
            let slug = segs.get(1)?.clone();
            valid_slug(&slug).then_some(slug)
        }
        Some("pub") if segs.len() >= 2 => {
            let rest = segs[1..].join("/");
            valid_pub(&rest).then(|| format!("pub:{rest}"))
        }
        _ => None,
    }
}

fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.chars().count() <= 100
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '%'))
        && !s.chars().any(char::is_uppercase)
}

fn valid_pub(s: &str) -> bool {
    !s.is_empty() && s.split('/').all(valid_slug)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if let Some(Ok(v)) = s.get(i + 1..i + 3).map(|h| u8::from_str_radix(h, 16)) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Strip `@`, a URL prefix for one of `hosts`, query and fragment, trailing
/// `/`, and lowercase. Returns the first path segment.
fn social_handle(raw: &str, hosts: &[&str]) -> Option<String> {
    let mut s = raw.trim().to_lowercase();
    for scheme in ["https://", "http://"] {
        if let Some(r) = s.strip_prefix(scheme) {
            s = r.to_string();
        }
    }
    for p in ["www.", "mobile.", "m."] {
        if let Some(r) = s.strip_prefix(p) {
            s = r.to_string();
        }
    }
    for h in hosts {
        if let Some(r) = s.strip_prefix(&format!("{h}/")) {
            s = r.to_string();
            break;
        }
    }
    let s = s.split(['?', '#']).next().unwrap_or("");
    let s = s.trim_start_matches('@');
    let first = s.split('/').find(|p| !p.is_empty())?;
    Some(first.to_string())
}

/// X handle without `@` or URL prefix, `^[a-z0-9_]{1,15}$`.
pub fn normalize_x_handle(raw: &str) -> Option<String> {
    let h = social_handle(raw, &["x.com", "twitter.com"])?;
    let ok = (1..=15).contains(&h.len())
        && h.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    (ok && !matches!(h.as_str(), "home" | "intent" | "i" | "search" | "share")).then_some(h)
}

/// Instagram handle, `^[a-z0-9._]{1,30}$`.
pub fn normalize_ig_handle(raw: &str) -> Option<String> {
    let h = social_handle(raw, &["instagram.com", "instagr.am"])?;
    let ok = (1..=30).contains(&h.len())
        && h.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.');
    (ok && !matches!(
        h.as_str(),
        "p" | "reel" | "reels" | "explore" | "stories" | "tv"
    ))
    .then_some(h)
}

const HONORIFICS: &[&str] = &[
    "mr", "mrs", "ms", "miss", "mx", "dr", "prof", "sir", "dame", "lord", "lady", "rev", "fr",
    "shri", "sri", "smt", "kumari", "herr", "frau", "mme", "mlle", "sr", "sra",
];
const SUFFIXES: &[&str] = &[
    "jr", "sr", "ii", "iii", "iv", "phd", "md", "mba", "cpa", "esq", "dds", "pe", "cfa", "obe",
    "mbe",
];
const QUOTES: &[char] = &[
    '"', '\'', '\u{201c}', '\u{201d}', '\u{2018}', '\u{2019}', '`', '\u{b4}',
];

/// The 11-step `name_key` (NFKC, strip emoji/quotes/brackets/"via" suffixes,
/// swap `Last, First`, lowercase, separators to spaces, drop honorifics and
/// suffixes, fold diacritics, collapse whitespace).
pub fn name_key(name: &str) -> String {
    // Applied to a fixpoint so the key is idempotent by construction.
    let mut cur = name_key_once(name);
    for _ in 0..4 {
        let next = name_key_once(&cur);
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

fn name_key_once(name: &str) -> String {
    // 1. NFKC.
    let s: String = name.nfkc().collect();
    // 2. Remove emoji, symbols and quotes; keep letters, digits, marks,
    //    whitespace and the punctuation later steps interpret.
    let s: String = s
        .chars()
        .filter(|&c| {
            if QUOTES.contains(&c) || is_emoji_joiner(c) {
                return false;
            }
            c.is_alphanumeric()
                || c.is_whitespace()
                || is_combining_mark(c)
                || "-_.,()[]{}|@".contains(c)
        })
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    // 3. Remove bracketed segments.
    let s = strip_brackets(&s);
    // 4. Remove trailing " via ...", " | ...", " - ..." when >= 2 tokens remain.
    let s = strip_separator_suffix(&s);
    // 5. `Last, First` -> `First Last`, else drop commas.
    let s = swap_comma(&s);
    // 6. Lowercase.
    let s = s.to_lowercase();
    // 7. Separators to spaces.
    let s: String = s
        .chars()
        .map(|c| {
            if matches!(c, '-' | '_' | '.' | '|' | ',') {
                ' '
            } else {
                c
            }
        })
        .collect();
    // 10 (before 8/9 so a folded token is still recognized). Fold diacritics.
    let s = fold_diacritics(&s);
    // 8, 9. Drop honorifics at the start and suffixes at the end.
    let mut toks: Vec<&str> = s.split_whitespace().collect();
    while toks.first().is_some_and(|t| HONORIFICS.contains(t)) {
        toks.remove(0);
    }
    while toks.last().is_some_and(|t| SUFFIXES.contains(t)) {
        toks.pop();
    }
    // 11. Collapse whitespace.
    toks.join(" ")
}

fn is_emoji_joiner(c: char) -> bool {
    matches!(c, '\u{200d}' | '\u{20e3}' | '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}'
        | '\u{1f3fb}'..='\u{1f3ff}' | '\u{e0020}'..='\u{e007f}')
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn token_count(s: &str) -> usize {
    s.split_whitespace()
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .count()
}

fn strip_separator_suffix(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut cut: Option<usize> = None;
    for sep in [" via ", " | ", " - "] {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(sep).map(|p| p + from) {
            // `lower` and `s` may differ in byte length for exotic case
            // mappings; only cut on a boundary valid in `s`.
            if s.is_char_boundary(pos) && token_count(&s[..pos]) >= 2 {
                cut = Some(cut.map_or(pos, |c| c.min(pos)));
                break;
            }
            from = pos + sep.len();
            if from >= lower.len() {
                break;
            }
        }
    }
    match cut {
        Some(p) => s[..p].to_string(),
        None => s.to_string(),
    }
}

fn swap_comma(s: &str) -> String {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() == 2 {
        let after = parts[1].trim();
        let after_key: String = after
            .chars()
            .filter(|c| *c != '.')
            .collect::<String>()
            .to_lowercase();
        let is_suffix = after_key.split_whitespace().all(|t| SUFFIXES.contains(&t));
        if !after.is_empty() && !is_suffix && token_count(parts[0]) >= 1 {
            return format!("{} {}", after, parts[0].trim());
        }
    }
    s.replace(',', " ")
}

fn fold_diacritics(s: &str) -> String {
    s.nfd()
        .filter(|c| !('\u{0300}'..='\u{036f}').contains(c))
        .nfc()
        .collect()
}

fn is_spaceless_script(c: char) -> bool {
    matches!(c,
        '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}' | '\u{20000}'..='\u{2ebef}'
        | '\u{f900}'..='\u{faff}' // Han
        | '\u{3040}'..='\u{309f}' // Hiragana
        | '\u{30a0}'..='\u{30ff}' | '\u{31f0}'..='\u{31ff}' // Katakana
        | '\u{ac00}'..='\u{d7af}' | '\u{1100}'..='\u{11ff}' | '\u{3130}'..='\u{318f}' // Hangul
        | '\u{0e00}'..='\u{0e7f}' // Thai
    )
}

/// A name key usable by rules 3-5: >= 2 tokens (or >= 2 chars of a
/// space-less script), no digits, no `@`.
pub fn is_matchable(key: &str) -> bool {
    if key.is_empty() || key.contains('@') || key.chars().any(char::is_numeric) {
        return false;
    }
    let toks: Vec<&str> = key.split_whitespace().collect();
    match toks.as_slice() {
        [] => false,
        [one] => one.chars().all(is_spaceless_script) && one.chars().count() >= 2,
        _ => true,
    }
}

const LEGAL_FORMS: &[&str] = &[
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
    "as",
    "kk",
];

/// `company_key`: NFKC, lowercase, drop punctuation, leading `the`, trailing
/// legal forms, collapse whitespace (before the domain/alias map).
pub fn company_key(name: &str) -> String {
    let s: String = name.nfkc().collect::<String>().to_lowercase();
    let s: String = s
        .chars()
        .filter(|c| !QUOTES.contains(c))
        .map(|c| {
            if c.is_alphanumeric() || is_combining_mark(c) {
                c
            } else {
                ' '
            }
        })
        .collect();
    let toks: Vec<&str> = s.split_whitespace().collect();
    let mut t: &[&str] = &toks;
    if t.len() > 1 && t[0] == "the" {
        t = &t[1..];
    }
    loop {
        let n = t.len();
        if n >= 4 && t[n - 3..] == ["l", "l", "c"] {
            t = &t[..n - 3];
        } else if n >= 2 && LEGAL_FORMS.contains(&t[n - 1]) {
            t = &t[..n - 1];
        } else {
            break;
        }
    }
    t.join(" ")
}

/// Rule 2 alias key: on `gmail_domains` drop dots and map to the first
/// (canonical) domain; on any domain strip `+tag` (position > 0).
pub fn alias_key(email: &str, gmail_domains: &[String]) -> String {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return email.to_string();
    };
    let mut local = match local.find('+') {
        Some(p) if p > 0 => local[..p].to_string(),
        _ => local.to_string(),
    };
    let mut domain = domain.to_string();
    if gmail_domains
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&domain))
    {
        local.retain(|c| c != '.');
        domain = gmail_domains[0].to_ascii_lowercase();
    }
    format!("{local}@{domain}")
}

/// Display fallback from an email local part (`jane.doe` -> `Jane Doe`).
/// Never used as a match key.
pub fn prettify_local_part(email: &str) -> String {
    let local = email.split('@').next().unwrap_or("");
    let local = match local.find('+') {
        Some(p) if p > 0 => &local[..p],
        _ => local,
    };
    local
        .split(['.', '_', '-'])
        .filter(|t| !t.is_empty())
        .map(|t| {
            let mut cs = t.chars();
            match cs.next() {
                Some(f) => f.to_uppercase().chain(cs).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(region: Option<&str>, kind: &str) -> NormalizeContext {
        NormalizeContext {
            default_region: region.map(str::to_string),
            importer_kind: kind.to_string(),
        }
    }

    /// (kind, raw, expected) storage-normalization table.
    #[allow(clippy::type_complexity)]
    const HANDLE_CASES: &[(HandleKind, &str, Option<&str>)] = &[
        // email
        (
            HandleKind::Email,
            "Jane.Doe@Example.COM",
            Some("jane.doe@example.com"),
        ),
        (
            HandleKind::Email,
            "  <jane@example.com> ",
            Some("jane@example.com"),
        ),
        (
            HandleKind::Email,
            "mailto:Jane@Example.com",
            Some("jane@example.com"),
        ),
        (
            HandleKind::Email,
            "MAILTO:jane@example.com",
            Some("jane@example.com"),
        ),
        (
            HandleKind::Email,
            "jane@example.com.",
            Some("jane@example.com"),
        ),
        (HandleKind::Email, "ab+x@gmail.com", Some("ab+x@gmail.com")),
        (
            HandleKind::Email,
            "a.b@googlemail.com",
            Some("a.b@googlemail.com"),
        ),
        (
            HandleKind::Email,
            "jane@bücher.example",
            Some("jane@xn--bcher-kva.example"),
        ),
        (
            HandleKind::Email,
            "ｊａｎｅ@example.com",
            Some("jane@example.com"),
        ),
        (HandleKind::Email, "no-at-sign.example.com", None),
        (HandleKind::Email, "@example.com", None),
        (HandleKind::Email, "jane@localhost", None),
        (
            HandleKind::Email,
            "undisclosed-recipients@example.com",
            None,
        ),
        (HandleKind::Email, "undisclosed-recipients:;", None),
        (HandleKind::Email, "a@b@example.com", None),
        (HandleKind::Email, "jane doe@example.com", None),
        (HandleKind::Email, "", None),
        // phone
        (HandleKind::Phone, "+1 (201) 555-0142", Some("+12015550142")),
        (HandleKind::Phone, "+1.201.555.0177", Some("+12015550177")),
        (
            HandleKind::Phone,
            "tel:+1-201-555-0188",
            Some("+12015550188"),
        ),
        (
            HandleKind::Phone,
            "+1 415 555 0123 x45",
            Some("+14155550123"),
        ),
        (
            HandleKind::Phone,
            "+1 415 555 0123 ext. 45",
            Some("+14155550123"),
        ),
        (HandleKind::Phone, "+91 98765 43210", Some("+919876543210")),
        (HandleKind::Phone, "+44 20 7183 8750", Some("+442071838750")),
        (HandleKind::Phone, "201 555 0142", None), // no + and no default region
        (HandleKind::Phone, "+1 123", None),
        (HandleKind::Phone, "not a phone", None),
        // linkedin
        (
            HandleKind::LinkedinUrl,
            "https://www.linkedin.com/in/Jane-Doe-0a1b2c/",
            Some("jane-doe-0a1b2c"),
        ),
        (
            HandleKind::LinkedinUrl,
            "linkedin.com/in/jane-doe",
            Some("jane-doe"),
        ),
        (
            HandleKind::LinkedinUrl,
            "http://uk.linkedin.com/in/jane-doe/en?trk=x#top",
            Some("jane-doe"),
        ),
        (
            HandleKind::LinkedinUrl,
            "https://www.linkedin.com/in/jos%C3%A9-garc%C3%ADa",
            Some("josé-garcía"),
        ),
        (
            HandleKind::LinkedinUrl,
            "https://www.linkedin.com/pub/jane-doe/1a/2b/3c",
            Some("pub:jane-doe/1a/2b/3c"),
        ),
        (
            HandleKind::LinkedinUrl,
            "https://www.linkedin.com/company/stripe",
            None,
        ),
        (
            HandleKind::LinkedinUrl,
            "https://www.linkedin.com/school/mit/",
            None,
        ),
        (
            HandleKind::LinkedinUrl,
            "https://www.linkedin.com/posts/jane_hello-123",
            None,
        ),
        (
            HandleKind::LinkedinUrl,
            "https://evil.example/in/jane-doe",
            None,
        ),
        (
            HandleKind::LinkedinUrl,
            "https://notlinkedin.com/in/jane",
            None,
        ),
        // x
        (HandleKind::XHandle, "@JaneDoe", Some("janedoe")),
        (
            HandleKind::XHandle,
            "https://x.com/jane_doe?s=20",
            Some("jane_doe"),
        ),
        (
            HandleKind::XHandle,
            "https://twitter.com/JaneDoe/",
            Some("janedoe"),
        ),
        (HandleKind::XHandle, "https://x.com/home", None),
        (HandleKind::XHandle, "this_handle_is_way_too_long", None),
        (HandleKind::XUserId, " 123456789 ", Some("123456789")),
        (HandleKind::XUserId, "12ab", None),
        // instagram
        (HandleKind::IgHandle, "@Jane.Doe", Some("jane.doe")),
        (
            HandleKind::IgHandle,
            "https://www.instagram.com/jane.doe/",
            Some("jane.doe"),
        ),
        (HandleKind::IgHandle, "https://instagram.com/p/", None),
        (HandleKind::IgHandle, "explore", None),
        // name_company / name_local
        (
            HandleKind::NameCompany,
            "Jane Doe|Stripe, Inc.",
            Some("jane doe|stripe"),
        ),
        (HandleKind::NameCompany, "Jane|Stripe", None),
        (HandleKind::NameLocal, "Mom", Some("vcf|mom")),
        (HandleKind::NameLocal, "vcf|Jane Doe", Some("vcf|jane doe")),
        (HandleKind::NameLocal, "  ", None),
    ];

    #[test]
    fn storage_normalization_table() {
        let c = ctx(None, "vcf");
        for (kind, raw, want) in HANDLE_CASES {
            let got = normalize_handle(*kind, raw, &c);
            assert_eq!(got.as_deref(), *want, "{kind} {raw:?}");
            if let Some(v) = &got {
                // Idempotent: normalizing the stored value is a no-op.
                assert_eq!(
                    normalize_handle(*kind, v, &c).as_deref(),
                    Some(v.as_str()),
                    "idempotence of {kind} {raw:?}"
                );
            }
        }
        assert!(HANDLE_CASES.len() >= 40);
    }

    #[test]
    fn phone_uses_default_region_without_plus() {
        assert_eq!(
            normalize_phone("(201) 555-0142", Some("US")).as_deref(),
            Some("+12015550142")
        );
        assert_eq!(
            normalize_phone("098765 43210", Some("IN")).as_deref(),
            Some("+919876543210")
        );
        assert_eq!(normalize_phone("(201) 555-0142", Some("ZZ")), None);
    }

    const NAME_CASES: &[(&str, &str)] = &[
        ("Jane Doe", "jane doe"),
        ("  JANE   DOE ", "jane doe"),
        ("Doe, Jane", "jane doe"),
        ("Dr. Jane Doe PhD", "jane doe"),
        ("Jane Doe 🚀", "jane doe"),
        ("Jane Doe (Stripe)", "jane doe"),
        ("Jane Doe [EXT]", "jane doe"),
        ("Jane Doe | Stripe", "jane doe"),
        ("Jane Doe via LinkedIn", "jane doe"),
        ("Jane Doe - Product", "jane doe"),
        ("Jean-Luc Picard", "jean luc picard"),
        ("Dr. Jane O'Neil", "jane oneil"),
        ("Hannah O’Neil", "hannah oneil"),
        ("\"Jane Doe\"", "jane doe"),
        ("José García", "jose garcia"),
        ("Jürgen Müller", "jurgen muller"),
        ("Martin Luther King, Jr.", "martin luther king"),
        ("Mr. John Smith III", "john smith"),
        ("Prof. Dr. Anna Schmidt", "anna schmidt"),
        ("jane_doe", "jane doe"),
        ("j.doe", "j doe"),
        ("王小明", "王小明"),
        ("Наталья Иванова", "наталья иванова"),
        ("Ελένη Παππά", "ελενη παππα"),
        ("प्रिया शर्मा", "प्रिया शर्मा"),
        ("Jane Doe (via Google Sheets)", "jane doe"),
        ("Ｊａｎｅ Ｄｏｅ", "jane doe"),
        ("", ""),
        ("🚀🚀", ""),
    ];

    #[test]
    fn name_key_table() {
        for (raw, want) in NAME_CASES {
            let got = name_key(raw);
            assert_eq!(&got, want, "name_key({raw:?})");
            assert_eq!(name_key(&got), got, "idempotence of {raw:?}");
        }
    }

    #[test]
    fn er21_variants_share_one_key() {
        let k = name_key("Jane Doe");
        for v in [
            "Doe, Jane",
            "Dr. Jane Doe PhD",
            "jane doe 😀",
            "Jane Doe (Stripe)",
        ] {
            assert_eq!(name_key(v), k, "{v}");
        }
        assert_ne!(name_key("J. Doe"), k);
    }

    #[test]
    fn matchability() {
        assert!(is_matchable("jane doe"));
        assert!(is_matchable("王小明"));
        assert!(is_matchable("김민준"));
        assert!(!is_matchable("jane"));
        assert!(!is_matchable("王"));
        assert!(!is_matchable(""));
        assert!(!is_matchable("jane doe2"));
        assert!(!is_matchable("jane@doe x"));
    }

    #[test]
    fn company_keys() {
        for (raw, want) in [
            ("Stripe, Inc.", "stripe"),
            ("The Northwind Traders Ltd", "northwind traders"),
            ("Contoso Pvt. Ltd.", "contoso"),
            ("Fabrikam GmbH", "fabrikam"),
            ("ACME L.L.C.", "acme"),
            ("Tailspin", "tailspin"),
            ("O'Reilly Media", "oreilly media"),
            ("  Halcyon   Labs  ", "halcyon labs"),
            ("Company", "company"),
            ("Ｓｔｒｉｐｅ", "stripe"),
        ] {
            assert_eq!(company_key(raw), want, "{raw}");
            assert_eq!(company_key(want), want, "idempotence of {raw}");
        }
    }

    #[test]
    fn alias_keys() {
        let gmail = vec!["gmail.com".to_string(), "googlemail.com".to_string()];
        assert_eq!(
            alias_key("jane.doe+news@googlemail.com", &gmail),
            "janedoe@gmail.com"
        );
        assert_eq!(alias_key("a.b@gmail.com", &gmail), "ab@gmail.com");
        assert_eq!(alias_key("ab+x@gmail.com", &gmail), "ab@gmail.com");
        assert_eq!(
            alias_key("jane+rec@acme.example", &gmail),
            "jane@acme.example"
        );
        assert_eq!(
            alias_key("jane.doe@acme.example", &gmail),
            "jane.doe@acme.example"
        );
        assert_eq!(alias_key("+x@acme.example", &gmail), "+x@acme.example");
        assert_eq!(
            alias_key("jane-news@yahoo.com", &gmail),
            "jane-news@yahoo.com"
        );
        let test = vec!["gmail.test".to_string(), "googlemail.test".to_string()];
        assert_eq!(
            alias_key("dana.whitfield+lists@googlemail.test", &test),
            "danawhitfield@gmail.test"
        );
    }

    #[test]
    fn prettified_local_parts() {
        assert_eq!(prettify_local_part("jane.doe@example.com"), "Jane Doe");
        assert_eq!(prettify_local_part("jane_doe+news@example.com"), "Jane Doe");
        assert_eq!(prettify_local_part("jdoe@example.com"), "Jdoe");
        assert_eq!(prettify_local_part("jean-luc@example.com"), "Jean Luc");
    }

    proptest::proptest! {
        #[test]
        fn normalizers_never_panic_and_are_idempotent(s in "\\PC{0,40}") {
            let k = name_key(&s);
            proptest::prop_assert_eq!(name_key(&k), k.clone());
            let c = company_key(&s);
            proptest::prop_assert_eq!(company_key(&c), c.clone());
            let c = ctx(Some("US"), "csv");
            for kind in HandleKind::ALL {
                if let Some(v) = normalize_handle(kind, &s, &c) {
                    proptest::prop_assert_eq!(normalize_handle(kind, &v, &c), Some(v.clone()));
                }
            }
        }
    }
}
