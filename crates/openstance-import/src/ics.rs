//! `ics` importer (Phase 3, feature `ics`): calendar exports (Google
//! Calendar, Apple Calendar, Outlook). Spec: `docs/tech-spec/ingestion.md`
//! 6.4.
//!
//! - RFC 5545 content lines, read as a stream: line unfolding (CRLF or LF
//!   followed by space/tab, joined at the byte level so folded UTF-8 survives),
//!   quoted parameter values, TEXT unescaping. Only `VEVENT`s are read; nested
//!   components (`VALARM`) and `VTODO`/`VJOURNAL`/`VTIMEZONE` are passed over.
//! - One `meeting` interaction per event instance: `subject` = `SUMMARY`,
//!   `at` = `DTSTART`, `dir_hint = Mutual`, `thread_key` = `UID`,
//!   participants `ORGANIZER` (role `organizer`) and each `ATTENDEE` (role
//!   `attendee`), `mailto:` (or Apple's `EMAIL=` parameter) -> `Email`, `CN`
//!   -> hints.name (unless the CN is itself an address).
//! - `external_id`: `UID` for a single event; `UID/<instant>` for an instance
//!   of a recurring series, where `<instant>` is the `RECURRENCE-ID` (or the
//!   expanded `DTSTART`) in UTC as `YYYYMMDDTHHMMSSZ`, so a moved occurrence
//!   and the occurrence it replaces share one id, and the same meeting in a
//!   Google and an Apple export dedupes globally.
//! - Times: `Z` is UTC; `TZID` resolves through the bundled tz database
//!   (IANA names, quoted names, `/vendor/.../Area/City` prefixes and common
//!   Windows names); floating times and all-day `VALUE=DATE` values use the
//!   calendar's `X-WR-TIMEZONE`, else [`ImportOptions::tz`]. An unknown
//!   `TZID` falls back the same way.
//! - Recurrence: `RRULE` is expanded (with `EXDATE` and `RECURRENCE-ID`
//!   overrides applied) only for instances before now, keeping the most
//!   recent [`MAX_INSTANCES_PER_SERIES`]. A series with no past instance is
//!   skipped (`Unsupported`). An `RRULE` that cannot be read keeps the first
//!   occurrence and counts one `Malformed` skip. `RDATE` is not expanded.
//!   Single (non-recurring) events are kept whatever their date; scoring
//!   ignores meetings after now.
//! - Skipped: `STATUS:CANCELLED` events and occurrences (`Unsupported`),
//!   events the owner declined (`Unsupported`), events with more than
//!   [`ImportOptions::ics_max_attendees`] attendees (`Unsupported`), events
//!   with nobody besides the organizer or the owner (`NoParticipants`), a
//!   missing or unreadable `DTSTART` (`BadDate`), an event never closed with
//!   `END:VEVENT` (`Malformed`). Attendees with `PARTSTAT=DECLINED` and
//!   `CUTYPE=ROOM|RESOURCE` are dropped from the participant list.
//! - The owner is known from [`ImportOptions::me_identities`] (emails) and,
//!   for Google exports, from an `X-WR-CALNAME` that is an address (reported
//!   as a self candidate).
//! - `DESCRIPTION`, `LOCATION`, `COMMENT`, conferencing data and alarms are
//!   never read.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

use crate::{
    sniff, Detection, Direction, Handle, HandleKind, Hints, ImportError, ImportOptions,
    ImportStats, Importer, InteractionKind, Participant, RawInteraction, Role, Sink, SkipReason,
};

/// At most this many past instances of one recurring series are kept (two
/// years of a weekly meeting); the most recent ones win.
pub const MAX_INSTANCES_PER_SERIES: usize = 104;

/// Logical lines longer than this are truncated.
const MAX_LOGICAL_LINE: usize = 256 * 1024;

/// Upper bound on recurrence instances walked for one series (a guard
/// against `FREQ=SECONDLY` style rules).
const MAX_EXPANSION_STEPS: usize = 1_000_000;

/// Calendar (`.ics`) importer.
#[derive(Debug, Clone)]
pub struct IcsImporter {
    opts: ImportOptions,
    now: Option<i64>,
}

impl IcsImporter {
    /// Construct with options from config.
    pub fn new(opts: ImportOptions) -> Self {
        IcsImporter { opts, now: None }
    }

    /// Use a fixed "now" (unix seconds) for recurrence expansion instead of
    /// the system clock. For tests and reproducible imports.
    pub fn with_now(mut self, now: i64) -> Self {
        self.now = Some(now);
        self
    }

    /// The options this importer was built with.
    pub fn options(&self) -> &ImportOptions {
        &self.opts
    }
}

impl Importer for IcsImporter {
    fn kind(&self) -> &'static str {
        "ics"
    }

    /// Strong: starts with `BEGIN:VCALENDAR` (after BOM/whitespace).
    /// Weak: `.ics` extension.
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
        if head.len() >= 15 && head[..15].eq_ignore_ascii_case(b"BEGIN:VCALENDAR") {
            return Detection::Strong;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("ics") | Some("ical") => Detection::Weak,
            _ => Detection::No,
        }
    }

    fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
        let started = std::time::Instant::now();
        let file = File::open(path)?;
        let total = file.metadata().ok().map(|m| m.len());
        let mut reader = BufReader::with_capacity(256 * 1024, file);
        let now = self.now.unwrap_or_else(|| Utc::now().timestamp());
        let mut p = Parser::new(&self.opts, now);
        let mut stats = ImportStats::default();
        let mut raw = Vec::with_capacity(256);
        let mut logical: Vec<u8> = Vec::with_capacity(256);
        let mut have_logical = false;
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
                if !trim_ascii(line).eq_ignore_ascii_case(b"BEGIN:VCALENDAR") {
                    return Err(ImportError::WrongFormat {
                        kind: "ics",
                        reason: "file does not start with BEGIN:VCALENDAR".into(),
                    });
                }
            }
            if let Some((b' ' | b'\t', cont)) = line.split_first() {
                if have_logical && logical.len() < MAX_LOGICAL_LINE {
                    let room = MAX_LOGICAL_LINE - logical.len();
                    logical.extend_from_slice(&cont[..cont.len().min(room)]);
                }
                continue;
            }
            if have_logical {
                p.content_line(&String::from_utf8_lossy(&logical), sink, &mut stats);
            }
            logical.clear();
            logical.extend_from_slice(&line[..line.len().min(MAX_LOGICAL_LINE)]);
            have_logical = true;
            if p.events_read >= reported + 1000 {
                reported = p.events_read;
                sink.progress(stats.bytes, total);
            }
        }
        if first {
            return Err(ImportError::WrongFormat {
                kind: "ics",
                reason: "file is empty".into(),
            });
        }
        if have_logical {
            p.content_line(&String::from_utf8_lossy(&logical), sink, &mut stats);
        }
        p.finish(sink, &mut stats);
        sink.progress(stats.bytes, total);
        stats.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(stats)
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
    &b[start..end.max(start)]
}

// ---------------------------------------------------------------------------
// Content lines
// ---------------------------------------------------------------------------

/// One parsed content line: upper-cased name, parameters (upper-cased names,
/// unquoted values), raw value.
#[derive(Debug, Clone, Default)]
struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Prop {
    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Split `NAME;P=v;Q="a:b":value` at the first unquoted `:`.
fn parse_prop(line: &str) -> Option<Prop> {
    let mut in_quotes = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = split_unquoted(head, ';').into_iter();
    let name = parts.next()?.trim();
    // Drop a group prefix (`item1.ATTENDEE`).
    let name = name.rsplit('.').next().unwrap_or(name).to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_ascii_uppercase(), unquote(v.trim()).to_string()))
        })
        .collect();
    Some(Prop {
        name,
        params,
        value: value.to_string(),
    })
}

fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut in_quotes = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == sep && !in_quotes {
            out.push(&s[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&s[start..]);
    out
}

fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .unwrap_or(s)
}

/// RFC 5545 TEXT unescaping.
fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') | Some('N') => out.push(' '),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out.trim().to_string()
}

// ---------------------------------------------------------------------------
// Times
// ---------------------------------------------------------------------------

/// A parsed DATE / DATE-TIME value, resolved to a UTC instant.
fn parse_time(value: &str, prop: &Prop, default_tz: Tz) -> Option<i64> {
    let v = value.trim();
    let is_date = prop
        .param("VALUE")
        .is_some_and(|t| t.eq_ignore_ascii_case("DATE"))
        || (v.len() == 8 && v.bytes().all(|b| b.is_ascii_digit()));
    if is_date {
        let d = NaiveDate::parse_from_str(v.get(..8)?, "%Y%m%d").ok()?;
        return local_to_utc(default_tz, d.and_hms_opt(0, 0, 0)?);
    }
    if let Some(z) = v.strip_suffix('Z').or_else(|| v.strip_suffix('z')) {
        let naive = NaiveDateTime::parse_from_str(z, "%Y%m%dT%H%M%S").ok()?;
        return Some(Utc.from_utc_datetime(&naive).timestamp());
    }
    let naive = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    let tz = prop
        .param("TZID")
        .and_then(resolve_tzid)
        .unwrap_or(default_tz);
    local_to_utc(tz, naive)
}

/// Local wall time to a UTC instant: ambiguous times take the earlier
/// instant, times in a DST gap move forward an hour.
fn local_to_utc(tz: Tz, naive: NaiveDateTime) -> Option<i64> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(t) => Some(t.timestamp()),
        LocalResult::Ambiguous(a, _) => Some(a.timestamp()),
        LocalResult::None => match tz.from_local_datetime(&(naive + chrono::Duration::hours(1))) {
            LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t.timestamp()),
            LocalResult::None => None,
        },
    }
}

/// Windows zone names Outlook / Exchange write as `TZID`.
const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("Mountain Standard Time", "America/Denver"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Central Standard Time", "America/Chicago"),
    ("Eastern Standard Time", "America/New_York"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("India Standard Time", "Asia/Kolkata"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("UTC", "UTC"),
    ("Coordinated Universal Time", "UTC"),
];

/// Resolve a `TZID` parameter to a bundled zone.
fn resolve_tzid(tzid: &str) -> Option<Tz> {
    let t = unquote(tzid.trim());
    if let Ok(tz) = Tz::from_str(t) {
        return Some(tz);
    }
    if let Some((_, iana)) = WINDOWS_ZONES
        .iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(t))
    {
        return Tz::from_str(iana).ok();
    }
    // `/mozilla.org/20050126_1/America/New_York`, `(GMT-05:00) ...`: try the
    // trailing path segments.
    let segs: Vec<&str> = t.split('/').filter(|s| !s.is_empty()).collect();
    for take in [3usize, 2] {
        if segs.len() >= take {
            if let Ok(tz) = Tz::from_str(&segs[segs.len() - take..].join("/")) {
                return Some(tz);
            }
        }
    }
    None
}

/// `YYYYMMDDTHHMMSSZ` for an instant.
fn compact_utc(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|d| d.format("%Y%m%dT%H%M%SZ").to_string())
        .unwrap_or_else(|| ts.to_string())
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// The properties of one VEVENT that matter.
#[derive(Debug, Default)]
struct Event {
    uid: Option<String>,
    summary: Option<String>,
    status: Option<String>,
    dtstart: Option<Prop>,
    recurrence_id: Option<Prop>,
    rrule: Option<String>,
    exdates: Vec<Prop>,
    organizer: Option<Prop>,
    attendees: Vec<Prop>,
}

/// One buffered recurring series (master and overrides share a `UID`).
#[derive(Default)]
struct Series {
    /// Master: first instant, RRULE text, excluded instants, template, locator.
    master: Option<Master>,
    /// RECURRENCE-ID instants that an override replaces or cancels.
    replaced: HashSet<i64>,
    /// Usable overrides: (recurrence-id instant, meeting).
    overrides: Vec<(i64, RawInteraction)>,
}

struct Master {
    start: i64,
    tz: Tz,
    rrule: String,
    exdates: HashSet<i64>,
    template: RawInteraction,
    locator: String,
}

struct Parser<'o> {
    opts: &'o ImportOptions,
    now: i64,
    /// Component stack (upper-cased names).
    stack: Vec<String>,
    /// The VEVENT being read (only when the stack is `.., VEVENT`).
    event: Option<Event>,
    /// Calendar default zone for floating / all-day values.
    default_tz: Tz,
    /// Owner emails, lowercased.
    owner: BTreeSet<String>,
    self_candidates: Vec<String>,
    /// Recurring series by UID, in first-seen order.
    series: Vec<Series>,
    series_index: HashMap<String, usize>,
    events_read: u64,
}

impl<'o> Parser<'o> {
    fn new(opts: &'o ImportOptions, now: i64) -> Self {
        let owner = opts
            .me_identities
            .iter()
            .filter_map(|h| {
                let (k, v) = h.split_once(':')?;
                k.trim()
                    .eq_ignore_ascii_case("email")
                    .then(|| v.trim().to_lowercase())
            })
            .collect();
        Parser {
            opts,
            now,
            stack: Vec::new(),
            event: None,
            default_tz: opts.tz,
            owner,
            self_candidates: Vec::new(),
            series: Vec::new(),
            series_index: HashMap::new(),
            events_read: 0,
        }
    }

    fn content_line(&mut self, line: &str, sink: &mut dyn Sink, stats: &mut ImportStats) {
        let Some(prop) = parse_prop(line) else {
            return;
        };
        match prop.name.as_str() {
            "BEGIN" => {
                let comp = prop.value.trim().to_ascii_uppercase();
                if comp == "VEVENT" {
                    if self.event.is_some() && self.stack.last().is_some_and(|c| c == "VEVENT") {
                        // A VEVENT inside an unclosed VEVENT: the first is broken.
                        self.event = None;
                        self.stack.pop();
                        self.malformed(sink, stats);
                    }
                    if self.stack.last().is_some_and(|c| c == "VCALENDAR") {
                        self.event = Some(Event::default());
                    }
                }
                self.stack.push(comp);
                return;
            }
            "END" => {
                let comp = prop.value.trim().to_ascii_uppercase();
                if let Some(pos) = self.stack.iter().rposition(|c| *c == comp) {
                    self.stack.truncate(pos);
                }
                if comp == "VEVENT" {
                    if let Some(ev) = self.event.take() {
                        self.events_read += 1;
                        stats.records_read += 1;
                        let locator = format!("event {}", self.events_read);
                        self.event_done(ev, &locator, sink, stats);
                    }
                }
                return;
            }
            _ => {}
        }
        match self.stack.last().map(String::as_str) {
            Some("VCALENDAR") => self.calendar_prop(&prop),
            Some("VEVENT") => {
                if let Some(ev) = self.event.as_mut() {
                    event_prop(ev, prop);
                }
            }
            _ => {}
        }
    }

    fn calendar_prop(&mut self, prop: &Prop) {
        match prop.name.as_str() {
            "X-WR-TIMEZONE" => {
                if let Some(tz) = resolve_tzid(&prop.value) {
                    self.default_tz = tz;
                }
            }
            "X-WR-CALNAME" => {
                let v = prop.value.trim().to_lowercase();
                if looks_like_email(&v) {
                    let cand = format!("email:{v}");
                    if !self.self_candidates.contains(&cand) {
                        self.self_candidates.push(cand);
                    }
                    self.owner.insert(v);
                }
            }
            _ => {}
        }
    }

    fn malformed(&mut self, sink: &mut dyn Sink, stats: &mut ImportStats) {
        self.events_read += 1;
        stats.records_read += 1;
        let locator = format!("event {}", self.events_read);
        sink.skip(SkipReason::Malformed, &locator);
        stats.record_skip(SkipReason::Malformed);
    }

    fn skip(
        &self,
        reason: SkipReason,
        locator: &str,
        sink: &mut dyn Sink,
        stats: &mut ImportStats,
    ) {
        sink.skip(reason, locator);
        stats.record_skip(reason);
    }

    fn series_mut(&mut self, uid: &str) -> &mut Series {
        let idx = match self.series_index.get(uid) {
            Some(i) => *i,
            None => {
                self.series.push(Series::default());
                self.series_index
                    .insert(uid.to_string(), self.series.len() - 1);
                self.series.len() - 1
            }
        };
        &mut self.series[idx]
    }

    fn event_done(
        &mut self,
        ev: Event,
        locator: &str,
        sink: &mut dyn Sink,
        stats: &mut ImportStats,
    ) {
        let tz = self.default_tz;
        // An override (RECURRENCE-ID) always takes its occurrence out of the
        // master's expansion, whatever becomes of it.
        let recurrence = ev
            .recurrence_id
            .as_ref()
            .and_then(|p| parse_time(&p.value, p, tz));
        let series_key = ev.uid.clone().unwrap_or_else(|| format!("\0{locator}"));
        if let Some(rid) = recurrence {
            self.series_mut(&series_key).replaced.insert(rid);
        }
        let cancelled = ev
            .status
            .as_deref()
            .is_some_and(|s| s.trim().eq_ignore_ascii_case("CANCELLED"));
        if cancelled {
            self.skip(SkipReason::Unsupported, locator, sink, stats);
            return;
        }
        let Some(start_prop) = ev.dtstart.as_ref() else {
            self.skip(SkipReason::BadDate, locator, sink, stats);
            return;
        };
        let Some(start) = parse_time(&start_prop.value, start_prop, tz) else {
            self.skip(SkipReason::BadDate, locator, sink, stats);
            return;
        };
        let participants = match self.participants(&ev) {
            Ok(p) => p,
            Err(reason) => {
                self.skip(reason, locator, sink, stats);
                return;
            }
        };
        let meeting = RawInteraction {
            kind: InteractionKind::Meeting,
            at: start,
            dir_hint: Some(Direction::Mutual),
            subject: ev.summary.clone().filter(|s| !s.is_empty()),
            external_id: ev.uid.clone(),
            thread_key: ev.uid.clone(),
            list_unsub: false,
            participants,
        };
        if let Some(rid) = recurrence {
            let mut m = meeting;
            m.external_id = ev.uid.as_ref().map(|u| format!("{u}/{}", compact_utc(rid)));
            self.series_mut(&series_key).overrides.push((rid, m));
            return;
        }
        if let Some(rule) = ev.rrule.clone() {
            let series_tz = if start_prop.value.trim().ends_with(['Z', 'z']) {
                chrono_tz::UTC
            } else {
                start_prop
                    .param("TZID")
                    .and_then(resolve_tzid)
                    .unwrap_or(tz)
            };
            let exdates = ev
                .exdates
                .iter()
                .flat_map(|p| {
                    p.value
                        .split(',')
                        .filter_map(|v| parse_time(v, p, series_tz))
                        .collect::<Vec<_>>()
                })
                .collect();
            let master = Master {
                start,
                tz: series_tz,
                rrule: rule,
                exdates,
                template: meeting,
                locator: locator.to_string(),
            };
            self.series_mut(&series_key).master = Some(master);
            return;
        }
        sink.interaction(meeting);
    }

    /// Organizer first, then attendees; applies the skip rules.
    fn participants(&self, ev: &Event) -> Result<Vec<Participant>, SkipReason> {
        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let organizer = ev.organizer.as_ref().and_then(person_of);
        if let Some((email, hints)) = &organizer {
            seen.insert(email.clone());
            out.push(Participant {
                handles: vec![Handle::new(HandleKind::Email, email.clone())],
                role: Role::Organizer,
                hints: hints.clone(),
            });
        }
        let mut invited: HashSet<String> = HashSet::new();
        let mut others = 0usize;
        for a in &ev.attendees {
            let cutype = a.param("CUTYPE").unwrap_or("").to_ascii_uppercase();
            if cutype == "ROOM" || cutype == "RESOURCE" {
                continue;
            }
            let Some((email, hints)) = person_of(a) else {
                continue;
            };
            invited.insert(email.clone());
            let declined = a
                .param("PARTSTAT")
                .is_some_and(|s| s.eq_ignore_ascii_case("DECLINED"));
            if declined {
                if self.owner.contains(&email) {
                    return Err(SkipReason::Unsupported); // the owner declined
                }
                continue;
            }
            if !seen.insert(email.clone()) {
                continue;
            }
            others += 1;
            out.push(Participant {
                handles: vec![Handle::new(HandleKind::Email, email)],
                role: Role::Attendee,
                hints,
            });
        }
        if invited.len() > self.opts.ics_max_attendees as usize {
            return Err(SkipReason::Unsupported); // all-hands
        }
        let non_owner = out
            .iter()
            .any(|p| p.handles.iter().any(|h| !self.owner.contains(&h.raw)));
        if others == 0 || !non_owner {
            return Err(SkipReason::NoParticipants);
        }
        Ok(out)
    }

    /// Expand buffered series at end of input.
    fn finish(&mut self, sink: &mut dyn Sink, stats: &mut ImportStats) {
        if self.event.take().is_some() {
            self.malformed(sink, stats);
        }
        let series = std::mem::take(&mut self.series);
        for s in series {
            let mut instances: Vec<RawInteraction> = Vec::new();
            if let Some(m) = &s.master {
                let (starts, ok) = expand(m, self.now, &s.replaced, s.overrides.len());
                if !ok {
                    self.skip(SkipReason::Malformed, &m.locator, sink, stats);
                }
                if starts.is_empty() && s.overrides.iter().all(|(_, o)| o.at > self.now) {
                    self.skip(SkipReason::Unsupported, &m.locator, sink, stats);
                }
                for at in starts {
                    let mut i = m.template.clone();
                    i.at = at;
                    i.external_id = m
                        .template
                        .external_id
                        .as_ref()
                        .map(|u| format!("{u}/{}", compact_utc(at)));
                    instances.push(i);
                }
            }
            for (_, o) in s.overrides {
                if o.at <= self.now {
                    instances.push(o);
                }
            }
            instances.sort_by_key(|i| i.at);
            let drop = instances.len().saturating_sub(MAX_INSTANCES_PER_SERIES);
            for i in instances.into_iter().skip(drop) {
                sink.interaction(i);
            }
        }
        for c in std::mem::take(&mut self.self_candidates) {
            if !stats.self_candidates.contains(&c) {
                stats.self_candidates.push(c);
            }
        }
    }
}

fn event_prop(ev: &mut Event, prop: Prop) {
    match prop.name.as_str() {
        "UID" => {
            let v = prop.value.trim();
            if !v.is_empty() {
                ev.uid = Some(v.to_string());
            }
        }
        "SUMMARY" => ev.summary = Some(unescape_text(&prop.value)),
        "STATUS" => ev.status = Some(prop.value),
        "DTSTART" => ev.dtstart = Some(prop),
        "RECURRENCE-ID" => ev.recurrence_id = Some(prop),
        "RRULE" => {
            if ev.rrule.is_none() {
                ev.rrule = Some(prop.value);
            }
        }
        "EXDATE" => ev.exdates.push(prop),
        "ORGANIZER" => ev.organizer = Some(prop),
        "ATTENDEE" => ev.attendees.push(prop),
        // DESCRIPTION, LOCATION, COMMENT, CONFERENCE, X-GOOGLE-* ...: never read.
        _ => {}
    }
}

/// Email and name hints of an ORGANIZER / ATTENDEE value.
fn person_of(p: &Prop) -> Option<(String, Hints)> {
    let v = p.value.trim();
    let email = match v.get(..7) {
        Some(pre) if pre.eq_ignore_ascii_case("mailto:") => v[7..].trim().to_string(),
        _ => p.param("EMAIL").map(|e| e.trim().to_string())?,
    };
    let email = email.to_lowercase();
    if !looks_like_email(&email) {
        return None;
    }
    let name = p
        .param("CN")
        .map(unescape_text)
        .filter(|n| !n.is_empty() && !n.contains('@'));
    Some((
        email,
        Hints {
            name,
            ..Default::default()
        },
    ))
}

fn looks_like_email(s: &str) -> bool {
    match s.split_once('@') {
        Some((l, d)) => !l.is_empty() && d.contains('.') && !s.contains(char::is_whitespace),
        None => false,
    }
}

/// Past instance starts of a master, excluding `replaced` occurrences, the
/// most recent `MAX_INSTANCES_PER_SERIES + extra` of them. `false` when the
/// rule could not be read (then only the first occurrence is returned).
fn expand(m: &Master, now: i64, replaced: &HashSet<i64>, extra: usize) -> (Vec<i64>, bool) {
    let keep = |t: i64| t <= now && !m.exdates.contains(&t) && !replaced.contains(&t);
    let first_only = || {
        if keep(m.start) {
            vec![m.start]
        } else {
            Vec::new()
        }
    };
    let Some(set) = build_rrule(m) else {
        return (first_only(), false);
    };
    let cap = MAX_INSTANCES_PER_SERIES + extra;
    let mut window: VecDeque<i64> = VecDeque::with_capacity(cap + 1);
    for (n, dt) in set.into_iter().enumerate() {
        if n >= MAX_EXPANSION_STEPS {
            break;
        }
        let t = dt.timestamp();
        if t > now {
            break;
        }
        if !keep(t) {
            continue;
        }
        window.push_back(t);
        if window.len() > cap {
            window.pop_front();
        }
    }
    (window.into(), true)
}

/// Build the rrule set, with `UNTIL` converted to UTC ourselves (exports
/// write it as a date, floating or zoned time, which the RFC and the
/// `rrule` crate disagree on).
fn build_rrule(m: &Master) -> Option<rrule::RRuleSet> {
    let mut until: Option<i64> = None;
    let mut parts: Vec<&str> = Vec::new();
    for part in m.rrule.trim().split(';').filter(|p| !p.trim().is_empty()) {
        match part.split_once('=') {
            Some((k, v)) if k.trim().eq_ignore_ascii_case("UNTIL") => {
                let fake = Prop::default();
                let v = v.trim();
                until = Some(if v.len() == 8 {
                    // A DATE: through the end of that day.
                    parse_time(v, &fake, m.tz)? + 86_399
                } else {
                    parse_time(v, &fake, m.tz)?
                });
            }
            _ => parts.push(part),
        }
    }
    let rtz: rrule::Tz = m.tz.into();
    let start = DateTime::<Utc>::from_timestamp(m.start, 0)?.with_timezone(&rtz);
    let mut rule: rrule::RRule<rrule::Unvalidated> = parts.join(";").parse().ok()?;
    if let Some(u) = until {
        if u < m.start {
            return None;
        }
        let u = DateTime::<Utc>::from_timestamp(u, 0)?.with_timezone(&rrule::Tz::UTC);
        rule = rule.until(u);
    }
    rule.build(start).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_prop_handles_quotes_and_groups() {
        let p =
            parse_prop("item1.ATTENDEE;CN=\"Doe, Jane: CEO\";PARTSTAT=ACCEPTED:mailto:j@x.example")
                .unwrap();
        assert_eq!(p.name, "ATTENDEE");
        assert_eq!(p.param("CN"), Some("Doe, Jane: CEO"));
        assert_eq!(p.param("PARTSTAT"), Some("ACCEPTED"));
        assert_eq!(p.value, "mailto:j@x.example");
        assert!(parse_prop("no colon here").is_none());
    }

    #[test]
    fn text_unescaping() {
        assert_eq!(unescape_text("a\\, b\\; c\\\\d\\ne"), "a, b; c\\d e");
    }

    #[test]
    fn tzid_resolution() {
        assert_eq!(
            resolve_tzid("Europe/Berlin"),
            Some(chrono_tz::Europe::Berlin)
        );
        assert_eq!(
            resolve_tzid("\"Europe/Berlin\""),
            Some(chrono_tz::Europe::Berlin)
        );
        assert_eq!(
            resolve_tzid("/citadel.org/20190914_1/America/New_York"),
            Some(chrono_tz::America::New_York)
        );
        assert_eq!(
            resolve_tzid("America/Argentina/Buenos_Aires"),
            Some(chrono_tz::America::Argentina::Buenos_Aires)
        );
        assert_eq!(
            resolve_tzid("eastern standard time"),
            Some(chrono_tz::America::New_York)
        );
        assert_eq!(resolve_tzid("Customized Time Zone"), None);
    }

    #[test]
    fn compact_utc_format() {
        assert_eq!(compact_utc(0), "19700101T000000Z");
    }
}
