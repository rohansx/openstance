//! Reading `enrich_log` for `openstance enrich log` (P3-CLI-04): every
//! priced or sent request, exactly what it carried, and what it cost.
//! Spec: `docs/tech-spec/enrichment.md` section 6, `interfaces.md` section 8.

use rusqlite::types::Value as SqlValue;
use serde::Serialize;
use serde_json::Value;

use crate::enrich::MicroUsd;
use crate::error::Result;
use crate::json::{iso, V};
use crate::{Graph, PersonId};

/// Which rows to list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFilter {
    /// Only this run.
    pub run: Option<String>,
    /// Only rows at or after this unix time.
    pub since: Option<i64>,
    /// Only rows about this person (any identity of their cluster).
    pub person: Option<PersonId>,
}

/// One `enrich_log` row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogRow {
    /// `enrich_log.id`.
    pub id: i64,
    /// Run id.
    pub run_id: String,
    /// ISO time (quote time, then send time).
    pub at: String,
    /// Backend id.
    pub backend: String,
    /// Upstream provider, when reported.
    pub provider: Option<String>,
    /// Playbook.
    pub playbook: Option<String>,
    /// Anchor person id (null for strangers, companies and web searches).
    pub person_id: Option<i64>,
    /// Current display name of that person, if any.
    pub person_name: Option<String>,
    /// Op.
    pub op: String,
    /// Exactly the outbound fields (`{"kind": op, ...}`).
    pub request: Value,
    /// Quoted price.
    pub quoted_micro_usd: MicroUsd,
    /// Actual cost (null until the backend answered).
    pub cost_micro_usd: Option<MicroUsd>,
    /// `quoted | sent | ok | not_found | error | blocked`.
    pub status: String,
    /// Error or block reason.
    pub error: Option<String>,
}

impl LogRow {
    /// The request left the machine (`sent`, `ok`, `not_found`, `error`).
    pub fn was_sent(&self) -> bool {
        matches!(self.status.as_str(), "sent" | "ok" | "not_found" | "error")
    }

    /// A short label of who or what the request was about, from the
    /// request itself: name, then email, domain or query.
    pub fn subject(&self) -> Option<String> {
        ["name", "email", "domain", "query"]
            .iter()
            .find_map(|k| self.request.get(*k).and_then(Value::as_str))
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| self.person_name.clone())
    }
}

/// Totals of the listed rows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LogTotals {
    /// Requests that left the machine.
    pub requests_sent: u32,
    /// Sum of actual costs.
    pub spent_micro_usd: MicroUsd,
    /// Priced but never sent (egress not allowed).
    pub quoted_only: u32,
    /// Refused by the gate.
    pub blocked: u32,
}

/// `openstance enrich log --json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EnrichLogResponse {
    /// Schema version.
    pub v: u32,
    /// Rows, oldest first.
    pub rows: Vec<LogRow>,
    /// Totals.
    pub totals: LogTotals,
}

impl Graph {
    /// The egress audit log, oldest first.
    pub fn enrich_log(&self, filter: &LogFilter) -> Result<EnrichLogResponse> {
        let conn = self.conn();
        let mut sql = String::from(
            "SELECT e.id, e.run_id, e.at, e.backend, e.provider, e.playbook, e.person_id,
                    (SELECT p.display_name FROM identity i JOIN person p ON p.id = i.person_id
                     WHERE i.id = e.person_id),
                    e.op, e.request_json, e.quoted_micro_usd, e.cost_micro_usd, e.status, e.error
             FROM enrich_log e WHERE 1 = 1",
        );
        let mut args: Vec<SqlValue> = Vec::new();
        if let Some(r) = &filter.run {
            sql.push_str(" AND e.run_id = ?");
            args.push(SqlValue::Text(r.clone()));
        }
        if let Some(t) = filter.since {
            sql.push_str(" AND e.at >= ?");
            args.push(SqlValue::Integer(t));
        }
        if let Some(p) = filter.person {
            let cluster = crate::overrides::cluster(conn, p)?;
            sql.push_str(&format!(
                " AND e.person_id IN ({})",
                vec!["?"; cluster.len()].join(",")
            ));
            args.extend(cluster.into_iter().map(SqlValue::Integer));
        }
        sql.push_str(" ORDER BY e.at, e.id");
        let mut st = conn.prepare(&sql)?;
        let rows = st
            .query_map(rusqlite::params_from_iter(args), |r| {
                let req: String = r.get(9)?;
                let quoted: i64 = r.get(10)?;
                let cost: Option<i64> = r.get(11)?;
                Ok(LogRow {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    at: iso(r.get(2)?),
                    backend: r.get(3)?,
                    provider: r.get(4)?,
                    playbook: r.get(5)?,
                    person_id: r.get(6)?,
                    person_name: r.get(7)?,
                    op: r.get(8)?,
                    request: serde_json::from_str(&req).unwrap_or(Value::String(req)),
                    quoted_micro_usd: u64::try_from(quoted).unwrap_or(0),
                    cost_micro_usd: cost.map(|c| u64::try_from(c).unwrap_or(0)),
                    status: r.get(12)?,
                    error: r.get(13)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut totals = LogTotals::default();
        for row in &rows {
            if row.was_sent() {
                totals.requests_sent += 1;
                totals.spent_micro_usd += row.cost_micro_usd.unwrap_or(0);
            } else if row.status == "quoted" {
                totals.quoted_only += 1;
            } else if row.status == "blocked" {
                totals.blocked += 1;
            }
        }
        Ok(EnrichLogResponse { v: V, rows, totals })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::egress::EgressGate;
    use crate::enrich::mock::MockEnricher;
    use crate::enrich::{EnrichJob, EnrichRequest};
    use crate::search::testkit::{self, NOW};
    use crate::{FixedClock, Providers};

    fn search(q: &str, person: Option<i64>) -> EnrichJob {
        EnrichJob {
            person_id: person,
            request: EnrichRequest::WebSearch { query: q.into() },
        }
    }

    fn graph_at(now: i64) -> Graph {
        let g = Graph::open_in_memory(
            Config::default(),
            Providers::null(),
            Box::new(FixedClock(now)),
        )
        .unwrap();
        testkit::seed(g.conn());
        g
    }

    #[test]
    fn lists_rows_with_totals_and_filters() {
        let g = graph_at(NOW);
        let m = MockEnricher::new().with_price("web_search", 5_000);
        let mut gate = EgressGate::new(&g, &m, "brief", "run-a", 500_000);
        let q = gate
            .quote(vec![search("\"Priya Raman\"", Some(2)), search("b", None)])
            .unwrap();
        gate.approve(&q).unwrap();
        gate.execute(&search("\"Priya Raman\"", Some(2))).unwrap();
        let mut gate = EgressGate::new(&g, &m, "brief", "run-b", 500_000);
        gate.quote(vec![search("c", None)]).unwrap();

        let all = g.enrich_log(&LogFilter::default()).unwrap();
        assert_eq!(all.v, 1);
        assert_eq!(all.rows.len(), 3);
        assert_eq!(
            all.totals,
            LogTotals {
                requests_sent: 1,
                spent_micro_usd: 5_000,
                quoted_only: 2,
                blocked: 0
            }
        );
        let first = &all.rows[0];
        assert_eq!(first.status, "not_found");
        assert_eq!(first.person_name.as_deref(), Some("Priya Raman"));
        assert_eq!(first.subject().as_deref(), Some("\"Priya Raman\""));
        assert_eq!(first.request["kind"], "web_search");
        assert_eq!(first.at, "2026-10-20T12:00:00Z");

        let run_b = g
            .enrich_log(&LogFilter {
                run: Some("run-b".into()),
                ..LogFilter::default()
            })
            .unwrap();
        assert_eq!(run_b.rows.len(), 1);
        let priya = g
            .enrich_log(&LogFilter {
                person: Some(2),
                ..LogFilter::default()
            })
            .unwrap();
        assert_eq!(priya.rows.len(), 1);
        let later = g
            .enrich_log(&LogFilter {
                since: Some(NOW + 1),
                ..LogFilter::default()
            })
            .unwrap();
        assert!(later.rows.is_empty());
    }
}
