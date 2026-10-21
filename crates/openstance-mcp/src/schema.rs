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

/// Every tool this build registers, in `tools/list` order.
pub const TOOL_NAMES: [&str; 3] = [SEARCH_PEOPLE, GET_PERSON, HOW_CONNECTED];

const SEARCH_DESCRIPTION: &str = "Search the user's own relationship graph (local, free). Returns people ranked by text relevance lifted by relationship score, with dated evidence. Filters you set are echoed back in `query`; edit and rerun instead of rephrasing.";

const GET_PERSON_DESCRIPTION: &str = "Look up one person in the user's own relationship graph (local, free): fields and where each came from, the relationship score with its five components, identities, recent interactions and the strongest mutual connections. An ambiguous name returns an `ambiguous` error with candidates; ask the user which one and call again with the id.";

const HOW_CONNECTED_DESCRIPTION: &str = "Find warm introduction paths to someone already in the user's graph (local, free). Each path goes through a mutual connection: strength = the user's score with the mutual x the mutual's tie to the target, with dated evidence. Returns `found: false` when the target is not in the graph.";

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

/// Local, read-only, free: nothing leaves the machine.
fn local_read_only() -> ToolAnnotations {
    let mut a = ToolAnnotations::new().read_only(true);
    a.open_world_hint = Some(false);
    a
}

fn tool(name: &'static str, description: &'static str, schema: Value) -> Tool {
    Tool::new(name, description, object(schema)).with_annotations(local_read_only())
}

/// The registered tools (interfaces.md 2.1, Phase 2 rows).
pub fn tools() -> Vec<Tool> {
    vec![
        tool(SEARCH_PEOPLE, SEARCH_DESCRIPTION, search_people_schema()),
        tool(GET_PERSON, GET_PERSON_DESCRIPTION, get_person_schema()),
        tool(
            HOW_CONNECTED,
            HOW_CONNECTED_DESCRIPTION,
            how_connected_schema(),
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
        assert!(!is_tool("path_to"));
    }
}
