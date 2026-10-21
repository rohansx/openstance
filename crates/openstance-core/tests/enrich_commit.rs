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
