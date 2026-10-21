//! P3-CORE-01 policy tests with the recording `MockEnricher`.

use super::*;
use crate::config::Config;
use crate::enrich::mock::{MockEnricher, MockEvent};
use crate::enrich::{EnrichRequest, NullEnricher};
use crate::search::testkit::{self, NOW};
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

fn profile(person: Option<PersonId>, name: &str) -> EnrichJob {
    EnrichJob {
        person_id: person,
        request: EnrichRequest::Profile {
            name: name.into(),
            company: None,
            linkedin: None,
        },
    }
}

fn search(q: &str) -> EnrichJob {
    EnrichJob {
        person_id: None,
        request: EnrichRequest::WebSearch { query: q.into() },
    }
}

fn rows(g: &Graph) -> Vec<(String, String, Option<i64>)> {
    g.conn()
        .prepare("SELECT op, status, cost_micro_usd FROM enrich_log ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn set_no_enrich(g: &Graph, person: PersonId) {
    g.conn()
        .execute(
            "INSERT INTO override(person_id, field, value, set_at) VALUES (?1, 'no_enrich', '1', 1)",
            [person],
        )
        .unwrap();
    g.conn()
        .execute("UPDATE person SET no_enrich = 1 WHERE id = ?1", [person])
        .unwrap();
}

#[test]
fn without_approval_nothing_is_sent_and_one_quoted_row_is_logged() {
    let g = graph();
    let mock = MockEnricher::new();
    let mut gate = EgressGate::new(&g, &mock, "path", "run-1", 500_000);
    let q = gate.quote(vec![profile(Some(2), "Priya Raman")]).unwrap();
    assert_eq!(q.total_micro_usd, 10_000);
    assert!(q.within_budget);
    assert!(mock.calls().is_empty());
    assert_eq!(rows(&g), vec![("profile".into(), "quoted".into(), None)]);
    // Executing without approval is refused and still sends nothing.
    assert!(gate.execute(&profile(Some(2), "Priya Raman")).is_err());
    assert!(mock.calls().is_empty());
    let r = gate.receipt();
    assert_eq!((r.requests_sent, r.spent_micro_usd), (0, 0));
    assert_eq!(r.enrich_log_ids.len(), 1);
}

#[test]
fn quote_comes_before_the_first_call() {
    let g = graph();
    let mock = MockEnricher::new();
    let mut gate = EgressGate::new(&g, &mock, "brief", "run-q", 500_000);
    let q = gate.quote(vec![search("a")]).unwrap();
    gate.approve(&q).unwrap();
    gate.execute(&search("a")).unwrap();
    let ev = mock.events();
    assert!(matches!(ev[0], MockEvent::Quote(_)));
    assert!(matches!(ev[1], MockEvent::Enrich(_)));
}

#[test]
fn budget_aborts_before_the_call_that_would_cross_it() {
    let g = graph();
    let mock = MockEnricher::new();
    let mut gate = EgressGate::new(&g, &mock, "refresh", "run-b", 50_000);
    let jobs: Vec<EnrichJob> = (0..10).map(|i| search(&format!("q{i}"))).collect();
    let q = gate.quote(jobs.clone()).unwrap();
    assert_eq!(q.total_micro_usd, 100_000);
    assert!(!q.within_budget);
    gate.approve(&q).unwrap();
    let mut sent = 0;
    let mut blocked = 0;
    for j in &jobs {
        match gate.execute(j).unwrap() {
            Executed::Done { .. } => sent += 1,
            Executed::Blocked {
                reason: SkipReason::OverBudget,
                ..
            } => blocked += 1,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!((sent, blocked), (5, 5));
    assert_eq!(mock.calls().len(), 5);
    let statuses: Vec<String> = rows(&g).into_iter().map(|r| r.1).collect();
    assert_eq!(statuses.iter().filter(|s| *s == "not_found").count(), 5);
    assert_eq!(statuses.iter().filter(|s| *s == "blocked").count(), 5);
    let r = gate.receipt();
    assert_eq!(r.spent_micro_usd, 50_000);
    assert_eq!(r.requests_sent, 5);
    assert_eq!(
        r.skipped
            .iter()
            .filter(|s| s.reason == SkipReason::OverBudget)
            .count(),
        5
    );
}

#[test]
fn no_enrich_people_are_never_sent() {
    let g = graph();
    set_no_enrich(&g, 3);
    let mock = MockEnricher::new();
    let mut gate = EgressGate::new(&g, &mock, "refresh", "run-n", 500_000);
    let q = gate
        .quote(vec![
            profile(Some(2), "Priya Raman"),
            profile(Some(3), "Marcus Lee"),
        ])
        .unwrap();
    assert_eq!(q.lines.len(), 1);
    assert_eq!(q.skipped.len(), 1);
    assert_eq!(q.skipped[0].reason, SkipReason::NoEnrich);
    assert_eq!(q.skipped[0].person_id, Some(3));
    // The backend is not even asked to price the no_enrich person.
    for e in mock.events() {
        let jobs = match e {
            MockEvent::Quote(j) => j,
            MockEvent::Enrich(j) => vec![j],
        };
        assert!(jobs.iter().all(|j| j.person_id != Some(3)));
    }
    gate.approve(&q).unwrap();
    // A job for the skipped person is not in the quote.
    assert!(gate.execute(&profile(Some(3), "Marcus Lee")).is_err());
    gate.execute(&profile(Some(2), "Priya Raman")).unwrap();
    assert!(mock.calls().iter().all(|j| j.person_id != Some(3)));
}

#[test]
fn no_enrich_set_after_the_quote_blocks_the_send() {
    let g = graph();
    let mock = MockEnricher::new();
    let mut gate = EgressGate::new(&g, &mock, "refresh", "run-late", 500_000);
    let q = gate.quote(vec![profile(Some(2), "Priya Raman")]).unwrap();
    gate.approve(&q).unwrap();
    set_no_enrich(&g, 2);
    let out = gate.execute(&profile(Some(2), "Priya Raman")).unwrap();
    assert!(matches!(
        out,
        Executed::Blocked {
            reason: SkipReason::NoEnrich,
            ..
        }
    ));
    assert!(mock.calls().is_empty());
    assert_eq!(rows(&g)[0].1, "blocked");
}

#[test]
fn do_not_share_is_respected_by_default() {
    let g = graph();
    g.conn()
        .execute("UPDATE person SET do_not_share = 1 WHERE id = 2", [])
        .unwrap();
    let mock = MockEnricher::new();
    let mut gate = EgressGate::new(&g, &mock, "refresh", "run-d", 500_000);
    let q = gate.quote(vec![profile(Some(2), "Priya Raman")]).unwrap();
    assert!(q.lines.is_empty());
    assert_eq!(q.skipped[0].reason, SkipReason::DoNotShare);
}

#[test]
fn the_log_row_is_written_even_when_the_call_errors() {
    let g = graph();
    let mock = MockEnricher::new().fail_on(EnrichRequest::WebSearch { query: "x".into() });
    let mut gate = EgressGate::new(&g, &mock, "brief", "run-e", 500_000);
    let q = gate.quote(vec![search("x"), search("y")]).unwrap();
    gate.approve(&q).unwrap();
    let out = gate.execute(&search("x")).unwrap();
    assert!(matches!(out, Executed::Failed { fatal: false, .. }));
    // A non-fatal error does not stop the run.
    assert!(matches!(
        gate.execute(&search("y")).unwrap(),
        Executed::Done { .. }
    ));
    let r = rows(&g);
    assert_eq!(r[0].1, "error");
    assert_eq!(r[1].1, "not_found");
    let err: Option<String> = g
        .conn()
        .query_row(
            "SELECT error FROM enrich_log WHERE id = ?1",
            [gate.receipt().enrich_log_ids[0]],
            |r| r.get(0),
        )
        .unwrap();
    assert!(err.unwrap().contains("upstream"));
}

#[test]
fn auth_errors_stop_the_run() {
    let g = graph();
    let mock = MockEnricher::new()
        .fail_with(EnrichRequest::WebSearch { query: "x".into() }, || {
            EnrichError::Auth("bad token".into())
        });
    let mut gate = EgressGate::new(&g, &mock, "brief", "run-a", 500_000);
    let q = gate.quote(vec![search("x"), search("y")]).unwrap();
    gate.approve(&q).unwrap();
    assert!(matches!(
        gate.execute(&search("x")).unwrap(),
        Executed::Failed { fatal: true, .. }
    ));
    assert!(matches!(
        gate.execute(&search("y")).unwrap(),
        Executed::Failed { fatal: true, .. }
    ));
    assert_eq!(mock.calls().len(), 1);
    assert_eq!(rows(&g)[1].1, "blocked");
}

#[test]
fn the_sent_row_records_exactly_what_left_and_the_actual_cost() {
    let g = graph();
    let mock = MockEnricher::new().with_price("find_email", 7_000);
    let mut gate = EgressGate::new(&g, &mock, "path", "run-f", 500_000);
    let planned = EnrichJob {
        person_id: None,
        request: EnrichRequest::FindEmail {
            name: "Dana Whitfield".into(),
            domain: String::new(),
        },
    };
    let q = gate.quote(vec![planned]).unwrap();
    gate.approve(&q).unwrap();
    let actual = EnrichJob {
        person_id: None,
        request: EnrichRequest::FindEmail {
            name: "Dana Whitfield".into(),
            domain: "lumenpay.example".into(),
        },
    };
    gate.execute(&actual).unwrap();
    let (req, status, cost): (String, String, i64) = g
        .conn()
        .query_row(
            "SELECT request_json, status, cost_micro_usd FROM enrich_log",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&req).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"kind": "find_email", "name": "Dana Whitfield", "domain": "lumenpay.example"})
    );
    assert_eq!((status.as_str(), cost), ("not_found", 7_000));
}

#[test]
fn null_backend_quotes_nothing_and_skips_everything() {
    let g = graph();
    let mut gate = EgressGate::new(&g, &NullEnricher, "path", "run-0", 500_000);
    let q = gate.quote(vec![profile(None, "Dana")]).unwrap();
    assert!(q.lines.is_empty());
    assert_eq!(q.skipped[0].reason, SkipReason::BackendNone);
    assert!(rows(&g).is_empty());
}

#[test]
fn approval_is_for_one_quote() {
    let g = graph();
    let mock = MockEnricher::new();
    let mut gate = EgressGate::new(&g, &mock, "brief", "run-x", 500_000);
    let q1 = gate.quote(vec![search("a")]).unwrap();
    let _q2 = gate.quote(vec![search("b")]).unwrap();
    assert!(gate.approve(&q1).is_err());
    assert!(gate.execute(&search("b")).is_err());
}

#[test]
fn budget_rules_per_surface() {
    let cfg = crate::config::EnrichSection::default();
    assert_eq!(effective_budget(&cfg, None, Surface::Agent), 500_000);
    assert_eq!(effective_budget(&cfg, Some(5.0), Surface::Agent), 500_000);
    assert_eq!(effective_budget(&cfg, Some(0.1), Surface::Agent), 100_000);
    assert_eq!(effective_budget(&cfg, Some(0.0), Surface::Agent), 0);
    assert_eq!(effective_budget(&cfg, Some(2.0), Surface::Cli), 2_000_000);
    assert_eq!(effective_budget(&cfg, None, Surface::Cli), 500_000);
}

#[test]
fn run_ids_are_ulid_shaped_and_unique() {
    let a = new_run_id(NOW);
    let b = new_run_id(NOW);
    assert_eq!(a.len(), 26);
    assert_ne!(a, b);
    assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    assert!(new_run_id(NOW + 10) > a);
}

#[test]
fn requoting_the_same_jobs_in_one_run_reuses_the_quoted_rows() {
    // The CLI shows the quote, asks, then runs again with the same run id:
    // the audit log keeps one row per job, not one per look at the price.
    let g = graph();
    let mock = MockEnricher::new();
    let jobs = vec![search("a"), search("b")];
    let mut first = EgressGate::new(&g, &mock, "brief", "run-r", 500_000);
    first.quote(jobs.clone()).unwrap();
    let mut second = EgressGate::new(&g, &mock, "brief", "run-r", 500_000);
    let q = second.quote(jobs.clone()).unwrap();
    assert_eq!(rows(&g).len(), 2);
    second.approve(&q).unwrap();
    second.execute(&search("a")).unwrap();
    second.execute(&search("b")).unwrap();
    let statuses: Vec<String> = rows(&g).into_iter().map(|r| r.1).collect();
    assert_eq!(statuses, vec!["not_found", "not_found"]);
    // Another run never reuses them.
    let mut other = EgressGate::new(&g, &mock, "brief", "run-s", 500_000);
    other.quote(jobs).unwrap();
    assert_eq!(rows(&g).len(), 4);
}
