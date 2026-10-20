# ADR-0011: CLI, MCP and HTTP share one JSON shape

- **Status:** Accepted
- **Date:** 2026-10-20
- **Related:** [tech-spec/interfaces.md](../tech-spec/interfaces.md), [architecture/overview.md](../architecture/overview.md)

## Context

openstance exposes the same capabilities through three surfaces: the CLI (humans and shell scripts), an MCP server (Claude Code, Cursor, other agents) and a localhost HTTP API (the optional web UI and integrations). The overview's thesis is that agents are the front door. If each surface had its own response format, behavior would drift, tests would triple, and agents would see different data than humans.

## Decision

- One set of `serde` response types lives in `openstance-core` (for example `SearchResponse`, `PersonDetail`, `HowConnected`, `Outcome`, `Quote`, `Receipt`, `ViewSummary`; names as in [tech-spec/interfaces.md](../tech-spec/interfaces.md)). Every surface serializes those types unchanged.
  - CLI: human table by default; `--json` prints exactly the core type.
  - HTTP (`openstance serve`, `127.0.0.1:7411`): returns the same JSON. Endpoints: `GET /people?q=`, `GET /people/{id}`, `POST /playbooks/{name}`, `POST /edits`, `GET /views`, `POST /import` (multipart), `GET /health`.
  - MCP (`openstance mcp`, stdio; also over HTTP via `serve`): tools return the same JSON as structured content.
- Canonical MCP tools: `search_people`, `get_person`, `how_connected`, `path_to`, `brief`, `list_views`, `search_view`, `quote_enrich`. "Who knows someone at X" is `search_people` with a company filter plus `how_connected`; there is no separate `who_knows` tool.
- Every money-spending operation takes `allow_egress` (default `false`) and `max_usd`, and returns a `Quote` instead of running when `allow_egress` is false.
- Every top-level response carries `"v": 1`, the JSON schema version ([tech-spec/interfaces.md](../tech-spec/interfaces.md#1-shared-json-shapes)); breaking changes bump it, additive fields do not.
- The web UI is a client of the HTTP API and nothing else.

## Consequences

**Positive**

- One contract test suite: snapshot the JSON for fixture queries; assert the CLI `--json`, HTTP and MCP outputs are byte-identical after normalization.
- Agents and humans see the same evidence and scores.
- Scripting (`ost search ... --json | jq`) is a first-class use.

**Negative**

- The JSON must suit three audiences; some fields are verbose for humans (handled by the CLI table renderer).
- Changing a field is a breaking change across all three surfaces at once; requires the `v` bump discipline.

## Alternatives considered

| Option | Why not |
|---|---|
| Separate MCP-specific shapes | Drift; agents see different data |
| GraphQL for HTTP | Overkill for a localhost API with seven endpoints |
| CLI text only, JSON for API | Breaks scripting and makes the CLI untestable as an API |
