//! P3-CORE-03..05 with the recording `MockEnricher` over the search test kit
//! (`search::testkit`, NOW = 2026-10-20 12:00 UTC) plus two ties:
//! Priya (2) - Marcus (3) by email, Marcus (3) - Ravi (7) by meeting.

use serde_json::{json, Value};

use super::*;
use crate::config::Config;
use crate::egress::Surface;
use crate::enrich::mock::{MockEnricher, MockEvent};
use crate::enrich::{EnrichData, EnrichRequest, NullEnricher, ProfileData, VerifyStatus, WebHit};
use crate::search::testkit::{self, ago, NOW};
use crate::{FixedClock, Providers};

fn graph() -> Graph {
    let g = Graph::open_in_memory(
        Config::default(),
        Providers::null(),
        Box::new(FixedClock(NOW)),
    )
    .unwrap();
    testkit::seed(g.conn());
    g.conn()
        .execute_batch(&format!(
            "INSERT INTO tie(a, b, kind, weight, last_at, n_shared) VALUES
               (2, 3, 'email', 0.4, {}, 1),
               (3, 7, 'meeting', 0.5, {}, 1);
             UPDATE person SET bio = 'Stanford GSB alum' WHERE id = 5;",
            ago(60),
            ago(20)
        ))
        .unwrap();
    g
}

fn dana_profile() -> EnrichRequest {
    EnrichRequest::Profile {
        name: "Dana Whitfield".into(),
        company: Some("Contoso".into()),
        linkedin: None,
    }
}

fn dana_find() -> EnrichRequest {
    EnrichRequest::FindEmail {
        name: "Dana Whitfield".into(),
        domain: "contoso.example".into(),
    }
}

fn dana_verify() -> EnrichRequest {
    EnrichRequest::VerifyEmail {
        email: "dana@contoso.example".into(),
    }
}

fn mock() -> MockEnricher {
    MockEnricher::new()
        .with_fetched_at(NOW)
        .with_response(
            dana_profile(),
            EnrichData::Profile(ProfileData {
                display_name: Some("Dana Whitfield".into()),
                title: Some("CTO".into()),
                company: Some("Contoso".into()),
                company_domain: Some("contoso.example".into()),
                location: None,
                linkedin: Some("dana-whitfield".into()),
                past_companies: vec!["Northwind".into()],
                schools: vec!["Stanford".into()],
                headline: None,
            }),
        )
        .with_response(
            dana_find(),
            EnrichData::Email {
                email: "dana@contoso.example".into(),
                confidence: 0.9,
            },
        )
        .with_response(
            dana_verify(),
            EnrichData::Verification {
                email: "dana@contoso.example".into(),
                status: VerifyStatus::Deliverable,
            },
        )
}

fn opts(allow: bool) -> RunOptions {
    RunOptions {
        allow_egress: allow,
        run_id: Some("RUN".into()),
        ..RunOptions::default()
    }
}

fn ops(m: &MockEnricher) -> Vec<&'static str> {
    m.calls().iter().map(|j| j.request.op()).collect()
}

fn log_count(g: &Graph) -> i64 {
    g.conn()
        .query_row("SELECT count(*) FROM enrich_log", [], |r| r.get(0))
        .unwrap()
}

fn vias(out: &Outcome) -> Vec<(String, String)> {
    out.result.as_ref().unwrap()["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["via"]["display_name"].as_str().unwrap().to_string(),
                p["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// P3-CORE-03: plan never sends
// ---------------------------------------------------------------------------

#[test]
fn plan_never_calls_enrich() {
    let g = graph();
    let m = mock();
    for (name, input) in [
        (
            "path",
            json!({"target": "Dana Whitfield", "company": "Contoso", "find_email": true}),
        ),
        ("brief", json!({"person": "Priya Raman", "web": true})),
    ] {
        let out = g
            .run_playbook_with(
                &m,
                name,
                &input,
                &RunOptions {
                    plan_only: true,
                    allow_egress: true,
                    ..opts(true)
                },
            )
            .unwrap();
        assert_eq!(out.status, RunStatus::Quote, "{name}");
        assert!(out.result.is_none());
        assert_eq!(out.receipt.requests_sent, 0);
    }
    assert!(m.calls().is_empty());
    assert!(m.events().iter().all(|e| matches!(e, MockEvent::Quote(_))));
}

#[test]
fn unknown_playbook_is_not_found() {
    let g = graph();
    let err = g
        .run_playbook_with(&NullEnricher, "watch", &json!({}), &opts(false))
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}

#[test]
fn outcome_serializes_to_the_documented_shape() {
    let g = graph();
    let out = g
        .run_playbook_with(
            &mock(),
            "path",
            &json!({"target": "Dana Whitfield", "company": "Contoso"}),
            &opts(false),
        )
        .unwrap();
    let v = serde_json::to_value(&out).unwrap();
    assert_eq!(v["v"], 1);
    assert_eq!(v["playbook"], "path");
    assert_eq!(v["status"], "quote");
    assert_eq!(v["plan"]["run_id"], "RUN");
    assert_eq!(v["plan"]["quote"]["quoted_at"], "2026-10-20T12:00:00Z");
    assert_eq!(v["plan"]["steps"][0]["will_run"], "run");
    assert_eq!(v["receipt"]["requests_sent"], 0);
    assert!(v["notes"][0]
        .as_str()
        .unwrap()
        .starts_with("egress not allowed: pass allow_egress=true to send 1 request for $0.0100"));
}

// ---------------------------------------------------------------------------
// P3-CORE-04: path
// ---------------------------------------------------------------------------

#[test]
fn path_to_an_in_graph_target_makes_no_calls_even_when_allowed() {
    let g = graph();
    let m = mock();
    let out = g
        .run_playbook_with(
            &m,
            "path",
            &json!({"target": "Marcus Lee", "find_email": true}),
            &opts(true),
        )
        .unwrap();
    assert_eq!(out.status, RunStatus::Done);
    assert!(m.events().is_empty(), "{:?}", m.events());
    assert_eq!(log_count(&g), 0);
    let r = out.result.as_ref().unwrap();
    assert_eq!(r["target"]["in_graph"], true);
    assert_eq!(r["target"]["person_id"], 3);
    assert_eq!(
        vias(&out),
        vec![
            ("Priya Raman".to_string(), "tie".to_string()),
            ("Ravi Iyer".to_string(), "tie".to_string())
        ]
    );
    assert!(out.plan.quote.lines.is_empty());
    assert!(out
        .plan
        .steps
        .iter()
        .filter(|s| s.egress.is_some())
        .all(|s| s.will_run == StepDecision::SkipLocalSufficient));
}

#[test]
fn path_to_a_stranger_makes_exactly_the_planned_calls_in_order() {
    let g = graph();
    let m = mock();
    let out = g
        .run_playbook_with(
            &m,
            "path",
            &json!({"target": "Dana Whitfield", "company": "Contoso", "find_email": true}),
            &opts(true),
        )
        .unwrap();
    assert_eq!(out.status, RunStatus::Done, "{:?}", out.notes);
    assert_eq!(ops(&m), vec!["profile", "find_email", "verify_email"]);
    assert_eq!(
        m.calls().into_iter().map(|j| j.request).collect::<Vec<_>>(),
        vec![dana_profile(), dana_find(), dana_verify()]
    );
    // The quote equals the sum of the step quotes.
    let steps: MicroUsd = out.plan.steps.iter().map(|s| s.quoted_micro_usd).sum();
    assert_eq!(out.plan.quote.total_micro_usd, steps);
    assert_eq!(steps, 30_000);
    assert_eq!(out.receipt.requests_sent, 3);
    assert_eq!(out.receipt.spent_micro_usd, 30_000);
    let r = out.result.as_ref().unwrap();
    assert_eq!(r["target"]["in_graph"], false);
    assert_eq!(r["target"]["company"], "Contoso");
    assert_eq!(r["target"]["profile_source"], "enrich:mock");
    assert_eq!(
        vias(&out),
        vec![
            ("Marcus Lee".to_string(), "same_company".to_string()),
            ("Priya Raman".to_string(), "past_company".to_string()),
            ("Sam Ito".to_string(), "school".to_string()),
        ]
    );
    let first = &r["paths"][0];
    assert_eq!(first["link_weight"], 1.0);
    assert_eq!(first["strength"], 0.5);
    // Evidence: the matched field, the enrich fact, and my last interaction.
    let kinds: Vec<&str> = first["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"field") && kinds.contains(&"enrich") && kinds.contains(&"interaction")
    );
    assert_eq!(
        r["email"],
        json!({"address": "dana@contoso.example", "status": "deliverable", "source": "enrich:mock"})
    );
    // No person row was created or changed for a stranger.
    let people: i64 = g
        .conn()
        .query_row("SELECT count(*) FROM person", [], |r| r.get(0))
        .unwrap();
    assert_eq!(people, 7);
}

#[test]
fn path_without_find_email_skips_the_email_steps() {
    let g = graph();
    let m = mock();
    let out = g
        .run_playbook_with(
            &m,
            "path",
            &json!({"target": "Dana Whitfield", "company": "Contoso"}),
            &opts(true),
        )
        .unwrap();
    assert_eq!(ops(&m), vec!["profile"]);
    assert_eq!(out.plan.quote.total_micro_usd, 10_000);
    let email_steps: Vec<StepDecision> = out
        .plan
        .steps
        .iter()
        .filter(|s| matches!(s.egress.as_deref(), Some("find_email" | "verify_email")))
        .map(|s| s.will_run)
        .collect();
    assert_eq!(
        email_steps,
        vec![
            StepDecision::SkipNotRequested,
            StepDecision::SkipNotRequested
        ]
    );
    assert_eq!(out.result.as_ref().unwrap()["email"], Value::Null);
}

#[test]
fn path_without_egress_returns_the_quote_and_local_paths_only() {
    let g = graph();
    let m = mock();
    let out = g
        .run_playbook_with(
            &m,
            "path",
            &json!({"target": "Dana Whitfield", "company": "Contoso", "find_email": true}),
            &opts(false),
        )
        .unwrap();
    assert_eq!(out.status, RunStatus::Quote);
    assert!(m.calls().is_empty());
    assert_eq!(out.plan.quote.lines.len(), 3);
    let statuses: Vec<String> = g
        .conn()
        .prepare("SELECT status FROM enrich_log ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(statuses, vec!["quoted"; 3]);
    // The free local step still matches the given company.
    assert_eq!(
        vias(&out),
        vec![("Marcus Lee".to_string(), "same_company".to_string())]
    );
    assert!(out
        .plan
        .steps
        .iter()
        .filter(|s| s.egress.is_some())
        .all(|s| s.will_run == StepDecision::SkipNoEgress));
}

#[test]
fn path_with_the_null_enricher_completes_locally_and_says_why() {
    let g = graph();
    let out = g
        .run_playbook_with(
            &NullEnricher,
            "path",
            &json!({"target": "Dana Whitfield", "company": "Contoso", "find_email": true}),
            &opts(true),
        )
        .unwrap();
    assert_eq!(out.status, RunStatus::Done);
    assert_eq!(log_count(&g), 0);
    assert!(
        out.notes.iter().any(|n| n.contains("enable an enricher")),
        "{:?}",
        out.notes
    );
    assert!(out
        .plan
        .steps
        .iter()
        .filter(|s| s.egress.as_deref() == Some("profile"))
        .all(|s| s.will_run == StepDecision::SkipNoEnricher));
    assert_eq!(
        vias(&out),
        vec![("Marcus Lee".to_string(), "same_company".to_string())]
    );
}

#[test]
fn path_over_budget_stops_before_the_crossing_call() {
    let g = graph();
    let m = mock();
    let out = g
        .run_playbook_with(
            &m,
            "path",
            &json!({"target": "Dana Whitfield", "company": "Contoso", "find_email": true}),
            &RunOptions {
                max_usd: Some(0.015),
                ..opts(true)
            },
        )
        .unwrap();
    assert_eq!(out.status, RunStatus::Partial);
    assert_eq!(ops(&m), vec!["profile"]);
    assert_eq!(out.receipt.budget_micro_usd, 15_000);
    assert!(out
        .receipt
        .skipped
        .iter()
        .any(|s| s.reason == SkipReason::OverBudget));
}

#[test]
fn agents_cannot_raise_the_budget_but_the_cli_can() {
    let g = graph();
    let input = json!({"target": "Dana Whitfield", "company": "Contoso"});
    let agent = g
        .run_playbook_with(
            &mock(),
            "path",
            &input,
            &RunOptions {
                max_usd: Some(5.0),
                ..opts(false)
            },
        )
        .unwrap();
    assert_eq!(agent.plan.quote.budget_micro_usd, 500_000);
    let cli = g
        .run_playbook_with(
            &mock(),
            "path",
            &input,
            &RunOptions {
                max_usd: Some(5.0),
                surface: Surface::Cli,
                ..opts(false)
            },
        )
        .unwrap();
    assert_eq!(cli.plan.quote.budget_micro_usd, 5_000_000);
}

#[test]
fn path_target_text_can_carry_the_company() {
    let g = graph();
    let m = mock();
    g.run_playbook_with(
        &m,
        "path",
        &json!({"target": "Dana Whitfield, CTO at Contoso"}),
        &opts(true),
    )
    .unwrap();
    assert_eq!(m.calls()[0].request, dana_profile());
}

// ---------------------------------------------------------------------------
// P3-CORE-05: brief
// ---------------------------------------------------------------------------

const BRIEF_GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/brief_priya.md");

fn brief(g: &Graph, m: &dyn Enricher, input: Value, allow: bool) -> Outcome {
    g.run_playbook_with(m, "brief", &input, &opts(allow))
        .unwrap()
}

#[test]
fn brief_markdown_matches_the_golden_file_with_egress_off() {
    let g = graph();
    let out = brief(&g, &NullEnricher, json!({"person": "Priya Raman"}), false);
    assert_eq!(out.status, RunStatus::Done);
    let md = out.result.as_ref().unwrap()["markdown"]
        .as_str()
        .unwrap()
        .to_string();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(BRIEF_GOLDEN, &md).unwrap();
    }
    let want = std::fs::read_to_string(BRIEF_GOLDEN).expect("run with UPDATE_GOLDEN=1");
    assert_eq!(md, want);
    assert!(md.lines().count() <= 10);
}

#[test]
fn every_brief_fact_maps_to_evidence() {
    let g = graph();
    let m = MockEnricher::new().with_fetched_at(NOW).with_response(
        EnrichRequest::WebSearch {
            query: "\"Priya Raman\" \"Northwind\"".into(),
        },
        EnrichData::WebHits {
            hits: vec![WebHit {
                title: "Northwind raises a seed round".into(),
                url: "https://news.example/northwind-seed".into(),
                snippet: None,
                published: Some("2026-10-01".into()),
            }],
        },
    );
    let out = brief(&g, &m, json!({"person": "Priya Raman", "web": true}), true);
    assert_eq!(out.status, RunStatus::Done, "{:?}", out.notes);
    assert_eq!(ops(&m), vec!["web_search"]);
    let r = out.result.as_ref().unwrap();
    let md = r["markdown"].as_str().unwrap();
    assert!(md.lines().count() <= 10, "{md}");
    assert!(md.contains("Northwind raises a seed round"), "{md}");
    let facts = r["facts"].as_array().unwrap();
    assert!(!facts.is_empty());
    for f in facts {
        if f["section"] == "who" {
            continue;
        }
        let n = |k: &str| f[k].as_array().map_or(0, Vec::len);
        assert!(
            n("interaction_ids") + n("enrich_log_ids") > 0,
            "fact without evidence: {f}"
        );
    }
    let sections: Vec<&str> = facts
        .iter()
        .map(|f| f["section"].as_str().unwrap())
        .collect();
    let order = [
        "who",
        "how_you_know",
        "last_contact",
        "best_mutual",
        "recent_public_activity",
        "opener",
    ];
    let mut pos = 0;
    for s in &sections {
        let i = order.iter().position(|o| o == s).unwrap();
        assert!(i >= pos, "sections out of order: {sections:?}");
        pos = i;
    }
    assert_eq!(r["web"][0]["enrich_log_id"], out.receipt.enrich_log_ids[0]);
}

#[test]
fn brief_web_step_needs_egress_and_respects_no_enrich() {
    let g = graph();
    let m = MockEnricher::new();
    let out = brief(&g, &m, json!({"person": "Priya Raman", "web": true}), false);
    assert_eq!(out.status, RunStatus::Quote);
    assert!(m.calls().is_empty());
    assert!(out.result.as_ref().unwrap()["markdown"].is_string());

    g.conn()
        .execute("UPDATE person SET no_enrich = 1 WHERE id = 2", [])
        .unwrap();
    let out = brief(&g, &m, json!({"person": "Priya Raman", "web": true}), true);
    assert!(m.calls().is_empty());
    assert_eq!(out.status, RunStatus::Done);
    assert!(
        out.notes.iter().any(|n| n.contains("no_enrich")),
        "{:?}",
        out.notes
    );
}

#[test]
fn brief_of_an_unknown_person_is_not_found() {
    let g = graph();
    let err = g
        .run_playbook_with(
            &NullEnricher,
            "brief",
            &json!({"person": "Nobody Here"}),
            &opts(false),
        )
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}
