//! The review queue's suggestion and info sections (entity-resolution.md
//! section 8): large-company guard suggestions (ER-11), Jaro-Winkler
//! support signals (ER-10), shared-handle and conflicting-override notes
//! (ER-12, ER-19).

mod resolve_support;

use openstance_core::config::Config;
use openstance_core::resolve::{self, fields, review};
use resolve_support::*;
use rusqlite::Connection;

fn settle(c: &Connection, cfg: &Config) {
    let res = resolve::resolve(c, cfg).unwrap();
    resolve::materialize_all(c, &res).unwrap();
    fields::materialize_fields(c, None).unwrap();
}

#[test]
fn er11_common_name_at_a_large_company_is_a_0_6_suggestion() {
    let c = new_db();
    let mut cfg = Config::default();
    cfg.resolution.large_company_threshold = 3;
    let o = ("owner@example.com", "");
    mbox(
        &c,
        &[
            Mail::new("m1", ("priya.s@acme.example", "Priya Sharma"), o),
            Mail::new("m2", ("a1@acme.example", "Ada One"), o),
            Mail::new("m3", ("a2@acme.example", "Ada Two"), o),
        ],
    );
    connection(&c, &["linkedin_url:priya-sharma"], "Priya Sharma", "Acme");
    settle(&c, &cfg);
    let q = review::queue(&c, &cfg).unwrap();
    assert_eq!(q.suggestions.len(), 1);
    assert_eq!(q.suggestions[0].rule, "large_company");
    assert_eq!(q.suggestions[0].confidence, 0.6);
    assert!(q.flagged.is_empty());
}

#[test]
fn er08_common_name_at_different_companies_suggests_nothing() {
    let c = new_db();
    mbox(
        &c,
        &[Mail::new(
            "m1",
            ("priya.s@acme.example", "Priya Sharma"),
            ("owner@example.com", ""),
        )],
    );
    connection(&c, &["linkedin_url:priya-sharma"], "Priya Sharma", "Stripe");
    settle(&c, &Config::default());
    assert!(review::queue(&c, &Config::default())
        .unwrap()
        .suggestions
        .is_empty());
}

#[test]
fn er10_jaro_winkler_needs_a_supporting_signal() {
    let c = new_db();
    // Common-ish names, no shared company or thread: no suggestion.
    mbox(
        &c,
        &[Mail::new(
            "m1",
            ("jon.smith@one.example", "Jon Smith"),
            ("owner@example.com", ""),
        )],
    );
    connection(&c, &["linkedin_url:john-smith"], "John Smith", "Two");
    settle(&c, &Config::default());
    assert!(review::queue(&c, &Config::default())
        .unwrap()
        .suggestions
        .is_empty());
    // Same company: suggested at 0.5 * jw.
    connection(&c, &["linkedin_url:john-smith-1"], "John Smith", "One");
    settle(&c, &Config::default());
    let q = review::queue(&c, &Config::default()).unwrap();
    assert_eq!(q.suggestions.len(), 1, "{:?}", q.suggestions);
    let s = &q.suggestions[0];
    assert_eq!(s.rule, "jaro_winkler");
    let jw = strsim::jaro_winkler("jon smith", "john smith");
    assert!((s.confidence - 0.5 * jw).abs() < 1e-4);
}

#[test]
fn er12_shared_handles_and_er19_conflicts_are_info_rows() {
    let c = new_db();
    card(
        &c,
        "vcf",
        &["email:info@startup.example", "phone:+12015550101"],
        "Jane Doe",
        None,
    );
    card(
        &c,
        "vcf",
        &["email:info@startup.example", "phone:+12015550102"],
        "John Roe",
        None,
    );
    let (a, b) = (
        identity_id(&c, "phone:+12015550101"),
        identity_id(&c, "phone:+12015550102"),
    );
    let info = identity_id(&c, "email:info@startup.example");
    add_override(&c, a, "split_from", Some(&b.to_string()), 1);
    add_override(&c, info, "merged_into", Some(&a.to_string()), 2);
    let conflict = add_override(&c, b, "merged_into", Some(&info.to_string()), 3);
    settle(&c, &Config::default());
    let q = review::queue(&c, &Config::default()).unwrap();
    let kinds: Vec<&str> = q.info.iter().map(|i| i.kind.as_str()).collect();
    assert_eq!(kinds, ["shared_handle", "conflicting_override"]);
    assert!(q.info[0].message.contains("2 cards"));
    assert!(q.info[1].message.contains(&format!("#{conflict}")));
}

#[test]
fn reject_is_idempotent_and_refuses_one_person() {
    let c = new_db();
    mbox(
        &c,
        &[Mail::new(
            "m1",
            ("a@one.example", ""),
            ("b@two.example", ""),
        )],
    );
    settle(&c, &Config::default());
    let (a, b) = (
        identity_id(&c, "email:a@one.example"),
        identity_id(&c, "email:b@two.example"),
    );
    let first = review::reject(&c, a, b, 1).unwrap();
    assert_eq!(review::reject(&c, a, b, 2).unwrap(), first);
    assert!(review::reject(&c, a, a, 3).is_err());
    assert!(review::confirm(&c, a, 4).is_err(), "nothing flagged");
}
