//! Shortlist, introductions, sources and dashboard numbers over the
//! fictional search test kit (people 1..7; 1 is the user, 4 is bulk).

use super::*;
use crate::search::testkit::{self, NOW};

fn conn() -> Connection {
    testkit::graph()
}

// --- shortlist -------------------------------------------------------------

#[test]
fn shortlist_starts_empty() {
    let c = conn();
    let r = shortlist(&c).unwrap();
    assert_eq!(r.v, V);
    assert!(r.shortlist.is_empty());
}

#[test]
fn shortlist_add_returns_the_entry_with_the_person() {
    let c = conn();
    let e = shortlist_add(&c, 2, Some("ask about the seed round"), NOW).unwrap();
    assert_eq!(e.person_id, 2);
    let p = e.person.as_ref().unwrap();
    assert_eq!(p.display_name.as_deref(), Some("Priya Raman"));
    assert_eq!(e.note.as_deref(), Some("ask about the seed round"));
    assert_eq!(e.added_at, iso(NOW));
    assert_eq!(shortlist(&c).unwrap().shortlist, vec![e]);
}

#[test]
fn shortlist_add_twice_keeps_added_at_and_replaces_the_note_only_when_given() {
    let c = conn();
    shortlist_add(&c, 2, Some("first"), NOW).unwrap();
    let again = shortlist_add(&c, 2, None, NOW + 100).unwrap();
    assert_eq!(again.added_at, iso(NOW));
    assert_eq!(again.note.as_deref(), Some("first"));
    let replaced = shortlist_add(&c, 2, Some("second"), NOW + 200).unwrap();
    assert_eq!(replaced.note.as_deref(), Some("second"));
    assert_eq!(shortlist(&c).unwrap().shortlist.len(), 1);
}

#[test]
fn shortlist_is_newest_first() {
    let c = conn();
    shortlist_add(&c, 3, None, NOW).unwrap();
    shortlist_add(&c, 2, None, NOW + 10).unwrap();
    let ids: Vec<i64> = shortlist(&c)
        .unwrap()
        .shortlist
        .iter()
        .map(|e| e.person_id)
        .collect();
    assert_eq!(ids, vec![2, 3]);
}

#[test]
fn shortlist_rejects_unknown_people_and_long_notes() {
    let c = conn();
    assert!(matches!(
        shortlist_add(&c, 999, None, NOW),
        Err(Error::NotFound(_))
    ));
    let long = "x".repeat(MAX_NOTE_CHARS + 1);
    assert!(matches!(
        shortlist_add(&c, 2, Some(&long), NOW),
        Err(Error::BadRequest(_))
    ));
}

#[test]
fn shortlist_set_note_and_remove() {
    let c = conn();
    shortlist_add(&c, 2, Some("a"), NOW).unwrap();
    let e = shortlist_set_note(&c, 2, Some("b")).unwrap();
    assert_eq!(e.note.as_deref(), Some("b"));
    let e = shortlist_set_note(&c, 2, None).unwrap();
    assert_eq!(e.note, None);
    shortlist_remove(&c, 2).unwrap();
    assert!(shortlist(&c).unwrap().shortlist.is_empty());
    assert!(matches!(shortlist_remove(&c, 2), Err(Error::NotFound(_))));
    assert!(matches!(
        shortlist_set_note(&c, 3, Some("x")),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn shortlist_follows_a_merge() {
    let c = conn();
    shortlist_add(&c, 3, Some("marcus"), NOW).unwrap();
    // Marcus (anchor 3) is merged into Ravi (person 7): identity 3 now
    // belongs to person 7.
    c.execute("UPDATE identity SET person_id = 7 WHERE id = 3", [])
        .unwrap();
    let r = shortlist(&c).unwrap();
    assert_eq!(r.shortlist.len(), 1);
    assert_eq!(r.shortlist[0].person_id, 7);
    assert_eq!(
        r.shortlist[0]
            .person
            .as_ref()
            .unwrap()
            .display_name
            .as_deref(),
        Some("Ravi Iyer")
    );
    // Removing by the current id removes the row stored under the old anchor.
    shortlist_remove(&c, 7).unwrap();
    assert!(shortlist(&c).unwrap().shortlist.is_empty());
}

// --- introductions ---------------------------------------------------------

fn new_intro(person: i64, connector: Option<i64>) -> NewIntro {
    NewIntro {
        person,
        connector,
        ..NewIntro::default()
    }
}

#[test]
fn stages_round_trip_and_serialize_snake_case() {
    for s in Stage::ALL {
        assert_eq!(Stage::parse(s.as_str()), Some(s));
        assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
    }
    assert_eq!(Stage::parse("intro made"), None);
    assert_eq!(Stage::IntroMade.as_str(), "intro_made");
}

#[test]
fn intro_create_defaults_to_draft_with_people_resolved() {
    let c = conn();
    let i = intro_create(&c, &new_intro(6, Some(2)), NOW).unwrap();
    assert_eq!(i.stage, Stage::Draft);
    assert_eq!(i.person_id, 6);
    assert_eq!(
        i.person.as_ref().unwrap().display_name.as_deref(),
        Some("Lena Park")
    );
    assert_eq!(
        i.connector.as_ref().unwrap().display_name.as_deref(),
        Some("Priya Raman")
    );
    assert_eq!(i.created_at, iso(NOW));
    assert_eq!(i.updated_at, iso(NOW));
    assert_eq!(intros(&c, None).unwrap().intros, vec![i]);
}

#[test]
fn intro_create_validates_input() {
    let c = conn();
    assert!(matches!(
        intro_create(&c, &new_intro(999, None), NOW),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        intro_create(&c, &new_intro(6, Some(999)), NOW),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        intro_create(&c, &new_intro(6, Some(6)), NOW),
        Err(Error::BadRequest(_))
    ));
    let bad_date = NewIntro {
        follow_up: Some("next tuesday".into()),
        ..new_intro(6, None)
    };
    assert!(matches!(
        intro_create(&c, &bad_date, NOW),
        Err(Error::BadRequest(_))
    ));
    let long = NewIntro {
        note: Some("y".repeat(MAX_NOTE_CHARS + 1)),
        ..new_intro(6, None)
    };
    assert!(matches!(
        intro_create(&c, &long, NOW),
        Err(Error::BadRequest(_))
    ));
}

#[test]
fn intro_update_changes_only_given_fields() {
    let c = conn();
    let i = intro_create(
        &c,
        &NewIntro {
            note: Some("warm".into()),
            follow_up: Some("2026-11-02".into()),
            ..new_intro(6, Some(2))
        },
        NOW,
    )
    .unwrap();
    let u = intro_update(
        &c,
        i.id,
        &IntroPatch {
            stage: Some(Stage::Asked),
            ..IntroPatch::default()
        },
        NOW + 60,
    )
    .unwrap();
    assert_eq!(u.stage, Stage::Asked);
    assert_eq!(u.note.as_deref(), Some("warm"));
    assert_eq!(u.follow_up.as_deref(), Some("2026-11-02"));
    assert_eq!(u.created_at, iso(NOW));
    assert_eq!(u.updated_at, iso(NOW + 60));
    let cleared = intro_update(
        &c,
        i.id,
        &IntroPatch {
            connector: Some(None),
            note: Some(None),
            follow_up: Some(None),
            ..IntroPatch::default()
        },
        NOW + 120,
    )
    .unwrap();
    assert_eq!(cleared.connector, None);
    assert_eq!(cleared.note, None);
    assert_eq!(cleared.follow_up, None);
    assert_eq!(cleared.stage, Stage::Asked);
}

#[test]
fn intro_update_and_delete_unknown_ids_are_not_found() {
    let c = conn();
    assert!(matches!(
        intro_update(&c, 42, &IntroPatch::default(), NOW),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(intro_delete(&c, 42), Err(Error::NotFound(_))));
}

#[test]
fn intro_delete_removes_the_row() {
    let c = conn();
    let i = intro_create(&c, &new_intro(6, None), NOW).unwrap();
    intro_delete(&c, i.id).unwrap();
    assert!(intros(&c, None).unwrap().intros.is_empty());
}

#[test]
fn intros_are_in_board_order_and_filter_by_stage() {
    let c = conn();
    let a = intro_create(
        &c,
        &NewIntro {
            stage: Some(Stage::Met),
            ..new_intro(2, None)
        },
        NOW,
    )
    .unwrap();
    let b = intro_create(&c, &new_intro(6, None), NOW + 1).unwrap();
    let all: Vec<i64> = intros(&c, None)
        .unwrap()
        .intros
        .iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(all, vec![b.id, a.id], "draft before met");
    let met = intros(&c, Some(Stage::Met)).unwrap().intros;
    assert_eq!(met.len(), 1);
    assert_eq!(met[0].id, a.id);
}

#[test]
fn intro_json_shape() {
    let c = conn();
    let i = intro_create(
        &c,
        &NewIntro {
            stage: Some(Stage::IntroMade),
            follow_up: Some("2026-11-02".into()),
            ..new_intro(6, Some(2))
        },
        NOW,
    )
    .unwrap();
    let v = serde_json::to_value(IntroResponse { v: V, intro: i }).unwrap();
    assert_eq!(v["v"], 1);
    assert_eq!(v["intro"]["stage"], "intro_made");
    assert_eq!(v["intro"]["follow_up"], "2026-11-02");
    assert_eq!(v["intro"]["person"]["id"], 6);
    assert_eq!(v["intro"]["connector"]["id"], 2);
    assert!(v["intro"]["note"].is_null());
}

// --- sources and stats -----------------------------------------------------

#[test]
fn sources_lists_every_source_oldest_first() {
    let c = conn();
    c.execute(
        "UPDATE source SET path = 'Connections.csv', imported_at = 50,
           stats_json = '{\"identities_seen\": 3}' WHERE id = 2",
        [],
    )
    .unwrap();
    let r = sources(&c).unwrap();
    assert_eq!(r.v, V);
    let ids: Vec<i64> = r.sources.iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![1, 3, 2]);
    let li = &r.sources[2];
    assert_eq!(li.kind, "linkedin_csv");
    assert_eq!(li.label.as_deref(), Some("linkedin 2026-10"));
    assert_eq!(li.path.as_deref(), Some("Connections.csv"));
    assert_eq!(li.imported_at, iso(50));
    assert_eq!(li.stats.as_ref().unwrap()["identities_seen"], 3);
    assert_eq!(r.sources[0].stats, None);
}

#[test]
fn base_stats_counts_people_sources_and_workspace() {
    let c = conn();
    c.execute("UPDATE source SET imported_at = 77 WHERE id = 3", [])
        .unwrap();
    shortlist_add(&c, 2, None, NOW).unwrap();
    intro_create(
        &c,
        &NewIntro {
            stage: Some(Stage::Asked),
            ..new_intro(6, Some(2))
        },
        NOW,
    )
    .unwrap();
    let s = base_stats(&c).unwrap();
    assert_eq!(s.v, V);
    assert_eq!(s.people, 5, "7 people minus self minus bulk");
    assert_eq!(s.bulk, 1);
    assert_eq!(s.sources, 3);
    assert_eq!(s.last_import.as_deref(), Some(iso(77).as_str()));
    assert_eq!(s.shortlist, 1);
    assert_eq!(s.intros.len(), Stage::ALL.len());
    assert_eq!(s.intros["asked"], 1);
    assert_eq!(s.intros["draft"], 0);
    assert_eq!(s.schema_version, crate::db::LATEST_VERSION);
}

#[test]
fn base_stats_on_an_empty_graph() {
    let mut c = crate::db::open_in_memory().unwrap();
    crate::db::migrate(&mut c, None, 0).unwrap();
    let s = base_stats(&c).unwrap();
    assert_eq!(s.people, 0);
    assert_eq!(s.sources, 0);
    assert_eq!(s.last_import, None);
    assert_eq!(s.review, ReviewCounts::default());
    assert_eq!(s.enrich, EnrichCounts::default());
}

// --- Graph facade ----------------------------------------------------------

mod facade {
    use super::*;
    use crate::resolve::lookup::PersonKey;
    use crate::{Config, FixedClock, Graph, Providers};

    fn graph() -> Graph {
        let g = Graph::open_in_memory(
            Config::default(),
            Providers::null(),
            Box::new(FixedClock(NOW)),
        )
        .unwrap();
        testkit::seed(g.conn());
        g
    }

    #[test]
    fn shortlist_by_reference() {
        let mut g = graph();
        let e = g
            .shortlist_add(&PersonKey::parse("priya@northwind.example"), Some("hi"))
            .unwrap();
        assert_eq!(e.person_id, 2);
        assert_eq!(e.added_at, iso(NOW));
        let e = g
            .shortlist_set_note(&PersonKey::parse("Priya Raman"), None)
            .unwrap();
        assert_eq!(e.note, None);
        assert_eq!(g.shortlist().unwrap().shortlist.len(), 1);
        g.shortlist_remove(&PersonKey::Id(2)).unwrap();
        assert!(g.shortlist().unwrap().shortlist.is_empty());
        assert!(matches!(
            g.shortlist_add(&PersonKey::parse("nobody@nowhere.example"), None),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn intros_by_reference() {
        let mut g = graph();
        let i = g
            .intro_create(&IntroRequest {
                person: PersonKey::parse("in/lena-park"),
                connector: Some(PersonKey::Id(2)),
                stage: None,
                note: Some("fintech angels".into()),
                follow_up: None,
            })
            .unwrap();
        assert_eq!(i.person_id, 6);
        assert_eq!(i.connector.as_ref().unwrap().id, 2);
        let u = g
            .intro_update(
                i.id,
                &IntroEdit {
                    stage: Some(Stage::Asked),
                    connector: Some(Some(PersonKey::Id(7))),
                    ..IntroEdit::default()
                },
            )
            .unwrap();
        assert_eq!(u.stage, Stage::Asked);
        assert_eq!(u.connector.as_ref().unwrap().id, 7);
        assert_eq!(g.intros(None).unwrap().intros.len(), 1);
        g.intro_delete(i.id).unwrap();
        assert!(g.intros(None).unwrap().intros.is_empty());
    }

    #[test]
    fn dashboard_stats_add_review_and_egress_numbers() {
        let g = graph();
        g.conn()
            .execute_batch(
                "INSERT INTO enrich_log(run_id, at, backend, op, request_json, quoted_micro_usd,
                                        cost_micro_usd, status)
                 VALUES ('r1', 1, 'mock', 'profile', '{}', 10000, 12000, 'ok'),
                        ('r1', 2, 'mock', 'profile', '{}', 10000, NULL, 'quoted');",
            )
            .unwrap();
        let s = g.dashboard_stats().unwrap();
        assert_eq!(s.people, 5);
        assert_eq!(s.enrich.requests_sent, 1);
        assert_eq!(s.enrich.spent_micro_usd, 12000);
        let q = g.review().unwrap();
        assert_eq!(s.review.flagged, q.flagged.len() as u64);
        assert_eq!(s.review.suggestions, q.suggestions.len() as u64);
        assert_eq!(g.sources().unwrap().sources.len(), 3);
    }
}
