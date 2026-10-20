//! P1-CORE-09 and the review / merge / split / edit commands through the
//! `Graph` facade: rebuild idempotence, incremental equals full, and
//! overrides that survive every rebuild (ER-14..18, ER-24).

mod resolve_support;

use openstance_core::config::Config;
use openstance_core::overrides::{Edit, OverrideField};
use openstance_core::rebuild;
use openstance_core::resolve::commands;
use openstance_core::resolve::lookup::{resolve_person, PersonKey};
use openstance_core::resolve::review::{self, ConfirmFilter};
use openstance_core::{Error, FixedClock, Graph, Providers};
use resolve_support::*;
use rusqlite::Connection;

fn graph(cfg: Config) -> Graph {
    Graph::open_in_memory(cfg, Providers::null(), Box::new(FixedClock(NOW))).unwrap()
}

fn me() -> Config {
    cfg_with_me(&["email:owner@example.com"])
}

/// A small cast: a flagged name+company merge, a thread merge, an alias
/// pair, a rare-name suggestion and a two-way correspondent.
fn sources(c: &Connection, which: usize) -> openstance_core::ingest::Touched {
    let o = ("owner@example.com", "Owen Ryder");
    match which {
        0 => mbox(
            c,
            &[
                Mail::new("m1", ("sam.ito@northwind.example", "Sam Ito"), o),
                Mail::new("m2", o, ("sam.ito@northwind.example", "Sam Ito")).thread("m1"),
                Mail::new("m3", ("tariq@fabrikam.example", "Tariq Bellweather"), o)
                    .thread("t")
                    .cc(("tariq.bell@gmail.com", "Tariq Bellweather"))
                    .cc(("sam.ito@northwind.example", "Sam Ito")),
                Mail::new("m4", ("dana.w@gmail.com", "Dana Whitfield"), o),
                Mail::new("m5", ("danaw+x@gmail.com", ""), o),
                Mail::new("m6", ("ines@halcyon.example", "Ines Quillfeather"), o),
            ],
        ),
        1 => connection(c, &["linkedin_url:sam-ito-nw"], "Sam Ito", "Northwind"),
        2 => card(
            c,
            "vcf",
            &["email:ines.q@mail.example", "phone:+12015550150"],
            "Ines Quillfeather",
            None,
        ),
        _ => card(
            c,
            "vcf",
            &["email:kofi@tailspin.example", "phone:+12015550111"],
            "Kofi Mensah",
            Some("Tailspin"),
        ),
    }
}

const N_SOURCES: usize = 4;

/// Canonical dump of every derived table.
fn dump(c: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    for (name, sql) in [
        (
            "person",
            "SELECT id, display_name, title, company, location, bio, first_seen, last_contact,
                    n_interactions, printf('%.9f', score), printf('%.9f', score_r),
                    printf('%.9f', score_f), printf('%.9f', score_c), printf('%.9f', score_m),
                    printf('%.9f', score_b), is_bulk, is_self, do_not_share, no_enrich
             FROM person ORDER BY id",
        ),
        (
            "identity",
            "SELECT id, person_id, printf('%.4f', confidence), match_rule FROM identity ORDER BY id",
        ),
        (
            "interaction",
            "SELECT id, direction FROM interaction ORDER BY id",
        ),
        (
            "interaction_person",
            "SELECT interaction_id, identity_id, role, person_id FROM interaction_person
             ORDER BY interaction_id, identity_id, role",
        ),
        (
            "tie",
            "SELECT a, b, kind, printf('%.9f', weight), last_at, n_shared FROM tie ORDER BY a, b, kind",
        ),
        (
            "person_fts",
            "SELECT rowid, display_name, title, company, location, bio, tags, handles, subjects
             FROM person_fts ORDER BY rowid",
        ),
    ] {
        let mut st = c.prepare(sql).unwrap();
        let n = st.column_count();
        let rows = st
            .query_map([], |r| {
                let mut s = String::from(name);
                for i in 0..n {
                    let v: rusqlite::types::Value = r.get(i)?;
                    s.push_str(&format!("|{v:?}"));
                }
                Ok(s)
            })
            .unwrap();
        out.extend(rows.map(Result::unwrap));
    }
    out
}

fn full(cfg: Config) -> Graph {
    let mut g = graph(cfg);
    for i in 0..N_SOURCES {
        sources(g.conn(), i);
    }
    g.rebuild().unwrap();
    g
}

fn pid(g: &Graph, spec: &str) -> i64 {
    resolve_person(g.conn(), &PersonKey::parse(spec))
        .unwrap()
        .person
}

#[test]
fn rebuild_twice_gives_identical_derived_tables() {
    let mut g = full(me());
    let first = dump(g.conn());
    assert!(
        first.iter().any(|r| r.starts_with("tie|")),
        "fixture has ties"
    );
    g.rebuild().unwrap();
    assert_eq!(dump(g.conn()), first);
}

#[test]
fn incremental_after_each_source_equals_one_full_rebuild() {
    let want = dump(full(me()).conn());
    for order in [[0, 1, 2, 3], [3, 2, 1, 0], [1, 3, 0, 2], [2, 0, 3, 1]] {
        let mut g = graph(me());
        for i in order {
            g.write(|tx, cfg| {
                let t = sources(tx, i);
                rebuild::incremental(tx, cfg, &t, NOW)
            })
            .unwrap();
        }
        // Identity ids depend on import order; compare against a full
        // rebuild of the same rows.
        let incremental = dump(g.conn());
        g.rebuild().unwrap();
        assert_eq!(incremental, dump(g.conn()), "order {order:?}");
        if order == [0, 1, 2, 3] {
            assert_eq!(incremental, want);
        }
    }
}

#[test]
fn rebuild_report_counts() {
    let mut g = full(me());
    let r = g.rebuild().unwrap();
    // self, sam (+slug, flagged), tariq (thread, flagged), dana (alias),
    // ines (mbox), ines (vcf), kofi
    assert_eq!(r.people, 7);
    assert_eq!(r.flagged, 2);
    assert_eq!(r.removed, 0);
    let meta: String = g
        .conn()
        .query_row(
            "SELECT value FROM meta WHERE key = 'last_rebuild_at'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(meta, NOW.to_string());
}

#[test]
fn review_lists_flags_and_suggestions_and_confirm_survives_rebuild() {
    let mut g = full(me());
    let q = g.review().unwrap();
    let mut rules: Vec<&str> = q.flagged.iter().map(|f| f.rule.as_str()).collect();
    rules.sort();
    assert_eq!(rules, ["name_company", "name_thread"]);
    let sam = q.flagged.iter().find(|f| f.rule == "name_company").unwrap();
    assert_eq!(sam.confidence, 0.85);
    assert!(sam.why.contains("\"sam ito\""), "{}", sam.why);
    assert!(sam.why.contains("\"northwind\""), "{}", sam.why);
    assert_eq!(q.suggestions.len(), 1);
    assert_eq!(q.suggestions[0].rule, "rare_name");
    assert_eq!(q.suggestions[0].confidence, 0.5);

    let sam_id = sam.person.id;
    let written = g.review_confirm(&PersonKey::Id(sam_id)).unwrap();
    assert_eq!(written.len(), 1);
    let q = g.review().unwrap();
    assert_eq!(q.flagged.len(), 1);
    g.rebuild().unwrap();
    assert_eq!(g.review().unwrap().flagged.len(), 1);
    let rule: String = g
        .conn()
        .query_row(
            "SELECT match_rule FROM identity WHERE value = 'sam-ito-nw'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rule, "override");
}

#[test]
fn rejecting_a_suggestion_removes_it_for_good() {
    let mut g = full(me());
    let s = g.review().unwrap().suggestions[0].clone();
    g.review_reject(&PersonKey::Id(s.a.id), &PersonKey::Id(s.b.id))
        .unwrap();
    assert!(g.review().unwrap().suggestions.is_empty());
    g.rebuild().unwrap();
    assert!(g.review().unwrap().suggestions.is_empty());
}

#[test]
fn confirm_all_respects_rule_and_confidence_filters() {
    let mut g = full(me());
    let now = g.now();
    let n = g
        .write(|tx, cfg| {
            review::confirm_all(
                tx,
                cfg,
                &ConfirmFilter {
                    rule: None,
                    min_confidence: Some(0.8),
                },
                now,
            )
        })
        .unwrap();
    assert_eq!(n, 1, "only the 0.85 name_company merge");
    g.rebuild().unwrap();
    let left = g.review().unwrap().flagged;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].rule, "name_thread");
}

#[test]
fn er15_merge_survives_rebuild_as_override() {
    let mut g = full(me());
    let a = pid(&g, "kofi@tailspin.example");
    let b = pid(&g, "dana.w@gmail.com");
    let out = g.merge(&[PersonKey::Id(a), PersonKey::Id(b)]).unwrap();
    assert_eq!(out.person, a.min(b));
    g.rebuild().unwrap();
    assert_eq!(
        pid(&g, "kofi@tailspin.example"),
        pid(&g, "dana.w@gmail.com")
    );
    let rule: String = g
        .conn()
        .query_row(
            "SELECT match_rule FROM identity WHERE id = ?1",
            [a.max(b)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rule, "override");
    // ER-24: the absorbed id still resolves, with resolved_from.
    let r = resolve_person(g.conn(), &PersonKey::Id(a.max(b))).unwrap();
    assert_eq!(r.person, a.min(b));
    assert_eq!(r.resolved_from, Some(a.max(b)));
    // merge --undo
    let now = g.now();
    g.write(|tx, cfg| {
        let n = commands::unmerge(tx, a.max(b), a.min(b))?;
        assert_eq!(n, 1);
        rebuild::rebuild(tx, cfg, now)
    })
    .unwrap();
    assert_ne!(
        pid(&g, "kofi@tailspin.example"),
        pid(&g, "dana.w@gmail.com")
    );
}

#[test]
fn merge_refuses_the_same_person_twice() {
    let mut g = full(me());
    let err = g
        .merge(&[
            PersonKey::parse("kofi@tailspin.example"),
            PersonKey::parse("+1 201 555 0111"),
        ])
        .unwrap_err();
    assert!(matches!(err, Error::BadRequest(_)), "{err}");
}

#[test]
fn merge_carries_privacy_flags_to_the_survivor() {
    let mut g = full(me());
    g.edit(
        &PersonKey::parse("dana.w@gmail.com"),
        &Edit {
            do_not_share: Some(true),
            ..Edit::default()
        },
    )
    .unwrap();
    g.merge(&[
        PersonKey::parse("kofi@tailspin.example"),
        PersonKey::parse("dana.w@gmail.com"),
    ])
    .unwrap();
    let p = pid(&g, "kofi@tailspin.example");
    let dns: i64 = g
        .conn()
        .query_row("SELECT do_not_share FROM person WHERE id = ?1", [p], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(dns, 1);
}

#[test]
fn er14_split_holds_even_when_a_later_import_matches_exactly() {
    let mut g = full(me());
    let sam = pid(&g, "sam.ito@northwind.example");
    let slug: i64 = g
        .conn()
        .query_row(
            "SELECT id FROM identity WHERE value = 'sam-ito-nw'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let out = g.split(&PersonKey::Id(sam), &[slug]).unwrap();
    assert_eq!((out.kept, out.moved, out.swapped), (sam, slug, false));
    assert_ne!(
        pid(&g, "sam.ito@northwind.example"),
        pid(&g, "in/sam-ito-nw")
    );
    // A new LinkedIn export now lists the email on the same row as the slug.
    g.write(|tx, cfg| {
        let t = connection(
            tx,
            &["linkedin_url:sam-ito-nw", "email:sam.ito@northwind.example"],
            "Sam Ito",
            "Northwind",
        );
        rebuild::incremental(tx, cfg, &t, NOW)
    })
    .unwrap();
    assert_ne!(
        pid(&g, "sam.ito@northwind.example"),
        pid(&g, "in/sam-ito-nw")
    );
    g.rebuild().unwrap();
    assert_ne!(
        pid(&g, "sam.ito@northwind.example"),
        pid(&g, "in/sam-ito-nw")
    );
    // split --undo --unpin lets the exact match join them again.
    let (a, b) = (
        pid(&g, "sam.ito@northwind.example"),
        pid(&g, "in/sam-ito-nw"),
    );
    let now = g.now();
    g.write(|tx, cfg| {
        assert!(commands::unsplit(tx, a, b, true)? >= 1);
        rebuild::rebuild(tx, cfg, now)
    })
    .unwrap();
    assert_eq!(
        pid(&g, "sam.ito@northwind.example"),
        pid(&g, "in/sam-ito-nw")
    );
}

#[test]
fn er16_split_choosing_the_anchor_swaps_sides_and_ids_are_stable() {
    let mut g = full(me());
    let sam = pid(&g, "sam.ito@northwind.example");
    let out = g.split(&PersonKey::Id(sam), &[sam]).unwrap();
    assert!(out.swapped);
    assert_eq!(out.kept, sam);
    let before = (
        pid(&g, "sam.ito@northwind.example"),
        pid(&g, "in/sam-ito-nw"),
    );
    assert_eq!(before.0, sam);
    g.rebuild().unwrap();
    assert_eq!(
        (
            pid(&g, "sam.ito@northwind.example"),
            pid(&g, "in/sam-ito-nw")
        ),
        before
    );
}

#[test]
fn split_validates_its_input() {
    let mut g = full(me());
    let sam = pid(&g, "sam.ito@northwind.example");
    let kofi_phone: i64 = g
        .conn()
        .query_row(
            "SELECT id FROM identity WHERE value = '+12015550111'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(matches!(
        g.split(&PersonKey::Id(sam), &[kofi_phone]).unwrap_err(),
        Error::BadRequest(_)
    ));
    assert!(matches!(
        g.split(&PersonKey::Id(sam), &[]).unwrap_err(),
        Error::BadRequest(_)
    ));
}

#[test]
fn edits_apply_at_once_and_survive_rebuild() {
    let mut g = full(me());
    let p = g
        .edit(
            &PersonKey::parse("Kofi Mensah"),
            &Edit {
                set: vec![(OverrideField::Title, "CTO".into())],
                add_tags: vec!["investor-intro".into()],
                ..Edit::default()
            },
        )
        .unwrap();
    let title = |g: &Graph| -> Option<String> {
        g.conn()
            .query_row("SELECT title FROM person WHERE id = ?1", [p], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(title(&g).as_deref(), Some("CTO"));
    let fts: i64 = g
        .conn()
        .query_row(
            "SELECT count(*) FROM person_fts WHERE person_fts MATCH 'title:cto' AND rowid = ?1",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fts, 1, "searchable without a rebuild");
    g.rebuild().unwrap();
    assert_eq!(title(&g).as_deref(), Some("CTO"));
}

#[test]
fn person_keys_parse_and_resolve() {
    let g = full(me());
    assert_eq!(PersonKey::parse("812"), PersonKey::Id(812));
    assert!(
        matches!(PersonKey::parse("@jdoe"), PersonKey::Handle(k, _) if k.as_str() == "x_handle")
    );
    assert!(
        matches!(PersonKey::parse("in/jane"), PersonKey::Handle(k, _) if k.as_str() == "linkedin_url")
    );
    assert!(
        matches!(PersonKey::parse("+1 201 555 0111"), PersonKey::Handle(k, _) if k.as_str() == "phone")
    );
    assert!(
        matches!(PersonKey::parse("a@b.example"), PersonKey::Handle(k, _) if k.as_str() == "email")
    );
    assert!(
        matches!(PersonKey::parse("phone:+12015550111"), PersonKey::Handle(k, _) if k.as_str() == "phone")
    );
    assert_eq!(
        PersonKey::parse("Jane Doe"),
        PersonKey::Name("Jane Doe".into())
    );

    let kofi = pid(&g, "kofi@tailspin.example");
    assert_eq!(pid(&g, "+1 (201) 555-0111"), kofi);
    assert_eq!(pid(&g, "kofi mensah"), kofi);
    assert_eq!(pid(&g, "KOFI@tailspin.example"), kofi);
    match resolve_person(g.conn(), &PersonKey::parse("Ines Quillfeather")).unwrap_err() {
        Error::Ambiguous { candidates, .. } => assert_eq!(candidates.len(), 2),
        e => panic!("{e}"),
    }
    assert!(matches!(
        resolve_person(g.conn(), &PersonKey::parse("nobody@nowhere.example")).unwrap_err(),
        Error::NotFound(_)
    ));
    assert!(matches!(
        resolve_person(g.conn(), &PersonKey::Id(999_999)).unwrap_err(),
        Error::NotFound(_)
    ));
}

#[test]
fn er23_self_person_is_absent_from_ties_and_fts() {
    let g = full(me());
    let self_id = pid(&g, "owner@example.com");
    let n: i64 = g
        .conn()
        .query_row(
            "SELECT (SELECT count(*) FROM tie WHERE a = ?1 OR b = ?1)
                  + (SELECT count(*) FROM person_fts WHERE rowid = ?1)",
            [self_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}
