# The v0.1 prototype (Phase 0)

The v0.1 prototype is a small, dependency-free browser app (MIT) built before the tech spec. It is **Phase 0: done**. It is kept as a reference and a fixture source. It is **not the product**: the Rust core described in [architecture/overview.md](architecture/overview.md) replaces it. This doc records what it does, what to carry forward, what is superseded, and where it should live.

Related: [plan/roadmap.md](plan/roadmap.md), [engineering/testing-strategy.md](engineering/testing-strategy.md), [engineering/dev-setup.md](engineering/dev-setup.md), [product/competitive-analysis.md](product/competitive-analysis.md).

## At a glance

| | |
|---|---|
| Stack | Plain ES modules, HTML, CSS. No bundler, no dependencies. Node 22+ only for the preview server and tests. |
| Storage | One `localStorage` key, `openstance.v1`, per browser origin. Not encrypted. |
| Run | `npm start` serves `dist/` on `http://127.0.0.1:4317` (loopback only; `PORT` to change). Or any static server; or `docker build -t openstance . && docker run --rm -p 8080:80 openstance` (nginx). |
| Tests | `npm test`: 12 tests with Node's built-in runner over `dist/model.js`. |
| Size | About 600 lines: `model.js` (~100), `app.js` (~240), `demo.js` (12 fictional people), `style.css`, `index.html`, `research.md`, `server.mjs`. |
| Licence | MIT. The README notes the name has not been cleared and the package was not published publicly. |

### Files

| File | Role |
|---|---|
| `dist/model.js` | Pure functions: query normalization and synonyms, `searchPeople`, `parseCSV`, `contactsFromCSV` (header aliases), `mergeContacts` (dedupe), `exportCSV` (formula-injection safe), `safeURL`, `isStale`, `draftIntro`. Constants `STORAGE_KEY`, `STAGES`, `FIELDS`. |
| `dist/app.js` | UI, state, events: search, people list, shortlist, person detail, intro board, data sources (import preview), settings (backup/restore/reset). |
| `dist/demo.js` | 12 clearly fictional people (Bengaluru-heavy AI founders, investors, a recruiter, a designer) with `via`, relationship, dates and notes. |
| `dist/research.md` | Sourced competitive research and staged roadmap, dated 2026-10-20. **Corrects some overview claims** (see below). |
| `server.mjs` | Loopback-only static server with path-traversal guard and `nosniff`. Not a production server. |
| `tests/model.test.mjs` | The 12 behavior tests. |
| `Dockerfile` | `nginx:alpine` serving `dist/`. |

## What it does

### Data model

A contact is a flat object with `FIELDS`:

```
name, role, company, location, tags, email, url, via, relationship,
lastContact, notes, source, verifiedAt   (+ id, demo)
```

`relationship` is one of `Strong | Warm | Weak | Unknown`, supplied by the user or the CSV. `via` is a user-asserted connector name. Nothing is derived from interactions; there are no interactions.

### Features

- **CSV import with preview.** RFC-4180-ish parser handling BOM, quoted commas, escaped quotes, CRLF and multiline values; actionable errors for unclosed quotes, missing name column, or no named rows. Limits: 5 MB and 10,000 rows per file.
- **Header aliases** for LinkedIn and Google Contacts exports, including the LinkedIn `Notes:` preamble before the real header row:

  | Field | Accepted headers (normalized) |
  |---|---|
  | name | `name`, `full name`, `display name` (or `first name`/`given name` + `last name`/`family name`) |
  | role | `role`, `position`, `title`, `job title`, `organization 1 title` |
  | company | `company`, `organization`, `organization 1 name` |
  | location | `location`, `city` |
  | tags | `tags`, `skills`, `expertise` |
  | email | `email`, `email address`, `e mail 1 value`, `email 1 value` |
  | url | `url`, `linkedin url`, `profile url`, `website` |
  | via | `via`, `introduced by`, `connector` |
  | relationship | `relationship`, `strength` |
  | lastContact | `lastcontact`, `last contact`, `last contact date` |
  | notes | `notes`, `note`, `bio` |

  LinkedIn's `Connected On` is deliberately **not** mapped to last contact (being connected is not contact).
- **Dedupe** by normalized email or profile URL (trailing slash stripped). **Never by name alone.**
- **Search** over structured fields only (name, role, company, location, tags). Freeform notes are excluded so "No evidence of AI experience" does not make someone an AI match. Query normalization: NFKD, strip diacritics, lowercase, stop-word removal, a small synonym map (`vc → investor`, `bangalore → bengaluru`, `sf → san francisco`, `devtools → developer tools`, plurals). Strict (all terms) or partial matching; filters for location, direct connection (no `via`) and shortlist. Results show **matched and missing terms** per person. Sort: matched count, then direct before indirect, then name.
- **Freshness**: a record is stale after 90 days; unknown or invalid dates are "unknown", not stale.
- **Shortlist** of saved people.
- **Intro drafts**: a template that asks the connector to *check whether the person would welcome an intro* (indirect) or a direct reconnect note. Nothing is sent.
- **Intro board** with stages `Shortlisted → Asked for intro → Introduced → Meeting booked` and a displayed (not scheduled) follow-up date.
- **Export**: CSV with spreadsheet formula injection neutralized (leading `'` on cells starting with `= + - @`); full JSON backup and restore with validation; reset.
- **Safety**: `safeURL` allows only `http`/`https` profile links; links open only on explicit click.
- **Honesty in the UI**: the demo network is labeled fictional; integrations are labeled as roadmap, not shown as connected; the graph illustration is labeled decorative.

## What to keep

| Asset | Why | Where it goes in the Rust design |
|---|---|---|
| **The 12 tests' intent** | They encode product rules that still hold | Port as Rust tests in `openstance-import` and `openstance-core` (table below) |
| **Header aliases** | Real LinkedIn/Google column names, including the LinkedIn preamble | `linkedin_csv` importer and the future generic `csv` importer; alias table as data in `openstance-import` |
| **CSV edge cases** (BOM, quoted commas, CRLF, multiline, unclosed quote) | Real exports hit all of these | Fixtures under `fixtures/` for every CSV-based importer |
| **"Connected On" is not contact** | Avoids inflating recency | `linkedin_csv` emits a `connection` interaction (kind `connection`), which does not count as a reply or meeting; whether it counts toward R/F is decided in [tech-spec/scoring.md](tech-spec/scoring.md) |
| **Never merge on name alone** | Two people share names | Codified in [ADR-0009](decisions/0009-conservative-entity-resolution.md) |
| **Notes are not expertise** | Negated prose must not match | Notes (`override` field `note`) are not indexed as facts in `person_fts` by default; evidence lines come from fields and interactions |
| **Formula-injection-safe CSV export** | Exported CSVs are opened in spreadsheets | `openstance export --format csv` applies the same rule; JSON export preserves raw values |
| **`safeURL`** | Profile URLs are attacker-controlled | Web UI (Phase 4) and any HTML/markdown rendering of `identity` URLs |
| **Fictional demo network** | Safe, labeled sample data | Converted once into `fixtures/legacy-v0.1/` as a CSV source (see [engineering/testing-strategy.md](engineering/testing-strategy.md)); names and companies seed the synthetic multi-source fixture cast |
| **Matched / missing terms** | Makes search inspectable; the research's top recommendation | Search JSON reports which `Query` filters and terms each result met (see [tech-spec/search.md](tech-spec/search.md)) |
| **Refuse to silently widen** | "AI founders in Paris" returns zero in strict mode rather than unrelated people | Structured filters are hard constraints; extracted filters are shown and editable |
| **Intro draft tone** | Asks the connector to check interest; never assumes consent | Template for the `brief` playbook's suggested ask |
| **Intro stages, follow-up dates** | Follow-through is a researched differentiator | Not in Phases 1 to 4; open question Q7 in [risks-and-open-questions.md](risks-and-open-questions.md) |
| **`dist/research.md`** | Sourced, dated, cautious competitive facts | Basis for [product/competitive-analysis.md](product/competitive-analysis.md); copy into `docs/` history or keep with the archived prototype |
| **UI copy and empty states** | "Not supplied", "Role unknown", "Record last checked", honest roadmap labels | Phase 4 web UI tone |

### Research corrections the docs adopt

`dist/research.md` is more careful than the product overview in several places. The docs follow the research:

| Overview says | Research says | Docs use |
|---|---|---|
| Happenstance free tier is "about 15 searches a month" | Older third-party figure; the official pricing page gives no numeric free allowance | Do not state a number; call it unconfirmed |
| Pro is $30/mo or $24 yearly | Pricing page shows $24/person/month billed annually; indexed description lists $30 monthly | $24/mo annual, $30 monthly, per their page |
| Happenstance profiles cannot be corrected; prose-only reasons | Their listing says they attempt to provide concrete evidence | Do not claim Happenstance lacks evidence or privacy controls |
| Privacy is a differentiator | Happenstance says it uses metadata only, previews exports locally, does not sell data | The difference is that the app and store stay under the user's control, not that the competitor is careless |
| MCP is a differentiator | Happenstance already has MCP, API and CLI | MCP is table stakes; the difference is that it is free and local |

## What is superseded

| v0.1 | Replaced by | Reference |
|---|---|---|
| Browser `localStorage` JSON blob | One SQLite file, `~/.openstance/graph.db` | [ADR-0003](decisions/0003-sqlite-fts5-sqlite-vec.md), [tech-spec/data-model.md](tech-spec/data-model.md) |
| Flat contact object | `source`, `person`, `identity`, `interaction`, `interaction_person`, `tie`, `override`, `view`, `enrich_log` + FTS5 + vec | [tech-spec/data-model.md](tech-spec/data-model.md) |
| CSV-only import | Importer trait: `mbox`, `linkedin_csv`, `vcf` (Phase 1), `ics`, `x_archive` (Phase 3), more later | [tech-spec/ingestion.md](tech-spec/ingestion.md) |
| Dedupe on import by email/URL | Entity resolution with confidences, flagged merges, `review`, merge/split overrides | [tech-spec/entity-resolution.md](tech-spec/entity-resolution.md) |
| Manual `relationship` Strong/Warm/Weak | Derived score R, F, C, M, B with presets | [tech-spec/scoring.md](tech-spec/scoring.md) |
| User-asserted `via` | Derived ties with evidence; `how_connected`, `path_to` | [tech-spec/playbooks.md](tech-spec/playbooks.md) |
| Keyword + synonym search | Hybrid: RuleParser, SQL filters, FTS5, embeddings, fusion, optional rerank | [tech-spec/search.md](tech-spec/search.md) |
| 90-day staleness flag | Recency component and dated evidence lines | [tech-spec/scoring.md](tech-spec/scoring.md) |
| Edit-in-place records | Overrides that survive rebuild | [tech-spec/data-model.md](tech-spec/data-model.md) |
| JSON backup/restore | The SQLite file is the backup; `openstance export --format csv\|json\|vcf\|parquet` | [tech-spec/interfaces.md](tech-spec/interfaces.md) |
| Browser UI as the product | CLI and MCP first; optional local web UI over the HTTP API in Phase 4 | [tech-spec/interfaces.md](tech-spec/interfaces.md) |
| Node static server on :4317 | `openstance serve` on `127.0.0.1:7411` with bearer token | [architecture/security-and-threat-model.md](architecture/security-and-threat-model.md) |
| Docker/nginx deploy | Single binary via cargo-dist | [engineering/ci-and-release.md](engineering/ci-and-release.md) |

## Mapping the 12 tests to the Rust suite

| v0.1 test | Rust equivalent | Phase |
|---|---|---|
| Strict search respects every term; aliases (`Bangalore` = `Bengaluru`, `founders` = `founder`) | RuleParser + search test: location/title filters are hard constraints; normalization and synonym table | 1 |
| Unsupported constraints do not silently expand results | Search returns zero results for an unmatched hard filter; partial mode marks missing terms | 1 |
| Negative freeform notes do not manufacture expertise | A person whose note says "not an AI founder" is not returned for "AI founder" unless a field or interaction supports it | 1 |
| Direct, location and shortlist filters compose | `Query` filters compose (company, location, `met_in_person`, `replied`, view scope) | 1 |
| CSV: BOM, quoted commas, quotes, CRLF, multiline | Shared CSV reader tests in `openstance-import` | 1 |
| LinkedIn preamble + aliases; `Connected On` is not last contact | `linkedin_csv` fixture test | 1 |
| Invalid CSV gives an actionable error | Importer error messages name the missing column; `detect()` rejects non-matching files | 1 |
| Dedupe uses email/profile URL, never only name | Entity resolution fixture: same name, different emails stays two people | 1 |
| CSV export defends against formula injection | `export --format csv` test | 1 |
| Freshness: stale / current / unknown | Recency R golden tests; a person with no dated interaction is shown as "unknown" recency, not stale (exact R value defined in [tech-spec/scoring.md](tech-spec/scoring.md)) | 1 |
| Profile URLs reject executable schemes | Web UI and markdown rendering tests | 4 |
| Intro drafts ask the connector to check interest | `brief` playbook output test | 3 |

## Where it should live

**Recommendation: move the prototype into this repository under `prototype/v0.1/`, frozen, with a short README pointer.**

```
openstance/
  prototype/
    v0.1/            # unchanged copy of the v0.1 source package
      README.md      # original README + a banner: "Frozen reference. The product is the Rust workspace."
      dist/ tests/ server.mjs package.json Dockerfile LICENSE CONTRIBUTING.md
  crates/ ...
  fixtures/legacy-v0.1/   # demo network converted to CSV once
```

Reasons:

- The tests, header aliases and research are referenced by Phase 1 work; keeping them next to the Rust code avoids link rot.
- It is small (about 600 lines) and has no dependencies, so it costs nothing to keep.
- Freezing it avoids two products. Its `CONTRIBUTING.md` priorities (IndexedDB storage, local service adapters) are superseded and must not attract contributions; the banner says so.

Rules for `prototype/v0.1/`:

1. No feature work. Only fixes needed to keep `npm test` green.
2. Not part of the Rust build or release. A weekly CI job runs `npm test` (see [engineering/dev-setup.md](engineering/dev-setup.md) and [engineering/ci-and-release.md](engineering/ci-and-release.md)).
3. Tag the import commit `v0.1.0` so the prototype is recoverable even if the folder is later deleted.

**Alternative:** archive it as a release asset (a `v0.1.0` tag with the zip attached) and delete it from the tree after Phase 1, once the tests are ported and `fixtures/legacy-v0.1/` exists. Choose this if the folder confuses contributors. Either way, do not keep it in the repo root, and do not ship it in the binary.

## Known limits of the prototype (for context)

- Single browser, single origin; no sync; clearing site data deletes everything.
- No interactions, so no derived relationship strength or verified paths.
- No model, embeddings, enrichment, OAuth, API or MCP.
- `localStorage` quotas can be below 5 MB; large imports fail with an error.
- Search is keyword-level; no negation, boolean logic or numeric comparisons.
