//! `TregEnricher`: one token, treg's routed people / company / search tools,
//! over blocking HTTP (P3-ENR-04).
//!
//! Wire facts (treg 0.27): auth is the `X-Treg-Token` header; prices come from
//! the open `GET /catalog/endpoints/<id>` (no token, no contact data); calls
//! are `POST /call/<id>` with a JSON body and an `X-Treg-Route-Max-Cost`
//! ceiling equal to the quoted price; the actual cost is the
//! `X-Treg-Cost-Micro` header; a miss is a 200 with `_treg.outcome = "miss"`.
//! See `docs/tech-spec/enrichment.md` section 3.2.
//!
//! The token is never logged, never put in an error, and never shown by
//! `Debug`. Request bodies (which carry contact data) are never logged.

pub mod map;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use crate::{
    EnrichData, EnrichError, EnrichJob, EnrichResult, Enricher, MicroUsd, QuoteBasis, QuoteLine,
};
use map::CallPlan;

/// The hosted treg router.
pub const DEFAULT_ENDPOINT: &str = "https://treg.to";
/// Environment variable that holds the token by default.
pub const DEFAULT_TOKEN_ENV: &str = "OPENSTANCE_TREG_TOKEN";
/// Default per-request timeout (`enrich.timeout_secs`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// See the module docs.
pub struct TregEnricher {
    endpoint: String,
    token: Option<String>,
    token_env: String,
    agent: ureq::Agent,
    prices: Mutex<HashMap<&'static str, (MicroUsd, QuoteBasis)>>,
}

impl std::fmt::Debug for TregEnricher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TregEnricher")
            .field("endpoint", &self.endpoint)
            .field("token", &self.token.as_ref().map(|_| "***"))
            .field("token_env", &self.token_env)
            .finish()
    }
}

impl TregEnricher {
    /// A client for `endpoint` (e.g. [`DEFAULT_ENDPOINT`] or a self-hosted
    /// router such as `http://127.0.0.1:18790`). With no token, quoting works
    /// and `enrich` fails with [`EnrichError::Auth`] before any call.
    pub fn new(endpoint: &str, token: Option<String>, timeout: Duration) -> Self {
        Self::build(endpoint, token, DEFAULT_TOKEN_ENV, timeout)
    }

    /// Like [`TregEnricher::new`], reading the token from the environment
    /// variable `token_env` (`enrich.treg.token_env`).
    pub fn from_env(endpoint: &str, token_env: &str, timeout: Duration) -> Self {
        let token = std::env::var(token_env)
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        Self::build(endpoint, token, token_env, timeout)
    }

    fn build(endpoint: &str, token: Option<String>, token_env: &str, timeout: Duration) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .user_agent(concat!("openstance/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        TregEnricher {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            token,
            token_env: token_env.to_string(),
            agent,
            prices: Mutex::new(HashMap::new()),
        }
    }

    /// Price of one routed tool: the upper bound of its routing plan (basis
    /// `estimate` when the plan has several prices), else the endpoint's own
    /// cost. Cached for the life of the process.
    fn price(&self, tool: &'static str) -> Result<(MicroUsd, QuoteBasis), EnrichError> {
        if let Some(p) = self.lock_prices().get(tool) {
            return Ok(*p);
        }
        let url = format!("{}/catalog/endpoints/{tool}", self.endpoint);
        let mut resp = self.agent.get(&url).call().map_err(transport_error)?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().map_err(transport_error)?;
        tracing::debug!(tool, status, "treg catalog price");
        match status {
            200 => {}
            404 => {
                return Err(EnrichError::Unsupported(format!(
                    "{tool} is not in the treg catalog"
                )))
            }
            _ => {
                return Err(EnrichError::Upstream(format!(
                    "treg catalog HTTP {status}: {}",
                    detail(&body)
                )))
            }
        }
        let v: Value = serde_json::from_str(&body)
            .map_err(|_| EnrichError::Upstream(format!("treg catalog for {tool} is not JSON")))?;
        let plan: Vec<f64> = v
            .pointer("/routing/plan")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.get("usd").and_then(Value::as_f64))
                    .collect()
            })
            .unwrap_or_default();
        let price = if let Some(max) = plan.iter().copied().reduce(f64::max) {
            let min = plan.iter().copied().fold(f64::INFINITY, f64::min);
            let basis = if plan.len() > 1 && min < max {
                QuoteBasis::Estimate
            } else {
                QuoteBasis::Exact
            };
            (crate::usd_to_micro(max), basis)
        } else if let Some(usd) = v.pointer("/endpoint/cost/usd").and_then(Value::as_f64) {
            (crate::usd_to_micro(usd), QuoteBasis::Exact)
        } else {
            return Err(EnrichError::Upstream(format!(
                "treg catalog has no price for {tool}"
            )));
        };
        tracing::debug!(tool, micro_usd = price.0, "treg price cached");
        self.lock_prices().insert(tool, price);
        Ok(price)
    }

    fn lock_prices(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<&'static str, (MicroUsd, QuoteBasis)>> {
        self.prices.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// One `POST /call/<tool>` with a cost ceiling equal to the quoted price.
    fn call(&self, token: &str, tool: &'static str, body: &Value) -> Result<Called, EnrichError> {
        let (ceiling, _) = self.price(tool)?;
        let url = format!("{}/call/{tool}", self.endpoint);
        let mut resp = self
            .agent
            .post(&url)
            .header("X-Treg-Token", token)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header(
                "X-Treg-Route-Max-Cost",
                format!("{:.6}", ceiling as f64 / 1e6),
            )
            .send(body.to_string())
            .map_err(transport_error)?;
        let status = resp.status().as_u16();
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let cost_header = header("X-Treg-Cost-Micro").and_then(|v| v.trim().parse::<u64>().ok());
        let served_by = header("X-Treg-Served-By");
        let retry_after = header("Retry-After").and_then(|v| v.trim().parse::<u64>().ok());
        let text = resp.body_mut().read_to_string().map_err(transport_error)?;
        tracing::debug!(
            tool,
            status,
            cost_micro = cost_header,
            served_by = served_by.as_deref(),
            "treg call"
        );
        match status {
            200 => {}
            202 => {
                return Err(EnrichError::Upstream(format!(
                    "treg call to {tool} is still pending"
                )))
            }
            401 | 403 => {
                return Err(EnrichError::Auth(format!(
                    "treg rejected the token (HTTP {status}): {}",
                    detail(&text)
                )))
            }
            402 => return Err(EnrichError::Payment(format!("treg: {}", detail(&text)))),
            404 => return Err(EnrichError::Unsupported(format!("treg has no tool {tool}"))),
            429 => return Err(EnrichError::RateLimited(retry_after)),
            _ => {
                return Err(EnrichError::Upstream(format!(
                    "treg HTTP {status}: {}",
                    detail(&text)
                )))
            }
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|_| EnrichError::Upstream(format!("treg response from {tool} is not JSON")))?;
        let meta = v.get("_treg");
        let outcome = meta
            .and_then(|m| m.get("outcome"))
            .and_then(Value::as_str)
            .unwrap_or("hit");
        if outcome == "pending" {
            return Err(EnrichError::Upstream(format!(
                "treg call to {tool} is still pending"
            )));
        }
        let charged = meta
            .and_then(|m| m.get("charged_micro"))
            .and_then(Value::as_u64);
        let provider = meta
            .and_then(|m| m.get("provider"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(served_by);
        Ok(Called {
            miss: outcome == "miss",
            output: v.get("output").cloned().unwrap_or(Value::Null),
            cost: cost_header.or(charged).unwrap_or(ceiling),
            provider,
        })
    }
}

struct Called {
    miss: bool,
    output: Value,
    cost: MicroUsd,
    provider: Option<String>,
}

fn transport_error(e: ureq::Error) -> EnrichError {
    match e {
        ureq::Error::Timeout(_) => EnrichError::Timeout,
        ureq::Error::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            EnrichError::Timeout
        }
        other => EnrichError::Upstream(format!("treg unreachable: {other}")),
    }
}

/// A short, human-readable reason from a treg error body.
fn detail(body: &str) -> String {
    let text = match serde_json::from_str::<Value>(body) {
        Ok(v) => match v.get("detail") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Object(o)) => ["error", "code", "reason", "message", "type"]
                .iter()
                .find_map(|k| o.get(*k).and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| Value::Object(o.clone()).to_string()),
            Some(other) => other.to_string(),
            None => v.to_string(),
        },
        Err(_) => body.trim().to_string(),
    };
    text.chars().take(200).collect()
}

impl Enricher for TregEnricher {
    fn name(&self) -> &'static str {
        "treg"
    }

    fn supports(&self, op: &str) -> bool {
        map::tool_for_op(op).is_some()
    }

    fn quote(&self, jobs: &[EnrichJob]) -> Result<Vec<QuoteLine>, EnrichError> {
        jobs.iter()
            .map(|job| {
                let plan = map::plan(&job.request);
                let tools = plan.tools();
                let mut micro_usd: MicroUsd = 0;
                let mut basis = if tools.is_empty() {
                    QuoteBasis::Free
                } else {
                    QuoteBasis::Exact
                };
                for tool in tools {
                    let (p, b) = self.price(tool)?;
                    micro_usd = micro_usd.saturating_add(p);
                    if b == QuoteBasis::Estimate {
                        basis = QuoteBasis::Estimate;
                    }
                }
                Ok(QuoteLine {
                    job: job.clone(),
                    micro_usd,
                    basis,
                    provider_hint: None,
                })
            })
            .collect()
    }

    fn enrich(&self, job: &EnrichJob) -> Result<EnrichResult, EnrichError> {
        let plan = map::plan(&job.request);
        if let CallPlan::Insufficient(why) = plan {
            return Err(EnrichError::Unsupported(why.to_string()));
        }
        let Some(token) = self.token.as_deref() else {
            return Err(EnrichError::Auth(format!(
                "no treg token: set {}",
                self.token_env
            )));
        };
        let result = |data, cost, provider| EnrichResult {
            job: job.clone(),
            cost_micro_usd: cost,
            provider,
            data,
            fetched_at: crate::unix_now(),
        };
        match plan {
            CallPlan::One { tool, body } => {
                let c = self.call(token, tool, &body)?;
                let data = if c.miss {
                    EnrichData::NotFound
                } else {
                    map::output_to_data(&job.request, &c.output)
                };
                Ok(result(data, c.cost, c.provider))
            }
            CallPlan::ProfileViaCompanyName { full_name, company } => {
                let c = self.call(token, map::COMPANY, &serde_json::json!({ "name": company }))?;
                let domain = (!c.miss).then(|| map::company_domain(&c.output)).flatten();
                let Some(domain) = domain else {
                    return Ok(result(EnrichData::NotFound, c.cost, c.provider));
                };
                let body = serde_json::json!({ "full_name": full_name, "domain": domain });
                // If this fails, the company lookup has already cost money;
                // the quote covered both calls and the gate logs the error.
                let p = self.call(token, map::PROFILE, &body)?;
                let data = if p.miss {
                    EnrichData::NotFound
                } else {
                    map::output_to_data(&job.request, &p.output)
                };
                Ok(result(data, c.cost.saturating_add(p.cost), p.provider))
            }
            CallPlan::Insufficient(why) => Err(EnrichError::Unsupported(why.to_string())),
        }
    }
}
