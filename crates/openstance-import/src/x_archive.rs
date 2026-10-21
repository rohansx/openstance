//! `x_archive` importer (Phase 3, feature `x-archive`): the X/Twitter
//! "download your data" archive. Spec: `docs/tech-spec/ingestion.md` 6.5.
//!
//! - Input: the archive `.zip`, the unzipped folder, or a single
//!   `data/*.js` file. Each data file is `window.YTD.<name>.partN = [ ... ]`;
//!   everything up to the first `=` is dropped and the array is read one
//!   element at a time (a byte-level splitter that tracks strings and
//!   nesting, then `serde_json` per element), so a large DM file is never
//!   held whole. Multi-part files (`following-part1.js`) are read in order.
//! - Zip safety: only the entries named below are opened, nothing is
//!   written to disk, and decompressed bytes are capped per entry and in
//!   total (an over-cap entry aborts the import).
//! - `manifest.js` / `account.js`: the owner's account id and username
//!   become self candidates (`x_user_id:<id>`, `x_handle:<name>`) and drive
//!   DM direction. `manifest.js`'s `generationDate` is the `at` of every
//!   follow (the archive has no follow dates); without it, the file's
//!   modification time.
//! - `following.js`: `follow` interaction, `dir_hint = Out`,
//!   `external_id = follow:out:<id>`, one participant (role `subject`) with
//!   an `XUserId` handle and no hints. `follower.js`: the same with `In` and
//!   `follow:in:<id>`. `subject` is always `None`.
//! - `direct-messages.js`: one `dm` per `messageCreate`, `external_id =
//!   dm:<message id>`, `thread_key` = conversation id, participants sender
//!   (role `sender`) and recipient (role `recipient`; taken from the
//!   conversation id `a-b` when missing). `direct-messages-group.js`: the
//!   sender plus every other member seen in the conversation
//!   (`participantsSnapshot`, `participantsJoin`, senders) with role
//!   `member`. `dir_hint` is `Out` for the owner's messages, `In` otherwise,
//!   `None` when the owner is unknown. Message text, reactions, media and
//!   conversation names are never read.
//! - The archive carries numeric ids only: no names, no handles for anyone
//!   but the owner. Follow-only ids stay dormant at resolution; ids link to
//!   people only through another source with the same id or a user edit.
//! - Skips: an item without a valid numeric id or with the wrong shape
//!   (`Malformed`), a message with an unreadable `createdAt` (`BadDate`).
//!   JSON that cannot be parsed at all is an error.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use chrono::DateTime;
use serde_json::Value;

use crate::zip_limits::Limited;
use crate::{
    sniff, Detection, Direction, Handle, HandleKind, Hints, ImportError, ImportOptions,
    ImportStats, Importer, InteractionKind, Participant, RawInteraction, Role, Sink, SkipReason,
};

/// Default cap on one decompressed archive entry.
pub const DEFAULT_MAX_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Default cap on all decompressed bytes read from one archive.
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// The data files this importer reads, in processing order.
const KINDS: [DataKind; 6] = [
    DataKind::Manifest,
    DataKind::Account,
    DataKind::Following,
    DataKind::Follower,
    DataKind::DirectMessages,
    DataKind::DirectMessagesGroup,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DataKind {
    Manifest,
    Account,
    Following,
    Follower,
    DirectMessages,
    DirectMessagesGroup,
}

impl DataKind {
    /// File stem in `data/` (`following` for `following.js` and
    /// `following-part1.js`).
    fn stem(self) -> &'static str {
        match self {
            DataKind::Manifest => "manifest",
            DataKind::Account => "account",
            DataKind::Following => "following",
            DataKind::Follower => "follower",
            DataKind::DirectMessages => "direct-messages",
            DataKind::DirectMessagesGroup => "direct-messages-group",
        }
    }

    /// The `window.YTD.<name>` name.
    fn ytd_name(self) -> &'static str {
        match self {
            DataKind::Manifest => "__THAR_CONFIG",
            DataKind::Account => "account",
            DataKind::Following => "following",
            DataKind::Follower => "follower",
            DataKind::DirectMessages => "direct_messages",
            DataKind::DirectMessagesGroup => "direct_messages_group",
        }
    }

    /// Classify a file name (`following.js`, `following-part2.js`).
    fn of_file_name(name: &str) -> Option<(DataKind, u32)> {
        let stem = name.strip_suffix(".js")?;
        let (base, part) = match stem.rsplit_once("-part") {
            Some((b, n)) if !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()) => {
                (b, n.parse().ok()?)
            }
            _ => (stem, 0),
        };
        KINDS
            .into_iter()
            .find(|k| k.stem() == base)
            .map(|k| (k, part))
    }

    /// Classify a single file by its `window.YTD.<name>.partN =` prefix.
    fn of_prefix(head: &[u8]) -> Option<DataKind> {
        let text = String::from_utf8_lossy(&head[..head.len().min(512)]);
        let t = text.trim_start_matches('\u{feff}').trim_start();
        if t.starts_with("window.__THAR_CONFIG") {
            return Some(DataKind::Manifest);
        }
        let rest = t.strip_prefix("window.YTD.")?;
        let name = rest.split(['.', ' ', '=']).next()?;
        KINDS.into_iter().find(|k| k.ytd_name() == name)
    }
}

/// X/Twitter archive importer.
#[derive(Debug, Clone)]
pub struct XArchiveImporter {
    opts: ImportOptions,
    max_entry: u64,
    max_total: u64,
}

impl XArchiveImporter {
    /// Construct with options from config.
    pub fn new(opts: ImportOptions) -> Self {
        XArchiveImporter {
            opts,
            max_entry: DEFAULT_MAX_ENTRY_BYTES,
            max_total: DEFAULT_MAX_TOTAL_BYTES,
        }
    }

    /// Override the decompressed size caps (per entry, whole archive).
    pub fn with_limits(mut self, max_entry: u64, max_total: u64) -> Self {
        self.max_entry = max_entry;
        self.max_total = max_total;
        self
    }

    /// The options this importer was built with.
    pub fn options(&self) -> &ImportOptions {
        &self.opts
    }
}

/// Where the data files come from.
enum Layout {
    Zip(PathBuf),
    Dir(PathBuf),
    File(PathBuf, DataKind),
}

/// `(kind, part, entry name or path)` for every relevant data file, sorted.
type Plan = Vec<(DataKind, u32, String)>;

fn plan_names<'a>(names: impl Iterator<Item = &'a str>) -> Plan {
    let mut plan: Plan = names
        .filter_map(|n| {
            let norm = n.replace('\\', "/");
            let mut segs = norm.rsplit('/');
            let file = segs.next()?;
            if segs.next()? != "data" {
                return None;
            }
            let (k, part) = DataKind::of_file_name(file)?;
            Some((k, part, n.to_string()))
        })
        .collect();
    plan.sort();
    plan
}

fn is_archive_plan(plan: &Plan) -> bool {
    plan.iter().any(|(k, _, _)| *k == DataKind::Manifest)
        && plan.iter().any(|(k, _, _)| {
            matches!(
                k,
                DataKind::Following
                    | DataKind::Follower
                    | DataKind::DirectMessages
                    | DataKind::DirectMessagesGroup
            )
        })
}

fn dir_plan(dir: &Path) -> Plan {
    let Ok(rd) = std::fs::read_dir(dir.join("data")) else {
        return Vec::new();
    };
    let names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| format!("data/{}", e.file_name().to_string_lossy()))
        .collect();
    plan_names(names.iter().map(String::as_str))
}

fn zip_plan(path: &Path) -> Option<Plan> {
    let f = File::open(path).ok()?;
    let z = zip::ZipArchive::new(f).ok()?;
    let names: Vec<String> = z
        .file_names()
        .collect::<Result<Vec<_>, _>>()
        .ok()?
        .into_iter()
        .map(|n| n.into_owned())
        .collect();
    Some(plan_names(names.iter().map(String::as_str)))
}

impl Importer for XArchiveImporter {
    fn kind(&self) -> &'static str {
        "x_archive"
    }

    /// Strong: a `.zip` (or folder) holding `data/manifest.js` and one of
    /// `following.js`, `follower.js`, `direct-messages.js`,
    /// `direct-messages-group.js`. Weak: a file
    /// starting with `window.YTD.`.
    fn detect(&self, path: &Path) -> Detection {
        if path.is_dir() {
            return if is_archive_plan(&dir_plan(path)) {
                Detection::Strong
            } else {
                Detection::No
            };
        }
        if !path.is_file() {
            return Detection::No;
        }
        let head = sniff(path);
        if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
            return match zip_plan(path) {
                Some(p) if is_archive_plan(&p) => Detection::Strong,
                _ => Detection::No,
            };
        }
        let body = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&head);
        let start = body
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(body.len());
        if body[start..].starts_with(b"window.YTD.") {
            Detection::Weak
        } else {
            Detection::No
        }
    }

    fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
        let started = std::time::Instant::now();
        let layout = if path.is_dir() {
            Layout::Dir(path.to_path_buf())
        } else {
            let head = sniff(path);
            if head.starts_with(b"PK") {
                Layout::Zip(path.to_path_buf())
            } else {
                match DataKind::of_prefix(&head) {
                    Some(k) => Layout::File(path.to_path_buf(), k),
                    None => {
                        return Err(ImportError::WrongFormat {
                            kind: "x_archive",
                            reason: "not an X archive zip, folder or window.YTD data file".into(),
                        })
                    }
                }
            }
        };
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        let mut st = State {
            follow_at: None,
            fallback_at: mtime,
            owner: None,
            stats: ImportStats::default(),
            seq: 0,
        };
        let total = Rc::new(Cell::new(0u64));
        match layout {
            Layout::Zip(p) => {
                let f = File::open(&p)?;
                let mut z = zip::ZipArchive::new(f).map_err(|e| ImportError::WrongFormat {
                    kind: "x_archive",
                    reason: format!("unreadable zip: {e}"),
                })?;
                let names: Vec<String> = z
                    .file_names()
                    .map(|n| n.map(|n| n.into_owned()))
                    .collect::<Result<_, _>>()
                    .map_err(|e| ImportError::WrongFormat {
                        kind: "x_archive",
                        reason: format!("unreadable zip entry name: {e}"),
                    })?;
                let plan = plan_names(names.iter().map(String::as_str));
                if !is_archive_plan(&plan) {
                    return Err(ImportError::WrongFormat {
                        kind: "x_archive",
                        reason: "zip has no data/manifest.js with follows or DMs".into(),
                    });
                }
                for (kind, _, name) in plan {
                    let entry = z.by_name(&name).map_err(|e| ImportError::Parse {
                        locator: name.clone(),
                        reason: e.to_string(),
                    })?;
                    let r = Limited::new(entry, self.max_entry, total.clone(), self.max_total);
                    st.read_file(kind, &name, r, sink)?;
                }
            }
            Layout::Dir(d) => {
                for (kind, _, name) in dir_plan(&d) {
                    let f = File::open(d.join(&name))?;
                    let r = Limited::new(f, self.max_entry, total.clone(), self.max_total);
                    st.read_file(kind, &name, r, sink)?;
                }
            }
            Layout::File(p, kind) => {
                let f = File::open(&p)?;
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let r = Limited::new(f, self.max_entry, total.clone(), self.max_total);
                st.read_file(kind, &name, r, sink)?;
            }
        }
        let mut stats = st.stats;
        stats.bytes = total.get();
        sink.progress(stats.bytes, None);
        stats.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(stats)
    }
}

// ---------------------------------------------------------------------------
// Streaming `window.YTD.x.part0 = [ ... ]`
// ---------------------------------------------------------------------------

/// Splits the top-level JSON array of a data file into element byte slices.
struct Elements<R> {
    r: BufReader<R>,
    started: bool,
    done: bool,
    locator: String,
}

impl<R: Read> Elements<R> {
    fn new(r: R, locator: &str) -> Self {
        Elements {
            r: BufReader::with_capacity(64 * 1024, r),
            started: false,
            done: false,
            locator: locator.to_string(),
        }
    }

    fn parse_err(&self, reason: &str) -> ImportError {
        ImportError::Parse {
            locator: self.locator.clone(),
            reason: reason.to_string(),
        }
    }

    fn next_byte(&mut self) -> Result<Option<u8>, ImportError> {
        let buf = self.r.fill_buf()?;
        let Some(&b) = buf.first() else {
            return Ok(None);
        };
        self.r.consume(1);
        Ok(Some(b))
    }

    /// Skip the `window.YTD... =` prefix up to and including `[`.
    fn start(&mut self) -> Result<(), ImportError> {
        let mut seen_eq = false;
        loop {
            match self.next_byte()? {
                None => return Err(self.parse_err("no JSON array")),
                Some(b'[') => return Ok(()),
                Some(b'=') if !seen_eq => seen_eq = true,
                Some(b) if seen_eq && !b.is_ascii_whitespace() => {
                    return Err(self.parse_err("expected a JSON array"))
                }
                Some(_) => {}
            }
        }
    }

    /// The next element's bytes, or `None` at the closing `]`.
    fn next_element(&mut self) -> Result<Option<Vec<u8>>, ImportError> {
        if self.done {
            return Ok(None);
        }
        if !self.started {
            self.start()?;
            self.started = true;
        }
        let mut out = Vec::new();
        let (mut depth, mut in_str, mut esc) = (0usize, false, false);
        loop {
            let Some(b) = self.next_byte()? else {
                return Err(self.parse_err("unexpected end of file"));
            };
            if in_str {
                out.push(b);
                if esc {
                    esc = false;
                } else if b == b'\\' {
                    esc = true;
                } else if b == b'"' {
                    in_str = false;
                }
                continue;
            }
            match b {
                b'"' => {
                    in_str = true;
                    out.push(b);
                }
                b'{' | b'[' => {
                    depth += 1;
                    out.push(b);
                }
                b'}' | b']' if depth > 0 => {
                    depth -= 1;
                    out.push(b);
                }
                b']' => {
                    self.done = true;
                    return Ok(if out.iter().all(u8::is_ascii_whitespace) {
                        None
                    } else {
                        Some(out)
                    });
                }
                b',' if depth == 0 => {
                    if out.iter().all(u8::is_ascii_whitespace) {
                        return Err(self.parse_err("empty array element"));
                    }
                    return Ok(Some(out));
                }
                b'}' => return Err(self.parse_err("unbalanced brackets")),
                _ => {
                    if !(depth == 0 && out.is_empty() && b.is_ascii_whitespace()) {
                        out.push(b);
                    }
                }
            }
        }
    }
}

/// Read a whole small file (manifest) and parse what follows the `=`.
fn read_assignment<R: Read>(mut r: R, locator: &str) -> Result<Value, ImportError> {
    let mut text = String::new();
    r.by_ref().take(4 * 1024 * 1024).read_to_string(&mut text)?;
    let body = text.split_once('=').map_or(text.as_str(), |(_, b)| b);
    let body = body.trim().trim_end_matches(';');
    serde_json::from_str(body).map_err(|e| ImportError::Parse {
        locator: locator.to_string(),
        reason: format!("invalid JSON at line {} column {}", e.line(), e.column()),
    })
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

struct State {
    /// manifest `generationDate`.
    follow_at: Option<i64>,
    /// File modification time.
    fallback_at: i64,
    owner: Option<String>,
    stats: ImportStats,
    seq: u64,
}

fn valid_id(v: Option<&Value>) -> Option<String> {
    let s = match v? {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit())).then_some(s)
}

fn parse_at(v: Option<&Value>) -> Option<i64> {
    let s = v?.as_str()?;
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|d| d.timestamp())
}

fn id_participant(id: &str, role: Role) -> Participant {
    Participant {
        handles: vec![Handle::new(HandleKind::XUserId, id)],
        role,
        hints: Hints::default(),
    }
}

/// Order ids numerically (shorter first), then by text.
fn id_order(a: &String, b: &String) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

impl State {
    fn skip(&mut self, sink: &mut dyn Sink, reason: SkipReason, locator: String) {
        sink.skip(reason, &locator);
        self.stats.record_skip(reason);
    }

    fn self_candidate(&mut self, c: String) {
        if !self.stats.self_candidates.contains(&c) {
            self.stats.self_candidates.push(c);
        }
    }

    fn owner_from(&mut self, id: Option<String>, name: Option<&str>) {
        if let Some(id) = id {
            self.self_candidate(format!("x_user_id:{id}"));
            if self.owner.is_none() {
                self.owner = Some(id);
            }
        }
        if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
            self.self_candidate(format!("x_handle:{n}"));
        }
    }

    fn direction(&self, sender: &str) -> Option<Direction> {
        self.owner.as_deref().map(|o| {
            if o == sender {
                Direction::Out
            } else {
                Direction::In
            }
        })
    }

    fn read_file<R: Read>(
        &mut self,
        kind: DataKind,
        name: &str,
        r: R,
        sink: &mut dyn Sink,
    ) -> Result<(), ImportError> {
        if kind == DataKind::Manifest {
            let v = read_assignment(r, name)?;
            let ui = v.get("userInfo");
            let id = valid_id(ui.and_then(|u| u.get("accountId")));
            let user = ui.and_then(|u| u.get("userName")).and_then(Value::as_str);
            self.owner_from(id, user);
            self.follow_at = parse_at(v.pointer("/archiveInfo/generationDate"));
            return Ok(());
        }
        let mut elems = Elements::new(r, name);
        while let Some(bytes) = elems.next_element()? {
            self.seq += 1;
            let locator = format!("{name} item {}", self.seq);
            let item: Value = serde_json::from_slice(&bytes).map_err(|e| ImportError::Parse {
                locator: locator.clone(),
                reason: format!("invalid JSON at line {} column {}", e.line(), e.column()),
            })?;
            match kind {
                DataKind::Manifest => {}
                DataKind::Account => {
                    let a = item.get("account");
                    let id = valid_id(a.and_then(|a| a.get("accountId")));
                    let user = a.and_then(|a| a.get("username")).and_then(Value::as_str);
                    self.owner_from(id, user);
                }
                DataKind::Following | DataKind::Follower => {
                    self.follow(kind, &item, locator, sink);
                }
                DataKind::DirectMessages | DataKind::DirectMessagesGroup => {
                    self.conversation(kind == DataKind::DirectMessagesGroup, &item, &locator, sink);
                }
            }
        }
        Ok(())
    }

    fn follow(&mut self, kind: DataKind, item: &Value, locator: String, sink: &mut dyn Sink) {
        self.stats.records_read += 1;
        let (key, dir, tag) = if kind == DataKind::Following {
            ("following", Direction::Out, "out")
        } else {
            ("follower", Direction::In, "in")
        };
        let Some(id) = valid_id(item.get(key).and_then(|f| f.get("accountId"))) else {
            self.skip(sink, SkipReason::Malformed, locator);
            return;
        };
        sink.interaction(RawInteraction {
            kind: InteractionKind::Follow,
            at: self.follow_at.unwrap_or(self.fallback_at),
            dir_hint: Some(dir),
            subject: None,
            external_id: Some(format!("follow:{tag}:{id}")),
            thread_key: None,
            list_unsub: false,
            participants: vec![id_participant(&id, Role::Subject)],
        });
    }

    fn conversation(&mut self, group: bool, item: &Value, locator: &str, sink: &mut dyn Sink) {
        let Some(conv) = item.get("dmConversation") else {
            self.stats.records_read += 1;
            self.skip(sink, SkipReason::Malformed, locator.to_string());
            return;
        };
        let conv_id = conv
            .get("conversationId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let empty = Vec::new();
        let messages = conv
            .get("messages")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        // Group members: snapshots, joins and senders.
        let mut members: BTreeSet<String> = BTreeSet::new();
        if group {
            for m in messages {
                if let Some(snap) = m
                    .pointer("/joinConversation/participantsSnapshot")
                    .and_then(Value::as_array)
                {
                    members.extend(snap.iter().filter_map(|v| valid_id(Some(v))));
                }
                if let Some(ids) = m
                    .pointer("/participantsJoin/userIds")
                    .and_then(Value::as_array)
                {
                    members.extend(ids.iter().filter_map(|v| valid_id(Some(v))));
                }
                members.extend(valid_id(m.pointer("/messageCreate/senderId")));
            }
        }
        let mut members: Vec<String> = members.into_iter().collect();
        members.sort_by(id_order);
        for (i, m) in messages.iter().enumerate() {
            let Some(mc) = m.get("messageCreate") else {
                continue;
            };
            self.stats.records_read += 1;
            let loc = format!("{locator} message {}", i + 1);
            let id = mc
                .get("id")
                .and_then(|v| match v {
                    Value::String(s) => Some(s.trim().to_string()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .filter(|s| !s.is_empty());
            let sender = valid_id(mc.get("senderId"));
            let (Some(id), Some(sender)) = (id, sender) else {
                self.skip(sink, SkipReason::Malformed, loc);
                continue;
            };
            let Some(at) = parse_at(mc.get("createdAt")) else {
                self.skip(sink, SkipReason::BadDate, loc);
                continue;
            };
            let mut participants = vec![id_participant(&sender, Role::Sender)];
            if group {
                for m in members.iter().filter(|m| **m != sender) {
                    participants.push(id_participant(m, Role::Member));
                }
            } else {
                let recipient = valid_id(mc.get("recipientId")).or_else(|| {
                    conv_id.as_deref().and_then(|c| {
                        c.split('-')
                            .find(|p| *p != sender)
                            .and_then(|p| valid_id(Some(&Value::String(p.to_string()))))
                    })
                });
                if let Some(r) = recipient {
                    participants.push(id_participant(&r, Role::Recipient));
                }
            }
            sink.interaction(RawInteraction {
                kind: InteractionKind::Dm,
                at,
                dir_hint: self.direction(&sender),
                subject: None,
                external_id: Some(format!("dm:{id}")),
                thread_key: conv_id.clone(),
                list_unsub: false,
                participants,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_classify() {
        assert_eq!(
            DataKind::of_file_name("following.js"),
            Some((DataKind::Following, 0))
        );
        assert_eq!(
            DataKind::of_file_name("direct-messages-group-part3.js"),
            Some((DataKind::DirectMessagesGroup, 3))
        );
        assert_eq!(DataKind::of_file_name("tweets.js"), None);
        assert_eq!(DataKind::of_file_name("following.json"), None);
    }

    #[test]
    fn prefixes_classify() {
        assert_eq!(
            DataKind::of_prefix(b"window.YTD.direct_messages.part0 = ["),
            Some(DataKind::DirectMessages)
        );
        assert_eq!(
            DataKind::of_prefix(b"\xEF\xBB\xBFwindow.YTD.follower.part2 = ["),
            Some(DataKind::Follower)
        );
        assert_eq!(DataKind::of_prefix(b"window.YTD.tweets.part0 = ["), None);
    }

    #[test]
    fn element_splitter_handles_strings_and_nesting() {
        let src = br#"window.YTD.x.part0 = [ {"a":"],\"}"}, [1,[2]] , "s,]" ]"#;
        let mut e = Elements::new(&src[..], "t");
        let mut got = Vec::new();
        while let Some(b) = e.next_element().unwrap() {
            got.push(String::from_utf8(b).unwrap().trim().to_string());
        }
        assert_eq!(got, [r#"{"a":"],\"}"}"#, "[1,[2]]", r#""s,]""#]);
        let mut empty = Elements::new(&b"x = [ ]"[..], "t");
        assert!(empty.next_element().unwrap().is_none());
        let mut bad = Elements::new(&b"x = { }"[..], "t");
        assert!(bad.next_element().is_err());
    }
}
