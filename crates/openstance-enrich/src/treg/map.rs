//! The one table that maps `EnrichRequest` to treg routed tools, plus the
//! request-body and output mappings. Verified against treg 0.27 (see
//! `docs/tech-spec/enrichment.md` section 3.2).

use serde_json::{json, Map, Value};

use crate::{CompanyData, EnrichData, EnrichRequest, ProfileData, VerifyStatus, WebHit};

/// Routed tool for a profile lookup.
pub const PROFILE: &str = "treg.people.enrich";
/// Routed tool for finding a work email.
pub const FIND_EMAIL: &str = "treg.people.email.find";
/// Routed tool for verifying an email.
pub const VERIFY_EMAIL: &str = "treg.people.email.verify";
/// Routed tool for company facts.
pub const COMPANY: &str = "treg.companies.enrich";
/// Routed tool for web search.
pub const WEB_SEARCH: &str = "treg.web.search";

/// Number of web hits asked for.
pub const WEB_SEARCH_LIMIT: u32 = 10;

/// treg tool id for an op name.
pub fn tool_for_op(op: &str) -> Option<&'static str> {
    Some(match op {
        "profile" => PROFILE,
        "find_email" => FIND_EMAIL,
        "verify_email" => VERIFY_EMAIL,
        "company" => COMPANY,
        "web_search" => WEB_SEARCH,
        _ => return None,
    })
}

/// How a request is sent: one call, or (for a profile known only by company
/// name) a company lookup to get the domain, then the profile call.
#[derive(Debug, Clone, PartialEq)]
pub enum CallPlan {
    /// One call to `tool` with `body`.
    One {
        /// treg tool id.
        tool: &'static str,
        /// JSON body.
        body: Value,
    },
    /// `treg.companies.enrich {name}` first, then `treg.people.enrich {full_name, domain}`.
    ProfileViaCompanyName {
        /// Person name.
        full_name: String,
        /// Company name.
        company: String,
    },
    /// Not enough identity to make a call.
    Insufficient(&'static str),
}

impl CallPlan {
    /// Tools this plan will call, in order (for quoting).
    pub fn tools(&self) -> Vec<&'static str> {
        match self {
            CallPlan::One { tool, .. } => vec![tool],
            CallPlan::ProfileViaCompanyName { .. } => vec![COMPANY, PROFILE],
            CallPlan::Insufficient(_) => vec![],
        }
    }
}

/// True when `s` looks like a bare domain (`stripe.com`), not a company name.
pub fn looks_like_domain(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && !s.contains(char::is_whitespace)
        && !s.contains('@')
        && s.contains('.')
        && !s.starts_with('.')
        && !s.ends_with('.')
}

fn is_linkedin_url(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://") || l.contains("linkedin.com/")
}

/// The canonical LinkedIn slug of a profile URL (or the input, if it is a slug).
pub fn linkedin_slug(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if !is_linkedin_url(s) {
        return Some(s.trim_matches('/').to_string());
    }
    let path = s.split(['?', '#']).next().unwrap_or(s);
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    while let Some(p) = parts.next() {
        if p == "in" || p == "pub" {
            return parts.next().map(str::to_string);
        }
    }
    None
}

/// Map a request to its treg call(s).
pub fn plan(req: &EnrichRequest) -> CallPlan {
    match req {
        EnrichRequest::Profile {
            name,
            company,
            linkedin,
        } => {
            if let Some(l) = linkedin.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
                let body = if is_linkedin_url(l) {
                    json!({ "linkedin_url": l })
                } else {
                    json!({ "linkedin_handle": l })
                };
                return CallPlan::One {
                    tool: PROFILE,
                    body,
                };
            }
            match company.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                Some(c) if looks_like_domain(c) => CallPlan::One {
                    tool: PROFILE,
                    body: json!({ "full_name": name, "domain": c.to_ascii_lowercase() }),
                },
                Some(c) => CallPlan::ProfileViaCompanyName {
                    full_name: name.clone(),
                    company: c.to_string(),
                },
                None => {
                    CallPlan::Insufficient("profile needs a LinkedIn URL or handle, or a company")
                }
            }
        }
        EnrichRequest::FindEmail { name, domain } => CallPlan::One {
            tool: FIND_EMAIL,
            body: json!({ "full_name": name, "domain": domain }),
        },
        EnrichRequest::VerifyEmail { email } => CallPlan::One {
            tool: VERIFY_EMAIL,
            body: json!({ "email": email }),
        },
        EnrichRequest::Company { domain } => CallPlan::One {
            tool: COMPANY,
            body: json!({ "domain": domain }),
        },
        EnrichRequest::WebSearch { query } => CallPlan::One {
            tool: WEB_SEARCH,
            body: json!({ "q": query, "limit": WEB_SEARCH_LIMIT }),
        },
    }
}

fn s(o: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match o.get(*k) {
        Some(Value::String(v)) if !v.trim().is_empty() => Some(v.trim().to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    })
}

fn strings(o: &Map<String, Value>, key: &str) -> Vec<String> {
    match o.get(key) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                Value::Object(m) => s(m, &["name", "company", "school"]),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Map a treg `output` object to `EnrichData`. A missing or empty required
/// field is a miss ([`EnrichData::NotFound`]).
pub fn output_to_data(req: &EnrichRequest, output: &Value) -> EnrichData {
    let Some(o) = output.as_object() else {
        return EnrichData::NotFound;
    };
    match req {
        EnrichRequest::Profile { .. } => {
            let first_last = match (s(o, &["first_name"]), s(o, &["last_name"])) {
                (Some(f), Some(l)) => Some(format!("{f} {l}")),
                (f, l) => f.or(l),
            };
            let Some(display_name) = s(o, &["full_name"]).or(first_last) else {
                return EnrichData::NotFound;
            };
            EnrichData::Profile(ProfileData {
                display_name: Some(display_name),
                title: s(o, &["title"]),
                company: s(o, &["company"]),
                company_domain: s(o, &["company_domain"]).map(|d| d.to_ascii_lowercase()),
                location: s(o, &["location"]),
                linkedin: s(o, &["linkedin_url", "linkedin_handle"])
                    .and_then(|l| linkedin_slug(&l)),
                past_companies: strings(o, "past_companies"),
                schools: strings(o, "schools"),
                headline: s(o, &["headline"]),
            })
        }
        EnrichRequest::FindEmail { .. } => {
            let Some(email) = s(o, &["email"]) else {
                return EnrichData::NotFound;
            };
            let mut confidence = o.get("confidence").and_then(Value::as_f64).unwrap_or(0.0);
            if confidence > 1.0 {
                confidence /= 100.0; // some providers report a percentage
            }
            EnrichData::Email {
                email,
                confidence: confidence.clamp(0.0, 1.0) as f32,
            }
        }
        EnrichRequest::VerifyEmail { email } => {
            let valid = o.get("valid").and_then(Value::as_bool);
            let status = s(o, &["status"]).map(|s| s.to_ascii_lowercase());
            if valid.is_none() && status.is_none() {
                return EnrichData::NotFound;
            }
            EnrichData::Verification {
                email: email.clone(),
                status: verify_status(status.as_deref(), valid),
            }
        }
        EnrichRequest::Company { domain } => {
            let Some(name) = s(o, &["name"]) else {
                return EnrichData::NotFound;
            };
            EnrichData::Company(CompanyData {
                name: Some(name),
                domain: s(o, &["domain"])
                    .map(|d| d.to_ascii_lowercase())
                    .unwrap_or_else(|| domain.clone()),
                industry: s(o, &["industry"]),
                size: s(o, &["employees", "size"]),
                location: s(o, &["location"]),
            })
        }
        EnrichRequest::WebSearch { .. } => {
            let Some(results) = o.get("results").and_then(Value::as_array) else {
                return EnrichData::NotFound;
            };
            let hits: Vec<WebHit> = results
                .iter()
                .filter_map(Value::as_object)
                .filter_map(|r| {
                    let url = s(r, &["url", "link", "href"])?;
                    Some(WebHit {
                        title: s(r, &["title", "name"]).unwrap_or_else(|| url.clone()),
                        url,
                        snippet: s(r, &["snippet", "description", "content", "text"]),
                        published: s(r, &["published", "published_date", "published_at", "date"]),
                    })
                })
                .collect();
            if hits.is_empty() {
                EnrichData::NotFound
            } else {
                EnrichData::WebHits { hits }
            }
        }
    }
}

/// The company domain from a `treg.companies.enrich` output, if any.
pub fn company_domain(output: &Value) -> Option<String> {
    let o = output.as_object()?;
    if let Some(d) = s(o, &["domain"]) {
        return Some(d.to_ascii_lowercase());
    }
    let site = s(o, &["website"])?;
    let host = site
        .split("://")
        .last()
        .unwrap_or(&site)
        .split(['/', '?', '#'])
        .next()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    looks_like_domain(&host).then_some(host)
}

fn verify_status(status: Option<&str>, valid: Option<bool>) -> VerifyStatus {
    match status {
        Some("valid" | "deliverable" | "ok" | "safe") => VerifyStatus::Deliverable,
        Some("invalid" | "undeliverable" | "bounce" | "bounced" | "disposable") => {
            VerifyStatus::Undeliverable
        }
        Some("accept_all" | "catch_all" | "catchall" | "risky" | "accept-all") => {
            VerifyStatus::Risky
        }
        _ => match valid {
            Some(true) => VerifyStatus::Deliverable,
            Some(false) => VerifyStatus::Undeliverable,
            None => VerifyStatus::Unknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_op_has_a_tool() {
        for op in [
            "profile",
            "find_email",
            "verify_email",
            "company",
            "web_search",
        ] {
            assert!(tool_for_op(op).unwrap().starts_with("treg."));
        }
        assert_eq!(tool_for_op("parse"), None);
    }

    #[test]
    fn domain_detection() {
        assert!(looks_like_domain("stripe.com"));
        assert!(looks_like_domain("acme.co.uk"));
        assert!(!looks_like_domain("Stripe"));
        assert!(!looks_like_domain("Acme Inc."));
        assert!(!looks_like_domain("a@b.com"));
    }

    #[test]
    fn slugs() {
        assert_eq!(
            linkedin_slug("https://www.linkedin.com/in/ada-x/?trk=1").as_deref(),
            Some("ada-x")
        );
        assert_eq!(linkedin_slug("ada-x").as_deref(), Some("ada-x"));
        assert_eq!(linkedin_slug("https://linkedin.com/company/acme"), None);
    }

    #[test]
    fn verify_statuses() {
        assert_eq!(
            verify_status(Some("accept_all"), Some(true)),
            VerifyStatus::Risky
        );
        assert_eq!(
            verify_status(Some("invalid"), None),
            VerifyStatus::Undeliverable
        );
        assert_eq!(
            verify_status(Some("weird"), Some(true)),
            VerifyStatus::Deliverable
        );
        assert_eq!(verify_status(None, None), VerifyStatus::Unknown);
    }

    #[test]
    fn company_domain_falls_back_to_website() {
        assert_eq!(
            company_domain(&json!({"name": "Acme", "website": "https://www.acme.example/about"}))
                .as_deref(),
            Some("acme.example")
        );
        assert_eq!(company_domain(&json!({"name": "Acme"})), None);
    }

    #[test]
    fn percentage_confidence_is_scaled() {
        let d = output_to_data(
            &EnrichRequest::FindEmail {
                name: "a".into(),
                domain: "b.c".into(),
            },
            &json!({"email": "a@b.c", "confidence": 90}),
        );
        assert_eq!(
            d,
            EnrichData::Email {
                email: "a@b.c".into(),
                confidence: 0.9
            }
        );
    }
}
