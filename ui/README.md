# openstance UI

The web UI for `openstance serve`: Vite + React + TypeScript. It is a client
of the local HTTP API (`docs/tech-spec/interfaces.md` section 3) and nothing
else. All data comes from the Rust engine; the UI stores only the API token,
in this tab's `sessionStorage`.

Pages: Search people (parsed filters as editable chips), People, Person
(provenance, inline edits, tags, notes, privacy flags, identities, mutuals,
how connected), Saved views, Playbooks (with the quote gate), Shortlist,
Introductions, Sources (import), Review queue, Integrations, Agent access,
Egress log, Settings.

The visual design is ported from the v0.3 browser prototype (MIT): its CSS
lives in `src/styles/prototype-*.css` with the landing-page rules removed;
`src/styles/app.css` adds the new components. Fonts are system fonts only
(Georgia for display, the system sans for text, SFMono/Consolas for data):
no web-font or CDN requests, because the served UI runs under
`Content-Security-Policy: default-src 'self'`.

## Develop

```sh
cd ui
npm ci
npm run dev                # http://localhost:5173
```

Vite proxies the API paths (`/people`, `/search`, `/edits`, `/playbooks`, ...)
to `http://127.0.0.1:7411` (override with `OPENSTANCE_API=http://127.0.0.1:PORT`),
so the browser stays same-origin and needs no CORS. The proxy rewrites `Host`
to `127.0.0.1:7411`, which passes the server's DNS-rebinding check.

1. Run `openstance serve` in another terminal.
2. Open `http://localhost:5173/#token=<token>` with the token from
   `~/.openstance/api.token` (or paste it into the connect screen).

To call the API directly instead of through the proxy, set
`VITE_API_BASE=http://127.0.0.1:7411` and add the dev origin to the server
config: `serve.cors_origins = ["http://localhost:5173"]`.

Without a graph, `node scripts/mock-api.mjs` serves the fictional test
fixtures on `127.0.0.1:7499`; run `OPENSTANCE_API=http://127.0.0.1:7499 npm run dev`
and open `http://localhost:5173/#token=dev`. It is a dev aid only.

## Token handoff

`openstance serve` prints `http://127.0.0.1:7411/#token=...`. On load the UI
reads the fragment (fragments are never sent to a server), stores the token
in `sessionStorage`, and replaces the URL with `#/` using `history.replaceState`,
so the token is not left in the address bar or in history. `#token=...&route=/review`
opens a page directly. A 401 forgets the token and shows the connect screen.
Settings has "Forget the token in this tab".

## Checks

```sh
npm run lint        # eslint (typescript-eslint, react-hooks)
npm run typecheck   # tsc --noEmit
npm test            # vitest + Testing Library, fetch mocked with interfaces.md-shaped fixtures
npm run build       # tsc + vite build -> dist/ (embedded by openstance-api)
npm run size        # gzipped size of dist/, fails above 300 KB
```

Tests never touch the network: `src/test/mockApi.ts` stubs `fetch`, and
`src/test/fakeXhr.ts` stubs the upload. All fixture people are fictional.

## The quote gate

A playbook run always posts `allow_egress: false` first and shows the plan and
the quote. The only code that sends `allow_egress: true` is the
"Spend $X" button. It appears only for a non-zero, within-budget quote whose
input has not changed since, and it sets `max_usd` to the quoted total (or a
lower cap you set). `src/pages/Playbooks.test.tsx` asserts the
`allow_egress: true` literal appears exactly once in the app source.

## 375 px

`src/styles/layout.test.ts` resolves the cascade for a 375 px viewport with
postcss and fails on any fixed width or grid wider than the content area,
a shell that does not stack, or tables outside a scrolling wrapper. jsdom has
no layout, so this is a CSS-level check; the pages were also measured in
Chrome at 375 px (no horizontal document overflow).

## Contract notes

Types in `src/api/types.ts` follow interfaces.md sections 1 and 3.4 and the
Rust serializers. Two spec/implementation differences the client tolerates:
`Query.since/until/last_contact_before` serialize as unix seconds from Rust but
ISO strings in the spec (both are read, strings are sent), and the review
confirm/reject actions use the `confirm` and `reject` edit ops.
