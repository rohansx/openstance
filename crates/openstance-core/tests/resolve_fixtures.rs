//! P1-CORE-12: the "must-hold assertions" of `fixtures/EXPECTED.md`, on the
//! real fixture files imported through the real importers, plus
//! order independence (incremental equals full).

use std::path::PathBuf;

use openstance_core::import::{registry, ImportOptions};
use openstance_core::ingest::ImportRequest;
use openstance_core::resolve::lookup::{resolve_person, PersonKey};
use openstance_core::{config, FixedClock, Graph, Providers};

const FIXTURE_NOW: i64 = 1_790_812_800;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

const FILES: [&str; 5] = [
    "mbox/small.mbox",
    "linkedin/Connections.csv",
    "linkedin/messages.csv",
    "linkedin/Invitations.csv",
    "vcf/contacts.vcf",
];

fn graph() -> Graph {
    let text = std::fs::read_to_string(fixtures().join("config.toml")).unwrap();
    let (cfg, _) = config::parse_str(&text, true).unwrap();
    let providers = Providers::with_importers(registry(&ImportOptions::default()));
    Graph::open_in_memory(cfg, providers, Box::new(FixedClock(FIXTURE_NOW))).unwrap()
}

fn imported(order: &[usize]) -> Graph {
    let mut g = graph();
    for &i in order {
        g.import(&[fixtures().join(FILES[i])], &ImportRequest::default())
            .unwrap();
    }
    g
}

fn person(g: &Graph, spec: &str) -> i64 {
    resolve_person(g.conn(), &PersonKey::parse(spec))
        .unwrap()
        .person
}

fn assignment(g: &Graph, kind: &str, value: &str) -> (i64, f64, String) {
    g.conn()
        .query_row(
            "SELECT person_id, confidence, match_rule FROM identity WHERE kind = ?1 AND value = ?2",
            [kind, value],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

#[test]
fn must_hold_assertions() {
    let g = imported(&[0, 1, 2, 3, 4]);
    let c = g.conn();

    // Alex Kim: common name, two people, no suggestion between them.
    let (nw, ct) = (
        person(&g, "in/alex-kim-northwind"),
        person(&g, "alex.kim@contoso.example"),
    );
    assert_ne!(nw, ct);
    let q = g.review().unwrap();
    assert!(!q
        .suggestions
        .iter()
        .any(|s| [s.a.id, s.b.id].contains(&nw) || [s.a.id, s.b.id].contains(&ct)));

    // Dana Whitfield: one person, two identities at gmail_alias 0.98.
    let dana = person(&g, "dana.whitfield@gmail.test");
    let n_alias: i64 = c
        .query_row(
            "SELECT count(*) FROM identity WHERE person_id = ?1 AND match_rule = 'gmail_alias'
             AND abs(confidence - 0.98) < 1e-9",
            [dana],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_alias, 2);
    assert_eq!(person(&g, "danawhitfield@gmail.test"), dana);
    assert_eq!(person(&g, "dana.whitfield+lists@googlemail.test"), dana);

    // Pat Moreno: corporate plus alias, not flagged.
    let pat = person(&g, "pat@fabrikam.example");
    assert_eq!(person(&g, "pat+hiring@fabrikam.example"), pat);
    let flagged: i64 = c
        .query_row(
            "SELECT count(*) FROM identity WHERE person_id = ?1
             AND match_rule IN ('name_company', 'name_thread')",
            [pat],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(flagged, 0);

    // Digest Weekly is the only bulk sender; Priya Raman is not bulk.
    let bulk: Vec<i64> = c
        .prepare("SELECT id FROM person WHERE is_bulk = 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(bulk, vec![person(&g, "news@digest.example")]);

    // Priya and Marcus share exactly 3 email interactions.
    let (p, m) = (
        person(&g, "priya@northwind.example"),
        person(&g, "marcus@contoso.example"),
    );
    let n: i64 = c
        .query_row(
            "SELECT n_shared FROM tie WHERE a = ?1 AND b = ?2 AND kind = 'email'",
            [p.min(m), p.max(m)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 3);
    let self_id = person(&g, "owner@example.com");
    let self_ties: i64 = c
        .query_row(
            "SELECT count(*) FROM tie WHERE a = ?1 OR b = ?1",
            [self_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(self_ties, 0);

    // Kofi, Priya and Hannah merge mbox with vcf by email at 1.0.
    for (email, phone_or_other) in [
        ("kofi@tailspin.example", None),
        ("priya@northwind.example", Some(("phone", "+12015550142"))),
        ("hannah@contoso.example", None),
    ] {
        let (pid, conf, rule) = assignment(&g, "email", email);
        assert_eq!(conf, 1.0, "{email}");
        assert!(rule == "anchor" || rule == "exact", "{email}: {rule}");
        let sources: i64 = c
            .query_row(
                "SELECT count(DISTINCT s.kind) FROM interaction_person ip
                 JOIN interaction i ON i.id = ip.interaction_id
                 JOIN source s ON s.id = i.source_id
                 WHERE ip.person_id = ?1 AND s.kind IN ('mbox', 'vcf')",
                [pid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sources, 2, "{email} seen in mbox and vcf");
        if let Some((k, v)) = phone_or_other {
            assert_eq!(assignment(&g, k, v).0, pid);
        }
    }

    // Rufus Pennyworth: a person whose only identity is a phone.
    let rufus = person(&g, "+12015550177");
    let ids: i64 = c
        .query_row(
            "SELECT count(*) FROM identity WHERE person_id = ?1",
            [rufus],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ids, 1);
    let name: String = c
        .query_row(
            "SELECT display_name FROM person WHERE id = ?1",
            [rufus],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(name, "Rufus Pennyworth");

    // Never stored: bodies and the skipped messages' handles.
    for absent in ["ghost@example.invalid", "prizes@spam.example"] {
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM identity WHERE value = ?1",
                [absent],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "{absent}");
    }
}

#[test]
fn the_sentinel_is_never_stored_in_any_column() {
    let g = imported(&[0, 1, 2, 3, 4]);
    let c = g.conn();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'person_fts_%' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for t in tables {
        let cols: Vec<String> = c
            .prepare(&format!("SELECT name FROM pragma_table_info('{t}')"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for col in cols {
            let n: i64 = c
                .query_row(
                    &format!("SELECT count(*) FROM \"{t}\" WHERE CAST(\"{col}\" AS TEXT) LIKE '%SENTINEL-BODY-7f3a%'"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{t}.{col}");
        }
    }
}

#[test]
fn suggestions_are_exactly_the_two_in_expected_md() {
    let g = imported(&[0, 1, 2, 3, 4]);
    let q = g.review().unwrap();
    let mut got: Vec<(String, String, String, f64)> = q
        .suggestions
        .iter()
        .map(|s| {
            let mut names = [
                s.a.display_name.clone().unwrap_or_default(),
                s.b.display_name.clone().unwrap_or_default(),
            ];
            names.sort();
            let [x, y] = names;
            (s.rule.clone(), x, y, s.confidence)
        })
        .collect();
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].0, "jaro_winkler");
    assert_eq!(
        (got[0].1.as_str(), got[0].2.as_str()),
        ("John Quillby", "Jon Quillby")
    );
    assert!((got[0].3 - 0.4889).abs() < 1e-4, "{}", got[0].3);
    assert_eq!(
        (
            got[1].0.as_str(),
            got[1].1.as_str(),
            got[1].2.as_str(),
            got[1].3
        ),
        ("rare_name", "Ines Quillfeather", "Ines Quillfeather", 0.5)
    );
    let mut flagged: Vec<String> = q.flagged.iter().map(|f| f.rule.clone()).collect();
    flagged.sort();
    assert_eq!(
        flagged,
        [
            "name_company",
            "name_company",
            "name_company",
            "name_thread"
        ]
    );
}

/// Person rows keyed by their handles (not ids), so different import
/// orders (different identity ids) can be compared. The match-rule column
/// is selected but not compared: which identity is the anchor depends on
/// insertion order.
fn canonical(g: &Graph) -> Vec<String> {
    let mut st = g
        .conn()
        .prepare(
            "SELECT (SELECT group_concat(kind || ':' || value, ' ') FROM
                       (SELECT kind, value FROM identity WHERE person_id = p.id ORDER BY kind, value)),
                    p.display_name, p.title, p.company, p.location, p.is_bulk, p.is_self,
                    printf('%.9f', p.score), p.n_interactions, p.last_contact,
                    (SELECT group_concat(match_rule || printf('%.2f', confidence), ' ') FROM
                       (SELECT match_rule, confidence FROM identity WHERE person_id = p.id
                        ORDER BY match_rule, confidence))
             FROM person p",
        )
        .unwrap();
    let mut rows: Vec<String> = st
        .query_map([], |r| {
            let mut s = String::new();
            for i in 0..10 {
                let v: rusqlite::types::Value = r.get(i)?;
                s.push_str(&format!("{v:?}|"));
            }
            Ok(s)
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows.sort();
    rows
}

#[test]
fn totals_and_people_do_not_depend_on_import_order() {
    let base = imported(&[0, 1, 2, 3, 4]);
    let want_counts = base.counts().unwrap();
    let want = canonical(&base);
    for order in [[4, 3, 2, 1, 0], [1, 4, 0, 3, 2], [2, 0, 4, 1, 3]] {
        let mut g = imported(&order);
        assert_eq!(g.counts().unwrap(), want_counts, "order {order:?}");
        // Incremental equals a full rebuild of the same rows.
        let before = canonical(&g);
        g.rebuild().unwrap();
        assert_eq!(canonical(&g), before, "rebuild after order {order:?}");
        // The people themselves (handles, fields, scores) match too.
        assert_eq!(before, want, "people after order {order:?}");
    }
}
