//! Reciprocity edge cases from scoring.md section 2.4: DM reply window,
//! subject-fallback threading, replies that need an earlier outbound
//! message to them, and counting an interaction once per person.

mod score_support;

use openstance_core::score;
use score_support::{Seed, DAY, NOW};

fn dm(s: &mut Seed, from: i64, to: i64, at: i64, dir: &str, conv: &str) {
    s.interaction(
        "linkedin_csv",
        "dm",
        at,
        dir,
        Some(conv),
        false,
        None,
        &[(from, "sender"), (to, "recipient")],
    );
}

#[test]
fn score_dm_reply_counts_only_within_14_days() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let a = s.person("a@northwind.example");
    // Conversation 1: they answer after 3 days -> reply.
    dm(&mut s, owner, a, NOW - 100 * DAY, "out", "c1");
    dm(&mut s, a, owner, NOW - 97 * DAY, "in", "c1");
    // Conversation 2: they answer after 30 days -> not a reply.
    dm(&mut s, owner, a, NOW - 80 * DAY, "out", "c2");
    dm(&mut s, a, owner, NOW - 50 * DAY, "in", "c2");
    let i = score::inputs(&s.conn, a, NOW).unwrap();
    assert_eq!(i.n_out, 2);
    assert_eq!(i.n_replies, 1);
    assert_eq!(i.n_threads_started, 0);
}

#[test]
fn score_inbound_on_a_thread_without_my_message_is_not_a_reply() {
    // They answer a thread started by someone else that I never wrote on.
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let a = s.person("a@northwind.example");
    let b = s.person("b@northwind.example");
    s.interaction(
        "mbox",
        "email",
        NOW - 10 * DAY,
        "in",
        Some("t"),
        false,
        Some("plan"),
        &[(b, "from"), (owner, "to"), (a, "cc")],
    );
    s.email_in(owner, a, NOW - 9 * DAY, "t");
    s.email_out(owner, a, NOW - 8 * DAY, "u");
    let i = score::inputs(&s.conn, a, NOW).unwrap();
    assert_eq!(i.n_replies, 0);
    assert_eq!(i.n_threads_started, 0, "b started the thread");
    assert_eq!(i.n_out, 1);
    let ib = score::inputs(&s.conn, b, NOW).unwrap();
    assert_eq!(ib.n_threads_started, 1);
}

#[test]
fn score_subject_fallback_without_thread_keys() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let a = s.person("a@northwind.example");
    let mail = |s: &mut Seed, from: i64, to: i64, at: i64, dir: &str, subj: &str| {
        s.interaction(
            "mbox",
            "email",
            at,
            dir,
            None,
            false,
            Some(subj),
            &[(from, "from"), (to, "to")],
        );
    };
    mail(&mut s, owner, a, NOW - 40 * DAY, "out", "Q3 plan");
    mail(&mut s, a, owner, NOW - 39 * DAY, "in", "Re: q3   PLAN"); // reply
    mail(&mut s, a, owner, NOW - 20 * DAY, "in", "Lunch?"); // starts a thread
    mail(&mut s, a, owner, NOW - 5 * DAY, "in", "RE: Q3 plan"); // > 30 days after: new thread
    mail(&mut s, a, owner, NOW - 4 * DAY, "in", ""); // no subject: neither
    let i = score::inputs(&s.conn, a, NOW).unwrap();
    assert_eq!(i.n_out, 1);
    assert_eq!(i.n_replies, 1);
    assert_eq!(i.n_threads_started, 2);
    assert_eq!(i.n_interactions, 5);
}

#[test]
fn score_interaction_counts_once_per_person_across_roles() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let a = s.person("a@northwind.example");
    let alias = s.alias(a, "a.alt@northwind.example");
    let id = s.email_out(owner, a, NOW - DAY, "t");
    // Same human also cc'd under another handle.
    s.conn
        .execute(
            "INSERT INTO interaction_person(interaction_id, identity_id, role, slot, person_id)
             VALUES (?1, ?2, 'cc', 2, ?3)",
            rusqlite::params![id, alias, a],
        )
        .unwrap();
    let i = score::inputs(&s.conn, a, NOW).unwrap();
    assert_eq!(i.n_interactions, 1);
    assert_eq!(i.n_out, 1);
}

#[test]
fn score_future_email_clamps_to_zero_days() {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let a = s.person("a@northwind.example");
    s.email_in(owner, a, NOW + 2 * DAY, "t");
    let i = score::inputs(&s.conn, a, NOW).unwrap();
    assert_eq!(i.days_since_last, Some(0.0));
    assert_eq!(i.last_contact, Some(NOW + 2 * DAY));
}

#[test]
fn score_no_contact_interactions_means_r_zero() {
    let mut s = Seed::new();
    let _owner = s.owner("owner@example.com");
    let a = s.person("a@northwind.example");
    s.interaction(
        "x_archive",
        "follow",
        NOW - DAY,
        "out",
        None,
        false,
        None,
        &[(a, "subject")],
    );
    s.interaction(
        "vcf",
        "contact",
        NOW - DAY,
        "none",
        None,
        false,
        None,
        &[(a, "subject")],
    );
    let i = score::inputs(&s.conn, a, NOW).unwrap();
    assert_eq!(i.last_contact, None);
    assert_eq!(i.n_interactions, 0);
    assert_eq!(i.channels, vec![score::Channel::Social]);
    assert!(score::inputs(&s.conn, 424_242, NOW).is_err());
}
