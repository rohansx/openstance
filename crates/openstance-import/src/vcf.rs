//! `vcf` importer (Phase 1, feature `vcf`): Apple / Google Contacts vCard
//! 2.1, 3.0 and 4.0. Spec: `docs/tech-spec/ingestion.md` 6.3.
//!
//! - line unfolding (CRLF + space/tab), QUOTED-PRINTABLE + CHARSET (with
//!   `=` soft line breaks), grouped properties (`item1.URL`), `ENCODING=b`
//!   photos skipped without decoding (their continuation lines are dropped
//!   as they stream past);
//! - one card = one `record` (UID, else `card:<ordinal>`); every handle is a
//!   `RawIdentity` with that record and the card's hints, `observed_at` = REV;
//! - FN (else N reassembled) -> hints.name; EMAIL -> Email; TEL -> Phone
//!   (`tel:` URIs unwrapped, normalization to E.164 is the Sink's job); ORG
//!   first component -> hints.company; TITLE -> hints.title; ADR
//!   locality/region -> hints.location; URL / X-SOCIALPROFILE / IMPP ->
//!   LinkedinUrl / XHandle / IgHandle; NOTE/BDAY/PHOTO/CATEGORIES ignored;
//! - a card with only a name emits `NameLocal("vcf|<name>")`; a card with
//!   neither handle nor name is skipped (`Malformed`), as is a card that is
//!   never closed with `END:VCARD`.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime, TimeZone};

use crate::{
    sniff, Detection, Handle, HandleKind, Hints, ImportError, ImportOptions, ImportStats, Importer,
    RawIdentity, Sink, SkipReason,
};

/// Logical lines longer than this are truncated (only PHOTO-like
/// properties get that big, and those are ignored anyway).
const MAX_LOGICAL_LINE: usize = 256 * 1024;

/// Properties whose (possibly huge, base64) values are never read.
const BINARY_PROPS: [&str; 4] = ["PHOTO", "LOGO", "SOUND", "KEY"];

/// vCard importer.
#[derive(Debug, Clone)]
pub struct VcfImporter {
    opts: ImportOptions,
}

impl VcfImporter {
    /// Construct with options from config.
    pub fn new(opts: ImportOptions) -> Self {
        VcfImporter { opts }
    }

    /// The options this importer was built with.
    pub fn options(&self) -> &ImportOptions {
        &self.opts
    }
}

impl Importer for VcfImporter {
    fn kind(&self) -> &'static str {
        "vcf"
    }

    /// Strong: starts with `BEGIN:VCARD` (after BOM/whitespace).
    /// Weak: `.vcf` / `.vcard` extension.
    fn detect(&self, path: &Path) -> Detection {
        if !path.is_file() {
            return Detection::No;
        }
        let buf = sniff(path);
        let body = buf.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&buf);
        let start = body
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(body.len());
        let head = &body[start..];
        if head.len() >= 11 && head[..11].eq_ignore_ascii_case(b"BEGIN:VCARD") {
            return Detection::Strong;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("vcf") | Some("vcard") => Detection::Weak,
            _ => Detection::No,
        }
    }

    fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
        let started = std::time::Instant::now();
        let file = File::open(path)?;
        let total = file.metadata().ok().map(|m| m.len());
        let mut reader = BufReader::with_capacity(256 * 1024, file);
        let mut parser = Parser::new(&self.opts);
        let mut stats = ImportStats::default();
        let mut raw = Vec::with_capacity(256);
        let mut first = true;
        let mut reported = 0u64;
        loop {
            raw.clear();
            let n = reader.read_until(b'\n', &mut raw)?;
            if n == 0 {
                break;
            }
            stats.bytes += n as u64;
            let mut line: &[u8] = &raw;
            if first {
                line = line.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(line);
            }
            while let Some((b'\n' | b'\r', rest)) = line.split_last() {
                line = rest;
            }
            if first {
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                first = false;
                if !trim_ascii(line).eq_ignore_ascii_case(b"BEGIN:VCARD") {
                    return Err(ImportError::WrongFormat {
                        kind: "vcf",
                        reason: "file does not start with BEGIN:VCARD".into(),
                    });
                }
            }
            parser.physical_line(line, sink, &mut stats);
            if parser.cards >= reported + 1000 {
                reported = parser.cards;
                sink.progress(stats.bytes, total);
            }
        }
        if first {
            return Err(ImportError::WrongFormat {
                kind: "vcf",
                reason: "file is empty".into(),
            });
        }
        parser.finish(sink, &mut stats);
        sink.progress(stats.bytes, total);
        stats.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(stats)
    }
}

// ---------------------------------------------------------------------------
// Line unfolding
// ---------------------------------------------------------------------------

struct Parser<'o> {
    opts: &'o ImportOptions,
    /// The logical line being assembled.
    cur: Vec<u8>,
    has_cur: bool,
    /// The current logical line is a binary property: drop continuations.
    cur_binary: bool,
    /// The current logical line is quoted-printable (soft breaks apply).
    cur_qp: bool,
    card: Option<Card>,
    /// Cards started so far (the ordinal of the current card).
    cards: u64,
}

impl<'o> Parser<'o> {
    fn new(opts: &'o ImportOptions) -> Self {
        Parser {
            opts,
            cur: Vec::new(),
            has_cur: false,
            cur_binary: false,
            cur_qp: false,
            card: None,
            cards: 0,
        }
    }

    fn physical_line(&mut self, line: &[u8], sink: &mut dyn Sink, stats: &mut ImportStats) {
        if self.has_cur {
            if let Some((b' ' | b'\t', rest)) = line.split_first() {
                if !self.cur_binary {
                    self.append(rest);
                }
                return;
            }
            if self.cur_qp && self.cur.last() == Some(&b'=') {
                // Quoted-printable soft line break (vCard 2.1).
                self.cur.pop();
                self.append(line);
                return;
            }
            self.flush(sink, stats);
        }
        self.cur.clear();
        self.has_cur = true;
        self.cur_binary = false;
        self.cur_qp = false;
        if let Some(colon) = find_colon(line) {
            let head = &line[..colon];
            let name = prop_name(head);
            self.cur_binary = BINARY_PROPS.iter().any(|b| name.eq_ignore_ascii_case(b));
            self.cur_qp = head_is_qp(head);
        }
        if self.cur_binary {
            // Keep only the property head; the value is never read.
            let colon = find_colon(line).unwrap_or(line.len());
            self.append(&line[..colon]);
        } else {
            self.append(line);
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        let room = MAX_LOGICAL_LINE.saturating_sub(self.cur.len());
        self.cur.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }

    fn flush(&mut self, sink: &mut dyn Sink, stats: &mut ImportStats) {
        if !self.has_cur {
            return;
        }
        self.has_cur = false;
        let line = std::mem::take(&mut self.cur);
        self.logical_line(&line, sink, stats);
        self.cur = line;
        self.cur.clear();
    }

    fn finish(&mut self, sink: &mut dyn Sink, stats: &mut ImportStats) {
        self.flush(sink, stats);
        if self.card.take().is_some() {
            // Never closed with END:VCARD.
            let loc = format!("card {}", self.cards);
            sink.skip(SkipReason::Malformed, &loc);
            stats.record_skip(SkipReason::Malformed);
        }
    }

    fn logical_line(&mut self, line: &[u8], sink: &mut dyn Sink, stats: &mut ImportStats) {
        let Some(colon) = find_colon(line) else {
            return;
        };
        let head = &line[..colon];
        let raw_value = &line[colon + 1..];
        let name = prop_name(head).to_ascii_uppercase();
        let params = parse_params(head);
        match name.as_str() {
            "BEGIN" if trim_ascii(raw_value).eq_ignore_ascii_case(b"VCARD") => {
                if self.card.is_some() {
                    // A card opened inside another: the outer one is broken.
                    let loc = format!("card {}", self.cards);
                    sink.skip(SkipReason::Malformed, &loc);
                    stats.record_skip(SkipReason::Malformed);
                }
                self.cards += 1;
                stats.records_read += 1;
                self.card = Some(Card::default());
            }
            "END" if trim_ascii(raw_value).eq_ignore_ascii_case(b"VCARD") => {
                if let Some(card) = self.card.take() {
                    card.emit(self.cards, self.opts, sink, stats);
                }
            }
            _ => {
                if let Some(card) = self.card.as_mut() {
                    let value = decode_value(raw_value, &params);
                    card.property(&name, &params, &value);
                }
            }
        }
    }
}

/// Index of the `:` separating property head and value (outside quotes).
fn find_colon(line: &[u8]) -> Option<usize> {
    let mut quoted = false;
    for (i, &b) in line.iter().enumerate() {
        match b {
            b'"' => quoted = !quoted,
            b':' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

/// Property name without group prefix (`item1.EMAIL` -> `EMAIL`).
fn prop_name(head: &[u8]) -> String {
    let name_end = head.iter().position(|&b| b == b';').unwrap_or(head.len());
    let name = &head[..name_end];
    let name = match name.iter().rposition(|&b| b == b'.') {
        Some(dot) => &name[dot + 1..],
        None => name,
    };
    String::from_utf8_lossy(trim_ascii(name)).into_owned()
}

/// Parameters as `(KEY uppercase, value)`; bare 2.1 params get key `TYPE`
/// except the encoding keywords, which become `ENCODING`.
fn parse_params(head: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(head);
    let mut out = Vec::new();
    for part in text.split(';').skip(1) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('=') {
            Some((k, v)) => {
                let k = k.trim().to_ascii_uppercase();
                for v in v.split(',') {
                    out.push((k.clone(), v.trim().trim_matches('"').to_string()));
                }
            }
            None => {
                let up = part.to_ascii_uppercase();
                let key = match up.as_str() {
                    "QUOTED-PRINTABLE" | "BASE64" | "B" | "8BIT" | "7BIT" => "ENCODING",
                    _ => "TYPE",
                };
                out.push((key.to_string(), part.to_string()));
            }
        }
    }
    out
}

fn param<'p>(params: &'p [(String, String)], key: &str) -> Option<&'p str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn head_is_qp(head: &[u8]) -> bool {
    parse_params(head)
        .iter()
        .any(|(k, v)| k == "ENCODING" && v.eq_ignore_ascii_case("QUOTED-PRINTABLE"))
}

/// Decode a raw value: quoted-printable if declared, then the charset.
fn decode_value(raw: &[u8], params: &[(String, String)]) -> String {
    let is_qp =
        param(params, "ENCODING").is_some_and(|e| e.eq_ignore_ascii_case("QUOTED-PRINTABLE"));
    let bytes: std::borrow::Cow<'_, [u8]> = if is_qp {
        match quoted_printable::decode(raw, quoted_printable::ParseMode::Robust) {
            Ok(b) => b.into(),
            Err(_) => raw.into(),
        }
    } else {
        raw.into()
    };
    let charset = param(params, "CHARSET").map(|c| c.to_ascii_uppercase());
    match charset.as_deref() {
        Some("ISO-8859-1" | "LATIN1" | "LATIN-1" | "WINDOWS-1252" | "CP1252") => {
            bytes.iter().map(|&b| b as char).collect()
        }
        _ => match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_string(),
            // Undeclared non-UTF-8 bytes: Latin-1 is the common culprit.
            Err(_) => bytes.iter().map(|&b| b as char).collect(),
        },
    }
}

fn trim_ascii(b: &[u8]) -> &[u8] {
    let start = b
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(b.len());
    let end = b
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .map_or(start, |e| e + 1);
    &b[start..end]
}

// ---------------------------------------------------------------------------
// Card
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Card {
    fn_name: Option<String>,
    n_name: Option<String>,
    handles: Vec<Handle>,
    company: Option<String>,
    title: Option<String>,
    location: Option<String>,
    rev: Option<String>,
    uid: Option<String>,
}

impl Card {
    fn property(&mut self, name: &str, params: &[(String, String)], value: &str) {
        match name {
            "FN" => set_once(&mut self.fn_name, unescape(value)),
            "N" => {
                let parts = split_components(value);
                let family = parts.first().map(String::as_str).unwrap_or("");
                let given = parts.get(1).map(String::as_str).unwrap_or("");
                let joined = [given, family]
                    .iter()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim())
                    .collect::<Vec<_>>()
                    .join(" ");
                set_once(&mut self.n_name, joined);
            }
            "EMAIL" => {
                let v = value.trim();
                let v = strip_prefix_ci(v, "mailto:").unwrap_or(v).trim();
                if !v.is_empty() {
                    self.handles.push(Handle::new(HandleKind::Email, v));
                }
            }
            "TEL" => {
                let v = value.trim();
                let v = strip_prefix_ci(v, "tel:").unwrap_or(v);
                let v = v.split(';').next().unwrap_or("").trim();
                if !v.is_empty() {
                    self.handles.push(Handle::new(HandleKind::Phone, v));
                }
            }
            "ORG" => {
                let first = split_components(value)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                set_once(&mut self.company, first);
            }
            "TITLE" => set_once(&mut self.title, unescape(value)),
            "ADR" => {
                let parts = split_components(value);
                let locality = parts.get(3).map(|s| s.trim()).unwrap_or("");
                let region = parts.get(4).map(|s| s.trim()).unwrap_or("");
                let loc = [locality, region]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ");
                set_once(&mut self.location, loc);
            }
            "URL" | "X-SOCIALPROFILE" | "IMPP" => {
                if let Some(h) = social_handle(name, params, value.trim()) {
                    self.handles.push(h);
                }
            }
            "REV" => set_once(&mut self.rev, value.trim().to_string()),
            "UID" => set_once(&mut self.uid, value.trim().to_string()),
            _ => {}
        }
    }

    fn emit(
        self,
        ordinal: u64,
        opts: &ImportOptions,
        sink: &mut dyn Sink,
        stats: &mut ImportStats,
    ) {
        let name = self.fn_name.or(self.n_name);
        let hints = Hints {
            name: name.clone(),
            title: self.title,
            company: self.company,
            location: self.location,
        };
        let mut handles = self.handles;
        if handles.is_empty() {
            match &name {
                Some(n) => handles.push(Handle::new(HandleKind::NameLocal, format!("vcf|{n}"))),
                None => {
                    let loc = format!("card {ordinal}");
                    sink.skip(SkipReason::Malformed, &loc);
                    stats.record_skip(SkipReason::Malformed);
                    return;
                }
            }
        }
        let record = self.uid.unwrap_or_else(|| format!("card:{ordinal}"));
        let observed_at = self.rev.as_deref().and_then(|r| parse_rev(r, opts.tz));
        for handle in handles {
            sink.identity(RawIdentity {
                handle,
                hints: hints.clone(),
                record: record.clone(),
                observed_at,
            });
        }
    }
}

fn set_once(slot: &mut Option<String>, value: String) {
    let v = value.trim();
    if slot.is_none() && !v.is_empty() {
        *slot = Some(v.to_string());
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && s[..prefix.len()].eq_ignore_ascii_case(prefix)
    {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Undo vCard text escaping (`\,` `\;` `\\` `\n`).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push(' '),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Split a structured value on unescaped `;`, unescaping each component.
fn split_components(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            cur.push('\\');
            cur.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ';' {
            parts.push(unescape(&cur));
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    if escaped {
        cur.push('\\');
    }
    parts.push(unescape(&cur));
    parts
}

/// Classify a URL-ish value into a social handle, or `None` for other URLs.
fn social_handle(prop: &str, params: &[(String, String)], value: &str) -> Option<Handle> {
    let lower = value.to_ascii_lowercase();
    if lower.contains("linkedin.com/in/") || lower.contains("linkedin.com/pub/") {
        return Some(Handle::new(HandleKind::LinkedinUrl, value));
    }
    let rest = lower
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(lower.as_str());
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host
        .trim_start_matches("www.")
        .trim_start_matches("mobile.")
        .trim_start_matches("m.");
    let first_seg = |path: &str| -> Option<String> {
        // Use the original casing of the value for the segment.
        let start = value.len() - path.len();
        let seg = value[start..].split(['/', '?', '#']).next().unwrap_or("");
        let seg = seg.trim().trim_start_matches('@');
        (!seg.is_empty()).then(|| seg.to_string())
    };
    match host {
        "x.com" | "twitter.com" => {
            let seg = first_seg(path)?;
            if ["intent", "home", "i", "share"].contains(&seg.to_ascii_lowercase().as_str()) {
                return None;
            }
            return Some(Handle::new(HandleKind::XHandle, seg));
        }
        "instagram.com" => return first_seg(path).map(|s| Handle::new(HandleKind::IgHandle, s)),
        _ => {}
    }
    if prop == "X-SOCIALPROFILE" && !lower.contains("://") {
        let kind = params
            .iter()
            .filter(|(k, _)| k == "TYPE")
            .find_map(|(_, v)| match v.to_ascii_lowercase().as_str() {
                "twitter" | "x" => Some(HandleKind::XHandle),
                "instagram" => Some(HandleKind::IgHandle),
                _ => None,
            })?;
        let handle = param(params, "X-USER")
            .map(str::to_string)
            .or_else(|| value.rsplit(':').next().map(str::to_string))?;
        let handle = handle.trim().trim_start_matches('@');
        return (!handle.is_empty()).then(|| Handle::new(kind, handle));
    }
    None
}

/// `REV` in the common basic / extended ISO 8601 forms.
fn parse_rev(s: &str, tz: chrono_tz::Tz) -> Option<i64> {
    let s = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }
    for f in ["%Y%m%dT%H%M%S%#z", "%Y-%m-%dT%H:%M:%S%#z"] {
        if let Ok(dt) = chrono::DateTime::parse_from_str(s, f) {
            return Some(dt.timestamp());
        }
    }
    let (body, utc) = match s.strip_suffix(['Z', 'z']) {
        Some(b) => (b, true),
        None => (s, false),
    };
    let naive = ["%Y%m%dT%H%M%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M:%S%.f"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(body, f).ok())
        .or_else(|| {
            ["%Y%m%d", "%Y-%m-%d"]
                .iter()
                .find_map(|f| NaiveDate::parse_from_str(body, f).ok())
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })?;
    if utc {
        Some(naive.and_utc().timestamp())
    } else {
        tz.from_local_datetime(&naive)
            .earliest()
            .map(|d| d.timestamp())
    }
}
