//! `linkedin_csv` importer (Phase 1, feature `linkedin`): `Connections.csv`,
//! `messages.csv`, `Invitations.csv`. Spec: `docs/tech-spec/ingestion.md` 6.2.
//!
//! - CSV quirks ported from v0.1: UTF-8 BOM, quoted commas, doubled quotes,
//!   CRLF, multiline values, a `Notes:` preamble of up to 10 records before
//!   the header, header aliases (`Email Address`/`E-mail Address`,
//!   `Position`/`Title`, ...; headers are compared lowercased with every
//!   non-alphanumeric character removed);
//! - Connections: one `Connection` interaction per row, `at` = Connected On
//!   (00:00 UTC), `dir_hint = Mutual`, external id `conn:<slug>`, one
//!   participant (role `Subject`) with `LinkedinUrl(URL)` and
//!   `Email(Email Address)` if non-empty, hints name/title/company;
//! - messages: one `Dm` per row, `thread_key` = CONVERSATION ID, external id
//!   `msg:<conv>:<date>:<sender slug>`, sender (role `Sender`) and each
//!   recipient (role `Recipient`); `dir_hint` Out for `FOLDER = SENT`, In for
//!   `INBOX`; CONTENT is never read; spam rows skipped (`Unsupported`); opaque
//!   member URLs (`ACoAA...`) joined to the slug of a sibling
//!   `Connections.csv` row only when the full name is unique there;
//! - Invitations: `Connection` kind, `subject = "invitation"`, `dir_hint` from
//!   Direction (INCOMING -> In, OUTGOING -> Out), one participant (role
//!   `Subject`): the other side of the invitation; external id
//!   `inv:<direction>:<slug>:<sent at>`; Message is never read;
//! - `default_label` = `linkedin <export date>` (export folder / zip name,
//!   else file mtime).

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime, TimeZone};

use crate::{
    sniff, Detection, Direction, Handle, HandleKind, Hints, ImportError, ImportOptions,
    ImportStats, Importer, InteractionKind, Participant, RawInteraction, Role, Sink, SkipReason,
};

/// Records allowed before the header row (the `Notes:` preamble).
const MAX_PREAMBLE_RECORDS: usize = 10;

/// Importer for the three LinkedIn export CSVs.
#[derive(Debug, Clone)]
pub struct LinkedinCsvImporter {
    opts: ImportOptions,
}

impl LinkedinCsvImporter {
    /// Construct with options from config.
    pub fn new(opts: ImportOptions) -> Self {
        LinkedinCsvImporter { opts }
    }

    /// The options this importer was built with.
    pub fn options(&self) -> &ImportOptions {
        &self.opts
    }
}

impl Importer for LinkedinCsvImporter {
    fn kind(&self) -> &'static str {
        "linkedin_csv"
    }

    /// Strong: header row (after an optional `Notes:` preamble) contains
    /// `First Name,Last Name` and `URL`, or `CONVERSATION ID` and
    /// `SENDER PROFILE URL`, or `Direction` and `inviterProfileUrl`.
    /// Weak: file name `Connections.csv`.
    fn detect(&self, path: &Path) -> Detection {
        if !path.is_file() {
            return Detection::No;
        }
        let buf = sniff(path);
        let body = buf.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&buf);
        let mut rdr = csv_reader(body);
        for rec in rdr.byte_records().take(MAX_PREAMBLE_RECORDS + 1) {
            let Ok(rec) = rec else { break };
            let fields: Vec<String> = rec
                .iter()
                .map(|f| String::from_utf8_lossy(f).into_owned())
                .collect();
            if FileKind::from_header(&fields).is_some() {
                return Detection::Strong;
            }
        }
        let is_connections = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("Connections.csv"));
        if is_connections {
            Detection::Weak
        } else {
            Detection::No
        }
    }

    fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
        let started = std::time::Instant::now();
        let file = File::open(path)?;
        let total = file.metadata().ok().map(|m| m.len());
        let mut rdr = csv_reader(open_without_bom(file)?);
        let mut stats = ImportStats::default();

        // Find the header row within the preamble allowance.
        let mut header: Option<(FileKind, Columns)> = None;
        let mut rec = csv::StringRecord::new();
        for _ in 0..=MAX_PREAMBLE_RECORDS {
            match rdr.read_record(&mut rec) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) if e.is_io_error() => return Err(into_io(e)),
                Err(_) => continue,
            }
            let fields: Vec<String> = rec.iter().map(str::to_string).collect();
            if let Some(kind) = FileKind::from_header(&fields) {
                header = Some((kind, Columns::new(&fields)));
                break;
            }
        }
        let Some((kind, cols)) = header else {
            return Err(ImportError::WrongFormat {
                kind: "linkedin_csv",
                reason: "no LinkedIn header row found in the first 11 records; expected \
                         `First Name`, `Last Name`, `URL` (Connections), `CONVERSATION ID`, \
                         `SENDER PROFILE URL` (messages) or `Direction`, `inviterProfileUrl` \
                         (Invitations)"
                    .into(),
            });
        };

        let joins = match kind {
            FileKind::Messages => sibling_connections(path),
            _ => HashMap::new(),
        };
        let mut row_mapper = RowMapper {
            opts: &self.opts,
            cols: &cols,
            joins: &joins,
            self_counts: BTreeMap::new(),
        };

        let mut row = 0u64;
        loop {
            match rdr.read_record(&mut rec) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) if e.is_io_error() => return Err(into_io(e)),
                Err(_) => {
                    row += 1;
                    stats.records_read += 1;
                    skip(sink, &mut stats, SkipReason::Malformed, row);
                    continue;
                }
            }
            if rec.len() == 1 && rec[0].trim().is_empty() {
                continue; // blank line
            }
            row += 1;
            stats.records_read += 1;
            let outcome = match kind {
                FileKind::Connections => row_mapper.connection(&rec),
                FileKind::Messages => row_mapper.message(&rec),
                FileKind::Invitations => row_mapper.invitation(&rec),
            };
            match outcome {
                Ok(i) => sink.interaction(i),
                Err(reason) => skip(sink, &mut stats, reason, row),
            }
            if row % 1000 == 0 {
                sink.progress(rdr.position().byte(), total);
            }
        }
        stats.bytes = rdr.position().byte();
        sink.progress(stats.bytes, total);

        let mut cands: Vec<(u64, String)> = row_mapper
            .self_counts
            .into_iter()
            .map(|(slug, n)| (n, slug))
            .collect();
        cands.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        stats.self_candidates = cands
            .into_iter()
            .map(|(_, slug)| format!("linkedin_url:{slug}"))
            .collect();
        stats.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(stats)
    }

    fn default_label(&self, path: &Path) -> Option<String> {
        let re = regex::Regex::new(r"(?i)LinkedInDataExport_(\d{2})-(\d{2})-(\d{4})").ok()?;
        let names = [path.file_name(), path.parent().and_then(|p| p.file_name())];
        for name in names.into_iter().flatten() {
            if let Some(c) = re.captures(&name.to_string_lossy()) {
                return Some(format!("linkedin {}-{}-{}", &c[3], &c[1], &c[2]));
            }
        }
        let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
        let dt: chrono::DateTime<chrono::Utc> = mtime.into();
        Some(format!("linkedin {}", dt.format("%Y-%m-%d")))
    }
}

fn skip(sink: &mut dyn Sink, stats: &mut ImportStats, reason: SkipReason, row: u64) {
    sink.skip(reason, &format!("row {row}"));
    stats.record_skip(reason);
}

fn into_io(e: csv::Error) -> ImportError {
    match e.into_kind() {
        csv::ErrorKind::Io(io) => ImportError::Io(io),
        other => ImportError::Parse {
            locator: "csv".into(),
            reason: format!("{other:?}"),
        },
    }
}

fn csv_reader<R: Read>(r: R) -> csv::Reader<R> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(r)
}

/// A 1 MiB buffered reader positioned after an optional UTF-8 BOM.
fn open_without_bom(file: File) -> std::io::Result<BufReader<File>> {
    let mut r = BufReader::with_capacity(1 << 20, file);
    let has_bom = r.fill_buf()?.starts_with(b"\xEF\xBB\xBF");
    if has_bom {
        r.consume(3);
    }
    Ok(r)
}

// ---------------------------------------------------------------------------
// Header aliases
// ---------------------------------------------------------------------------

/// Header name key: lowercase ASCII alphanumerics only
/// (`E-mail Address` -> `emailaddress`).
fn header_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Col {
    FirstName,
    LastName,
    Url,
    Email,
    Company,
    Position,
    ConnectedOn,
    ConversationId,
    ConversationTitle,
    From,
    SenderUrl,
    To,
    RecipientUrls,
    Date,
    Subject,
    Folder,
    SentAt,
    Direction,
    InviterUrl,
    InviteeUrl,
}

/// Alias table: normalized header key -> column. Seeded from the v0.1
/// prototype's `headerAliases` plus the observed LinkedIn variants.
const ALIASES: &[(&str, Col)] = &[
    ("firstname", Col::FirstName),
    ("givenname", Col::FirstName),
    ("lastname", Col::LastName),
    ("familyname", Col::LastName),
    ("surname", Col::LastName),
    ("url", Col::Url),
    ("profileurl", Col::Url),
    ("linkedinurl", Col::Url),
    ("publicprofileurl", Col::Url),
    ("emailaddress", Col::Email),
    ("email", Col::Email),
    ("email1value", Col::Email),
    ("company", Col::Company),
    ("companyname", Col::Company),
    ("organization", Col::Company),
    ("organisation", Col::Company),
    ("position", Col::Position),
    ("title", Col::Position),
    ("jobtitle", Col::Position),
    ("connectedon", Col::ConnectedOn),
    ("conversationid", Col::ConversationId),
    ("conversationtitle", Col::ConversationTitle),
    ("from", Col::From),
    ("senderprofileurl", Col::SenderUrl),
    ("to", Col::To),
    ("recipientprofileurls", Col::RecipientUrls),
    ("recipientprofileurl", Col::RecipientUrls),
    ("date", Col::Date),
    ("subject", Col::Subject),
    ("folder", Col::Folder),
    ("sentat", Col::SentAt),
    ("direction", Col::Direction),
    ("inviterprofileurl", Col::InviterUrl),
    ("inviteeprofileurl", Col::InviteeUrl),
];

/// Column positions resolved from a header row (first match wins).
struct Columns {
    pos: HashMap<Col, usize>,
    width: usize,
}

impl Columns {
    fn new(header: &[String]) -> Self {
        let mut pos = HashMap::new();
        for (i, h) in header.iter().enumerate() {
            let key = header_key(h);
            if let Some((_, col)) = ALIASES.iter().find(|(k, _)| *k == key) {
                pos.entry(*col).or_insert(i);
            }
        }
        Columns {
            pos,
            width: header.len(),
        }
    }

    fn has(&self, c: Col) -> bool {
        self.pos.contains_key(&c)
    }

    /// Trimmed cell, `""` when the column is absent or the row is short.
    fn get<'r>(&self, rec: &'r csv::StringRecord, c: Col) -> &'r str {
        self.pos
            .get(&c)
            .and_then(|&i| rec.get(i))
            .map(str::trim)
            .unwrap_or("")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Connections,
    Messages,
    Invitations,
}

impl FileKind {
    fn from_header(fields: &[String]) -> Option<FileKind> {
        let cols = Columns::new(fields);
        if cols.has(Col::FirstName) && cols.has(Col::LastName) && cols.has(Col::Url) {
            Some(FileKind::Connections)
        } else if cols.has(Col::ConversationId) && cols.has(Col::SenderUrl) {
            Some(FileKind::Messages)
        } else if cols.has(Col::Direction) && cols.has(Col::InviterUrl) {
            Some(FileKind::Invitations)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Row mapping
// ---------------------------------------------------------------------------

struct RowMapper<'a> {
    opts: &'a ImportOptions,
    cols: &'a Columns,
    /// Unique full-name key -> Connections profile URL (messages only).
    joins: &'a HashMap<String, Option<String>>,
    /// Owner slug -> how often it showed up as the owner's side.
    self_counts: BTreeMap<String, u64>,
}

impl RowMapper<'_> {
    fn connection(&mut self, rec: &csv::StringRecord) -> Result<RawInteraction, SkipReason> {
        let c = self.cols;
        if rec.len() < c.width {
            return Err(SkipReason::Malformed);
        }
        let url = c.get(rec, Col::Url);
        let email = c.get(rec, Col::Email);
        let mut handles = Vec::new();
        if !url.is_empty() {
            handles.push(Handle::new(HandleKind::LinkedinUrl, url));
        }
        if !email.is_empty() {
            handles.push(Handle::new(HandleKind::Email, email));
        }
        let at = parse_day(c.get(rec, Col::ConnectedOn)).ok_or(SkipReason::BadDate)?;
        if handles.is_empty() {
            return Err(SkipReason::NoParticipants);
        }
        let name = format!(
            "{} {}",
            c.get(rec, Col::FirstName),
            c.get(rec, Col::LastName)
        );
        let external_id = if url.is_empty() {
            Some(format!("conn:email:{}", email.to_lowercase()))
        } else {
            Some(format!("conn:{}", slug(url)))
        };
        Ok(RawInteraction {
            kind: InteractionKind::Connection,
            at,
            dir_hint: Some(Direction::Mutual),
            subject: None,
            external_id,
            thread_key: None,
            list_unsub: false,
            participants: vec![Participant {
                handles,
                role: Role::Subject,
                hints: Hints {
                    name: clean(&name),
                    title: clean(c.get(rec, Col::Position)),
                    company: clean(c.get(rec, Col::Company)),
                    location: None,
                },
            }],
        })
    }

    fn message(&mut self, rec: &csv::StringRecord) -> Result<RawInteraction, SkipReason> {
        let c = self.cols;
        let conv = c.get(rec, Col::ConversationId);
        let from = c.get(rec, Col::From);
        let sender_url = c.get(rec, Col::SenderUrl);
        let folder = c.get(rec, Col::Folder);
        if folder.eq_ignore_ascii_case("SPAM")
            || (sender_url.is_empty() && from.eq_ignore_ascii_case("LinkedIn Member"))
        {
            return Err(SkipReason::Unsupported);
        }
        if conv.is_empty() {
            return Err(SkipReason::Malformed);
        }
        let date_raw = c.get(rec, Col::Date);
        let at = parse_datetime(date_raw, self.opts.tz).ok_or(SkipReason::BadDate)?;

        let mut participants = Vec::new();
        if let Some(p) = self.dm_participant(sender_url, from, Role::Sender) {
            participants.push(p);
        }
        let urls: Vec<&str> = c
            .get(rec, Col::RecipientUrls)
            .split(|ch: char| ch.is_whitespace() || ch == ',')
            .filter(|s| !s.is_empty())
            .collect();
        let to = c.get(rec, Col::To);
        let names: Vec<&str> = if urls.len() == 1 {
            vec![to]
        } else {
            let split: Vec<&str> = to.split(',').map(str::trim).collect();
            if split.len() == urls.len() {
                split
            } else {
                vec![""; urls.len()]
            }
        };
        for (u, n) in urls.iter().zip(names) {
            if let Some(p) = self.dm_participant(u, n, Role::Recipient) {
                participants.push(p);
            }
        }
        if participants.is_empty() {
            return Err(SkipReason::NoParticipants);
        }

        let dir_hint = if folder.eq_ignore_ascii_case("SENT") {
            if !sender_url.is_empty() {
                *self.self_counts.entry(slug(sender_url)).or_insert(0) += 1;
            }
            Some(Direction::Out)
        } else if folder.eq_ignore_ascii_case("INBOX") {
            Some(Direction::In)
        } else {
            None
        };
        let subject =
            clean(c.get(rec, Col::Subject)).or_else(|| clean(c.get(rec, Col::ConversationTitle)));
        Ok(RawInteraction {
            kind: InteractionKind::Dm,
            at,
            dir_hint,
            subject,
            external_id: Some(format!("msg:{conv}:{date_raw}:{}", slug(sender_url))),
            thread_key: Some(conv.to_string()),
            list_unsub: false,
            participants,
        })
    }

    fn dm_participant(&self, url: &str, name: &str, role: Role) -> Option<Participant> {
        if url.is_empty() {
            return None;
        }
        let mut handles = vec![Handle::new(HandleKind::LinkedinUrl, url)];
        if is_opaque(url) {
            if let Some(Some(joined)) = self.joins.get(&name_key(name)) {
                handles.push(Handle::new(HandleKind::LinkedinUrl, joined.clone()));
            }
        }
        Some(Participant {
            handles,
            role,
            hints: Hints {
                name: clean(name),
                ..Default::default()
            },
        })
    }

    fn invitation(&mut self, rec: &csv::StringRecord) -> Result<RawInteraction, SkipReason> {
        let c = self.cols;
        let direction = c.get(rec, Col::Direction);
        let (dir, name, url, owner_url) = if direction.eq_ignore_ascii_case("OUTGOING") {
            (
                Direction::Out,
                c.get(rec, Col::To),
                c.get(rec, Col::InviteeUrl),
                c.get(rec, Col::InviterUrl),
            )
        } else if direction.eq_ignore_ascii_case("INCOMING") {
            (
                Direction::In,
                c.get(rec, Col::From),
                c.get(rec, Col::InviterUrl),
                c.get(rec, Col::InviteeUrl),
            )
        } else {
            return Err(SkipReason::Malformed);
        };
        let sent_at = c.get(rec, Col::SentAt);
        let at = parse_sent_at(sent_at, self.opts.tz).ok_or(SkipReason::BadDate)?;
        if url.is_empty() {
            return Err(SkipReason::NoParticipants);
        }
        if !owner_url.is_empty() {
            *self.self_counts.entry(slug(owner_url)).or_insert(0) += 1;
        }
        Ok(RawInteraction {
            kind: InteractionKind::Connection,
            at,
            dir_hint: Some(dir),
            subject: Some("invitation".into()),
            external_id: Some(format!(
                "inv:{}:{}:{sent_at}",
                direction.to_ascii_lowercase(),
                slug(url)
            )),
            thread_key: None,
            list_unsub: false,
            participants: vec![Participant {
                handles: vec![Handle::new(HandleKind::LinkedinUrl, url)],
                role: Role::Subject,
                hints: Hints {
                    name: clean(name),
                    ..Default::default()
                },
            }],
        })
    }
}

/// Collapse internal whitespace (multiline cells) and drop empty values.
fn clean(s: &str) -> Option<String> {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    (!joined.is_empty()).then_some(joined)
}

fn name_key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The `/in/<slug>` part of a profile URL, lowercased (falls back to the
/// whole trimmed URL). Only used to build stable external ids; the Sink does
/// the real normalization.
fn slug(url: &str) -> String {
    let lower = url.trim().to_lowercase();
    match lower.find("/in/") {
        Some(i) => lower[i + 4..]
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .to_string(),
        None => lower.trim_end_matches('/').to_string(),
    }
}

/// LinkedIn's opaque member ids (`ACoAAB...`) used in messages.csv.
fn is_opaque(url: &str) -> bool {
    slug(url).starts_with("acoa")
}

/// Unique full name -> profile URL from a `Connections.csv` next to `path`.
/// Names seen more than once map to `None` (never joined).
fn sibling_connections(path: &Path) -> HashMap<String, Option<String>> {
    let mut map = HashMap::new();
    let Some(dir) = path.parent() else {
        return map;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return map;
    };
    let conn = entries.flatten().map(|e| e.path()).find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("Connections.csv"))
    });
    let Some(conn) = conn else { return map };
    let Ok(file) = File::open(&conn) else {
        return map;
    };
    let Ok(r) = open_without_bom(file) else {
        return map;
    };
    let mut rdr = csv_reader(r);
    let mut cols: Option<Columns> = None;
    for (i, rec) in rdr.records().enumerate() {
        let Ok(rec) = rec else { continue };
        match &cols {
            None => {
                if i > MAX_PREAMBLE_RECORDS {
                    return map;
                }
                let fields: Vec<String> = rec.iter().map(str::to_string).collect();
                if FileKind::from_header(&fields) == Some(FileKind::Connections) {
                    cols = Some(Columns::new(&fields));
                }
            }
            Some(c) => {
                let url = c.get(&rec, Col::Url);
                if url.is_empty() {
                    continue;
                }
                let key = name_key(&format!(
                    "{} {}",
                    c.get(&rec, Col::FirstName),
                    c.get(&rec, Col::LastName)
                ));
                map.entry(key)
                    .and_modify(|v| *v = None)
                    .or_insert_with(|| Some(url.to_string()));
            }
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

/// `Connected On` (`12 Mar 2024`, plus a few export variants), 00:00 UTC.
fn parse_day(s: &str) -> Option<i64> {
    [
        "%d %b %Y",
        "%Y-%m-%d",
        "%b %d, %Y",
        "%d %B %Y",
        "%m/%d/%Y",
        "%m/%d/%y",
    ]
    .iter()
    .find_map(|f| NaiveDate::parse_from_str(s, f).ok())
    .and_then(|d| d.and_hms_opt(0, 0, 0))
    .map(|d| d.and_utc().timestamp())
}

/// messages.csv `DATE` (`2024-10-15 10:00:00 UTC`); naive values use `tz`.
fn parse_datetime(s: &str, tz: chrono_tz::Tz) -> Option<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S %z") {
        return Some(dt.timestamp());
    }
    for suffix in [" UTC", " GMT", "Z"] {
        if let Some(body) = s.strip_suffix(suffix) {
            return ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M"]
                .iter()
                .find_map(|f| NaiveDateTime::parse_from_str(body.trim(), f).ok())
                .map(|d| d.and_utc().timestamp());
        }
    }
    let naive = ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())?;
    local(naive, tz)
}

/// Invitations `Sent At` (`9/15/26, 10:00 AM`), in `tz`.
fn parse_sent_at(s: &str, tz: chrono_tz::Tz) -> Option<i64> {
    let naive = [
        "%m/%d/%y, %I:%M %p",
        "%m/%d/%Y, %I:%M %p",
        "%m/%d/%y, %H:%M",
        "%m/%d/%Y, %H:%M",
        "%Y-%m-%d %H:%M:%S",
    ]
    .iter()
    .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok());
    match naive {
        Some(n) => local(n, tz),
        None => parse_datetime(s, tz),
    }
}

fn local(n: NaiveDateTime, tz: chrono_tz::Tz) -> Option<i64> {
    tz.from_local_datetime(&n).earliest().map(|d| d.timestamp())
}
