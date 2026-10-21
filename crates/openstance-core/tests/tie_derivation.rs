//! P1-CORE-13 tie derivation (scoring.md section 8, data-model.md 4.6), on
//! rows seeded directly against the 0001 schema.

mod score_support;

use openstance_core::config::ScoreSection;
use openstance_core::score::{self, ties, ScoreConfig};
use rusqlite::params;
use score_support::{Seed, DAY, NOW};

fn cfg() -> ScoreConfig {
    ScoreConfig::from_config(&ScoreSection::default())
        .unwrap()
        .0
}

/// Owner writes to `to`, cc `cc`.
fn cc_mail(s: &mut Seed, owner: i64, to: i64, cc: &[i64], at: i64, thread: &str) -> i64 {
    let mut parts = vec![(owner, "from"), (to, "to")];
    parts.extend(cc.iter().map(|c| (*c, "cc")));
    s.interaction(
        "mbox",
        "email",
        at,
        "out",
        Some(thread),
        false,
        Some(thread),
        &parts,
    )
}

#[test]
fn tie_two_people_cc_d_on_three_threads() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let priya = s.person("priya@northwind.example");
    let marcus = s.person("marcus@contoso.example");
    let ages = [3, 40, 200];
    let mut ids = Vec::new();
    for (k, age) in ages.iter().enumerate() {
        ids.push(cc_mail(
            &mut s,
            owner,
            priya,
            &[marcus],
            NOW - age * DAY,
            &format!("t{k}"),
        ));
    }
    // Not shared: priya alone.
    s.email_out(owner, priya, NOW - DAY, "solo");

    let report = score::recompute_all(&s.conn, &cfg(), NOW).unwrap();
    assert_eq!(report.ties, 1);
    let rows = s.ties();
    assert_eq!(rows.len(), 1);
    let (a, b, kind, weight, last_at, n) = &rows[0];
    assert_eq!((*a, *b), (priya.min(marcus), priya.max(marcus)));
    assert_eq!(kind, "email");
    assert_eq!(*n, 3);
    assert_eq!(*last_at, NOW - 3 * DAY);
    let want = ties::tie_weight(&[3.0, 40.0, 200.0], 180.0, 3.0);
    assert!((weight - want).abs() < 1e-12);
    // Hand value: D = 2^(-3/180) + 2^(-40/180) + 2^(-200/180).
    let d: f64 = [3.0f64, 40.0, 200.0]
        .iter()
        .map(|x| 2f64.powf(-x / 180.0))
        .sum();
    assert!((weight - (1.0 - (-d / 3.0).exp())).abs() < 1e-12);

    // Evidence: exactly those 3, newest first, either argument order.
    let ev = ties::evidence(&s.conn, priya, marcus, 10).unwrap();
    assert_eq!(
        ev.iter().map(|e| e.interaction_id).collect::<Vec<_>>(),
        vec![ids[0], ids[1], ids[2]]
    );
    assert!(ev.iter().all(|e| e.kind == "email" && e.source_id == 1));
    assert_eq!(ev[0].subject.as_deref(), Some("t0"));
    assert_eq!(ev[0].at, NOW - 3 * DAY);
    assert_eq!(ties::evidence(&s.conn, marcus, priya, 2).unwrap().len(), 2);
}

#[test]
fn tie_owner_never_appears() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let a = s.person("a@northwind.example");
    let b = s.person("b@northwind.example");
    cc_mail(&mut s, owner, a, &[b], NOW, "t");
    s.email_out(owner, a, NOW, "u");
    s.meeting(owner, &[a], NOW - DAY);
    score::recompute_all(&s.conn, &cfg(), NOW).unwrap();
    for (x, y, ..) in s.ties() {
        assert_ne!(x, owner);
        assert_ne!(y, owner);
        assert!(x < y);
    }
    assert_eq!(s.ties().len(), 1);
}

#[test]
fn tie_kinds_follow_interaction_kind() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let a = s.person("a@northwind.example");
    let b = s.person("b@northwind.example");
    cc_mail(&mut s, owner, a, &[b], NOW - DAY, "t");
    s.meeting(owner, &[a, b], NOW - 2 * DAY);
    s.interaction(
        "whatsapp",
        "dm",
        NOW - 3 * DAY,
        "in",
        Some("g1"),
        false,
        None,
        &[(a, "sender"), (owner, "member"), (b, "member")],
    );
    // Connections, contacts, follows and mentions never tie people.
    for kind in ["connection", "contact", "follow", "mention"] {
        s.interaction(
            "linkedin_csv",
            kind,
            NOW - DAY,
            "none",
            None,
            false,
            None,
            &[(a, "subject"), (b, "subject")],
        );
    }
    score::recompute_all(&s.conn, &cfg(), NOW).unwrap();
    let kinds: Vec<String> = s.ties().into_iter().map(|t| t.2).collect();
    assert_eq!(kinds, vec!["dm", "email", "meeting"]);
    assert!(s.ties().iter().all(|t| t.5 == 1));
}

#[test]
fn tie_participant_cap() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let people: Vec<i64> = (0..21)
        .map(|k| s.person(&format!("p{k}@crowd.example")))
        .collect();
    // 20 non-owner participants: every pair ties (190 rows).
    cc_mail(
        &mut s,
        owner,
        people[0],
        &people[1..20],
        NOW - DAY,
        "twenty",
    );
    let r = score::recompute_all(&s.conn, &cfg(), NOW).unwrap();
    assert_eq!(r.ties, 190);
    // 21 participants: no ties from that interaction.
    let mut t = Seed::new();
    let owner = t.owner("owner@example.com");
    let people: Vec<i64> = (0..21)
        .map(|k| t.person(&format!("p{k}@crowd.example")))
        .collect();
    cc_mail(
        &mut t,
        owner,
        people[0],
        &people[1..],
        NOW - DAY,
        "all-hands",
    );
    assert_eq!(score::recompute_all(&t.conn, &cfg(), NOW).unwrap().ties, 0);
    assert!(t.ties().is_empty());
}

#[test]
fn tie_bulk_people_are_excluded() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let a = s.person("a@northwind.example");
    let news = s.person("newsletter@digest.example");
    let b = s.person("b@northwind.example");
    for k in 0..60 {
        s.interaction(
            "mbox",
            "email",
            NOW - k * DAY,
            "in",
            Some(&format!("n{k}")),
            true,
            None,
            &[(news, "from"), (owner, "to")],
        );
    }
    // The newsletter "cc'd" with a and b.
    s.interaction(
        "mbox",
        "email",
        NOW,
        "in",
        Some("x"),
        true,
        None,
        &[(news, "from"), (a, "to"), (b, "to"), (owner, "to")],
    );
    // Bulk + a only: one eligible participant, no tie.
    s.interaction(
        "mbox",
        "email",
        NOW,
        "in",
        Some("y"),
        true,
        None,
        &[(news, "from"), (a, "to")],
    );
    score::recompute_all(&s.conn, &cfg(), NOW).unwrap();
    let rows = s.ties();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].0, rows[0].1), (a.min(b), a.max(b)));
}

#[test]
fn tie_future_meetings_are_ignored() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let a = s.person("a@northwind.example");
    let b = s.person("b@northwind.example");
    s.meeting(owner, &[a, b], NOW + 5 * DAY);
    score::recompute_all(&s.conn, &cfg(), NOW).unwrap();
    assert!(s.ties().is_empty());
}

fn crowd() -> (Seed, i64, Vec<i64>) {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let p: Vec<i64> = (0..6)
        .map(|k| s.person(&format!("p{k}@team.example")))
        .collect();
    cc_mail(&mut s, owner, p[0], &[p[1], p[2]], NOW - 10 * DAY, "a");
    cc_mail(&mut s, owner, p[2], &[p[3]], NOW - 20 * DAY, "b");
    s.meeting(owner, &[p[3], p[4], p[5]], NOW - 5 * DAY);
    (s, owner, p)
}

#[test]
fn tie_incremental_matches_full_rebuild() {
    let (mut s, owner, p) = crowd();
    let c = cfg();
    score::recompute_all(&s.conn, &c, NOW).unwrap();
    // New mail touches p1 only.
    cc_mail(&mut s, owner, p[1], &[p[4]], NOW - DAY, "c");
    cc_mail(&mut s, owner, p[1], &[p[2]], NOW - DAY, "d");
    score::recompute_people(&s.conn, &c, &[p[1]], NOW).unwrap();
    let incremental = s.ties();
    score::recompute_all(&s.conn, &c, NOW).unwrap();
    assert_eq!(incremental, s.ties());
}

#[test]
fn tie_incremental_handles_bulk_flip_changing_the_cap() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let people: Vec<i64> = (0..21)
        .map(|k| s.person(&format!("p{k}@crowd.example")))
        .collect();
    cc_mail(&mut s, owner, people[0], &people[1..], NOW - DAY, "all");
    let c = cfg();
    score::recompute_all(&s.conn, &c, NOW).unwrap();
    assert!(s.ties().is_empty());
    // p20 is flagged bulk by override; the other 20 now tie.
    s.add_override(people[20], "is_bulk", Some("1"), NOW);
    score::recompute_people(&s.conn, &c, &[people[20]], NOW).unwrap();
    let incremental = s.ties();
    assert_eq!(incremental.len(), 190);
    score::recompute_all(&s.conn, &c, NOW).unwrap();
    assert_eq!(incremental, s.ties());
}

#[test]
fn tie_rows_of_absorbed_people_are_removed() {
    let (mut s, _owner, p) = crowd();
    let c = cfg();
    score::recompute_all(&s.conn, &c, NOW).unwrap();
    assert!(s.ties().iter().any(|t| t.0 == p[3] || t.1 == p[3]));
    // Simulate resolution merging p3 into p2.
    let tx = s.conn.transaction().unwrap();
    tx.execute(
        "UPDATE identity SET person_id = ?1 WHERE person_id = ?2",
        params![p[2], p[3]],
    )
    .unwrap();
    tx.execute(
        "UPDATE interaction_person SET person_id = ?1 WHERE person_id = ?2",
        params![p[2], p[3]],
    )
    .unwrap();
    tx.execute("DELETE FROM person WHERE id = ?1", [p[3]])
        .unwrap();
    score::recompute_people(&tx, &c, &[p[2]], NOW).unwrap();
    tx.commit().unwrap();
    let rows = s.ties();
    assert!(rows.iter().all(|t| t.0 != p[3] && t.1 != p[3]));
    let incremental = rows;
    score::recompute_all(&s.conn, &c, NOW).unwrap();
    assert_eq!(incremental, s.ties());
}

#[test]
fn tie_weights_decay_with_now() {
    let (s, _owner, _p) = crowd();
    let c = cfg();
    score::recompute_all(&s.conn, &c, NOW).unwrap();
    let before: f64 = s.ties().iter().map(|t| t.3).sum();
    score::recompute_all(&s.conn, &c, NOW + 365 * DAY).unwrap();
    let after: f64 = s.ties().iter().map(|t| t.3).sum();
    assert!(after < before);
    assert!(s.ties().iter().all(|t| t.3 > 0.0 && t.3 < 1.0));
}

#[test]
fn tie_rebuild_function_scoped_and_full() {
    let (s, _owner, p) = crowd();
    let c = cfg();
    score::recompute_all(&s.conn, &c, NOW).unwrap();
    let full = s.ties();
    s.conn.execute("DELETE FROM tie", []).unwrap();
    let n = ties::rebuild(&s.conn, &c, None, NOW).unwrap();
    assert_eq!(n as usize, full.len());
    assert_eq!(s.ties(), full);
    // A scoped rebuild leaves unrelated rows alone and rewrites p0's.
    s.conn
        .execute(
            "UPDATE tie SET weight = 0.123 WHERE a <> ?1 AND b <> ?1",
            [p[0]],
        )
        .unwrap();
    ties::rebuild(&s.conn, &c, Some(&[p[0]]), NOW).unwrap();
    let rows = s.ties();
    assert!(rows
        .iter()
        .filter(|t| t.0 == p[0] || t.1 == p[0])
        .all(|t| t.3 != 0.123));
    assert!(rows.iter().any(|t| t.3 == 0.123));
}
