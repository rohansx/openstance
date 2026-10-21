# Connecting MCP clients

`openstance mcp` runs a [Model Context Protocol](../tech-spec/interfaces.md#2-mcp-server) server on stdio. An agent such as Claude Code, Cursor or Claude Desktop starts it as a child process and can then use eight tools:

| Tool | What it answers | Cost |
| --- | --- | --- |
| `search_people` | "who do I know at Northwind", "fintech founders I met since 2025", "who have I lost touch with" | free, local |
| `get_person` | everything about one person: fields, score and why, identities, recent interactions, mutuals | free, local |
| `how_connected` | warm introduction paths to someone already in your graph | free, local |
| `list_views` | your saved views and the views other people sent you | free, local |
| `search_view` | people in a view someone shared with you | free, local |
| `quote_enrich` | the price of enrichment requests; sends no contact data | free (prices only) |
| `path_to` | intro paths to anyone, including people not in your graph | may spend, only with `allow_egress: true` |
| `brief` | a short fact-only brief on one person, optionally with one web search | may spend, only with `allow_egress: true` |

The first five read `~/.openstance/graph.db` (and `~/.openstance/views/`), cost nothing, and make no network calls. `path_to` and `brief` return a quote (`status: "quote"`) and send nothing unless the call sets `allow_egress: true`, and with the default config (`enrich.backend = "none"`) nothing is ever sent at all. `max_usd` can only lower the configured `enrich.max_usd_per_run`. Every paid request is in `openstance enrich log`. Input schemas are in [interfaces.md section 2.2](../tech-spec/interfaces.md#22-input-schemas).

## Before you start

1. Install openstance and import something (see the [README](../../README.md)). The server serves whatever graph is there, and an empty graph returns empty results.
2. Find the absolute path of the binary. Some clients (Claude Desktop in particular) do not start servers with your shell's `PATH`:

   ```console
   $ command -v openstance
   /Users/you/.cargo/bin/openstance
   ```

3. Check that the server starts. It prints nothing and waits for JSON-RPC on stdin. Press Ctrl-D to exit:

   ```console
   $ openstance mcp
   ```

   If it exits at once with an error (on stderr), fix that first. A common cause is a missing graph: run `openstance init`.

## Claude Code

```console
$ claude mcp add openstance -- openstance mcp
```

This adds the server for the current project. Add `--scope user` to make it available in every project. Check it with `claude mcp list`, or `/mcp` inside a session.

To serve a different graph, pass global flags after `mcp`:

```console
$ claude mcp add openstance -- openstance mcp --db /path/to/graph.db
```

Agents pick the right tool more reliably when they also have the skill files in [`playbooks/`](../../playbooks/README.md). [`openstance-search.md`](../../playbooks/openstance-search.md) explains when to use each free tool and asks the agent to quote evidence lines instead of paraphrasing them. [`path.md`](../../playbooks/path.md) and [`brief.md`](../../playbooks/brief.md) tell the agent to ask for a quote first and to spend only after you agree. In Claude Code, copy it into the project's `.claude/skills/openstance-search/SKILL.md` (or `~/.claude/skills/...` for all projects).

## Cursor

Global config is `~/.cursor/mcp.json`. For one project only, use `.cursor/mcp.json` in that project:

```json
{
  "mcpServers": {
    "openstance": {
      "command": "openstance",
      "args": ["mcp"]
    }
  }
}
```

If Cursor reports that the server failed to start, replace `"openstance"` with the absolute path from `command -v openstance`. The skill file can be added as a project rule (`.cursor/rules/openstance-search.md`).

## Claude Desktop

Edit the config file (Settings, Developer, Edit Config opens it):

- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "openstance": {
      "command": "/Users/you/.cargo/bin/openstance",
      "args": ["mcp"]
    }
  }
}
```

Use the absolute path, because Claude Desktop does not read your shell profile. To use a non-default home, add `"env": { "OPENSTANCE_HOME": "/path/to/home" }` next to `"args"`. Restart Claude Desktop after editing the file.

## MCP Inspector

To see the tools and call them by hand:

```console
$ npx @modelcontextprotocol/inspector openstance mcp
```

## Notes

- **Importing while the server runs.** The database is in WAL mode, so `openstance import` and `rebuild` can run while a client is connected. The next tool call sees the new data.
- **Logs.** The server writes logs to stderr only. stdout carries the protocol and nothing else. Set `OPENSTANCE_LOG=debug` in the client's `env` block to get more detail in the client's server log.
- **Embeddings.** `search_people` uses the embedder set in `model.embedder`, the same as `openstance search`, so its ranking matches the CLI. Nothing loads at startup. The model loads on the first search that needs it, from the local cache (`model.cache_dir`, or `OPENSTANCE_MODEL_DIR`). To keep the server fully offline, run `openstance rebuild --embeddings` once with network, or set `model.embedder = "none"` to get full-text search only. If the embedder changed since the vectors were built, the server logs a warning on stderr and searches use full-text only until you run `openstance rebuild --embeddings`.
- **Fixed time.** `openstance mcp --now 2026-10-20` pins "now" for scores and relative dates (`since: "90d"`). This is useful for demos and tests.
- **`--read-only`.** Also set by `[mcp] read_only = true`. It forces `allow_egress` to false on `path_to` and `brief`, so those tools only ever return quotes.
- **Same JSON as the CLI.** A tool's `structuredContent` is the document the CLI prints with `--json`. `search_people` with `{"text": "devtools dinner", "limit": 5}` returns the same JSON as `openstance search "devtools dinner" --limit 5 --json` (apart from `took_ms`). Its `rerun` field is that CLI command, so you can repeat an agent's search in a terminal.
- **Privacy.** Over stdio, results include people marked `do_not_share`, as local search does. That flag only affects sharing views.

## Verification log

P2-MCP-06 requires a hand test of each snippet against a real client, recorded in the PR. Fill this in when you run them:

| Client | Version | Snippet | Result | Date |
| --- | --- | --- | --- | --- |
| Claude Code | | `claude mcp add` | not yet verified by hand | |
| Cursor | | `~/.cursor/mcp.json` | not yet verified by hand | |
| Claude Desktop | | `claude_desktop_config.json` | not yet verified by hand | |
| MCP Inspector | | `npx @modelcontextprotocol/inspector` | not yet verified by hand | |

The protocol side is covered by automated tests. `crates/openstance/tests/mcp.rs` starts the real binary, performs the `initialize` handshake, lists the tools, calls each one, and checks that stdout carries only JSON-RPC.
