# Developer setup

How to go from a clean machine to a built `openstance` binary running against the synthetic fixtures, plus the notes you need for SQLite extensions, local models and the optional web UI.

Related: [conventions.md](conventions.md), [testing-strategy.md](testing-strategy.md), [ci-and-release.md](ci-and-release.md), [../architecture/crates-and-dependencies.md](../architecture/crates-and-dependencies.md), [../tech-spec/configuration.md](../tech-spec/configuration.md).

## Prerequisites

| Tool | Version | Why |
|---|---|---|
| Rust via `rustup` | stable (see `rust-toolchain.toml`) | everything |
| MSRV | **1.85** (edition 2024) | declared as `rust-version` in the workspace `Cargo.toml`; CI checks it |
| C compiler | Xcode CLT (macOS), `build-essential` (Linux), MSVC Build Tools (Windows) | bundled SQLite and sqlite-vec are compiled from C via `cc` |
| `just` | >= 1.25 | task runner (`cargo install just` or `brew install just`) |
| `cargo-nextest` | latest | faster test runner |
| `cargo-insta` | latest | snapshot review |
| Node.js | 22 LTS | only for `ui/` and the v0.1 prototype |
| `pnpm` | 9+ | only for `ui/` |
| Optional: `cargo-llvm-cov`, `cargo-fuzz` (+ nightly), `cargo-deny`, `cargo-dist`, `hyperfine`, Ollama | | coverage, fuzzing, license audit, release dry-runs, CLI timing, model-tier evals |

`rust-toolchain.toml` pins the channel and components so everyone (and CI) gets the same `rustfmt`/`clippy`:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy", "llvm-tools-preview"]
```

The MSRV is a floor, not the dev toolchain: day-to-day you build on latest stable; CI runs one job on `1.85` with `cargo check --workspace --all-features`. Raising the MSRV is a minor-version change and goes in the changelog.

One-shot install of the helper tools:

```sh
rustup show                      # installs the pinned toolchain
cargo install --locked just cargo-nextest cargo-insta cargo-llvm-cov cargo-deny
```

## Clone, build, run

```sh
git clone https://github.com/<org>/openstance.git
cd openstance
just build                       # cargo build --workspace
just test                        # unit + integration + golden + playbook tests
cargo run -p openstance -- --help
```

Never point a development build at your real `~/.openstance`. Every dev command below sets `OPENSTANCE_HOME` to a throwaway directory. The binary resolves its data dir as: `--home` flag, then `OPENSTANCE_HOME`, then `~/.openstance/`.

### Running against fixtures

```sh
export OPENSTANCE_HOME="$PWD/target/dev-home"
cargo run -p openstance -- init
cargo run -p openstance -- import fixtures/mbox/work.mbox fixtures/linkedin/Connections.csv fixtures/vcf/contacts.vcf
cargo run -p openstance -- review                     # flagged merges from the fixture overlaps
cargo run -p openstance -- search "founders in Bengaluru" --met --json | jq '.results[0]'
cargo run -p openstance -- who maya@lumenlabs.example
```

`just demo` does all of the above into `target/dev-home` (and wipes it first). `just demo-heavy` generates the synthetic 20k-person graph (see [performance-targets.md](performance-targets.md#synthetic-data)) and imports it, which takes a few minutes the first time and is cached under `target/synth/`.

The `ost` alias: release installers place an `ost` link next to `openstance`. In dev, use `cargo run -p openstance --` or `alias ost="cargo run -q -p openstance --"`.

## Workspace layout (short)

```
crates/openstance-core     schema, migrations, resolution, scoring, search, playbooks (sync)
crates/openstance-import   Importer trait + mbox, linkedin_csv, vcf, ics, x_archive, csv (feature-gated)
crates/openstance-enrich   Enricher trait + Null, treg, ownkeys; MockEnricher under `test-util`
crates/openstance-model    Embedder / QueryParser / Reranker traits + fastembed, ollama, openai, anthropic
crates/openstance-mcp      rmcp server (tokio)
crates/openstance-api      axum HTTP API on 127.0.0.1:7411 (tokio), embeds ui/dist
crates/openstance          clap CLI, wires everything
ui/                        optional TypeScript + Vite UI
playbooks/                 markdown skill files
fixtures/                  synthetic data only
xtask/                     cargo xtask helpers
```

Full dependency rules are in [../architecture/crates-and-dependencies.md](../architecture/crates-and-dependencies.md).

## SQLite, FTS5 and sqlite-vec

- `rusqlite` is built with the `bundled` feature, so no system SQLite is used or needed. FTS5 is enabled in the bundled build (`bundled` + `SQLITE_ENABLE_FTS5` via rusqlite's feature set). Do not add a dependency that pulls `rusqlite` without `bundled`; `cargo deny` and a CI check enforce one SQLite in the tree.
- `sqlite-vec` is statically linked via the `sqlite-vec` crate and registered at connection open with `sqlite3_auto_extension`. There is no `.dylib`/`.so` to ship or load at runtime; `load_extension` stays disabled.
- Inspecting a dev database: the stock `sqlite3` CLI can read everything except `person_vec`. Use `cargo run -p openstance -- debug sql "<query>"`, which opens the database with the extension registered, or `sqlite3 target/dev-home/graph.db` for non-vector tables.
- WAL mode is on; you will see `graph.db-wal` and `graph.db-shm` next to the file while the binary runs. Do not copy only `graph.db` while a process is writing.
- Optional SQLCipher build: `cargo build -p openstance --features sqlcipher` swaps in `bundled-sqlcipher-vendored-openssl`. It is slower to compile and not part of the default CI matrix (one Linux job builds it nightly).

## Embedding models (fastembed)

- Default embedder: `fastembed:bge-small-en-v1.5`, quantized, 384 dims, CPU via ONNX Runtime. It is downloaded on first use (~33 MB) and cached.
- Model cache directory: `$OPENSTANCE_HOME/models/` (so `~/.openstance/models/` for users). Override with `OPENSTANCE_MODEL_DIR` or `model.cache_dir` (CI uses this to share a cache across jobs). The binary never downloads a model unless an embedding-dependent command runs and the configured embedder needs it; it prints the size and destination before the download.
- ONNX Runtime: the `fastembed` crate's default `ort` setup downloads a prebuilt ONNX Runtime at build time. For offline or reproducible builds, set `ORT_LIB_LOCATION` to a local ORT build, or build with `--no-default-features --features embed-none`, which drops fastembed and uses the Null embedder (search falls back to SQL + FTS5).
- Tests never download models unless `OPENSTANCE_TEST_MODELS=1`. Default test runs use the deterministic hash embedder from `openstance-model`'s `test-util` feature.
- Changing the embedder (name or dims) triggers a re-index on next run; in dev, `cargo run -p openstance -- rebuild --reindex` forces it.

## Optional local LLMs

For the model-tier evals and for trying the Ollama parser or reranker:

```sh
ollama pull qwen2.5:7b
cat >> "$OPENSTANCE_HOME/config.toml" <<'EOF'
[model]
parser = "ollama:qwen2.5:7b"
EOF
OPENSTANCE_EVAL_OLLAMA=http://127.0.0.1:11434 cargo xtask eval --tier ollama
```

API-backed models (Anthropic, OpenAI) read keys from environment variables named in config (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`). Never put keys in `config.toml` that lives inside the repo or in fixtures.

## Web UI (optional)

The UI is a client of the HTTP API only.

```sh
# terminal 1: API with a fixed dev token
export OPENSTANCE_HOME="$PWD/target/dev-home"
cargo run -p openstance -- serve --port 7411 --dev-token devtoken
# terminal 2: Vite dev server proxying /api to 127.0.0.1:7411
cd ui
pnpm install
VITE_OPENSTANCE_TOKEN=devtoken pnpm dev        # http://127.0.0.1:5173
```

`--dev-token` is only accepted by debug builds. In `serve --dev`, CORS additionally allows `http://127.0.0.1:5173`; release builds allow only the embedded UI origin.

Production embedding: `just ui-build` runs `pnpm build` and writes `ui/dist/`, which `openstance-api` embeds with `include_dir!`/`rust-embed` when the `ui` feature is on (default in release builds). If `ui/dist` is missing, the build uses a one-page placeholder so Rust contributors never need Node.

## The v0.1 prototype

The browser prototype is kept as reference and a fixture source, not as the product (see [../prototype-v0.1.md](../prototype-v0.1.md)). To run it from its directory (location given in that doc): `npm start` and open `http://127.0.0.1:4317`. `npm test` runs its 12 Node tests. It is not built or tested by the Rust CI except a weekly job that keeps its tests green.

## Task runner

A `justfile` at the repo root is the documented entry point; it shells out to `cargo` and `cargo xtask` (an `xtask/` binary crate for anything longer than a one-liner, so Windows contributors are not dependent on shell scripts).

```just
default: check test

build:
    cargo build --workspace

check:
    cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings

fix:
    cargo fmt --all && cargo clippy --workspace --all-targets --all-features --fix --allow-dirty

test:
    cargo nextest run --workspace --all-features

test-models:
    OPENSTANCE_TEST_MODELS=1 cargo nextest run --workspace --all-features --run-ignored all

eval:
    cargo xtask eval --tier rules

eval-models:
    cargo xtask eval --tier fastembed

props-long:
    PROPTEST_CASES=10000 cargo test --release -p openstance-core --test rebuild_props

fuzz target secs="60":
    cargo +nightly fuzz run {{target}} --fuzz-dir crates/openstance-import/fuzz -- -max_total_time={{secs}}

bench:
    cargo xtask bench

cov:
    cargo xtask coverage --check

fixtures:
    cargo xtask gen-fixtures && git diff --exit-code fixtures/

deny:
    cargo deny check

demo:
    cargo xtask demo --home target/dev-home

demo-heavy:
    cargo xtask demo --home target/dev-home-heavy --synthetic heavy

ui-build:
    cd ui && pnpm install --frozen-lockfile && pnpm build

release-dry:
    cargo dist plan && cargo dist build --artifacts=local

```

`cargo xtask` subcommands: `gen-fixtures`, `gen-graph`, `eval`, `bench`, `coverage`, `demo`, `fuzz-smoke`, `scan-fixtures` (the no-real-contact-data check), `schema-dump` (writes `docs`-ready SQL for the current head schema).

## Useful environment variables

| Variable | Effect |
|---|---|
| `OPENSTANCE_HOME` | data dir (default `~/.openstance`) |
| `OPENSTANCE_MODEL_DIR` | model cache (default `$OPENSTANCE_HOME/models`) |
| `OPENSTANCE_LOG` | `tracing` filter, e.g. `openstance_core=debug,info` |
| `OPENSTANCE_LOG_FORMAT` | `pretty` (default on TTY) or `json` |
| `OPENSTANCE_TEST_MODELS` | `1` enables tests that download/run fastembed |
| `OPENSTANCE_EVAL_OLLAMA` | Ollama base URL for the local eval tier |
| `OPENSTANCE_NOW` | debug builds only: fixed unix time for scoring, used by goldens and demos |
| `PROPTEST_CASES` | proptest case count |

## Editor setup

- rust-analyzer with `"rust-analyzer.cargo.features": "all"` so feature-gated importers are analyzed.
- Format on save with rustfmt; the repo `rustfmt.toml` sets `imports_granularity = "Crate"` and `group_imports = "StdExternalCrate"` (requires nightly rustfmt for those two options; stable ignores them with a warning, CI runs stable rustfmt and only enforces stable options).
- For `ui/`: the repo's ESLint + Prettier config; `pnpm lint`.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `no such module: fts5` | Something linked a system SQLite. Run `cargo tree -i libsqlite3-sys` and make sure only the bundled one remains. |
| `no such module: vec0` | The connection was opened without the core's `open_db()` helper, which registers sqlite-vec. Always open through `openstance_core::db::open`. |
| ONNX Runtime download fails at build | Offline or proxied network. Use `ORT_LIB_LOCATION` or build with `--features embed-none`. |
| Linker errors on Windows | Install "Desktop development with C++" in Visual Studio Build Tools; use the MSVC target, not GNU. |
| Tests touch real data | They should not: every test uses `TestGraph` temp dirs. If you see `~/.openstance` in a test, that is a bug; file it. |
