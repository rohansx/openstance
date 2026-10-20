# openstance

A local-first relationship graph for your own contacts. openstance reads your exports (a Google Takeout mbox, LinkedIn CSV exports, a `.vcf` address book), works out who is who, scores how strong each relationship is, and lets you search the graph with flags or plain text. Every result comes with its score breakdown and evidence lines.

Everything stays on your machine in one SQLite file (`~/.openstance/graph.db`). Phase 1 makes no network calls and uses no models.

> Status: **v0.2.0, Phase 1, build from source only.** The design lives in [`docs/`](docs/README.md); the Phase 1 plan is [`docs/plan/phase-1.md`](docs/plan/phase-1.md).

## Build from source

Requirements: Rust stable (the toolchain is pinned in `rust-toolchain.toml`, MSRV 1.85) and a C compiler, because SQLite is compiled in (Xcode Command Line Tools on macOS, `build-essential` on Linux).

```sh
git clone <this repo> openstance
cd openstance
cargo install --path crates/openstance     # installs `openstance` and its short alias `ost`
openstance --help
```

To run without installing, use `cargo run --release -p openstance -- <command>`.

## Quickstart

### 1. Create your graph

```sh
openstance init --me you@example.com
```

This creates `~/.openstance/` with `graph.db` and a commented `config.toml`. `--me` (repeatable) records your own addresses so that direction (sent or received) and the self person are correct. Running `init` again is a no-op that prints the existing paths.

To try openstance without touching `~/.openstance`, point it at a throwaway directory with `OPENSTANCE_HOME=/tmp/ost-demo`, or pass `--db` and `--config` on each command.

### 2. Import your exports

```sh
openstance import ~/Downloads/Takeout/Mail/All\ mail\ Including\ Spam\ and\ Trash.mbox \
                  ~/Downloads/LinkedInDataExport_10-15-2026/Connections.csv \
                  ~/Downloads/LinkedInDataExport_10-15-2026/messages.csv \
                  ~/Downloads/contacts.vcf
```

| Importer | Files | What is read |
|---|---|---|
| `mbox` | Google Takeout (or any) mbox | headers only: From, To, Cc, Date, Subject, Message-ID, threading and list headers. Bodies are never read or stored. |
| `linkedin_csv` | `Connections.csv`, `messages.csv`, `Invitations.csv` from an unzipped LinkedIn export | names, profile URLs, email when present, company and position, dates. Message text is never read. |
| `vcf` | vCard 2.1, 3.0 or 4.0 (Apple, Google) | names, emails, phones, organisation, title, locality |

The file type is detected from its content; `--as <kind>` forces one. Importing a file you already imported is a no-op. After the import, openstance resolves people, scores them and prints a summary:

```text
graph         30 people, 44 identities, 193 interactions
resolution    9 auto-merged, 4 flagged -> openstance review
scoring       preset default, 1 bulk senders excluded
```

### 3. Review uncertain merges

Some merges are made with less than full confidence (same name and company, or same name on one email thread). They are applied but flagged for you to check:

```sh
openstance review                 # interactive: [a]ccept [s]plit [k]skip [q]uit
openstance review --list          # just print flagged merges and suggestions
openstance review --confirm 4     # accept one
openstance review --split 23      # undo one
openstance review --accept-all-above 0.85
```

Suggestions (an identical rare name, a near-identical name at the same company) are never merged automatically; use `openstance merge <a> <b>` if they are the same person, or `openstance review --reject <a> <b>` to keep them apart. Every decision is stored as an override and survives `openstance rebuild`.

### 4. Search

```sh
openstance search --company Northwind --since 2025-01-01
openstance search "founders I've met since 2025" --explain
openstance search "devtools" --json | jq '.results[0].person | {display_name, title, score}'
openstance who priya@northwind.example
```

Free text is parsed by a rule-based parser into structured filters (company, title, location, dates, met, replied, lost touch, tags) plus leftover text for full-text search. `--explain` prints the parsed query and a `rerun:` line you can paste back as flags. Results are ranked by full-text relevance lifted by the relationship score, or by score alone when there is no text. Bulk senders (newsletters, no-reply addresses) are excluded unless you pass `--include-bulk`.

`who` shows one person: the five score components (recency, frequency, reciprocity, meetings, breadth) with their weights, identities, recent interactions, mutual contacts and where each field came from.

### 5. Edit, merge, split, rebuild, export

```sh
openstance edit "Priya Raman" --title CTO --tag investor-intro --note "met at the devtools dinner"
openstance merge <a> <b>
openstance split <person> --identity email:old@example.com
openstance rebuild                         # recompute everything; edits are kept
openstance export --format csv > people.csv
openstance export --format json > people.json
openstance config                          # print the effective config
```

A person can be named by id, email, LinkedIn slug, @handle or a unique name; an ambiguous name lists the candidates. Edits never change your source data: they are override rows applied on top of it.

### Configuration

`~/.openstance/config.toml` (or `--config <file>`, or `OPENSTANCE_CONFIG`) is optional: every key has a default, and unknown keys produce a warning. Useful keys in Phase 1: `[me] identities`, `[score] preset` (`default`, `founder`, `recruiter`, `seller`, `community`) or explicit `weights`, `[score] half_life_days`, `[import] timezone` and `default_region`. See [configuration](docs/tech-spec/configuration.md).

## Try it on the synthetic fixtures

The repository ships fictional data under `fixtures/` (all names, domains and phone numbers are made up). Their expected counts are in [`fixtures/EXPECTED.md`](fixtures/EXPECTED.md).

```sh
export OPENSTANCE_HOME=$(mktemp -d)/.openstance
export OPENSTANCE_CONFIG=$PWD/fixtures/config.toml   # fixture owner + test Gmail domains
openstance init
openstance import fixtures/mbox/small.mbox fixtures/linkedin/Connections.csv \
  fixtures/linkedin/messages.csv fixtures/linkedin/Invitations.csv fixtures/vcf/contacts.vcf
openstance review --list
openstance search --company Northwind
```

## What is not there yet

Phase 1 is a local CLI. Not included yet (see the [roadmap](docs/plan/roadmap.md)):

- Embeddings, semantic search, model-based query parsing and reranking (Phase 2).
- Calendar (`.ics`) and X archive importers, so `--met` has no meetings to find yet (Phase 3). Email bodies are never read; `--bodies` reports "not supported".
- Zipped exports: unzip a Takeout or LinkedIn export first and import the files inside.
- MCP server, HTTP API, shared views, enrichment (later phases).
- `config set/edit`, `import --dry-run`, `source`, `gc`, `doctor` and `backup` commands, and `import --replace`.
- Release binaries and installers. Build from source for now.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
./scripts/check-deps.sh          # crate dependency direction
```

See [CONTRIBUTING.md](CONTRIBUTING.md), [dev setup](docs/engineering/dev-setup.md), [conventions](docs/engineering/conventions.md) and [testing strategy](docs/engineering/testing-strategy.md). Never use real contact data in tests, fixtures or bug reports.

## License

Apache-2.0.
