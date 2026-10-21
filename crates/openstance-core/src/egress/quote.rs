//! Price enrich requests without running a playbook (P3-MCP-03
//! `quote_enrich`, CLI `enrich quote`). Goes through [`EgressGate::quote`],
//! so `no_enrich` / `do_not_share` people are dropped before the backend
//! prices anything, the budget rule of the surface applies, and every priced
//! job gets an `enrich_log` row with status `quoted` (playbook
//! `quote_enrich`). Nothing is ever sent: there is no approve step.

use serde_json::Value;

use super::{effective_budget, new_run_id, EgressGate, Surface};
use crate::enrich::{EnrichJob, Enricher, Quote};
use crate::error::Result;
use crate::json::{iso, V};
use crate::Graph;

/// The `enrich_log.playbook` value of these rows.
pub const QUOTE_PLAYBOOK: &str = "quote_enrich";

impl Graph {
    /// Price `jobs` with the configured enricher. Never sends.
    pub fn quote_enrich(
        &self,
        jobs: Vec<EnrichJob>,
        max_usd: Option<f64>,
        surface: Surface,
    ) -> Result<Quote> {
        self.quote_enrich_with(self.providers.enricher.as_ref(), jobs, max_usd, surface)
    }

    /// [`Graph::quote_enrich`] with an explicit enricher.
    pub fn quote_enrich_with(
        &self,
        enricher: &dyn Enricher,
        jobs: Vec<EnrichJob>,
        max_usd: Option<f64>,
        surface: Surface,
    ) -> Result<Quote> {
        let budget = effective_budget(&self.cfg.enrich, max_usd, surface);
        let run_id = new_run_id(self.now());
        let mut gate = EgressGate::new(self, enricher, QUOTE_PLAYBOOK, run_id, budget);
        gate.quote(jobs)
    }
}

/// The `Quote` JSON of interfaces.md 1.9 as a top-level document:
/// `{ "v": 1, ...quote }` with `quoted_at` in ISO 8601.
pub fn quote_json(q: &Quote) -> Result<Value> {
    let mut v = serde_json::to_value(q)?;
    if let Some(m) = v.as_object_mut() {
        m.insert("quoted_at".into(), Value::String(iso(q.quoted_at)));
        m.insert("v".into(), Value::from(V));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::enrich::mock::MockEnricher;
    use crate::enrich::{EnrichRequest, NullEnricher, SkipReason};
    use crate::search::testkit::{self, NOW};
    use crate::{FixedClock, Providers};

    fn graph(cfg: Config) -> Graph {
        let g = Graph::open_in_memory(cfg, Providers::null(), Box::new(FixedClock(NOW))).unwrap();
        testkit::seed(g.conn());
        g
    }

    fn verify(person: Option<i64>, email: &str) -> EnrichJob {
        EnrichJob {
            person_id: person,
            request: EnrichRequest::VerifyEmail {
                email: email.into(),
            },
        }
    }

    fn rows(g: &Graph) -> Vec<(String, String, String)> {
        g.conn()
            .prepare("SELECT op, status, playbook FROM enrich_log ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn prices_without_sending_and_logs_quoted_rows() {
        let g = graph(Config::default());
        let m = MockEnricher::new();
        let q = g
            .quote_enrich_with(
                &m,
                vec![verify(None, "a@x.example"), verify(Some(2), "p@x.example")],
                None,
                Surface::Agent,
            )
            .unwrap();
        assert_eq!(q.lines.len(), 2);
        assert_eq!(q.total_micro_usd, 20_000);
        assert_eq!(q.budget_micro_usd, 500_000);
        assert!(q.within_budget);
        assert!(m.calls().is_empty());
        assert_eq!(
            rows(&g),
            vec![
                (
                    "verify_email".into(),
                    "quoted".into(),
                    QUOTE_PLAYBOOK.into()
                ),
                (
                    "verify_email".into(),
                    "quoted".into(),
                    QUOTE_PLAYBOOK.into()
                ),
            ]
        );
    }

    #[test]
    fn agents_can_only_lower_the_budget() {
        let g = graph(Config::default());
        let m = MockEnricher::new();
        let jobs = || vec![verify(None, "a@x.example")];
        let hi = g
            .quote_enrich_with(&m, jobs(), Some(5.0), Surface::Agent)
            .unwrap();
        assert_eq!(hi.budget_micro_usd, 500_000);
        let lo = g
            .quote_enrich_with(&m, jobs(), Some(0.005), Surface::Agent)
            .unwrap();
        assert_eq!(lo.budget_micro_usd, 5_000);
        assert!(!lo.within_budget);
    }

    #[test]
    fn no_enrich_people_are_skipped_before_pricing() {
        let g = graph(Config::default());
        g.conn()
            .execute("UPDATE person SET no_enrich = 1 WHERE id = 3", [])
            .unwrap();
        let m = MockEnricher::new();
        let q = g
            .quote_enrich_with(
                &m,
                vec![verify(Some(3), "m@x.example")],
                None,
                Surface::Agent,
            )
            .unwrap();
        assert!(q.lines.is_empty());
        assert_eq!(q.skipped[0].reason, SkipReason::NoEnrich);
        assert!(m.quotes().iter().all(Vec::is_empty) || m.quotes().is_empty());
        assert!(rows(&g).is_empty());
    }

    #[test]
    fn backend_none_quotes_zero_and_the_json_is_versioned() {
        let g = graph(Config::default());
        let q = g
            .quote_enrich_with(
                &NullEnricher,
                vec![verify(None, "a@x.example")],
                None,
                Surface::Cli,
            )
            .unwrap();
        assert_eq!(q.total_micro_usd, 0);
        assert_eq!(q.skipped[0].reason, SkipReason::BackendNone);
        let v = quote_json(&q).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["backend"], "none");
        assert_eq!(v["quoted_at"], "2026-10-20T12:00:00Z");
    }
}
