//! Tool definitions. The input schemas are written out by hand so they are
//! exactly the ones in `docs/tech-spec/interfaces.md` section 2.2 (a test
//! parses that section and compares); deriving them would add `null` unions
//! and `$defs` that some clients handle badly ("keep the tool schemas flat").

use std::sync::Arc;

use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use serde_json::{json, Value};

/// `search_people`.
pub const SEARCH_PEOPLE: &str = "search_people";
/// `get_person`.
pub const GET_PERSON: &str = "get_person";
/// `how_connected`.
pub const HOW_CONNECTED: &str = "how_connected";

/// `path_to` (P3).
pub const PATH_TO: &str = "path_to";
/// `brief` (P3).
pub const BRIEF: &str = "brief";
/// `list_views` (P3).
pub const LIST_VIEWS: &str = "list_views";
/// `search_view` (P3).
pub const SEARCH_VIEW: &str = "search_view";
/// `quote_enrich` (P3).
pub const QUOTE_ENRICH: &str = "quote_enrich";

/// Every tool this build registers, in `tools/list` order (interfaces.md 2.1).
pub const TOOL_NAMES: [&str; 8] = [
    SEARCH_PEOPLE,
    GET_PERSON,
    HOW_CONNECTED,
    PATH_TO,
    BRIEF,
    LIST_VIEWS,
    SEARCH_VIEW,
    QUOTE_ENRICH,
];

/// The local, free, read-only tools of Phase 2.
pub const LOCAL_TOOLS: [&str; 3] = [SEARCH_PEOPLE, GET_PERSON, HOW_CONNECTED];

const SEARCH_DESCRIPTION: &str = "Search the user's own relationship graph (local, free). Returns people ranked by text relevance lifted by relationship score, with dated evidence. Filters you set are echoed back in `query`; edit and rerun instead of rephrasing.";

const GET_PERSON_DESCRIPTION: &str = "Look up one person in the user's own relationship graph (local, free): fields and where each came from, the relationship score with its five components, identities, recent interactions and the strongest mutual connections. An ambiguous name returns an `ambiguous` error with candidates; ask the user which one and call again with the id.";

const HOW_CONNECTED_DESCRIPTION: &str = "Find warm introduction paths to someone already in the user's graph (local, free). Each path goes through a mutual connection: strength = the user's score with the mutual x the mutual's tie to the target, with dated evidence. Returns `found: false` when the target is not in the graph.";

const PATH_TO_DESCRIPTION: &str = "Find warm intro paths to a person, including people not in the graph. Local steps are free. Looking up a stranger's employers or email costs money: call with allow_egress=false first, show the user the quote, and only then call with allow_egress=true.";

const BRIEF_DESCRIPTION: &str = "Write a short, fact-only intro brief about one person in the user's graph: who they are, how the user knows them, last contact, the best mutual and a suggested opener. Local and free unless web=true, which adds one paid web search: call with allow_egress=false first, show the user the quote, and only then call with allow_egress=true.";

const LIST_VIEWS_DESCRIPTION: &str = "List shared views (local, free): the user's own saved views and the encrypted views other people sent them. `direction` filters own, imported or all.";

const SEARCH_VIEW_DESCRIPTION: &str = "Search a view someone shared with the user (local, free). These people are not in the user's own graph; each result names the view and its claimed owner. Use '*' to search every imported, unexpired view.";

const QUOTE_ENRICH_DESCRIPTION: &str = "Price enrichment requests (profile, find_email, verify_email, company, web_search) with the configured backend. Never sends contact data and never spends; returns a Quote. People marked no_enrich come back under `skipped`.";

fn object(v: Value) -> Arc<JsonObject> {
    match v {
        Value::Object(m) => Arc::new(m),
        _ => unreachable!("tool schemas are JSON objects"),
    }
}

fn search_people_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "text":          { "type": "string", "description": "free text, e.g. 'fintech founders I met'" },
            "company":       { "type": "array", "items": { "type": "string" } },
            "title":         { "type": "array", "items": { "type": "string" } },
            "location":      { "type": "array", "items": { "type": "string" } },
            "industry":      { "type": "array", "items": { "type": "string" }, "description": "soft: boosts relevance, never excludes" },
            "tags":          { "type": "array", "items": { "type": "string" }, "description": "exact match on the user's tags" },
            "since":         { "type": "string", "description": "had an interaction on or after: YYYY, YYYY-MM, YYYY-MM-DD or 90d/6m/1y" },
            "until":         { "type": "string", "description": "had an interaction before this date (with since: inside the window)" },
            "last_contact_before": { "type": "string", "description": "last contact strictly before this date (\"haven't talked since\")" },
            "met_in_person": { "type": "boolean" },
            "replied":       { "type": "boolean" },
            "min_score":     { "type": "number", "minimum": 0, "maximum": 1 },
            "lost_touch":    { "type": "boolean", "description": "high frequency, low recency" },
            "include_bulk":  { "type": "boolean", "default": false },
            "limit":         { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
            "parse":         { "type": "boolean", "default": true, "description": "run the query parser on text; false = take text and filters literally (search.md no_parse)" },
            "explain":       { "type": "boolean", "default": false }
        },
        "additionalProperties": false
    })
}

fn get_person_schema() -> Value {
    json!({ "type": "object", "required": ["person"],
        "properties": {
            "person": { "type": "string", "description": "id, email, linkedin url/slug, @x_handle, or name" },
            "interactions": { "type": "integer", "minimum": 0, "maximum": 50, "default": 10 },
            "mutuals": { "type": "integer", "minimum": 0, "maximum": 50, "default": 10 } },
        "additionalProperties": false })
}

fn how_connected_schema() -> Value {
    json!({ "type": "object", "required": ["target"],
        "properties": {
            "target": { "type": "string", "description": "person id, email, linkedin, or name (must be in the graph; use path_to for strangers)" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 10, "default": 3 } },
        "additionalProperties": false })
}

fn path_to_schema() -> Value {
    json!({ "type": "object", "required": ["target"],
        "properties": {
            "target": { "type": "string", "description": "name, email, linkedin url, or person id" },
            "company": { "type": "string" },
            "linkedin": { "type": "string" },
            "find_email": { "type": "boolean", "default": false },
            "limit": { "type": "integer", "minimum": 1, "maximum": 10, "default": 3 },
            "allow_egress": { "type": "boolean", "default": false },
            "max_usd": { "type": "number", "minimum": 0 },
            "plan_only": { "type": "boolean", "default": false } },
        "additionalProperties": false })
}

fn brief_schema() -> Value {
    json!({ "type": "object", "required": ["person"],
        "properties": {
            "person": { "type": "string" },
            "web": { "type": "boolean", "default": false, "description": "one web search for recent public posts (costs money)" },
            "allow_egress": { "type": "boolean", "default": false },
            "max_usd": { "type": "number", "minimum": 0 } },
        "additionalProperties": false })
}

fn list_views_schema() -> Value {
    json!({ "type": "object",
        "properties": { "direction": { "enum": ["own", "imported", "all"], "default": "all" } },
        "additionalProperties": false })
}

fn search_view_schema() -> Value {
    json!({ "type": "object", "required": ["view"],
        "properties": {
            "view": { "type": "string", "description": "view name or id; '*' searches every imported, unexpired view" },
            "text": { "type": "string" },
            "company": { "type": "array", "items": { "type": "string" } },
            "title": { "type": "array", "items": { "type": "string" } },
            "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 } },
        "additionalProperties": false })
}

fn quote_enrich_schema() -> Value {
    json!({ "type": "object", "required": ["requests"],
        "properties": { "requests": { "type": "array", "maxItems": 500, "items": {
          "type": "object", "required": ["kind"],
          "properties": {
            "kind": { "enum": ["profile", "find_email", "verify_email", "company", "web_search"] },
            "person_id": { "type": "integer" },
            "name": { "type": "string" }, "company": { "type": "string" }, "linkedin": { "type": "string" },
            "domain": { "type": "string" }, "email": { "type": "string" }, "query": { "type": "string" } } } } },
        "additionalProperties": false })
}

/// Local, read-only, free: nothing leaves the machine.
fn local_read_only() -> ToolAnnotations {
    let mut a = ToolAnnotations::new().read_only(true);
    a.open_world_hint = Some(false);
    a
}

/// Reads only, but asks the configured backend for prices (no contact data).
fn priced_read_only() -> ToolAnnotations {
    let mut a = ToolAnnotations::new().read_only(true);
    a.open_world_hint = Some(true);
    a
}

/// May spend money and reach the network (only with `allow_egress: true`);
/// writes the audit log but never deletes or overwrites user data.
fn may_spend() -> ToolAnnotations {
    let mut a = ToolAnnotations::new().read_only(false).destructive(false);
    a.open_world_hint = Some(true);
    a.idempotent_hint = Some(false);
    a
}

fn tool(name: &'static str, description: &'static str, schema: Value) -> Tool {
    tool_with(name, description, schema, local_read_only())
}

fn tool_with(
    name: &'static str,
    description: &'static str,
    schema: Value,
    annotations: ToolAnnotations,
) -> Tool {
    Tool::new(name, description, object(schema)).with_annotations(annotations)
}

/// The registered tools (interfaces.md 2.1).
pub fn tools() -> Vec<Tool> {
    vec![
        tool(SEARCH_PEOPLE, SEARCH_DESCRIPTION, search_people_schema()),
        tool(GET_PERSON, GET_PERSON_DESCRIPTION, get_person_schema()),
        tool(
            HOW_CONNECTED,
            HOW_CONNECTED_DESCRIPTION,
            how_connected_schema(),
        ),
        tool_with(PATH_TO, PATH_TO_DESCRIPTION, path_to_schema(), may_spend()),
        tool_with(BRIEF, BRIEF_DESCRIPTION, brief_schema(), may_spend()),
        tool(LIST_VIEWS, LIST_VIEWS_DESCRIPTION, list_views_schema()),
        tool(SEARCH_VIEW, SEARCH_VIEW_DESCRIPTION, search_view_schema()),
        tool_with(
            QUOTE_ENRICH,
            QUOTE_ENRICH_DESCRIPTION,
            quote_enrich_schema(),
            priced_read_only(),
        ),
    ]
}

/// Whether `name` is a registered tool.
pub fn is_tool(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_definitions_line_up() {
        let names: Vec<String> = tools().iter().map(|t| t.name.to_string()).collect();
        assert_eq!(names, TOOL_NAMES);
        assert!(is_tool("get_person"));
        assert!(is_tool("path_to"));
        assert!(!is_tool("watch"));
    }
}
