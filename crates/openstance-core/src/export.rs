//! `openstance export --format csv|json`: people, identities, score
//! components and applied overrides. `do_not_share` and bulk people are
//! excluded unless asked. CSV cells starting with `= + - @` get a leading
//! apostrophe (formula-injection guard ported from v0.1).
//! Spec: `docs/tech-spec/data-model.md` section 9 (P1-CORE-18).
//!
//! Owned by the "search" stream.

use std::borrow::Cow;
use std::io::Write;

use openstance_model::Query;
use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::json::{PersonDetail, V};
use crate::search::person::{self, DetailOptions};
use crate::search::{filters, fts, fuse};
use crate::PersonId;

/// Output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// One row per person.
    Csv,
    /// `{"v": 1, "people": [PersonDetail-like objects]}`.
    Json,
}

impl ExportFormat {
    /// `csv` or `json`.
    pub fn parse(s: &str) -> Option<ExportFormat> {
        match s.trim().to_ascii_lowercase().as_str() {
            "csv" => Some(ExportFormat::Csv),
            "json" => Some(ExportFormat::Json),
            _ => None,
        }
    }
}

/// What to export.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportOptions {
    /// Only people matching this query (`--query` + filters).
    pub query: Option<Query>,
    /// `--include-bulk`.
    pub include_bulk: bool,
    /// `--include-do-not-share`.
    pub include_do_not_share: bool,
}

/// Counts written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ExportReport {
    /// People written.
    pub people: u64,
    /// People left out because of `do_not_share`.
    pub excluded_do_not_share: u64,
}

/// CSV columns, in order.
pub const CSV_COLUMNS: &[&str] = &[
    "id",
    "display_name",
    "title",
    "company",
    "location",
    "bio",
    "tags",
    "identities",
    "first_seen",
    "last_contact",
    "score",
    "recency",
    "frequency",
    "reciprocity",
    "meetings",
    "breadth",
    "preset",
    "is_bulk",
    "do_not_share",
    "no_enrich",
    "overridden",
];

#[derive(Serialize)]
struct JsonExport<'a> {
    v: u32,
    people: &'a [PersonDetail],
}

/// The people to export, in structured order (or text relevance order).
fn select(conn: &Connection, cfg: &Config, opts: &ExportOptions) -> Result<Vec<PersonId>> {
    let mut q = opts.query.clone().unwrap_or_default();
    q.include_bulk = q.include_bulk || opts.include_bulk;
    let allowed = filters::allowed_ids(conn, &q)?;
    let Some(text) = q.text.as_deref().filter(|t| !t.trim().is_empty()) else {
        return Ok(allowed);
    };
    let restrict = q.has_structured_filters().then_some(allowed.as_slice());
    let hits = fts::candidates(conn, text, restrict, usize::MAX >> 1)?;
    let lift = cfg.search.relationship_lift;
    let scores: std::collections::BTreeMap<PersonId, f64> =
        person::people(conn, cfg, &hits.iter().map(|h| h.0).collect::<Vec<_>>())?
            .into_iter()
            .map(|p| (p.id, p.score))
            .collect();
    let fused = fuse::fuse(&hits, None, cfg.search.text_weight, lift, &|id| {
        scores.get(&id).copied().unwrap_or(0.0)
    });
    Ok(fused.into_iter().map(|f| f.id).collect())
}

fn is_do_not_share(conn: &Connection, id: PersonId) -> Result<bool> {
    let v: i64 = conn
        .prepare_cached("SELECT do_not_share FROM person WHERE id = ?1")?
        .query_row([id], |r| r.get(0))?;
    Ok(v != 0)
}

fn io(e: std::io::Error) -> Error {
    Error::io("writing", "export output", e)
}

/// Write the export.
pub fn export(
    conn: &Connection,
    cfg: &Config,
    format: ExportFormat,
    opts: &ExportOptions,
    out: &mut dyn Write,
    now: i64,
) -> Result<ExportReport> {
    let mut report = ExportReport::default();
    let mut ids = Vec::new();
    for id in select(conn, cfg, opts)? {
        if !opts.include_do_not_share && is_do_not_share(conn, id)? {
            report.excluded_do_not_share += 1;
            continue;
        }
        ids.push(id);
    }
    let opts_detail = DetailOptions {
        interactions: 0,
        mutuals: 0,
    };
    let mut details = Vec::with_capacity(ids.len());
    for id in &ids {
        details.push(person::detail(conn, cfg, *id, None, opts_detail, now)?);
    }
    report.people = details.len() as u64;
    match format {
        ExportFormat::Json => {
            serde_json::to_writer_pretty(
                &mut *out,
                &JsonExport {
                    v: V,
                    people: &details,
                },
            )?;
            out.write_all(b"\n").map_err(io)?;
        }
        ExportFormat::Csv => {
            let mut w = csv::Writer::from_writer(&mut *out);
            w.write_record(CSV_COLUMNS)
                .map_err(|e| Error::Internal(format!("csv: {e}")))?;
            for d in &details {
                let row = csv_row(d);
                w.write_record(row.iter().map(|c| escape_csv_cell(c).into_owned()))
                    .map_err(|e| Error::Internal(format!("csv: {e}")))?;
            }
            w.flush().map_err(io)?;
        }
    }
    Ok(report)
}

fn csv_row(d: &PersonDetail) -> Vec<String> {
    let p = &d.person;
    let c = &p.components;
    let opt = |v: &Option<String>| v.clone().unwrap_or_default();
    let identities: Vec<String> = d
        .identities
        .iter()
        .map(|i| format!("{}:{}", i.kind, i.value))
        .collect();
    let overridden: Vec<&str> = d.overrides.iter().map(|o| o.field.as_str()).collect();
    vec![
        p.id.to_string(),
        opt(&p.display_name),
        opt(&p.title),
        opt(&p.company),
        opt(&p.location),
        opt(&p.bio),
        p.tags.join("; "),
        identities.join("; "),
        opt(&p.first_seen),
        opt(&p.last_contact),
        p.score.to_string(),
        c.recency.to_string(),
        c.frequency.to_string(),
        c.reciprocity.to_string(),
        c.meetings.to_string(),
        c.breadth.to_string(),
        c.preset.clone(),
        p.is_bulk.to_string(),
        p.do_not_share.to_string(),
        p.no_enrich.to_string(),
        overridden.join("; "),
    ]
}

/// Prefix `'` to cells starting with `=`, `+`, `-` or `@`.
pub fn escape_csv_cell(cell: &str) -> Cow<'_, str> {
    if cell.starts_with(['=', '+', '-', '@']) {
        Cow::Owned(format!("'{cell}"))
    } else {
        Cow::Borrowed(cell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::testkit::{indexed, NOW};

    fn csv_rows(out: &[u8]) -> Vec<csv::StringRecord> {
        csv::Reader::from_reader(out)
            .records()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    }

    fn header(out: &[u8]) -> Vec<String> {
        csv::Reader::from_reader(out)
            .headers()
            .unwrap()
            .iter()
            .map(str::to_string)
            .collect()
    }

    /// Ported from the v0.1 prototype's exportCSV test.
    #[test]
    fn formula_cells_get_a_leading_apostrophe() {
        for (cell, want) in [
            ("=SUM(A1:A9)", "'=SUM(A1:A9)"),
            ("+1 201 555 0100", "'+1 201 555 0100"),
            ("-2", "'-2"),
            ("@cmd", "'@cmd"),
            (
                "=HYPERLINK(\"http://x.invalid\",\"click\")",
                "'=HYPERLINK(\"http://x.invalid\",\"click\")",
            ),
            ("Priya Raman", "Priya Raman"),
            ("", ""),
            ("a=b", "a=b"),
        ] {
            assert_eq!(escape_csv_cell(cell), want, "{cell}");
        }
        assert!(matches!(escape_csv_cell("plain"), Cow::Borrowed(_)));
    }

    #[test]
    fn csv_has_one_row_per_person_excluding_self_bulk_and_do_not_share() {
        let c = indexed();
        c.execute("UPDATE person SET do_not_share = 1 WHERE id = 6", [])
            .unwrap();
        c.execute(
            "UPDATE person SET title = '=cmd|'' /C calc''!A0' WHERE id = 3",
            [],
        )
        .unwrap();
        let mut out = Vec::new();
        let r = export(
            &c,
            &Config::default(),
            ExportFormat::Csv,
            &ExportOptions::default(),
            &mut out,
            NOW,
        )
        .unwrap();
        assert_eq!(r.people, 4);
        assert_eq!(r.excluded_do_not_share, 1);
        let h = header(&out);
        assert_eq!(&h[..4], ["id", "display_name", "title", "company"]);
        for col in [
            "identities",
            "score",
            "recency",
            "breadth",
            "tags",
            "overridden",
        ] {
            assert!(h.iter().any(|x| x == col), "{col}");
        }
        let rows = csv_rows(&out);
        let ids: Vec<&str> = rows.iter().map(|r| &r[0]).collect();
        assert_eq!(ids, ["2", "3", "7", "5"]);
        let col = |name: &str| h.iter().position(|x| x == name).unwrap();
        assert_eq!(&rows[1][col("title")], "'=cmd|' /C calc'!A0");
        assert_eq!(
            &rows[0][col("identities")],
            "email:priya@northwind.example; linkedin_url:priya-raman-nw"
        );
        assert_eq!(&rows[0][col("tags")], "investor-intro");
        assert_eq!(&rows[0][col("score")], "0.8");
        assert_eq!(&rows[0][col("recency")], "0.95");
    }

    #[test]
    fn include_flags_add_bulk_and_do_not_share_people() {
        let c = indexed();
        c.execute("UPDATE person SET do_not_share = 1 WHERE id = 6", [])
            .unwrap();
        let mut out = Vec::new();
        let r = export(
            &c,
            &Config::default(),
            ExportFormat::Csv,
            &ExportOptions {
                include_bulk: true,
                include_do_not_share: true,
                ..ExportOptions::default()
            },
            &mut out,
            NOW,
        )
        .unwrap();
        assert_eq!(r.people, 6);
        assert_eq!(r.excluded_do_not_share, 0);
    }

    #[test]
    fn overridden_column_lists_fields_set_by_the_user() {
        let c = indexed();
        c.execute_batch(
            "INSERT INTO override(person_id, field, value, set_at) VALUES
               (3, 'title', 'Staff Engineer', 1), (3, 'company', 'Contoso', 2),
               (3, 'location', 'X', 3), (3, 'location', NULL, 4);",
        )
        .unwrap();
        let mut out = Vec::new();
        export(
            &c,
            &Config::default(),
            ExportFormat::Csv,
            &ExportOptions::default(),
            &mut out,
            NOW,
        )
        .unwrap();
        let h = header(&out);
        let rows = csv_rows(&out);
        let marcus = rows.iter().find(|r| &r[0] == "3").unwrap();
        assert_eq!(
            &marcus[h.iter().position(|x| x == "overridden").unwrap()],
            "title; company"
        );
    }

    #[test]
    fn json_round_trips_with_the_same_people_and_fields() {
        let c = indexed();
        let mut out = Vec::new();
        let r = export(
            &c,
            &Config::default(),
            ExportFormat::Json,
            &ExportOptions::default(),
            &mut out,
            NOW,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["v"], 1);
        let people: Vec<crate::json::PersonDetail> =
            serde_json::from_value(v["people"].clone()).unwrap();
        assert_eq!(people.len() as u64, r.people);
        assert_eq!(people.len(), 5);
        let priya = &people[0];
        assert_eq!(priya.person.id, 2);
        assert_eq!(priya.person.title.as_deref(), Some("Co-founder & CEO"));
        assert_eq!(priya.person.tags, ["investor-intro"]);
        assert_eq!(priya.identities.len(), 2);
        assert_eq!(priya.person.components.recency, 0.95);
        // JSON keeps raw values (no formula escaping).
        c.execute("UPDATE person SET title = '=x' WHERE id = 2", [])
            .unwrap();
        let mut out = Vec::new();
        export(
            &c,
            &Config::default(),
            ExportFormat::Json,
            &ExportOptions::default(),
            &mut out,
            NOW,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["people"][0]["person"]["title"], "=x");
    }

    #[test]
    fn query_limits_the_export() {
        let c = indexed();
        let mut out = Vec::new();
        let r = export(
            &c,
            &Config::default(),
            ExportFormat::Csv,
            &ExportOptions {
                query: Some(Query {
                    company: vec!["contoso".into()],
                    limit: 1,
                    ..Query::default()
                }),
                ..ExportOptions::default()
            },
            &mut out,
            NOW,
        )
        .unwrap();
        assert_eq!(r.people, 2, "limit does not cap an export");
        let mut out = Vec::new();
        let r = export(
            &c,
            &Config::default(),
            ExportFormat::Csv,
            &ExportOptions {
                query: Some(Query::text("devtools")),
                ..ExportOptions::default()
            },
            &mut out,
            NOW,
        )
        .unwrap();
        assert_eq!(r.people, 2);
        let rows = csv_rows(&out);
        assert_eq!(
            rows.iter().map(|r| r[0].to_string()).collect::<Vec<_>>(),
            ["7", "3"]
        );
    }
}
