//! Tool handlers: tool arguments -> core request -> the shared JSON shape.
//!
//! Synchronous and runtime-free: the server calls [`call`] on the blocking
//! pool. No business logic lives here; each tool maps onto the same core
//! function the CLI command uses, so `openstance search --json` and
//! `search_people` return the same document for the same input and `now`.

use openstance_core::egress::quote::quote_json;
use openstance_core::egress::Surface;
use openstance_core::enrich::{EnrichJob, EnrichRequest};
use openstance_core::graphq::{self, HowOptions};
use openstance_core::json::{ErrorResponse, V};
use openstance_core::playbook::{Outcome, RunOptions};
use openstance_core::resolve::lookup::PersonKey;
use openstance_core::search::person::DetailOptions;
use openstance_core::search::rules::parse_date_arg;
use openstance_core::search::{self, SearchRequest};
use openstance_core::view::inbox::{self, ViewQuery};
use openstance_core::view::{Direction, ViewList, ViewOwner, ViewSearchResponse};
use openstance_core::{Config, Error, Graph, QueryPatch};
use rmcp::model::JsonObject;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;

use crate::schema::{
    BRIEF, GET_PERSON, HOW_CONNECTED, LIST_VIEWS, PATH_TO, QUOTE_ENRICH, SEARCH_PEOPLE, SEARCH_VIEW,
};
use crate::text;
use crate::ViewsContext;

/// What the tools need besides the graph.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolCtx {
    /// Imported views and my owner identity (`None`: own views only).
    pub views: Option<ViewsContext>,
}

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

/// Run tool `name` without a views context. `None` when no such tool is
/// registered.
pub fn call(graph: &Graph, name: &str, args: Option<JsonObject>) -> Option<ToolOutput> {
    call_with(graph, &ToolCtx::default(), name, args)
}

/// Run tool `name`. `None` when no such tool is registered.
pub fn call_with(
    graph: &Graph,
    ctx: &ToolCtx,
    name: &str,
    args: Option<JsonObject>,
) -> Option<ToolOutput> {
    let args = Value::Object(args.unwrap_or_default());
    let out = match name {
        SEARCH_PEOPLE => search_people(graph, args),
        GET_PERSON => get_person(graph, args),
        HOW_CONNECTED => how_connected(graph, args),
        PATH_TO => path_to(graph, args),
        BRIEF => brief(graph, args),
        LIST_VIEWS => list_views(graph, ctx, args),
        SEARCH_VIEW => search_view(graph, ctx, args),
        QUOTE_ENRICH => quote_enrich(graph, args),
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

// ---------------------------------------------------------------------------
// Egress arguments shared by path_to and brief (interfaces.md 2, egress rule)
// ---------------------------------------------------------------------------

/// `max_usd` must be a finite, non-negative number.
fn max_usd(tool: &str, v: Option<f64>) -> Result<Option<f64>, Error> {
    match v {
        Some(m) if !m.is_finite() || m < 0.0 => Err(bad(format!(
            "{tool}: max_usd must be a number >= 0 (got {m})"
        ))),
        m => Ok(m),
    }
}

/// The run options of an agent call. `allow_egress` defaults to
/// `enrich.allow_egress_default` (false) and a read-only server never
/// allows it. `max_usd` can only lower the configured budget
/// ([`Surface::Agent`]).
fn agent_run(
    graph: &Graph,
    tool: &str,
    allow_egress: Option<bool>,
    max: Option<f64>,
    plan_only: bool,
) -> Result<RunOptions, Error> {
    let cfg = graph.config();
    let allow = allow_egress.unwrap_or(cfg.enrich.allow_egress_default) && !cfg.mcp.read_only;
    Ok(RunOptions {
        allow_egress: allow,
        max_usd: max_usd(tool, max)?,
        surface: Surface::Agent,
        plan_only,
        run_id: None,
    })
}

fn outcome(out: &Outcome, render: fn(&Value) -> String) -> Result<ToolOutput, Error> {
    let v = to_value(out)?;
    let t = render(&v);
    Ok(ToolOutput::ok(v, t))
}

fn non_blank(tool: &str, field: &str, s: &str) -> Result<(), Error> {
    if s.trim().is_empty() {
        return Err(bad(format!("{tool}: {field} must not be empty")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// path_to (P3-MCP-01)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathToArgs {
    target: String,
    company: Option<String>,
    linkedin: Option<String>,
    find_email: Option<bool>,
    limit: Option<i64>,
    allow_egress: Option<bool>,
    max_usd: Option<f64>,
    plan_only: Option<bool>,
}

fn path_to(graph: &Graph, args: Value) -> Result<ToolOutput, Error> {
    let a: PathToArgs = parse_args(PATH_TO, args)?;
    non_blank(PATH_TO, "target", &a.target)?;
    let limit = int_in(PATH_TO, "limit", a.limit, 1, 10, 3)?;
    let opts = agent_run(
        graph,
        PATH_TO,
        a.allow_egress,
        a.max_usd,
        a.plan_only.unwrap_or(false),
    )?;
    // The same input object `openstance path` builds from its flags.
    let input = serde_json::json!({
        "target": a.target,
        "company": a.company,
        "linkedin": a.linkedin,
        "find_email": a.find_email.unwrap_or(false),
        "limit": limit,
    });
    let out = graph.run_playbook("path", &input, &opts)?;
    outcome(&out, text::path_to)
}

// ---------------------------------------------------------------------------
// brief (P3-MCP-02)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BriefArgs {
    person: String,
    web: Option<bool>,
    allow_egress: Option<bool>,
    max_usd: Option<f64>,
}

fn brief(graph: &Graph, args: Value) -> Result<ToolOutput, Error> {
    let a: BriefArgs = parse_args(BRIEF, args)?;
    non_blank(BRIEF, "person", &a.person)?;
    let opts = agent_run(graph, BRIEF, a.allow_egress, a.max_usd, false)?;
    let input = serde_json::json!({ "person": a.person, "web": a.web.unwrap_or(false) });
    let out = graph.run_playbook("brief", &input, &opts)?;
    outcome(&out, text::brief)
}

// ---------------------------------------------------------------------------
// quote_enrich (P3-MCP-03)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuoteEnrichArgs {
    requests: Vec<QuoteRequestArg>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuoteRequestArg {
    kind: String,
    person_id: Option<i64>,
    name: Option<String>,
    company: Option<String>,
    linkedin: Option<String>,
    domain: Option<String>,
    email: Option<String>,
    query: Option<String>,
}

/// Most requests per `quote_enrich` call (the schema's `maxItems`).
const MAX_QUOTE_REQUESTS: usize = 500;

fn given(v: &Option<String>) -> Option<String> {
    v.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl QuoteRequestArg {
    /// The enrich job, with `name` (and `company` for a profile) filled from
    /// the graph when only `person_id` was given.
    fn job(&self, graph: &Graph, i: usize) -> Result<EnrichJob, Error> {
        let need = |field: &str, v: Option<String>| {
            v.ok_or_else(|| {
                bad(format!(
                    "{QUOTE_ENRICH}: requests[{i}] ({}) needs {field}",
                    self.kind
                ))
            })
        };
        let person = match self.person_id {
            Some(id) => {
                let key = PersonKey::Id(id);
                let d = graphq::get_person(
                    graph.conn(),
                    graph.config(),
                    &key,
                    DetailOptions {
                        interactions: 0,
                        mutuals: 0,
                    },
                    graph.now(),
                )?;
                Some(d)
            }
            None => None,
        };
        let anchor = person.as_ref().map(|d| d.person.id).or(self.person_id);
        let pname = person.as_ref().and_then(|d| d.person.display_name.clone());
        let pcompany = person.as_ref().and_then(|d| d.person.company.clone());
        let request = match self.kind.as_str() {
            "profile" => EnrichRequest::Profile {
                name: need("name", given(&self.name).or(pname))?,
                company: given(&self.company).or(pcompany),
                linkedin: given(&self.linkedin),
            },
            "find_email" => EnrichRequest::FindEmail {
                name: need("name", given(&self.name).or(pname))?,
                domain: need("domain", given(&self.domain))?,
            },
            "verify_email" => EnrichRequest::VerifyEmail {
                email: need("email", given(&self.email))?,
            },
            "company" => EnrichRequest::Company {
                domain: need("domain", given(&self.domain))?,
            },
            "web_search" => EnrichRequest::WebSearch {
                query: need("query", given(&self.query))?,
            },
            other => {
                return Err(bad(format!(
                    "{QUOTE_ENRICH}: requests[{i}].kind {other:?} is not one of profile, find_email, verify_email, company, web_search"
                )))
            }
        };
        Ok(EnrichJob {
            person_id: anchor,
            request,
        })
    }
}

fn quote_enrich(graph: &Graph, args: Value) -> Result<ToolOutput, Error> {
    let a: QuoteEnrichArgs = parse_args(QUOTE_ENRICH, args)?;
    if a.requests.is_empty() || a.requests.len() > MAX_QUOTE_REQUESTS {
        return Err(bad(format!(
            "{QUOTE_ENRICH}: requests must hold 1 to {MAX_QUOTE_REQUESTS} items (got {})",
            a.requests.len()
        )));
    }
    let jobs = a
        .requests
        .iter()
        .enumerate()
        .map(|(i, r)| r.job(graph, i))
        .collect::<Result<Vec<_>, _>>()?;
    let q = graph.quote_enrich(jobs, None, Surface::Agent)?;
    let v = quote_json(&q)?;
    let t = text::quote(&v);
    Ok(ToolOutput::ok(v, t))
}

// ---------------------------------------------------------------------------
// list_views / search_view (P3-MCP-04)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListViewsArgs {
    direction: Option<String>,
}

fn list_views(graph: &Graph, ctx: &ToolCtx, args: Value) -> Result<ToolOutput, Error> {
    let a: ListViewsArgs = parse_args(LIST_VIEWS, args)?;
    let dir = match a.direction.as_deref() {
        None => Direction::All,
        Some(d) => Direction::parse(d).ok_or_else(|| {
            bad(format!(
                "{LIST_VIEWS}: direction must be own, imported or all (got {d:?})"
            ))
        })?,
    };
    let mut views = Vec::new();
    if dir != Direction::Imported {
        let owner = ctx.views.as_ref().map_or(
            ViewOwner {
                pubkey: None,
                label: None,
            },
            |v| v.owner.clone(),
        );
        views.extend(graph.view_summaries(&owner)?);
    }
    if dir != Direction::Own {
        if let Some(v) = &ctx.views {
            views.extend(inbox::list(&v.dir, graph.now())?);
        }
    }
    let v = to_value(&ViewList { v: V, views })?;
    let t = text::views(&v);
    Ok(ToolOutput::ok(v, t))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchViewArgs {
    view: String,
    text: Option<String>,
    company: Option<Vec<String>>,
    title: Option<Vec<String>>,
    limit: Option<i64>,
}

fn search_view(graph: &Graph, ctx: &ToolCtx, args: Value) -> Result<ToolOutput, Error> {
    let a: SearchViewArgs = parse_args(SEARCH_VIEW, args)?;
    non_blank(SEARCH_VIEW, "view", &a.view)?;
    let limit = int_in(SEARCH_VIEW, "limit", a.limit, 1, 100, 20)?;
    let resp = match &ctx.views {
        Some(v) => inbox::search(
            &v.dir,
            &ViewQuery {
                view: a.view,
                text: a.text,
                company: a.company.unwrap_or_default(),
                title: a.title.unwrap_or_default(),
                limit: Some(limit),
            },
            graph.now(),
        )?,
        None if a.view.trim() == "*" => ViewSearchResponse {
            v: V,
            results: Vec::new(),
        },
        None => return Err(Error::NotFound(format!("imported view {:?}", a.view))),
    };
    let v = to_value(&resp)?;
    let t = text::view_search(&v);
    Ok(ToolOutput::ok(v, t))
}
