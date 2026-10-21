//! The agent skills (P2-MCP-05, P3-MCP-05) and the client setup guide
//! (P2-MCP-06) stay in step with the registered tools.

use std::path::{Path, PathBuf};

use openstance_mcp::schema::{LOCAL_TOOLS, TOOL_NAMES};
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

/// One frontmatter value.
fn get(fm: &[(String, String)], k: &str) -> String {
    fm.iter()
        .find(|(key, _)| key == k)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| panic!("frontmatter {k}"))
}

/// The `tools: [a, b]` list.
fn tools_of(fm: &[(String, String)]) -> Vec<String> {
    get(fm, "tools")
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|t| t.trim().to_string())
        .collect()
}

/// Every skill file in `playbooks/` (not the README).
fn skills() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(repo("playbooks"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter(|p| p.file_name().is_some_and(|n| n != "README.md"))
        .map(|p| {
            let rel = format!("playbooks/{}", p.file_name().unwrap().to_string_lossy());
            let doc = read(&rel);
            (rel, doc)
        })
        .collect();
    out.sort();
    out
}

#[test]
fn skill_frontmatter_names_only_registered_tools() {
    let doc = read("playbooks/openstance-search.md");
    let fm = frontmatter(&doc);
    assert_eq!(get(&fm, "name"), "openstance-search");
    assert!(get(&fm, "description").len() > 40);
    assert_eq!(get(&fm, "egress"), "none");
    assert_eq!(tools_of(&fm), LOCAL_TOOLS);
}

#[test]
fn skill_mentions_every_tool_and_no_outside_links() {
    let doc = read("playbooks/openstance-search.md");
    for t in LOCAL_TOOLS {
        assert!(doc.contains(&format!("`{t}`")), "{t} explained");
    }
    // The free search skill never steers the agent to a paid tool.
    for paid in ["path_to", "quote_enrich", "allow_egress"] {
        assert!(!doc.contains(paid), "{paid} belongs in path.md / brief.md");
    }
    assert!(!doc.contains("http://") && !doc.contains("https://"));
    assert!(!doc.to_lowercase().contains("star this"));
}

#[test]
fn every_skill_has_frontmatter_registered_tools_and_no_promotion() {
    let all = skills();
    let names: Vec<&str> = all.iter().map(|(p, _)| p.as_str()).collect();
    for want in [
        "playbooks/brief.md",
        "playbooks/openstance-search.md",
        "playbooks/path.md",
    ] {
        assert!(names.contains(&want), "{want} exists: {names:?}");
    }
    for (path, doc) in &all {
        let fm = frontmatter(doc);
        assert!(get(&fm, "name").starts_with("openstance-"), "{path} name");
        assert!(get(&fm, "description").len() > 40, "{path} description");
        get(&fm, "egress");
        for t in tools_of(&fm) {
            assert!(
                TOOL_NAMES.contains(&t.as_str()),
                "{path}: {t} is not registered"
            );
            assert!(doc.contains(&format!("`{t}`")), "{path}: {t} explained");
        }
        let lower = doc.to_lowercase();
        assert!(
            !lower.contains("http://") && !lower.contains("https://"),
            "{path} has a URL"
        );
        for promo in ["star this", "treg", "sign up", "affiliate", "upgrade to"] {
            assert!(!lower.contains(promo), "{path} mentions {promo:?}");
        }
    }
}

#[test]
fn paid_skills_quote_first_and_wait_for_consent() {
    for (path, tool) in [
        ("playbooks/path.md", "path_to"),
        ("playbooks/brief.md", "brief"),
    ] {
        let doc = read(path);
        let fm = frontmatter(&doc);
        assert!(
            tools_of(&fm).contains(&tool.to_string()),
            "{path} uses {tool}"
        );
        assert!(get(&fm, "egress").starts_with("optional"), "{path} egress");
        let first = doc.find("allow_egress: false").expect("quote call first");
        let then = doc.find("allow_egress: true").expect("then the paid call");
        assert!(first < then, "{path}: quote before spending");
        let lower = doc.to_lowercase();
        assert!(lower.contains("explicit"), "{path}: explicit consent");
        assert!(lower.contains("quote"), "{path}: shows the quote");
        assert!(lower.contains("no_enrich"), "{path}: respects no_enrich");
    }
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
