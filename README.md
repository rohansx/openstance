<!--
Launch README draft (P2-DOC-03), written to docs/plan/launch.md "README requirements".
Before the Show HN post goes live:
  - [ ] Record the demo (P2-DOC-02: scripts/demo.sh + asciinema) and replace the placeholder GIF and cast link.
  - [ ] Re-run the Quickstart against the release build and paste the real output (the blocks below are
        trimmed debug-build output on the demo dataset with `--now 2026-10-15`).
  - [ ] Licence: launch.md says MIT, but Cargo.toml says Apache-2.0 and there is no LICENSE file yet. Decide, add
        LICENSE, and make the badge, crate metadata and the section at the bottom agree.
  - [ ] Point the CI badge at the real repository once it is public.
  - [ ] ADR-0001 (name): decide or deliberately defer, and update the naming note.
-->

# openstance

**Your network as a file. Free local search, works in Claude Code.**

[![CI](https://github.com/openstance/openstance/actions/workflows/ci.yml/badge.svg)](https://github.com/openstance/openstance/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](#license-and-name)

openstance reads the exports you can already download (a Google Takeout mbox, LinkedIn's `Connections.csv`, a `.vcf` address book), works out who is who, scores how strong each relationship is, and lets you search the result in plain English, from the terminal or from an agent. Everything lives in one SQLite file on your machine. There is no account, no API key and no per-search cost.

## The 60-second demo

![openstance demo: import three exports, search, explain a score, find a warm path](docs/assets/demo.gif)

[Watch it as an asciinema cast](https://asciinema.org/a/TODO) · reproduce it yourself with [`scripts/demo.sh`](scripts/demo.sh) · the people and companies in it are fictional ([`fixtures/demo/`](fixtures/demo/)).

## Install

```sh
cargo install --git https://github.com/openstance/openstance openstance   # installs `openstance` and the short alias `ost`
```

You need Rust stable (MSRV 1.85) and a C compiler, because SQLite is compiled in (Xcode Command Line Tools on macOS, `build-essential` on Linux). Prebuilt binaries, an install script and Homebrew arrive with v1.0.

## Quickstart

Five commands. The output below comes from the fictional demo dataset, so you can reproduce it exactly:

```sh
git clone https://github.com/openstance/openstance && cd openstance
export OPENSTANCE_HOME=$(mktemp -d)/.openstance     # leave your real ~/.openstance alone
```

**1. Create your graph.** `--me` records your own addresses so sent and received mail is told apart.

```console
$ ost init --me you@example.com
created ~/.openstance/graph.db (schema 1)
created ~/.openstance/config.toml
next: openstance import ~/Downloads/takeout/*.mbox ~/Downloads/Connections.csv
```

**2. Import your exports.** The file type is detected from the content. Re-importing a file is a no-op.

```console
$ ost import fixtures/demo/takeout.mbox fixtures/demo/Connections.csv fixtures/demo/contacts.vcf
mbox          takeout.mbox  6505 records  14420 identities  6505 new interactions
linkedin_csv  Connections.csv  304 records  477 identities  304 new interactions
vcf           contacts.vcf  213 records  363 identities  213 new interactions
graph         612 people, 939 identities, 6809 interactions
resolution    323 auto-merged, 3 flagged -> openstance review
scoring       preset default, 7 bulk senders excluded
```

**3. Review the uncertain merges.** Merges made on less than an exact match (same name and company, same name on one thread) are applied but flagged for you to confirm or undo.

```console
$ ost review --list
flagged merges (3):
 conf  rule          person
 0.75  name_thread   Marco Bianchi (#416)  email:marco@kitebox.example  +  email:marco@bianchi-home.example
       why: same name "marco bianchi"; both on one email thread or meeting
 0.85  name_company  Elif Kaya (#391)  email:elif@stackforge.example  +  linkedin_url:elif-kaya-393
       why: same name "elif kaya"; company "stackforge"
 ...
confirm: openstance review --confirm <id>   split: openstance review --split <id>
```

**4. Search.** Plain English is parsed into filters you can see and rerun, and every result carries the evidence it was found on.

```console
$ ost search "top 5 fintech founders who replied"
  #  person         title / company               score  last contact
  1  Maya Chen      Co-founder & CEO, Ledgerline   0.81  2026-10-09
       2026-10-09  email out  "Re: Fintech founders breakfast, November"  (mbox 2026-10-15)
  2  Arjun Mehta    Founder & CEO, Tallyfin        0.56  2026-04-15
       2026-04-15  email in  "Re: Fintech founders breakfast"  (mbox 2026-10-15)
  ...
11 results (5 shown, by score)
filters: title=[founder] industry=[fintech] replied=true limit=5  (parser: rules)
rerun:   openstance search --title founder --industry fintech --replied --limit 5
```

**5. Ask why.** `who` shows one person: each score component with its weight, identities, recent interactions and mutual contacts.

```console
$ ost who "Maya Chen"
Maya Chen - Co-founder & CEO, Ledgerline - New York, NY    score 0.81 (preset default, #69)
  recency      0.980 x 0.35 = 0.343   5 days since last contact (half-life 180)
  frequency    0.961 x 0.20 = 0.192   65 interactions
  reciprocity  1.000 x 0.25 = 0.250   22 replies + 7 threads started (x2) / 34 sent
  meetings     0.000 x 0.15 = 0.000   0 meetings
  breadth      0.500 x 0.05 = 0.025   email, linkedin
  sum                       = 0.810
identities  email:maya@ledgerline.example (mbox, 1.00 anchor)  linkedin_url:maya-chen-79e (linkedin_csv, 1.00 exact)  ...
recent      2026-10-09  email out  "Re: Fintech founders breakfast, November"  (mbox 2026-10-15)
            ...
mutuals     Dana Whitfield (email, 10 shared, tie 0.83)  Theo Marsh (email, 6 shared, tie 0.58)
```

And to find the warmest way in to someone you barely know (`ost how` for short):

```console
$ ost how-connected "Dana Whitfield"
direct: score 0.50, last contact 2026-09-19
 1  you -> Theo Marsh (score 0.77) -> Dana Whitfield (tie 0.90)   strength 0.691 = 0.77 x 0.90
      18 shared email (tie 0.90), last 2026-09-19
      2026-09-19  email out  "Re: Lumen Pay API changes for Q4"  (mbox 2026-10-15)
      ...
 2  you -> Maya Chen (score 0.81) -> Dana Whitfield (tie 0.83)   strength 0.672 = 0.81 x 0.83
      10 shared email (tie 0.83), last 2026-07-06
      2026-07-06  email out  "Re: Lumen Pay sandbox access for Ledgerline"  (mbox 2026-10-15)
      ...
 3  you -> Rafael Ortiz (score 0.63) -> Dana Whitfield (tie 0.67)   strength 0.419 = 0.63 x 0.67
      7 shared email (tie 0.67), last 2026-06-14
      2026-06-14  email in  "Re: Lumen Pay board prep"  (mbox 2026-10-15)
      ...
```

Search uses SQLite FTS5 plus local embeddings (bge-small-en-v1.5 on your CPU), so "people who build developer tools" also finds titles that say "SDK" or "API platform". With `model.embedder = "none"` it is keyword search only and needs no model at all. Other useful commands: `edit` (fix a title, add a tag or a note; edits survive re-import), `merge`, `split`, `rebuild`, `export --format csv|json`, `config`. `openstance --help` lists them all.

## Where to get your exports

| Source | How | Typical wait |
|---|---|---|
| **Gmail** (Google Takeout) | [takeout.google.com](https://takeout.google.com): "Deselect all", tick **Mail**, export once as `.zip`. Unzip it and import `Takeout/Mail/All mail Including Spam and Trash.mbox`. Only headers are read | minutes to a day or two, depending on mailbox size |
| **LinkedIn** | Settings, Data privacy, **Get a copy of your data**. Choose "Connections" (and "Messages" if you like) or the full archive. Unzip and import `Connections.csv` (and `messages.csv`, `Invitations.csv`) | about 10 minutes for connections only; up to 24 hours for the full archive |
| **Apple Contacts** | Contacts app: select all (Cmd-A), then File, Export, **Export vCard** | immediate |
| **Google Contacts** | [contacts.google.com](https://contacts.google.com): select all, Export, **vCard (for iOS Contacts)** | immediate |

Zipped exports are not read directly yet: unzip first. Calendar (`.ics`) and X archives arrive in the next release.

## Use from an agent

The same binary is an MCP server. It is local, read-only and free: the tools read your graph file and make no network calls.

**Claude Code**

```sh
claude mcp add openstance -- openstance mcp
```

**Cursor** (`~/.cursor/mcp.json`) and **Claude Desktop** (`claude_desktop_config.json`; use the absolute path from `command -v openstance`):

```json
{
  "mcpServers": {
    "openstance": { "command": "openstance", "args": ["mcp"] }
  }
}
```

| Tool | Answers |
|---|---|
| `search_people` | "who do I know at Lumen Pay", "fintech founders I've emailed since 2025", "who have I lost touch with". Returns the parsed filters and the same JSON as `openstance search --json` |
| `get_person` | everything about one person (the `who` view): fields and where they came from, score components, identities, recent interactions, mutuals |
| `how_connected` | the top three warm paths to someone already in your graph, each with the threads behind it |

Then ask: *"Who do I know at Lumen Pay, and who's my warmest way in to their CTO?"* The agent skill [`playbooks/openstance-search.md`](playbooks/openstance-search.md) tells an agent which tool to use when and to quote evidence lines rather than paraphrase them. Per-client details and troubleshooting: [docs/guides/mcp-clients.md](docs/guides/mcp-clients.md).

## Privacy and what costs money

openstance has no telemetry and never phones home. Nothing leaves your machine unless you turn on an enricher or a hosted model, and then every call is quoted first and logged.

| Path out of your machine | Default | What is sent | When it costs money | How to audit |
|---|---|---|---|---|
| Embedding model download | once, on first semantic search or import | nothing about you: a one-time download of the bge-small-en-v1.5 model (about 33 MB) from Hugging Face into `~/.openstance/models/` | never | set `model.embedder = "none"` to skip it entirely |
| Enrichment (people outside your graph) | **off**, arrives in the next release | the name and company of the person you asked about, to the enricher you configure | per call, priced before it runs | `openstance enrich log` (with the enrichment release) |
| Hosted models (query parsing, reranking) | **off** | your query text, to the provider you configure | per call, at that provider's price | the same log |

What is read from your exports: email **headers only** (From, To, Cc, Date, Subject, Message-ID, threading and list headers; bodies are never read or stored), LinkedIn names, profile URLs, companies, positions and dates (message text is never read), and vCard names, emails, phones, organisation, title and locality. Searching, scoring, `who`, `how-connected` and the MCP server are free and offline.

## How the score works

Each person gets a relationship score in [0, 1] from five components you can inspect with `who`:

```
S = w_r*R + w_f*F + w_c*C + w_m*M + w_b*B
```

| | Component | Formula |
|---|---|---|
| R | recency | `exp(-ln 2 * days_since_last / half_life_days)` (0.5 at the half-life, 180 days by default) |
| F | frequency | `1 - exp(-n_interactions / 20)` |
| C | reciprocity | `min(1, (replies + 2 * threads they started) / max(1, messages you sent))`, 0 if you never wrote |
| M | meetings | `1 - exp(-n_meetings / 3)` |
| B | breadth | channels seen (email, calendar, LinkedIn, ...) / 4 |

Presets set the weights: `default` (0.35, 0.20, 0.25, 0.15, 0.05), `founder`, `recruiter`, `seller` and `community`. Or set your own in `~/.openstance/config.toml`:

```toml
[score]
preset = "founder"            # default | founder | recruiter | seller | community
half_life_days = 180
# or explicit weights instead of a preset:
# weights = { recency = 0.30, frequency = 0.15, reciprocity = 0.30, meetings = 0.20, breadth = 0.05 }
```

Newsletters and no-reply senders are detected and scored 0 (`edit <p> --not-bulk` if one is a real person). Full details: [scoring](docs/tech-spec/scoring.md).

## Status and limits

**Works today (v0.3.0):** importers for Takeout mbox, LinkedIn CSV and vCard; conservative entity resolution with a review queue; explained scores; keyword plus semantic search with evidence lines; `who`; `how-connected` for people already in your graph; the MCP server with three tools; edits, merges and splits that survive re-import and rebuild; CSV and JSON export.

**Next (Phase 3):** a path to someone *outside* your graph through optional, per-call enrichment (quoted first, logged); the intro brief; encrypted shareable views of a slice of your graph; calendar (`.ics`) and X archive importers.

**Then (Phase 4, v1.0):** installers and Homebrew, a local web UI, bring-your-own local or hosted models for parsing and reranking.

**Known limits:**

- Merges are deliberately conservative: some duplicates stay separate until you merge them, and uncertain merges wait in `review`.
- Email bodies are never read, so search knows subjects and people, not what was said.
- No live sync: re-import a fresh export to update.
- Until the calendar importer lands, `M` (meetings) is 0 for everyone and "people I've met" finds no one.
- Zipped exports must be unzipped first.

## The file

Your graph is one SQLite file, `~/.openstance/graph.db`, with a documented schema. Open it with any SQLite tool:

```console
$ sqlite3 ~/.openstance/graph.db '.tables'
enrich_log          meta                person_fts          source
identity            override            person_vec          tie
interaction         person              person_embed        view
interaction_person  ...
```

`person` holds one row per human (fields, score and components), `identity` every handle (email, LinkedIn URL, phone) with how it was matched, `interaction` and `interaction_person` the headers-only record of who was on what, `tie` the derived links between your contacts, `override` your edits, and `person_fts` / `person_vec` the search indexes. `openstance export --format csv|json` gives you the same data without SQL. Schema reference: [data model](docs/tech-spec/data-model.md).

## Contributing

Bug reports from real exports are the most useful thing you can send, but please never paste real contacts into an issue: [CONTRIBUTING.md](CONTRIBUTING.md) shows how to reproduce a problem with synthetic data and how to report a bad merge with the names redacted. New importers are the easiest way in; CONTRIBUTING.md walks through adding one, and issues labelled [`good first issue`](https://github.com/openstance/openstance/labels/good%20first%20issue) are a good start. The design lives in [`docs/`](docs/README.md) (plan, tech spec, architecture, decisions).

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
./scripts/check-deps.sh          # crate dependency direction
```

## License and name

Licensed under Apache-2.0 (see `Cargo.toml`).

"openstance" is a working name. The naming decision is still open in [ADR-0001](docs/decisions/0001-product-name.md), and the binary, crate and directory names may change before v1.0. All people, companies, domains and phone numbers in this repository's fixtures and demo are fictional.
