//! The user's follow-through workspace (P4, schema v4): a shortlist of
//! people worth a conversation and a hand-kept introductions board, plus
//! the read-only numbers the web UI's dashboard shows (`GET /stats`) and the
//! source list (`GET /sources`).
//!
//! Both tables are user-authored, like `override`: `rebuild` never touches
//! them. Rows store the anchor identity id the user picked and are resolved
//! through `identity.person_id` on every read, so merges and splits keep
//! them attached to the right person. Nothing here sends anything anywhere.
//! Spec: `docs/tech-spec/data-model.md` sections 4.11 and 4.12,
//! `docs/tech-spec/interfaces.md` section 3.4.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::json::{iso, iso_opt, PersonRef, V};
use crate::resolve::lookup::{resolve_person, PersonKey};
use crate::search::person::person_ref;
use crate::PersonId;

pub mod edits;

#[cfg(test)]
mod tests;

/// Longest note accepted, in characters.
pub const MAX_NOTE_CHARS: usize = 4000;

// ---------------------------------------------------------------------------
// Shortlist
// ---------------------------------------------------------------------------

/// One shortlist row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortlistEntry {
    /// The person's current id.
    pub person_id: PersonId,
    /// The person (null when the person no longer exists, e.g. after a
    /// source was removed).
    pub person: Option<PersonRef>,
    /// The user's note.
    pub note: Option<String>,
    /// When it was added.
    pub added_at: String,
}

/// `GET /shortlist`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortlistResponse {
    /// Schema version.
    pub v: u32,
    /// Newest first.
    pub shortlist: Vec<ShortlistEntry>,
}

/// `POST /shortlist`, `PATCH /shortlist/{person}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortlistEntryResponse {
    /// Schema version.
    pub v: u32,
    /// The row after the change.
    pub entry: ShortlistEntry,
}

/// Every shortlist row, newest first (one row per current person).
pub fn shortlist(conn: &Connection) -> Result<ShortlistResponse> {
    let mut st = conn.prepare_cached(
        "SELECT s.person_id, i.person_id, s.note, s.added_at
         FROM shortlist s LEFT JOIN identity i ON i.id = s.person_id
         ORDER BY s.added_at DESC, s.person_id DESC",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for (stored, current, note, added_at) in rows {
        let id = current.unwrap_or(stored);
        if !seen.insert(id) {
            continue;
        }
        out.push(ShortlistEntry {
            person_id: id,
            person: person_ref(conn, id)?,
            note,
            added_at: iso(added_at),
        });
    }
    Ok(ShortlistResponse {
        v: V,
        shortlist: out,
    })
}

/// The current person `id` must exist.
fn require_person(conn: &Connection, id: PersonId, what: &str) -> Result<PersonRef> {
    person_ref(conn, id)?.ok_or_else(|| Error::NotFound(format!("{what} {id}")))
}

fn check_note(note: Option<&str>) -> Result<Option<String>> {
    match note {
        Some(n) if n.chars().count() > MAX_NOTE_CHARS => Err(Error::BadRequest(format!(
            "note is longer than {MAX_NOTE_CHARS} characters"
        ))),
        Some(n) if n.trim().is_empty() => Ok(None),
        Some(n) => Ok(Some(n.to_string())),
        None => Ok(None),
    }
}

/// The stored shortlist key of a current person (any identity of theirs).
fn shortlist_key(conn: &Connection, person: PersonId) -> Result<Option<i64>> {
    Ok(conn
        .prepare_cached(
            "SELECT s.person_id FROM shortlist s JOIN identity i ON i.id = s.person_id
             WHERE i.person_id = ?1 ORDER BY s.added_at DESC LIMIT 1",
        )?
        .query_row([person], |r| r.get(0))
        .optional()?
        .or(conn
            .prepare_cached("SELECT person_id FROM shortlist WHERE person_id = ?1")?
            .query_row([person], |r| r.get(0))
            .optional()?))
}

fn shortlist_entry(conn: &Connection, person: PersonId) -> Result<ShortlistEntry> {
    shortlist(conn)?
        .shortlist
        .into_iter()
        .find(|e| e.person_id == person)
        .ok_or_else(|| Error::NotFound(format!("person {person} on the shortlist")))
}

/// Add `person` (a current person id) to the shortlist. Adding a person who
/// is already there keeps `added_at` and replaces the note when one is given.
pub fn shortlist_add(
    conn: &Connection,
    person: PersonId,
    note: Option<&str>,
    now: i64,
) -> Result<ShortlistEntry> {
    let note = check_note(note)?;
    require_person(conn, person, "person")?;
    match shortlist_key(conn, person)? {
        Some(key) => {
            if note.is_some() {
                conn.execute(
                    "UPDATE shortlist SET note = ?2 WHERE person_id = ?1",
                    params![key, note],
                )?;
            }
        }
        None => {
            conn.execute(
                "INSERT INTO shortlist(person_id, note, added_at) VALUES (?1, ?2, ?3)",
                params![person, note, now],
            )?;
        }
    }
    shortlist_entry(conn, person)
}

/// Replace the note of a shortlisted person (`None` clears it).
pub fn shortlist_set_note(
    conn: &Connection,
    person: PersonId,
    note: Option<&str>,
) -> Result<ShortlistEntry> {
    let note = check_note(note)?;
    let key = shortlist_key(conn, person)?
        .ok_or_else(|| Error::NotFound(format!("person {person} on the shortlist")))?;
    conn.execute(
        "UPDATE shortlist SET note = ?2 WHERE person_id = ?1",
        params![key, note],
    )?;
    shortlist_entry(conn, person)
}

/// Remove a person from the shortlist. `NotFound` when they were not on it.
pub fn shortlist_remove(conn: &Connection, person: PersonId) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM shortlist WHERE person_id = ?1
            OR person_id IN (SELECT id FROM identity WHERE person_id = ?1)",
        [person],
    )?;
    if n == 0 {
        return Err(Error::NotFound(format!("person {person} on the shortlist")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Introductions
// ---------------------------------------------------------------------------

/// Where an introduction stands. Updated by hand; nothing is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Planned; the ask is not out yet.
    Draft,
    /// The connector was asked.
    Asked,
    /// The connector made the introduction.
    IntroMade,
    /// The user met the person.
    Met,
    /// Done (or dropped).
    Closed,
}

impl Stage {
    /// Every stage, in board order.
    pub const ALL: [Stage; 5] = [
        Stage::Draft,
        Stage::Asked,
        Stage::IntroMade,
        Stage::Met,
        Stage::Closed,
    ];

    /// The stored and JSON name.
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Draft => "draft",
            Stage::Asked => "asked",
            Stage::IntroMade => "intro_made",
            Stage::Met => "met",
            Stage::Closed => "closed",
        }
    }

    /// Parse a stored or JSON name.
    pub fn parse(s: &str) -> Option<Stage> {
        Stage::ALL.into_iter().find(|st| st.as_str() == s)
    }
}

/// One tracked introduction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intro {
    /// `intro.id`.
    pub id: i64,
    /// The current id of the person the user wants to meet.
    pub person_id: PersonId,
    /// That person (null when they no longer exist).
    pub person: Option<PersonRef>,
    /// Who could introduce them (null: a direct ask).
    pub connector: Option<PersonRef>,
    /// Stage.
    pub stage: Stage,
    /// The user's note.
    pub note: Option<String>,
    /// Follow-up date, `YYYY-MM-DD`.
    pub follow_up: Option<String>,
    /// Created.
    pub created_at: String,
    /// Last changed.
    pub updated_at: String,
}

/// `GET /intros`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntrosResponse {
    /// Schema version.
    pub v: u32,
    /// Board order (stage), then follow-up date, then newest.
    pub intros: Vec<Intro>,
}

/// `POST /intros`, `PATCH /intros/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntroResponse {
    /// Schema version.
    pub v: u32,
    /// The row after the change.
    pub intro: Intro,
}

/// A new introduction (people already resolved to current ids).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewIntro {
    /// Who the user wants to meet.
    pub person: PersonId,
    /// Who could introduce them.
    pub connector: Option<PersonId>,
    /// Default `draft`.
    pub stage: Option<Stage>,
    /// Note.
    pub note: Option<String>,
    /// `YYYY-MM-DD`.
    pub follow_up: Option<String>,
}

/// A partial update: `None` keeps a field, `Some(None)` clears it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IntroPatch {
    /// New stage.
    pub stage: Option<Stage>,
    /// New connector.
    pub connector: Option<Option<PersonId>>,
    /// New note.
    pub note: Option<Option<String>>,
    /// New follow-up date.
    pub follow_up: Option<Option<String>>,
}

/// Every introduction, optionally only one stage.
pub fn intros(conn: &Connection, stage: Option<Stage>) -> Result<IntrosResponse> {
    let ids: Vec<i64> = conn
        .prepare_cached(
            "SELECT id FROM intro WHERE ?1 IS NULL OR stage = ?1
             ORDER BY CASE stage WHEN 'draft' THEN 0 WHEN 'asked' THEN 1
                      WHEN 'intro_made' THEN 2 WHEN 'met' THEN 3 ELSE 4 END,
                      follow_up IS NULL, follow_up, created_at DESC, id DESC",
        )?
        .query_map([stage.map(Stage::as_str)], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(load_intro(conn, id)?);
    }
    Ok(IntrosResponse { v: V, intros: out })
}

/// The current person of a stored anchor id.
fn current(conn: &Connection, stored: i64) -> Result<PersonId> {
    let p: Option<Option<i64>> = conn
        .prepare_cached("SELECT person_id FROM identity WHERE id = ?1")?
        .query_row([stored], |r| r.get(0))
        .optional()?;
    Ok(p.flatten().unwrap_or(stored))
}

fn load_intro(conn: &Connection, id: i64) -> Result<Intro> {
    let row = conn
        .prepare_cached(
            "SELECT person_id, connector_id, stage, note, follow_up, created_at, updated_at
             FROM intro WHERE id = ?1",
        )?
        .query_row([id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("introduction {id}")))?;
    let (person, connector, stage, note, follow_up, created_at, updated_at) = row;
    let person_id = current(conn, person)?;
    let connector = match connector {
        Some(c) => person_ref(conn, current(conn, c)?)?,
        None => None,
    };
    Ok(Intro {
        id,
        person_id,
        person: person_ref(conn, person_id)?,
        connector,
        stage: Stage::parse(&stage).unwrap_or(Stage::Draft),
        note,
        follow_up,
        created_at: iso(created_at),
        updated_at: iso(updated_at),
    })
}

fn check_date(d: Option<&str>) -> Result<Option<String>> {
    let Some(d) = d.map(str::trim).filter(|d| !d.is_empty()) else {
        return Ok(None);
    };
    let f = time::macros::format_description!("[year]-[month]-[day]");
    time::Date::parse(d, &f)
        .map(|_| Some(d.to_string()))
        .map_err(|_| Error::BadRequest(format!("follow_up must be YYYY-MM-DD (got {d:?})")))
}

fn check_connector(conn: &Connection, person: PersonId, c: Option<PersonId>) -> Result<()> {
    if let Some(c) = c {
        if c == person {
            return Err(Error::BadRequest(
                "the connector must be someone other than the person".into(),
            ));
        }
        require_person(conn, c, "connector")?;
    }
    Ok(())
}

/// Start tracking an introduction.
pub fn intro_create(conn: &Connection, new: &NewIntro, now: i64) -> Result<Intro> {
    require_person(conn, new.person, "person")?;
    check_connector(conn, new.person, new.connector)?;
    let note = check_note(new.note.as_deref())?;
    let follow_up = check_date(new.follow_up.as_deref())?;
    conn.execute(
        "INSERT INTO intro(person_id, connector_id, stage, note, follow_up, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            new.person,
            new.connector,
            new.stage.unwrap_or(Stage::Draft).as_str(),
            note,
            follow_up,
            now
        ],
    )?;
    load_intro(conn, conn.last_insert_rowid())
}

/// Change an introduction. `NotFound` for an unknown id.
pub fn intro_update(conn: &Connection, id: i64, patch: &IntroPatch, now: i64) -> Result<Intro> {
    let cur = load_intro(conn, id)?;
    if let Some(stage) = patch.stage {
        conn.execute(
            "UPDATE intro SET stage = ?2 WHERE id = ?1",
            params![id, stage.as_str()],
        )?;
    }
    if let Some(c) = patch.connector {
        check_connector(conn, cur.person_id, c)?;
        conn.execute(
            "UPDATE intro SET connector_id = ?2 WHERE id = ?1",
            params![id, c],
        )?;
    }
    if let Some(n) = &patch.note {
        let n = check_note(n.as_deref())?;
        conn.execute("UPDATE intro SET note = ?2 WHERE id = ?1", params![id, n])?;
    }
    if let Some(d) = &patch.follow_up {
        let d = check_date(d.as_deref())?;
        conn.execute(
            "UPDATE intro SET follow_up = ?2 WHERE id = ?1",
            params![id, d],
        )?;
    }
    conn.execute(
        "UPDATE intro SET updated_at = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    load_intro(conn, id)
}

/// Stop tracking an introduction. `NotFound` for an unknown id.
pub fn intro_delete(conn: &Connection, id: i64) -> Result<()> {
    if conn.execute("DELETE FROM intro WHERE id = ?1", [id])? == 0 {
        return Err(Error::NotFound(format!("introduction {id}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sources and dashboard numbers
// ---------------------------------------------------------------------------

/// One `source` row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRow {
    /// `source.id`.
    pub id: i64,
    /// `mbox`, `linkedin_csv`, ..., `enrich:<backend>`.
    pub kind: String,
    /// Label.
    pub label: Option<String>,
    /// The path as given at import (display only).
    pub path: Option<String>,
    /// Import time.
    pub imported_at: String,
    /// Importer version.
    pub importer_version: i64,
    /// `ImportStats` as stored (null for enrichment sources).
    pub stats: Option<Value>,
}

/// `GET /sources` (`openstance source list --json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourcesResponse {
    /// Schema version.
    pub v: u32,
    /// Oldest first.
    pub sources: Vec<SourceRow>,
}

/// Every source, oldest first.
pub fn sources(conn: &Connection) -> Result<SourcesResponse> {
    let mut st = conn.prepare_cached(
        "SELECT id, kind, label, path, imported_at, importer_version, stats_json
         FROM source ORDER BY imported_at, id",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok((
                SourceRow {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    label: r.get(2)?,
                    path: r.get(3)?,
                    imported_at: iso(r.get(4)?),
                    importer_version: r.get(5)?,
                    stats: None,
                },
                r.get::<_, Option<String>>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let sources = rows
        .into_iter()
        .map(|(mut row, stats)| {
            row.stats = stats.and_then(|s| serde_json::from_str(&s).ok());
            row
        })
        .collect();
    Ok(SourcesResponse { v: V, sources })
}

/// The review numbers on the dashboard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewCounts {
    /// Flagged merges waiting for a decision.
    pub flagged: u64,
    /// Merge suggestions.
    pub suggestions: u64,
}

/// The egress numbers on the dashboard (from `enrich_log`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrichCounts {
    /// Requests that left the machine.
    pub requests_sent: u64,
    /// Sum of actual costs.
    pub spent_micro_usd: u64,
}

/// `GET /stats`: the dashboard numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsResponse {
    /// Schema version.
    pub v: u32,
    /// People in the graph (not you, not bulk senders).
    pub people: u64,
    /// Bulk senders (hidden from search).
    pub bulk: u64,
    /// Imported sources (enrichment sources included).
    pub sources: u64,
    /// The most recent import, if any.
    pub last_import: Option<String>,
    /// Review queue.
    pub review: ReviewCounts,
    /// Egress so far.
    pub enrich: EnrichCounts,
    /// Shortlisted people.
    pub shortlist: u64,
    /// Introductions per stage (every stage present).
    pub intros: std::collections::BTreeMap<String, u64>,
    /// Graph schema version.
    pub schema_version: u32,
}

/// The dashboard numbers that come straight from SQL. The facade adds the
/// review and egress blocks.
pub fn base_stats(conn: &Connection) -> Result<StatsResponse> {
    let count = |sql: &str| -> Result<u64> {
        let n: i64 = conn.query_row(sql, [], |r| r.get(0))?;
        Ok(u64::try_from(n).unwrap_or(0))
    };
    let last: Option<i64> =
        conn.query_row("SELECT max(imported_at) FROM source", [], |r| r.get(0))?;
    let mut intros: std::collections::BTreeMap<String, u64> = Stage::ALL
        .iter()
        .map(|s| (s.as_str().to_string(), 0))
        .collect();
    let mut st = conn.prepare_cached("SELECT stage, count(*) FROM intro GROUP BY stage")?;
    for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (stage, n) = row?;
        intros.insert(stage, u64::try_from(n).unwrap_or(0));
    }
    Ok(StatsResponse {
        v: V,
        people: count("SELECT count(*) FROM person WHERE is_self = 0 AND is_bulk = 0")?,
        bulk: count("SELECT count(*) FROM person WHERE is_bulk = 1")?,
        sources: count("SELECT count(*) FROM source")?,
        last_import: iso_opt(last),
        review: ReviewCounts::default(),
        enrich: EnrichCounts::default(),
        shortlist: shortlist(conn)?.shortlist.len() as u64,
        intros,
        schema_version: crate::db::schema_version(conn)?,
    })
}

// ---------------------------------------------------------------------------
// Graph facade
// ---------------------------------------------------------------------------

/// A new introduction by person reference (`POST /intros`).
#[derive(Debug, Clone, PartialEq)]
pub struct IntroRequest {
    /// Who the user wants to meet.
    pub person: PersonKey,
    /// Who could introduce them.
    pub connector: Option<PersonKey>,
    /// Default `draft`.
    pub stage: Option<Stage>,
    /// Note.
    pub note: Option<String>,
    /// `YYYY-MM-DD`.
    pub follow_up: Option<String>,
}

/// A partial update by person reference (`PATCH /intros/{id}`): `None`
/// keeps a field, `Some(None)` clears it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IntroEdit {
    /// New stage.
    pub stage: Option<Stage>,
    /// New connector.
    pub connector: Option<Option<PersonKey>>,
    /// New note.
    pub note: Option<Option<String>>,
    /// New follow-up date.
    pub follow_up: Option<Option<String>>,
}

fn resolve(conn: &Connection, key: &PersonKey) -> Result<PersonId> {
    Ok(resolve_person(conn, key)?.person)
}

impl crate::Graph {
    /// The shortlist.
    pub fn shortlist(&self) -> Result<ShortlistResponse> {
        shortlist(self.conn())
    }

    /// Add a person to the shortlist (idempotent).
    pub fn shortlist_add(
        &mut self,
        person: &PersonKey,
        note: Option<&str>,
    ) -> Result<ShortlistEntry> {
        let now = self.now();
        self.write(|tx, _| {
            let p = resolve(tx, person)?;
            shortlist_add(tx, p, note, now)
        })
    }

    /// Replace a shortlisted person's note.
    pub fn shortlist_set_note(
        &mut self,
        person: &PersonKey,
        note: Option<&str>,
    ) -> Result<ShortlistEntry> {
        self.write(|tx, _| {
            let p = resolve(tx, person)?;
            shortlist_set_note(tx, p, note)
        })
    }

    /// Remove a person from the shortlist.
    pub fn shortlist_remove(&mut self, person: &PersonKey) -> Result<()> {
        self.write(|tx, _| {
            let p = resolve(tx, person)?;
            shortlist_remove(tx, p)
        })
    }

    /// The introductions board.
    pub fn intros(&self, stage: Option<Stage>) -> Result<IntrosResponse> {
        intros(self.conn(), stage)
    }

    /// Start tracking an introduction.
    pub fn intro_create(&mut self, req: &IntroRequest) -> Result<Intro> {
        let now = self.now();
        self.write(|tx, _| {
            let new = NewIntro {
                person: resolve(tx, &req.person)?,
                connector: req.connector.as_ref().map(|k| resolve(tx, k)).transpose()?,
                stage: req.stage,
                note: req.note.clone(),
                follow_up: req.follow_up.clone(),
            };
            intro_create(tx, &new, now)
        })
    }

    /// Change an introduction.
    pub fn intro_update(&mut self, id: i64, edit: &IntroEdit) -> Result<Intro> {
        let now = self.now();
        self.write(|tx, _| {
            let connector = match &edit.connector {
                None => None,
                Some(None) => Some(None),
                Some(Some(k)) => Some(Some(resolve(tx, k)?)),
            };
            let patch = IntroPatch {
                stage: edit.stage,
                connector,
                note: edit.note.clone(),
                follow_up: edit.follow_up.clone(),
            };
            intro_update(tx, id, &patch, now)
        })
    }

    /// Stop tracking an introduction.
    pub fn intro_delete(&mut self, id: i64) -> Result<()> {
        self.write(|tx, _| intro_delete(tx, id))
    }

    /// Every source (`GET /sources`).
    pub fn sources(&self) -> Result<SourcesResponse> {
        sources(self.conn())
    }

    /// The dashboard numbers (`GET /stats`).
    pub fn dashboard_stats(&self) -> Result<StatsResponse> {
        let mut s = base_stats(self.conn())?;
        let q = self.review()?;
        s.review = ReviewCounts {
            flagged: q.flagged.len() as u64,
            suggestions: q.suggestions.len() as u64,
        };
        let log = self.enrich_log(&crate::egress::log::LogFilter::default())?;
        s.enrich = EnrichCounts {
            requests_sent: u64::from(log.totals.requests_sent),
            spent_micro_usd: log.totals.spent_micro_usd,
        };
        Ok(s)
    }
}
