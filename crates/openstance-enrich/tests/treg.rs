//! TregEnricher against an in-process mock of treg's HTTP API (P3-ENR-04).
//! No test here reaches the internet: every request goes to 127.0.0.1.
#![cfg(feature = "treg")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use openstance_enrich::treg::TregEnricher;
use openstance_enrich::{
    EnrichData, EnrichError, EnrichJob, EnrichRequest, Enricher, QuoteBasis, VerifyStatus,
};
use serde_json::Value;

const TOKEN: &str = "treg_SENTINEL_TOKEN_5f2c";

// ---------------------------------------------------------------- mock server

#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

impl Req {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
    delay: Duration,
}

impl Resp {
    fn json(status: u16, body: impl Into<String>) -> Self {
        Resp {
            status,
            headers: vec![],
            body: body.into(),
            delay: Duration::ZERO,
        }
    }
    fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
    fn delayed(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }
}

type Handler = dyn Fn(&Req) -> Resp + Send + Sync;

struct Server {
    base: String,
    log: Arc<Mutex<Vec<Req>>>,
}

impl Server {
    fn start(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let log2 = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (handler, log) = (handler.clone(), log2.clone());
                std::thread::spawn(move || serve(stream, &*handler, &log));
            }
        });
        Server { base, log }
    }
    fn requests(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }
}

fn serve(stream: TcpStream, handler: &Handler, log: &Mutex<Vec<Req>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    reader.read_exact(&mut body).unwrap();
    let req = Req {
        method,
        path,
        headers,
        body: String::from_utf8(body).unwrap(),
    };
    log.lock().unwrap().push(req.clone());
    let resp = handler(&req);
    std::thread::sleep(resp.delay);
    let mut out = stream;
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        resp.status,
        resp.body.len()
    );
    for (k, v) in &resp.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(resp.body.as_bytes());
    let _ = out.flush();
}

fn fixture(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/enrich/treg")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// A treg that serves the catalog fixtures and the hit fixtures.
fn happy_treg(req: &Req) -> Resp {
    let catalog = |id: &str| match id {
        "treg.people.enrich" => Some("catalog_people_enrich.json"),
        "treg.people.email.find" => Some("catalog_email_find.json"),
        "treg.people.email.verify" => Some("catalog_email_verify.json"),
        "treg.companies.enrich" => Some("catalog_companies_enrich.json"),
        "treg.web.search" => Some("catalog_web_search.json"),
        _ => None,
    };
    if req.method == "GET" {
        if let Some(id) = req.path.strip_prefix("/catalog/endpoints/") {
            return match catalog(id) {
                Some(f) => Resp::json(200, fixture(f)),
                None => Resp::json(404, r#"{"detail":"not found"}"#),
            };
        }
    }
    if req.method == "POST" {
        let (file, cost) = match req.path.as_str() {
            "/call/treg.people.enrich" => ("call_people_enrich_hit.json", "20000"),
            "/call/treg.people.email.find" => ("call_email_find_hit.json", "10000"),
            "/call/treg.people.email.verify" => ("call_email_verify_hit.json", "0"),
            "/call/treg.companies.enrich" => ("call_companies_enrich_hit.json", "50000"),
            "/call/treg.web.search" => ("call_web_search_hit.json", "5000"),
            _ => return Resp::json(404, r#"{"detail":"no such tool"}"#),
        };
        return Resp::json(200, fixture(file))
            .header("X-Treg-Cost-Micro", cost)
            .header("X-Treg-Served-By", "alpha.x")
            .header("X-Treg-Call-Id", "call_1");
    }
    Resp::json(405, r#"{"detail":"method"}"#)
}

fn enricher(base: &str) -> TregEnricher {
    TregEnricher::new(base, Some(TOKEN.to_string()), Duration::from_secs(5))
}

fn job(request: EnrichRequest) -> EnrichJob {
    EnrichJob {
        person_id: Some(42),
        request,
    }
}

fn find_email() -> EnrichRequest {
    EnrichRequest::FindEmail {
        name: "Priya Raman".into(),
        domain: "stripe.com".into(),
    }
}

// ---------------------------------------------------------------- quoting

#[test]
fn name_and_supported_ops() {
    let e = enricher("http://127.0.0.1:9");
    assert_eq!(e.name(), "treg");
    for op in [
        "profile",
        "find_email",
        "verify_email",
        "company",
        "web_search",
    ] {
        assert!(e.supports(op), "{op}");
    }
    assert!(!e.supports("parse"));
}

#[test]
fn quote_uses_catalog_upper_bound_without_token_or_contact_data() {
    let s = Server::start(happy_treg);
    let e = enricher(&s.base);
    let jobs = vec![
        job(find_email()),
        job(EnrichRequest::VerifyEmail {
            email: "priya@stripe.com".into(),
        }),
        job(EnrichRequest::WebSearch {
            query: "\"Priya Raman\"".into(),
        }),
    ];
    let lines = e.quote(&jobs).unwrap();
    assert_eq!(
        lines.iter().map(|l| l.micro_usd).collect::<Vec<_>>(),
        [150_000, 24_000, 160_000]
    );
    assert!(lines.iter().all(|l| l.basis == QuoteBasis::Estimate));
    assert_eq!(lines[0].job, jobs[0]);

    let reqs = s.requests();
    assert!(reqs.iter().all(|r| r.method == "GET"));
    assert!(reqs.iter().all(|r| r.header("x-treg-token").is_none()));
    for r in &reqs {
        for leaked in ["Priya", "stripe.com", "priya@"] {
            assert!(!r.path.contains(leaked), "{} leaked {leaked}", r.path);
            assert!(!r.body.contains(leaked));
        }
    }
}

#[test]
fn quote_prices_are_cached_per_process() {
    let s = Server::start(happy_treg);
    let e = enricher(&s.base);
    e.quote(&[job(find_email())]).unwrap();
    e.quote(&[job(find_email()), job(find_email())]).unwrap();
    assert_eq!(s.requests().len(), 1);
}

#[test]
fn single_child_plan_is_exact() {
    let s = Server::start(|_req: &Req| {
        Resp::json(
            200,
            r#"{"endpoint":{"cost":{"usd":0.01}},"routing":{"plan":[{"endpoint_id":"a","usd":0.03}]}}"#,
        )
    });
    let l = enricher(&s.base)
        .quote(&[job(EnrichRequest::Company {
            domain: "stripe.com".into(),
        })])
        .unwrap();
    assert_eq!(l[0].micro_usd, 30_000);
    assert_eq!(l[0].basis, QuoteBasis::Exact);
}

#[test]
fn direct_cost_is_used_when_there_is_no_plan() {
    let s = Server::start(|_req: &Req| Resp::json(200, r#"{"endpoint":{"cost":{"usd":0.01225}}}"#));
    let l = enricher(&s.base)
        .quote(&[job(EnrichRequest::Company {
            domain: "stripe.com".into(),
        })])
        .unwrap();
    assert_eq!(l[0].micro_usd, 12_250);
    assert_eq!(l[0].basis, QuoteBasis::Exact);
}

#[test]
fn profile_by_company_name_quotes_company_lookup_too() {
    let s = Server::start(happy_treg);
    let e = enricher(&s.base);
    let by_name = job(EnrichRequest::Profile {
        name: "Priya Raman".into(),
        company: Some("Stripe".into()),
        linkedin: None,
    });
    let by_domain = job(EnrichRequest::Profile {
        name: "Priya Raman".into(),
        company: Some("stripe.com".into()),
        linkedin: None,
    });
    let l = e.quote(&[by_name, by_domain]).unwrap();
    assert_eq!(l[0].micro_usd, 380_000 + 380_000);
    assert_eq!(l[1].micro_usd, 380_000);
}

// ---------------------------------------------------------------- calls

#[test]
fn find_email_posts_mapped_body_with_token_and_cost_ceiling() {
    let s = Server::start(happy_treg);
    let e = enricher(&s.base);
    let j = job(find_email());
    e.quote(std::slice::from_ref(&j)).unwrap();
    let r = e.enrich(&j).unwrap();
    assert_eq!(r.job, j);
    assert_eq!(r.cost_micro_usd, 10_000);
    assert_eq!(r.provider.as_deref(), Some("alpha"));
    assert_eq!(
        r.data,
        EnrichData::Email {
            email: "priya@stripe.com".into(),
            confidence: 0.97
        }
    );
    let call = s
        .requests()
        .into_iter()
        .find(|r| r.method == "POST")
        .unwrap();
    assert_eq!(call.path, "/call/treg.people.email.find");
    assert_eq!(call.header("x-treg-token"), Some(TOKEN));
    assert!(call.header("authorization").is_none());
    assert_eq!(call.header("x-treg-route-max-cost"), Some("0.150000"));
    assert!(call
        .header("content-type")
        .unwrap()
        .starts_with("application/json"));
    assert_eq!(
        call.json(),
        serde_json::json!({"full_name": "Priya Raman", "domain": "stripe.com"})
    );
}

#[test]
fn profile_maps_output_and_linkedin_slug() {
    let s = Server::start(happy_treg);
    let e = enricher(&s.base);
    let r = e
        .enrich(&job(EnrichRequest::Profile {
            name: "Priya Raman".into(),
            company: None,
            linkedin: Some("https://www.linkedin.com/in/priya-raman-example".into()),
        }))
        .unwrap();
    let EnrichData::Profile(p) = r.data else {
        panic!("{:?}", r.data)
    };
    assert_eq!(p.display_name.as_deref(), Some("Priya Raman"));
    assert_eq!(p.title.as_deref(), Some("Staff Engineer"));
    assert_eq!(p.company.as_deref(), Some("Stripe"));
    assert_eq!(p.company_domain.as_deref(), Some("stripe.com"));
    assert_eq!(p.location.as_deref(), Some("San Francisco, CA"));
    assert_eq!(p.linkedin.as_deref(), Some("priya-raman-example"));
    assert_eq!(r.cost_micro_usd, 20_000);
    let call = s.requests().pop().unwrap();
    assert_eq!(
        call.json(),
        serde_json::json!({"linkedin_url": "https://www.linkedin.com/in/priya-raman-example"})
    );
}

#[test]
fn profile_request_bodies_by_identity() {
    let s = Server::start(happy_treg);
    let e = enricher(&s.base);
    e.enrich(&job(EnrichRequest::Profile {
        name: "Priya Raman".into(),
        company: Some("Stripe".into()),
        linkedin: Some("priya-raman-example".into()),
    }))
    .unwrap();
    e.enrich(&job(EnrichRequest::Profile {
        name: "Priya Raman".into(),
        company: Some("stripe.com".into()),
        linkedin: None,
    }))
    .unwrap();
    let bodies: Vec<Value> = s
        .requests()
        .iter()
        .filter(|r| r.method == "POST")
        .map(Req::json)
        .collect();
    assert_eq!(
        bodies,
        vec![
            serde_json::json!({"linkedin_handle": "priya-raman-example"}),
            serde_json::json!({"full_name": "Priya Raman", "domain": "stripe.com"}),
        ]
    );
}

#[test]
fn profile_by_company_name_resolves_domain_first_and_sums_cost() {
    let s = Server::start(happy_treg);
    let e = enricher(&s.base);
    let r = e
        .enrich(&job(EnrichRequest::Profile {
            name: "Priya Raman".into(),
            company: Some("Stripe".into()),
            linkedin: None,
        }))
        .unwrap();
    assert!(matches!(r.data, EnrichData::Profile(_)));
    assert_eq!(r.cost_micro_usd, 50_000 + 20_000);
    let posts: Vec<Req> = s
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .collect();
    assert_eq!(posts[0].path, "/call/treg.companies.enrich");
    assert_eq!(posts[0].json(), serde_json::json!({"name": "Stripe"}));
    assert_eq!(posts[1].path, "/call/treg.people.enrich");
    assert_eq!(
        posts[1].json(),
        serde_json::json!({"full_name": "Priya Raman", "domain": "stripe.com"})
    );
}

#[test]
fn profile_with_name_only_is_refused_without_network() {
    let s = Server::start(happy_treg);
    let e = enricher(&s.base);
    let j = job(EnrichRequest::Profile {
        name: "Priya Raman".into(),
        company: None,
        linkedin: None,
    });
    let l = e.quote(std::slice::from_ref(&j)).unwrap();
    assert_eq!(l[0].micro_usd, 0);
    assert!(matches!(e.enrich(&j), Err(EnrichError::Unsupported(_))));
    // The gate learns this before it marks the request sent.
    assert!(matches!(e.preflight(&j), Err(EnrichError::Unsupported(_))));
    assert!(s.requests().is_empty());
}

#[test]
fn verify_company_and_search_map_outputs() {
    let s = Server::start(happy_treg);
    let e = enricher(&s.base);

    let v = e
        .enrich(&job(EnrichRequest::VerifyEmail {
            email: "priya@stripe.com".into(),
        }))
        .unwrap();
    assert_eq!(
        v.data,
        EnrichData::Verification {
            email: "priya@stripe.com".into(),
            status: VerifyStatus::Deliverable
        }
    );
    assert_eq!(v.cost_micro_usd, 0);

    let c = e
        .enrich(&job(EnrichRequest::Company {
            domain: "stripe.com".into(),
        }))
        .unwrap();
    let EnrichData::Company(cd) = c.data else {
        panic!()
    };
    assert_eq!(cd.name.as_deref(), Some("Stripe"));
    assert_eq!(cd.domain, "stripe.com");
    assert_eq!(cd.industry.as_deref(), Some("Financial Services"));
    assert_eq!(cd.size.as_deref(), Some("8000"));
    assert_eq!(cd.location.as_deref(), Some("San Francisco, CA"));

    let w = e
        .enrich(&job(EnrichRequest::WebSearch {
            query: "\"Priya Raman\" \"Stripe\"".into(),
        }))
        .unwrap();
    let EnrichData::WebHits { hits } = w.data else {
        panic!()
    };
    assert_eq!(hits.len(), 2, "the hit without a url is dropped");
    assert_eq!(hits[0].published.as_deref(), Some("2026-09-30"));
    assert_eq!(hits[1].title, "Priya Raman on the Infra Podcast");
    assert_eq!(hits[1].url, "https://podcast.example.com/episodes/42");
    assert_eq!(hits[1].snippet.as_deref(), Some("Episode 42"));

    let bodies: Vec<(String, Value)> = s
        .requests()
        .iter()
        .filter(|r| r.method == "POST")
        .map(|r| (r.path.clone(), r.json()))
        .collect();
    assert_eq!(
        bodies[0].1,
        serde_json::json!({"email": "priya@stripe.com"})
    );
    assert_eq!(bodies[1].1, serde_json::json!({"domain": "stripe.com"}));
    assert_eq!(
        bodies[2].1,
        serde_json::json!({"q": "\"Priya Raman\" \"Stripe\"", "limit": 10})
    );
}

#[test]
fn miss_is_not_found_and_still_costs() {
    let s = Server::start(|req: &Req| {
        if req.method == "GET" {
            return happy_treg(req);
        }
        Resp::json(200, fixture("call_miss.json"))
            .header("X-Treg-Cost-Micro", "10000")
            .header("X-Treg-Route-Outcome", "miss")
    });
    let r = enricher(&s.base).enrich(&job(find_email())).unwrap();
    assert_eq!(r.data, EnrichData::NotFound);
    assert_eq!(r.cost_micro_usd, 10_000);
}

#[test]
fn empty_required_field_is_a_miss_and_charged_micro_is_the_fallback_cost() {
    let s = Server::start(|req: &Req| {
        if req.method == "GET" {
            return happy_treg(req);
        }
        Resp::json(
            200,
            r#"{"output":{"email":""},"raw":null,"_treg":{"outcome":"weak","charged_micro":7000}}"#,
        )
    });
    let r = enricher(&s.base).enrich(&job(find_email())).unwrap();
    assert_eq!(r.data, EnrichData::NotFound);
    assert_eq!(r.cost_micro_usd, 7_000);
}

// ---------------------------------------------------------------- errors

fn call_error(resp: impl Fn() -> Resp + Send + Sync + 'static) -> EnrichError {
    let s = Server::start(move |req: &Req| {
        if req.method == "GET" {
            return happy_treg(req);
        }
        resp()
    });
    enricher(&s.base).enrich(&job(find_email())).unwrap_err()
}

#[test]
fn unauthorized_is_auth_error() {
    let e = call_error(|| Resp::json(401, r#"{"detail":"invalid token"}"#));
    assert!(matches!(e, EnrichError::Auth(_)), "{e:?}");
    assert!(!e.to_string().contains(TOKEN));
}

#[test]
fn insufficient_balance_is_payment_error() {
    let e = call_error(|| {
        Resp::json(
            402,
            r#"{"detail":{"error":"insufficient_balance","balance_micro":0,"estimated_cost_micro":150000,"topup_url":"https://treg.to/billing"}}"#,
        )
    });
    match e {
        EnrichError::Payment(m) => assert!(m.contains("insufficient_balance"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn route_max_cost_is_payment_error() {
    let e = call_error(|| Resp::json(402, r#"{"detail":"route_max_cost"}"#));
    match e {
        EnrichError::Payment(m) => assert!(m.contains("route_max_cost"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn rate_limit_carries_retry_after() {
    let e = call_error(|| Resp::json(429, r#"{"detail":"slow down"}"#).header("Retry-After", "7"));
    assert!(matches!(e, EnrichError::RateLimited(Some(7))), "{e:?}");
}

#[test]
fn provider_failures_are_upstream_errors() {
    for (status, body) in [
        (503, r#"{"detail":"provider_capacity_unavailable"}"#),
        (422, r#"{"detail":"route_no_candidate"}"#),
        (500, "oops"),
        (202, r#"{"output":null,"_treg":{"outcome":"pending"}}"#),
    ] {
        let e = call_error(move || Resp::json(status, body));
        assert!(matches!(e, EnrichError::Upstream(_)), "{status}: {e:?}");
    }
}

#[test]
fn unparseable_body_is_upstream_error() {
    let e = call_error(|| Resp::json(200, "<html>"));
    assert!(matches!(e, EnrichError::Upstream(_)), "{e:?}");
}

#[test]
fn slow_treg_times_out() {
    let s = Server::start(|req: &Req| {
        if req.method == "GET" {
            return happy_treg(req);
        }
        Resp::json(200, fixture("call_email_find_hit.json")).delayed(Duration::from_millis(1500))
    });
    let e = TregEnricher::new(&s.base, Some(TOKEN.to_string()), Duration::from_millis(200));
    let err = e.enrich(&job(find_email())).unwrap_err();
    assert!(matches!(err, EnrichError::Timeout), "{err:?}");
}

#[test]
fn unreachable_treg_is_upstream_error() {
    // Bind and drop to get a port nobody listens on.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let e = enricher(&format!("http://127.0.0.1:{port}"));
    assert!(matches!(
        e.enrich(&job(find_email())),
        Err(EnrichError::Upstream(_) | EnrichError::Timeout)
    ));
    assert!(matches!(
        e.quote(&[job(find_email())]),
        Err(EnrichError::Upstream(_) | EnrichError::Timeout)
    ));
}

#[test]
fn catalog_without_the_tool_is_unsupported() {
    let s = Server::start(|_req: &Req| Resp::json(404, r#"{"detail":"not found"}"#));
    let e = enricher(&s.base);
    assert!(matches!(
        e.quote(&[job(find_email())]),
        Err(EnrichError::Unsupported(_))
    ));
}

#[test]
fn missing_token_fails_before_any_call() {
    let s = Server::start(happy_treg);
    let e = TregEnricher::new(&s.base, None, Duration::from_secs(5));
    // Quoting needs no token.
    e.quote(&[job(find_email())]).unwrap();
    let err = e.enrich(&job(find_email())).unwrap_err();
    assert!(matches!(err, EnrichError::Auth(_)), "{err:?}");
    assert!(err.to_string().contains("OPENSTANCE_TREG_TOKEN"));
    // The gate learns this before it marks the request sent.
    let pre = e.preflight(&job(find_email())).unwrap_err();
    assert!(matches!(pre, EnrichError::Auth(_)), "{pre:?}");
    assert!(s.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn preflight_with_a_token_and_enough_input_passes_without_network() {
    let s = Server::start(happy_treg);
    let e = enricher(&s.base);
    e.preflight(&job(find_email())).unwrap();
    assert!(s.requests().is_empty());
}

#[test]
fn from_env_reads_named_variable_and_endpoint_trailing_slash_is_ok() {
    let s = Server::start(happy_treg);
    let var = "OPENSTANCE_TEST_TREG_TOKEN_FROM_ENV";
    std::env::set_var(var, TOKEN);
    let e = TregEnricher::from_env(&format!("{}/", s.base), var, Duration::from_secs(5));
    std::env::remove_var(var);
    e.enrich(&job(find_email())).unwrap();
    let call = s.requests().pop().unwrap();
    assert_eq!(call.path, "/call/treg.people.email.find");
    assert_eq!(call.header("x-treg-token"), Some(TOKEN));
}

#[test]
fn default_endpoint_is_treg_to() {
    assert_eq!(openstance_enrich::treg::DEFAULT_ENDPOINT, "https://treg.to");
    assert_eq!(
        openstance_enrich::treg::DEFAULT_TOKEN_ENV,
        "OPENSTANCE_TREG_TOKEN"
    );
}

// ---------------------------------------------------------------- secrecy

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);
impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// One global subscriber for this test binary. A thread-local one is not
/// reliable here: callsites hit first by a parallel test cache "no interest".
fn captured_tracing() -> Buf {
    static BUF: std::sync::OnceLock<Buf> = std::sync::OnceLock::new();
    BUF.get_or_init(|| {
        let buf = Buf::default();
        let w = buf.clone();
        let sub = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || w.clone())
            .finish();
        tracing::subscriber::set_global_default(sub).unwrap();
        buf
    })
    .clone()
}

#[test]
fn token_never_appears_in_tracing_output_debug_or_errors() {
    let buf = captured_tracing();
    let s = Server::start(|req: &Req| {
        if req.path.ends_with("verify") {
            return Resp::json(401, r#"{"detail":"invalid token"}"#);
        }
        happy_treg(req)
    });
    let e = enricher(&s.base);
    e.quote(&[job(find_email())]).unwrap();
    e.enrich(&job(find_email())).unwrap();
    let err = e
        .enrich(&job(EnrichRequest::VerifyEmail {
            email: "a@b.c".into(),
        }))
        .unwrap_err();
    assert!(!err.to_string().contains(TOKEN));
    assert!(!format!("{err:?}").contains(TOKEN));
    assert!(!format!("{e:?}").contains(TOKEN));
    let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(
        out.contains("treg.people.email.find"),
        "tracing captured: {out}"
    );
    assert!(!out.contains(TOKEN), "token leaked into tracing output");
}
