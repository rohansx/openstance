//! Imported views: `.ost` files other people sent me (P3-CORE-08).
//!
//! A file is decrypted with my age identity, validated as untrusted input,
//! and stored as `<views dir>/<view_id>.db` (mode 0600) with a local FTS5
//! index. Nothing here opens `graph.db`: imported people are never merged,
//! scored, embedded or enriched. Expired copies are refused and purged.
//!
//! Spec: `docs/tech-spec/sharing-and-views.md` sections 4.4 and 6.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use super::ost::{ALLOWED_COLUMNS, APPLICATION_ID, FORMAT_VERSION};
use super::{
    meta_map, ViewOwner, ViewPerson, ViewRef, ViewSearchResponse, ViewSearchResult, ViewSummary,
};
use crate::error::{Error, Result};

/// `share.max_import_mb` in bytes.
pub fn max_bytes(mb: u32) -> u64 {
    u64::from(mb) * 1024 * 1024
}

/// `import` outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    /// First copy of this view.
    Imported,
    /// Replaced an older export of the same view.
    Replaced,
    /// A copy at least as new was already there; the file was ignored.
    KeptNewer,
}

/// What `view import` did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportReport {
    /// Schema version.
    pub v: u32,
    /// What happened.
    pub status: ImportStatus,
    /// The stored view.
    pub view: ViewSummary,
    /// Where it is stored.
    pub path: PathBuf,
}

fn invalid(why: impl std::fmt::Display) -> Error {
    Error::BadRequest(format!("not a valid .ost view file: {why}"))
}

/// Decrypt `ciphertext` (binary or armored) with `identity`, reading at
/// most `max` plaintext bytes.
pub fn decrypt(identity: &age::x25519::Identity, ciphertext: &[u8], max: u64) -> Result<Vec<u8>> {
    let dec = age::Decryptor::new_buffered(age::armor::ArmoredReader::new(ciphertext))
        .map_err(|e| invalid(format!("age: {e}")))?;
    let reader = match dec.decrypt(std::iter::once(identity as &dyn age::Identity)) {
        Ok(r) => r,
        Err(age::DecryptError::NoMatchingKeys) => {
            return Err(Error::ViewExpired(
                "this view file is not addressed to my key (see `openstance keys show`)".into(),
            ))
        }
        Err(e) => return Err(invalid(format!("age: {e}"))),
    };
    let mut out = Vec::new();
    reader
        .take(max.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(|e| invalid(format!("decrypting: {e}")))?;
    if out.len() as u64 > max {
        return Err(Error::BadRequest(format!(
            "view file is larger than share.max_import_mb ({} MB)",
            max / (1024 * 1024)
        )));
    }
    Ok(out)
}

fn open_untrusted(path: &Path) -> Result<Connection> {
    let c = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    c.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    c.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA,
        false,
    )?;
    c.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
        false,
    )?;
    c.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW,
        false,
    )?;
    Ok(c)
}

/// The validated `meta` of an incoming file.
#[derive(Debug, Clone)]
struct Meta {
    map: BTreeMap<String, String>,
    view_id: String,
    exported_at: i64,
    expires_at: Option<i64>,
}

fn int_meta(map: &BTreeMap<String, String>, key: &str) -> Result<i64> {
    map.get(key)
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| invalid(format!("meta.{key} missing or not a number")))
}

fn parse_meta(c: &Connection) -> Result<Meta> {
    let map = meta_map(c)?;
    for k in [
        "format_version",
        "view_id",
        "name",
        "exported_at",
        "created_at",
        "query_json",
    ] {
        if !map.contains_key(k) {
            return Err(invalid(format!("meta.{k} missing")));
        }
    }
    let view_id = map["view_id"].clone();
    // The id becomes a file name: only a canonical ULID is accepted.
    match ulid::Ulid::from_string(&view_id) {
        Ok(u) if u.to_string() == view_id => {}
        _ => return Err(invalid("meta.view_id is not a ULID")),
    }
    let expires_at = match map.get("expires_at").map(|s| s.trim()) {
        None | Some("") => None,
        Some(_) => Some(int_meta(&map, "expires_at")?),
    };
    Ok(Meta {
        exported_at: int_meta(&map, "exported_at")?,
        expires_at,
        view_id,
        map,
    })
}

/// Validate an incoming payload (section 4.4 steps 3 to 5).
fn validate(c: &Connection, now: i64) -> Result<Meta> {
    let app: i64 = c
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .map_err(invalid)?;
    if app != APPLICATION_ID {
        return Err(invalid("wrong application_id"));
    }
    let ver: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if !(1..=FORMAT_VERSION).contains(&ver) {
        return Err(invalid(format!(
            "format version {ver} is not supported (max {FORMAT_VERSION}); upgrade openstance"
        )));
    }
    let mut stmt = c.prepare("SELECT type, name, tbl_name FROM sqlite_master")?;
    let objects = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let allowed = |t: &str| ALLOWED_COLUMNS.iter().any(|(n, _)| *n == t);
    for (ty, name, tbl) in &objects {
        let ok = match ty.as_str() {
            "table" => allowed(name),
            "index" => name.starts_with("sqlite_autoindex_") && allowed(tbl),
            _ => false,
        };
        if !ok {
            return Err(invalid(format!("unexpected {ty} {name:?}")));
        }
    }
    for (table, cols) in ALLOWED_COLUMNS {
        let got = c
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if got != *cols {
            return Err(invalid(format!("table {table} has unexpected columns")));
        }
    }
    let meta = parse_meta(c)?;
    if meta.expires_at.is_some_and(|e| e <= now) {
        return Err(Error::ViewExpired(format!(
            "view {:?} expired on {}; ask the owner for a fresh export",
            meta.map["name"],
            crate::json::iso(meta.expires_at.unwrap_or_default())
        )));
    }
    Ok(meta)
}

fn ensure_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|e| Error::io("creating", dir, e))?;
        crate::keys::set_mode(dir, 0o700)?;
    }
    Ok(())
}

/// Removes a path on drop unless disarmed.
struct Cleanup(Option<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// `view import`: decrypt, validate, and store under `dir`, keeping only
/// the newest export per view id.
pub fn import(
    dir: &Path,
    identity: &age::x25519::Identity,
    ciphertext: &[u8],
    max: u64,
    now: i64,
) -> Result<ImportReport> {
    let plain = decrypt(identity, ciphertext, max)?;
    ensure_dir(dir)?;
    let tmp = dir.join(format!(".incoming-{}.tmp", ulid::Ulid::new()));
    let mut guard = Cleanup(Some(tmp.clone()));
    {
        use std::io::Write;
        let mut f = crate::keys::create_private(&tmp)?;
        f.write_all(&plain)
            .and_then(|()| f.sync_all())
            .map_err(|e| Error::io("writing", &tmp, e))?;
    }
    drop(plain);
    let meta = {
        let c = open_untrusted(&tmp).map_err(|_| invalid("not an SQLite database"))?;
        validate(&c, now).map_err(|e| match e {
            Error::Db(e) => invalid(e),
            e => e,
        })?
    };
    let dest = dir.join(format!("{}.db", meta.view_id));
    let status = if dest.exists() {
        let old = open_untrusted(&dest)
            .and_then(|c| parse_meta(&c))
            .map(|m| m.exported_at)
            .unwrap_or(i64::MIN);
        if old >= meta.exported_at {
            let view = summary(&dest)?;
            return Ok(ImportReport {
                v: crate::json::V,
                status: ImportStatus::KeptNewer,
                view,
                path: dest,
            });
        }
        ImportStatus::Replaced
    } else {
        ImportStatus::Imported
    };
    {
        let c = Connection::open(&tmp)?;
        c.execute_batch(
            "CREATE VIRTUAL TABLE person_fts USING fts5(
               display_name, title, company, location,
               content = 'person', content_rowid = 'id',
               tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3');
             INSERT INTO person_fts(person_fts) VALUES ('rebuild');",
        )?;
    }
    std::fs::rename(&tmp, &dest).map_err(|e| Error::io("storing", &dest, e))?;
    guard.0 = None;
    let view = summary(&dest)?;
    Ok(ImportReport {
        v: crate::json::V,
        status,
        view,
        path: dest,
    })
}

fn owner(map: &BTreeMap<String, String>) -> ViewOwner {
    let get = |k: &str| map.get(k).filter(|v| !v.is_empty()).cloned();
    ViewOwner {
        pubkey: get("owner_pubkey"),
        label: get("owner_label"),
    }
}

fn summary(path: &Path) -> Result<ViewSummary> {
    let c = open_untrusted(path)?;
    let m = parse_meta(&c)?;
    let rows: i64 = c.query_row("SELECT count(*) FROM person", [], |r| r.get(0))?;
    Ok(ViewSummary {
        id: Some(m.view_id.clone()),
        name: m.map["name"].clone(),
        direction: "imported".into(),
        owner: owner(&m.map),
        created_at: crate::json::iso(int_meta(&m.map, "created_at")?),
        expires_at: crate::json::iso_opt(m.expires_at),
        rows: u64::try_from(rows).unwrap_or(0),
        query: serde_json::from_str(&m.map["query_json"]).unwrap_or(serde_json::Value::Null),
        recipients: Vec::new(),
        excluded_do_not_share: None,
        exported_at: Some(crate::json::iso(m.exported_at)),
    })
}

/// One stored view: its path and validated meta.
struct Stored {
    path: PathBuf,
    meta: Meta,
}

/// Every stored view; expired ones are deleted (`purged` lists their names).
fn scan(dir: &Path, now: i64) -> Result<(Vec<Stored>, Vec<String>)> {
    let mut out = Vec::new();
    let mut purged = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((out, purged)),
        Err(e) => return Err(Error::io("reading", dir, e)),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "db"))
        .collect();
    paths.sort();
    for path in paths {
        let Ok(meta) = open_untrusted(&path).and_then(|c| parse_meta(&c)) else {
            tracing::warn!(path = %path.display(), "skipping unreadable imported view");
            continue;
        };
        if meta.expires_at.is_some_and(|e| e <= now) {
            purged.push(meta.map["name"].clone());
            let _ = std::fs::remove_file(&path);
            continue;
        }
        out.push(Stored { path, meta });
    }
    Ok((out, purged))
}

/// Delete every expired imported view; returns their names.
pub fn purge_expired(dir: &Path, now: i64) -> Result<Vec<String>> {
    Ok(scan(dir, now)?.1)
}

/// Summaries of every unexpired imported view (expired ones are purged).
pub fn list(dir: &Path, now: i64) -> Result<Vec<ViewSummary>> {
    let (stored, _) = scan(dir, now)?;
    let mut out = stored
        .iter()
        .map(|s| summary(&s.path))
        .collect::<Result<Vec<_>>>()?;
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Ok(out)
}

/// Delete an imported view's local copy (by name or id).
pub fn remove(dir: &Path, view: &str, now: i64) -> Result<()> {
    let (stored, _) = scan(dir, now)?;
    let hits: Vec<&Stored> = stored.iter().filter(|s| matches(s, view)).collect();
    if hits.is_empty() {
        return Err(Error::NotFound(format!("imported view {view:?}")));
    }
    for s in hits {
        std::fs::remove_file(&s.path).map_err(|e| Error::io("removing", &s.path, e))?;
    }
    Ok(())
}

fn matches(s: &Stored, view: &str) -> bool {
    s.meta.view_id == view || s.meta.map["name"] == view
}

/// Search input.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewQuery {
    /// View name or id, or `*` for every imported, unexpired view.
    pub view: String,
    /// Free text (FTS over name, title, company, location).
    pub text: Option<String>,
    /// Company contains (OR-ed, case-insensitive).
    pub company: Vec<String>,
    /// Title contains (OR-ed, case-insensitive).
    pub title: Vec<String>,
    /// Max results (default 20).
    pub limit: Option<usize>,
}

fn contains_any(field: &Option<String>, needles: &[String]) -> bool {
    needles.is_empty()
        || field.as_deref().is_some_and(|f| {
            let f = f.to_lowercase();
            needles.iter().any(|n| f.contains(&n.to_lowercase()))
        })
}

fn search_one(s: &Stored, q: &ViewQuery) -> Result<Vec<(ViewPerson, f64)>> {
    let c = open_untrusted(&s.path)?;
    let text = q.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let hits: Vec<(i64, f64)> = match text {
        Some(t) => {
            let m = crate::search::fts::escape_query(t);
            if m.is_empty() {
                Vec::new()
            } else {
                c.prepare(
                    "SELECT rowid, -bm25(person_fts) FROM person_fts WHERE person_fts MATCH ?1
                     ORDER BY 2 DESC, rowid",
                )?
                .query_map([m], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
            }
        }
        None => c
            .prepare("SELECT id, 1.0 FROM person ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    };
    let top = hits.iter().map(|h| h.1).fold(f64::MIN, f64::max);
    let mut person = c.prepare(
        "SELECT display_name, title, company, location, score_bucket, last_contact_month
         FROM person WHERE id = ?1",
    )?;
    let mut idents =
        c.prepare("SELECT kind, value FROM identity WHERE person_id = ?1 ORDER BY kind, value")?;
    let mut out = Vec::new();
    for (id, rel) in hits {
        let mut p = person.query_row([id], |r| {
            Ok(ViewPerson {
                display_name: r.get(0)?,
                title: r.get(1)?,
                company: r.get(2)?,
                location: r.get(3)?,
                score_bucket: r.get(4)?,
                last_contact_month: r.get(5)?,
                emails: Vec::new(),
                linkedin: None,
            })
        })?;
        if !contains_any(&p.company, &q.company) || !contains_any(&p.title, &q.title) {
            continue;
        }
        for row in idents.query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })? {
            let (kind, value) = row?;
            match kind.as_str() {
                "email" => p.emails.push(value),
                "linkedin_url" if p.linkedin.is_none() => p.linkedin = Some(value),
                _ => {}
            }
        }
        let relevance = if top > 0.0 {
            (rel / top).clamp(0.0, 1.0)
        } else {
            1.0
        };
        out.push((p, relevance));
    }
    Ok(out)
}

fn bucket_rank(b: &str) -> u8 {
    match b {
        "strong" => 0,
        "medium" => 1,
        _ => 2,
    }
}

/// `view search` / MCP `search_view` over imported, unexpired views.
pub fn search(dir: &Path, q: &ViewQuery, now: i64) -> Result<ViewSearchResponse> {
    let all = q.view.trim() == "*";
    if !all {
        // A named view that expired is refused (and its copy deleted).
        let (stored, purged) = scan(dir, now)?;
        if !stored.iter().any(|s| matches(s, &q.view)) {
            if purged.iter().any(|n| n == &q.view) {
                return Err(Error::ViewExpired(format!(
                    "imported view {:?} expired and was deleted",
                    q.view
                )));
            }
            return Err(Error::NotFound(format!("imported view {:?}", q.view)));
        }
    }
    let (stored, _) = scan(dir, now)?;
    let mut results: Vec<ViewSearchResult> = Vec::new();
    for s in stored.iter().filter(|s| all || matches(s, &q.view)) {
        let vref = ViewRef {
            id: s.meta.view_id.clone(),
            name: s.meta.map["name"].clone(),
            owner: owner(&s.meta.map),
        };
        for (person, relevance) in search_one(s, q)? {
            if all {
                let shared = results.iter_mut().find(|r| {
                    r.person.emails.iter().any(|e| person.emails.contains(e))
                        || (r.person.linkedin.is_some() && r.person.linkedin == person.linkedin)
                });
                if let Some(r) = shared {
                    r.also_in.push(vref.clone());
                    r.relevance = r.relevance.max(relevance);
                    if bucket_rank(&person.score_bucket) < bucket_rank(&r.person.score_bucket) {
                        r.person.score_bucket = person.score_bucket.clone();
                    }
                    continue;
                }
            }
            results.push(ViewSearchResult {
                view: vref.clone(),
                also_in: Vec::new(),
                person,
                relevance,
            });
        }
    }
    results.sort_by(|a, b| {
        b.relevance
            .total_cmp(&a.relevance)
            .then(bucket_rank(&a.person.score_bucket).cmp(&bucket_rank(&b.person.score_bucket)))
            .then(a.person.display_name.cmp(&b.person.display_name))
    });
    results.truncate(q.limit.unwrap_or(20).clamp(1, 200));
    Ok(ViewSearchResponse {
        v: crate::json::V,
        results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::search::testkit;
    use crate::view::ost::{self, ExportOptions};
    use crate::view::{create, CreateView, Ttl};

    const DAY: i64 = 86_400;
    const MB: u64 = 1024 * 1024;

    struct Fx {
        owner: Connection,
        cfg: Config,
        owner_key: crate::keys::KeyPair,
        me: crate::keys::KeyPair,
        inbox: PathBuf,
        _dir: tempfile::TempDir,
    }

    fn fx(query: &str, ttl: Ttl) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let owner_key = crate::keys::ensure(&dir.path().join("owner.key"), 0).unwrap();
        let me = crate::keys::ensure(&dir.path().join("me.key"), 0).unwrap();
        let owner = testkit::indexed();
        create(
            &owner,
            &CreateView {
                name: "devtools".into(),
                query: serde_json::from_str(query).unwrap(),
                ttl,
                recipients: vec![me.public()],
                replace: false,
            },
            testkit::NOW,
        )
        .unwrap();
        Fx {
            owner,
            cfg: Config::default(),
            inbox: dir.path().join("views"),
            owner_key,
            me,
            _dir: dir,
        }
    }

    fn export_at(f: &Fx, now: i64) -> Vec<u8> {
        let pk = f.owner_key.public();
        ost::export(
            &f.owner,
            &f.cfg,
            "devtools",
            &ExportOptions {
                owner_pubkey: &pk,
                owner_label: "owen",
                armor: false,
                tmp_parent: None,
                version: "test",
            },
            now,
        )
        .unwrap()
        .bytes
    }

    #[test]
    fn import_stores_a_0600_copy_and_search_finds_people() {
        let f = fx("{}", Ttl::Secs(30 * DAY));
        let bytes = export_at(&f, testkit::NOW);
        let r = import(&f.inbox, &f.me.identity, &bytes, 100 * MB, testkit::NOW).unwrap();
        assert_eq!(r.status, ImportStatus::Imported);
        assert_eq!(r.view.name, "devtools");
        assert_eq!(r.view.direction, "imported");
        assert_eq!(r.view.owner.label.as_deref(), Some("owen"));
        assert!(r.path.starts_with(&f.inbox));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&r.path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let res = search(
            &f.inbox,
            &ViewQuery {
                view: "devtools".into(),
                text: Some("northwind".into()),
                ..ViewQuery::default()
            },
            testkit::NOW,
        )
        .unwrap();
        assert_eq!(res.results.len(), 1);
        let p = &res.results[0].person;
        assert_eq!(p.display_name.as_deref(), Some("Priya Raman"));
        assert_eq!(p.emails, vec!["priya@northwind.example".to_string()]);
        assert_eq!(p.linkedin.as_deref(), Some("priya-raman-nw"));
        assert_eq!(p.score_bucket, "strong");
        assert_eq!(res.results[0].view.id, r.view.id.clone().unwrap());
        // Filters without text.
        let res = search(
            &f.inbox,
            &ViewQuery {
                view: "*".into(),
                company: vec!["contoso".into()],
                ..ViewQuery::default()
            },
            testkit::NOW,
        )
        .unwrap();
        assert_eq!(res.results.len(), 1);
        assert_eq!(
            res.results[0].person.display_name.as_deref(),
            Some("Marcus Lee")
        );
        assert_eq!(list(&f.inbox, testkit::NOW).unwrap().len(), 1);
        // No incoming temp files left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&f.inbox)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn expired_files_are_refused_and_expired_copies_purged() {
        let f = fx("{}", Ttl::Secs(DAY));
        let bytes = export_at(&f, testkit::NOW);
        let err = import(
            &f.inbox,
            &f.me.identity,
            &bytes,
            100 * MB,
            testkit::NOW + 2 * DAY,
        )
        .unwrap_err();
        assert!(
            matches!(&err, Error::ViewExpired(m) if m.contains("expired")),
            "{err}"
        );
        assert_eq!(err.code().exit_code(), 7);
        // Imported in time, then searched after expiry: refused and deleted.
        import(&f.inbox, &f.me.identity, &bytes, 100 * MB, testkit::NOW).unwrap();
        let q = ViewQuery {
            view: "devtools".into(),
            ..ViewQuery::default()
        };
        let err = search(&f.inbox, &q, testkit::NOW + 2 * DAY).unwrap_err();
        assert!(matches!(err, Error::ViewExpired(_)), "{err}");
        assert!(list(&f.inbox, testkit::NOW + 2 * DAY).unwrap().is_empty());
    }

    #[test]
    fn wrong_key_is_refused_with_exit_7() {
        let f = fx("{}", Ttl::Secs(DAY));
        let bytes = export_at(&f, testkit::NOW);
        let err = import(
            &f.inbox,
            &f.owner_key.identity,
            &bytes,
            100 * MB,
            testkit::NOW,
        )
        .unwrap_err();
        assert!(
            matches!(&err, Error::ViewExpired(m) if m.contains("not addressed")),
            "{err}"
        );
        assert_eq!(err.code().exit_code(), 7);
    }

    #[test]
    fn newer_export_replaces_older_and_older_never_replaces_newer() {
        let f = fx("{}", Ttl::Secs(30 * DAY));
        let old = export_at(&f, testkit::NOW);
        f.owner
            .execute("UPDATE person SET do_not_share = 1 WHERE id = 2", [])
            .unwrap();
        let new = export_at(&f, testkit::NOW + 60);
        let a = import(&f.inbox, &f.me.identity, &old, 100 * MB, testkit::NOW + 100).unwrap();
        let b = import(&f.inbox, &f.me.identity, &new, 100 * MB, testkit::NOW + 100).unwrap();
        assert_eq!(b.status, ImportStatus::Replaced);
        assert_eq!(a.path, b.path);
        assert_eq!(b.view.rows + 1, a.view.rows);
        let c = import(&f.inbox, &f.me.identity, &old, 100 * MB, testkit::NOW + 100).unwrap();
        assert_eq!(c.status, ImportStatus::KeptNewer);
        assert_eq!(c.view.rows, b.view.rows);
        assert_eq!(list(&f.inbox, testkit::NOW).unwrap().len(), 1);
    }

    /// Encrypt a hand-made SQLite payload to `me`.
    fn forge(f: &Fx, build: impl FnOnce(&Connection)) -> Vec<u8> {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.db");
        {
            let c = Connection::open(&p).unwrap();
            c.execute_batch(&format!(
                "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = 1;"
            ))
            .unwrap();
            c.execute_batch(ost::SCHEMA).unwrap();
            c.execute_batch(&format!(
                "INSERT INTO meta VALUES ('format_version','1'),('view_id','{}'),('name','x'),
                 ('exported_at','1'),('created_at','1'),('expires_at',''),('query_json','{{}}');",
                ulid::Ulid::new()
            ))
            .unwrap();
            build(&c);
        }
        let plain = std::fs::read(&p).unwrap();
        ost::encrypt(&[f.me.identity.to_public()], &plain, false).unwrap()
    }

    #[test]
    fn import_rejects_extra_tables_triggers_and_wrong_application_id() {
        let f = fx("{}", Ttl::Never);
        let good = forge(&f, |_| {});
        assert!(import(&f.inbox, &f.me.identity, &good, MB, 0).is_ok());
        let cases: Vec<(&str, Vec<u8>)> = vec![
            (
                "extra table",
                forge(&f, |c| {
                    c.execute_batch("CREATE TABLE interaction(x)").unwrap()
                }),
            ),
            (
                "trigger",
                forge(&f, |c| {
                    c.execute_batch(
                        "CREATE TRIGGER t AFTER INSERT ON person BEGIN DELETE FROM meta; END;",
                    )
                    .unwrap()
                }),
            ),
            (
                "view",
                forge(&f, |c| {
                    c.execute_batch("CREATE VIEW v AS SELECT 1").unwrap()
                }),
            ),
            (
                "extra column",
                forge(&f, |c| {
                    c.execute_batch("ALTER TABLE person ADD COLUMN notes TEXT")
                        .unwrap()
                }),
            ),
            (
                "application_id",
                forge(&f, |c| {
                    c.execute_batch("PRAGMA application_id = 7").unwrap()
                }),
            ),
            (
                "bad view id",
                forge(&f, |c| {
                    c.execute_batch("UPDATE meta SET value = '../../evil' WHERE key = 'view_id'")
                        .unwrap()
                }),
            ),
        ];
        for (what, bytes) in cases {
            let err = import(&f.inbox, &f.me.identity, &bytes, MB, 0).unwrap_err();
            assert!(matches!(err, Error::BadRequest(_)), "{what}: {err}");
        }
        assert!(import(&f.inbox, &f.me.identity, b"not age at all", MB, 0).is_err());
    }

    #[test]
    fn oversized_payloads_are_refused() {
        let f = fx("{}", Ttl::Never);
        let bytes = export_at(&f, testkit::NOW);
        let err = import(&f.inbox, &f.me.identity, &bytes, 100, testkit::NOW).unwrap_err();
        assert!(
            matches!(&err, Error::BadRequest(m) if m.contains("max_import_mb")),
            "{err}"
        );
    }

    #[test]
    fn star_search_merges_rows_shared_across_views() {
        let f = fx(r#"{"company":["Northwind"]}"#, Ttl::Never);
        let a = export_at(&f, testkit::NOW);
        // A second view of the same person from the "same" owner, new id.
        f.owner
            .execute("UPDATE view SET view_id = NULL", [])
            .unwrap();
        let b = export_at(&f, testkit::NOW);
        import(&f.inbox, &f.me.identity, &a, MB, testkit::NOW).unwrap();
        import(&f.inbox, &f.me.identity, &b, MB, testkit::NOW).unwrap();
        let res = search(
            &f.inbox,
            &ViewQuery {
                view: "*".into(),
                ..ViewQuery::default()
            },
            testkit::NOW,
        )
        .unwrap();
        assert_eq!(res.results.len(), 1);
        assert_eq!(res.results[0].also_in.len(), 1);
        assert!(matches!(
            search(
                &f.inbox,
                &ViewQuery {
                    view: "nope".into(),
                    ..ViewQuery::default()
                },
                0
            ),
            Err(Error::NotFound(_))
        ));
    }
}
