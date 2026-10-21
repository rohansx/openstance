//! The agent skill (P2-MCP-05) and the client setup guide (P2-MCP-06) stay
//! in step with the registered tools.

use std::path::{Path, PathBuf};

use openstance_mcp::schema::TOOL_NAMES;
use serde_json::Value;

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// `key: value` lines of the `---` frontmatter.
fn frontmatter(doc: &str) -> Vec<(String, String)> {
    let body = doc.strip_prefix("---\n").expect("frontmatter");
    let (fm, _) = body.split_once("\n---\n").expect("closing ---");
    fm.lines()
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

#[test]
fn skill_frontmatter_names_only_registered_tools() {
    let doc = read("playbooks/openstance-search.md");
    let fm = frontmatter(&doc);
    let get = |k: &str| {
        fm.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("frontmatter {k}"))
    };
    assert_eq!(get("name"), "openstance-search");
    assert!(get("description").len() > 40);
    assert_eq!(get("egress"), "none");
    let tools: Vec<String> = get("tools")
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|t| t.trim().to_string())
        .collect();
    assert_eq!(tools, TOOL_NAMES);
}

#[test]
fn skill_mentions_every_tool_and_no_outside_links() {
    let doc = read("playbooks/openstance-search.md");
    for t in TOOL_NAMES {
        assert!(doc.contains(&format!("`{t}`")), "{t} explained");
    }
    for unregistered in ["path_to", "brief", "quote_enrich", "list_views"] {
        assert!(
            !doc.contains(unregistered),
            "{unregistered} is not a Phase 2 tool"
        );
    }
    assert!(!doc.contains("http://") && !doc.contains("https://"));
    assert!(!doc.to_lowercase().contains("star this"));
}

#[test]
fn client_guide_json_snippets_start_openstance_mcp() {
    let doc = read("docs/guides/mcp-clients.md");
    assert!(doc.contains("claude mcp add openstance -- openstance mcp"));
    let blocks: Vec<Value> = doc
        .split("```json")
        .skip(1)
        .filter_map(|b| b.split("```").next())
        .map(|b| serde_json::from_str(b).unwrap_or_else(|e| panic!("{e}: {b}")))
        .collect();
    assert_eq!(blocks.len(), 2, "Cursor and Claude Desktop");
    for b in blocks {
        let s = &b["mcpServers"]["openstance"];
        assert!(s["command"].as_str().unwrap().ends_with("openstance"));
        assert_eq!(s["args"], serde_json::json!(["mcp"]));
    }
}
