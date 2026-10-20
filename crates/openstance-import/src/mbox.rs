//! `mbox` importer (Phase 1, feature `mbox`): Google Takeout and any
//! mboxo/mboxrd file, headers only. Spec: `docs/tech-spec/ingestion.md` 6.1.
//!
//! - streams the file through a 1 MiB reader one line at a time, keeping at
//!   most 1 KiB of any body line (only to recognize the next `From `
//!   separator) and never buffering a body;
//! - a separator is a line starting with `From ` at the start of the file or
//!   right after a blank line, so unescaped `From ` lines inside a paragraph
//!   stay body text;
//! - parses header blocks (cap 256 KiB -> `SkipReason::HeaderTooLarge`) with
//!   `mail-parser`: From/To/Cc/Bcc/Date/Subject/Message-ID/In-Reply-To/
//!   References/List-Unsubscribe/Precedence/X-Gmail-Labels/X-GM-THRID/
//!   Auto-Submitted, RFC 2047 decoding, address groups;
//! - `at` from `Date`, fallback to the `From ` line date, else `BadDate` skip;
//! - `external_id` = Message-ID (angle brackets stripped, lowercased);
//! - `thread_key` = X-GM-THRID, else first References id, else In-Reply-To,
//!   else own Message-ID;
//! - `dir_hint = Out` when X-Gmail-Labels contains `Sent`;
//! - `list_unsub` from List-Unsubscribe / Precedence / Auto-Submitted;
//! - skips Spam/Trash (unless `include_spam`) and Chat as `Unsupported`, and
//!   messages whose body exceeds `max_message_bytes` as `Unsupported`;
//! - reports the dominant `Sent` From address in `ImportStats.self_candidates`;
//! - `email_bodies = true` returns `ImportError::Unsupported` in Phase 1.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

use chrono::{NaiveDateTime, TimeZone};
use mail_parser::{Address, HeaderValue, Message, MessageParser};

use crate::{
    sniff, Detection, Direction, Handle, HandleKind, Hints, ImportError, ImportOptions,
    ImportStats, Importer, InteractionKind, Participant, RawInteraction, Role, Sink, SkipReason,
};

/// Read buffer size.
const READ_BUF: usize = 1 << 20;
/// Largest header block parsed; bigger ones are skipped.
const MAX_HEADER_BYTES: usize = 256 * 1024;
/// Bytes kept of a body line (enough to recognize and parse a `From ` line).
const MAX_BODY_LINE: usize = 1024;
/// Progress is reported every this many messages.
const PROGRESS_EVERY: u64 = 1000;

/// Headers-only mbox importer.
#[derive(Debug, Clone)]
pub struct MboxImporter {
    opts: ImportOptions,
}

impl MboxImporter {
    /// Construct with options from config.
    pub fn new(opts: ImportOptions) -> Self {
        MboxImporter { opts }
    }

    /// The options this importer was built with.
    pub fn options(&self) -> &ImportOptions {
        &self.opts
    }
}

impl Importer for MboxImporter {
    fn kind(&self) -> &'static str {
        "mbox"
    }

    /// Strong: first non-empty line starts with `From ` and the first 64 KiB
    /// contain `Message-ID:` or `Date:`. Weak: `.mbox` extension.
    fn detect(&self, path: &Path) -> Detection {
        if !path.is_file() {
            return Detection::No;
        }
        let buf = sniff(path);
        let first = buf
            .split(|&b| b == b'\n')
            .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
            .find(|l| !l.iter().all(u8::is_ascii_whitespace));
        if first.is_some_and(|l| l.starts_with(b"From ")) {
            let lower = buf.to_ascii_lowercase();
            let has = |needle: &[u8]| memchr::memmem::find(&lower, needle).is_some();
            if has(b"\nmessage-id:") || has(b"\ndate:") {
                return Detection::Strong;
            }
        }
        let is_mbox = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("mbox"));
        if is_mbox {
            Detection::Weak
        } else {
            Detection::No
        }
    }

    fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
        if self.opts.email_bodies {
            return Err(ImportError::Unsupported(
                "reading email bodies (--bodies) is not supported until a later release".into(),
            ));
        }
        let started = std::time::Instant::now();
        let file = File::open(path)?;
        let total = file.metadata().ok().map(|m| m.len());
        let mut lines = LineReader::new(file);
        let mut stats = ImportStats::default();
        let mut sent_from: BTreeMap<String, u64> = BTreeMap::new();
        let parser = MessageParser::new();

        // The first non-empty line must be a separator.
        let mut line = Vec::with_capacity(MAX_BODY_LINE);
        let mut from_line = loop {
            if !lines.read(&mut line, MAX_BODY_LINE)? {
                return Err(wrong_format("file is empty"));
            }
            if is_blank(&line) {
                continue;
            }
            if line.starts_with(b"From ") {
                break line.clone();
            }
            return Err(wrong_format("first line is not a `From ` separator"));
        };

        let mut header = Vec::with_capacity(8 * 1024);
        let mut msg_no = 0u64;
        loop {
            msg_no += 1;
            stats.records_read += 1;

            // Header block: until the first blank line (or EOF / separator).
            header.clear();
            let mut too_large = false;
            let mut next_from: Option<Vec<u8>> = None;
            let mut eof = false;
            loop {
                let room = (MAX_HEADER_BYTES + 1).saturating_sub(header.len()).max(1);
                if !lines.read(&mut line, room.min(MAX_HEADER_BYTES + 1))? {
                    eof = true;
                    break;
                }
                if is_blank(&line) {
                    break;
                }
                if header.is_empty() && line.starts_with(b"From ") {
                    // A separator right after a separator: an empty message.
                    next_from = Some(line.clone());
                    break;
                }
                if too_large || header.len() + line.len() > MAX_HEADER_BYTES || lines.truncated {
                    too_large = true;
                    continue;
                }
                header.extend_from_slice(&line);
            }

            // Body: skipped line by line until the next separator.
            let mut body_bytes = 0u64;
            if next_from.is_none() && !eof {
                let mut prev_blank = true;
                loop {
                    let before = lines.bytes;
                    if !lines.read(&mut line, MAX_BODY_LINE)? {
                        break;
                    }
                    if prev_blank && line.starts_with(b"From ") {
                        next_from = Some(line.clone());
                        break;
                    }
                    body_bytes += lines.bytes - before;
                    prev_blank = is_blank(&line);
                }
            }

            let locator = format!("message {msg_no}");
            let outcome = if too_large {
                Err(SkipReason::HeaderTooLarge)
            } else if body_bytes > self.opts.max_message_bytes {
                Err(SkipReason::Unsupported)
            } else if header.is_empty() {
                Err(SkipReason::Malformed)
            } else {
                header.push(b'\n');
                match parser.parse_headers(&header[..]) {
                    Some(m) => self.map_message(&m, &from_line, &mut sent_from),
                    None => Err(SkipReason::Malformed),
                }
            };
            match outcome {
                Ok(i) => sink.interaction(i),
                Err(reason) => {
                    sink.skip(reason, &locator);
                    stats.record_skip(reason);
                }
            }
            if msg_no % PROGRESS_EVERY == 0 {
                sink.progress(lines.bytes, total);
            }

            match next_from {
                Some(f) => from_line = f,
                None => break,
            }
        }

        stats.bytes = lines.bytes;
        sink.progress(stats.bytes, total);
        let mut cands: Vec<(u64, String)> = sent_from.into_iter().map(|(a, n)| (n, a)).collect();
        cands.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        stats.self_candidates = cands
            .into_iter()
            .take(1)
            .map(|(_, a)| format!("email:{a}"))
            .collect();
        stats.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(stats)
    }
}

impl MboxImporter {
    fn map_message(
        &self,
        m: &Message<'_>,
        from_line: &[u8],
        sent_from: &mut BTreeMap<String, u64>,
    ) -> Result<RawInteraction, SkipReason> {
        let labels: Vec<String> = text(m, "X-Gmail-Labels")
            .map(|l| {
                l.split(',')
                    .map(|s| s.trim().trim_matches('"').trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let has_label = |name: &str| labels.iter().any(|l| l == name);
        if has_label("chat") {
            return Err(SkipReason::Unsupported);
        }
        if !self.opts.include_spam && (has_label("spam") || has_label("trash")) {
            return Err(SkipReason::Unsupported);
        }

        let at = m
            .date()
            .filter(|d| d.is_valid())
            .map(|d| match m.header_raw("Date").and_then(named_zone_offset) {
                // mail-parser drops RFC 822 zone names; apply them here.
                Some(off) if d.tz_hour == 0 && d.tz_minute == 0 => d.to_timestamp_local() - off,
                _ => d.to_timestamp(),
            })
            .or_else(|| from_line_date(from_line, self.opts.tz))
            .ok_or(SkipReason::BadDate)?;

        let mut participants = Vec::new();
        for (role, addr) in [
            (Role::From, m.from()),
            (Role::To, m.to()),
            (Role::Cc, m.cc()),
            (Role::Bcc, m.bcc()),
        ] {
            if let Some(a) = addr {
                push_addresses(&mut participants, role, a);
            }
        }
        if participants.is_empty() {
            return Err(SkipReason::NoParticipants);
        }

        let sent = has_label("sent");
        if sent {
            if let Some(p) = participants.iter().find(|p| p.role == Role::From) {
                *sent_from
                    .entry(p.handles[0].raw.to_lowercase())
                    .or_insert(0) += 1;
            }
        }

        let message_id = m.message_id().and_then(clean_id);
        let thread_key = text(m, "X-GM-THRID")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| first_id(m.references()))
            .or_else(|| first_id(m.in_reply_to()))
            .or_else(|| message_id.clone());

        let precedence_bulk = text(m, "Precedence").is_some_and(|p| {
            matches!(
                p.trim().to_ascii_lowercase().as_str(),
                "bulk" | "list" | "junk"
            )
        });
        let auto_submitted = text(m, "Auto-Submitted")
            .is_some_and(|a| !a.trim().is_empty() && !a.trim().eq_ignore_ascii_case("no"));
        let list_unsub = !m.list_unsubscribe().is_empty() || precedence_bulk || auto_submitted;

        Ok(RawInteraction {
            kind: InteractionKind::Email,
            at,
            dir_hint: sent.then_some(Direction::Out),
            subject: m
                .subject()
                .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|s| !s.is_empty()),
            external_id: message_id,
            thread_key,
            list_unsub,
            participants,
        })
    }
}

fn wrong_format(reason: &str) -> ImportError {
    ImportError::WrongFormat {
        kind: "mbox",
        reason: reason.into(),
    }
}

fn is_blank(line: &[u8]) -> bool {
    line.iter().all(|&b| b == b'\n' || b == b'\r')
}

/// A header's decoded text (unknown headers are kept as raw text).
fn text<'a>(m: &'a Message<'_>, name: &'static str) -> Option<&'a str> {
    match m.header(name)? {
        HeaderValue::Text(t) => Some(t.as_ref()),
        HeaderValue::TextList(l) => l.first().map(|s| s.as_ref()),
        _ => m.header_raw(name),
    }
}

/// Message ids are compared lowercased without angle brackets.
fn clean_id(s: &str) -> Option<String> {
    let id = s
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    (!id.is_empty()).then(|| id.to_lowercase())
}

fn first_id(v: &HeaderValue<'_>) -> Option<String> {
    match v {
        HeaderValue::Text(t) => t.split_whitespace().next().and_then(clean_id),
        HeaderValue::TextList(l) => l.iter().find_map(|t| clean_id(t)),
        _ => None,
    }
}

fn push_addresses(out: &mut Vec<Participant>, role: Role, addr: &Address<'_>) {
    let mut push = |a: &mail_parser::Addr<'_>| {
        let Some(email) = a
            .address
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
        else {
            return;
        };
        let name = a
            .name
            .as_deref()
            .map(|n| n.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case(email));
        out.push(Participant {
            handles: vec![Handle::new(HandleKind::Email, email)],
            role,
            hints: Hints {
                name,
                ..Default::default()
            },
        });
    };
    match addr {
        Address::List(list) => list.iter().for_each(&mut push),
        Address::Group(groups) => groups
            .iter()
            .flat_map(|g| g.addresses.iter())
            .for_each(&mut push),
    }
}

/// Offset in seconds east of UTC for an RFC 822 zone name ending a `Date`
/// value (`EST`, `PDT`, ...), ignoring a trailing `(comment)`.
fn named_zone_offset(raw: &str) -> Option<i64> {
    let raw = match raw.find('(') {
        Some(i) => &raw[..i],
        None => raw,
    };
    let zone = raw.split_whitespace().last()?.to_ascii_uppercase();
    let hours = match zone.as_str() {
        "EST" => -5,
        "EDT" => -4,
        "CST" => -6,
        "CDT" => -5,
        "MST" => -7,
        "MDT" => -6,
        "PST" => -8,
        "PDT" => -7,
        _ => return None,
    };
    Some(hours * 3600)
}

/// Date from the separator line: `From <addr> <asctime>` (optionally with a
/// numeric zone before the year, as in Takeout).
fn from_line_date(line: &[u8], tz: chrono_tz::Tz) -> Option<i64> {
    let s = String::from_utf8_lossy(line);
    let mut words = s.split_whitespace().skip(2); // "From", sender
    let rest = words.by_ref().collect::<Vec<_>>().join(" ");
    if let Ok(dt) = chrono::DateTime::parse_from_str(&rest, "%a %b %d %H:%M:%S %z %Y") {
        return Some(dt.timestamp());
    }
    let naive = ["%a %b %d %H:%M:%S %Y", "%a %b %d %H:%M %Y"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(&rest, f).ok())?;
    tz.from_local_datetime(&naive)
        .earliest()
        .map(|d| d.timestamp())
}

// ---------------------------------------------------------------------------
// Bounded line reader
// ---------------------------------------------------------------------------

/// Reads lines through a 1 MiB buffer, keeping at most `cap` bytes of each
/// line (the rest is consumed and dropped).
struct LineReader<R: Read> {
    inner: BufReader<R>,
    /// Bytes consumed so far.
    bytes: u64,
    /// The last line was longer than its cap.
    truncated: bool,
}

impl<R: Read> LineReader<R> {
    fn new(r: R) -> Self {
        LineReader {
            inner: BufReader::with_capacity(READ_BUF, r),
            bytes: 0,
            truncated: false,
        }
    }

    /// Read the next line (with its terminator) into `buf`. `false` at EOF.
    fn read(&mut self, buf: &mut Vec<u8>, cap: usize) -> io::Result<bool> {
        buf.clear();
        self.truncated = false;
        let mut any = false;
        loop {
            let avail = match self.inner.fill_buf() {
                Ok(a) => a,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if avail.is_empty() {
                return Ok(any);
            }
            any = true;
            let (take, done) = match memchr::memchr(b'\n', avail) {
                Some(i) => (i + 1, true),
                None => (avail.len(), false),
            };
            let room = cap.saturating_sub(buf.len());
            if take > room {
                self.truncated = true;
            }
            buf.extend_from_slice(&avail[..take.min(room)]);
            self.inner.consume(take);
            self.bytes += take as u64;
            if done {
                return Ok(true);
            }
        }
    }
}
