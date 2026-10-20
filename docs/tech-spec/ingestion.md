# Ingestion

Status: build-ready. Traits frozen at the end of Phase 1.
Owner crate: `openstance-import` (trait definitions in `openstance-core::ingest` so core never depends on a concrete importer; built-in importers in `openstance-import`, each behind a cargo feature).

Related: [data-model.md](data-model.md) (tables written), [entity-resolution.md](entity-resolution.md) (runs after every import), [scoring.md](scoring.md) (bulk detection thresholds), [configuration.md](configuration.md) (`[import]`, `[me]`), [interfaces.md](interfaces.md) (`openstance import`, `POST /import`), [../architecture/data-flow.md](../architecture/data-flow.md), [../decisions/0004-export-first-ingestion.md](../decisions/0004-export-first-ingestion.md), [../decisions/0010-email-headers-only-by-default.md](../decisions/0010-email-headers-only-by-default.md), [../engineering/performance-targets.md](../engineering/performance-targets.md).

---

## 1. Contract in one paragraph

An importer turns a file into a stream of `RawIdentity` and `RawInteraction` records pushed into a `Sink`. It never reads or writes `person`, never decides who is who, and never touches the network. The Sink normalizes handles, dedupes, and writes the raw tables (`source`, `identity`, `interaction`, `interaction_person`) inside one transaction. When the importer returns, the same transaction runs incremental resolution, bulk detection, scoring and FTS maintenance for the people the import touched, then commits. A buggy importer can therefore be fixed and its source replaced (`openstance import <file> --replace <source_id>`) without corrupting anything derived.

```mermaid
sequenceDiagram
    participant CLI as openstance import
    participant Reg as Registry
    participant Imp as Importer
    participant Sink as DbSink
    participant DB as SQLite (one txn)
    participant Res as Resolution + scoring

    CLI->>Reg: detect(path) for each importer
    Reg-->>CLI: linkedin_csv
    CLI->>DB: BEGIN IMMEDIATE
    CLI->>DB: file_hash already in source? -> no-op, ROLLBACK
    CLI->>DB: INSERT source
    CLI->>Imp: import(path, &mut sink)
    loop streaming
        Imp->>Sink: identity(RawIdentity) / interaction(RawInteraction)
        Sink->>DB: normalize, dedupe, INSERT (batched prepared stmts)
    end
    Imp-->>CLI: ImportStats
    CLI->>Res: resolve_incremental(touched identities)
    Res->>DB: person, identity.person_id, ties, scores, person_fts
    CLI->>DB: UPDATE source.stats_json; COMMIT
    CLI->>DB: PRAGMA wal_checkpoint(TRUNCATE)
    CLI->>CLI: embed changed people (separate txns, if embedder configured)
```

---

## 2. Privacy rules every importer follows

| Rule | Enforcement |
|---|---|
| Headers and metadata only. Email bodies, DM text, chat message text, meeting descriptions and LinkedIn message content are **never** stored. | `RawInteraction` has no body field. Review checklist for new importers. |
| Bodies are opt-in (`[import] email_bodies = true` or `--bodies`) and only used to extract signature hints (title, company, location, phone), then dropped. | `mbox`/`imap` only. Not available in Phase 1 (`--bodies` prints "not supported yet"); Phase 3 stretch, not a Phase 3 exit criterion. |
| No network. | `openstance-import` has no HTTP client dependency; `cargo deny` bans `reqwest`/`hyper` in this crate. The `imap` importer is the single exception and lives behind its own feature with its own crate-level allowance. |
| Subjects are truncated to 512 characters; control characters stripped. | Sink. |
| Fixtures are synthetic. | [../engineering/testing-strategy.md](../engineering/testing-strategy.md#fixture-rules). |

---

## 3. Traits and record types

```rust
// openstance-core/src/ingest/mod.rs
use std::path::Path;

pub trait Importer: Send + Sync {
    /// Stable id, stored in source.kind. e.g. "linkedin_csv".
    fn kind(&self) -> &'static str;

    /// Cheap sniff so `openstance import <file>` needs no flag.
    /// Must read at most 64 KiB (or the zip central directory) and must not fail:
    /// I/O errors return Detection::No.
    fn detect(&self, path: &Path) -> Detection;

    /// Stream records into the sink. Must not buffer the whole input.
    /// Returning Err rolls back the whole import.
    fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats, ImportError>;

    /// Bumped whenever the emitted records change for the same input.
    fn version(&self) -> u32 { 1 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Detection { No, Weak, Strong }   // Strong beats Weak; ties are an error listing both

pub trait Sink {
    /// A handle seen outside any event (vCard, CSV row). Identities with the same
    /// `record` key in one import are asserted to be the same human.
    fn identity(&mut self, r: RawIdentity);
    /// An event and its participants.
    fn interaction(&mut self, r: RawInteraction);
    /// Optional progress callback; the CLI renders rows/s and bytes.
    fn progress(&mut self, _bytes_done: u64, _bytes_total: Option<u64>) {}
    /// Record a skipped input unit with a reason (counted in ImportStats.skipped).
    fn skip(&mut self, reason: SkipReason, locator: &str);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandleKind { Email, Phone, LinkedinUrl, XHandle, XUserId, IgHandle, NameCompany, NameLocal }

/// An unnormalized handle as it appeared in the file. The Sink normalizes it
/// (entity-resolution.md section 2) and drops it if invalid.
#[derive(Debug, Clone)]
pub struct Handle { pub kind: HandleKind, pub raw: String }

#[derive(Debug, Clone, Default)]
pub struct Hints {
    pub name: Option<String>,
    pub title: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RawIdentity {
    pub handle: Handle,
    pub hints: Hints,              // name hint, company hint, ...
    pub record: String,            // source_ref: vCard UID / CSV row number; groups handles of one card
    pub observed_at: Option<i64>,  // vCard REV, CSV "updated" column; else import time
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionKind { Email, Meeting, Dm, Mention, Follow, Connection }   // Contact is Sink-internal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction { In, Out, Mutual, None }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { From, To, Cc, Bcc, Attendee, Organizer, Sender, Recipient, Member, Subject }

#[derive(Debug, Clone)]
pub struct Participant {
    pub handles: Vec<Handle>,      // >1 when the record says these handles are one human
    pub role: Role,
    pub hints: Hints,              // e.g. email display name, LinkedIn Position/Company
}

#[derive(Debug, Clone)]
pub struct RawInteraction {
    pub kind: InteractionKind,
    pub at: i64,                          // unix seconds UTC
    pub dir_hint: Option<Direction>,      // only when the file itself says so
    pub subject: Option<String>,          // subject/title only, never a body
    pub external_id: Option<String>,      // Message-ID, UID; None => Sink synthesizes
    pub thread_key: Option<String>,
    pub list_unsub: bool,                 // List-Unsubscribe or Precedence: bulk|list|junk
    pub participants: Vec<Participant>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImportStats {
    pub records_read: u64,          // messages, rows, cards, events, lines
    pub interactions_new: u64,
    pub interactions_dup: u64,      // already present (same source re-run or global id)
    pub identities_new: u64,
    pub identities_seen: u64,
    pub skipped: std::collections::BTreeMap<String, u64>,   // reason -> count
    pub bytes: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum SkipReason { BadDate, NoParticipants, InvalidHandle, HeaderTooLarge, Malformed, Unsupported }
```

Importers get their options at construction from config, not through `import()`:

```rust
pub struct ImportOptions {
    pub email_bodies: bool,               // [import] email_bodies
    pub default_region: Option<String>,   // [import] default_region, for E.164 ("US", "IN")
    pub csv_map: Option<ColumnMap>,       // --map / [import.csv.<name>]
    pub tz: chrono_tz::Tz,                // [import] timezone for naive timestamps; default local
}

pub fn registry(opts: &ImportOptions) -> Vec<Box<dyn Importer>>;  // compiled-in importers, feature-gated
```

### What the Sink does with each record

| Record | Writes |
|---|---|
| `RawIdentity`s sharing one `record` key | One `interaction(kind='contact', at=observed_at or now, external_id='contact:'+record)` with one participant slot holding all the handles, role `subject`, hints on each row. Contact interactions carry no weight in scoring and never create ties; they exist so a vCard's "these three handles are one human" assertion and its hints have a raw home. |
| `RawInteraction` | One `interaction` row; for each participant `p` (slot = index) and each normalized handle `h` in `p.handles`: upsert `identity(kind, value)` (insert if new, `source_id` = this source), then insert `interaction_person(interaction_id, identity_id, role, slot, hints)`. |
| Invalid handle | Dropped, `skip(InvalidHandle)`. A participant with no valid handles is dropped; an interaction with no participants is skipped (`NoParticipants`). |

The Sink caches `(kind, value) -> identity.id` in a bounded LRU (100k entries, ~10 MB) to avoid a SELECT per participant; misses fall back to `INSERT ... ON CONFLICT(kind, value) DO NOTHING RETURNING id` / `SELECT id`.

---

## 4. Detection (`detect()` sniffing)

`openstance import <file...>` runs `detect()` of every registered importer on each path and picks the single `Strong` match, else the single `Weak` match. Zero matches: error listing supported kinds and `--as <kind>`. Two matches at the same level: error naming both and suggesting `--as`. Directories are walked one level deep (a Takeout folder, an unzipped LinkedIn export) and each file is detected on its own; files nobody claims are listed as ignored.

| Importer | Strong | Weak |
|---|---|---|
| `mbox` | First non-empty line starts with `From ` and the next 64 KiB contain a `Message-ID:` or `Date:` header | `.mbox` extension |
| `linkedin_csv` | CSV whose header row (after an optional `Notes:` preamble of up to 10 lines) contains `First Name,Last Name` and `URL` (Connections), or `CONVERSATION ID` and `SENDER PROFILE URL` (messages), or `Direction` and `inviterProfileUrl` (Invitations); or a `.zip` containing `Connections.csv` | `Connections.csv` file name |
| `x_archive` | `.zip` (or directory) containing `data/manifest.js` and any of `data/following.js`, `data/follower.js`, `data/direct-messages.js` | a `.js` file starting with `window.YTD.` |
| `vcf` | Starts with `BEGIN:VCARD` (after BOM/whitespace) | `.vcf` / `.vcard` |
| `ics` | Starts with `BEGIN:VCALENDAR` | `.ics` |
| `whatsapp` | `.txt` whose first 20 non-empty lines match the WhatsApp line regexes (section 6.7) in >= 80% of lines | file name `WhatsApp Chat with *.txt` or `_chat.txt` |
| `csv` | never Strong (generic) | `.csv`/`.tsv` with a recognizable name column (v0.1 header aliases) |
| `imap` | not file-based: `openstance import imap://user@host/[folder]` | |

A Google Takeout `.zip` is unpacked by streaming each matching member through the detected importer; nested `.mbox` members are imported as one source per member, labelled with the Takeout label (`All mail Including Spam and Trash.mbox` is skipped by default because it duplicates labelled mailboxes; `--include-all-mail` imports it instead of the labelled ones).

---

## 5. Dedupe

Three layers, from cheapest to finest:

1. **Same file.** `file_hash = 'blake3:' + hex(blake3(file bytes))`, computed streaming before the import starts (5 GB in ~3 s). If a `source` with that hash exists, print `already imported as source #3 "work gmail" (2026-10-12); nothing to do` and exit 0. `--replace <id>` is the only way to re-run a file (data-model.md section 8).
2. **Same record within a source kind.** `UNIQUE(source_id, external_id)` and `INSERT OR IGNORE`. `external_id` per importer:

| Importer | `external_id` |
|---|---|
| mbox / imap | `Message-ID` (angle brackets stripped, lowercased). Missing: synthesized. |
| linkedin_csv | Connections: `conn:` + slug. messages: `msg:` + CONVERSATION ID + `:` + DATE + `:` + sender slug. Invitations: `inv:` + direction + slug + `:` + Sent At. |
| x_archive | DM: `dm:` + message id. Follow: `follow:in:` / `follow:out:` + account id. |
| vcf | `contact:` + `UID`, else synthesized from the card's sorted normalized handles. |
| ics | `UID` + `/` + `RECURRENCE-ID` (or `DTSTART` for expanded instances). |
| whatsapp | synthesized (one interaction per chat, sender and day, section 6.7). |
| csv | `contact:` + row's id column if mapped, else synthesized from sorted normalized handles. |

   Synthesized ids: `syn:` + first 16 hex of `blake3(kind | at | sorted normalized handles | subject)`. Stable across re-exports of the same data, so `--replace` and new exports dedupe correctly.

3. **Same event across sources.** For `email` and `meeting`, `external_id` is globally meaningful, so before inserting, the Sink checks `interaction_global_id (kind, external_id)` across all sources and skips duplicates (`interactions_dup`). A Takeout export followed by IMAP sync, or two Takeout exports with overlapping date ranges, do not double-count. Other kinds are not globally deduped (two LinkedIn exports are supposed to keep their own connection rows; the diff between them is the `watch` playbook).

---

## 6. Importers

Phases follow [../plan/roadmap.md](../plan/roadmap.md).

| Importer | Phase | Feature flag | Input | Emits |
|---|---|---|---|---|
| `mbox` | 1 | `mbox` (default) | Google Takeout / any mboxo/mboxrd file, `.zip` Takeout | email interactions, email identities |
| `linkedin_csv` | 1 | `linkedin` (default) | `Connections.csv`, `messages.csv`, `Invitations.csv`, or the export `.zip` | linkedin_url (+ email) identities, connection + dm interactions |
| `vcf` | 1 | `vcf` (default) | Apple / Google Contacts `.vcf` (3.0, 4.0) | contact records: email, phone, linkedin/x/ig handles, name/title/company hints |
| `ics` | 3 | `ics` (default) | Calendar `.ics` export | meeting interactions with attendees |
| `x_archive` | 3 | `x` (default) | X/Twitter archive `.zip` | x_user_id identities, follow + dm interactions |
| `csv` | 5 (P5-IMP-08; good first issue, may be pulled earlier) | `csv` (default) | any CSV/TSV + column map | contact records (and optional interactions) |
| `whatsapp` | 5 | `whatsapp` | chat export `.txt` / `.zip` | dm interactions keyed by phone or local name |
| `imap` | 5 | `imap` (off by default) | IMAP server, app password | same as mbox, incremental by UID |
| pst, eml, slack, discord, telegram, notes (Granola/Obsidian/Apple Notes) | 5 | one each | see [../plan/phase-5-and-beyond.md](../plan/phase-5-and-beyond.md) | |

### 6.1 `mbox` (Phase 1)

- **Parsing.** Read with a 1 MiB `BufReader`; split messages on lines starting with `From ` at a line start (after `\n`), accepting both mboxo and mboxrd (`>From ` unescaping is irrelevant because bodies are skipped). Parse the header block only (until the first empty line) with `mail-parser`'s header API; cap a header block at 256 KiB (`HeaderTooLarge`). Skip the body by scanning for the next `\nFrom ` with `memchr::memmem`, never buffering it.
- **Headers read:** `From`, `To`, `Cc`, `Bcc` (present in sent mail), `Date`, `Subject`, `Message-ID`, `In-Reply-To`, `References`, `List-Unsubscribe`, `List-Id`, `Precedence`, `X-Gmail-Labels`, `X-GM-THRID`, `Auto-Submitted`.
- **Decoding:** RFC 2047 encoded words in names and subjects, RFC 5322 address lists including groups (`undisclosed-recipients:;` yields no participants), quoted display names, comments.
- **Mapping:**

| Field | From |
|---|---|
| `kind` | `email` |
| `at` | `Date` (RFC 2822, with common malformations: missing seconds, named zones, two-digit years); fallback to the `From ` line date; else `skip(BadDate)` |
| `subject` | `Subject`, decoded, `Re:`/`Fwd:`/`AW:`/`SV:` prefixes kept (shown as evidence as the user saw it) |
| `external_id` | `Message-ID` |
| `thread_key` | `X-GM-THRID` if present, else the first id in `References`, else `In-Reply-To`, else own `Message-ID` |
| `dir_hint` | `out` if `X-Gmail-Labels` contains `Sent`; else none (direction is derived from self identities at resolution) |
| `list_unsub` | `List-Unsubscribe` present, or `Precedence` in (bulk, list, junk), or `Auto-Submitted` other than `no` |
| participants | `From` -> role `from`; `To` -> `to`; `Cc` -> `cc`; `Bcc` -> `bcc`; each address is one participant with one `Email` handle and the display name as `hints.name` |

- **Skips:** `X-Gmail-Labels` containing `Spam` or `Trash` (counted, configurable with `--include-spam`); `Chat` labelled items (Hangouts logs) skipped.
- **Self inference:** the importer counts `From` addresses on `Sent`-labelled messages and reports the top one in `ImportStats` (`self_candidates`); the CLI proposes it for `[me] identities` on first import.
- **Bodies (opt-in, Phase 3 stretch):** with `email_bodies = true`, for messages **from** non-self people only, read up to 8 KiB of the `text/plain` part, take the block after `-- ` or the last 12 lines, and extract title/company/phone/location with regexes plus the domain map. Hints go to the `from` participant's `hints`; the body is discarded. Never applied to bulk senders.
- **Target:** 5 GB mbox in under 2 minutes, RSS under 500 MB ([../engineering/performance-targets.md](../engineering/performance-targets.md)).

### 6.2 `linkedin_csv` (Phase 1)

All three files may come as separate CSVs or in the export zip. Each import of an export is its own `source`, labelled `linkedin <export date>` (export date from the zip name `Basic_LinkedInDataExport_10-15-2026.zip` or file mtime). CSV parsing ports the v0.1 prototype's tested behavior: UTF-8 BOM, quoted commas, doubled quotes, CRLF, multiline values, and a `Notes:` preamble before the header (see [../prototype-v0.1.md](../prototype-v0.1.md)).

| File | Columns used | Emits |
|---|---|---|
| `Connections.csv` | `First Name`, `Last Name`, `URL`, `Email Address`, `Company`, `Position`, `Connected On` (`15 Oct 2024`) | `connection` interaction, `at` = Connected On (00:00 UTC), `dir_hint=mutual`, one participant role `subject` with handles `LinkedinUrl(URL)` and `Email(Email Address)` if non-empty, hints name = First + Last, title = Position, company = Company. `Connected On` is **not** last contact (scoring treats `connection` as a weak channel signal only; see [scoring.md](scoring.md)). |
| `messages.csv` | `CONVERSATION ID`, `CONVERSATION TITLE`, `FROM`, `SENDER PROFILE URL`, `TO`, `RECIPIENT PROFILE URLS`, `DATE` (`2024-10-15 10:00:00 UTC`), `SUBJECT`, `FOLDER` | `dm` interaction per message row, `thread_key` = CONVERSATION ID, `subject` = SUBJECT if non-empty else CONVERSATION TITLE, participants sender (role `sender`) and each recipient (role `recipient`; `RECIPIENT PROFILE URLS` is space- or comma-separated). `CONTENT` is never read. `FOLDER = INBOX/SENT` informs `dir_hint` only when the export owner is known. Sponsored/InMail spam (`FOLDER = SPAM` or sender name `LinkedIn Member` with no URL) is skipped. |
| `Invitations.csv` | `From`, `To`, `Sent At`, `Direction` (`INCOMING`/`OUTGOING`), `inviterProfileUrl`, `inviteeProfileUrl` | `connection`-kind interaction with `subject = 'invitation'`, `dir_hint` from Direction. `Message` is never read. |
| `Profile.csv` (if present) | `First Name`, `Last Name`, public profile URL | Not emitted as a person; reported as the self LinkedIn identity candidate. |

LinkedIn quirks the importer must handle:

- **Opaque member URLs** in messages (`https://www.linkedin.com/in/ACoAAB...`) do not match the vanity slugs in Connections. Within one export the importer builds a `full name -> slug` map from Connections and, when a message participant has an opaque URL and their exact full name maps to exactly one connection, adds that slug as a second handle in the same participant slot (an importer-asserted join, unique names only). Otherwise the opaque id is kept as its own `linkedin_url` identity and resolution handles it by name rules.
- Header names vary across export years and locales (`Email Address` vs `E-mail Address`, `Position` vs `Title`); a static alias table (seeded from the v0.1 `headerAliases`) maps them.
- Empty `Email Address` is the norm (members opt out of sharing).

Target: 5k connection rows in under 2 s.

### 6.3 `vcf` (Phase 1)

- vCard 2.1, 3.0 and 4.0; line unfolding (CRLF + space/tab), `QUOTED-PRINTABLE` + `CHARSET` (2.1 and Apple exports), `ENCODING=b` photos skipped without decoding, grouped properties (`item1.URL`, `item1.X-ABLabel`).
- One card = one `record` (key `UID` or the card's ordinal), all handles emitted as `RawIdentity` with the same `record`, so they land in one `contact` interaction slot.

| Property | Handle / hint |
|---|---|
| `FN` (else `N` reassembled given + family) | hints.name |
| `EMAIL` (all) | `Email` |
| `TEL` (all) | `Phone`, parsed to E.164 with `ImportOptions.default_region`; unparseable numbers skipped with `InvalidHandle` |
| `ORG` (first component) | hints.company |
| `TITLE` | hints.title |
| `ADR` locality / region | hints.location (`locality, region`) |
| `URL`, `X-SOCIALPROFILE`, `IMPP` | `LinkedinUrl` for linkedin.com/in, `XHandle` for x.com / twitter.com, `IgHandle` for instagram.com; other URLs ignored |
| `REV` | `observed_at` |
| `NOTE`, `BDAY`, `PHOTO`, `CATEGORIES` | ignored (user-private, not needed for search) |

- Cards with neither a handle nor a name are skipped. Cards with only a name produce a `NameLocal("vcf|<name>")` handle so the contact is still a searchable person.
- Phone is the strong join key for WhatsApp later.

### 6.4 `ics` (Phase 3)

- RFC 5545 via the `icalendar` parser; unfolding, `TZID` resolution with bundled tz data, all-day events (`VALUE=DATE`, `at` = 00:00 in `ImportOptions.tz`).
- One `meeting` interaction per `VEVENT` instance: `subject` = `SUMMARY`; `at` = `DTSTART`; `external_id` = `UID/RECURRENCE-ID`; participants: `ORGANIZER` (role `organizer`), each `ATTENDEE` (role `attendee`) with `mailto:` -> `Email`, `CN` -> hints.name.
- **Recurrence:** `RRULE` is expanded only for instances in the past and at most 104 instances per series (two years of weekly), since each past instance is evidence of a real meeting; future instances are ignored.
- Skipped: events with `STATUS:CANCELLED`, attendees with `PARTSTAT=DECLINED`, events with no attendee other than the organizer, events with more than `[import] ics_max_attendees` (default 50) attendees (all-hands), resource calendars (`CUTYPE=ROOM|RESOURCE`).
- `DESCRIPTION`, `LOCATION` (often a Zoom link), conferencing data: never read.

### 6.5 `x_archive` (Phase 3)

- Input: the archive zip (or unzipped folder). Each `data/*.js` file is `window.YTD.<name>.part0 = [ ... ]`; strip everything up to the first `=` and parse JSON with `serde_json` streaming (`StreamDeserializer` over array elements).

| File | Fields | Emits |
|---|---|---|
| `data/account.js` | `accountId`, `username`, `accountDisplayName` | self candidates (`XUserId`, `XHandle`) |
| `data/following.js` | `following.accountId`, `following.userLink` | `follow` interaction, `dir_hint=out`, participant role `subject` with `XUserId` |
| `data/follower.js` | `follower.accountId` | `follow` interaction, `dir_hint=in` |
| `data/direct-messages.js` | `dmConversation.conversationId`, `messages[].messageCreate.{id, senderId, recipientId, createdAt}` | `dm` interaction per message, `thread_key` = conversationId; `text` never read |
| `data/direct-messages-group.js` | same + group members | `dm` with role `member` for others |

- `at` for follows: the archive has no follow date; use the archive generation date from `manifest.js` and mark `subject = NULL`. Scoring treats `follow` as a breadth signal only.
- **Honest limit:** the archive contains numeric account ids, not handles or names. An `x_user_id` identity attaches to a person only through another source that carries the same id or through enrichment (Phase 3+, opt-in, quoted). Follow-only ids with no name stay as **dormant** identities: resolution does not create a person for an identity cluster that has no name hint and only `follow` interactions. They are counted in `ImportStats` and `openstance source show` ("812 X accounts not yet linked to anyone").

### 6.6 `csv` (Phase 5, may be pulled earlier)

- Any CSV/TSV with a column map. Default auto-map uses the v0.1 prototype aliases (`name/full name/display name`, `first name`+`last name`, `email/email address/e-mail 1 - value`, `company/organization/organization 1 - name`, `title/position/job title`, `location/city`, `url/linkedin url/profile url`, `phone`, `notes/bio`, `tags`).
- Explicit map: `openstance import attendees.csv --map name="Full Name",email=Email,company=Org,tags=Track` or a named map in config `[import.csv.luma]`. Unknown target names are an error.
- Each row is one `record` -> one `contact` interaction. Optional `--as-event "Devtools dinner 2026-09-12"` instead emits one `meeting` interaction (`subject` = the event name, `at` = the given date) with every row as an attendee: this is how event attendee lists become evidence ("who was at the devtools dinner").
- Rows with only a name and company emit `NameCompany("<name>|<company>")`; name only: `NameLocal("csv|<name>")`.
- `tags` and `notes` columns become hints only on the CSV itself; they are **not** turned into overrides (overrides are user edits made in openstance). They feed FTS `bio` only when mapped to `bio`.
- Limits inherited from v0.1: cells capped at 10,000 chars; no row limit (streaming).

### 6.7 `whatsapp` (Phase 5)

- Input: `WhatsApp Chat with <name>.txt` or the `.zip` with media (media members ignored). Line formats (locale-dependent), all supported:
  - iOS: `[15/10/2024, 10:00:00] Jane Doe: text`
  - Android: `15/10/24, 10:00 - Jane Doe: text`
  - 12-hour variants with `AM/PM`, and `MM/DD/YY` for US locales (disambiguated per file by checking which interpretation keeps all dates valid and monotonic).
- Continuation lines (no timestamp prefix) belong to the previous message and are ignored. System lines ("Messages and calls are end-to-end encrypted", "X added Y") are skipped.
- Aggregation: one `dm` interaction per (chat, sender, calendar day), `at` = first message that day, so a 200-message day is one unit of evidence, not 200. Group chats (more than two distinct senders): role `member`, `thread_key` = file name hash.
- Sender handles: a sender shown as a phone number (`+91 98765 43210`) -> `Phone` (E.164); a saved-contact name -> `NameLocal("whatsapp|<name>")`. Names resolve to people only when a vcf supplies a matching name **and** phone, or by user merge; see [entity-resolution.md](entity-resolution.md).
- Message text is never stored; `subject` is NULL.

### 6.8 `imap` (Phase 5)

- `openstance import imap://user@imap.example.com/INBOX --password-cmd "security find-generic-password -s ost-imap -w"`; app passwords only, no OAuth in v1. TLS required (`rustls`).
- Fetches `BODY.PEEK[HEADER.FIELDS (FROM TO CC BCC DATE SUBJECT MESSAGE-ID IN-REPLY-TO REFERENCES LIST-UNSUBSCRIBE PRECEDENCE)]` in batches of 500 UIDs. Same mapping as mbox.
- Incremental: the source row stores `UIDVALIDITY` and the highest UID in `stats_json`; a re-run creates a **new** source covering only new UIDs (each sync is a source, `file_hash` NULL). Global Message-ID dedupe makes overlapping runs harmless.

---

## 7. Transactions, WAL, streaming and memory limits

- Connection pragmas: `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`, `busy_timeout=5000` ([data-model.md](data-model.md#7-connection-settings-and-transactions)).
- **One import = one `BEGIN IMMEDIATE ... COMMIT`.** Raw inserts, incremental resolution, scoring and FTS updates for touched people all commit together. Any `Err` from the importer, a panic (caught with `catch_unwind` at the CLI boundary), or Ctrl-C (signal handler sets a cancel flag the Sink checks every 1,000 records) rolls back. Nothing partial is ever visible.
- Multiple files in one command: each file is its own transaction, in the order given; a failure stops the run and reports which files were committed.
- Embedding of changed people runs after commit, in batches of 256 per transaction, interruptible; an interrupted embed resumes on the next command because `text_hash` differs.
- After commit: `PRAGMA wal_checkpoint(TRUNCATE)`; on imports over 100k interactions, `PRAGMA optimize`.
- **Write path:** prepared statements cached per connection (`prepare_cached`), inserts batched implicitly by the single transaction. Indexes stay in place (dropping and recreating them costs more than it saves at 500k rows).
- **Memory budget** (5 GB mbox, target RSS under 500 MB):

| Component | Bound |
|---|---|
| Read buffer | 1 MiB |
| Current header block | 256 KiB max |
| Identity LRU cache | 100k entries (~10 MB) |
| SQLite page cache | 64 MB (`cache_size=-65536`) + mmap (does not count as anonymous RSS) |
| Touched-identity set for incremental resolution | `u32` ids in a `roaring` bitmap (< 1 MB for 50k) |
| Zip members | streamed via `zip::read::ZipFile`, never extracted to disk |

- Importers must not collect records into a `Vec` over the whole file; the review checklist and a test that imports a 1 GB generated mbox under a `ulimit -v`-style allocator cap (`cap` crate in tests, 300 MB) enforce it.

---

## 8. Bulk-sender detection

Runs as part of resolution/scoring on touched people (and fully on rebuild), not inside importers. Importers only record the raw signals (`list_unsub`, `dir_hint`, roles). A person gets `is_bulk = 1`, `score = 0`, all components 0, and is excluded from search (unless `--include-bulk`) and from ties, when **any** of:

| Signal | Rule |
|---|---|
| Volume without reply | >= 50 interactions where they are `from`/`sender`, and zero interactions with `direction='out'` addressed to them, and zero replies in either direction (no shared `thread_key` with an out message) |
| List headers | >= 3 messages from them and > 50% of them have `list_unsub = 1` |
| No-reply local part | any of their email identities matches `^(no-?reply|do-?not-?reply|notifications?|mailer-daemon|bounce[s]?|newsletter|news|updates|info|marketing|support)([+._-].*)?@` and there is no `out` interaction to them |
| Role-only | only ever appears in `from` on messages with `Auto-Submitted` (counted via `list_unsub`) |

Thresholds live in config `[score.bulk]` ([configuration.md](configuration.md)); `openstance edit <p> --not-bulk` writes an `is_bulk = '0'` override, which always wins. Rationale and the effect on score: [scoring.md](scoring.md). The Phase 1 fixture includes one bulk sender and one busy two-way correspondent that must **not** be flagged.

---

## 9. CLI surface

```
openstance import <path...> [--as <kind>] [--label <text>] [--replace <source_id>]
                            [--bodies] [--map k=v,...] [--as-event "<name> <date>"]
                            [--include-spam] [--include-all-mail] [--dry-run] [--json]
openstance source list [--json]
openstance source show <id>
openstance source rm <id>
```

`--dry-run` runs the importer into a `MemorySink` and prints `ImportStats` plus the first 10 parsed interactions without writing. Example output:

```
$ openstance import ~/Downloads/takeout-2026-10.zip ~/Downloads/Connections.csv
mbox          work gmail (Inbox, Sent)   412,233 messages  38.2s  10,791/s
              skipped: 1,204 spam/trash, 37 bad date, 2 header too large
              self: rohan@example.com (from Sent) -> added to [me] identities
linkedin_csv  linkedin 2026-10-15        4,812 connections, 3,140 messages  0.9s
resolution    +6,912 people, 214 merged by email, 31 flagged (name+company 22, name+thread 9)
              run `openstance review` to confirm or split flagged merges
```

The HTTP equivalent is `POST /import` (multipart) and returns the same stats JSON ([interfaces.md](interfaces.md)).

---

## 10. How to write a new importer

1. **Pick a kind** (`snake_case`, e.g. `telegram_json`) and add a cargo feature of the same name to `crates/openstance-import/Cargo.toml`, plus to the `openstance` binary's feature list.
2. **Create** `crates/openstance-import/src/importers/telegram_json.rs` implementing `Importer`. Keep it a pure function of the file: no network, no config reads beyond `ImportOptions`, no access to the database.
3. **Detect conservatively.** Return `Strong` only on content you can verify in the first 64 KiB; extension-only matches are `Weak`.
4. **Map to records:** one `RawInteraction` per real event, participants with handles and role, hints only for what the file actually says. Pick a stable `external_id` (or let the Sink synthesize). Never put a body or message text in `subject`.
5. **Stream.** Read incrementally; call `sink.progress()` periodically and `sink.skip()` for every dropped unit with a reason.
6. **Register** in `registry()` behind `#[cfg(feature = "telegram_json")]`.
7. **Fixtures:** generate synthetic input from `fixtures/persona/people.toml` via `cargo xtask gen-fixtures` into `fixtures/telegram_json/`, with at least one overlap with an existing fixture source (same phone or email) and one malformed record. Fictional domains and phone ranges only.
8. **Tests** (in `crates/openstance-import/tests/telegram_json.rs`):
   - `detect` is `Strong` on the fixture and `No` on every other importer's fixtures (the shared `detect_matrix` test does this automatically once registered).
   - Golden `ImportStats` and golden first-N records (`insta` snapshot of `MemorySink` contents).
   - Re-import is a no-op; `--replace` yields identical raw rows.
   - Resolution counts in `fixtures/EXPECTED.md` updated deliberately.
   - Fuzz target `fuzz/fuzz_targets/import_telegram_json.rs` seeded with the fixture (no panics, bounded memory).
9. **Docs:** add a row to the table in section 6 of this file and a subsection with the exact files and fields read and what is ignored.
10. **Privacy review checklist** (PR template): no body/text stored; subjects only; no network; skips are counted; large inputs stream; fixture contains no real data.

Minimal example:

```rust
pub struct LumaCsv;

impl Importer for LumaCsv {
    fn kind(&self) -> &'static str { "luma_csv" }

    fn detect(&self, path: &Path) -> Detection {
        match sniff_csv_header(path) {
            Ok(h) if h.contains("api_id") && h.contains("approval_status") => Detection::Strong,
            _ => Detection::No,
        }
    }

    fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats, ImportError> {
        let mut stats = ImportStats::default();
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(path)?;
        let hdr = rdr.headers()?.clone();
        let (name, email) = (col(&hdr, "name")?, col(&hdr, "email")?);
        let mut participants = Vec::new();
        for (i, row) in rdr.records().enumerate() {
            let row = match row { Ok(r) => r, Err(_) => { sink.skip(SkipReason::Malformed, &format!("row {i}")); continue } };
            stats.records_read += 1;
            participants.push(Participant {
                handles: vec![Handle { kind: HandleKind::Email, raw: row[email].to_string() }],
                role: Role::Attendee,
                hints: Hints { name: Some(row[name].to_string()), ..Default::default() },
            });
        }
        sink.interaction(RawInteraction {
            kind: InteractionKind::Meeting, at: event_time(path)?, dir_hint: Some(Direction::Mutual),
            subject: Some(event_title(path)), external_id: None, thread_key: None,
            list_unsub: false, participants,
        });
        Ok(stats)
    }
}
```

---

## 11. Acceptance criteria

Phase 1 (mbox, linkedin_csv, vcf):

- [ ] Each Phase 1 fixture is detected `Strong` by exactly one importer; an unknown file errors with the supported kinds list.
- [ ] Importing a file twice leaves all row counts unchanged and prints "already imported".
- [ ] An importer error or panic midway leaves the database byte-identical to before (rollback test).
- [ ] mbox: 5 GB generated mbox imports in under 2 min with RSS under 500 MB (perf job, not per-PR CI); spam/trash skipped and counted; RFC 2047 subjects decoded; Takeout + overlapping second mbox does not double-count (global Message-ID dedupe).
- [ ] linkedin_csv: 5k rows in under 2 s; preamble/BOM/CRLF/multiline/quoted-comma cases from v0.1 pass; `Connected On` never sets `last_contact`; opaque message URLs joined to slugs only when the name is unique.
- [ ] vcf: a card with two emails and a phone yields one person; the fixture vcf merges with mbox people by email at confidence 1.0; a phone-only card yields a person with a `phone` identity.
- [ ] No table contains any body or message text from fixtures (test greps the db for sentinel strings planted in fixture bodies).
- [ ] The fixture bulk sender is flagged; the busy two-way correspondent is not.

Phase 3 (ics, x_archive): recurrence expansion cap, declined/cancelled skipped, dormant X ids not shown as people, `--as-event` creates one meeting with N attendees. Phase 5 (csv: column maps and v0.1 header aliases; whatsapp, imap): locale date formats, per-day aggregation, incremental UID sync.
