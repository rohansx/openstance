//! Views: saved queries that can be exported as age-encrypted `.ost` files
//! and imported by a colleague (P3-CORE-06..08).
//!
//! | module | role |
//! |---|---|
//! | this file | the `view` row: create, get, list, TTLs, recipients, JSON shapes |
//! | [`ost`] | export: run the query, materialize the allow-listed columns, encrypt |
//! | [`inbox`] | import, list and search views other people sent (never `graph.db`) |
//!
//! Spec: `docs/tech-spec/sharing-and-views.md`.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::Query;

pub mod inbox;
pub mod ost;

/// Seconds per day.
const DAY: i64 = 86_400;

/// How long a view stays importable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ttl {
    /// No expiry (discouraged; needs `--yes` on the CLI).
    Never,
    /// Seconds after creation.
    Secs(i64),
}

/// Parse `Nd | Nw | Nm | Ny | never` (`m` = 30 days, `y` = 365 days).
pub fn parse_ttl(s: &str) -> Result<Ttl> {
    let s = s.trim().to_ascii_lowercase();
    if s == "never" {
        return Ok(Ttl::Never);
    }
    let bad = || Error::BadRequest(format!("invalid ttl {s:?}: use Nd, Nw, Nm, Ny or never"));
    let (n, unit) = s.split_at(s.len().saturating_sub(1));
    let n: i64 = n.parse().map_err(|_| bad())?;
    let days = match unit {
        "d" => 1,
        "w" => 7,
        "m" => 30,
        "y" => 365,
        _ => return Err(bad()),
    };
    if n <= 0 || n > 100 * 365 / days {
        return Err(bad());
    }
    Ok(Ttl::Secs(n * days * DAY))
}

/// A view definition (`view` row with `origin = 'local'`).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewDef {
    /// `view.id`.
    pub id: i64,
    /// Unique name.
    pub name: String,
    /// ULID, set on first export.
    pub view_id: Option<String>,
    /// The saved query.
    pub query: Query,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds, `None` = never.
    pub expires_at: Option<i64>,
    /// `age1...` recipients (empty = encrypt to self).
    pub recipients: Vec<String>,
    /// Last export time.
    pub last_exported_at: Option<i64>,
}

/// Input of [`create`].
#[derive(Debug, Clone, PartialEq)]
pub struct CreateView {
    /// View name (letters, digits, `-`, `_`, `.`).
    pub name: String,
    /// The query, already parsed.
    pub query: Query,
    /// Expiry.
    pub ttl: Ttl,
    /// Resolved `age1...` recipients.
    pub recipients: Vec<String>,
    /// Replace an existing view of the same name.
    pub replace: bool,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "*"
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Store a view definition. An existing name is replaced only with
/// `replace`; the view id (ULID) survives a replace so recipients keep
/// replacing their older copy.
pub fn create(conn: &Connection, req: &CreateView, now: i64) -> Result<ViewDef> {
    if !valid_name(&req.name) {
        return Err(Error::BadRequest(format!(
            "invalid view name {:?}: use letters, digits, '-', '_' or '.'",
            req.name
        )));
    }
    for r in &req.recipients {
        crate::keys::parse_recipient(r)?;
    }
    let query_json = serde_json::to_string(&req.query)?;
    let expires_at = match req.ttl {
        Ttl::Never => None,
        Ttl::Secs(s) => Some(now + s),
    };
    let recipients = req.recipients.join("\n");
    let existing = find(conn, &req.name)?;
    match existing {
        Some(_) if !req.replace => {
            return Err(Error::BadRequest(format!(
                "view {:?} already exists; pass --yes to replace it",
                req.name
            )))
        }
        Some(v) => {
            conn.execute(
                "UPDATE view SET query_json = ?2, created_at = ?3, expires_at = ?4,
                    recipient_pubkey = ?5 WHERE id = ?1",
                params![v.id, query_json, now, expires_at, recipients],
            )?;
        }
        None => {
            conn.execute(
                "INSERT INTO view(name, query_json, created_at, expires_at, recipient_pubkey, origin)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'local')",
                params![req.name, query_json, now, expires_at, recipients],
            )?;
        }
    }
    get(conn, &req.name)
}

fn row_to_def(r: &rusqlite::Row<'_>) -> rusqlite::Result<(ViewDef, String)> {
    let recipients: Option<String> = r.get(6)?;
    Ok((
        ViewDef {
            id: r.get(0)?,
            name: r.get(1)?,
            view_id: r.get(2)?,
            query: Query::default(),
            created_at: r.get(4)?,
            expires_at: r.get(5)?,
            recipients: recipients
                .unwrap_or_default()
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect(),
            last_exported_at: r.get(7)?,
        },
        r.get(3)?,
    ))
}

const SELECT: &str = "SELECT id, name, view_id, query_json, created_at, expires_at,
    recipient_pubkey, last_exported_at FROM view WHERE origin = 'local'";

fn parse_def((mut d, q): (ViewDef, String)) -> Result<ViewDef> {
    d.query = serde_json::from_str(&q)?;
    Ok(d)
}

/// A local view by name, if any.
pub fn find(conn: &Connection, name: &str) -> Result<Option<ViewDef>> {
    conn.query_row(&format!("{SELECT} AND name = ?1"), [name], row_to_def)
        .optional()?
        .map(parse_def)
        .transpose()
}

/// A local view by name.
pub fn get(conn: &Connection, name: &str) -> Result<ViewDef> {
    find(conn, name)?.ok_or_else(|| Error::NotFound(format!("view {name:?}")))
}

/// Every local view, by name.
pub fn list_local(conn: &Connection) -> Result<Vec<ViewDef>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY name"))?;
    let rows = stmt
        .query_map([], row_to_def)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(parse_def).collect()
}

/// Delete a local view definition. Files already sent are unaffected.
pub fn remove(conn: &Connection, name: &str) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM view WHERE origin = 'local' AND name = ?1",
        [name],
    )?;
    if n == 0 {
        return Err(Error::NotFound(format!("view {name:?}")));
    }
    Ok(())
}

/// Resolve `--to` values: `age1...` keys, `[share.recipients]` aliases, and
/// `self` (needs `self_pubkey`). Duplicates are dropped, order kept.
pub fn resolve_recipients(
    cfg: &Config,
    to: &[String],
    self_pubkey: Option<&str>,
) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for t in to {
        let t = t.trim();
        let key = if t == "self" {
            self_pubkey
                .ok_or_else(|| Error::BadRequest("--to self needs an age key".into()))?
                .to_string()
        } else if let Some(k) = cfg.share.recipients.get(t) {
            k.clone()
        } else if t.starts_with("age1") {
            t.to_string()
        } else {
            return Err(Error::BadRequest(format!(
                "unknown recipient {t:?}: pass an age1... key, `self`, or an alias from [share.recipients]"
            )));
        };
        crate::keys::parse_recipient(&key)?;
        if !out.contains(&key) {
            out.push(key);
        }
    }
    Ok(out)
}

/// `strong | medium | weak` for a score (`share.bucket_*`).
pub fn score_bucket(cfg: &Config, score: f64) -> &'static str {
    if score >= cfg.share.bucket_strong {
        "strong"
    } else if score >= cfg.share.bucket_medium {
        "medium"
    } else {
        "weak"
    }
}

/// `YYYY-MM` (UTC) of a unix time.
pub fn month(ts: i64) -> Option<String> {
    let t = time::OffsetDateTime::from_unix_timestamp(ts).ok()?;
    Some(format!("{:04}-{:02}", t.year(), u8::from(t.month())))
}

// ---------------------------------------------------------------------------
// JSON shapes (interfaces.md 1.10)
// ---------------------------------------------------------------------------

/// The claimed owner of a view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewOwner {
    /// `age1...` (claimed for imported views).
    pub pubkey: Option<String>,
    /// Free-text label (claimed).
    pub label: Option<String>,
}

/// `ViewSummary`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewSummary {
    /// The ULID (`null` for an own view never exported).
    pub id: Option<String>,
    /// Name.
    pub name: String,
    /// `own | imported`.
    pub direction: String,
    /// Owner (claimed for imported views).
    pub owner: ViewOwner,
    /// ISO time.
    pub created_at: String,
    /// ISO time, `null` = never.
    pub expires_at: Option<String>,
    /// Rows: in the file (imported) or that an export would write now (own).
    pub rows: u64,
    /// The query.
    pub query: serde_json::Value,
    /// Own views: `age1...` recipients.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub recipients: Vec<String>,
    /// Own views: people left out as do-not-share right now.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub excluded_do_not_share: Option<u64>,
    /// Imported views: ISO export time.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exported_at: Option<String>,
}

/// `ViewPerson`: one row of an imported view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewPerson {
    /// Name.
    pub display_name: Option<String>,
    /// Title.
    pub title: Option<String>,
    /// Company.
    pub company: Option<String>,
    /// Location.
    pub location: Option<String>,
    /// `strong | medium | weak`.
    pub score_bucket: String,
    /// `YYYY-MM`.
    pub last_contact_month: Option<String>,
    /// Emails.
    pub emails: Vec<String>,
    /// LinkedIn slug.
    pub linkedin: Option<String>,
}

/// The view a search hit came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewRef {
    /// ULID.
    pub id: String,
    /// Name.
    pub name: String,
    /// Claimed owner.
    pub owner: ViewOwner,
}

/// One `search_view` result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewSearchResult {
    /// Source view (the first one, when merged across views).
    pub view: ViewRef,
    /// Every view the row came from when `view = "*"` merged it.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub also_in: Vec<ViewRef>,
    /// The person.
    pub person: ViewPerson,
    /// Text relevance in `[0, 1]` (1 without text).
    pub relevance: f64,
}

/// `{ v, results }` for `view search --json` and MCP `search_view`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewSearchResponse {
    /// Schema version.
    pub v: u32,
    /// Hits, best first.
    pub results: Vec<ViewSearchResult>,
}

/// `{ v, views }` for `view list --json` and MCP `list_views`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewList {
    /// Schema version.
    pub v: u32,
    /// Views.
    pub views: Vec<ViewSummary>,
}

/// `own | imported | all`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// Views I defined.
    Own,
    /// Views others sent me.
    Imported,
    /// Both.
    #[default]
    All,
}

impl Direction {
    /// Parse `own | imported | all`.
    pub fn parse(s: &str) -> Option<Direction> {
        match s.trim().to_ascii_lowercase().as_str() {
            "own" => Some(Direction::Own),
            "imported" => Some(Direction::Imported),
            "all" => Some(Direction::All),
            _ => None,
        }
    }
}

/// Summaries of local views, with the row counts an export would write now.
pub fn summarize_local(
    conn: &Connection,
    cfg: &Config,
    owner: &ViewOwner,
) -> Result<Vec<ViewSummary>> {
    let mut out = Vec::new();
    for d in list_local(conn)? {
        let sel = ost::select(conn, cfg, &d.query)?;
        out.push(ViewSummary {
            id: d.view_id.clone(),
            name: d.name.clone(),
            direction: "own".into(),
            owner: owner.clone(),
            created_at: crate::json::iso(d.created_at),
            expires_at: crate::json::iso_opt(d.expires_at),
            rows: sel.people.len() as u64,
            query: serde_json::to_value(&d.query)?,
            recipients: d.recipients.clone(),
            excluded_do_not_share: Some(sel.excluded_do_not_share),
            exported_at: None,
        });
    }
    Ok(out)
}

/// Where the age key and imported views live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareDirs {
    /// `share.keypair`, expanded.
    pub key: std::path::PathBuf,
    /// `<home>/views`: imported views.
    pub views: std::path::PathBuf,
}

impl ShareDirs {
    /// From the effective config and paths.
    pub fn new(
        cfg: &Config,
        paths: &crate::config::Paths,
        user_home: Option<&std::path::Path>,
    ) -> ShareDirs {
        ShareDirs {
            key: crate::keys::key_path(cfg, paths, user_home),
            views: paths.home.join("views"),
        }
    }
}

/// The claimed owner label: `share.owner_label`, else `$USER`.
pub fn owner_label(cfg: &Config, user: Option<&str>) -> String {
    let l = cfg.share.owner_label.trim();
    if l.is_empty() {
        user.unwrap_or_default().to_string()
    } else {
        l.to_string()
    }
}

impl crate::Graph {
    /// `view create` (P3-CORE-06).
    pub fn view_create(&mut self, req: &CreateView) -> Result<ViewDef> {
        let now = self.now();
        self.write(|tx, _| create(tx, req, now))
    }

    /// One local view definition.
    pub fn view_get(&self, name: &str) -> Result<ViewDef> {
        get(&self.conn, name)
    }

    /// The people an export of `name` would write now.
    pub fn view_selection(&self, name: &str) -> Result<ost::Selection> {
        ost::select(&self.conn, &self.cfg, &get(&self.conn, name)?.query)
    }

    /// `view export` (P3-CORE-07): encrypted `.ost` bytes plus a report.
    pub fn view_export(
        &mut self,
        name: &str,
        opts: &ost::ExportOptions<'_>,
    ) -> Result<ost::ExportedView> {
        let now = self.now();
        self.write(|tx, cfg| ost::export(tx, cfg, name, opts, now))
    }

    /// `view rm` for a local view.
    pub fn view_remove(&mut self, name: &str) -> Result<()> {
        self.write(|tx, _| remove(tx, name))
    }

    /// Local view summaries (`direction = own`).
    pub fn view_summaries(&self, owner: &ViewOwner) -> Result<Vec<ViewSummary>> {
        summarize_local(&self.conn, &self.cfg, owner)
    }
}

/// A map of the given pairs (meta table rows).
pub(crate) fn meta_map(conn: &Connection) -> Result<BTreeMap<String, String>> {
    let mut stmt = conn.prepare("SELECT key, value FROM meta")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<BTreeMap<String, String>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::testkit;

    fn q() -> Query {
        serde_json::from_str(r#"{"title":["investor"],"text":"devtools"}"#).unwrap()
    }

    #[test]
    fn ttl_parses_units_and_never() {
        assert_eq!(parse_ttl("30d").unwrap(), Ttl::Secs(30 * DAY));
        assert_eq!(parse_ttl("2w").unwrap(), Ttl::Secs(14 * DAY));
        assert_eq!(parse_ttl("1m").unwrap(), Ttl::Secs(30 * DAY));
        assert_eq!(parse_ttl("1y").unwrap(), Ttl::Secs(365 * DAY));
        assert_eq!(parse_ttl("never").unwrap(), Ttl::Never);
        for bad in ["", "d", "0d", "-1d", "3h", "abc"] {
            assert!(parse_ttl(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn create_stores_query_json_that_round_trips() {
        let c = testkit::graph();
        let req = CreateView {
            name: "devtools-investors".into(),
            query: q(),
            ttl: Ttl::Secs(30 * DAY),
            recipients: vec![],
            replace: false,
        };
        let d = create(&c, &req, 1000).unwrap();
        assert_eq!(d.query, q());
        assert_eq!(d.created_at, 1000);
        assert_eq!(d.expires_at, Some(1000 + 30 * DAY));
        assert!(d.view_id.is_none());
        let stored: String = c
            .query_row(
                "SELECT query_json FROM view WHERE name = 'devtools-investors'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let back: Query = serde_json::from_str(&stored).unwrap();
        assert_eq!(back, q());
        assert_eq!(serde_json::to_string(&back).unwrap(), stored);
    }

    #[test]
    fn create_refuses_to_replace_without_flag_and_keeps_view_id() {
        let c = testkit::graph();
        let mut req = CreateView {
            name: "v".into(),
            query: q(),
            ttl: Ttl::Never,
            recipients: vec![],
            replace: false,
        };
        create(&c, &req, 1).unwrap();
        c.execute("UPDATE view SET view_id = 'ULIDX' WHERE name = 'v'", [])
            .unwrap();
        assert!(matches!(create(&c, &req, 2), Err(Error::BadRequest(_))));
        req.replace = true;
        req.ttl = Ttl::Secs(DAY);
        let d = create(&c, &req, 5).unwrap();
        assert_eq!(d.view_id.as_deref(), Some("ULIDX"));
        assert_eq!(d.expires_at, Some(5 + DAY));
        assert_eq!(list_local(&c).unwrap().len(), 1);
    }

    #[test]
    fn create_validates_names_and_recipients() {
        let c = testkit::graph();
        let mut req = CreateView {
            name: "bad name".into(),
            query: q(),
            ttl: Ttl::Never,
            recipients: vec![],
            replace: false,
        };
        assert!(create(&c, &req, 1).is_err());
        req.name = "ok".into();
        req.recipients = vec!["age1nope".into()];
        assert!(create(&c, &req, 1).is_err());
    }

    #[test]
    fn recipients_resolve_aliases_self_and_keys() {
        let dir = tempfile::tempdir().unwrap();
        let me = crate::keys::ensure(&dir.path().join("me"), 0)
            .unwrap()
            .public();
        let alice = crate::keys::ensure(&dir.path().join("a"), 0)
            .unwrap()
            .public();
        let mut cfg = Config::default();
        cfg.share.recipients.insert("alice".into(), alice.clone());
        let got = resolve_recipients(
            &cfg,
            &["alice".into(), "self".into(), alice.clone()],
            Some(&me),
        )
        .unwrap();
        assert_eq!(got, vec![alice, me]);
        assert!(resolve_recipients(&cfg, &["bob".into()], None).is_err());
        assert!(resolve_recipients(&cfg, &["self".into()], None).is_err());
    }

    #[test]
    fn buckets_and_months() {
        let cfg = Config::default();
        assert_eq!(score_bucket(&cfg, 0.8), "strong");
        assert_eq!(score_bucket(&cfg, 0.6), "strong");
        assert_eq!(score_bucket(&cfg, 0.3), "medium");
        assert_eq!(score_bucket(&cfg, 0.29), "weak");
        assert_eq!(month(testkit::NOW).as_deref(), Some("2026-10"));
    }

    #[test]
    fn remove_deletes_or_reports_not_found() {
        let c = testkit::graph();
        let req = CreateView {
            name: "v".into(),
            query: q(),
            ttl: Ttl::Never,
            recipients: vec![],
            replace: false,
        };
        create(&c, &req, 1).unwrap();
        remove(&c, "v").unwrap();
        assert!(matches!(remove(&c, "v"), Err(Error::NotFound(_))));
    }
}
