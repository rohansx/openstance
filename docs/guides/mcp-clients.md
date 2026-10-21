# Connecting MCP clients

`openstance mcp` runs a [Model Context Protocol](../tech-spec/interfaces.md#2-mcp-server) server on stdio. An agent such as Claude Code, Cursor or Claude Desktop starts it as a child process and can then search your graph with three tools:

| Tool | What it answers |
| --- | --- |
| `search_people` | "who do I know at Northwind", "fintech founders I met since 2025", "who have I lost touch with" |
| `get_person` | everything about one person: fields, score and why, identities, recent interactions, mutuals |
| `how_connected` | warm introduction paths to someone already in your graph |

All three are read-only and local. They read `~/.openstance/graph.db`, cost nothing, and make no network calls. Input schemas are in [interfaces.md section 2.2](../tech-spec/interfaces.md#22-input-schemas).

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

Agents pick the right tool more reliably when they also have the skill file [`playbooks/openstance-search.md`](../../playbooks/openstance-search.md). It explains when to use each tool and asks the agent to quote evidence lines instead of paraphrasing them. In Claude Code, copy it into the project's `.claude/skills/openstance-search/SKILL.md` (or `~/.claude/skills/...` for all projects).

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
- **`--read-only`.** Accepted now, and also set by `[mcp] read_only = true`. It matters once tools that can spend money (`path_to`, `brief`) arrive in Phase 3. It forces `allow_egress` to false.
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
