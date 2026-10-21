//! The enrichment seam: the [`Enricher`] trait, its request / quote / result
//! types, and [`NullEnricher`]. This is the only crate allowed to send user
//! data to a third party. Concrete backends sit behind cargo features:
//! `treg` (`TregEnricher`, blocking HTTP), `ownkeys` (a reserved stub) and
//! `mock` / `test-util` (`MockEnricher`, for tests).
//!
//! Authoritative spec: `docs/tech-spec/enrichment.md` section 2. Budget,
//! `no_enrich` and `enrich_log` enforcement live in core's `EgressGate`
//! (Phase 3), never in a backend. Money is integer micro-USD.
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

#[cfg(any(test, feature = "mock"))]
pub mod mock;

/// Current unix time in seconds (0 if the clock is before 1970).
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Integer micro-USD (`1 USD = 1_000_000`).
pub type MicroUsd = u64;

/// Convert a user-facing dollar amount to micro-USD (rounded, never negative).
pub fn usd_to_micro(usd: f64) -> MicroUsd {
    if usd.is_finite() && usd > 0.0 {
        (usd * 1_000_000.0).round() as MicroUsd
    } else {
        0
    }
}

/// One unit of outside work. Serialized as `{"kind": "...", ...fields}`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnrichRequest {
    /// Public professional profile.
    Profile {
        /// Person name.
        name: String,
        /// Company hint.
        company: Option<String>,
        /// LinkedIn slug or URL.
        linkedin: Option<String>,
    },
    /// Find a likely work email at a domain.
    FindEmail {
        /// Person name.
        name: String,
        /// Company domain.
        domain: String,
    },
    /// Check deliverability of one address.
    VerifyEmail {
        /// Address to check.
        email: String,
    },
    /// Company facts by domain.
    Company {
        /// Company domain.
        domain: String,
    },
    /// Generic web search.
    WebSearch {
        /// Query string.
        query: String,
    },
}

impl EnrichRequest {
    /// `"profile" | "find_email" | "verify_email" | "company" | "web_search"`.
    pub fn op(&self) -> &'static str {
        match self {
            EnrichRequest::Profile { .. } => "profile",
            EnrichRequest::FindEmail { .. } => "find_email",
            EnrichRequest::VerifyEmail { .. } => "verify_email",
            EnrichRequest::Company { .. } => "company",
            EnrichRequest::WebSearch { .. } => "web_search",
        }
    }

    /// The exact fields that will leave the machine, for display and for
    /// `enrich_log.request_json`. Unset optional fields are not listed.
    pub fn outbound_fields(&self) -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        match self {
            EnrichRequest::Profile {
                name,
                company,
                linkedin,
            } => {
                v.push(("name", name.clone()));
                if let Some(c) = company {
                    v.push(("company", c.clone()));
                }
                if let Some(l) = linkedin {
                    v.push(("linkedin", l.clone()));
                }
            }
            EnrichRequest::FindEmail { name, domain } => {
                v.push(("name", name.clone()));
                v.push(("domain", domain.clone()));
            }
            EnrichRequest::VerifyEmail { email } => v.push(("email", email.clone())),
            EnrichRequest::Company { domain } => v.push(("domain", domain.clone())),
            EnrichRequest::WebSearch { query } => v.push(("query", query.clone())),
        }
        v
    }
}

/// A request bound to the person it is about.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EnrichJob {
    /// Anchor person id; `None` for company / web search not tied to a person.
    pub person_id: Option<i64>,
    /// The request.
    pub request: EnrichRequest,
}

/// How a quoted price was obtained.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuoteBasis {
    /// Backend publishes a per-call price.
    Exact,
    /// Derived from user-configured prices or a range.
    Estimate,
    /// Costs nothing.
    Free,
}

/// The price of one job.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct QuoteLine {
    /// The job.
    pub job: EnrichJob,
    /// Price.
    pub micro_usd: MicroUsd,
    /// How the price was obtained.
    pub basis: QuoteBasis,
    /// Informational upstream provider.
    pub provider_hint: Option<String>,
}

impl QuoteLine {
    /// Sum of the prices of `lines`, saturating.
    pub fn total(lines: &[QuoteLine]) -> MicroUsd {
        lines
            .iter()
            .fold(0, |acc: MicroUsd, l| acc.saturating_add(l.micro_usd))
    }
}

/// Why a job was removed during planning.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The person has `no_enrich`.
    NoEnrich,
    /// The person has `do_not_share` and `respect_do_not_share` is on.
    DoNotShare,
    /// The graph already has the answer.
    LocalDataSufficient,
    /// Would exceed the budget.
    OverBudget,
    /// `enrich.backend = "none"`.
    BackendNone,
    /// The backend does not support the op.
    Unsupported,
}

/// A job that will not be sent, and why.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Skipped {
    /// Person, if any.
    pub person_id: Option<i64>,
    /// Op name.
    pub op: String,
    /// Reason.
    pub reason: SkipReason,
}

/// A priced plan of outside work. Built by core's `EgressGate`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Quote {
    /// `"treg"`, `"ownkeys"`, `"none"`, `"mock"`.
    pub backend: String,
    /// Priced jobs.
    pub lines: Vec<QuoteLine>,
    /// Sum of `lines`.
    pub total_micro_usd: MicroUsd,
    /// Effective budget for this run.
    pub budget_micro_usd: MicroUsd,
    /// `total <= budget`.
    pub within_budget: bool,
    /// Jobs removed during planning.
    pub skipped: Vec<Skipped>,
    /// Unix seconds.
    pub quoted_at: i64,
}

impl Quote {
    /// Number of calls this quote would make.
    pub fn calls(&self) -> usize {
        self.lines.len()
    }
}

/// Deliverability verdict.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerifyStatus {
    /// Accepts mail.
    Deliverable,
    /// Bounces.
    Undeliverable,
    /// Catch-all or similar.
    Risky,
    /// Could not tell.
    Unknown,
}

/// Normalized profile facts.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProfileData {
    /// Name.
    pub display_name: Option<String>,
    /// Title.
    pub title: Option<String>,
    /// Current company.
    pub company: Option<String>,
    /// Current company domain.
    pub company_domain: Option<String>,
    /// Location.
    pub location: Option<String>,
    /// Canonical LinkedIn slug.
    pub linkedin: Option<String>,
    /// Past employers.
    pub past_companies: Vec<String>,
    /// Schools.
    pub schools: Vec<String>,
    /// Headline; becomes `bio` only if bio is empty.
    pub headline: Option<String>,
}

/// Normalized company facts.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CompanyData {
    /// Canonical name.
    pub name: Option<String>,
    /// Domain.
    pub domain: String,
    /// Industry.
    pub industry: Option<String>,
    /// Size bucket.
    pub size: Option<String>,
    /// HQ location.
    pub location: Option<String>,
}

/// One web search hit.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebHit {
    /// Page title.
    pub title: String,
    /// URL.
    pub url: String,
    /// Snippet.
    pub snippet: Option<String>,
    /// Publication date, if known.
    pub published: Option<String>,
}

/// The normalized payload of a result.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EnrichData {
    /// Profile facts.
    Profile(ProfileData),
    /// A found email.
    Email {
        /// Address.
        email: String,
        /// Backend confidence.
        confidence: f32,
    },
    /// A verification verdict.
    Verification {
        /// Address.
        email: String,
        /// Verdict.
        status: VerifyStatus,
    },
    /// Company facts.
    Company(CompanyData),
    /// Web search hits.
    WebHits {
        /// Hits.
        hits: Vec<WebHit>,
    },
    /// Nothing found (may still cost money).
    NotFound,
}

/// The outcome of one executed job.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EnrichResult {
    /// The job.
    pub job: EnrichJob,
    /// Actual cost reported by the backend.
    pub cost_micro_usd: MicroUsd,
    /// Upstream provider.
    pub provider: Option<String>,
    /// Payload.
    pub data: EnrichData,
    /// Unix seconds.
    pub fetched_at: i64,
}

/// Errors from an enrichment backend.
#[derive(Debug, thiserror::Error)]
pub enum EnrichError {
    /// `enrich.backend = "none"` (the plan calls this "EgressDisabled").
    #[error("enrichment is off: set enrich.backend")]
    Disabled,
    /// The backend is misconfigured (for example, no token in the environment).
    #[error("enrichment backend is not configured: {0}")]
    Config(String),
    /// Op not supported by this backend.
    #[error("operation not supported by backend: {0}")]
    Unsupported(String),
    /// Bad or missing credentials.
    #[error("authentication failed: {0}")]
    Auth(String),
    /// Retry later.
    #[error("rate limited, retry after {0:?}s")]
    RateLimited(Option<u64>),
    /// The backend refused to spend: no balance, or no provider fits the
    /// per-call cost ceiling. Nothing was charged.
    #[error("payment required: {0}")]
    Payment(String),
    /// Upstream failure.
    #[error("upstream error: {0}")]
    Upstream(String),
    /// Request timed out.
    #[error("timeout")]
    Timeout,
}

/// An enrichment backend. Called only by core's `EgressGate`.
pub trait Enricher: Send + Sync {
    /// Stable backend id: `"none"`, `"treg"`, `"ownkeys"`, `"mock"`.
    fn name(&self) -> &'static str;
    /// Which ops this backend can serve.
    fn supports(&self, op: &str) -> bool;
    /// Price each job BEFORE any call. Must not send contact data.
    fn quote(&self, jobs: &[EnrichJob]) -> Result<Vec<QuoteLine>, EnrichError>;
    /// Perform one job.
    fn enrich(&self, job: &EnrichJob) -> Result<EnrichResult, EnrichError>;
}

/// The default backend: supports nothing, quotes nothing, sends nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullEnricher;

impl Enricher for NullEnricher {
    fn name(&self) -> &'static str {
        "none"
    }
    fn supports(&self, _op: &str) -> bool {
        false
    }
    fn quote(&self, _jobs: &[EnrichJob]) -> Result<Vec<QuoteLine>, EnrichError> {
        Ok(Vec::new())
    }
    fn enrich(&self, _job: &EnrichJob) -> Result<EnrichResult, EnrichError> {
        Err(EnrichError::Disabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_with_kind_tag() {
        let r = EnrichRequest::Profile {
            name: "Priya Example".into(),
            company: Some("Northwind".into()),
            linkedin: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["kind"], "profile");
        assert_eq!(v["company"], "Northwind");
        let back: EnrichRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn op_names() {
        let ops: Vec<_> = [
            EnrichRequest::Profile {
                name: "a".into(),
                company: None,
                linkedin: None,
            },
            EnrichRequest::FindEmail {
                name: "a".into(),
                domain: "example.com".into(),
            },
            EnrichRequest::VerifyEmail {
                email: "a@example.com".into(),
            },
            EnrichRequest::Company {
                domain: "example.com".into(),
            },
            EnrichRequest::WebSearch { query: "q".into() },
        ]
        .iter()
        .map(EnrichRequest::op)
        .collect();
        assert_eq!(
            ops,
            [
                "profile",
                "find_email",
                "verify_email",
                "company",
                "web_search"
            ]
        );
    }

    #[test]
    fn outbound_fields_list_exactly_what_is_sent() {
        let r = EnrichRequest::Profile {
            name: "Ada".into(),
            company: None,
            linkedin: Some("ada-x".into()),
        };
        assert_eq!(
            r.outbound_fields(),
            vec![
                ("name", "Ada".to_string()),
                ("linkedin", "ada-x".to_string())
            ]
        );
    }

    #[test]
    fn null_enricher_never_sends() {
        let e = NullEnricher;
        let job = EnrichJob {
            person_id: Some(1),
            request: EnrichRequest::Company {
                domain: "example.com".into(),
            },
        };
        assert_eq!(e.name(), "none");
        assert!(!e.supports("profile"));
        assert!(e.quote(std::slice::from_ref(&job)).unwrap().is_empty());
        assert!(matches!(e.enrich(&job), Err(EnrichError::Disabled)));
    }

    #[test]
    fn null_enricher_says_how_to_turn_it_on() {
        let job = EnrichJob {
            person_id: None,
            request: EnrichRequest::WebSearch { query: "q".into() },
        };
        let err = NullEnricher.enrich(&job).unwrap_err();
        assert_eq!(err.to_string(), "enrichment is off: set enrich.backend");
    }

    #[test]
    fn trait_is_object_safe() {
        let e: Box<dyn Enricher> = Box::new(NullEnricher);
        assert_eq!(e.name(), "none");
    }

    #[test]
    fn quote_helpers_count_calls_and_sum() {
        let job = |q: &str| EnrichJob {
            person_id: None,
            request: EnrichRequest::WebSearch { query: q.into() },
        };
        let lines = vec![
            QuoteLine {
                job: job("a"),
                micro_usd: 10_000,
                basis: QuoteBasis::Exact,
                provider_hint: None,
            },
            QuoteLine {
                job: job("b"),
                micro_usd: 25_000,
                basis: QuoteBasis::Estimate,
                provider_hint: None,
            },
        ];
        assert_eq!(QuoteLine::total(&lines), 35_000);
        let q = Quote {
            backend: "mock".into(),
            total_micro_usd: QuoteLine::total(&lines),
            lines,
            budget_micro_usd: 500_000,
            within_budget: true,
            skipped: vec![],
            quoted_at: 0,
        };
        assert_eq!(q.calls(), 2);
    }

    #[test]
    fn quote_serializes_to_documented_shape() {
        // interfaces.md section 1.9 (quoted_at is converted to ISO 8601 at the surface).
        let q = Quote {
            backend: "treg".into(),
            lines: vec![QuoteLine {
                job: EnrichJob {
                    person_id: None,
                    request: EnrichRequest::Profile {
                        name: "Priya Raman".into(),
                        company: Some("Stripe".into()),
                        linkedin: None,
                    },
                },
                micro_usd: 10_000,
                basis: QuoteBasis::Exact,
                provider_hint: None,
            }],
            total_micro_usd: 10_000,
            budget_micro_usd: 500_000,
            within_budget: true,
            skipped: vec![],
            quoted_at: 0,
        };
        let v = serde_json::to_value(&q).unwrap();
        assert_eq!(
            v["lines"][0],
            serde_json::json!({ "job": { "person_id": null, "request": { "kind": "profile", "name": "Priya Raman", "company": "Stripe", "linkedin": null } },
                   "micro_usd": 10000, "basis": "exact", "provider_hint": null })
        );
        assert_eq!(v["total_micro_usd"], 10_000);
        assert_eq!(v["within_budget"], true);
        assert_eq!(v["skipped"], serde_json::json!([]));
    }

    #[test]
    fn usd_conversion() {
        assert_eq!(usd_to_micro(0.5), 500_000);
        assert_eq!(usd_to_micro(0.01), 10_000);
        assert_eq!(usd_to_micro(-1.0), 0);
        assert_eq!(usd_to_micro(f64::NAN), 0);
    }

    #[test]
    fn enrich_data_is_type_tagged() {
        let d = EnrichData::Email {
            email: "a@example.com".into(),
            confidence: 0.5,
        };
        assert_eq!(serde_json::to_value(&d).unwrap()["type"], "email");
        assert_eq!(
            serde_json::to_value(EnrichData::NotFound).unwrap()["type"],
            "not_found"
        );
    }
}
