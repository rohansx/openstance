# treg integration notes (for the Rust `TregEnricher`)

Source: `dev/treg/src` (treg 0.27.0). Read-only checks were run against the local instance at
`http://127.0.0.1:18790`. Marked **[verified]** = seen on the live instance; **[code]** = read from
source, not exercised (sign-up and calls write state, so they were not run).

## 0. Corrections to `docs/tech-spec/enrichment.md` §3.2
- Auth header is **`X-Treg-Token: <token>`**, not `Authorization: Bearer`. (Bearer is accepted only on
  the `/mcp` front door.) A user/identity token also needs `X-Treg-Org: <slug>`; a team token or `treg_…`
  API key has the team baked in. [code: `domain/identity/access.py::require_member`]
- The hosted base URL is `https://treg.to` (the docs and CLI use it); `api.treg.to` appears nowhere in treg's source.
- Routed tool ids (all `POST`, JSON body) [verified]:

| `EnrichRequest` | treg id | body (identity variants) | contract `output` |
|---|---|---|---|
| Profile | `treg.people.enrich` | `{email}` \| `{linkedin_url}` \| `{linkedin_handle}` \| `{full_name,domain}` \| `{first_name,last_name,domain}` | full_name*, first_name, last_name, title, company, company_domain, linkedin_url, location |
| FindEmail | `treg.people.email.find` | `{full_name,domain}` \| `{first_name,last_name,domain}` \| `{linkedin_url}` \| `{linkedin_handle}` | email*, confidence (0..1), first_name, last_name, verified |
| VerifyEmail | `treg.people.email.verify` | `{email}` | valid*, status, score |
| Company | `treg.companies.enrich` | `{domain}` \| `{website}` \| `{name}` \| `{linkedin_url}` \| `{linkedin_handle}` \| `{email}` | name*, domain, website, description, industry, employees, founded, location, linkedin_url |
| WebSearch | `treg.web.search` | `{q, limit?}` | results* (provider-native list), count |

Profile "by company" means a company **domain**. If you only have a company name, call `treg.companies.enrich {name}` first.
(`*` = required. A miss is defined as the required field being null or empty.)

## 1. Calling the API

All calls go to `{base}`. Catalog reads are **open** (no token). Calls need a token.

**Search** [verified] `GET /catalog/search?q=find+work+email&limit=25` →
`{"query":…, "count":25, "total":118, "results":[{"id","provider","name","summary","method","path","kind":"routed|data","call_template","cost":{…}}…]}`

**Get + price** [verified] `GET /catalog/endpoints/{id}` → `{endpoint, provider, siblings, routing, call_template, example_response, hints}`
- `endpoint.cost` holds `{"type":"per_call|per_success|per_result|free", "value", "currency":"USD|credit", "unit", "note", "usd": <float USD>}`.
  `cost.usd` is the USD figure to quote (credits are already converted). Example: `hunter.people.email.verify`
  → `"usd": 0.01225`.
- For routed (`kind:"routed"`) ids, `routing.plan` is the ranked list of children with prices:
  `[{"endpoint_id":"contactout.people.email.verify","accepts":[["email"]],"usd":0.0}, …]`. `endpoint.cost.usd`
  is the cheapest child, so it is a lower bound. The note says "misses are billed too … up to X-Treg-Route-Max-Cost
  (default $1)". Upper bounds seen today: email.find $0.15, verify $0.024, people.enrich $0.38, companies.enrich $0.38,
  web.search $0.16.
- **quote() without spending.** Take `max(routing.plan[].usd)` (spec: use the upper bound, `basis=estimate`),
  and send `X-Treg-Route-Max-Cost: <budget usd>` on the call so treg enforces the ceiling. Treg answers 402
  `route_max_cost` (nothing charged) if no child fits. Cache the result per process. No contact data is sent.
- With a token, `GET /catalog/endpoints/{id}/access` returns how *this team* would be served. [code]
  For a routed id: `{"tier":"routed","plan":[{"endpoint_id","provider","tier":"tool|credential|platform|anonymous",
  "price_micro","expected_cost_per_hit_micro","hit_rate",…}]}`. For a direct id:
  `{"tier":"credential|platform|…","estimated_cost_micro":…}`. Own-key tiers (`tool`, `credential`) cost $0.
- Server margin: `TREG_PLATFORM_MARGIN` (default 0.0), applied to the ledger. Units are micro-USD (1e-6).

**Call** [code: `routers/call.py`, `application/call/route.py`]
```
POST {base}/call/treg.people.email.find
X-Treg-Token: <token>
Content-Type: application/json
X-Treg-Route-Max-Cost: 0.05          # optional USD ceiling for the whole call
X-Treg-Route-Waterfall: 0            # optional: stop at first miss
X-Treg-Route-Prefer: hunter          # optional; also X-Treg-Route-Exclude
{"full_name":"Patrick Collison","domain":"stripe.com"}
```
Hit (200):
`{"output":{"email":"…","confidence":0.97,…},"raw":<provider body>,"_treg":{"served_by":"hunter.people.email.find","provider":"hunter","tier":"credential","outcome":"hit|weak","tried":[{"endpoint_id","provider","outcome","status","charged_micro"}],"charged_micro":0}}`.
Miss (200): `output` has every field null, `raw:null`, and `_treg.outcome:"miss"`.
Response headers: `X-Treg-Call-Id`, `X-Treg-Cost-Micro` (actual cost), `X-Treg-Served-By`, `X-Treg-Providers-Tried`, `X-Treg-Route-Outcome`.
Errors return `{"detail": …}`:
- 401 bad or missing token
- 402 `insufficient_balance` (`balance_micro`, `estimated_cost_micro`, `topup_url`) or `route_max_cost`
- 422 `route_no_candidate`. On a self-hosted registry with no keys this is the usual result.
- 503 `provider_capacity_unavailable`
- 429 (with `Retry-After`)
- 202 with `_treg.outcome:"pending"` when an async child is still running

A direct (non-routed) id relays the vendor's request and response unchanged, e.g. `GET /call/hunter.people.email.verify?email=a@b.com`.
Path and query params ride in the query string, and the body is passed through.

## 2. Local mock end-to-end ($0, no external network)

**Short answer:** no supported API lets you point a *catalog/routed* id at a custom URL.
- Provider base URLs are hard-coded in `src/treg/oauth_providers.py`, e.g. hunter → `https://api.hunter.io/v2`.
- The catalog loads from the package dir (`src/treg/catalog/*.yaml`).
- A "your own tool" (`POST /tools`) is reachable only by name (`/call/<tool>/<path>`) or by URL passthrough. It never
  replaces a catalog id's upstream unless its host equals the provider's real host.
- `POST /tools` refuses loopback, private and `localhost` base URLs (422). Every relay also refuses private IPs unless
  `TREG_PROXY_SSRF_CHECK=false`.

**Option A: full catalog/routed path, config only (recommended; [code], not yet run).** treg's credential ladder serves an
id from a team credential named after the provider (tier `credential`, never billed). Make the provider's real
hostname resolve to a mock inside Docker, and make treg trust the mock's TLS cert:
1. Make a throwaway CA and a server cert with SANs `api.hunter.io`, `api.search.tinyfish.ai`. Use the mkcert or openssl
   tool. The mock serves HTTPS on :443.
2. Add a `mock` service to `compose.yaml`, on the same network as treg, with
   `networks: {default: {aliases: [api.hunter.io, api.search.tinyfish.ai]}}`.
   - Hunter paths, all `GET` under `/v2`, key header `X-API-KEY`: `/email-finder`, `/email-verifier`, `/people/find`, `/companies/find`.
   - Tinyfish search: `GET /` on `api.search.tinyfish.ai`, key header `X-API-Key`.
   - Canned bodies already exist: `src/src/treg/catalog/examples/hunter.people.email.find.json`, `…email.verify.json`,
     `…people.enrich.json`, `…companies.enrich.json`, `tinyfish.web.search.json`. These are the real vendor shapes, so
     treg's adapters map them into `output`.
3. Give the treg service `TREG_PROXY_SSRF_CHECK=false` (the mock is on a private IP) and
   `SSL_CERT_FILE=/certs/ca.pem` (mounted read-only).
   httpx 0.28.1 in the image reads `SSL_CERT_FILE` (checked in the container). This replaces the certifi bundle,
   which is fine offline.
4. Sign in (section 3). Then store dummy keys, one per provider; no provider probe is made:
   `POST /secrets` `{"name":"hunter","value":"dummy","kind":"env"}` and `{"name":"tinyfish","value":"dummy","kind":"env"}`.
5. Call `treg.people.email.find` etc. with `X-Treg-Route-Prefer: hunter` (or `tinyfish` for search). Expect
   `_treg.tier:"credential"`, `X-Treg-Cost-Micro: 0`.
   With no platform keys configured, only own-credential children are candidates, so routing goes to the mock. Other
   providers are dropped, and `route_no_candidate` means a secret is missing.

**Option B: simplest, but skips catalog routing.** Register a vault tool against a DNS-named mock and call it by name:
- Set `TREG_PROXY_SSRF_CHECK=false`. Add a compose service `mock` (plain HTTP).
- `POST /secrets {"name":"mock-key","value":"x"}` → `id`.
- `POST /tools {"name":"mock","base_url":"http://mock:8080","bindings":[{"secret_id":<id>,"injector":"env","location":"header","name":"Authorization","format":"Bearer {secret}"}]}`.
  The hostname `mock` passes the registration check, because only literal private IPs and `localhost` are refused.
  CLI: `treg tool add mock --base-url http://mock:8080 --secret <id>`.
- Call `/call/mock/<path>`. Bodies are relayed verbatim. Prices still come from `GET /catalog/endpoints/{id}` (open, local
  YAML, no network).
- This tests auth, transport, headers and quoting, not routed `output` mapping.

In both options, CI unit tests should keep using `MockEnricher` and a wiremock-style server for treg's own response shapes (above).

## 3. Headless token on the local dev instance

**Email OTP (dev mode)** [code]. `expose_dev_code` is true here because the instance runs with
`TREG_EMAIL_DEV_MODE=true` and a sqlite DB. Avoid `@demo.treg.local` and `@public-demo.treg.local` addresses.
No captcha is required unless Turnstile is configured.
```
POST /auth/email/start   {"email":"ci@openstance.test"}           → {"sent":true,"email":…,"dev_code":"123456"}
POST /auth/email/verify  {"email":…,"code":"123456"}               → {"token":"<bootstrap identity token>","email":…} + Set-Cookie treg_session
POST /orgs   X-Treg-Token:<bootstrap>   {"name":"openstance-ci"}   → {"org":"openstance-ci","org_id":1,"role":"owner","token":"<team token>"}
POST /orgs/{org_id}/api-keys  X-Treg-Token:<team token>  {"name":"enricher-tests"} → {…,"secret":"treg_…"}   # optional stable key
```
- The bootstrap token (7-day TTL) is for onboarding only. Use the `token` from `POST /orgs`, or the `treg_…` key, as `X-Treg-Token`.
- On a later login of the same user, `POST /orgs` makes another team. Instead, use the `treg_session` cookie from the verify
  call: `GET /auth/cli-token` with header `X-Treg-Org: <slug>` returns a team-baked token. (A bootstrap token is refused there.)
- Do not send `ngrok-skip-browser-warning: 1` together with a `python-httpx/` User-Agent, or you get the old-CLI 426 gate.
- New teams get the signup gift balance. It is irrelevant for own-key calls.

**Easier: single-user mode** [code: `api.py::_bootstrap_single_user`].
- Add `TREG_SINGLE_USER=true` and `TREG_SINGLE_USER_TOKEN_FILE=/data/local-token` to the container env.
- On boot treg creates user + team "Personal" (owner) and writes a stable per-team token: `docker compose exec treg cat /data/local-token`.
- Use it directly as `X-Treg-Token`; no `X-Treg-Org` is needed.
- It works only with a sqlite DB and a loopback `TREG_PUBLIC_URL`, which is true here (`http://localhost:18790`).

## 4. Licence (`src/LICENSE`)
Apache-2.0 plus "Additional Terms" (Superdesign, 2026):
- Allowed: use, modify and redistribute, including commercial use inside your own organisation. Running your own registry
  for your own team is "expressly permitted". Running it locally for tests is fine.
- Not allowed without written permission (jason@superdesign.dev): offering it, original or modified, as a hosted, managed
  or embedded service to third parties. That includes making it "a component of a product or service that is sold,
  licensed, or otherwise commercially distributed to others".
- For openstance: talk to treg over HTTP only. Do not bundle or ship treg's code, Docker image or catalog YAML/examples
  inside openstance. (`dev/treg/src` is already gitignored.) If you copy example JSON into openstance fixtures, keep the
  Apache NOTICE and attribution, or write your own fixtures.
- Provider ToS still apply to any real-key calls. The mock setup makes none.
