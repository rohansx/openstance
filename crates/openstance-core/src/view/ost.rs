//! `view export`: run the saved query now, materialize only the allow-listed
//! columns into a fresh SQLite file in a private temp dir, encrypt it with
//! age, and return the ciphertext (P3-CORE-07). The plaintext file never
//! leaves the temp dir, which is deleted on success and on failure.
//!
//! Spec: `docs/tech-spec/sharing-and-views.md` section 4.

use std::io::Write;
use std::path::Path;

use rusqlite::{params, Connection};
use serde::Serialize;

use super::{get, month, score_bucket, ViewDef};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::export::ExportOptions as PeopleExportOptions;
use crate::{PersonId, Query};

/// `PRAGMA application_id` of a `.ost` payload: `'OST1'`.
pub const APPLICATION_ID: i64 = 0x4F53_5431;
/// `PRAGMA user_version` (format version) written by this build.
pub const FORMAT_VERSION: i64 = 1;

/// The payload schema (format version 1). The import side validates
/// against exactly these tables and columns.
pub const SCHEMA: &str = "
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE person (
  id                 INTEGER PRIMARY KEY,
  display_name       TEXT,
  title              TEXT,
  company            TEXT,
  location           TEXT,
  score_bucket       TEXT NOT NULL CHECK (score_bucket IN ('strong','medium','weak')),
  last_contact_month TEXT
);
CREATE TABLE identity (
  person_id INTEGER NOT NULL REFERENCES person(id),
  kind      TEXT    NOT NULL CHECK (kind IN ('email','linkedin_url')),
  value     TEXT    NOT NULL,
  PRIMARY KEY (person_id, kind, value)
);
";

/// The allow-list: every table in a `.ost` and its columns, in order.
pub const ALLOWED_COLUMNS: &[(&str, &[&str])] = &[
    ("meta", &["key", "value"]),
    (
        "person",
        &[
            "id",
            "display_name",
            "title",
            "company",
            "location",
            "score_bucket",
            "last_contact_month",
        ],
    ),
    ("identity", &["person_id", "kind", "value"]),
];

/// The people a view's query selects right now, after the view exclusions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// People to write, in query order.
    pub people: Vec<PersonId>,
    /// Matched but `do_not_share`.
    pub excluded_do_not_share: u64,
    /// Matched but bulk senders.
    pub excluded_bulk: u64,
}

/// Run `q` (limit ignored) and drop do-not-share, bulk and self rows.
pub fn select(conn: &Connection, cfg: &Config, q: &Query) -> Result<Selection> {
    let mut q = q.clone();
    // Bulk is excluded below (and counted), whatever the query says.
    q.include_bulk = true;
    let ids = crate::export::select(
        conn,
        cfg,
        &PeopleExportOptions {
            query: Some(q),
            include_bulk: true,
            include_do_not_share: true,
        },
    )?;
    let mut sel = Selection::default();
    let mut stmt =
        conn.prepare_cached("SELECT do_not_share, is_bulk, is_self FROM person WHERE id = ?1")?;
    for id in ids {
        let (dns, bulk, me): (i64, i64, i64) =
            stmt.query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        if me != 0 {
            continue;
        }
        if dns != 0 {
            sel.excluded_do_not_share += 1;
        } else if bulk != 0 {
            sel.excluded_bulk += 1;
        } else {
            sel.people.push(id);
        }
    }
    Ok(sel)
}

/// Who the file claims to come from, and how to write it.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions<'a> {
    /// My `age1...` public key (also the recipient when the view has none).
    pub owner_pubkey: &'a str,
    /// `share.owner_label` or `$USER`.
    pub owner_label: &'a str,
    /// ASCII armor.
    pub armor: bool,
    /// Parent of the private temp dir (default: the system temp dir).
    pub tmp_parent: Option<&'a Path>,
    /// `openstance_version` written to meta.
    pub version: &'a str,
}

/// What `view export` wrote.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportViewReport {
    /// Schema version.
    pub v: u32,
    /// The view ULID.
    pub view_id: String,
    /// Name.
    pub name: String,
    /// People written.
    pub people: u64,
    /// Identities written.
    pub identities: u64,
    /// Left out: do-not-share.
    pub excluded_do_not_share: u64,
    /// Left out: bulk senders.
    pub excluded_bulk: u64,
    /// Encrypted to these keys.
    pub recipients: Vec<String>,
    /// ISO expiry.
    pub expires_at: Option<String>,
    /// ASCII armored.
    pub armor: bool,
    /// Encrypted size in bytes.
    pub bytes: u64,
}

/// The encrypted file plus its report.
#[derive(Debug, Clone)]
pub struct ExportedView {
    /// age ciphertext (binary or armored).
    pub bytes: Vec<u8>,
    /// Counts.
    pub report: ExportViewReport,
}

/// Export view `name`: assign its ULID on first export, materialize,
/// encrypt, and record `last_exported_at`. Run inside a write transaction.
pub fn export(
    conn: &Connection,
    cfg: &Config,
    name: &str,
    opts: &ExportOptions<'_>,
    now: i64,
) -> Result<ExportedView> {
    let mut def = get(conn, name)?;
    if def.expires_at.is_some_and(|e| e <= now) {
        return Err(Error::ViewExpired(format!(
            "view {name:?} expired; re-run `openstance view create {name} --yes --ttl ...` to extend it"
        )));
    }
    let view_id = match &def.view_id {
        Some(id) => id.clone(),
        None => {
            let id = ulid::Ulid::from_datetime(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(now.max(0).unsigned_abs()),
            )
            .to_string();
            conn.execute(
                "UPDATE view SET view_id = ?2 WHERE id = ?1",
                params![def.id, id],
            )?;
            def.view_id = Some(id.clone());
            id
        }
    };
    let recipients = if def.recipients.is_empty() {
        vec![opts.owner_pubkey.to_string()]
    } else {
        def.recipients.clone()
    };
    let tmp = match opts.tmp_parent {
        Some(p) => tempfile::Builder::new().prefix("ost-").tempdir_in(p),
        None => tempfile::Builder::new().prefix("openstance-ost-").tempdir(),
    }
    .map_err(|e| Error::io("creating", "a private temp dir", e))?;
    let db_path = tmp.path().join("view.db");
    let sel = select(conn, cfg, &def.query)?;
    let (people, identities) = materialize(conn, cfg, &def, &view_id, &sel, opts, now, &db_path)?;
    let plaintext = std::fs::read(&db_path).map_err(|e| Error::io("reading", &db_path, e))?;
    let parsed = recipients
        .iter()
        .map(|r| crate::keys::parse_recipient(r))
        .collect::<Result<Vec<_>>>()?;
    let bytes = encrypt(&parsed, &plaintext, opts.armor)?;
    drop(plaintext);
    tmp.close()
        .map_err(|e| Error::io("removing", "the private temp dir", e))?;
    conn.execute(
        "UPDATE view SET last_exported_at = ?2 WHERE id = ?1",
        params![def.id, now],
    )?;
    Ok(ExportedView {
        report: ExportViewReport {
            v: crate::json::V,
            view_id,
            name: def.name.clone(),
            people,
            identities,
            excluded_do_not_share: sel.excluded_do_not_share,
            excluded_bulk: sel.excluded_bulk,
            recipients,
            expires_at: crate::json::iso_opt(def.expires_at),
            armor: opts.armor,
            bytes: bytes.len() as u64,
        },
        bytes,
    })
}

/// Encrypt `plaintext` to every recipient.
pub fn encrypt(
    recipients: &[age::x25519::Recipient],
    plaintext: &[u8],
    armor: bool,
) -> Result<Vec<u8>> {
    let enc = age::Encryptor::with_recipients(recipients.iter().map(|r| r as &dyn age::Recipient))
        .map_err(|e| Error::Internal(format!("age: {e}")))?;
    let io = |e: std::io::Error| Error::io("encrypting", "view", e);
    let mut out = Vec::with_capacity(plaintext.len() + 512);
    if armor {
        let a = age::armor::ArmoredWriter::wrap_output(&mut out, age::armor::Format::AsciiArmor)
            .map_err(io)?;
        let mut w = enc.wrap_output(a).map_err(io)?;
        w.write_all(plaintext).map_err(io)?;
        w.finish().and_then(|a| a.finish()).map_err(io)?;
    } else {
        let mut w = enc.wrap_output(&mut out).map_err(io)?;
        w.write_all(plaintext).map_err(io)?;
        w.finish().map_err(io)?;
    }
    Ok(out)
}

/// Shuffle key for view-local ids, so the file leaks neither the graph's
/// size nor its insertion order.
fn shuffle_key(salt: &[u8; 16], id: PersonId) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(salt);
    h.update(&id.to_le_bytes());
    *h.finalize().as_bytes()
}

/// `display_name, title, company, location, score, last_contact`.
type PersonRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    f64,
    Option<i64>,
);

#[allow(clippy::too_many_arguments)]
fn materialize(
    conn: &Connection,
    cfg: &Config,
    def: &ViewDef,
    view_id: &str,
    sel: &Selection,
    opts: &ExportOptions<'_>,
    now: i64,
    path: &Path,
) -> Result<(u64, u64)> {
    let out = Connection::open(path)?;
    out.execute_batch(&format!(
        "PRAGMA application_id = {APPLICATION_ID};
         PRAGMA user_version = {FORMAT_VERSION};
         PRAGMA journal_mode = DELETE;"
    ))?;
    out.execute_batch(SCHEMA)?;

    let salt = ulid::Ulid::new().0.to_le_bytes();
    let mut order: Vec<PersonId> = sel.people.clone();
    order.sort_by_cached_key(|id| shuffle_key(&salt, *id));

    let mut person_q = conn.prepare_cached(
        "SELECT display_name, title, company, location, score, last_contact
         FROM person WHERE id = ?1",
    )?;
    let mut ident_q = conn.prepare_cached(
        "SELECT kind, value FROM identity
         WHERE person_id = ?1 AND kind IN ('email','linkedin_url') ORDER BY kind, value",
    )?;
    let mut n_ident = 0u64;
    out.execute_batch("BEGIN")?;
    for (i, id) in order.iter().enumerate() {
        let local = i64::try_from(i).unwrap_or(i64::MAX) + 1;
        let (name, title, company, location, score, last): PersonRow =
            person_q.query_row([id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?;
        out.execute(
            "INSERT INTO person(id, display_name, title, company, location, score_bucket,
                last_contact_month) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                local,
                name,
                title,
                company,
                location,
                score_bucket(cfg, score),
                last.and_then(month)
            ],
        )?;
        let idents = ident_q
            .query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (kind, value) in idents {
            n_ident += out.execute(
                "INSERT OR IGNORE INTO identity(person_id, kind, value) VALUES (?1, ?2, ?3)",
                params![local, kind, value],
            )? as u64;
        }
    }
    let meta: Vec<(&str, String)> = vec![
        ("format_version", FORMAT_VERSION.to_string()),
        ("view_id", view_id.to_string()),
        ("name", def.name.clone()),
        ("owner_pubkey", opts.owner_pubkey.to_string()),
        ("owner_label", opts.owner_label.to_string()),
        ("created_at", def.created_at.to_string()),
        ("exported_at", now.to_string()),
        (
            "expires_at",
            def.expires_at.map(|e| e.to_string()).unwrap_or_default(),
        ),
        ("query_json", serde_json::to_string(&def.query)?),
        ("row_count", order.len().to_string()),
        (
            "score_buckets",
            format!(
                "strong>={:.2};medium>={:.2};weak",
                cfg.share.bucket_strong, cfg.share.bucket_medium
            ),
        ),
        ("openstance_version", opts.version.to_string()),
    ];
    for (k, v) in meta {
        out.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)",
            params![k, v],
        )?;
    }
    out.execute_batch("COMMIT; VACUUM;")?;
    out.close().map_err(|(_, e)| Error::Db(e))?;
    Ok((order.len() as u64, n_ident))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::testkit;
    use crate::view::{create, CreateView, Ttl};

    struct Fx {
        conn: Connection,
        cfg: Config,
        key: crate::keys::KeyPair,
        _dir: tempfile::TempDir,
    }

    fn fx(query: &str) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let key = crate::keys::ensure(&dir.path().join("age.key"), 0).unwrap();
        let conn = testkit::indexed();
        create(
            &conn,
            &CreateView {
                name: "v".into(),
                query: serde_json::from_str(query).unwrap(),
                ttl: Ttl::Secs(30 * 86_400),
                recipients: vec![],
                replace: false,
            },
            testkit::NOW,
        )
        .unwrap();
        Fx {
            conn,
            cfg: Config::default(),
            key,
            _dir: dir,
        }
    }

    fn opts<'a>(fx: &'a Fx, pk: &'a str, tmp: Option<&'a Path>) -> ExportOptions<'a> {
        let _ = fx;
        ExportOptions {
            owner_pubkey: pk,
            owner_label: "owen",
            armor: false,
            tmp_parent: tmp,
            version: "test",
        }
    }

    fn decrypt_to_db(fx: &Fx, bytes: &[u8]) -> (tempfile::TempDir, Connection, Vec<u8>) {
        let plain = age::decrypt(&fx.key.identity, bytes).unwrap();
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.db");
        std::fs::write(&p, &plain).unwrap();
        let c = Connection::open(&p).unwrap();
        (d, c, plain)
    }

    #[test]
    fn export_contains_only_allow_listed_tables_and_columns() {
        let f = fx("{}");
        let pk = f.key.public();
        let out = export(&f.conn, &f.cfg, "v", &opts(&f, &pk, None), testkit::NOW).unwrap();
        let (_d, db, plain) = decrypt_to_db(&f, &out.bytes);
        let objects: Vec<(String, String)> = db
            .prepare("SELECT type, name FROM sqlite_master ORDER BY name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        for (ty, name) in &objects {
            match ty.as_str() {
                "table" => assert!(ALLOWED_COLUMNS.iter().any(|(t, _)| t == name), "{name}"),
                "index" => assert!(name.starts_with("sqlite_autoindex_"), "{name}"),
                other => panic!("unexpected {other} {name}"),
            }
        }
        for (table, cols) in ALLOWED_COLUMNS {
            let got: Vec<String> = db
                .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            assert_eq!(got, cols.to_vec(), "{table}");
        }
        let app: i64 = db
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .unwrap();
        assert_eq!(app, APPLICATION_ID);
        // No interaction subjects, notes, bios or tags anywhere in the plaintext.
        let text = String::from_utf8_lossy(&plain);
        for secret in [
            "seed round",
            "Board deck",
            "devtools dinner",
            "secret-note-text",
            "Angel investor",
            "angels",
            "investor-intro",
            "news@digest.example",
            "owner@example.com",
        ] {
            assert!(!text.contains(secret), "{secret} leaked");
        }
        // Self and bulk never exported.
        let names: Vec<String> = db
            .prepare("SELECT display_name FROM person")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(!names.contains(&"Owen Ryder".to_string()));
        assert!(!names.contains(&"Digest Weekly".to_string()));
        assert_eq!(out.report.people, names.len() as u64);
        assert_eq!(out.report.excluded_bulk, 1);
    }

    #[test]
    fn do_not_share_people_are_absent_and_counted() {
        let f = fx(r#"{"company":["Northwind"]}"#);
        f.conn
            .execute("UPDATE person SET do_not_share = 1 WHERE id = 2", [])
            .unwrap();
        let pk = f.key.public();
        let out = export(&f.conn, &f.cfg, "v", &opts(&f, &pk, None), testkit::NOW).unwrap();
        let (_d, db, plain) = decrypt_to_db(&f, &out.bytes);
        let n: i64 = db
            .query_row("SELECT count(*) FROM person", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        assert_eq!(out.report.excluded_do_not_share, 1);
        assert!(!String::from_utf8_lossy(&plain).contains("Priya"));
    }

    #[test]
    fn rows_carry_buckets_months_and_only_email_and_linkedin() {
        let f = fx(r#"{"company":["Northwind"]}"#);
        f.conn
            .execute(
                "INSERT INTO identity(kind, value, source_id, person_id) VALUES ('phone', '+15550100', 1, 2)",
                [],
            )
            .unwrap();
        let pk = f.key.public();
        let out = export(&f.conn, &f.cfg, "v", &opts(&f, &pk, None), testkit::NOW).unwrap();
        let (_d, db, plain) = decrypt_to_db(&f, &out.bytes);
        let (bucket, month): (String, String) = db
            .query_row(
                "SELECT score_bucket, last_contact_month FROM person",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(bucket, "strong");
        assert_eq!(month, "2026-10");
        let idents: Vec<(String, String)> = db
            .prepare("SELECT kind, value FROM identity ORDER BY kind")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            idents,
            vec![
                ("email".into(), "priya@northwind.example".into()),
                ("linkedin_url".into(), "priya-raman-nw".into())
            ]
        );
        assert!(!String::from_utf8_lossy(&plain).contains("+15550100"));
        assert_eq!(out.report.identities, 2);
        // No exact score.
        assert!(!String::from_utf8_lossy(&plain).contains("0.8"));
    }

    #[test]
    fn view_id_is_assigned_once_and_meta_is_complete() {
        let f = fx("{}");
        let pk = f.key.public();
        let a = export(&f.conn, &f.cfg, "v", &opts(&f, &pk, None), testkit::NOW).unwrap();
        let b = export(&f.conn, &f.cfg, "v", &opts(&f, &pk, None), testkit::NOW + 1).unwrap();
        assert_eq!(a.report.view_id, b.report.view_id);
        assert_eq!(a.report.view_id.len(), 26);
        let (_d, db, _) = decrypt_to_db(&f, &b.bytes);
        let meta = crate::view::meta_map(&db).unwrap();
        for k in [
            "format_version",
            "view_id",
            "name",
            "owner_pubkey",
            "owner_label",
            "created_at",
            "exported_at",
            "expires_at",
            "query_json",
            "row_count",
            "score_buckets",
            "openstance_version",
        ] {
            assert!(meta.contains_key(k), "{k}");
        }
        assert_eq!(meta["view_id"], a.report.view_id);
        assert_eq!(meta["owner_pubkey"], pk);
        let last: i64 = f
            .conn
            .query_row(
                "SELECT last_exported_at FROM view WHERE name='v'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(last, testkit::NOW + 1);
    }

    #[test]
    fn plaintext_temp_dir_is_removed_on_success_and_failure() {
        let f = fx("{}");
        let tmp = tempfile::tempdir().unwrap();
        let pk = f.key.public();
        export(
            &f.conn,
            &f.cfg,
            "v",
            &opts(&f, &pk, Some(tmp.path())),
            testkit::NOW,
        )
        .unwrap();
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
        // A broken recipient fails before anything is written.
        f.conn
            .execute(
                "UPDATE view SET recipient_pubkey = 'age1broken' WHERE name='v'",
                [],
            )
            .unwrap();
        assert!(export(
            &f.conn,
            &f.cfg,
            "v",
            &opts(&f, &pk, Some(tmp.path())),
            testkit::NOW
        )
        .is_err());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn expired_own_view_is_not_exported() {
        let f = fx("{}");
        let pk = f.key.public();
        let err = export(
            &f.conn,
            &f.cfg,
            "v",
            &opts(&f, &pk, None),
            testkit::NOW + 31 * 86_400,
        )
        .unwrap_err();
        assert!(matches!(err, Error::ViewExpired(_)), "{err}");
    }

    #[test]
    fn armored_export_decrypts() {
        let f = fx("{}");
        let pk = f.key.public();
        let mut o = opts(&f, &pk, None);
        o.armor = true;
        let out = export(&f.conn, &f.cfg, "v", &o, testkit::NOW).unwrap();
        assert!(out.bytes.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----"));
        let (_d, db, _) = decrypt_to_db(&f, &out.bytes);
        let n: i64 = db
            .query_row("SELECT count(*) FROM person", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n as u64, out.report.people);
    }
}
