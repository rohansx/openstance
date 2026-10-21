//! Tool handlers: tool arguments -> core request -> the shared JSON shape.
//!
//! Synchronous and runtime-free: the server calls [`call`] on the blocking
//! pool. No business logic lives here; each tool maps onto the same core
//! function the CLI command uses, so `openstance search --json` and
//! `search_people` return the same document for the same input and `now`.

use openstance_core::graphq::{self, HowOptions};
use openstance_core::json::ErrorResponse;
use openstance_core::resolve::lookup::PersonKey;
use openstance_core::search::person::DetailOptions;
use openstance_core::search::rules::parse_date_arg;
use openstance_core::search::{self, SearchRequest};
use openstance_core::{Config, Error, Graph, QueryPatch};
use rmcp::model::JsonObject;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;

use crate::schema::{GET_PERSON, HOW_CONNECTED, SEARCH_PEOPLE};
use crate::text;

/// What a tool call produced: the JSON shape and its plain-text rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    /// `structuredContent`: the shared JSON shape, or `{"v":1,"error":{...}}`.
    pub structured: Value,
    /// Short plain-text summary for clients that ignore structured output.
    pub text: String,
    /// The call failed (`structured` is an error envelope).
    pub is_error: bool,
}

impl ToolOutput {
    fn ok(structured: Value, text: String) -> Self {
        ToolOutput {
            structured,
            text,
            is_error: false,
        }
    }

    /// The error envelope of `e` (interfaces.md 1.11).
    pub fn error(e: &Error) -> Self {
        Self::from_response(ErrorResponse::from(e))
    }

    /// The `internal` envelope (a bug; details are in the log).
    pub fn internal(message: &str) -> Self {
        Self::error(&Error::Internal(message.to_string()))
    }

    fn from_response(resp: ErrorResponse) -> Self {
        let text = text::error(&resp);
        ToolOutput {
            structured: serde_json::to_value(&resp).unwrap_or(Value::Null),
            text,
            is_error: true,
        }
    }
}

/// Run tool `name`. `None` when no such tool is registered.
pub fn call(graph: &Graph, name: &str, args: Option<JsonObject>) -> Option<ToolOutput> {
    let args = Value::Object(args.unwrap_or_default());
    let out = match name {
        SEARCH_PEOPLE => search_people(graph, args),
        GET_PERSON => get_person(graph, args),
        HOW_CONNECTED => how_connected(graph, args),
        _ => return None,
    };
    Some(out.unwrap_or_else(|e| ToolOutput::error(&e)))
}

fn parse_args<T: DeserializeOwned>(tool: &str, args: Value) -> Result<T, Error> {
    serde_json::from_value(args).map_err(|e| Error::BadRequest(format!("{tool}: {e}")))
}

fn bad(msg: String) -> Error {
    Error::BadRequest(msg)
}

/// An integer argument inside `[min, max]`, or `default`.
fn int_in(
    tool: &str,
    field: &str,
    v: Option<i64>,
    min: i64,
    max: i64,
    default: usize,
) -> Result<usize, Error> {
    match v {
        None => Ok(default),
        Some(n) if (min..=max).contains(&n) => Ok(usize::try_from(n).unwrap_or(default)),
        Some(n) => Err(bad(format!(
            "{tool}: {field} must be between {min} and {max} (got {n})"
        ))),
    }
}

fn to_value<T: serde::Serialize>(v: &T) -> Result<Value, Error> {
    Ok(serde_json::to_value(v)?)
}

// ---------------------------------------------------------------------------
// search_people
// ---------------------------------------------------------------------------

/// `search_people` arguments (interfaces.md 2.2).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchPeopleArgs {
    /// Free text.
    pub text: Option<String>,
    /// Companies (OR-ed).
    pub company: Option<Vec<String>>,
    /// Titles (OR-ed).
    pub title: Option<Vec<String>>,
    /// Locations (OR-ed).
    pub location: Option<Vec<String>>,
    /// Industries (soft).
    pub industry: Option<Vec<String>>,
    /// Tags (exact).
    pub tags: Option<Vec<String>>,
    /// Window start.
    pub since: Option<String>,
    /// Window end.
    pub until: Option<String>,
    /// Last contact strictly before.
    pub last_contact_before: Option<String>,
    /// Has a meeting.
    pub met_in_person: Option<bool>,
    /// They replied.
    pub replied: Option<bool>,
    /// Minimum relationship score.
    pub min_score: Option<f64>,
    /// High frequency, low recency.
    pub lost_touch: Option<bool>,
    /// Include bulk senders.
    pub include_bulk: Option<bool>,
    /// Maximum results.
    pub limit: Option<i64>,
    /// Run the query parser on `text` (default true).
    pub parse: Option<bool>,
    /// Fill `timings_ms`.
    pub explain: Option<bool>,
}

impl SearchPeopleArgs {
    /// The core request, built the way `openstance search` builds it from
    /// the equivalent flags (so the two surfaces agree byte for byte).
    pub fn to_request(&self, cfg: &Config, now: i64) -> Result<SearchRequest, Error> {
        let tz = search::timezone(cfg);
        let today = search::today_start(now, tz);
        let list = |v: &Option<Vec<String>>| v.clone().filter(|v| !v.is_empty());
        let date = |field: &str, v: &Option<String>| -> Result<Option<i64>, Error> {
            match v {
                None => Ok(None),
                Some(s) => parse_date_arg(s, today, tz).map(Some).ok_or_else(|| {
                    bad(format!(
                        "{SEARCH_PEOPLE}: {field}: could not parse date '{s}' (use YYYY, YYYY-MM, YYYY-MM-DD, 90d, 6m or 1y)"
                    ))
                }),
            }
        };
        let min_score = match self.min_score {
            Some(m) if !(0.0..=1.0).contains(&m) => {
                return Err(bad(format!(
                    "{SEARCH_PEOPLE}: min_score must be between 0 and 1 (got {m})"
                )))
            }
            #[allow(clippy::cast_possible_truncation)]
            m => m.map(|m| m as f32),
        };
        let default_limit = usize::try_from(cfg.mcp.default_limit).unwrap_or(20);
        let limit = int_in(SEARCH_PEOPLE, "limit", self.limit, 1, 100, default_limit)?;
        let filters = QueryPatch {
            company: list(&self.company),
            title: list(&self.title),
            location: list(&self.location),
            industry: list(&self.industry),
            tags: list(&self.tags),
            since: date("since", &self.since)?,
            until: date("until", &self.until)?,
            last_contact_before: date("last_contact_before", &self.last_contact_before)?,
            met_in_person: self.met_in_person,
            replied: self.replied,
            min_score,
            lost_touch: self.lost_touch,
            // The CLI only has `--include-bulk`; `false` is the default.
            include_bulk: self.include_bulk.filter(|b| *b),
            limit: Some(limit),
            ..QueryPatch::default()
        };
        Ok(SearchRequest {
            text: self.text.clone(),
            filters,
            no_parse: !self.parse.unwrap_or(true),
            explain: self.explain.unwrap_or(false),
            today,
            now,
        })
    }
}

fn search_people(graph: &Graph, args: Value) -> Result<ToolOutput, Error> {
    let a: SearchPeopleArgs = parse_args(SEARCH_PEOPLE, args)?;
    let req = a.to_request(graph.config(), graph.now())?;
    let resp = graph.search(&req)?;
    let v = to_value(&resp)?;
    let t = text::search(&v);
    Ok(ToolOutput::ok(v, t))
}

// ---------------------------------------------------------------------------
// get_person
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetPersonArgs {
    person: String,
    interactions: Option<i64>,
    mutuals: Option<i64>,
}

fn get_person(graph: &Graph, args: Value) -> Result<ToolOutput, Error> {
    let a: GetPersonArgs = parse_args(GET_PERSON, args)?;
    let defaults = DetailOptions::default();
    let opts = DetailOptions {
        interactions: int_in(
            GET_PERSON,
            "interactions",
            a.interactions,
            0,
            50,
            defaults.interactions,
        )?,
        mutuals: int_in(GET_PERSON, "mutuals", a.mutuals, 0, 50, defaults.mutuals)?,
    };
    let key = PersonKey::parse(&a.person);
    let d = graphq::get_person(graph.conn(), graph.config(), &key, opts, graph.now())?;
    let v = to_value(&d)?;
    let t = text::person(&v);
    Ok(ToolOutput::ok(v, t))
}

// ---------------------------------------------------------------------------
// how_connected
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HowConnectedArgs {
    target: String,
    limit: Option<i64>,
}

fn how_connected(graph: &Graph, args: Value) -> Result<ToolOutput, Error> {
    let a: HowConnectedArgs = parse_args(HOW_CONNECTED, args)?;
    let limit = int_in(
        HOW_CONNECTED,
        "limit",
        a.limit,
        1,
        10,
        HowOptions::default().limit,
    )?;
    let key = PersonKey::parse(&a.target);
    let r = graphq::how_connected(graph.conn(), &key, HowOptions { limit }, graph.now())?;
    let v = to_value(&r)?;
    let t = text::how(&v);
    Ok(ToolOutput::ok(v, t))
}
