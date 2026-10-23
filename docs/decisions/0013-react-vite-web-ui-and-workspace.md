# ADR-0013: The web UI is Vite + React + TypeScript; shortlist and introductions are stored in the graph

- **Status:** Accepted
- **Date:** 2026-10-21
- **Related:** [ADR-0011](0011-one-json-shape-across-surfaces.md), [plan/phase-4.md](../plan/phase-4.md) (P4-API-*, P4-UI-*), [tech-spec/interfaces.md](../tech-spec/interfaces.md#3-http-api-p4), [tech-spec/data-model.md](../tech-spec/data-model.md), [ui/README.md](../../ui/README.md)

## Context

Phase 4 adds a local web UI served by `openstance serve`. The plan (P4-UI-01) asked for Vite + TypeScript with "no heavy framework requirement (a small library such as Preact or Solid is fine)". The maintainer had already built a vanilla-JS browser prototype of the dashboard (sidebar workspace, search with chips, person page, shortlist, introductions board, integrations) and chose **Vite + React + TypeScript** for the real UI.

The prototype also had two workspace features that the graph did not store: a **shortlist** of people to act on and an **introductions board** (who you want to meet, through whom, at which stage). In the prototype both lived in `localStorage` against fake data. A real UI needs them in the graph, so they survive a browser change and are visible to the CLI and agents later.

## Decision

1. **UI stack: Vite + React 19 + TypeScript** (the maintainer's choice), in `ui/`. It overrides the "no heavy framework" note in P4-UI-01. Guard rails:
   - Budget: the built bundle stays **under 300 KB gzipped** (`npm run size`, checked in CI; about 105 KB at the time of writing).
   - No runtime dependencies beyond `react` and `react-dom`; system fonts; no CDN requests; no inline scripts, so the server's `default-src 'self'` CSP holds.
   - The UI is a client of the HTTP API only ([ADR-0011](0011-one-json-shape-across-surfaces.md)): every screen reads and writes through the documented routes. The only browser storage is the API token, in `sessionStorage`.
   - The prototype's visual language (palette, serif display headings, sidebar, cards, chips) is reused; its fake data model is not. Integrations that do not exist are labelled *Planned*; nothing is shown as connected unless the config says so.
   - Tests: Vitest + Testing Library for components, Playwright smoke tests (`ui/e2e`) against a real `openstance serve` on the fictional demo dataset.
   - `ui/dist` is embedded in the binary with `rust-embed` behind the `ui` feature (on by default). Without a built `ui/dist` the binary still builds and serves a placeholder page.
   - The landing page (`site/`) uses the same stack but is a separate static site with no API access.
2. **Shortlist and introductions are graph data** (schema v4, migration `0004_workspace.sql`): tables `shortlist` and `intro` (stages `draft`, `asked`, `intro_made`, `met`, `closed`, with note, follow-up date and an optional connector). Rows store anchor person ids and are resolved on read, so a merge keeps them on the right person. They are user data like overrides: `rebuild` never touches them. HTTP routes: `GET/POST /shortlist`, `PATCH/DELETE /shortlist/{person}`, `GET/POST /intros`, `PATCH/DELETE /intros/{id}`, plus `GET /stats` for the sidebar counts ([interfaces.md 3.4](../tech-spec/interfaces.md)).

## Consequences

**Positive**

- The dashboard matches the prototype the maintainer designed, with real data.
- React's ecosystem (Testing Library, react-hooks lint) keeps the UI testable; the bundle stays small.
- Shortlist and intros are durable, local, and in the same file as the rest of the graph; a later CLI or MCP surface can expose them without a migration.

**Negative**

- Two Node projects (`ui/`, `site/`) join a Rust workspace: CI needs a Node job, and the release build must build `ui/` before `cargo build` (the `ui-release` CI job does both). Contributors without Node still get a working binary (placeholder UI).
- Schema v4 adds two tables that are not derived data; they must be kept by every future migration and by `rebuild`.
- Shortlist and intros have HTTP routes only for now; there is no CLI command or MCP tool for them yet.

## Alternatives considered

| Option | Why not |
|---|---|
| Preact or Solid (the plan's suggestion) | Smaller, but the maintainer chose React; at about 105 KB gzipped the size gain does not matter under the 300 KB budget |
| Ship the vanilla-JS prototype | Its data model is fake and `localStorage`-based; rewriting it against the API in plain JS would lose typing and testability |
| Keep shortlist and intros in browser storage | Lost on a browser change, invisible to the CLI and agents, and against "one file holds your network" |
