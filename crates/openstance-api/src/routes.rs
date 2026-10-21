//! Route handlers (interfaces.md section 3.1). Each one parses its input,
//! runs one core call on the pool, and returns the shared JSON shape. Where
//! an MCP tool exists for the operation the handler calls the tool itself
//! (`openstance_mcp::tools`), so HTTP, MCP and CLI agree byte for byte.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{FromRequest, Multipart, Path, Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::Router;
use openstance_core::egress::log::LogFilter;
use openstance_core::egress::Surface;
use openstance_core::ingest::ImportRequest;
use openstance_core::json::V;
use openstance_core::playbook::RunOptions;
use openstance_core::resolve::lookup::{resolve_person, PersonKey};
use openstance_core::search::rules::{parse_date_arg, Lexicon, RuleParser};
use openstance_core::search::{self};
use openstance_core::view::inbox;
use openstance_core::view::{self as views, CreateView, Ttl, ViewOwner};
use openstance_core::workspace::edits::{EditsRequest, PersonArg};
use openstance_core::workspace::{IntroEdit, IntroRequest, Stage};
use openstance_core::{Graph, Query as CoreQuery};
use openstance_mcp::tools::{SearchPeopleArgs, ToolCtx};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{ok_json, ApiError};
use crate::server::{AppState, St};

/// The API routes (all behind the bearer token).
pub fn router(max_upload: usize) -> Router<Arc<AppState>> {
    use axum::extract::DefaultBodyLimit;
    Router::new()
        .route("/people", get(people))
        .route("/search", post(search_post))
        .route("/people/{id}", get(person))
        .route("/people/{id}/ties/{other}", get(ties))
        .route("/connections", get(connections))
        .route("/playbooks/{name}", post(playbook))
        .route("/quote", post(quote))
        .route("/edits", post(edits))
        .route("/review", get(review))
        .route("/views", get(views_list).post(views_create))
        .route("/views/{name}/search", get(views_search))
        .route("/views/{name}/export", post(views_export))
        .route(
            "/views/import",
            post(views_import).layer(DefaultBodyLimit::max(max_upload)),
        )
        .route(
            "/import",
            post(import).layer(DefaultBodyLimit::max(max_upload)),
        )
        .route("/sources", get(sources))
        .route("/enrich-log", get(enrich_log))
        .route("/config", get(config))
        .route("/stats", get(stats))
        .route("/shortlist", get(shortlist).post(shortlist_add))
        .route(
            "/shortlist/{person}",
            patch(shortlist_patch).delete(shortlist_remove),
        )
        .route("/intros", get(intros).post(intro_create))
        .route("/intros/{id}", patch(intro_patch).delete(intro_delete))
}

type Res = Result<Response, ApiError>;

// ---------------------------------------------------------------------------
// Extractors
// ---------------------------------------------------------------------------

/// A JSON body whose rejections use the error envelope.
pub struct Body<T>(pub T);

impl<S, T> FromRequest<S> for Body<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|e| {
                if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::too_large("request body is too large")
                } else {
                    ApiError::bad_request(format!("could not read the body: {e}"))
                }
            })?;
        serde_json::from_slice(&bytes)
            .map(Body)
            .map_err(|e| ApiError::bad_request(format!("invalid JSON body: {e}")))
    }
}

/// Query pairs in order (repeat a key for arrays).
type Pairs = Query<Vec<(String, String)>>;

fn parse_bool(key: &str, v: &str) -> Result<bool, ApiError> {
    match v.trim() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ApiError::bad_request(format!(
            "{key} must be true or false (got {v:?})"
        ))),
    }
}

fn parse_num<T: std::str::FromStr>(key: &str, v: &str) -> Result<T, ApiError> {
    v.trim()
        .parse()
        .map_err(|_| ApiError::bad_request(format!("{key} must be a number (got {v:?})")))
}

fn only_keys(pairs: &[(String, String)], allowed: &[&str]) -> Result<(), ApiError> {
    for (k, _) in pairs {
        if !allowed.contains(&k.as_str()) {
            return Err(ApiError::bad_request(format!(
                "unknown query parameter {k:?}; expected one of: {}",
                allowed.join(", ")
            )));
        }
    }
    Ok(())
}

fn first<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// The structured output of MCP tool `name`, or its error as an HTTP error.
fn tool(g: &Graph, ctx: &ToolCtx, name: &str, args: Value) -> Result<Value, ApiError> {
    let out = openstance_mcp::tools::call_with(g, ctx, name, args.as_object().cloned())
        .ok_or_else(|| ApiError::internal(format!("no tool {name}")))?;
    if out.is_error {
        return Err(ApiError::from_envelope(out.structured));
    }
    Ok(out.structured)
}

fn tool_ctx(st: &AppState) -> ToolCtx {
    ToolCtx {
        views: st
            .opts
            .share
            .as_ref()
            .map(|s| openstance_mcp::ViewsContext {
                dir: s.views.clone(),
                owner: ViewOwner {
                    pubkey: openstance_core::keys::load(&s.key).ok().map(|k| k.public()),
                    label: st.opts.owner_label.clone(),
                },
            }),
    }
}

fn json_ok(v: Value) -> Res {
    ok_json(&v)
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

/// `GET /health` (no auth).
pub async fn health(State(st): St) -> Res {
    let schema = st
        .pool
        .read(|g| Ok(openstance_core::db::schema_version(g.conn())?))
        .await?;
    json_ok(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "schema_version": schema,
    }))
}

// ---------------------------------------------------------------------------
// Search and people
// ---------------------------------------------------------------------------

const PEOPLE_KEYS: &[&str] = &[
    "q",
    "text",
    "company",
    "title",
    "location",
    "industry",
    "tag",
    "tags",
    "since",
    "until",
    "before",
    "last_contact_before",
    "met",
    "replied",
    "min_score",
    "lost_touch",
    "include_bulk",
    "limit",
    "parse",
    "explain",
];

/// The `search_people` arguments of a `GET /people` query string.
fn people_args(pairs: &[(String, String)]) -> Result<SearchPeopleArgs, ApiError> {
    only_keys(pairs, PEOPLE_KEYS)?;
    let mut a = SearchPeopleArgs::default();
    let push = |slot: &mut Option<Vec<String>>, v: &str| {
        slot.get_or_insert_with(Vec::new).push(v.to_string());
    };
    for (k, v) in pairs {
        match k.as_str() {
            "q" | "text" => a.text = Some(v.clone()),
            "company" => push(&mut a.company, v),
            "title" => push(&mut a.title, v),
            "location" => push(&mut a.location, v),
            "industry" => push(&mut a.industry, v),
            "tag" | "tags" => push(&mut a.tags, v),
            "since" => a.since = Some(v.clone()),
            "until" => a.until = Some(v.clone()),
            "before" | "last_contact_before" => a.last_contact_before = Some(v.clone()),
            "met" => a.met_in_person = Some(parse_bool(k, v)?),
            "replied" => a.replied = Some(parse_bool(k, v)?),
            "min_score" => a.min_score = Some(parse_num(k, v)?),
            "lost_touch" => a.lost_touch = Some(parse_bool(k, v)?),
            "include_bulk" => a.include_bulk = Some(parse_bool(k, v)?),
            "limit" => a.limit = Some(parse_num(k, v)?),
            "parse" => a.parse = Some(parse_bool(k, v)?),
            "explain" => a.explain = Some(parse_bool(k, v)?),
            _ => {}
        }
    }
    Ok(a)
}

async fn run_search(st: &AppState, args: SearchPeopleArgs) -> Res {
    let v = st
        .pool
        .read(move |g| {
            // Built exactly like `openstance search` builds it from flags;
            // without a limit the configured `search.default_limit` applies,
            // as on the CLI.
            let had_limit = args.limit.is_some();
            let mut req = args.to_request(g.config(), g.now())?;
            if !had_limit {
                req.filters.limit = None;
            }
            Ok(g.search(&req)?)
        })
        .await?;
    ok_json(&v)
}

async fn people(State(st): St, Query(pairs): Pairs) -> Res {
    let args = people_args(&pairs)?;
    run_search(&st, args).await
}

async fn search_post(State(st): St, Body(args): Body<SearchPeopleArgs>) -> Res {
    run_search(&st, args).await
}

async fn person(State(st): St, Path(id): Path<String>, Query(pairs): Pairs) -> Res {
    only_keys(&pairs, &["interactions", "mutuals"])?;
    let mut args = json!({ "person": id });
    for k in ["interactions", "mutuals"] {
        if let Some(v) = first(&pairs, k) {
            args[k] = json!(parse_num::<i64>(k, v)?);
        }
    }
    let ctx = tool_ctx(&st);
    let v = st
        .pool
        .read(move |g| tool(g, &ctx, "get_person", args))
        .await?;
    json_ok(v)
}

async fn ties(
    State(st): St,
    Path((id, other)): Path<(String, String)>,
    Query(pairs): Pairs,
) -> Res {
    only_keys(&pairs, &["limit"])?;
    let limit = match first(&pairs, "limit") {
        Some(v) => parse_num::<usize>("limit", v)?,
        None => 10,
    };
    if !(1..=50).contains(&limit) {
        return Err(ApiError::bad_request("limit must be between 1 and 50"));
    }
    let v = st
        .pool
        .read(move |g| {
            Ok(g.tie_evidence(&PersonKey::parse(&id), &PersonKey::parse(&other), limit)?)
        })
        .await?;
    ok_json(&v)
}

async fn connections(State(st): St, Query(pairs): Pairs) -> Res {
    only_keys(&pairs, &["target", "limit"])?;
    let target = first(&pairs, "target")
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("target is required"))?;
    let mut args = json!({ "target": target });
    if let Some(v) = first(&pairs, "limit") {
        args["limit"] = json!(parse_num::<i64>("limit", v)?);
    }
    let ctx = tool_ctx(&st);
    let v = st
        .pool
        .read(move |g| tool(g, &ctx, "how_connected", args))
        .await?;
    json_ok(v)
}

// ---------------------------------------------------------------------------
// Playbooks, quotes, edits, review
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaybookBody {
    #[serde(default)]
    input: Value,
    allow_egress: Option<bool>,
    max_usd: Option<f64>,
    #[serde(default)]
    plan_only: bool,
}

async fn playbook(State(st): St, Path(name): Path<String>, Body(b): Body<PlaybookBody>) -> Res {
    if let Some(m) = b.max_usd {
        if !m.is_finite() || m < 0.0 {
            return Err(ApiError::bad_request(format!(
                "max_usd must be a number >= 0 (got {m})"
            )));
        }
    }
    let input = if b.input.is_null() {
        json!({})
    } else {
        b.input
    };
    if !input.is_object() {
        return Err(ApiError::bad_request("input must be an object"));
    }
    // Spending goes through the writer: it holds the configured enricher
    // and writes the enrich_log rows. HTTP callers can only lower the
    // configured budget (Surface::Agent), like MCP.
    let out = st
        .pool
        .write(move |g| {
            let opts = RunOptions {
                allow_egress: b
                    .allow_egress
                    .unwrap_or(g.config().enrich.allow_egress_default),
                max_usd: b.max_usd,
                surface: Surface::Agent,
                plan_only: b.plan_only,
                run_id: None,
            };
            Ok(g.run_playbook(&name, &input, &opts)?)
        })
        .await?;
    ok_json(&out)
}

async fn quote(State(st): St, Body(body): Body<Value>) -> Res {
    let ctx = tool_ctx(&st);
    let v = st
        .pool
        .write(move |g| tool(g, &ctx, "quote_enrich", body))
        .await?;
    json_ok(v)
}

async fn edits(State(st): St, Body(req): Body<EditsRequest>) -> Res {
    let v = st.pool.write(move |g| Ok(g.apply_edits(&req.ops)?)).await?;
    ok_json(&v)
}

async fn review(State(st): St, Query(pairs): Pairs) -> Res {
    only_keys(&pairs, &["min_confidence"])?;
    let min = match first(&pairs, "min_confidence") {
        Some(v) => {
            let m: f64 = parse_num("min_confidence", v)?;
            if !(0.0..=1.0).contains(&m) {
                return Err(ApiError::bad_request(
                    "min_confidence must be between 0 and 1",
                ));
            }
            Some(m)
        }
        None => None,
    };
    let q = st
        .pool
        .read(move |g| {
            let mut q = g.review()?;
            if let Some(m) = min {
                q.flagged.retain(|f| f.confidence >= m - 1e-9);
                q.suggestions.retain(|s| s.confidence >= m - 1e-9);
            }
            Ok(q)
        })
        .await?;
    ok_json(&q)
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

fn share(st: &AppState) -> Result<openstance_core::view::ShareDirs, ApiError> {
    st.opts
        .share
        .clone()
        .ok_or_else(|| ApiError::internal("views are not configured on this server"))
}

async fn views_list(State(st): St, Query(pairs): Pairs) -> Res {
    only_keys(&pairs, &["direction"])?;
    let mut args = json!({});
    if let Some(d) = first(&pairs, "direction") {
        args["direction"] = json!(d);
    }
    let ctx = tool_ctx(&st);
    let v = st
        .pool
        .read(move |g| {
            if let Some(v) = &ctx.views {
                for name in inbox::purge_expired(&v.dir, g.now())? {
                    tracing::info!(view = %name, "deleted expired imported view");
                }
            }
            tool(g, &ctx, "list_views", args)
        })
        .await?;
    json_ok(v)
}

async fn views_search(State(st): St, Path(name): Path<String>, Query(pairs): Pairs) -> Res {
    only_keys(&pairs, &["q", "text", "company", "title", "limit"])?;
    let mut args = json!({ "view": name });
    let mut company = Vec::new();
    let mut title = Vec::new();
    for (k, v) in &pairs {
        match k.as_str() {
            "q" | "text" => args["text"] = json!(v),
            "company" => company.push(v.clone()),
            "title" => title.push(v.clone()),
            "limit" => args["limit"] = json!(parse_num::<i64>("limit", v)?),
            _ => {}
        }
    }
    if !company.is_empty() {
        args["company"] = json!(company);
    }
    if !title.is_empty() {
        args["title"] = json!(title);
    }
    let ctx = tool_ctx(&st);
    let v = st
        .pool
        .read(move |g| tool(g, &ctx, "search_view", args))
        .await?;
    json_ok(v)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateViewBody {
    name: String,
    #[serde(default)]
    query: Value,
    ttl: Option<String>,
    #[serde(default)]
    to: Vec<String>,
    /// Replace an existing view and allow `ttl: "never"` (CLI `--yes`).
    #[serde(default, alias = "replace")]
    yes: bool,
}

/// The view's query: `"text"` (parsed like `view create --query`), a full
/// `Query` as `GET /views` and search responses echo it (or `"{...}"`
/// text), or a `POST /search` input object (text parsed, filters laid over
/// it like the CLI's flags).
fn view_query(g: &Graph, q: &Value) -> Result<CoreQuery, ApiError> {
    let bad =
        |e: serde_json::Error| ApiError::bad_request(format!("query is not a valid Query: {e}"));
    let tz = search::timezone(g.config());
    let parse_text = |t: &str| -> Result<CoreQuery, ApiError> {
        let parser = RuleParser::new(Lexicon::load(g.conn())?, tz);
        Ok(parser
            .parse_detailed(t, search::today_start(g.now(), tz))
            .query)
    };
    match q {
        Value::Null => Ok(CoreQuery::default()),
        Value::Object(m) => {
            // The full echoed shape (every array plus include_bulk) is a
            // core Query, used as is; anything else is search input.
            let echoed = [
                "company",
                "title",
                "location",
                "industry",
                "tags",
                "include_bulk",
            ]
            .iter()
            .all(|k| m.contains_key(*k));
            if echoed {
                return serde_json::from_value(q.clone()).map_err(bad);
            }
            if let Ok(args) = serde_json::from_value::<SearchPeopleArgs>(q.clone()) {
                let req = args.to_request(g.config(), g.now())?;
                let mut query = match req.text.as_deref().map(str::trim) {
                    Some(t) if !t.is_empty() && !req.no_parse => parse_text(t)?,
                    Some(t) => CoreQuery {
                        text: Some(t.to_string()).filter(|t| !t.is_empty()),
                        ..CoreQuery::default()
                    },
                    None => CoreQuery::default(),
                };
                let mut patch = req.filters;
                if args.limit.is_none() {
                    patch.limit = None;
                }
                patch.apply(&mut query);
                return Ok(query);
            }
            serde_json::from_value(q.clone()).map_err(bad)
        }
        Value::String(t) if t.trim().starts_with('{') => serde_json::from_str(t).map_err(bad),
        Value::String(t) if t.trim().is_empty() => Ok(CoreQuery::default()),
        Value::String(t) => parse_text(t),
        _ => Err(ApiError::bad_request(
            "query must be text or a Query object",
        )),
    }
}

fn owner(
    st: &AppState,
    key_path: &std::path::Path,
    now: i64,
) -> Result<(String, ViewOwner), ApiError> {
    let key = openstance_core::keys::ensure(key_path, now)?;
    let pk = key.public();
    Ok((
        pk.clone(),
        ViewOwner {
            pubkey: Some(pk),
            label: st.opts.owner_label.clone(),
        },
    ))
}

async fn views_create(State(st): St, Body(b): Body<CreateViewBody>) -> Res {
    let dirs = share(&st)?;
    let st2 = Arc::clone(&st);
    let v = st
        .pool
        .write(move |g| {
            let ttl_text = b
                .ttl
                .clone()
                .unwrap_or_else(|| g.config().share.default_ttl.clone());
            let ttl = views::parse_ttl(&ttl_text)?;
            if ttl == Ttl::Never && !b.yes {
                return Err(ApiError::bad_request(
                    "ttl never shares the view with no expiry; send \"yes\": true to confirm",
                ));
            }
            let query = view_query(g, &b.query)?;
            let (pk, own) = owner(&st2, &dirs.key, g.now())?;
            let self_pk = b.to.iter().any(|t| t == "self").then_some(pk.as_str());
            let recipients = views::resolve_recipients(g.config(), &b.to, self_pk)?;
            let def = g.view_create(&CreateView {
                name: b.name.clone(),
                query,
                ttl,
                recipients,
                replace: b.yes,
            })?;
            let s = g
                .view_summaries(&own)?
                .into_iter()
                .find(|s| s.name == def.name)
                .ok_or_else(|| ApiError::internal("the new view vanished"))?;
            let mut out = serde_json::Map::new();
            out.insert("v".into(), json!(V));
            if let Value::Object(m) = serde_json::to_value(&s)? {
                out.extend(m);
            }
            Ok(Value::Object(out))
        })
        .await?;
    json_ok(v)
}

async fn views_export(State(st): St, Path(name): Path<String>, Query(pairs): Pairs) -> Res {
    only_keys(&pairs, &["armor"])?;
    let armor_q = match first(&pairs, "armor") {
        Some(v) => Some(parse_bool("armor", v)?),
        None => None,
    };
    let dirs = share(&st)?;
    let label = st.opts.owner_label.clone().unwrap_or_default();
    let file = format!("{name}.ost");
    let (bytes, armor) = st
        .pool
        .write(move |g| {
            let key = openstance_core::keys::ensure(&dirs.key, g.now())?;
            let pk = key.public();
            let armor = armor_q.unwrap_or(g.config().share.armor);
            let out = g.view_export(
                &name,
                &openstance_core::view::ost::ExportOptions {
                    owner_pubkey: &pk,
                    owner_label: &label,
                    armor,
                    tmp_parent: None,
                    version: env!("CARGO_PKG_VERSION"),
                },
            )?;
            Ok((out.bytes, armor))
        })
        .await?;
    let mut r = (StatusCode::OK, bytes).into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(if armor {
            "text/plain; charset=utf-8"
        } else {
            "application/octet-stream"
        }),
    );
    let safe: String = file
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{safe}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(r)
}

async fn views_import(State(st): St, mut mp: Multipart) -> Res {
    let dirs = share(&st)?;
    let mut bytes = None;
    while let Some(field) = mp.next_field().await.map_err(multipart_err)? {
        match field.name() {
            Some("file") => {
                if bytes.is_some() {
                    return Err(ApiError::bad_request("send one file per view import"));
                }
                bytes = Some(field.bytes().await.map_err(multipart_err)?);
            }
            Some(other) => {
                return Err(ApiError::bad_request(format!(
                    "unknown form field {other:?}; expected file"
                )))
            }
            None => {}
        }
    }
    let bytes = bytes.ok_or_else(|| ApiError::bad_request("missing form field file"))?;
    let r = st
        .pool
        .read(move |g| {
            let key = openstance_core::keys::ensure(&dirs.key, g.now())?;
            Ok(inbox::import(
                &dirs.views,
                &key.identity,
                &bytes,
                inbox::max_bytes(g.config().share.max_import_mb),
                g.now(),
            )?)
        })
        .await?;
    ok_json(&r)
}

fn multipart_err(e: axum::extract::multipart::MultipartError) -> ApiError {
    if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::too_large("upload is larger than serve.max_upload_mb")
    } else {
        ApiError::bad_request(format!("invalid multipart body: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// The file name without any directory part, safe to create in a temp dir.
fn safe_name(raw: Option<&str>, n: usize) -> String {
    let base = raw
        .unwrap_or_default()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim()
        .trim_start_matches('.');
    let clean: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    if clean.is_empty() {
        format!("upload-{n}")
    } else {
        clean
    }
}

async fn import(State(st): St, mut mp: Multipart) -> Res {
    // Uploads stream to files in a private temp dir (0700; files 0600),
    // keeping the original file name so detection works, then run through
    // the normal importer. The dir is removed when the request ends.
    let dir = tempfile::Builder::new()
        .prefix("openstance-upload-")
        .tempdir()
        .map_err(|e| ApiError::internal(format!("creating a temp dir: {e}")))?;
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut req = ImportRequest::default();
    while let Some(mut field) = mp.next_field().await.map_err(multipart_err)? {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "file" => {
                let fname = safe_name(field.file_name(), files.len());
                let sub = dir.path().join(files.len().to_string());
                std::fs::create_dir(&sub)
                    .map_err(|e| ApiError::internal(format!("creating a temp dir: {e}")))?;
                let path = sub.join(&fname);
                let mut f = tokio::fs::File::create(&path)
                    .await
                    .map_err(|e| ApiError::internal(format!("creating a temp file: {e}")))?;
                use tokio::io::AsyncWriteExt;
                while let Some(chunk) = field.chunk().await.map_err(multipart_err)? {
                    f.write_all(&chunk)
                        .await
                        .map_err(|e| ApiError::internal(format!("writing a temp file: {e}")))?;
                }
                f.flush()
                    .await
                    .map_err(|e| ApiError::internal(format!("writing a temp file: {e}")))?;
                files.push((path, fname));
            }
            "kind" => {
                req.kind = Some(
                    field
                        .text()
                        .await
                        .map_err(multipart_err)?
                        .trim()
                        .to_string(),
                )
                .filter(|s| !s.is_empty())
            }
            "label" => {
                req.label = Some(
                    field
                        .text()
                        .await
                        .map_err(multipart_err)?
                        .trim()
                        .to_string(),
                )
                .filter(|s| !s.is_empty())
            }
            "bodies" => {
                let v = field.text().await.map_err(multipart_err)?;
                if parse_bool("bodies", &v)? {
                    return Err(ApiError::bad_request(
                        "bodies: parsing email bodies is not supported in this release",
                    ));
                }
            }
            other => {
                return Err(ApiError::bad_request(format!(
                    "unknown form field {other:?}; expected file, kind, label or bodies"
                )))
            }
        }
    }
    if files.is_empty() {
        return Err(ApiError::bad_request("missing form field file"));
    }
    let resp = st
        .pool
        .write(move |g| {
            let paths: Vec<PathBuf> = files.iter().map(|(p, _)| p.clone()).collect();
            let mut resp = g.import(&paths, &req)?;
            // Report and store the uploaded file name, never the temp path.
            for (s, (_, fname)) in resp.sources.iter_mut().zip(&files) {
                if !s.skipped_duplicate_file {
                    g.set_source_path(s.id, fname)?;
                }
                s.path = fname.clone();
            }
            Ok(resp)
        })
        .await?;
    drop(dir);
    ok_json(&resp)
}

// ---------------------------------------------------------------------------
// Sources, enrich log, config, stats
// ---------------------------------------------------------------------------

async fn sources(State(st): St) -> Res {
    let v = st.pool.read(|g| Ok(g.sources()?)).await?;
    ok_json(&v)
}

async fn enrich_log(State(st): St, Query(pairs): Pairs) -> Res {
    only_keys(&pairs, &["run", "since", "person"])?;
    let run = first(&pairs, "run").map(str::to_string);
    let since = first(&pairs, "since").map(str::to_string);
    let person = first(&pairs, "person").map(str::to_string);
    let v = st
        .pool
        .read(move |g| {
            let since = match since {
                None => None,
                Some(s) => Some(since_arg(g, &s)?),
            };
            let person = match person {
                None => None,
                Some(p) => Some(resolve_person(g.conn(), &PersonKey::parse(&p))?.person),
            };
            Ok(g.enrich_log(&LogFilter { run, since, person })?)
        })
        .await?;
    ok_json(&v)
}

/// `12h` or any `parse_date_arg` form (`openstance enrich log --since`).
fn since_arg(g: &Graph, s: &str) -> Result<i64, ApiError> {
    let t = s.trim().to_ascii_lowercase();
    if let Some(h) = t.strip_suffix('h').and_then(|n| n.parse::<i64>().ok()) {
        return Ok(g.now() - h * 3600);
    }
    let tz = search::timezone(g.config());
    parse_date_arg(&t, search::today_start(g.now(), tz), tz)
        .ok_or_else(|| ApiError::bad_request(format!("could not parse since {s:?}")))
}

async fn config(State(st): St) -> Res {
    json_ok(st.opts.config_doc.clone())
}

async fn stats(State(st): St) -> Res {
    let v = st.pool.read(|g| Ok(g.dashboard_stats()?)).await?;
    ok_json(&v)
}

// ---------------------------------------------------------------------------
// Shortlist and introductions
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShortlistBody {
    person: PersonArg,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteBody {
    note: Option<String>,
}

async fn shortlist(State(st): St) -> Res {
    let v = st.pool.read(|g| Ok(g.shortlist()?)).await?;
    ok_json(&v)
}

async fn shortlist_add(State(st): St, Body(b): Body<ShortlistBody>) -> Res {
    let e = st
        .pool
        .write(move |g| Ok(g.shortlist_add(&b.person.key(), b.note.as_deref())?))
        .await?;
    json_ok(json!({ "v": V, "entry": e }))
}

async fn shortlist_patch(State(st): St, Path(p): Path<String>, Body(b): Body<NoteBody>) -> Res {
    let e = st
        .pool
        .write(move |g| Ok(g.shortlist_set_note(&PersonKey::parse(&p), b.note.as_deref())?))
        .await?;
    json_ok(json!({ "v": V, "entry": e }))
}

async fn shortlist_remove(State(st): St, Path(p): Path<String>) -> Res {
    st.pool
        .write(move |g| Ok(g.shortlist_remove(&PersonKey::parse(&p))?))
        .await?;
    json_ok(json!({ "v": V, "removed": true }))
}

fn parse_stage(s: &str) -> Result<Stage, ApiError> {
    Stage::parse(s).ok_or_else(|| {
        ApiError::bad_request(format!(
            "stage must be one of draft, asked, intro_made, met, closed (got {s:?})"
        ))
    })
}

async fn intros(State(st): St, Query(pairs): Pairs) -> Res {
    only_keys(&pairs, &["stage"])?;
    let stage = first(&pairs, "stage").map(parse_stage).transpose()?;
    let v = st.pool.read(move |g| Ok(g.intros(stage)?)).await?;
    ok_json(&v)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntroBody {
    person: PersonArg,
    connector: Option<PersonArg>,
    stage: Option<String>,
    note: Option<String>,
    follow_up: Option<String>,
}

async fn intro_create(State(st): St, Body(b): Body<IntroBody>) -> Res {
    let req = IntroRequest {
        person: b.person.key(),
        connector: b.connector.map(|c| c.key()),
        stage: b.stage.as_deref().map(parse_stage).transpose()?,
        note: b.note,
        follow_up: b.follow_up,
    };
    let i = st.pool.write(move |g| Ok(g.intro_create(&req)?)).await?;
    json_ok(json!({ "v": V, "intro": i }))
}

/// `PATCH /intros/{id}`: a missing key keeps the field, `null` clears it.
fn intro_edit(body: &Value) -> Result<IntroEdit, ApiError> {
    let obj = body
        .as_object()
        .ok_or_else(|| ApiError::bad_request("body must be a JSON object"))?;
    let mut e = IntroEdit::default();
    for (k, v) in obj {
        let text = |v: &Value| -> Result<Option<String>, ApiError> {
            match v {
                Value::Null => Ok(None),
                Value::String(s) => Ok(Some(s.clone())),
                _ => Err(ApiError::bad_request(format!(
                    "{k} must be a string or null"
                ))),
            }
        };
        match k.as_str() {
            "stage" => {
                let s = v
                    .as_str()
                    .ok_or_else(|| ApiError::bad_request("stage must be a string"))?;
                e.stage = Some(parse_stage(s)?);
            }
            "note" => e.note = Some(text(v)?),
            "follow_up" => e.follow_up = Some(text(v)?),
            "connector" => {
                e.connector = Some(match v {
                    Value::Null => None,
                    other => Some(
                        serde_json::from_value::<PersonArg>(other.clone())
                            .map_err(|_| {
                                ApiError::bad_request(
                                    "connector must be a person id, a reference or null",
                                )
                            })?
                            .key(),
                    ),
                });
            }
            other => {
                return Err(ApiError::bad_request(format!(
                    "unknown field {other:?}; expected stage, connector, note or follow_up"
                )))
            }
        }
    }
    Ok(e)
}

async fn intro_patch(State(st): St, Path(id): Path<i64>, Body(body): Body<Value>) -> Res {
    let edit = intro_edit(&body)?;
    let i = st
        .pool
        .write(move |g| Ok(g.intro_update(id, &edit)?))
        .await?;
    json_ok(json!({ "v": V, "intro": i }))
}

async fn intro_delete(State(st): St, Path(id): Path<i64>) -> Res {
    st.pool.write(move |g| Ok(g.intro_delete(id)?)).await?;
    json_ok(json!({ "v": V, "removed": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(q: &str) -> Vec<(String, String)> {
        q.split('&')
            .filter(|s| !s.is_empty())
            .map(|kv| {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                (k.to_string(), v.to_string())
            })
            .collect()
    }

    #[test]
    fn people_args_repeat_keys_into_arrays() {
        let a = people_args(&pairs("q=x&company=A&company=B&met=true&limit=3")).unwrap();
        assert_eq!(a.text.as_deref(), Some("x"));
        assert_eq!(a.company, Some(vec!["A".to_string(), "B".to_string()]));
        assert_eq!(a.met_in_person, Some(true));
        assert_eq!(a.limit, Some(3));
        assert!(people_args(&pairs("bogus=1")).is_err());
        assert!(people_args(&pairs("met=perhaps")).is_err());
    }

    #[test]
    fn safe_name_strips_directories() {
        assert_eq!(safe_name(Some("../../etc/passwd"), 0), "passwd");
        assert_eq!(
            safe_name(Some("C:\\x\\Connections.csv"), 0),
            "Connections.csv"
        );
        assert_eq!(safe_name(Some(".hidden"), 0), "hidden");
        assert_eq!(safe_name(None, 2), "upload-2");
        assert_eq!(safe_name(Some("../"), 3), "upload-3");
    }

    #[test]
    fn intro_edit_distinguishes_missing_from_null() {
        let e = intro_edit(&json!({ "stage": "met", "note": null })).unwrap();
        assert_eq!(e.stage, Some(Stage::Met));
        assert_eq!(e.note, Some(None));
        assert_eq!(e.follow_up, None);
        assert_eq!(e.connector, None);
        let e = intro_edit(&json!({ "connector": 4 })).unwrap();
        assert_eq!(e.connector, Some(Some(PersonKey::Id(4))));
        assert!(intro_edit(&json!({ "stage": "nope" })).is_err());
        assert!(intro_edit(&json!({ "colour": "red" })).is_err());
        assert!(intro_edit(&json!([1])).is_err());
    }
}
