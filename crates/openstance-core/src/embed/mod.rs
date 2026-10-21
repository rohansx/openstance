//! Embedding text, the incremental vector index and vector candidates
//! (P2-CORE-02, the vector half of P2-CORE-03). Spec:
//! `docs/tech-spec/models.md` section 6 and `search.md` section 6.2.
//!
//! Owned by the "vectors" stream.
//!
//! - [`embed_text`]: one passage per non-bulk, non-self person;
//! - [`index`]: re-embeds only people whose input hash changed, in chunks
//!   that each commit on their own (an interrupted run resumes), after the
//!   rebuild/import transaction has committed;
//! - [`status`]: stored embedder (meta) vs the configured one;
//! - [`vector_candidates`]: top-k cosine hits, optionally restricted.
//!
//! Meta keys: `embedder_id`, `embedder_dims`, `embedder_state`
//! (`building` | `ready`) and `person_vec_dims` (the table's dimension).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use openstance_model::{EmbedKind, Embedder};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::db::vec::to_blob;
use crate::error::Result;
use crate::{write_tx, PersonId};

mod facade;
#[cfg(test)]
mod tests;

pub use facade::Hook;

/// Cap on the embedding text (about 512 tokens; the backend truncates
/// exactly to the model's limit).
pub const MAX_TEXT_CHARS: usize = 2000;
/// Topics (distinct normalized subjects) per person.
pub const MAX_TOPICS: usize = 20;
/// People per embed call and per commit.
pub const DEFAULT_CHUNK: usize = 256;

/// The warning every surface shows when the configured embedder differs
/// from the one that built the stored vectors (P2-MODEL-02).
pub const CHANGED_WARNING: &str = "embedder changed, run `openstance rebuild --embeddings`";

/// Stored vectors vs the configured embedder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    /// `embedder = "none"` (dims 0): vector search is off.
    Disabled,
    /// No embedding pass has run yet.
    NotIndexed,
    /// A pass with this embedder started and has not finished.
    Building,
    /// Vectors are current for this embedder.
    Ready,
    /// Vectors were built by another embedder (or dims).
    Changed {
        /// `meta.embedder_id`.
        stored: String,
        /// The configured embedder id.
        configured: String,
    },
}

impl Status {
    /// Vector search may use the stored vectors.
    pub fn is_ready(&self) -> bool {
        *self == Status::Ready
    }
}

/// Options for [`index`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexOptions {
    /// Re-embed everyone and recreate `person_vec` when the dims differ
    /// (`rebuild --embeddings`). Also the only way past [`Status::Changed`].
    pub force: bool,
    /// Only these people (after `edit`, `merge`, `split`); `None` = all.
    pub people: Option<Vec<PersonId>>,
    /// People per embed call and per commit.
    pub chunk: usize,
}

impl Default for IndexOptions {
    fn default() -> Self {
        IndexOptions {
            force: false,
            people: None,
            chunk: DEFAULT_CHUNK,
        }
    }
}

/// Progress of one [`index`] run (for the CLI progress line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// People embedded so far.
    pub done: usize,
    /// People to embed in this run.
    pub total: usize,
}

/// What [`index`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EmbedReport {
    /// Embedder id.
    pub embedder: String,
    /// People (re-)embedded.
    pub embedded: usize,
    /// People whose input hash was unchanged.
    pub unchanged: usize,
    /// Vectors deleted (person gone, bulk or self).
    pub removed: usize,
    /// `person_vec` was dropped and recreated with new dims.
    pub recreated: bool,
    /// Why nothing was attempted (no embedder, embedder changed).
    pub skipped: Option<String>,
    /// The embedder failed (model missing, offline); search stays FTS-only
    /// for the people not embedded yet.
    pub error: Option<String>,
    /// Wall time.
    pub took_ms: u64,
}

impl EmbedReport {
    /// A user-facing warning, if any.
    pub fn warning(&self) -> Option<String> {
        if let Some(e) = &self.error {
            return Some(format!(
                "embedder unavailable: {e}; run 'openstance rebuild --embeddings' when online"
            ));
        }
        self.skipped
            .as_ref()
            .filter(|s| s.as_str() == CHANGED_WARNING)
            .cloned()
    }
}

// ---------------------------------------------------------------------------
// Embedding text
// ---------------------------------------------------------------------------

/// Subjects that say nothing about a person.
const GENERIC: &[&str] = &[
    "hi",
    "hello",
    "hey",
    "meeting",
    "call",
    "chat",
    "catch up",
    "intro",
    "no subject",
    "(no subject)",
    "untitled",
    "linkedin message",
    "message",
    "invitation",
    "updated invitation",
];

/// Prefixes of calendar noise (`Invitation: ...`, `Accepted: ...`).
const GENERIC_PREFIXES: &[&str] = &[
    "invitation:",
    "updated invitation",
    "accepted:",
    "declined:",
    "tentative:",
    "tentatively accepted:",
    "canceled:",
    "cancelled:",
    "canceled event:",
];

/// Strip any run of `Re:` / `Fwd:` / `Fw:` / `Aw:` prefixes.
fn strip_reply_prefixes(s: &str) -> &str {
    let mut s = s.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let cut = ["re:", "fwd:", "fw:", "aw:", "wg:", "re :", "fwd :"]
            .iter()
            .find(|p| lower.starts_with(*p))
            .map(|p| p.len());
        match cut {
            Some(n) => s = s[n..].trim_start(),
            None => return s,
        }
    }
}

/// Up to `max` topics from `(subject, at)` rows: reply prefixes stripped,
/// generic subjects dropped, grouped case-insensitively, ranked by count,
/// then most recent, then text. The display form is the newest spelling.
pub fn topics(subjects: &[(String, i64)], max: usize) -> Vec<String> {
    struct Topic {
        display: String,
        count: usize,
        last: i64,
    }
    let mut by_key: BTreeMap<String, Topic> = BTreeMap::new();
    for (raw, at) in subjects {
        let s = strip_reply_prefixes(raw);
        let key = s
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if key.is_empty()
            || GENERIC.contains(&key.as_str())
            || GENERIC_PREFIXES.iter().any(|p| key.starts_with(p))
        {
            continue;
        }
        let t = by_key.entry(key).or_insert(Topic {
            display: s.to_string(),
            count: 0,
            last: i64::MIN,
        });
        t.count += 1;
        if *at > t.last {
            t.last = *at;
            t.display = s.split_whitespace().collect::<Vec<_>>().join(" ");
        }
    }
    let mut v: Vec<Topic> = by_key.into_values().collect();
    v.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then(b.last.cmp(&a.last))
            .then(a.display.cmp(&b.display))
    });
    v.into_iter().take(max).map(|t| t.display).collect()
}

fn person_subjects(conn: &Connection, id: PersonId) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT i.subject, i.at FROM interaction_person ip
         JOIN interaction i ON i.id = ip.interaction_id
         WHERE ip.person_id = ?1 AND i.kind IN ('email', 'meeting', 'dm')
           AND i.subject IS NOT NULL AND trim(i.subject) <> ''",
    )?;
    let rows = stmt
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
}

/// The embedding passage for one person (models.md section 6.1), or `None`
/// for self, bulk and missing people. Notes are never included.
///
/// ```text
/// {display_name}. {title} at {company}. {location}. {bio}
/// Tags: {tag1}, {tag2}
/// Topics: {subject1}; {subject2}; ...
/// ```
pub fn embed_text(conn: &Connection, id: PersonId) -> Result<Option<String>> {
    type Row = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let row: Option<Row> = conn
        .prepare_cached(
            "SELECT display_name, title, company, location, bio FROM person
             WHERE id = ?1 AND is_bulk = 0 AND is_self = 0",
        )?
        .query_row([id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .optional()?;
    let Some((name, title, company, location, bio)) = row else {
        return Ok(None);
    };
    let role = match (clean(title), clean(company)) {
        (Some(t), Some(c)) => Some(format!("{t} at {c}")),
        (t, c) => t.or(c),
    };
    let head: Vec<String> = [clean(name), role, clean(location), clean(bio)]
        .into_iter()
        .flatten()
        .map(|p| p.trim_end_matches('.').to_string())
        .collect();
    let mut text = head.join(". ");
    let tags = crate::search::person::tags(conn, id)?;
    if !tags.is_empty() {
        text.push_str("\nTags: ");
        text.push_str(&tags.join(", "));
    }
    let t = topics(&person_subjects(conn, id)?, MAX_TOPICS);
    if !t.is_empty() {
        text.push_str("\nTopics: ");
        text.push_str(&t.join("; "));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        text = text.chars().take(MAX_TEXT_CHARS).collect();
    }
    Ok(Some(text))
}

/// `blake3(embedder_id + "\n" + text)`, hex.
pub fn text_hash(embedder_id: &str, text: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(embedder_id.as_bytes());
    h.update(b"\n");
    h.update(text.as_bytes());
    h.finalize().to_hex().to_string()
}

// ---------------------------------------------------------------------------
// Status and meta
// ---------------------------------------------------------------------------

fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT value FROM meta WHERE key = ?1")?
        .query_row([key], |r| r.get(0))
        .optional()?)
}

fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )?
    .execute([key, value])?;
    Ok(())
}

/// The dimension `person_vec` was created with.
pub fn table_dims(conn: &Connection) -> Result<usize> {
    Ok(meta_get(conn, "person_vec_dims")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(384))
}

/// Compare the stored vectors with `embedder`.
pub fn status(conn: &Connection, embedder: &dyn Embedder) -> Result<Status> {
    if embedder.dims() == 0 {
        return Ok(Status::Disabled);
    }
    let Some(stored) = meta_get(conn, "embedder_id")? else {
        return Ok(Status::NotIndexed);
    };
    let dims = meta_get(conn, "embedder_dims")?.and_then(|d| d.parse::<usize>().ok());
    if stored != embedder.id() || dims != Some(embedder.dims()) {
        return Ok(Status::Changed {
            stored,
            configured: embedder.id().to_string(),
        });
    }
    Ok(match meta_get(conn, "embedder_state")?.as_deref() {
        Some("ready") => Status::Ready,
        _ => Status::Building,
    })
}

fn recreate_table(conn: &Connection, dims: usize) -> Result<()> {
    conn.execute_batch(&format!(
        "DROP TABLE IF EXISTS person_vec;
         CREATE VIRTUAL TABLE person_vec USING vec0(
           person_id INTEGER PRIMARY KEY,
           embedding float[{dims}]
         );
         DELETE FROM person_embed;"
    ))?;
    meta_set(conn, "person_vec_dims", &dims.to_string())
}

fn delete_people(conn: &Connection, ids: &[PersonId]) -> Result<()> {
    let mut v = conn.prepare_cached("DELETE FROM person_vec WHERE person_id = ?1")?;
    let mut h = conn.prepare_cached("DELETE FROM person_embed WHERE person_id = ?1")?;
    for id in ids {
        v.execute([id])?;
        h.execute([id])?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

fn ms(t: Instant) -> u64 {
    u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Bring `person_vec` up to date for `embedder`. Each chunk is embedded
/// outside any transaction and written in its own `IMMEDIATE` transaction.
/// Model failures are reported in [`EmbedReport::error`], never raised;
/// only database errors are.
pub fn index(
    conn: &mut Connection,
    embedder: &dyn Embedder,
    opts: &IndexOptions,
    progress: &mut dyn FnMut(Progress),
) -> Result<EmbedReport> {
    let started = Instant::now();
    let mut report = EmbedReport {
        embedder: embedder.id().to_string(),
        ..EmbedReport::default()
    };
    let dims = embedder.dims();
    let st = status(conn, embedder)?;
    match &st {
        Status::Disabled => {
            report.skipped = Some("no embedder configured".into());
            return Ok(report);
        }
        Status::Changed { .. } if !opts.force => {
            report.skipped = Some(CHANGED_WARNING.into());
            return Ok(report);
        }
        _ => {}
    }

    // Start: (re)create the table if needed, mark the run as building.
    let id = embedder.id().to_string();
    report.recreated = write_tx(conn, |tx| {
        let recreate = table_dims(tx)? != dims;
        if recreate {
            recreate_table(tx, dims)?;
        } else if opts.force {
            tx.execute("DELETE FROM person_embed", [])?;
        }
        meta_set(tx, "embedder_id", &id)?;
        meta_set(tx, "embedder_dims", &dims.to_string())?;
        meta_set(tx, "embedder_state", "building")?;
        Ok(recreate)
    })?;

    // Plan: who needs a vector, who loses one.
    let full = opts.people.is_none();
    let candidates: Vec<PersonId> = match &opts.people {
        Some(ids) => {
            let mut v = ids.clone();
            v.sort_unstable();
            v.dedup();
            v
        }
        None => conn
            .prepare("SELECT id FROM person WHERE is_bulk = 0 AND is_self = 0 ORDER BY id")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    };
    let stored: HashMap<PersonId, String> = conn
        .prepare("SELECT person_id, text_hash FROM person_embed")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut todo: Vec<(PersonId, String, String)> = Vec::new();
    let mut gone: Vec<PersonId> = Vec::new();
    let mut eligible: HashSet<PersonId> = HashSet::with_capacity(candidates.len());
    for pid in &candidates {
        match embed_text(conn, *pid)? {
            Some(text) => {
                eligible.insert(*pid);
                let h = text_hash(&id, &text);
                if stored.get(pid) == Some(&h) {
                    report.unchanged += 1;
                } else {
                    todo.push((*pid, text, h));
                }
            }
            None => {
                if stored.contains_key(pid) {
                    gone.push(*pid);
                }
            }
        }
    }
    if full {
        gone.extend(stored.keys().filter(|p| !eligible.contains(p)).copied());
        // vectors without a hash row (an interrupted older run)
        let orphans: Vec<PersonId> = conn
            .prepare("SELECT person_id FROM person_vec")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<PersonId>>>()?
            .into_iter()
            .filter(|p| !eligible.contains(p) && !stored.contains_key(p))
            .collect();
        gone.extend(orphans);
    }
    gone.sort_unstable();
    gone.dedup();
    if !gone.is_empty() {
        write_tx(conn, |tx| delete_people(tx, &gone))?;
        report.removed = gone.len();
    }

    // Embed and write, chunk by chunk.
    let total = todo.len();
    if total > 0 {
        progress(Progress { done: 0, total });
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    for chunk in todo.chunks(opts.chunk.max(1)) {
        let texts: Vec<&str> = chunk.iter().map(|t| t.1.as_str()).collect();
        let vectors = match embedder.embed(&texts, EmbedKind::Passage) {
            Ok(v) if v.len() == chunk.len() && v.iter().all(|x| x.len() == dims) => v,
            Ok(_) => {
                report.error = Some("the embedder returned vectors of the wrong shape".into());
                break;
            }
            Err(e) => {
                report.error = Some(e.to_string());
                break;
            }
        };
        write_tx(conn, |tx| {
            let mut del = tx.prepare_cached("DELETE FROM person_vec WHERE person_id = ?1")?;
            let mut ins =
                tx.prepare_cached("INSERT INTO person_vec(person_id, embedding) VALUES (?1, ?2)")?;
            let mut hash = tx.prepare_cached(
                "INSERT INTO person_embed(person_id, text_hash, at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(person_id) DO UPDATE SET text_hash = excluded.text_hash,
                                                      at = excluded.at",
            )?;
            for ((pid, _, h), v) in chunk.iter().zip(&vectors) {
                del.execute([pid])?;
                ins.execute(params![pid, to_blob(v)])?;
                hash.execute(params![pid, h, now])?;
            }
            Ok(())
        })?;
        report.embedded += chunk.len();
        progress(Progress {
            done: report.embedded,
            total,
        });
    }
    if report.error.is_none() {
        write_tx(conn, |tx| meta_set(tx, "embedder_state", "ready"))?;
    }
    report.took_ms = ms(started);
    Ok(report)
}

// ---------------------------------------------------------------------------
// Vector candidates
// ---------------------------------------------------------------------------

/// Top `k` people by cosine similarity to `qvec` (`cos = 1 - distance`),
/// best first, ties by id. With `allowed`, a brute-force scan over the
/// stored vectors keeps only those ids; without it, the vec0 KNN index is
/// used (vectors are L2-normalized, so L2 order equals cosine order). A
/// query of the wrong length gives no hits.
pub fn vector_candidates(
    conn: &Connection,
    qvec: &[f32],
    allowed: Option<&HashSet<PersonId>>,
    k: usize,
) -> Result<Vec<(PersonId, f64)>> {
    if k == 0 || qvec.is_empty() || qvec.len() != table_dims(conn)? {
        return Ok(Vec::new());
    }
    if allowed.is_some_and(HashSet::is_empty) {
        return Ok(Vec::new());
    }
    let q = to_blob(qvec);
    let mut hits: Vec<(PersonId, f64)> = match allowed {
        None => {
            let k = i64::try_from(k.min(4096)).unwrap_or(4096);
            conn.prepare_cached(
                "SELECT person_id, vec_distance_cosine(embedding, ?1) FROM person_vec
                 WHERE embedding MATCH ?1 AND k = ?2",
            )?
            .query_map(params![q, k], |r| {
                Ok((r.get::<_, PersonId>(0)?, 1.0 - r.get::<_, f64>(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?
        }
        Some(set) => {
            let mut stmt = conn.prepare_cached(
                "SELECT person_id, vec_distance_cosine(embedding, ?1) FROM person_vec",
            )?;
            let mut rows = stmt.query(params![q])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let id: PersonId = r.get(0)?;
                if set.contains(&id) {
                    out.push((id, 1.0 - r.get::<_, f64>(1)?));
                }
            }
            out
        }
    };
    hits.retain(|h| h.1.is_finite());
    hits.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    hits.truncate(k);
    Ok(hits)
}
