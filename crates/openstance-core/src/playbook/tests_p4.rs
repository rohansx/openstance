//! P4-CORE-01..05: `reconnect`, `accounts`, `refresh`, `verify`, `watch`
//! over the search test kit (`search::testkit`, NOW = 2026-10-20 12:00 UTC)
//! with the recording `MockEnricher`. The LinkedIn-history parts (alumni,
//! the export diff) run on real imports in `tests/playbooks_linkedin.rs`.

use serde_json::{json, Value};

use super::*;
use crate::config::Config;
use crate::enrich::mock::MockEnricher;
use crate::enrich::{
    EnrichData, EnrichRequest, NullEnricher, ProfileData, SkipReason, VerifyStatus,
};
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
    g
}

fn run(g: &Graph, m: &dyn Enricher, name: &str, input: Value, allow: bool) -> Outcome {
    g.run_playbook_with(
        m,
        name,
        &input,
        &RunOptions {
            allow_egress: allow,
            run_id: Some("RUN".into()),
            ..RunOptions::default()
        },
    )
    .unwrap()
}

fn run_budget(g: &Graph, m: &dyn Enricher, name: &str, input: Value, usd: f64) -> Outcome {
    g.run_playbook_with(
        m,
        name,
        &input,
        &RunOptions {
            allow_egress: true,
            max_usd: Some(usd),
            run_id: Some("RUN".into()),
            ..RunOptions::default()
        },
    )
    .unwrap()
}

fn result(out: &Outcome) -> &Value {
    out.result.as_ref().expect("result")
}

fn ids(v: &Value) -> Vec<i64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_i64().unwrap())
        .collect()
}

fn log_count(g: &Graph) -> i64 {
    g.conn()
        .query_row("SELECT count(*) FROM enrich_log", [], |r| r.get(0))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Registry and the shared rules
// ---------------------------------------------------------------------------

#[test]
fn every_phase_4_playbook_is_registered() {
    for name in ["reconnect", "accounts", "refresh", "verify", "watch"] {
        let pb = by_name(name).unwrap_or_else(|| panic!("{name} not registered"));
        assert_eq!(pb.name(), name);
    }
}

#[test]
fn plan_only_never_calls_the_enricher() {
    let g = graph();
    g.conn()
        .execute("UPDATE person SET n_interactions = 9 WHERE id = 7", [])
        .unwrap();
    let m = MockEnricher::new();
    for (name, input) in [
        ("reconnect", json!({})),
        ("accounts", json!({"companies": ["Contoso"]})),
        ("refresh", json!({})),
        ("verify", json!({"person": "Priya Raman"})),
        ("watch", json!({"check_profiles": true})),
    ] {
        let out = g
            .run_playbook_with(
                &m,
                name,
                &input,
                &RunOptions {
                    allow_egress: true,
                    plan_only: true,
                    ..RunOptions::default()
                },
            )
            .unwrap();
        assert!(out.result.is_none(), "{name}");
        assert!(m.calls().is_empty(), "{name} sent something while planning");
    }
}

#[test]
fn every_playbook_completes_with_the_null_enricher() {
    let g = graph();
    g.conn()
        .execute("UPDATE person SET n_interactions = 9 WHERE id = 7", [])
        .unwrap();
    for (name, input) in [
        ("reconnect", json!({})),
        ("accounts", json!({"companies": ["Contoso"]})),
        ("refresh", json!({})),
        ("verify", json!({"person": "Priya Raman"})),
        ("watch", json!({"check_profiles": true})),
    ] {
        let out = run(&g, &NullEnricher, name, input, true);
        assert!(out.result.is_some(), "{name}");
        assert_eq!(out.receipt.requests_sent, 0, "{name}");
        for s in out.plan.steps.iter().filter(|s| s.egress.is_some()) {
            assert_eq!(
                s.will_run,
                StepDecision::SkipNoEnricher,
                "{name} step {}",
                s.n
            );
        }
    }
}

#[test]
fn local_playbooks_make_zero_egress_calls() {
    let g = graph();
    let m = MockEnricher::new();
    for (name, input) in [
        ("reconnect", json!({})),
        ("accounts", json!({"companies": ["Contoso", "Northwind"]})),
        ("watch", json!({})),
    ] {
        let out = run(&g, &m, name, input, true);
        assert_eq!(out.status, RunStatus::Done, "{name}");
        assert!(out
            .plan
            .egress_max
            .iter()
            .all(|e| name == "watch" || e.max == 0));
    }
    assert!(m.calls().is_empty());
    assert_eq!(log_count(&g), 0);
}

// ---------------------------------------------------------------------------
// P4-CORE-01 reconnect
// ---------------------------------------------------------------------------

#[test]
fn reconnect_ranks_lapsed_people_by_past_score_with_last_subject() {
    let g = graph();
    // Marcus lapses too; the bulk sender looks lapsed but never counts.
    g.conn()
        .execute_batch(
            "UPDATE person SET score_r = 0.15 WHERE id = 3;
             UPDATE person SET score_f = 0.9, score_r = 0.05 WHERE id = 4;",
        )
        .unwrap();
    let out = run(&g, &NullEnricher, "reconnect", json!({}), false);
    assert_eq!(out.status, RunStatus::Done);
    let people = &result(&out)["people"];
    assert_eq!(
        ids(people),
        [5, 3],
        "Sam (past 0.705) before Marcus (0.5275)"
    );
    let sam = &people[0];
    assert_eq!(sam["display_name"], "Sam Ito");
    assert_eq!(sam["past_score"], 0.705);
    assert_eq!(sam["score"], 0.35);
    assert_eq!(sam["last_subject"], "Q3 hiring plan");
    assert_eq!(sam["last_interaction_id"], 105);
    assert_eq!(sam["last_contact"], crate::json::iso(ago(400)));
    // Marcus's last *past* interaction (the future offsite is not his).
    assert_eq!(people[1]["last_subject"], "devtools dinner");
    assert_eq!(out.receipt.requests_sent, 0);
    assert!(out.plan.quote.lines.is_empty());
}

#[test]
fn reconnect_honours_limit_and_thresholds() {
    let g = graph();
    g.conn()
        .execute("UPDATE person SET score_r = 0.15 WHERE id = 3", [])
        .unwrap();
    let out = run(&g, &NullEnricher, "reconnect", json!({"limit": 1}), false);
    assert_eq!(ids(&result(&out)["people"]), [5]);
    let out = run(
        &g,
        &NullEnricher,
        "reconnect",
        json!({"min_frequency": 0.7}),
        false,
    );
    assert_eq!(
        ids(&result(&out)["people"]),
        [5],
        "Marcus F 0.6 is below 0.7"
    );
    let out = run(
        &g,
        &NullEnricher,
        "reconnect",
        json!({"max_recency": 0.12}),
        false,
    );
    assert_eq!(
        ids(&result(&out)["people"]),
        [5],
        "Marcus R 0.15 is above 0.12"
    );
}

// ---------------------------------------------------------------------------
// P4-CORE-02 accounts
// ---------------------------------------------------------------------------

#[test]
fn accounts_coverage_table_with_current_domain_and_one_hop() {
    let g = graph();
    // Sam shares two threads with Marcus: one hop into Contoso.
    g.conn()
        .execute(
            &format!(
                "INSERT INTO tie(a, b, kind, weight, last_at, n_shared)
                 VALUES (3, 5, 'email', 0.5, {}, 2), (2, 3, 'email', 0.9, {}, 1)",
                ago(30),
                ago(30)
            ),
            [],
        )
        .unwrap();
    let out = run(
        &g,
        &NullEnricher,
        "accounts",
        json!({"companies": ["Contoso", "northwind.example", "Nowhere Inc"]}),
        false,
    );
    assert_eq!(out.status, RunStatus::Done);
    let rows = result(&out)["companies"].as_array().unwrap();
    assert_eq!(rows.len(), 3);

    let contoso = &rows[0];
    assert_eq!(contoso["company"], "Contoso");
    // Marcus by company; Ravi has no company but a contoso.example address.
    assert_eq!(ids(&contoso["current"]), [3, 7]);
    assert_eq!(contoso["current"][0]["score"], 0.5);
    assert_eq!(contoso["alumni"], json!([]));
    // Priya's tie to Marcus has n_shared 1, so only Sam is one hop away.
    let hop = &contoso["one_hop"];
    assert_eq!(ids(hop), [5]);
    assert_eq!(hop[0]["via"], 3);
    assert_eq!(hop[0]["strength"], 0.175);
    assert_eq!(contoso["coverage"], "known", "best current score 0.5 < 0.6");
    assert_eq!(contoso["best"], 3);

    let nw = &rows[1];
    assert_eq!(
        nw["company"], "Northwind",
        "display name from the people found"
    );
    assert_eq!(nw["domain"], "northwind.example");
    assert_eq!(ids(&nw["current"]), [2]);
    assert_eq!(nw["coverage"], "warm");
    assert_eq!(nw["best"], 2);

    let none = &rows[2];
    assert_eq!(none["company"], "Nowhere Inc");
    assert_eq!(none["coverage"], "none");
    assert_eq!(none["best"], Value::Null);
}

#[test]
fn accounts_counts_an_old_work_address_as_alumni_and_skips_one_hop_on_request() {
    let g = graph();
    let out = run(
        &g,
        &NullEnricher,
        "accounts",
        json!({"companies": ["stripe.example"], "one_hop": false}),
        false,
    );
    let row = &result(&out)["companies"][0];
    // Sam works at Fabrikam now but has sam@stripe.example.
    assert_eq!(ids(&row["current"]), Vec::<i64>::new());
    assert_eq!(ids(&row["alumni"]), [5]);
    assert_eq!(row["alumni"][0]["seen_in"], "email_domain");
    assert_eq!(row["coverage"], "known");
    assert_eq!(row["best"], 5);
}

#[test]
fn accounts_needs_at_least_one_company() {
    let g = graph();
    let err = g
        .run_playbook_with(
            &NullEnricher,
            "accounts",
            &json!({"companies": []}),
            &RunOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(err, Error::BadRequest(_)), "{err}");
}

// ---------------------------------------------------------------------------
// P4-CORE-04 refresh
// ---------------------------------------------------------------------------

/// Eight people with no title, 6 interactions each, scores 0.30 .. 0.23.
fn stale_people(g: &Graph) {
    for i in 0..8 {
        let id = 20 + i;
        g.conn()
            .execute(
                "INSERT INTO person(id, display_name, company, n_interactions, score)
                 VALUES (?1, ?2, 'Contoso', 6, ?3)",
                rusqlite::params![id, format!("Stale Person {i}"), 0.30 - f64::from(i) * 0.01],
            )
            .unwrap();
        g.conn()
            .execute(
                "INSERT INTO identity(id, kind, value, source_id, person_id, confidence, match_rule)
                 VALUES (?1, 'email', ?2, 1, ?1, 1.0, 'anchor')",
                rusqlite::params![id, format!("stale{i}@contoso.example")],
            )
            .unwrap();
    }
}

fn stale_profile(i: i64) -> EnrichRequest {
    EnrichRequest::Profile {
        name: format!("Stale Person {i}"),
        company: Some("Contoso".into()),
        linkedin: None,
    }
}

#[test]
fn refresh_with_a_5_cent_budget_enriches_exactly_5_in_score_order() {
    let g = graph();
    stale_people(&g);
    let m = MockEnricher::new().with_fetched_at(NOW).with_response(
        stale_profile(0),
        EnrichData::Profile(ProfileData {
            title: Some("Head of Data".into()),
            company: Some("Contoso".into()),
            ..ProfileData::default()
        }),
    );
    let out = run_budget(&g, &m, "refresh", json!({}), 0.05);
    assert_eq!(m.calls().len(), 5);
    let sent: Vec<EnrichRequest> = m.calls().into_iter().map(|j| j.request).collect();
    assert_eq!(sent, (0..5).map(stale_profile).collect::<Vec<_>>());
    assert_eq!(out.receipt.requests_sent, 5);
    assert_eq!(out.receipt.spent_micro_usd, 50_000);
    assert_eq!(out.status, RunStatus::Partial, "3 were over budget");
    assert_eq!(
        out.receipt
            .skipped
            .iter()
            .filter(|s| s.reason == SkipReason::OverBudget)
            .count(),
        3
    );
    let r = result(&out);
    assert_eq!(r["updated"][0]["id"], 20);
    assert_eq!(r["updated"][0]["fields"]["title"], "Head of Data");
    assert_eq!(r["not_found"], json!([21, 22, 23, 24]));
    // The commit itself (fields under an `enrich:mock` source) is checked on
    // a real import in tests/playbooks_commit.rs.
}

#[test]
fn refresh_skips_no_enrich_overridden_and_thin_people_and_quotes_without_egress() {
    let g = graph();
    stale_people(&g);
    g.conn()
        .execute_batch(
            "UPDATE person SET no_enrich = 1 WHERE id = 20;
             UPDATE person SET n_interactions = 2 WHERE id = 21;
             INSERT INTO override(person_id, field, value, set_at) VALUES (22, 'title', '', 5);",
        )
        .unwrap();
    let overrides_before: i64 = g
        .conn()
        .query_row("SELECT count(*) FROM override", [], |r| r.get(0))
        .unwrap();
    let m = MockEnricher::new();
    let out = run(&g, &m, "refresh", json!({"limit": 3}), false);
    assert_eq!(out.status, RunStatus::Quote);
    assert!(m.calls().is_empty());
    let people: Vec<Option<i64>> = out
        .plan
        .quote
        .lines
        .iter()
        .map(|l| l.job.person_id)
        .collect();
    assert_eq!(people, [Some(23), Some(24), Some(25)]);
    assert!(
        out.notes.iter().any(|n| n.contains("no_enrich")),
        "{:?}",
        out.notes
    );
    let after: i64 = g
        .conn()
        .query_row("SELECT count(*) FROM override", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, overrides_before);
}

// ---------------------------------------------------------------------------
// P4-CORE-05 verify
// ---------------------------------------------------------------------------

fn verify_mock(status: VerifyStatus) -> MockEnricher {
    MockEnricher::new().with_fetched_at(NOW).with_response(
        EnrichRequest::VerifyEmail {
            email: "priya@northwind.example".into(),
        },
        EnrichData::Verification {
            email: "priya@northwind.example".into(),
            status,
        },
    )
}

fn with_replying_mutual(g: &Graph) {
    // Ravi (C 0.5) shares a thread with Priya; Marcus (C 0) never replied.
    g.conn()
        .execute(
            &format!(
                "INSERT INTO tie(a, b, kind, weight, last_at, n_shared)
                 VALUES (2, 3, 'email', 0.9, {}, 3), (2, 7, 'email', 0.4, {}, 1)",
                ago(1),
                ago(19)
            ),
            [],
        )
        .unwrap();
}

#[test]
fn verify_outcomes_deliverable_invalid_and_catch_all() {
    for (status, want) in [
        (VerifyStatus::Deliverable, "deliverable"),
        (VerifyStatus::Undeliverable, "undeliverable"),
        (VerifyStatus::Risky, "risky"),
    ] {
        let g = graph();
        with_replying_mutual(&g);
        let m = verify_mock(status);
        let out = run(&g, &m, "verify", json!({"person": "Priya Raman"}), true);
        assert_eq!(out.status, RunStatus::Done, "{want}");
        assert_eq!(m.calls().len(), 1);
        let r = result(&out);
        assert_eq!(r["email"], "priya@northwind.example");
        assert_eq!(r["status"], want);
        assert_eq!(r["verified_at"], crate::json::iso(NOW));
        assert_eq!(r["mutual"]["id"], 7);
        assert_eq!(r["mutual"]["display_name"], "Ravi Iyer");
        assert_eq!(r["mutual"]["last_reply"], crate::json::iso(ago(19)));
        assert_eq!(r["mutual"]["last_subject"], "Re: devtools dinner");
        assert_eq!(out.receipt.requests_sent, 1);
    }
}

#[test]
fn verify_without_egress_returns_the_candidate_unverified() {
    let g = graph();
    let m = verify_mock(VerifyStatus::Deliverable);
    let out = run(&g, &m, "verify", json!({"person": "Sam Ito"}), false);
    assert_eq!(out.status, RunStatus::Quote);
    assert!(m.calls().is_empty());
    let r = result(&out);
    // 1.0 beats the 0.98 alias.
    assert_eq!(r["email"], "sam@stripe.example");
    assert_eq!(r["status"], "unverified");
    assert_eq!(r["mutual"], Value::Null);
    assert_eq!(out.plan.quote.lines.len(), 1);
}

#[test]
fn verify_respects_no_enrich_and_needs_an_email() {
    let g = graph();
    g.conn()
        .execute("UPDATE person SET no_enrich = 1 WHERE id = 2", [])
        .unwrap();
    let m = verify_mock(VerifyStatus::Deliverable);
    let out = run(&g, &m, "verify", json!({"person": "Priya Raman"}), true);
    assert!(m.calls().is_empty());
    assert_eq!(result(&out)["status"], "unverified");
    assert_eq!(out.plan.steps[1].will_run, StepDecision::SkipPrivacy);

    let out = run(&g, &m, "verify", json!({"person": "Lena Park"}), true);
    assert_eq!(out.status, RunStatus::NothingToDo);
    assert!(out
        .notes
        .iter()
        .any(|n| n == "no email in graph; try path --find-email"));
}

// ---------------------------------------------------------------------------
// P4-CORE-03 watch (profile checks; the LinkedIn diff is an integration test)
// ---------------------------------------------------------------------------

fn priya_profile() -> EnrichRequest {
    EnrichRequest::Profile {
        name: "Priya Raman".into(),
        company: Some("Northwind".into()),
        linkedin: Some("priya-raman-nw".into()),
    }
}

#[test]
fn watch_profile_checks_respect_no_enrich_budget_and_flag_target_moves() {
    let g = graph();
    g.conn()
        .execute("UPDATE person SET no_enrich = 1 WHERE id = 3", [])
        .unwrap();
    let m = MockEnricher::new().with_fetched_at(NOW).with_response(
        priya_profile(),
        EnrichData::Profile(ProfileData {
            title: Some("CEO".into()),
            company: Some("Anthropic".into()),
            ..ProfileData::default()
        }),
    );
    let out = run_budget(
        &g,
        &m,
        "watch",
        json!({"check_profiles": true, "top_n": 3, "targets": ["anthropic.com"]}),
        0.01,
    );
    // Top 3 by score without Marcus (no_enrich): Priya, Ravi, Sam. Only one
    // fits in $0.01.
    let asked: Vec<Option<i64>> = out
        .plan
        .quote
        .lines
        .iter()
        .map(|l| l.job.person_id)
        .collect();
    assert_eq!(asked, [Some(2), Some(7), Some(5)]);
    assert_eq!(m.calls().len(), 1);
    assert_eq!(m.calls()[0].request, priya_profile());
    assert_eq!(out.status, RunStatus::Partial);
    let r = result(&out);
    assert_eq!(r["checked"], 1);
    let mv = &r["moves"][0];
    assert_eq!(mv["id"], 2);
    assert_eq!(mv["from"], "Northwind");
    assert_eq!(mv["to"], "Anthropic");
    assert_eq!(mv["detected_by"], "enrich:mock");
    assert_eq!(mv["target_hit"], true);
    assert!(mv["enrich_log_id"].is_i64());
}

#[test]
fn watch_without_egress_quotes_the_checks_and_sends_nothing() {
    let g = graph();
    let m = MockEnricher::new();
    let out = run(
        &g,
        &m,
        "watch",
        json!({"check_profiles": true, "top_n": 2}),
        false,
    );
    assert_eq!(out.status, RunStatus::Quote);
    assert!(m.calls().is_empty());
    assert_eq!(out.plan.quote.lines.len(), 2);
    assert_eq!(result(&out)["checked"], 0);
    assert!(
        out.notes.iter().any(|n| n.contains("LinkedIn")),
        "{:?}",
        out.notes
    );
}
