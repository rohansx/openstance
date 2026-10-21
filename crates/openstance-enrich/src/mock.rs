//! `MockEnricher`: a scriptable, recording backend for tests (P3-ENR-03).
//!
//! It never touches the network. It records every `quote` and `enrich` call
//! in order, answers from canned responses (usually `fixtures/enrich/*.json`),
//! and prices each op from a configurable table. A request with no canned
//! response returns [`EnrichData::NotFound`] and still costs its quoted price,
//! like a real miss.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Deserialize;

use crate::{
    EnrichData, EnrichError, EnrichJob, EnrichRequest, EnrichResult, Enricher, MicroUsd,
    QuoteBasis, QuoteLine,
};

/// Every op the mock knows.
pub const ALL_OPS: [&str; 5] = [
    "profile",
    "find_email",
    "verify_email",
    "company",
    "web_search",
];

/// Default price per call: $0.01.
pub const DEFAULT_PRICE_MICRO_USD: MicroUsd = 10_000;

/// The repo's `fixtures/enrich` directory.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/enrich")
}

/// One canned answer.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct MockResponse {
    /// Payload.
    pub data: EnrichData,
    /// Upstream provider reported with the result.
    #[serde(default)]
    pub provider: Option<String>,
    /// Actual cost; defaults to the quoted price for the op.
    #[serde(default)]
    pub cost_micro_usd: Option<MicroUsd>,
}

#[derive(Deserialize)]
struct FixtureEntry {
    request: EnrichRequest,
    #[serde(flatten)]
    response: MockResponse,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FixtureFile {
    Many(Vec<FixtureEntry>),
    One(Box<FixtureEntry>),
}

/// Something the mock was asked to do, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum MockEvent {
    /// `quote` with these jobs.
    Quote(Vec<EnrichJob>),
    /// `enrich` with this job (recorded even if it failed).
    Enrich(EnrichJob),
}

/// A fixture file that could not be read or parsed.
#[derive(Debug, thiserror::Error)]
#[error("enrich fixture {path}: {message}")]
pub struct FixtureError {
    /// The offending file or directory.
    pub path: PathBuf,
    /// What went wrong.
    pub message: String,
}

type ErrFn = fn() -> EnrichError;

/// See the module docs.
pub struct MockEnricher {
    prices: HashMap<String, MicroUsd>,
    basis: QuoteBasis,
    provider_hint: Option<String>,
    supported: HashSet<String>,
    responses: HashMap<EnrichRequest, MockResponse>,
    fail_on: HashMap<EnrichRequest, ErrFn>,
    fail_before_send: HashMap<EnrichRequest, ErrFn>,
    quote_error: Option<ErrFn>,
    fetched_at: Option<i64>,
    events: Mutex<Vec<MockEvent>>,
}

impl Default for MockEnricher {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MockEnricher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockEnricher")
            .field("prices", &self.prices)
            .field("responses", &self.responses.len())
            .field("events", &self.events().len())
            .finish()
    }
}

impl MockEnricher {
    /// Supports every op at $0.01 per call, with no canned responses.
    pub fn new() -> Self {
        MockEnricher {
            prices: ALL_OPS
                .iter()
                .map(|op| (op.to_string(), DEFAULT_PRICE_MICRO_USD))
                .collect(),
            basis: QuoteBasis::Exact,
            provider_hint: None,
            supported: ALL_OPS.iter().map(|s| s.to_string()).collect(),
            responses: HashMap::new(),
            fail_on: HashMap::new(),
            fail_before_send: HashMap::new(),
            quote_error: None,
            fetched_at: None,
            events: Mutex::new(Vec::new()),
        }
    }

    /// [`MockEnricher::new`] loaded with the repo's `fixtures/enrich/*.json`.
    pub fn from_fixtures() -> Result<Self, FixtureError> {
        Self::from_dir(fixtures_dir())
    }

    /// [`MockEnricher::new`] loaded with every `*.json` file in `dir`
    /// (not recursive). Each file holds one entry or an array of entries:
    /// `{"request": EnrichRequest, "data": EnrichData, "provider"?, "cost_micro_usd"?}`.
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Self, FixtureError> {
        let dir = dir.as_ref();
        let err = |path: &Path, message: String| FixtureError {
            path: path.to_path_buf(),
            message,
        };
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| err(dir, e.to_string()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        let mut m = Self::new();
        for path in files {
            let text = std::fs::read_to_string(&path).map_err(|e| err(&path, e.to_string()))?;
            let parsed: FixtureFile =
                serde_json::from_str(&text).map_err(|e| err(&path, e.to_string()))?;
            let entries = match parsed {
                FixtureFile::Many(v) => v,
                FixtureFile::One(e) => vec![*e],
            };
            for e in entries {
                m.responses.insert(e.request, e.response);
            }
        }
        Ok(m)
    }

    /// Price one op.
    pub fn with_price(mut self, op: &str, micro_usd: MicroUsd) -> Self {
        self.prices.insert(op.to_string(), micro_usd);
        self
    }

    /// Basis reported on every quote line (default `exact`).
    pub fn with_basis(mut self, basis: QuoteBasis) -> Self {
        self.basis = basis;
        self
    }

    /// `provider_hint` reported on every quote line.
    pub fn with_provider_hint(mut self, hint: &str) -> Self {
        self.provider_hint = Some(hint.to_string());
        self
    }

    /// Stop supporting `op`.
    pub fn without_op(mut self, op: &str) -> Self {
        self.supported.remove(op);
        self
    }

    /// Canned payload for an exact request.
    pub fn with_response(self, request: EnrichRequest, data: EnrichData) -> Self {
        self.with_mock_response(
            request,
            MockResponse {
                data,
                provider: None,
                cost_micro_usd: None,
            },
        )
    }

    /// Canned response (payload, provider, cost) for an exact request.
    pub fn with_mock_response(mut self, request: EnrichRequest, response: MockResponse) -> Self {
        self.responses.insert(request, response);
        self
    }

    /// Make `enrich(request)` fail with `Upstream`.
    pub fn fail_on(self, request: EnrichRequest) -> Self {
        self.fail_with(request, || EnrichError::Upstream("mock failure".into()))
    }

    /// Make `enrich(request)` fail with the given error.
    pub fn fail_with(mut self, request: EnrichRequest, err: fn() -> EnrichError) -> Self {
        self.fail_on.insert(request, err);
        self
    }

    /// Make `preflight(request)` fail with the given error: the backend
    /// refuses before anything would leave the machine (no token, not
    /// enough input), so `enrich` is never called for it.
    pub fn fail_before_send(mut self, request: EnrichRequest, err: fn() -> EnrichError) -> Self {
        self.fail_before_send.insert(request, err);
        self
    }

    /// Make every `quote` fail with the given error.
    pub fn quote_fails_with(mut self, err: fn() -> EnrichError) -> Self {
        self.quote_error = Some(err);
        self
    }

    /// Fixed `fetched_at` for results (default: the current time).
    pub fn with_fetched_at(mut self, unix_secs: i64) -> Self {
        self.fetched_at = Some(unix_secs);
        self
    }

    /// Every `enrich` call, in order.
    pub fn calls(&self) -> Vec<EnrichJob> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                MockEvent::Enrich(j) => Some(j),
                MockEvent::Quote(_) => None,
            })
            .collect()
    }

    /// Every `quote` call's jobs, in order.
    pub fn quotes(&self) -> Vec<Vec<EnrichJob>> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                MockEvent::Quote(j) => Some(j),
                MockEvent::Enrich(_) => None,
            })
            .collect()
    }

    /// Every quote and enrich call, interleaved in order.
    pub fn events(&self) -> Vec<MockEvent> {
        self.lock().clone()
    }

    /// Forget recorded calls.
    pub fn reset(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<MockEvent>> {
        // A panicking test thread must not hide the record from the others.
        self.events.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn price(&self, op: &str) -> MicroUsd {
        self.prices.get(op).copied().unwrap_or(0)
    }
}

impl Enricher for MockEnricher {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn supports(&self, op: &str) -> bool {
        self.supported.contains(op)
    }

    fn quote(&self, jobs: &[EnrichJob]) -> Result<Vec<QuoteLine>, EnrichError> {
        self.lock().push(MockEvent::Quote(jobs.to_vec()));
        if let Some(f) = self.quote_error {
            return Err(f());
        }
        jobs.iter()
            .map(|job| {
                let op = job.request.op();
                if !self.supports(op) {
                    return Err(EnrichError::Unsupported(op.to_string()));
                }
                Ok(QuoteLine {
                    job: job.clone(),
                    micro_usd: self.price(op),
                    basis: self.basis,
                    provider_hint: self.provider_hint.clone(),
                })
            })
            .collect()
    }

    fn preflight(&self, job: &EnrichJob) -> Result<(), EnrichError> {
        match self.fail_before_send.get(&job.request) {
            Some(f) => Err(f()),
            None => Ok(()),
        }
    }

    fn enrich(&self, job: &EnrichJob) -> Result<EnrichResult, EnrichError> {
        self.lock().push(MockEvent::Enrich(job.clone()));
        let op = job.request.op();
        if !self.supports(op) {
            return Err(EnrichError::Unsupported(op.to_string()));
        }
        if let Some(f) = self.fail_on.get(&job.request) {
            return Err(f());
        }
        let (data, provider, cost) = match self.responses.get(&job.request) {
            Some(r) => (r.data.clone(), r.provider.clone(), r.cost_micro_usd),
            None => (EnrichData::NotFound, None, None),
        };
        Ok(EnrichResult {
            job: job.clone(),
            cost_micro_usd: cost.unwrap_or_else(|| self.price(op)),
            provider,
            data,
            fetched_at: self.fetched_at.unwrap_or_else(crate::unix_now),
        })
    }
}
