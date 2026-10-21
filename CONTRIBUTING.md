# Contributing to openstance

Thanks for helping. This file covers the day-to-day rules and, in detail, how to add an importer. The design docs under [`docs/`](docs/README.md) are the source of truth; when this file and a tech spec disagree, the spec wins and this file should be fixed.

## Ground rules

- **No real contact data, anywhere.** Not in fixtures, tests, snapshots, issues or screenshots. Use fictional people on reserved domains (`*.example`, `*.test`, `*.invalid`, `example.com`) and the `555-01xx` phone range. See [conventions](docs/engineering/conventions.md#no-real-contact-data).
- **No network in core or importers.** `openstance-core` and `openstance-import` must not depend on an HTTP client or async runtime; `scripts/check-deps.sh` and `cargo deny` enforce this.
- **Tests first.** Write the failing test, watch it fail, then implement. Prefer fixture-backed and golden tests over mocks. See [testing strategy](docs/engineering/testing-strategy.md).
- **Before you push:**

  ```sh
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ./scripts/check-deps.sh
  cargo run -q -p openstance-fixtures --bin gen-fixtures && git diff --exit-code fixtures/
  ```

- **Commits** use conventional-commit style with the plan task id when there is one, e.g. `feat(import): P1-IMP-04 vcf importer`.
- **Frozen interfaces.** The `Importer` and `Sink` traits, the `Query` type and the result JSON shapes are frozen at the end of Phase 1. Changing them needs an ADR in [`docs/decisions/`](docs/decisions/).

## Adding an importer

The full contract is [ingestion.md](docs/tech-spec/ingestion.md) (section 3 for the traits, section 10 for the checklist). This is the practical version for the current code layout.

### What an importer is

An importer turns **one file** into a stream of raw records pushed into a `Sink`:

- `RawIdentity`: a handle seen outside any event (a vCard, a CSV row), with hints such as name, company and title. Identities that share a `record` key in one import are asserted to be the same human.
- `RawInteraction`: an event (email, DM, connection, meeting) with its time, direction hint, subject, `external_id`, `thread_key` and participants (each with handles, a role and hints).

An importer **never** touches the database, decides who is who, reads config beyond `ImportOptions`, or uses the network. Resolution, dedupe, normalisation (E.164 phones, LinkedIn slugs, email folding) and storage all happen in core's `Sink`. Pass handles through raw.

### Privacy rules (review checklist)

- Headers and metadata only. Never put a body, DM text, invitation message or note into any field, `subject` included.
- Every dropped unit is reported with `sink.skip(reason, locator)` and counted in `ImportStats`; the locator names the unit (line, row, message number), never its contents.
- Large inputs stream: read incrementally with bounded buffers and never hold the whole file in memory.
- The fixture contains no real data.

### Steps

1. **Pick a kind** in `snake_case` (e.g. `telegram_json`). It is stored in `source.kind` and used by `--as <kind>`.

2. **Add a cargo feature** in `crates/openstance-import/Cargo.toml` that gates the module and its parser dependencies (see `mbox`, `linkedin`, `vcf`). Parser crates are `optional = true` and enabled only by your feature. Add the feature to `importers-default` in `crates/openstance/Cargo.toml` if it should ship by default, and to the `openstance-import` dev-dependency features in `crates/openstance-core/Cargo.toml` if core's integration tests import its fixture.

3. **Create the module** `crates/openstance-import/src/<kind>.rs` and declare it in `src/lib.rs` behind `#[cfg(feature = "<feature>")]`. Implement `Importer`:

   ```rust
   impl Importer for TelegramJson {
       fn kind(&self) -> &'static str { "telegram_json" }
       fn detect(&self, path: &Path) -> Detection { /* read at most DETECT_SNIFF_BYTES */ }
       fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats, ImportError> { /* stream */ }
       fn version(&self) -> u32 { 1 }            // bump when output changes for the same input
       fn default_label(&self, path: &Path) -> Option<String> { None }
   }
   ```

   The crate-level rustdoc in `src/lib.rs` has a complete, compiling toy importer you can copy.

4. **Detect conservatively.** Return `Strong` only for content you verified in the first 64 KiB (`DETECT_SNIFF_BYTES`); an extension or file-name match alone is `Weak`. Never fail: I/O errors return `No`. Two importers matching at the same level is a user-visible error, so be specific.

5. **Map to records.** One `RawInteraction` per real event. Give it a stable `external_id` (Message-ID, row id, event UID) so re-imports dedupe; when you have none, leave it `None` and the Sink synthesizes one. Set `thread_key` when the format has threads. Only put hints the file actually states.

6. **Register** it in `registry()` in `src/lib.rs`, behind the same `#[cfg]`. The order there is the detection tie-break order.

7. **Fixtures.** Extend the generator in `fixtures/gen` (`cargo run -p openstance-fixtures --bin gen-fixtures`) so it writes your input under `fixtures/<kind>/`. Include at least one overlap with an existing source (same email or phone) and at least one malformed record. If resolution counts change, update `fixtures/EXPECTED.md` deliberately and explain why in the commit. CI regenerates fixtures and fails on any diff.

8. **Tests** in `crates/openstance-import/tests/<kind>.rs` (shared helpers are in `tests/common/mod.rs`):
   - add your fixture to the detection matrix in `tests/detect.rs`: `Strong` for your importer and `No` for every other one, and theirs `No` for yours;
   - a golden `ImportStats` snapshot (`insta`, under `tests/snapshots/`) and assertions on the emitted records via `MemorySink`;
   - every skip reason you emit, each with its own case;
   - a sentinel string placed in a body or message field that must never appear in any emitted record.

   In core, re-importing the same file must be a no-op (`crates/openstance-core/tests/ingest_sink.rs` shows the pattern).

9. **Docs.** Add a row to the importer table in section 6 of [ingestion.md](docs/tech-spec/ingestion.md) and a subsection listing exactly which files and fields are read and what is ignored. Update the importer table in `README.md`.

### Feature-flag check

An importer must also build cleanly on its own and with every importer off:

```sh
cargo check -p openstance-import --no-default-features
cargo check -p openstance-import --no-default-features --features <feature>
cargo clippy --workspace --all-targets --no-default-features -- -D warnings
```
