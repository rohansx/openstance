//! MockEnricher behaviour (P3-ENR-03).
#![cfg(feature = "mock")]

use openstance_enrich::mock::{MockEnricher, MockEvent};
use openstance_enrich::{
    EnrichData, EnrichError, EnrichJob, EnrichRequest, Enricher, QuoteBasis, VerifyStatus,
};

fn job(person_id: Option<i64>, request: EnrichRequest) -> EnrichJob {
    EnrichJob { person_id, request }
}

fn priya_profile() -> EnrichRequest {
    EnrichRequest::Profile {
        name: "Priya Raman".into(),
        company: Some("Stripe".into()),
        linkedin: None,
    }
}

#[test]
fn fixtures_load_and_answer_matching_requests() {
    let m = MockEnricher::from_fixtures()
        .unwrap()
        .with_fetched_at(1_700_000_000);
    assert_eq!(m.name(), "mock");
    for op in [
        "profile",
        "find_email",
        "verify_email",
        "company",
        "web_search",
    ] {
        assert!(m.supports(op), "{op}");
    }

    let r = m.enrich(&job(Some(7), priya_profile())).unwrap();
    assert_eq!(r.job.person_id, Some(7));
    assert_eq!(r.provider.as_deref(), Some("mockdata"));
    assert_eq!(r.cost_micro_usd, 10_000);
    assert_eq!(r.fetched_at, 1_700_000_000);
    match r.data {
        EnrichData::Profile(p) => {
            assert_eq!(p.title.as_deref(), Some("Staff Engineer"));
            assert_eq!(p.past_companies, ["Northwind Traders", "Contoso"]);
        }
        other => panic!("unexpected {other:?}"),
    }

    let v = m
        .enrich(&job(
            None,
            EnrichRequest::VerifyEmail {
                email: "bounce@acme.example".into(),
            },
        ))
        .unwrap();
    assert!(matches!(
        v.data,
        EnrichData::Verification {
            status: VerifyStatus::Undeliverable,
            ..
        }
    ));
}

#[test]
fn unknown_request_is_not_found_and_still_costs_the_quoted_price() {
    let m = MockEnricher::new().with_price("company", 12_345);
    let r = m
        .enrich(&job(
            None,
            EnrichRequest::Company {
                domain: "nowhere.example".into(),
            },
        ))
        .unwrap();
    assert_eq!(r.data, EnrichData::NotFound);
    assert_eq!(r.cost_micro_usd, 12_345);
}

#[test]
fn records_quotes_and_calls_in_order() {
    let m = MockEnricher::from_fixtures().unwrap();
    let jobs = vec![
        job(Some(1), priya_profile()),
        job(
            Some(1),
            EnrichRequest::FindEmail {
                name: "Priya Raman".into(),
                domain: "stripe.com".into(),
            },
        ),
    ];
    let lines = m.quote(&jobs).unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].job, jobs[0]);
    assert_eq!(lines[0].basis, QuoteBasis::Exact);
    assert!(m.calls().is_empty(), "quote must not enrich");

    m.enrich(&jobs[0]).unwrap();
    m.enrich(&jobs[1]).unwrap();
    assert_eq!(m.calls(), jobs);
    assert_eq!(m.quotes(), vec![jobs.clone()]);
    assert_eq!(
        m.events(),
        vec![
            MockEvent::Quote(jobs.clone()),
            MockEvent::Enrich(jobs[0].clone()),
            MockEvent::Enrich(jobs[1].clone()),
        ]
    );
    m.reset();
    assert!(m.events().is_empty());
}

#[test]
fn quote_is_configurable_per_op() {
    let m = MockEnricher::new()
        .with_price("profile", 1)
        .with_price("web_search", 2)
        .with_basis(QuoteBasis::Estimate)
        .with_provider_hint("mockdata");
    let lines = m
        .quote(&[
            job(None, priya_profile()),
            job(None, EnrichRequest::WebSearch { query: "q".into() }),
        ])
        .unwrap();
    assert_eq!(lines[0].micro_usd, 1);
    assert_eq!(lines[1].micro_usd, 2);
    assert_eq!(lines[1].basis, QuoteBasis::Estimate);
    assert_eq!(lines[1].provider_hint.as_deref(), Some("mockdata"));
    // Default price is $0.01 per call.
    let d = MockEnricher::new()
        .quote(&[job(None, priya_profile())])
        .unwrap();
    assert_eq!(d[0].micro_usd, 10_000);
}

#[test]
fn responses_and_failures_can_be_set_in_code() {
    let q = EnrichRequest::WebSearch { query: "x".into() };
    let bad = EnrichRequest::Company {
        domain: "down.example".into(),
    };
    let m = MockEnricher::new()
        .with_response(q.clone(), EnrichData::WebHits { hits: vec![] })
        .fail_on(bad.clone())
        .fail_with(
            EnrichRequest::VerifyEmail {
                email: "a@b.c".into(),
            },
            || EnrichError::Timeout,
        );
    assert_eq!(
        m.enrich(&job(None, q)).unwrap().data,
        EnrichData::WebHits { hits: vec![] }
    );
    assert!(matches!(
        m.enrich(&job(None, bad)),
        Err(EnrichError::Upstream(_))
    ));
    assert!(matches!(
        m.enrich(&job(
            None,
            EnrichRequest::VerifyEmail {
                email: "a@b.c".into()
            }
        )),
        Err(EnrichError::Timeout)
    ));
    // Failed calls are still recorded: they were attempted.
    assert_eq!(m.calls().len(), 3);
}

#[test]
fn unsupported_ops_are_refused() {
    let m = MockEnricher::new().without_op("verify_email");
    assert!(!m.supports("verify_email"));
    let j = job(
        None,
        EnrichRequest::VerifyEmail {
            email: "a@b.c".into(),
        },
    );
    assert!(matches!(
        m.quote(std::slice::from_ref(&j)),
        Err(EnrichError::Unsupported(_))
    ));
    assert!(matches!(m.enrich(&j), Err(EnrichError::Unsupported(_))));
}

#[test]
fn quote_failure_is_configurable() {
    let m = MockEnricher::new().quote_fails_with(|| EnrichError::Auth("nope".into()));
    assert!(matches!(
        m.quote(&[job(None, priya_profile())]),
        Err(EnrichError::Auth(_))
    ));
}

#[test]
fn bad_fixture_dir_is_an_error() {
    let dir = std::env::temp_dir().join(format!("os-enrich-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("x.json"), "{ not json").unwrap();
    let err = MockEnricher::from_dir(&dir).unwrap_err();
    assert!(err.to_string().contains("x.json"), "{err}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn usable_as_trait_object_across_threads() {
    let m: std::sync::Arc<dyn Enricher> = std::sync::Arc::new(MockEnricher::new());
    let m2 = m.clone();
    std::thread::spawn(move || {
        m2.enrich(&job(None, EnrichRequest::WebSearch { query: "t".into() }))
            .unwrap();
    })
    .join()
    .unwrap();
}
