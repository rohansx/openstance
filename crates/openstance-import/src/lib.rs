//! Importer and Sink traits, the raw record types importers emit, and the
//! built-in importers (each behind a cargo feature).
//!
//! An importer turns one file into a stream of [`RawIdentity`] and
//! [`RawInteraction`] records pushed into a [`Sink`]. It never touches the
//! database, never decides who is who, and never touches the network. The
//! authoritative contract is `docs/tech-spec/ingestion.md` section 3; these
//! traits are frozen at the end of Phase 1 (later changes need an ADR).
//!
//! # Writing an importer
//!
//! ```
//! use std::path::Path;
//! use openstance_import::*;
//!
//! /// A toy importer: every line of a text file is `email,name`.
//! struct Lines;
//!
//! impl Importer for Lines {
//!     fn kind(&self) -> &'static str { "lines" }
//!     fn detect(&self, path: &Path) -> Detection {
//!         if path.extension().is_some_and(|e| e == "lines") { Detection::Weak } else { Detection::No }
//!     }
//!     fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
//!         let mut stats = ImportStats::default();
//!         let text = std::fs::read_to_string(path)?;
//!         for (i, line) in text.lines().enumerate() {
//!             stats.records_read += 1;
//!             let Some((email, name)) = line.split_once(',') else {
//!                 sink.skip(SkipReason::Malformed, &format!("line {}", i + 1));
//!                 stats.record_skip(SkipReason::Malformed);
//!                 continue;
//!             };
//!             sink.identity(RawIdentity {
//!                 handle: Handle::new(HandleKind::Email, email),
//!                 hints: Hints { name: Some(name.to_string()), ..Default::default() },
//!                 record: format!("line:{}", i + 1),
//!                 observed_at: None,
//!             });
//!         }
//!         Ok(stats)
//!     }
//! }
//!
//! let dir = tempfile::tempdir().unwrap();
//! let path = dir.path().join("people.lines");
//! std::fs::write(&path, "ada@example.com,Ada Example\nbroken line\n").unwrap();
//!
//! let mut sink = MemorySink::default();
//! let stats = Lines.import(&path, &mut sink).unwrap();
//! assert_eq!(stats.records_read, 2);
//! assert_eq!(sink.identities.len(), 1);
//! assert_eq!(sink.skips.len(), 1);
//! ```
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[cfg(feature = "ics")]
pub mod ics;
#[cfg(feature = "linkedin")]
pub mod linkedin_csv;
#[cfg(feature = "mbox")]
pub mod mbox;
#[cfg(feature = "vcf")]
pub mod vcf;
#[cfg(feature = "x-archive")]
pub mod x_archive;
#[cfg(any(feature = "linkedin", feature = "x-archive"))]
mod zip_limits;

/// Bytes an importer's `detect` may read from the start of a file.
pub const DETECT_SNIFF_BYTES: usize = 64 * 1024;

/// Maximum stored subject length in characters (the Sink truncates).
pub const MAX_SUBJECT_CHARS: usize = 512;

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Turns one input file into raw records. Implementations must be pure
/// functions of the file and their [`ImportOptions`]: no network, no database.
pub trait Importer: Send + Sync {
    /// Stable id, stored in `source.kind`, e.g. `"linkedin_csv"`.
    fn kind(&self) -> &'static str;

    /// Cheap sniff so `openstance import <file>` needs no flag. Must read at
    /// most [`DETECT_SNIFF_BYTES`] and must not fail: I/O errors return
    /// [`Detection::No`].
    fn detect(&self, path: &Path) -> Detection;

    /// Stream records into the sink. Must not buffer the whole input.
    /// Returning `Err` rolls back the whole import.
    fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats, ImportError>;

    /// Bumped whenever the emitted records change for the same input.
    fn version(&self) -> u32 {
        1
    }

    /// Optional default `source.label` for this file (for example
    /// `"linkedin 2026-10-15"` derived from the export's zip name). `None`
    /// lets core derive `<kind> <yyyy-mm-dd>`.
    fn default_label(&self, _path: &Path) -> Option<String> {
        None
    }
}

/// How confident an importer is that it can read a file.
/// `Strong` beats `Weak`; two matches at the same level are an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Detection {
    /// Not this importer's format.
    No,
    /// Plausible, usually from the file name or extension only.
    Weak,
    /// Verified from the content.
    Strong,
}

/// Receives the records an importer emits. Core's implementation writes the
/// raw tables inside one transaction; [`MemorySink`] collects them for tests
/// and `--dry-run`.
pub trait Sink {
    /// A handle seen outside any event (vCard, CSV row). Identities with the
    /// same `record` key in one import are asserted to be the same human.
    fn identity(&mut self, r: RawIdentity);
    /// An event and its participants.
    fn interaction(&mut self, r: RawInteraction);
    /// Optional progress callback; the CLI renders rows/s and bytes.
    fn progress(&mut self, _bytes_done: u64, _bytes_total: Option<u64>) {}
    /// Record a skipped input unit with a reason (counted in
    /// [`ImportStats::skipped`]). `locator` names the unit (line, row,
    /// message number), never its contents.
    fn skip(&mut self, reason: SkipReason, locator: &str);
}

// ---------------------------------------------------------------------------
// Record types
// ---------------------------------------------------------------------------

/// The kind of a handle. Mirrors the `identity.kind` check constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HandleKind {
    /// Email address.
    Email,
    /// Phone number (normalized to E.164 by the Sink).
    Phone,
    /// LinkedIn profile URL (normalized to the slug by the Sink).
    LinkedinUrl,
    /// X / Twitter handle.
    XHandle,
    /// X numeric account id.
    XUserId,
    /// Instagram handle.
    IgHandle,
    /// `"<name>|<company>"` for rows with no other handle.
    NameCompany,
    /// `"<importer kind>|<name>"` for names that only exist in one importer's world.
    NameLocal,
}

impl HandleKind {
    /// Every kind, in `identity.kind` order.
    pub const ALL: [HandleKind; 8] = [
        HandleKind::Email,
        HandleKind::Phone,
        HandleKind::LinkedinUrl,
        HandleKind::XHandle,
        HandleKind::XUserId,
        HandleKind::IgHandle,
        HandleKind::NameCompany,
        HandleKind::NameLocal,
    ];

    /// The string stored in `identity.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            HandleKind::Email => "email",
            HandleKind::Phone => "phone",
            HandleKind::LinkedinUrl => "linkedin_url",
            HandleKind::XHandle => "x_handle",
            HandleKind::XUserId => "x_user_id",
            HandleKind::IgHandle => "ig_handle",
            HandleKind::NameCompany => "name_company",
            HandleKind::NameLocal => "name_local",
        }
    }

    /// Parse an `identity.kind` string.
    pub fn parse(s: &str) -> Option<HandleKind> {
        HandleKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

impl fmt::Display for HandleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An unnormalized handle as it appeared in the file. The Sink normalizes it
/// (entity-resolution.md section 2) and drops it if invalid.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Handle {
    /// What kind of handle this is.
    pub kind: HandleKind,
    /// The raw text from the file.
    pub raw: String,
}

impl Handle {
    /// Convenience constructor.
    pub fn new(kind: HandleKind, raw: impl Into<String>) -> Self {
        Handle {
            kind,
            raw: raw.into(),
        }
    }
}

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle")
            .field("kind", &self.kind)
            .field("raw", &REDACTED)
            .finish()
    }
}

/// What a record showed for a participant. Person fields are aggregated from
/// these at rebuild time.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Hints {
    /// Display name.
    pub name: Option<String>,
    /// Job title.
    pub title: Option<String>,
    /// Company name.
    pub company: Option<String>,
    /// Location (city, region).
    pub location: Option<String>,
}

impl Hints {
    /// True when no hint is set.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.title.is_none()
            && self.company.is_none()
            && self.location.is_none()
    }
}

impl fmt::Debug for Hints {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let show = |o: &Option<String>| if o.is_some() { REDACTED } else { "None" };
        f.debug_struct("Hints")
            .field("name", &show(&self.name))
            .field("title", &show(&self.title))
            .field("company", &show(&self.company))
            .field("location", &show(&self.location))
            .finish()
    }
}

/// A handle seen outside any event (one vCard, one CSV row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawIdentity {
    /// The handle.
    pub handle: Handle,
    /// Name, title, company, location shown with it.
    pub hints: Hints,
    /// Source reference: vCard UID or ordinal, CSV row number. Identities
    /// sharing a `record` in one import are one human.
    pub record: String,
    /// vCard `REV`, CSV "updated" column; `None` means import time.
    pub observed_at: Option<i64>,
}

/// Source reference of a record (vCard UID, CSV row number).
pub type SourceRef = String;

/// Interaction kinds an importer may emit. `contact` is Sink-internal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InteractionKind {
    /// An email message.
    Email,
    /// A calendar meeting.
    Meeting,
    /// A direct message.
    Dm,
    /// A public mention.
    Mention,
    /// A follow (social graph edge).
    Follow,
    /// A connection (LinkedIn) or invitation.
    Connection,
}

impl InteractionKind {
    /// The string stored in `interaction.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            InteractionKind::Email => "email",
            InteractionKind::Meeting => "meeting",
            InteractionKind::Dm => "dm",
            InteractionKind::Mention => "mention",
            InteractionKind::Follow => "follow",
            InteractionKind::Connection => "connection",
        }
    }
}

/// Direction of an interaction relative to the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    /// Someone else to the owner.
    In,
    /// The owner to someone else.
    Out,
    /// Both ways (a meeting, a connection).
    Mutual,
    /// Unknown or not applicable.
    None,
}

impl Direction {
    /// The string stored in `interaction.dir_hint` / `direction`.
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::In => "in",
            Direction::Out => "out",
            Direction::Mutual => "mutual",
            Direction::None => "none",
        }
    }
}

/// A participant's role in an interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Email `From`.
    From,
    /// Email `To`.
    To,
    /// Email `Cc`.
    Cc,
    /// Email `Bcc` (sent mail only).
    Bcc,
    /// Meeting attendee.
    Attendee,
    /// Meeting organizer.
    Organizer,
    /// DM sender.
    Sender,
    /// DM recipient.
    Recipient,
    /// Group DM member.
    Member,
    /// The person a contact / connection / follow row is about.
    Subject,
}

impl Role {
    /// The string stored in `interaction_person.role`.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::From => "from",
            Role::To => "to",
            Role::Cc => "cc",
            Role::Bcc => "bcc",
            Role::Attendee => "attendee",
            Role::Organizer => "organizer",
            Role::Sender => "sender",
            Role::Recipient => "recipient",
            Role::Member => "member",
            Role::Subject => "subject",
        }
    }
}

/// One human taking part in an interaction. Its index in
/// [`RawInteraction::participants`] becomes `interaction_person.slot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    /// More than one when the record says these handles are one human.
    pub handles: Vec<Handle>,
    /// Role in the interaction.
    pub role: Role,
    /// What the record showed for this participant.
    pub hints: Hints,
}

/// An event between the owner and one or more people.
#[derive(Clone, PartialEq, Eq)]
pub struct RawInteraction {
    /// Interaction kind.
    pub kind: InteractionKind,
    /// Unix seconds, UTC.
    pub at: i64,
    /// Only when the file itself says so.
    pub dir_hint: Option<Direction>,
    /// Subject or title only, never a body.
    pub subject: Option<String>,
    /// Message-ID, UID, ...; `None` lets the Sink synthesize one.
    pub external_id: Option<String>,
    /// Thread / conversation key.
    pub thread_key: Option<String>,
    /// `List-Unsubscribe`, `Precedence: bulk|list|junk`, or `Auto-Submitted`.
    pub list_unsub: bool,
    /// Participants in slot order.
    pub participants: Vec<Participant>,
}

impl fmt::Debug for RawInteraction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawInteraction")
            .field("kind", &self.kind)
            .field("at", &self.at)
            .field("dir_hint", &self.dir_hint)
            .field("subject", &self.subject.as_ref().map(|_| REDACTED))
            .field("external_id", &self.external_id.as_ref().map(|_| REDACTED))
            .field("thread_key", &self.thread_key.as_ref().map(|_| REDACTED))
            .field("list_unsub", &self.list_unsub)
            .field("participants", &self.participants)
            .finish()
    }
}

const REDACTED: &str = "[redacted]";

/// Counters an importer returns. Stored as `source.stats_json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportStats {
    /// Messages, rows, cards, events, lines read.
    pub records_read: u64,
    /// Interactions written (filled by the Sink side).
    pub interactions_new: u64,
    /// Already present (same source re-run or global id).
    pub interactions_dup: u64,
    /// New identity rows (filled by the Sink side).
    pub identities_new: u64,
    /// Identity observations, new or not (filled by the Sink side).
    pub identities_seen: u64,
    /// Reason (see [`SkipReason::as_str`]) -> count.
    pub skipped: BTreeMap<String, u64>,
    /// Bytes read.
    pub bytes: u64,
    /// Wall time in milliseconds.
    pub elapsed_ms: u64,
    /// Handles that look like the owner's (`"email:..."`, `"linkedin_url:..."`),
    /// most likely first. Proposed for `[me] identities` on first import.
    pub self_candidates: Vec<String>,
}

impl ImportStats {
    /// Increment the skip counter for `reason`.
    pub fn record_skip(&mut self, reason: SkipReason) {
        *self.skipped.entry(reason.as_str().to_string()).or_insert(0) += 1;
    }

    /// Total skipped units across reasons.
    pub fn skipped_total(&self) -> u64 {
        self.skipped.values().sum()
    }
}

/// Why an input unit was skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SkipReason {
    /// Unparseable date and no fallback.
    BadDate,
    /// No participant with a valid handle.
    NoParticipants,
    /// A handle failed normalization.
    InvalidHandle,
    /// Header block above the cap.
    HeaderTooLarge,
    /// Structurally broken record.
    Malformed,
    /// Recognized but deliberately not imported (spam, trash, chat logs, ...).
    Unsupported,
}

impl SkipReason {
    /// Stable snake_case key used in `ImportStats.skipped`.
    pub fn as_str(self) -> &'static str {
        match self {
            SkipReason::BadDate => "bad_date",
            SkipReason::NoParticipants => "no_participants",
            SkipReason::InvalidHandle => "invalid_handle",
            SkipReason::HeaderTooLarge => "header_too_large",
            SkipReason::Malformed => "malformed",
            SkipReason::Unsupported => "unsupported",
        }
    }
}

/// Errors that abort a whole import (and roll it back).
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// Reading the file failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not in the expected format at all.
    #[error("not a {kind} file: {reason}")]
    WrongFormat {
        /// Importer kind.
        kind: &'static str,
        /// Why (no contact data).
        reason: String,
    },
    /// A fatal parse error at a location (no contact data in `reason`).
    #[error("parse error at {locator}: {reason}")]
    Parse {
        /// Line / row / message number.
        locator: String,
        /// Why.
        reason: String,
    },
    /// The requested option is not supported by this build or release.
    #[error("not supported: {0}")]
    Unsupported(String),
    /// Cancelled by the user (Ctrl-C).
    #[error("import cancelled")]
    Cancelled,
}

/// Options importers receive at construction, from config and CLI flags.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// `[import] email_bodies` / `--bodies`. Not supported in Phase 1.
    pub email_bodies: bool,
    /// `[import] default_region`, ISO 3166 alpha-2, for phones without `+`.
    pub default_region: Option<String>,
    /// `--map` / `[import.csv.<name>]` (Phase 5 csv importer).
    pub csv_map: Option<ColumnMap>,
    /// `[import] timezone` for naive timestamps.
    pub tz: chrono_tz::Tz,
    /// mbox: also import messages labelled Spam/Trash (`--include-spam`).
    pub include_spam: bool,
    /// mbox: skip messages larger than this many bytes after the header.
    pub max_message_bytes: u64,
    /// `[import] ics_max_attendees`: calendar events with more attendees
    /// (all-hands) are skipped.
    pub ics_max_attendees: u32,
    /// `[me] identities` (`"email:me@example.com"`, ...). Importers use them
    /// only to recognize the owner's side of a record (for example a
    /// calendar event the owner declined); they never emit them.
    pub me_identities: Vec<String>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            email_bodies: false,
            default_region: None,
            csv_map: None,
            tz: chrono_tz::UTC,
            include_spam: false,
            max_message_bytes: 50 * 1024 * 1024,
            ics_max_attendees: 50,
            me_identities: Vec::new(),
        }
    }
}

/// Column map for the generic csv importer: target field -> source column.
pub type ColumnMap = BTreeMap<String, String>;

// ---------------------------------------------------------------------------
// Registry and detection
// ---------------------------------------------------------------------------

/// The importers compiled into this build, in detection order.
#[allow(clippy::vec_init_then_push)]
pub fn registry(opts: &ImportOptions) -> Vec<Box<dyn Importer>> {
    #[allow(unused_mut)]
    let mut v: Vec<Box<dyn Importer>> = Vec::new();
    #[cfg(feature = "mbox")]
    v.push(Box::new(mbox::MboxImporter::new(opts.clone())));
    #[cfg(feature = "linkedin")]
    v.push(Box::new(linkedin_csv::LinkedinCsvImporter::new(
        opts.clone(),
    )));
    #[cfg(feature = "vcf")]
    v.push(Box::new(vcf::VcfImporter::new(opts.clone())));
    #[cfg(feature = "ics")]
    v.push(Box::new(ics::IcsImporter::new(opts.clone())));
    #[cfg(feature = "x-archive")]
    v.push(Box::new(x_archive::XArchiveImporter::new(opts.clone())));
    let _ = opts;
    v
}

/// Why [`detect`] could not pick one importer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DetectError {
    /// No importer claimed the file.
    #[error("no importer recognizes this file; supported kinds: {}; use --as <kind>", supported.join(", "))]
    NoMatch {
        /// Kinds compiled into this build.
        supported: Vec<&'static str>,
    },
    /// Two importers claimed it at the same level.
    #[error("both {a} and {b} recognize this file; use --as <kind>")]
    Ambiguous {
        /// First claimant.
        a: &'static str,
        /// Second claimant.
        b: &'static str,
    },
}

/// Pick the single `Strong` match, else the single `Weak` match.
pub fn detect<'a>(
    importers: &'a [Box<dyn Importer>],
    path: &Path,
) -> Result<&'a dyn Importer, DetectError> {
    let results: Vec<(Detection, &'a dyn Importer)> = importers
        .iter()
        .map(|i| (i.detect(path), i.as_ref()))
        .collect();
    for level in [Detection::Strong, Detection::Weak] {
        let hits: Vec<&'a dyn Importer> = results
            .iter()
            .filter(|(d, _)| *d == level)
            .map(|(_, i)| *i)
            .collect();
        match hits.as_slice() {
            [] => continue,
            [one] => return Ok(*one),
            [a, b, ..] => {
                return Err(DetectError::Ambiguous {
                    a: a.kind(),
                    b: b.kind(),
                })
            }
        }
    }
    Err(DetectError::NoMatch {
        supported: importers.iter().map(|i| i.kind()).collect(),
    })
}

/// Find an importer by its kind (`--as <kind>`).
pub fn by_kind<'a>(importers: &'a [Box<dyn Importer>], kind: &str) -> Option<&'a dyn Importer> {
    importers
        .iter()
        .find(|i| i.kind() == kind)
        .map(|b| b.as_ref())
}

/// Read up to [`DETECT_SNIFF_BYTES`] from the start of a file, for `detect`.
/// I/O errors yield an empty buffer.
pub fn sniff(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut buf = Vec::with_capacity(4096);
    let _ = f.take(DETECT_SNIFF_BYTES as u64).read_to_end(&mut buf);
    buf
}

// ---------------------------------------------------------------------------
// MemorySink
// ---------------------------------------------------------------------------

/// A [`Sink`] that keeps everything in memory. Used by tests, golden
/// snapshots and `openstance import --dry-run`.
#[derive(Debug, Default, Clone)]
pub struct MemorySink {
    /// Every identity received, in order.
    pub identities: Vec<RawIdentity>,
    /// Every interaction received, in order.
    pub interactions: Vec<RawInteraction>,
    /// Every skip, in order.
    pub skips: Vec<(SkipReason, String)>,
    /// Last progress report.
    pub last_progress: Option<(u64, Option<u64>)>,
}

impl MemorySink {
    /// Skip counts by reason key, like [`ImportStats::skipped`].
    pub fn skip_counts(&self) -> BTreeMap<String, u64> {
        let mut m = BTreeMap::new();
        for (r, _) in &self.skips {
            *m.entry(r.as_str().to_string()).or_insert(0) += 1;
        }
        m
    }
}

impl Sink for MemorySink {
    fn identity(&mut self, r: RawIdentity) {
        self.identities.push(r);
    }
    fn interaction(&mut self, r: RawInteraction) {
        self.interactions.push(r);
    }
    fn progress(&mut self, bytes_done: u64, bytes_total: Option<u64>) {
        self.last_progress = Some((bytes_done, bytes_total));
    }
    fn skip(&mut self, reason: SkipReason, locator: &str) {
        self.skips.push((reason, locator.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Fake {
        kind: &'static str,
        level: Detection,
    }
    impl Importer for Fake {
        fn kind(&self) -> &'static str {
            self.kind
        }
        fn detect(&self, _path: &Path) -> Detection {
            self.level
        }
        fn import(&self, _path: &Path, _sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
            Ok(ImportStats::default())
        }
    }

    fn fakes(levels: &[(&'static str, Detection)]) -> Vec<Box<dyn Importer>> {
        levels
            .iter()
            .map(|(k, l)| Box::new(Fake { kind: k, level: *l }) as Box<dyn Importer>)
            .collect()
    }

    #[test]
    fn detect_prefers_strong_over_weak() {
        let reg = fakes(&[
            ("a", Detection::Weak),
            ("b", Detection::Strong),
            ("c", Detection::No),
        ]);
        assert_eq!(detect(&reg, &PathBuf::from("x")).unwrap().kind(), "b");
    }

    #[test]
    fn detect_single_weak_wins_when_no_strong() {
        let reg = fakes(&[("a", Detection::No), ("b", Detection::Weak)]);
        assert_eq!(detect(&reg, &PathBuf::from("x")).unwrap().kind(), "b");
    }

    #[test]
    fn detect_tie_is_an_error_naming_both() {
        let reg = fakes(&[("a", Detection::Strong), ("b", Detection::Strong)]);
        let err = detect(&reg, &PathBuf::from("x")).err().unwrap();
        assert_eq!(err, DetectError::Ambiguous { a: "a", b: "b" });
    }

    #[test]
    fn detect_no_match_lists_supported_kinds() {
        let reg = fakes(&[("mbox", Detection::No), ("vcf", Detection::No)]);
        let err = detect(&reg, &PathBuf::from("x")).err().unwrap();
        assert_eq!(
            err.to_string(),
            "no importer recognizes this file; supported kinds: mbox, vcf; use --as <kind>"
        );
    }

    #[test]
    fn by_kind_finds_importer() {
        let reg = fakes(&[("mbox", Detection::No), ("vcf", Detection::No)]);
        assert_eq!(by_kind(&reg, "vcf").unwrap().kind(), "vcf");
        assert!(by_kind(&reg, "ics").is_none());
    }

    #[test]
    fn memory_sink_collects_everything_in_order() {
        let mut s = MemorySink::default();
        s.identity(RawIdentity {
            handle: Handle::new(HandleKind::Phone, "+1 201 555 0100"),
            hints: Hints::default(),
            record: "card:1".into(),
            observed_at: Some(5),
        });
        s.interaction(RawInteraction {
            kind: InteractionKind::Email,
            at: 10,
            dir_hint: None,
            subject: Some("hello".into()),
            external_id: None,
            thread_key: None,
            list_unsub: false,
            participants: vec![Participant {
                handles: vec![Handle::new(HandleKind::Email, "a@example.com")],
                role: Role::From,
                hints: Hints::default(),
            }],
        });
        s.skip(SkipReason::BadDate, "message 3");
        s.skip(SkipReason::BadDate, "message 9");
        s.skip(SkipReason::Unsupported, "message 12");
        s.progress(10, Some(100));
        assert_eq!(s.identities.len(), 1);
        assert_eq!(s.interactions[0].participants[0].role, Role::From);
        assert_eq!(s.skip_counts().get("bad_date"), Some(&2));
        assert_eq!(s.skip_counts().get("unsupported"), Some(&1));
        assert_eq!(s.last_progress, Some((10, Some(100))));
    }

    #[test]
    fn debug_output_redacts_contact_data() {
        let i = RawInteraction {
            kind: InteractionKind::Email,
            at: 1,
            dir_hint: Some(Direction::In),
            subject: Some("Secret plans".into()),
            external_id: Some("<id@example.com>".into()),
            thread_key: None,
            list_unsub: false,
            participants: vec![Participant {
                handles: vec![Handle::new(HandleKind::Email, "jane@example.com")],
                role: Role::From,
                hints: Hints {
                    name: Some("Jane Example".into()),
                    ..Default::default()
                },
            }],
        };
        let dbg = format!("{i:?}");
        assert!(!dbg.contains("jane@example.com"), "{dbg}");
        assert!(!dbg.contains("Jane Example"), "{dbg}");
        assert!(!dbg.contains("Secret plans"), "{dbg}");
        assert!(!dbg.contains("id@example.com"), "{dbg}");
        assert!(dbg.contains("Email"));
    }

    #[test]
    fn stats_serialize_with_reason_keys() {
        let mut st = ImportStats::default();
        st.record_skip(SkipReason::BadDate);
        st.record_skip(SkipReason::BadDate);
        st.record_skip(SkipReason::HeaderTooLarge);
        assert_eq!(st.skipped_total(), 3);
        let v = serde_json::to_value(&st).unwrap();
        assert_eq!(v["skipped"]["bad_date"], 2);
        assert_eq!(v["skipped"]["header_too_large"], 1);
        assert_eq!(v["records_read"], 0);
    }

    #[test]
    fn string_forms_match_schema_check_constraints() {
        let kinds: Vec<_> = HandleKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "email",
                "phone",
                "linkedin_url",
                "x_handle",
                "x_user_id",
                "ig_handle",
                "name_company",
                "name_local"
            ]
        );
        assert_eq!(
            HandleKind::parse("linkedin_url"),
            Some(HandleKind::LinkedinUrl)
        );
        assert_eq!(HandleKind::parse("nope"), None);
        assert_eq!(Role::Subject.as_str(), "subject");
        assert_eq!(InteractionKind::Connection.as_str(), "connection");
        assert_eq!(Direction::Mutual.as_str(), "mutual");
    }

    #[test]
    fn sniff_reads_at_most_64k_and_survives_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big");
        std::fs::write(&p, vec![b'x'; DETECT_SNIFF_BYTES + 10]).unwrap();
        assert_eq!(sniff(&p).len(), DETECT_SNIFF_BYTES);
        assert!(sniff(&dir.path().join("missing")).is_empty());
    }
}
