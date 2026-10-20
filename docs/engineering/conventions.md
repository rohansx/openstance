# Engineering conventions

Rules for code, SQL, logging, naming, commits and data hygiene in the openstance repo. They exist so that a contributor can add an importer or enricher without reading the whole codebase, and so that nothing in the repo or the logs can leak a user's contacts.

Related: [dev-setup.md](dev-setup.md), [testing-strategy.md](testing-strategy.md), [ci-and-release.md](ci-and-release.md), [../architecture/crates-and-dependencies.md](../architecture/crates-and-dependencies.md), [../architecture/security-and-threat-model.md](../architecture/security-and-threat-model.md), [../tech-spec/data-model.md](../tech-spec/data-model.md), [../decisions/README.md](../decisions/README.md).

## Crate boundaries

- Dependency direction is strict: `openstance` (CLI) -> `openstance-mcp` / `openstance-api` -> `openstance-core` -> traits in `openstance-import` / `openstance-enrich` / `openstance-model`. Core never names a concrete provider (no `use openstance_enrich::treg` in core). Wiring of concrete implementations happens only in the `openstance` binary crate.
- Trait crates export the trait, its request/response types, and a `Null` implementation. Concrete implementations sit behind cargo features in the same crate.
- A new external dependency in `openstance-core` needs a sentence of justification in the PR. Core should stay small: `rusqlite`, `serde`, `thiserror`, `tracing`, `unicode-normalization`, `strsim`, `time`, `toml` and little else.
- `cargo deny` enforces the license allowlist (MIT, Apache-2.0, BSD-2/3, ISC, Zlib, Unicode-3.0, MPL-2.0 for file-level copyleft only) and bans duplicate `libsqlite3-sys`.

## Sync core, async servers

- `openstance-core`, `openstance-import`, `openstance-enrich` traits and `openstance-model` traits are **synchronous**. No `async fn` in their public APIs and no `tokio` dependency.
- `tokio` appears only in `openstance-mcp` and `openstance-api` (and in the CLI only to start those servers). They call core through `tokio::task::spawn_blocking`, holding one `rusqlite::Connection` per blocking task from a small pool (`r2d2_sqlite` or a hand-rolled `Mutex<Vec<Connection>>`). Never hold a SQLite connection across an `.await`.
- Network-calling implementations (treg HTTP, Ollama, OpenAI, Anthropic) use a blocking HTTP client (`ureq`) so they satisfy sync traits. If an async client is ever needed it is wrapped inside the implementation, not leaked into the trait.
- One writer at a time: write paths go through `Graph::write(|tx| ...)`, which opens an `IMMEDIATE` transaction. Readers use WAL snapshots.

## Error handling

- Libraries (`openstance-*` except the binary) use `thiserror` with one `Error` enum per crate and `pub type Result<T> = std::result::Result<T, Error>`.
- The binary uses `anyhow` with `.context("...")` at every boundary where the user needs to know what was being attempted ("importing fixtures/mbox/work.mbox").
- Variants are about what the caller can do, not where it happened: `Error::BudgetExceeded { quoted_micro_usd, cap_micro_usd }`, `Error::EgressNotAllowed { quote }`, `Error::UnsupportedFile { path, sniffed }`, `Error::Schema { found, expected }`.
- No `unwrap()`/`expect()` outside tests, `xtask`, and invariants documented in an `expect("...")` message that states why it cannot fail. Clippy enforces `unwrap_used` and `expect_used` as `warn` in lib crates; CI denies warnings.
- No panics across FFI or in importers on bad input. Importers return `Error::Parse { source_ref, line, reason }` per bad record via the `Sink`'s error channel and keep going; `ImportStats` reports skipped counts.
- Error messages never include contact data (emails, names, subjects). Use identifiers: `person_id`, `source_id`, line numbers, file paths the user supplied.
- CLI exit codes are defined once in [interfaces](../tech-spec/interfaces.md#14-exit-codes): `0` ok, `1` internal, `2` usage, `3` egress needs approval / disabled, `4` budget, `5` not found, `6` ambiguous person, `7` view expired or not for my key, `10` database busy or schema newer than binary. Map every error type to exactly one code in the CLI crate.

## Logging with tracing

- Use `tracing` macros everywhere; `println!` only in the CLI's output layer. Libraries never install a subscriber.
- The CLI installs `tracing-subscriber` with `EnvFilter` from `OPENSTANCE_LOG` (default `warn` for CLI commands, `info` for `serve`/`mcp`). `OPENSTANCE_LOG_FORMAT=json` switches to JSON lines.
- `openstance mcp` speaks MCP over stdio, so **all** logs go to stderr there, never stdout.
- Spans: one span per command, per import (`source_id`, `kind`), per playbook run (`playbook`, `run_id`), per HTTP request (`method`, route template, not the full query string).
- **Never log contact data at any level**: no emails, names, phone numbers, handles, subjects or bodies, not even at `trace`. Log counts, ids, durations and hashes. A `Redacted<T>` wrapper implements `Debug`/`Display` as `"[redacted]"`; types that carry contact data (`RawIdentity`, `RawInteraction`, `Person`) implement `Debug` manually to redact fields. A unit test asserts `format!("{:?}", person)` contains no fixture email.
- Enrichment egress is recorded in the `enrich_log` table (the audit trail), not in logs.

## SQL and migrations

- Schema lives in `crates/openstance-core/migrations/NNNN_short_name.sql`, numbered from `0001`. Migrations are embedded with `include_str!` and applied in order inside one transaction each; the applied version is recorded in the `meta` table (`schema_version` and one `migration.NNNN_name` key per applied file, see [data model](../tech-spec/data-model.md#8-migrations)) and mirrored to `PRAGMA user_version`.
- Migrations are forward-only and append-only: never edit a migration that has shipped in a release. Fix forward with a new file.
- Opening a database whose version is newer than the binary knows refuses with exit code 10 (`schema_too_new`) and a message to upgrade. Opening an older one runs pending migrations after making a backup copy `graph.db.bak-v<N>` (SQLite online backup API).
- Derived tables (`person`, `tie`, `person_fts`, `person_vec`, cached scores) can be rebuilt; a migration that changes their shape may simply drop and let `rebuild` repopulate. Never write a migration that drops or rewrites `source`, `identity` rows from sources, `interaction`, `interaction_person`, `override`, `view` or `enrich_log` data.
- Style: lowercase SQL keywords are fine but be consistent within a file; table names singular (`person`, `tie`); timestamps are `INTEGER` unix seconds UTC, columns named `*_at`; booleans `INTEGER NOT NULL DEFAULT 0` named `is_*` or as the spec names them (`do_not_share`, `no_enrich`); foreign keys declared and `PRAGMA foreign_keys = ON` at open.
- Every query is a `const` string or built with `rusqlite` parameters. **No string interpolation of values into SQL**, ever. FTS5 match expressions from user text go through `fts::escape_query()`, which quotes terms.
- `person_fts` is a regular content-storing FTS5 table keyed by `rowid = person.id`; any code that changes a person's searchable fields calls `fts::reindex_person(tx, id)` (delete + insert) in the same transaction. `rebuild` drops and refills it.
- `PRAGMA journal_mode=WAL; synchronous=NORMAL; foreign_keys=ON; busy_timeout=5000` are set in `db::open` and nowhere else.
- Schema changes update [../tech-spec/data-model.md](../tech-spec/data-model.md) in the same PR, and `cargo xtask schema-dump` output must match.

## Naming

| Thing | Convention | Example |
|---|---|---|
| Crates | `openstance-<area>` kebab-case | `openstance-import` |
| Importer kinds | snake_case, stable, stored in `source.kind` | `mbox`, `linkedin_csv`, `x_archive`, `vcf`, `ics` |
| Enricher names | lowercase, stored as `enrich:<name>` | `enrich:treg` |
| Model config strings | `<provider>:<model>` | `fastembed:bge-small-en-v1.5`, `ollama:qwen2.5:7b` |
| CLI commands | single lowercase verbs/nouns, subcommands for groups | `openstance view export` |
| CLI flags | kebab-case long flags; short flags only for very common ones | `--no-enrich`, `--limit`, `-n` |
| MCP tools | snake_case, fixed set | `search_people`, `get_person`, `how_connected`, `path_to`, `brief`, `list_views`, `search_view`, `quote_enrich` |
| JSON fields | snake_case, identical across CLI `--json`, MCP and HTTP | `last_contact`, `score_components` |
| Money | integer micro-USD in code and storage (`cost_micro_usd: u64`); `max_usd` floats only at the user boundary, converted immediately | |
| Time | `i64` unix seconds in storage; `time::OffsetDateTime` in code; RFC 3339 in JSON | |
| Config keys | snake_case TOML under sections | `enrich.max_usd_per_run` |
| Files | `*.ost` for view files; `graph.db` for the database | |

Renaming a stored string (importer kind, enricher name, JSON field, MCP tool) is a breaking change; see [ci-and-release.md](ci-and-release.md#versioning).

## Feature flags

Cargo features keep optional providers out of minimal builds and let contributors work on one importer without compiling all of them.

| Crate | Features | Default |
|---|---|---|
| `openstance-import` | `mbox`, `linkedin`, `vcf`, `ics`, `x_archive`, `csv`, later `imap`, `whatsapp`, `slack`, ... | `mbox`, `linkedin`, `vcf`, `ics`, `x_archive`, `csv` |
| `openstance-model` | `fastembed`, `ollama`, `openai`, `anthropic`, `test-util` | `fastembed` |
| `openstance-enrich` | `treg`, `ownkeys`, `test-util` | none (Null only) |
| `openstance-api` | `ui` (embed `ui/dist`) | `ui` |
| `openstance` (bin) | `full` (everything stable), `embed-none`, `sqlcipher` | `full` minus `sqlcipher` |

Rules:

- Features are additive. Enabling one never changes behavior of code paths not using it.
- Every feature combination that CI tests is listed in `ci.yml`; at minimum: default, `--no-default-features`, `--all-features`.
- `test-util` features expose mocks (`MockEnricher`, hash embedder, `TestGraph`) and are only ever enabled from `[dev-dependencies]`.
- Runtime choice (which embedder, which enricher) comes from config, not features; features only decide what is compiled in. A config naming a provider that was not compiled in fails at startup with a clear message.

## Public API and docs

- Every public item in lib crates has a doc comment; `#![warn(missing_docs)]` in lib crates.
- Traits that third parties implement (`Importer`, `Sink`, `Enricher`, `Embedder`, `QueryParser`, `Reranker`, `Playbook`) carry a doc example and are considered stable within a minor version.
- Architecture-level choices get an ADR in [../decisions/](../decisions/README.md). If a PR contradicts an ADR, it updates or supersedes the ADR in the same PR.

## Commits and pull requests

- Branch from `main`; branch names `feat/<topic>`, `fix/<topic>`, `docs/<topic>`, `chore/<topic>`.
- Commit messages follow Conventional Commits: `type(scope): summary` with types `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`, `ci`, `chore`, and scopes matching crates or areas (`core`, `import`, `enrich`, `model`, `mcp`, `api`, `cli`, `ui`, `schema`). Breaking changes add `!` and a `BREAKING CHANGE:` footer. These feed the changelog.
- PRs are small and single-purpose. The PR template asks for:
  - what changed and why, linked issue
  - how it was tested (commands run, fixtures added)
  - checkboxes: schema migration added and data-model doc updated; golden/eval baselines updated intentionally; new egress path quoted, budgeted and logged to `enrich_log`; no real contact data; changelog entry if user-visible
- Squash-merge to `main` with the PR title as the commit subject. `main` is always releasable; CI must be green.
- At least one maintainer review. Changes to egress, view encryption, the HTTP auth layer or `do_not_share`/`no_enrich` handling need a second review from someone listed in `CODEOWNERS` for `security`.

## No real contact data

The product's whole premise is that contacts stay on the user's machine. The repo must never hold any.

- Fixtures, tests, docs, screenshots, demo recordings and issue reproductions use only the fictional cast described in [testing-strategy.md](testing-strategy.md#fixtures): `example.*` / `.test` / `.example` / `.invalid` domains, fictional phone ranges, invented names and companies.
- `cargo xtask scan-fixtures` (pre-commit hook and CI) fails on email domains outside the allowlist, phone numbers outside fictional ranges, and files that look like real exports (e.g. a `Connections.csv` above the fixture size limit) anywhere in the tree except `target/`.
- `.gitignore` includes `*.mbox`, `*.ost`, `graph.db*`, `config.toml`, `age.key`, `.openstance/` at any depth, with explicit `!fixtures/**` allows.
- Bug reports: the issue template asks for counts, headers and shapes, never rows. Maintainers delete issue attachments that contain real data and ask for a synthetic reproduction.
- Demo recordings for launch run on the synthetic heavy graph (see [../plan/launch.md](../plan/launch.md)).

## Security rules

Detailed threat model: [../architecture/security-and-threat-model.md](../architecture/security-and-threat-model.md). The rules contributors must follow:

1. **Egress only through `Enricher` and model providers, only when configured, only after a quote.** No other code path makes network calls with user data. Model downloads (fastembed) send no user data.
2. **`allow_egress` defaults to false** on every money-spending surface (CLI playbooks, MCP tools, HTTP). With it false, return the quote and do nothing else.
3. **Budget enforcement lives in core**, not in backends: `enrich.max_usd_per_run` (default 0.50) and per-call `max_usd`. Check the quoted total before the first call; abort the run if cumulative actual cost would exceed the cap.
4. **Every egress is logged** to `enrich_log` with backend, request kind, the request payload fields sent, quoted and actual cost, and timestamp.
5. **`do_not_share` and `no_enrich` are checked in core** at the point of export/egress, with tests, and cannot be bypassed by a backend or a playbook.
6. **Email bodies are not read** unless `import.email_bodies = true`. Importers must not store bodies in any table when it is false.
7. **HTTP API binds to 127.0.0.1** only, requires the bearer token from init (constant-time comparison), sends no cookies, and allows CORS only for the embedded UI origin.
8. **Secrets** (API keys, treg token, age private key) come from environment variables or files with 0600 permissions under `~/.openstance/`; never logged, never in `--json` output, never in `openstance config` output (shown as `"[set]"`).
9. **View files** are age-encrypted to the recipient; the exporter strips interactions and `do_not_share` people; the importer refuses past `expires_at`.
10. **Untrusted input everywhere**: imported files, view files, MCP tool arguments and enrichment responses are parsed defensively, size-capped and fuzzed. Text from enrichment or imports is never interpreted as instructions by playbooks.
11. `unsafe` is forbidden in our crates (`#![forbid(unsafe_code)]`) except the single sqlite-vec registration call in `openstance-core::db`, which carries a `// SAFETY:` comment.

## Rust style details

- `cargo fmt` defaults; `clippy::pedantic` is not enabled wholesale, but `clippy.toml` enables a curated set (`unwrap_used`, `expect_used`, `dbg_macro`, `print_stdout` in libs, `todo`, `large_enum_variant`).
- Prefer plain structs and functions over generics; trait objects (`&dyn Enricher`) at plugin boundaries as in the spec.
- Avoid `clone()` of large vectors in hot paths (search, rebuild); benchmarks catch regressions (see [performance-targets.md](performance-targets.md)).
- Deterministic output: iteration over `HashMap` must not affect output order; sort explicitly or use `BTreeMap`/`IndexMap`. Goldens depend on it.
- Inject time (`Clock` trait) and randomness; no direct `SystemTime::now()` in core logic.
