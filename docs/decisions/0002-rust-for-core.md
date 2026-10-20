# ADR-0002: Rust for the core, CLI and servers

- **Status:** Accepted
- **Date:** 2026-10-20
- **Related:** [architecture/crates-and-dependencies.md](../architecture/crates-and-dependencies.md), [engineering/dev-setup.md](../engineering/dev-setup.md), [engineering/ci-and-release.md](../engineering/ci-and-release.md)

## Context

openstance promises "your network as a file, one binary, no daemon". The implementation language decides three things that matter to that promise:

1. **Distribution:** a single static binary installed with one command (`cargo binstall`, Homebrew, a curl script), no runtime to install.
2. **Parsing speed:** importers stream multi-GB inputs (a 5 GB Takeout mbox must import in under 2 minutes and under 500 MB RAM; see [engineering/performance-targets.md](../engineering/performance-targets.md)).
3. **Local inference:** embeddings (and optionally small rerankers) must run on CPU inside the same process with no extra install.

The v0.1 prototype is browser JavaScript over `localStorage` (see [prototype-v0.1.md](../prototype-v0.1.md)). It proves the UX but cannot meet any of the three requirements.

## Decision

Write the core, importers, enrichers, model adapters, CLI, MCP server and HTTP API in **Rust**, as one Cargo workspace producing one binary (`openstance`, with alias `ost`). TypeScript is used **only** for the optional web UI (Vite build, embedded as static files in the binary).

Supporting choices:

| Concern | Choice |
|---|---|
| SQLite binding | `rusqlite` with the `bundled` feature (no system SQLite dependency) |
| Embeddings | `fastembed` (ONNX via `ort`), model `bge-small-en-v1.5`, 384 dims, quantized |
| Async | `tokio`, **only** in `openstance-mcp` and `openstance-api`; `openstance-core` stays synchronous |
| CLI | `clap` (derive) |
| HTTP | `axum` |
| MCP | `rmcp` |
| View encryption | `age` (X25519) |
| Config | `toml` + `serde` |
| Logging | `tracing` + `tracing-subscriber` |
| Release | `cargo-dist` (cross-compiled binaries, installers, Homebrew tap) |

## Consequences

**Positive**

- Single binary, cross-compiled for macOS (arm64, x86_64), Linux (x86_64, arm64, musl) and Windows; installs without a runtime.
- Native ecosystem for local ML: `fastembed`, `ort`, `candle`, and `sqlite-vec` as a loadable or statically linked extension.
- Streaming parsers with predictable memory.
- Keeping the core sync makes it simple to test and lets the CLI avoid a runtime entirely for most commands.

**Negative**

- Slower to write than Go or TypeScript; compile times are noticeable.
- Smaller contributor pool. Mitigation: importer and enricher traits are small and well documented, with fixtures and a template crate; see [engineering/conventions.md](../engineering/conventions.md).
- ONNX runtime adds binary size and platform quirks. Mitigation: models download on first use (about 33 MB) and are cached; binary target is under 40 MB without models.

## Alternatives considered

| Option | Ship speed | Runtime speed | Local ML | Single binary | Verdict |
|---|---|---|---|---|---|
| **Rust** | slower to write | fastest | fastembed, ort, candle, sqlite-vec native | yes | **chosen** for core, CLI, MCP, API |
| Go | fastest to write | fast | cgo for ONNX; immature | yes | Fallback if Rust velocity becomes the bottleneck |
| TypeScript (Bun) | fast | fine | transformers.js; slow on CPU | mostly | Web UI only |
| Python | fast | slow parsing | best ecosystem | no | Rejected: users would need an environment |
| Keep the v0.1 browser app | already exists | fine for 10k rows | none | n/a | Rejected: localStorage limits, no mbox parsing, no MCP/CLI |

**Revisit if:** Phase 1 slips by more than a week because of Rust-specific friction, at which point Go for the CLI with a separate embedding sidecar is the documented fallback.
