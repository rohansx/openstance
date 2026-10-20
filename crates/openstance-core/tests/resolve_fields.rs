//! Step 15: person fields from importer hints by source priority, then
//! scalar overrides (data-model.md 4.5 and 4.7, entity-resolution.md 4).

mod resolve_support;

use openstance_core::config::Config;
use openstance_core::import::{
    Direction, Handle, HandleKind, Hints, InteractionKind, Participant, RawInteraction, Role, Sink,
};
use openstance_core::resolve::{self, fields};
use resolve_support::*;
use rusqlite::Connection;

fn settle(c: &Connection, cfg: &Config) {
    let res = resolve::resolve(c, cfg).unwrap();
    resolve::materialize_all(c, &res).unwrap();
    fields::materialize_fields(c, None).unwrap();
}

fn field(c: &Connection, spec: &str, col: &str) -> Option<String> {
    let id = identity_id(c, spec);
    c.query_row(
        &format!(
            "SELECT p.{col} FROM person p JOIN identity i ON i.person_id = p.id WHERE i.id = ?1"
        ),
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

fn flag(c: &Connection, spec: &str, col: &str) -> i64 {
    let id = identity_id(c, spec);
    c.query_row(
        &format!(
            "SELECT p.{col} FROM person p JOIN identity i ON i.person_id = p.id WHERE i.id = ?1"
        ),
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

fn linkedin_row(
    c: &Connection,
    slug: &str,
    email: Option<&str>,
    name: &str,
    title: &str,
    company: &str,
) {
    let mut handles = vec![Handle::new(
        HandleKind::LinkedinUrl,
        format!("linkedin.com/in/{slug}"),
    )];
    if let Some(e) = email {
        handles.push(Handle::new(HandleKind::Email, e));
    }
    with_sink(c, "linkedin_csv", |s| {
        s.interaction(RawInteraction {
            kind: InteractionKind::Connection,
            at: NOW - 300 * DAY,
            dir_hint: Some(Direction::Mutual),
            subject: None,
            external_id: Some(format!("conn:{slug}")),
            thread_key: None,
            list_unsub: false,
            participants: vec![Participant {
                handles,
                role: Role::Subject,
                hints: Hints {
                    name: Some(name.into()),
                    title: Some(title.into()),
                    company: Some(company.into()),
                    location: None,
                },
            }],
        })
    });
}

#[test]
fn linkedin_beats_vcf_beats_email_display_names() {
    let c = new_db();
    let o = ("owner@example.com", "");
    mbox(
        &c,
        &[
            Mail::new("m1", ("priya@northwind.example", "Priya R."), o),
            Mail::new(
                "m2",
                ("priya@northwind.example", "Priya Raman (Northwind)"),
                o,
            ),
            Mail::new(
                "m3",
                ("priya@northwind.example", "Priya Raman (Northwind)"),
                o,
            ),
        ],
    );
    settle(&c, &Config::default());
    assert_eq!(
        field(&c, "email:priya@northwind.example", "display_name").as_deref(),
        Some("Priya Raman (Northwind)")
    );

    card(
        &c,
        "vcf",
        &["email:priya@northwind.example", "phone:+12015550142"],
        "Priya Raman",
        Some("Northwind Inc"),
    );
    settle(&c, &Config::default());
    assert_eq!(
        field(&c, "email:priya@northwind.example", "display_name").as_deref(),
        Some("Priya Raman")
    );
    assert_eq!(
        field(&c, "email:priya@northwind.example", "company").as_deref(),
        Some("Northwind Inc")
    );

    linkedin_row(
        &c,
        "priya-raman-nw",
        Some("priya@northwind.example"),
        "Priya Raman-Iyer",
        "VP Engineering",
        "Northwind",
    );
    settle(&c, &Config::default());
    assert_eq!(
        field(&c, "email:priya@northwind.example", "display_name").as_deref(),
        Some("Priya Raman-Iyer")
    );
    assert_eq!(
        field(&c, "email:priya@northwind.example", "title").as_deref(),
        Some("VP Engineering")
    );
    assert_eq!(
        field(&c, "email:priya@northwind.example", "company").as_deref(),
        Some("Northwind")
    );
    let src = fields::field_sources(&c, identity_id(&c, "email:priya@northwind.example")).unwrap();
    assert_eq!(src["title"].as_deref(), Some("linkedin_csv:linkedin_csv"));
    assert_eq!(src["location"], None);
}

#[test]
fn single_token_display_names_fall_back_to_the_local_part() {
    let c = new_db();
    mbox(
        &c,
        &[Mail::new(
            "m1",
            ("jane.doe@acme.example", "Jane"),
            ("owner@example.com", ""),
        )],
    );
    settle(&c, &Config::default());
    assert_eq!(
        field(&c, "email:jane.doe@acme.example", "display_name").as_deref(),
        Some("Jane Doe")
    );
}

#[test]
fn company_from_a_mapped_email_domain_only() {
    let c = new_db();
    let o = ("owner@example.com", "");
    mbox(
        &c,
        &[
            Mail::new("m1", ("a@alphabet.com", "Ada Example"), o),
            Mail::new("m2", ("b@contoso.example", "Bo Example"), o),
        ],
    );
    settle(&c, &Config::default());
    assert_eq!(
        field(&c, "email:a@alphabet.com", "company").as_deref(),
        Some("Google")
    );
    assert_eq!(field(&c, "email:b@contoso.example", "company"), None);
}

#[test]
fn scalar_overrides_win_tombstones_fall_back_and_empty_is_explicit() {
    let c = new_db();
    linkedin_row(&c, "sam-ito-nw", None, "Sam Ito", "Engineer", "Northwind");
    settle(&c, &Config::default());
    let p = identity_id(&c, "linkedin_url:sam-ito-nw");
    add_override(&c, p, "title", Some("CTO"), 10);
    fields::materialize_fields(&c, Some(&[p])).unwrap();
    assert_eq!(
        field(&c, "linkedin_url:sam-ito-nw", "title").as_deref(),
        Some("CTO")
    );
    assert_eq!(
        fields::field_sources(&c, p).unwrap()["title"].as_deref(),
        Some("override")
    );
    add_override(&c, p, "title", None, 11);
    fields::materialize_fields(&c, Some(&[p])).unwrap();
    assert_eq!(
        field(&c, "linkedin_url:sam-ito-nw", "title").as_deref(),
        Some("Engineer")
    );
    add_override(&c, p, "company", Some(""), 12);
    fields::materialize_fields(&c, Some(&[p])).unwrap();
    assert_eq!(
        field(&c, "linkedin_url:sam-ito-nw", "company").as_deref(),
        Some("")
    );
}

#[test]
fn er17_title_override_on_an_absorbed_person_survives_a_merge() {
    let c = new_db();
    let o = ("owner@example.com", "");
    mbox(
        &c,
        &[
            Mail::new("m1", ("a@one.example", "Ann Example"), o),
            Mail::new("m2", ("b@two.example", "Ann Example"), o),
        ],
    );
    let (a, b) = (
        identity_id(&c, "email:a@one.example"),
        identity_id(&c, "email:b@two.example"),
    );
    add_override(&c, b, "title", Some("Partner"), 5);
    add_override(&c, b, "merged_into", Some(&a.to_string()), 6);
    settle(&c, &Config::default());
    assert_eq!(
        field(&c, "email:a@one.example", "title").as_deref(),
        Some("Partner")
    );
}

#[test]
fn er18_privacy_flags_survive_rule_merges() {
    let c = new_db();
    let o = ("owner@example.com", "");
    mbox(
        &c,
        &[
            Mail::new("m1", ("a@gmail.com", "Ann Example"), o),
            Mail::new("m2", ("a+x@gmail.com", ""), o),
        ],
    );
    let (a, b) = (
        identity_id(&c, "email:a@gmail.com"),
        identity_id(&c, "email:a+x@gmail.com"),
    );
    add_override(&c, b, "do_not_share", Some("1"), 5);
    add_override(&c, a, "do_not_share", Some("0"), 6);
    add_override(&c, b, "no_enrich", Some("1"), 5);
    settle(&c, &Config::default());
    assert_eq!(flag(&c, "email:a@gmail.com", "do_not_share"), 1);
    assert_eq!(flag(&c, "email:a@gmail.com", "no_enrich"), 1);
}

#[test]
fn location_comes_from_vcf_hints() {
    let c = new_db();
    with_sink(&c, "vcf", |s| {
        s.identity(openstance_core::import::RawIdentity {
            handle: handle("email:lars@tailspin.example"),
            hints: Hints {
                name: Some("Lars Eriksen".into()),
                location: Some("Oslo, Oslo".into()),
                ..Hints::default()
            },
            record: "c1".into(),
            observed_at: None,
        })
    });
    settle(&c, &Config::default());
    assert_eq!(
        field(&c, "email:lars@tailspin.example", "location").as_deref(),
        Some("Oslo, Oslo")
    );
}
