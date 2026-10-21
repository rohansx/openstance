# playbooks/

Agent-facing skill files. Each one tells an agent (Claude Code, Cursor and
others that read skills) when to call which openstance MCP tool, in
openstance's own words. See `docs/tech-spec/playbooks.md` section 10.

| File | Tools | Egress |
| --- | --- | --- |
| `openstance-search.md` | `search_people`, `get_person`, `how_connected` | none |
| `path.md` | `how_connected`, `path_to` | optional, quote first |
| `brief.md` | `get_person`, `brief` | optional (one web search), quote first |

The paid skills always call with `allow_egress: false` first, show the user
the quote, and spend only after the user explicitly agrees. The remaining
playbooks (`accounts`, `refresh`, `watch`, `reconnect`, `verify`) arrive in
Phase 4. `crates/openstance-mcp/tests/docs.rs` lints every file here.
