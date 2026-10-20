//! Storage normalization (in the Sink, versioned by
//! `meta.normalizer_version`) and match keys (recomputed on every run).
//! Spec: `docs/tech-spec/entity-resolution.md` section 2 (P1-CORE-06).
//!
//! Owned by the "resolve" stream. Every function is pure, never panics on
//! arbitrary UTF-8, and is idempotent (`f(f(x)) == f(x)`).

use openstance_import::HandleKind;

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
    let _ = (kind, raw, ctx);
    todo!("resolve stream: P1-CORE-06 normalize_handle")
}

/// Email: trim, strip `<>`/`mailto:`, NFKC, lowercase, IDNA domain, strip a
/// trailing dot. Rejects no `@`, empty local part, dotless domain, > 254
/// chars, `undisclosed-recipients`. Gmail aliasing is NOT applied here.
pub fn normalize_email(raw: &str) -> Option<String> {
    let _ = raw;
    todo!("resolve stream: P1-CORE-06 normalize_email")
}

/// Phone to E.164 with the `phonenumber` crate; extensions dropped.
pub fn normalize_phone(raw: &str, default_region: Option<&str>) -> Option<String> {
    let _ = (raw, default_region);
    todo!("resolve stream: P1-CORE-06 normalize_phone")
}

/// LinkedIn URL to the lowercase slug (`/in/<slug>`; legacy `/pub/...` ->
/// `pub:...`). Company / school / post URLs are rejected.
pub fn normalize_linkedin(raw: &str) -> Option<String> {
    let _ = raw;
    todo!("resolve stream: P1-CORE-06 normalize_linkedin")
}

/// X handle without `@` or URL prefix, `^[a-z0-9_]{1,15}$`.
pub fn normalize_x_handle(raw: &str) -> Option<String> {
    let _ = raw;
    todo!("resolve stream: P1-CORE-06 normalize_x_handle")
}

/// Instagram handle, `^[a-z0-9._]{1,30}$`.
pub fn normalize_ig_handle(raw: &str) -> Option<String> {
    let _ = raw;
    todo!("resolve stream: P1-CORE-06 normalize_ig_handle")
}

/// The 11-step `name_key` (NFKC, strip emoji/quotes/brackets/"via" suffixes,
/// swap `Last, First`, lowercase, separators to spaces, drop honorifics and
/// suffixes, fold diacritics, collapse whitespace).
pub fn name_key(name: &str) -> String {
    let _ = name;
    todo!("resolve stream: P1-CORE-06 name_key")
}

/// A name key usable by rules 3-5: >= 2 tokens (or >= 2 chars of a
/// space-less script), no digits, no `@`.
pub fn is_matchable(key: &str) -> bool {
    let _ = key;
    todo!("resolve stream: P1-CORE-06 is_matchable")
}

/// `company_key`: NFKC, lowercase, drop punctuation, leading `the`, trailing
/// legal forms, collapse whitespace (before the domain/alias map).
pub fn company_key(name: &str) -> String {
    let _ = name;
    todo!("resolve stream: P1-CORE-06 company_key")
}

/// Rule 2 alias key: on `gmail_domains` drop dots and map to the first
/// (canonical) domain; on any domain strip `+tag` (position > 0).
pub fn alias_key(email: &str, gmail_domains: &[String]) -> String {
    let _ = (email, gmail_domains);
    todo!("resolve stream: P1-CORE-06 alias_key")
}

/// Display fallback from an email local part (`jane.doe` -> `Jane Doe`).
/// Never used as a match key.
pub fn prettify_local_part(email: &str) -> String {
    let _ = email;
    todo!("resolve stream: P1-CORE-06 prettify_local_part")
}
