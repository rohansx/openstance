//! P1-CORE-03: the schema after all migrations matches the golden snapshot.
//! Regenerate deliberately with `UPDATE_GOLDEN=1 cargo test -p openstance-core --test schema_snapshot`.

use openstance_core::db::{migrate, open_in_memory};

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/schema.sql");

fn dump() -> String {
    let mut c = open_in_memory().unwrap();
    migrate(&mut c, None, 0).unwrap();
    let mut stmt = c
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .unwrap();
    let mut out = String::new();
    for row in rows {
        let (ty, name, tbl, sql) = row.unwrap();
        out.push_str(&format!("-- {ty} {name} on {tbl}\n"));
        match sql {
            Some(s) => out.push_str(&format!("{s};\n\n")),
            None => out.push_str("-- (auto)\n\n"),
        }
    }
    out
}

#[test]
fn schema_matches_golden_snapshot() {
    let got = dump();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(GOLDEN, &got).unwrap();
    }
    let want = std::fs::read_to_string(GOLDEN)
        .expect("golden schema snapshot missing; run with UPDATE_GOLDEN=1");
    assert_eq!(got, want, "schema drifted from tests/golden/schema.sql");
}

#[test]
fn schema_has_the_nine_regular_tables_plus_meta_and_person_fts() {
    let mut c = open_in_memory().unwrap();
    migrate(&mut c, None, 0).unwrap();
    let mut stmt = c
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table'
             AND name NOT LIKE 'person_fts%' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap();
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "enrich_log",
            "identity",
            "interaction",
            "interaction_person",
            "meta",
            "override",
            "person",
            "source",
            "tie",
            "view"
        ]
    );
}

#[test]
fn person_fts_is_a_content_storing_fts5_table() {
    let mut c = open_in_memory().unwrap();
    migrate(&mut c, None, 0).unwrap();
    let sql: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'person_fts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sql.contains("USING fts5"));
    assert!(!sql.contains("content=''"), "must not be contentless");
    let cols: Vec<String> = {
        let mut s = c
            .prepare("SELECT name FROM pragma_table_info('person_fts')")
            .unwrap();
        s.query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(
        cols,
        [
            "display_name",
            "title",
            "company",
            "location",
            "bio",
            "tags",
            "handles",
            "subjects"
        ]
    );
    // content-storing: values can be read back, and rows deleted by rowid
    c.execute(
        "INSERT INTO person_fts(rowid, display_name, title) VALUES (7, 'Ada Example', 'CTO')",
        [],
    )
    .unwrap();
    let t: String = c
        .query_row("SELECT title FROM person_fts WHERE rowid = 7", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(t, "CTO");
    c.execute("DELETE FROM person_fts WHERE rowid = 7", [])
        .unwrap();
    let n: i64 = c
        .query_row(
            "SELECT count(*) FROM person_fts WHERE person_fts MATCH 'cto'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn schema_constraints_hold() {
    let mut c = open_in_memory().unwrap();
    migrate(&mut c, None, 0).unwrap();
    c.execute(
        "INSERT INTO source(id, kind, imported_at) VALUES (1, 'vcf', 0)",
        [],
    )
    .unwrap();
    // person.is_bulk defaults to 0
    c.execute(
        "INSERT INTO identity(id, kind, value, source_id) VALUES (1, 'email', 'a@example.com', 1)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO identity(id, kind, value, source_id) VALUES (2, 'email', 'b@example.com', 1)",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO person(id) VALUES (1), (2)", [])
        .unwrap();
    let bulk: i64 = c
        .query_row("SELECT is_bulk FROM person WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(bulk, 0);
    // tie requires a < b
    assert!(c
        .execute("INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES (2, 1, 'email', 0.5, 0, 1)", [])
        .is_err());
    c.execute(
        "INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES (1, 2, 'email', 0.5, 0, 1)",
        [],
    )
    .unwrap();
    // identity (kind, value) is unique
    assert!(c
        .execute(
            "INSERT INTO identity(kind, value, source_id) VALUES ('email', 'a@example.com', 1)",
            []
        )
        .is_err());
    // override: id primary key, scalar fields may repeat, NULL tombstones allowed
    c.execute(
        "INSERT INTO override(person_id, field, value, set_at) VALUES (1, 'title', 'CTO', 1)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO override(person_id, field, value, set_at) VALUES (1, 'title', NULL, 2)",
        [],
    )
    .unwrap();
    // multi-valued tag rows are unique per value
    c.execute(
        "INSERT INTO override(person_id, field, value, set_at) VALUES (1, 'tag', 'vc', 1)",
        [],
    )
    .unwrap();
    assert!(c
        .execute(
            "INSERT INTO override(person_id, field, value, set_at) VALUES (1, 'tag', 'vc', 2)",
            []
        )
        .is_err());
    // unknown override fields are rejected
    assert!(c
        .execute(
            "INSERT INTO override(person_id, field, value, set_at) VALUES (1, 'shoe_size', '9', 1)",
            []
        )
        .is_err());
}

#[test]
fn documented_indexes_exist() {
    let mut c = open_in_memory().unwrap();
    migrate(&mut c, None, 0).unwrap();
    for idx in [
        "interaction_at",
        "interaction_global_id",
        "interaction_person_person",
        "tie_b",
        "person_score",
        "person_last_contact",
        "source_file_hash",
    ] {
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                [idx],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "missing index {idx}");
    }
}
