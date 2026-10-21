//! P1-CORE-07: resolution rules 1-4, overrides as must/cannot-links, shared
//! handles, the dormant filter and determinism (entity-resolution.md
//! sections 2-5, test ids ER-xx from section 11).

mod resolve_support;

use openstance_core::config::Config;
use openstance_core::resolve::{materialize_all, MatchRule};
use resolve_support::*;

#[test]
fn er01_same_email_in_mbox_and_vcf_is_one_person() {
    let c = new_db();
    mbox(
        &c,
        &[Mail::new(
            "m1",
            ("kofi@tailspin.example", "Kofi Mensah"),
            ("owner@example.com", ""),
        )],
    );
    card(
        &c,
        "vcf",
        &["email:kofi@tailspin.example", "phone:+12015550111"],
        "Kofi Mensah",
        None,
    );
    let res = resolve(&c, &Config::default());
    assert!(same(
        &c,
        &res,
        "email:kofi@tailspin.example",
        "phone:+12015550111"
    ));
    assert_eq!(
        rule_of(&c, &res, "email:kofi@tailspin.example"),
        (MatchRule::Anchor, 1.0)
    );
    assert_eq!(
        rule_of(&c, &res, "phone:+12015550111"),
        (MatchRule::Exact, 1.0)
    );
}

#[test]
fn er02_one_vcard_with_two_emails_and_a_phone_is_one_person() {
    let c = new_db();
    card(
        &c,
        "vcf",
        &[
            "email:mira.c@mail.example",
            "email:mira@tailspin.example",
            "phone:+12015550188",
        ],
        "Mira Castellanos",
        Some("Tailspin"),
    );
    let res = resolve(&c, &Config::default());
    assert_eq!(res.people().len(), 1);
}

#[test]
fn er03_gmail_dots_plus_and_googlemail_are_one_person() {
    let c = new_db();
    let o = ("owner@example.com", "");
    mbox(
        &c,
        &[
            Mail::new("m1", ("a.b@gmail.com", ""), o),
            Mail::new("m2", ("ab@gmail.com", ""), o),
            Mail::new("m3", ("ab+x@googlemail.com", ""), o),
        ],
    );
    let res = resolve(&c, &Config::default());
    assert!(same(&c, &res, "email:a.b@gmail.com", "email:ab@gmail.com"));
    assert!(same(
        &c,
        &res,
        "email:a.b@gmail.com",
        "email:ab+x@googlemail.com"
    ));
    assert_eq!(
        rule_of(&c, &res, "email:ab@gmail.com"),
        (MatchRule::GmailAlias, 0.98)
    );
    assert_eq!(
        rule_of(&c, &res, "email:ab+x@googlemail.com"),
        (MatchRule::GmailAlias, 0.98)
    );
}

#[test]
fn gmail_domains_come_from_config() {
    let c = new_db();
    let o = ("owner@example.com", "");
    mbox(
        &c,
        &[
            Mail::new("m1", ("dana.whitfield@gmail.test", ""), o),
            Mail::new("m2", ("danawhitfield+lists@googlemail.test", ""), o),
        ],
    );
    assert!(!same(
        &c,
        &resolve(&c, &Config::default()),
        "email:dana.whitfield@gmail.test",
        "email:danawhitfield+lists@googlemail.test"
    ));
    let mut cfg = Config::default();
    cfg.resolution.gmail_domains = vec!["gmail.test".into(), "googlemail.test".into()];
    assert!(same(
        &c,
        &resolve(&c, &cfg),
        "email:dana.whitfield@gmail.test",
        "email:danawhitfield+lists@googlemail.test"
    ));
}

#[test]
fn er04_dots_on_other_domains_are_significant() {
    let c = new_db();
    let o = ("owner@example.com", "");
    mbox(
        &c,
        &[
            Mail::new("m1", ("a.b@acme.example", ""), o),
            Mail::new("m2", ("ab@acme.example", ""), o),
        ],
    );
    let res = resolve(&c, &Config::default());
    assert!(!same(
        &c,
        &res,
        "email:a.b@acme.example",
        "email:ab@acme.example"
    ));
}

#[test]
fn er05_corporate_plus_addressing_is_an_alias() {
    let c = new_db();
    let o = ("owner@example.com", "");
    mbox(
        &c,
        &[
            Mail::new("m1", ("jane+rec@acme.example", ""), o),
            Mail::new("m2", ("jane@acme.example", ""), o),
        ],
    );
    let res = resolve(&c, &Config::default());
    assert!(same(
        &c,
        &res,
        "email:jane+rec@acme.example",
        "email:jane@acme.example"
    ));
    assert_eq!(
        rule_of(&c, &res, "email:jane@acme.example"),
        (MatchRule::GmailAlias, 0.98)
    );
}

#[test]
fn er06_name_and_company_merge_is_flagged() {
    let c = new_db();
    mbox(
        &c,
        &[Mail::new(
            "m1",
            ("jane@stripe.example", "Jane Doe"),
            ("owner@example.com", ""),
        )],
    );
    connection(
        &c,
        &["linkedin_url:https://www.linkedin.com/in/jane-doe-0a1b2c"],
        "Jane Doe",
        "Stripe",
    );
    let res = resolve(&c, &Config::default());
    assert!(same(
        &c,
        &res,
        "email:jane@stripe.example",
        "linkedin_url:jane-doe-0a1b2c"
    ));
    assert_eq!(
        rule_of(&c, &res, "linkedin_url:jane-doe-0a1b2c"),
        (MatchRule::NameCompany, 0.85)
    );
}

#[test]
fn name_company_does_not_match_on_different_companies() {
    let c = new_db();
    mbox(
        &c,
        &[Mail::new(
            "m1",
            ("jane@acme.example", "Jane Doe"),
            ("owner@example.com", ""),
        )],
    );
    connection(&c, &["linkedin_url:jane-doe"], "Jane Doe", "Stripe");
    let res = resolve(&c, &Config::default());
    assert!(!same(
        &c,
        &res,
        "email:jane@acme.example",
        "linkedin_url:jane-doe"
    ));
}

#[test]
fn er07_same_name_on_one_thread_is_flagged_name_thread() {
    let c = new_db();
    mbox(
        &c,
        &[Mail::new(
            "m1",
            ("tariq@fabrikam.example", "Tariq Bellweather"),
            ("owner@example.com", ""),
        )
        .thread("t")
        .cc(("tariq.bell@gmail.com", "Tariq Bellweather"))],
    );
    let res = resolve(&c, &Config::default());
    assert!(same(
        &c,
        &res,
        "email:tariq@fabrikam.example",
        "email:tariq.bell@gmail.com"
    ));
    assert_eq!(
        rule_of(&c, &res, "email:tariq.bell@gmail.com"),
        (MatchRule::NameThread, 0.75)
    );
}

#[test]
fn er08_common_name_alone_never_merges() {
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
    let res = resolve(&c, &Config::default());
    assert!(!same(
        &c,
        &res,
        "email:priya.s@acme.example",
        "linkedin_url:priya-sharma"
    ));
}

#[test]
fn er09_rare_name_alone_never_merges() {
    let c = new_db();
    card(
        &c,
        "vcf",
        &["email:r.quill@mail.example"],
        "Rohan Quillfeather",
        None,
    );
    connection(
        &c,
        &["linkedin_url:rohan-quill"],
        "Rohan Quillfeather",
        "Example Labs",
    );
    let res = resolve(&c, &Config::default());
    assert!(!same(
        &c,
        &res,
        "email:r.quill@mail.example",
        "linkedin_url:rohan-quill"
    ));
}

#[test]
fn er11_large_company_guard_blocks_common_names() {
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
    let res = resolve(&c, &cfg);
    assert!(!same(
        &c,
        &res,
        "email:priya.s@acme.example",
        "linkedin_url:priya-sharma"
    ));
    // A rare name at the same large company still merges (flagged).
    mbox(
        &c,
        &[Mail::new(
            "m4",
            ("rq@acme.example", "Rohan Quillfeather"),
            o,
        )],
    );
    connection(&c, &["linkedin_url:rohan-q"], "Rohan Quillfeather", "Acme");
    let res = resolve(&c, &cfg);
    assert!(same(
        &c,
        &res,
        "email:rq@acme.example",
        "linkedin_url:rohan-q"
    ));
}

#[test]
fn er12_role_address_on_two_cards_glues_nobody() {
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
    let res = resolve(&c, &Config::default());
    assert!(!same(&c, &res, "phone:+12015550101", "phone:+12015550102"));
    assert!(!same(
        &c,
        &res,
        "email:info@startup.example",
        "phone:+12015550101"
    ));
    assert!(!same(
        &c,
        &res,
        "email:info@startup.example",
        "phone:+12015550102"
    ));
    assert_eq!(res.people().len(), 3);
}

#[test]
fn shared_family_handle_on_two_cards_stays_separate() {
    let c = new_db();
    card(
        &c,
        "vcf",
        &["email:family@home.example", "phone:+12015550103"],
        "Jane Doe",
        None,
    );
    card(
        &c,
        "vcf",
        &["email:family@home.example", "phone:+12015550104"],
        "John Doe",
        None,
    );
    let res = resolve(&c, &Config::default());
    assert!(!same(&c, &res, "phone:+12015550103", "phone:+12015550104"));
    assert_eq!(res.people().len(), 3);
}

#[test]
fn role_address_claimed_by_exactly_one_card_attaches() {
    let c = new_db();
    card(
        &c,
        "vcf",
        &["phone:+12015550105", "email:hello@solo.example"],
        "Solo Founder",
        None,
    );
    let res = resolve(&c, &Config::default());
    assert!(same(
        &c,
        &res,
        "email:hello@solo.example",
        "phone:+12015550105"
    ));
}

#[test]
fn er13_multi_name_handle_is_never_used_for_matching() {
    let c = new_db();
    let o = ("owner@example.com", "");
    let names = [
        "Jane Doe",
        "Ada One",
        "Bo Two",
        "Cy Three",
        "Di Four",
        "Ed Five",
        "Fay Six",
        "Gil Seven",
        "Hal Eight",
        "Ivy Nine",
    ];
    let ids: Vec<String> = (0..names.len()).map(|i| format!("n{i}")).collect();
    let display: Vec<String> = names
        .iter()
        .map(|n| format!("{n} (via Google Docs)"))
        .collect();
    let mails: Vec<Mail<'_>> = (0..names.len())
        .map(|i| {
            Mail::new(
                &ids[i],
                ("comments-noreply@docs.google.com", &display[i]),
                o,
            )
        })
        .collect();
    mbox(&c, &mails);
    mbox(&c, &[Mail::new("j", ("jane@google.com", "Jane Doe"), o)]);
    let res = resolve(&c, &Config::default());
    assert!(!same(
        &c,
        &res,
        "email:comments-noreply@docs.google.com",
        "email:jane@google.com"
    ));
}

#[test]
fn er19_conflicting_must_links_skip_the_later_one_deterministically() {
    let c = new_db();
    let o = ("owner@example.com", "");
    mbox(
        &c,
        &[
            Mail::new("m1", ("a@one.example", ""), o),
            Mail::new("m2", ("b@two.example", ""), o),
            Mail::new("m3", ("c@three.example", ""), o),
        ],
    );
    let (a, b, cc) = (
        identity_id(&c, "email:a@one.example"),
        identity_id(&c, "email:b@two.example"),
        identity_id(&c, "email:c@three.example"),
    );
    add_override(&c, a, "split_from", Some(&cc.to_string()), 1);
    add_override(&c, b, "merged_into", Some(&a.to_string()), 2);
    let later = add_override(&c, cc, "merged_into", Some(&b.to_string()), 3);
    let res = resolve(&c, &Config::default());
    assert!(same(&c, &res, "email:a@one.example", "email:b@two.example"));
    assert!(!same(
        &c,
        &res,
        "email:b@two.example",
        "email:c@three.example"
    ));
    assert_eq!(res.conflicting_overrides, vec![later]);
    assert_eq!(
        rule_of(&c, &res, "email:b@two.example"),
        (MatchRule::Override, 1.0)
    );
    assert!(res.blocked_by_override >= 1);
}

#[test]
fn split_from_blocks_even_exact_co_reference() {
    let c = new_db();
    mbox(
        &c,
        &[
            Mail::new("m1", ("x@one.example", ""), ("owner@example.com", "")),
            Mail::new("m2", ("y@two.example", ""), ("owner@example.com", "")),
        ],
    );
    let (x, y) = (
        identity_id(&c, "email:x@one.example"),
        identity_id(&c, "email:y@two.example"),
    );
    add_override(&c, y, "split_from", Some(&x.to_string()), 1);
    card(
        &c,
        "vcf",
        &["email:x@one.example", "email:y@two.example"],
        "Xavier Yarrow",
        None,
    );
    let res = resolve(&c, &Config::default());
    assert!(!same(
        &c,
        &res,
        "email:x@one.example",
        "email:y@two.example"
    ));
    assert_eq!(res.blocked_by_override, 1);
}

#[test]
fn er22_follow_only_ids_without_names_are_dormant() {
    let c = new_db();
    with_sink(&c, "x_archive", |s| {
        use openstance_core::import::*;
        s.interaction(RawInteraction {
            kind: InteractionKind::Follow,
            at: NOW,
            dir_hint: Some(Direction::Out),
            subject: None,
            external_id: Some("follow:out:42".into()),
            thread_key: None,
            list_unsub: false,
            participants: vec![Participant {
                handles: vec![Handle::new(HandleKind::XUserId, "42")],
                role: Role::Subject,
                hints: Hints::default(),
            }],
        });
    });
    let res = resolve(&c, &Config::default());
    assert!(res.assignments.is_empty());
}

#[test]
fn er23_me_handles_form_one_self_person() {
    let c = new_db();
    let cfg = cfg_with_me(&[
        "email:owner@example.com",
        "linkedin_url:owen-ryder-example",
        "email:missing@example.com",
    ]);
    mbox(
        &c,
        &[Mail::new(
            "m1",
            ("owner@example.com", "Owen Ryder"),
            ("kofi@tailspin.example", ""),
        )],
    );
    with_sink(&c, "linkedin_csv", |s| {
        use openstance_core::import::*;
        s.interaction(RawInteraction {
            kind: InteractionKind::Dm,
            at: NOW,
            dir_hint: None,
            subject: None,
            external_id: Some("dm1".into()),
            thread_key: Some("c1".into()),
            list_unsub: false,
            participants: vec![
                Participant {
                    handles: vec![Handle::new(
                        HandleKind::LinkedinUrl,
                        "linkedin.com/in/owen-ryder-example",
                    )],
                    role: Role::Sender,
                    hints: Hints::default(),
                },
                Participant {
                    handles: vec![Handle::new(
                        HandleKind::LinkedinUrl,
                        "linkedin.com/in/hugo-brandt",
                    )],
                    role: Role::Recipient,
                    hints: Hints::default(),
                },
            ],
        });
    });
    let res = resolve(&c, &cfg);
    assert!(same(
        &c,
        &res,
        "email:owner@example.com",
        "linkedin_url:owen-ryder-example"
    ));
    assert_eq!(
        res.self_person,
        person_of(&c, &res, "email:owner@example.com")
    );
    assert_eq!(
        rule_of(&c, &res, "linkedin_url:owen-ryder-example"),
        (MatchRule::Override, 1.0)
    );
}

#[test]
fn person_ids_are_anchor_identity_ids_and_resolution_is_deterministic() {
    let c = new_db();
    card(
        &c,
        "vcf",
        &["phone:+12015550142", "email:priya@northwind.example"],
        "Priya Raman",
        None,
    );
    mbox(
        &c,
        &[Mail::new(
            "m1",
            ("priya@northwind.example", "Priya Raman"),
            ("owner@example.com", ""),
        )],
    );
    let a = resolve(&c, &Config::default());
    let b = resolve(&c, &Config::default());
    assert_eq!(a, b);
    let p = person_of(&c, &a, "email:priya@northwind.example").unwrap();
    assert_eq!(
        p,
        identity_id(&c, "phone:+12015550142").min(identity_id(&c, "email:priya@northwind.example"))
    );
}

#[test]
fn materialize_all_writes_people_assignments_self_and_direction() {
    let c = new_db();
    let cfg = cfg_with_me(&["email:owner@example.com"]);
    let o = ("owner@example.com", "Owen Ryder");
    mbox(
        &c,
        &[
            Mail::new("in1", ("kofi@tailspin.example", "Kofi Mensah"), o),
            Mail::new("out1", o, ("kofi@tailspin.example", "Kofi Mensah")),
            Mail::new("other", ("x@one.example", ""), ("y@two.example", "")),
        ],
    );
    card(
        &c,
        "vcf",
        &["email:kofi@tailspin.example", "phone:+12015550111"],
        "Kofi Mensah",
        None,
    );
    let res = resolve(&c, &cfg);
    let m = materialize_all(&c, &res).unwrap();
    assert!(m.removed.is_empty());
    assert_eq!(count(&c, "SELECT count(*) FROM person"), 4);
    assert_eq!(
        count(&c, "SELECT count(*) FROM person WHERE is_self = 1"),
        1
    );
    assert_eq!(
        count(&c, "SELECT count(*) FROM identity WHERE person_id IS NULL OR confidence IS NULL OR match_rule IS NULL"),
        0
    );
    assert_eq!(
        count(&c, "SELECT count(*) FROM interaction_person ip JOIN identity i ON i.id = ip.identity_id WHERE ip.person_id IS NOT i.person_id"),
        0
    );
    let dir = |ext: &str| -> String {
        c.query_row(
            "SELECT direction FROM interaction WHERE external_id = ?1",
            [ext],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(dir("in1"), "in");
    assert_eq!(dir("out1"), "out");
    assert_eq!(dir("other"), "none");
    assert_eq!(dir("contact:card"), "none");
    let cached: String = c
        .query_row(
            "SELECT value FROM meta WHERE key = 'self_identity_ids'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        cached,
        format!("[{}]", identity_id(&c, "email:owner@example.com"))
    );
}
