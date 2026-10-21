//! P1-CORE-10 bulk-sender detection (ingestion.md section 8, scoring.md
//! section 3), on rows seeded directly against the 0001 schema.

mod score_support;

use openstance_core::config::{BulkSection, ScoreSection};
use openstance_core::score::{self, bulk, ScoreConfig};
use score_support::{Seed, DAY, NOW};

fn cfg() -> ScoreConfig {
    ScoreConfig::from_config(&ScoreSection::default())
        .unwrap()
        .0
}

/// `n` inbound messages from `them`, each its own thread.
fn inbound(s: &mut Seed, owner: i64, them: i64, n: i64, unsub: i64, tag: &str) {
    for k in 0..n {
        s.interaction(
            "mbox",
            "email",
            NOW - (k + 1) * DAY,
            "in",
            Some(&format!("{tag}-{k}")),
            k < unsub,
            Some("update"),
            &[(them, "from"), (owner, "to")],
        );
    }
}

#[test]
fn bulk_volume_without_reply_is_flagged_at_threshold() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let fifty = s.person("alerts@vendor.example");
    let forty_nine = s.person("ops@vendor.example");
    inbound(&mut s, owner, fifty, 50, 0, "a");
    inbound(&mut s, owner, forty_nine, 49, 0, "b");
    let n = bulk::detect(&s.conn, &cfg(), None).unwrap();
    assert_eq!(n, 1);
    assert!(s.is_bulk(fifty));
    assert!(!s.is_bulk(forty_nine));
}

#[test]
fn bulk_busy_two_way_correspondent_is_not_flagged() {
    // Like fixture Priya: 52 inbound, 26 outbound, threaded.
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let priya = s.person("priya@northwind.example");
    for k in 0..26 {
        let t = format!("p-{k}");
        s.email_out(owner, priya, NOW - (2 * k + 2) * DAY, &t);
        s.email_in(owner, priya, NOW - (2 * k + 1) * DAY, &t);
        s.email_in(owner, priya, NOW - (2 * k + 1) * DAY + 60, &t);
    }
    assert_eq!(bulk::detect(&s.conn, &cfg(), None).unwrap(), 0);
    assert!(!s.is_bulk(priya));
}

#[test]
fn bulk_volume_rule_needs_no_owner_message_on_their_threads() {
    // The owner never wrote to them, but did write on one of their threads.
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let them = s.person("chatty@vendor.example");
    let colleague = s.person("colleague@northwind.example");
    inbound(&mut s, owner, them, 60, 0, "c");
    s.interaction(
        "mbox",
        "email",
        NOW,
        "out",
        Some("c-3"),
        false,
        Some("re: update"),
        &[(owner, "from"), (colleague, "to")],
    );
    bulk::detect(&s.conn, &cfg(), None).unwrap();
    assert!(!s.is_bulk(them));
}

#[test]
fn bulk_list_header_rule_needs_more_than_the_ratio() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let half = s.person("half@lists.example");
    let most = s.person("most@lists.example");
    let two = s.person("two@lists.example");
    inbound(&mut s, owner, half, 4, 2, "h"); // 50%: not more than half
    inbound(&mut s, owner, most, 3, 2, "m"); // 67%, 3 messages
    inbound(&mut s, owner, two, 2, 1, "t"); // under 3 messages
                                            // Owner writes to each so the other rules stay quiet.
    for (p, t) in [(half, "h-x"), (most, "m-x"), (two, "t-x")] {
        s.email_out(owner, p, NOW - 100 * DAY, t);
    }
    assert_eq!(bulk::detect(&s.conn, &cfg(), None).unwrap(), 1);
    assert!(!s.is_bulk(half));
    assert!(s.is_bulk(most));
    assert!(!s.is_bulk(two));
}

#[test]
fn bulk_noreply_local_part_without_outbound() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let noreply = s.person("no-reply@shop.example");
    let support = s.person("support@saas.example");
    inbound(&mut s, owner, noreply, 2, 0, "n");
    inbound(&mut s, owner, support, 2, 0, "s");
    s.email_out(owner, support, NOW - 3 * DAY, "s-x");
    bulk::detect(&s.conn, &cfg(), None).unwrap();
    assert!(s.is_bulk(noreply));
    assert!(!s.is_bulk(support), "I wrote to support");
}

#[test]
fn bulk_noreply_rule_checks_every_email_identity() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let p = s.person("billing@shop.example");
    s.alias(p, "notifications@shop.example");
    inbound(&mut s, owner, p, 1, 0, "x");
    bulk::detect(&s.conn, &cfg(), None).unwrap();
    assert!(s.is_bulk(p));
}

#[test]
fn bulk_auto_submitted_only_sender() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let auto = s.person("robot@ci.example");
    let saved = s.person("alerts@ci.example");
    inbound(&mut s, owner, auto, 1, 1, "r");
    inbound(&mut s, owner, saved, 1, 1, "q");
    // A vCard entry means the owner saved this contact on purpose.
    s.interaction(
        "vcf",
        "contact",
        NOW,
        "none",
        None,
        false,
        None,
        &[(saved, "subject")],
    );
    bulk::detect(&s.conn, &cfg(), None).unwrap();
    assert!(s.is_bulk(auto));
    assert!(!s.is_bulk(saved));
}

#[test]
fn bulk_override_always_wins_and_tombstone_restores() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let news = s.person("news@digest.example");
    let friend = s.person("friend@northwind.example");
    inbound(&mut s, owner, news, 60, 60, "d");
    s.email_out(owner, friend, NOW - DAY, "f");
    s.email_in(owner, friend, NOW, "f");

    s.add_override(news, "is_bulk", Some("0"), NOW - 10);
    s.add_override(friend, "is_bulk", Some("1"), NOW - 10);
    bulk::detect(&s.conn, &cfg(), None).unwrap();
    assert!(!s.is_bulk(news), "--not-bulk wins over every rule");
    assert!(s.is_bulk(friend), "is_bulk = '1' override wins too");

    // A later tombstone removes the override; an older value never wins.
    s.add_override(news, "is_bulk", None, NOW);
    s.add_override(friend, "is_bulk", Some("0"), NOW - 20);
    bulk::detect(&s.conn, &cfg(), None).unwrap();
    assert!(s.is_bulk(news));
    assert!(s.is_bulk(friend));
}

#[test]
fn bulk_override_on_a_merged_anchor_applies_to_the_cluster() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let news = s.person("news@digest.example");
    let alias = s.alias(news, "digest@digest.example");
    inbound(&mut s, owner, news, 60, 60, "d");
    s.add_override(alias, "is_bulk", Some("0"), NOW);
    bulk::detect(&s.conn, &cfg(), None).unwrap();
    assert!(!s.is_bulk(news));
}

#[test]
fn bulk_self_is_never_bulk() {
    let mut s = Seed::new();
    let owner = s.owner("info@owner.example");
    let other = s.person("x@northwind.example");
    for k in 0..60 {
        s.interaction(
            "mbox",
            "email",
            NOW - k * DAY,
            "out",
            Some(&format!("o-{k}")),
            true,
            None,
            &[(owner, "from"), (other, "to")],
        );
    }
    bulk::detect(&s.conn, &cfg(), None).unwrap();
    assert!(!s.is_bulk(owner));
}

#[test]
fn bulk_people_score_zero_with_zero_components() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let news = s.person("news@digest.example");
    inbound(&mut s, owner, news, 60, 60, "d");
    s.meeting(owner, &[news], NOW - DAY);
    let report = score::recompute_all(&s.conn, &cfg(), NOW).unwrap();
    assert_eq!(report.bulk, 1);
    assert!(s.is_bulk(news));
    assert_eq!(s.stored(news), [0.0; 6]);
}

#[test]
fn bulk_detect_limited_to_people() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let a = s.person("news@a.example");
    let b = s.person("news@b.example");
    inbound(&mut s, owner, a, 60, 60, "a");
    inbound(&mut s, owner, b, 60, 60, "b");
    assert_eq!(bulk::detect(&s.conn, &cfg(), Some(&[a])).unwrap(), 1);
    assert!(s.is_bulk(a));
    assert!(!s.is_bulk(b), "not in the list, not touched");
}

#[test]
fn bulk_thresholds_come_from_config() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let p = s.person("ops@vendor.example");
    inbound(&mut s, owner, p, 10, 0, "o");
    let (c, _) = ScoreConfig::from_config(&ScoreSection {
        bulk: BulkSection {
            min_messages: 10,
            list_unsubscribe_ratio: 0.5,
        },
        ..ScoreSection::default()
    })
    .unwrap();
    bulk::detect(&s.conn, &cfg(), None).unwrap();
    assert!(!s.is_bulk(p));
    bulk::detect(&s.conn, &c, None).unwrap();
    assert!(s.is_bulk(p));
}

#[test]
fn bulk_classify_names_the_rule() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let news = s.person("weekly@digest.example");
    let friend = s.person("friend@northwind.example");
    inbound(&mut s, owner, news, 60, 0, "d");
    s.email_out(owner, friend, NOW, "f");
    assert_eq!(
        bulk::classify(&s.conn, &cfg(), news).unwrap(),
        Some(bulk::BulkRule::Volume)
    );
    assert_eq!(bulk::classify(&s.conn, &cfg(), friend).unwrap(), None);
    assert_eq!(bulk::BulkRule::Volume.as_str(), "volume_without_reply");
}
