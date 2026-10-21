//! P3-CORE-02: enrichment results are written under `source.kind =
//! 'enrich:<backend>'`, never beat an override, survive `rebuild` (which
//! replays them from `enrich_log`), and show in `who` provenance.

use std::path::Path;

use openstance_core::egress::EgressGate;
use openstance_core::enrich::mock::MockEnricher;
use openstance_core::enrich::{EnrichData, EnrichJob, EnrichRequest, ProfileData};
use openstance_core::import::ImportOptions;
use openstance_core::ingest::ImportRequest;
use openstance_core::overrides::{Edit, OverrideField};
use openstance_core::resolve::lookup::{resolve_person, PersonKey};
use openstance_core::{Config, FixedClock, Graph, PersonId, Providers};
use proptest::prelude::*;

const NOW: i64 = 1_792_497_600;

fn graph() -> Graph {
    let importers = openstance_core::import::registry(&ImportOptions::default());
    let mut g = Graph::open_in_memory(
        Config::default(),
        Providers::with_importers(importers),
        Box::new(FixedClock(NOW)),
    )
    .unwrap();
    let vcf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/vcf/contacts.vcf");
    g.import(&[vcf], &ImportRequest::default()).unwrap();
    g
}

fn id(g: &Graph, name: &str) -> PersonId {
    resolve_person(g.conn(), &PersonKey::Name(name.into()))
        .unwrap()
        .person
}

fn profile_req(name: &str) -> EnrichRequest {
    EnrichRequest::Profile {
        name: name.into(),
        company: None,
        linkedin: None,
    }
}

fn profile_data(title: &str, company: &str) -> EnrichData {
    EnrichData::Profile(ProfileData {
        display_name: None,
        title: Some(title.into()),
        company: Some(company.into()),
        company_domain: None,
        location: Some("Remote".into()),
        linkedin: None,
        past_companies: vec![],
        schools: vec![],
        headline: Some("Builds data platforms".into()),
    })
}

/// Quote, approve, send one profile job for `person`, and commit.
fn enrich(g: &Graph, mock: &MockEnricher, person: PersonId, name: &str) {
    let job = EnrichJob {
        person_id: Some(person),
        request: profile_req(name),
    };
    let mut gate = EgressGate::new(g, mock, "refresh", format!("run-{person}"), 500_000);
    let q = gate.quote(vec![job.clone()]).unwrap();
    gate.approve(&q).unwrap();
    gate.execute(&job).unwrap();
    let stats = gate.commit().unwrap();
    assert!(stats.source_id.is_some());
}

fn field(g: &Graph, person: PersonId, f: &str) -> Option<String> {
    g.conn()
        .query_row(
            &format!("SELECT {f} FROM person WHERE id = ?1"),
            [person],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn results_land_under_an_enrich_source_and_fill_empty_fields() {
    let g = graph();
    let ines = id(&g, "Ines Quillfeather");
    assert_eq!(field(&g, ines, "title"), None);
    let mock = MockEnricher::new().with_response(
        profile_req("Ines Quillfeather"),
        profile_data("Head of Data", "Ramp"),
    );
    enrich(&g, &mock, ines, "Ines Quillfeather");
    assert_eq!(field(&g, ines, "title").as_deref(), Some("Head of Data"));
    assert_eq!(field(&g, ines, "company").as_deref(), Some("Ramp"));
    assert_eq!(
        field(&g, ines, "bio").as_deref(),
        Some("Builds data platforms")
    );
    let kind: String = g
        .conn()
        .query_row(
            "SELECT s.kind FROM enrich_log e JOIN source s ON s.id = e.source_id",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kind, "enrich:mock");
    // `who` provenance names the enrichment source.
    let who = g.who(&PersonKey::Id(ines)).unwrap();
    let src = who.person.field_sources["title"].clone().unwrap();
    assert!(src.starts_with("enrich:mock"), "{src}");
}

#[test]
fn override_beats_enrichment_after_commit_and_after_rebuild() {
    let mut g = graph();
    let priya = id(&g, "Priya Raman");
    g.edit(
        &PersonKey::Id(priya),
        &Edit {
            set: vec![(OverrideField::Title, "CTO".into())],
            ..Edit::default()
        },
    )
    .unwrap();
    let mock = MockEnricher::new()
        .with_response(profile_req("Priya Raman"), profile_data("VP Eng", "Stripe"));
    enrich(&g, &mock, priya, "Priya Raman");
    assert_eq!(field(&g, priya, "title").as_deref(), Some("CTO"));
    // Enrichment outranks importer hints for fields without an override.
    assert_eq!(field(&g, priya, "company").as_deref(), Some("Stripe"));
    g.rebuild().unwrap();
    assert_eq!(field(&g, priya, "title").as_deref(), Some("CTO"));
    assert_eq!(field(&g, priya, "company").as_deref(), Some("Stripe"));
    let who = g.who(&PersonKey::Id(priya)).unwrap();
    assert_eq!(
        who.person.field_sources["title"].as_deref(),
        Some("override")
    );
}

#[test]
fn uncommitted_and_failed_results_are_never_applied() {
    let g = graph();
    let ines = id(&g, "Ines Quillfeather");
    let mock = MockEnricher::new().with_response(
        profile_req("Ines Quillfeather"),
        profile_data("Head of Data", "Ramp"),
    );
    let job = EnrichJob {
        person_id: Some(ines),
        request: profile_req("Ines Quillfeather"),
    };
    // Quoted only (egress not allowed): no fact.
    let mut gate = EgressGate::new(&g, &mock, "refresh", "r1", 500_000);
    gate.quote(vec![job]).unwrap();
    drop(gate);
    let mut g = g;
    g.rebuild().unwrap();
    assert_eq!(field(&g, ines, "title"), None);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]
    #[test]
    fn override_then_enrich_then_rebuild_override_wins(
        f in prop::sample::select(vec!["title", "company"]),
        mine in "[A-Z][a-z]{2,8}",
        theirs in "[A-Z][a-z]{2,8}",
    ) {
        let mut g = graph();
        let lena = id(&g, "Lena Park");
        let of = if f == "title" { OverrideField::Title } else { OverrideField::Company };
        let edit = Edit { set: vec![(of, mine.clone())], ..Edit::default() };
        g.edit(&PersonKey::Id(lena), &edit).unwrap();
        let mock = MockEnricher::new()
            .with_response(profile_req("Lena Park"), profile_data(&theirs, &theirs));
        enrich(&g, &mock, lena, "Lena Park");
        g.rebuild().unwrap();
        prop_assert_eq!(field(&g, lena, f), Some(mine));
    }
}

// ---------------------------------------------------------------------------
// Identities from enrichment (enrichment.md section 7, entity-resolution.md
// section 7): found emails and LinkedIn URLs become identity rows under the
// run's `enrich:<backend>` source, attach to the person through resolution
// step 12, and are replayed by `rebuild`.
// ---------------------------------------------------------------------------

fn find_req(name: &str) -> EnrichRequest {
    EnrichRequest::FindEmail {
        name: name.into(),
        domain: "ramp.example".into(),
    }
}

fn verify_req(email: &str) -> EnrichRequest {
    EnrichRequest::VerifyEmail {
        email: email.into(),
    }
}

/// Quote, approve, send `requests` for `person` in order, and commit.
fn run(g: &Graph, mock: &MockEnricher, person: PersonId, requests: Vec<EnrichRequest>) {
    let jobs: Vec<EnrichJob> = requests
        .into_iter()
        .map(|request| EnrichJob {
            person_id: Some(person),
            request,
        })
        .collect();
    let mut gate = EgressGate::new(g, mock, "refresh", format!("run-{person}"), 500_000);
    let q = gate.quote(jobs.clone()).unwrap();
    gate.approve(&q).unwrap();
    for j in &jobs {
        gate.execute(j).unwrap();
    }
    gate.commit().unwrap();
}

fn identity_of(
    g: &Graph,
    person: PersonId,
    kind: &str,
    value: &str,
) -> Option<openstance_core::json::Identity> {
    g.who(&PersonKey::Id(person))
        .unwrap()
        .identities
        .into_iter()
        .find(|i| i.kind == kind && i.value == value)
}

fn identity_row(g: &Graph, kind: &str, value: &str) -> Option<(i64, String, Option<i64>)> {
    use rusqlite::OptionalExtension;
    g.conn()
        .query_row(
            "SELECT i.id, s.kind, i.person_id FROM identity i JOIN source s ON s.id = i.source_id
             WHERE i.kind = ?1 AND i.value = ?2",
            [kind, value],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .unwrap()
}

#[test]
fn found_email_becomes_a_normalized_enrich_identity_that_survives_rebuild() {
    let mut g = graph();
    let ines = id(&g, "Ines Quillfeather");
    let mock = MockEnricher::new().with_response(
        find_req("Ines Quillfeather"),
        EnrichData::Email {
            email: "  Ines.Q@Ramp.Example ".into(),
            confidence: 0.97,
        },
    );
    run(&g, &mock, ines, vec![find_req("Ines Quillfeather")]);
    let (_, kind, owner) = identity_row(&g, "email", "ines.q@ramp.example").expect("identity row");
    assert_eq!(kind, "enrich:mock");
    assert_eq!(owner, Some(ines));
    let ident = identity_of(&g, ines, "email", "ines.q@ramp.example").expect("on the person");
    assert_eq!(ident.match_rule.as_deref(), Some("enrich"));
    // Capped at 0.9 whatever the backend says.
    assert!((ident.confidence.unwrap() - 0.9).abs() < 1e-6);
    assert_eq!(ident.source.kind, "enrich:mock");
    let before = g.who(&PersonKey::Id(ines)).unwrap().identities;
    g.rebuild().unwrap();
    assert_eq!(id(&g, "Ines Quillfeather"), ines);
    assert_eq!(g.who(&PersonKey::Id(ines)).unwrap().identities, before);
}

#[test]
fn found_email_confidence_below_the_cap_is_kept() {
    let g = graph();
    let ines = id(&g, "Ines Quillfeather");
    let mock = MockEnricher::new().with_response(
        find_req("Ines Quillfeather"),
        EnrichData::Email {
            email: "ines@ramp.example".into(),
            confidence: 0.6,
        },
    );
    run(&g, &mock, ines, vec![find_req("Ines Quillfeather")]);
    let ident = identity_of(&g, ines, "email", "ines@ramp.example").unwrap();
    assert!((ident.confidence.unwrap() - 0.6).abs() < 1e-6);
}

#[test]
fn profile_linkedin_url_becomes_a_slug_identity() {
    let mut g = graph();
    let ines = id(&g, "Ines Quillfeather");
    let mut data = profile_data("Head of Data", "Ramp");
    if let EnrichData::Profile(p) = &mut data {
        p.linkedin = Some("https://www.linkedin.com/in/Ines-Quill-0a1b2c/".into());
    }
    let mock = MockEnricher::new().with_response(profile_req("Ines Quillfeather"), data);
    enrich(&g, &mock, ines, "Ines Quillfeather");
    let ident = identity_of(&g, ines, "linkedin_url", "ines-quill-0a1b2c").expect("slug");
    assert_eq!(ident.match_rule.as_deref(), Some("enrich"));
    assert!((ident.confidence.unwrap() - 0.9).abs() < 1e-6);
    g.rebuild().unwrap();
    assert!(identity_of(&g, ines, "linkedin_url", "ines-quill-0a1b2c").is_some());
}

#[test]
fn an_email_that_belongs_to_someone_else_is_never_merged() {
    let mut g = graph();
    let ines = id(&g, "Ines Quillfeather");
    let priya = id(&g, "Priya Raman");
    let mock = MockEnricher::new().with_response(
        find_req("Ines Quillfeather"),
        EnrichData::Email {
            email: "priya@northwind.example".into(),
            confidence: 0.9,
        },
    );
    run(&g, &mock, ines, vec![find_req("Ines Quillfeather")]);
    for _ in 0..2 {
        assert_ne!(id(&g, "Ines Quillfeather"), id(&g, "Priya Raman"));
        let mine = identity_of(&g, priya, "email", "priya@northwind.example").unwrap();
        assert_ne!(mine.match_rule.as_deref(), Some("enrich"));
        assert_eq!(mine.source.kind, "vcf");
        assert!(identity_of(&g, ines, "email", "priya@northwind.example").is_none());
        g.rebuild().unwrap();
    }
}

#[test]
fn a_split_override_beats_an_enrichment_identity_after_rebuild() {
    let mut g = graph();
    let ines = id(&g, "Ines Quillfeather");
    let mock = MockEnricher::new().with_response(
        find_req("Ines Quillfeather"),
        EnrichData::Email {
            email: "wrong@ramp.example".into(),
            confidence: 0.8,
        },
    );
    run(&g, &mock, ines, vec![find_req("Ines Quillfeather")]);
    let (ident, _, _) = identity_row(&g, "email", "wrong@ramp.example").unwrap();
    assert!(identity_of(&g, ines, "email", "wrong@ramp.example").is_some());
    g.split(&PersonKey::Id(ines), &[ident]).unwrap();
    g.rebuild().unwrap();
    assert!(identity_of(&g, ines, "email", "wrong@ramp.example").is_none());
}

#[test]
fn no_enrich_people_get_no_enrichment_identities() {
    let mut g = graph();
    let ines = id(&g, "Ines Quillfeather");
    g.edit(
        &PersonKey::Id(ines),
        &Edit {
            set: vec![(OverrideField::NoEnrich, "1".into())],
            ..Edit::default()
        },
    )
    .unwrap();
    let mock = MockEnricher::new().with_response(
        find_req("Ines Quillfeather"),
        EnrichData::Email {
            email: "ines@ramp.example".into(),
            confidence: 0.9,
        },
    );
    let job = EnrichJob {
        person_id: Some(ines),
        request: find_req("Ines Quillfeather"),
    };
    let mut gate = EgressGate::new(&g, &mock, "refresh", "run-ne", 500_000);
    let q = gate.quote(vec![job.clone()]).unwrap();
    assert!(q.lines.is_empty());
    gate.commit().unwrap();
    g.rebuild().unwrap();
    assert!(mock.calls().is_empty());
    assert!(identity_row(&g, "email", "ines@ramp.example").is_none());
}

#[test]
fn verification_status_is_stored_on_the_email_identity_and_survives_rebuild() {
    let mut g = graph();
    let ines = id(&g, "Ines Quillfeather");
    let mock = MockEnricher::new()
        .with_fetched_at(NOW)
        .with_response(
            find_req("Ines Quillfeather"),
            EnrichData::Email {
                email: "ines@ramp.example".into(),
                confidence: 0.9,
            },
        )
        .with_response(
            verify_req("ines@ramp.example"),
            EnrichData::Verification {
                email: "ines@ramp.example".into(),
                status: openstance_core::enrich::VerifyStatus::Deliverable,
            },
        )
        .with_response(
            verify_req("ines.q@mail.test"),
            EnrichData::Verification {
                email: "ines.q@mail.test".into(),
                status: openstance_core::enrich::VerifyStatus::Risky,
            },
        );
    run(
        &g,
        &mock,
        ines,
        vec![
            find_req("Ines Quillfeather"),
            verify_req("ines@ramp.example"),
            verify_req("ines.q@mail.test"),
        ],
    );
    for _ in 0..2 {
        let found = identity_of(&g, ines, "email", "ines@ramp.example").unwrap();
        let v = found.verification.expect("verification on the found email");
        assert_eq!(v.status, "deliverable");
        assert!(v.via.starts_with("enrich:mock"), "{}", v.via);
        // An imported email can be verified too.
        let imported = identity_of(&g, ines, "email", "ines.q@mail.test").unwrap();
        assert_eq!(imported.verification.unwrap().status, "risky");
        g.rebuild().unwrap();
    }
}
