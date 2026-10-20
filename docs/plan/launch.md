# Launch plan

The public launch is the **Show HN at the end of [Phase 2](phase-2.md)**. It is followed by smaller announcements for [Phase 3](phase-3.md) (path to a stranger, cents per call) and [Phase 4](phase-4.md) (v1.0, installers, local UI). This document covers positioning, channels, the launch checklist, README requirements, the metrics to watch, and the park criterion. The timeline is in the [roadmap](roadmap.md).

Related: [vision](../product/vision.md), [personas and use cases](../product/personas-and-use-cases.md), [competitive analysis](../product/competitive-analysis.md), [pricing and cloud](../product/pricing-and-cloud.md), [security and threat model](../architecture/security-and-threat-model.md).

---

## Positioning

**Lead line:** free search, your graph as a file, agent-native.

**One-liner:** Your network as a file. Search it with any model. Share slices, never the graph.

The three claims, with the proof each one needs to show on launch day:

| Claim | What it means | Proof in the launch materials |
|---|---|---|
| **Free search** | Unlimited local search. No account, no API key, no per-query cost. Money is spent only on optional, per-call egress, and the quote is shown first | The demo runs with Wi-Fi off. The README's "what costs money" section has one row (enrichment, off by default) |
| **Your graph as a file** | One SQLite file at `~/.openstance/graph.db` with a documented schema. Export to CSV and JSON whenever you like. Every field can be edited, and edits survive re-import | `sqlite3 ~/.openstance/graph.db '.tables'` in the README, plus a link to [data model](../tech-spec/data-model.md) |
| **Agent-native** | An MCP server in the same binary, free, works in Claude Code and Cursor | The last 15 seconds of the demo are Claude Code calling `search_people` and `how_connected` |

Supporting points, in this order: explained scores ("a number you can argue with"), evidence lines instead of prose, conservative entity resolution with review, and privacy defaults (headers only, enrichment off).

### What we do not say

- Not "a better Happenstance", "Happenstance killer", "open Happenstance", or anything that leans on their brand. No parody. The name question is open in [ADR-0001](../decisions/0001-product-name.md) (see [decisions](../decisions/README.md)), and launch copy should not make a trademark problem worse.
- No claims that Happenstance lacks privacy controls, reads email bodies, or does not show evidence. Its security page says it uses communication metadata and not bodies, and it offers privacy settings. Its product attempts to provide evidence.
- No free-tier numbers for competitors that we cannot source. Happenstance's pricing page shows Pro at **$24/month billed annually**, with **$30 monthly** in its indexed pricing description. The "about 15 searches a month free" figure is third-party and **unconfirmed**, so it is not used in any launch copy.
- No performance numbers that were not measured on the release build. Quote the [performance targets](../engineering/performance-targets.md) only as measured results, with the hardware named.
- We do not say "nothing ever leaves your machine". The accurate statement is: "nothing leaves your machine unless you turn on an enricher or a hosted model, and then every call is quoted first and logged".

If a comparison is needed (on HN someone will ask), link to one factual, dated comparison page built from [competitive analysis](../product/competitive-analysis.md). It must also list where Happenstance is stronger: live OAuth connectors, the iOS app, and group reach.

---

## Channels

| Channel | When | Asset | Notes |
|---|---|---|---|
| **Hacker News (Show HN)** | Launch day, a weekday morning US Eastern | Post + first comment | Primary channel. The maintainer stays available for 6+ hours to answer comments |
| **Product Hunt** | 1-7 days after HN, once HN feedback has been folded in | Gallery: demo GIF, `who` score breakdown, Claude Code screenshot | Standalone launch. Do not drop links into competitors' launch threads. Reply there only if someone asks directly for an open-source alternative, and say who you are |
| **Claude Code / agent skill and MCP directories** | Launch day | `playbooks/*.md` skills, MCP server listing with the install line | Submit to the MCP server registries and the skill/plugin directories that exist at launch time (Claude Code plugin and skill lists, Cursor MCP listings, OpenClaw-style skill directories). The listing text is our own and makes no third-party claims |
| **Founder Slacks and communities** | Launch week | A short personal post, not a cross-post blast | Only in communities where the maintainer is already a member, and following each community's self-promotion rules. Ask for feedback on real exports, not for upvotes |
| **r/selfhosted, Lobsters (if invited), Rust community (This Week in Rust)** | Launch week to week 2 | Technical angle: Rust, SQLite + FTS5 + sqlite-vec, local embeddings | Lead with the engineering, not the competitor |
| **X / Bluesky / LinkedIn** | Launch day | The 60-second video natively uploaded | One thread showing the demo and three facts |
| **Follow-up posts** | Phase 3 week, Phase 4 week | "Path to a stranger for about $0.02, quoted first"; "v1.0: installers, local UI, local models" | See [post-launch cadence](#post-launch-cadence) |

### Show HN

**Title** (pick one, 80 characters or fewer, no superlatives):

- `Show HN: Openstance – your network as a SQLite file, free search, works in Claude Code`
- `Show HN: Openstance – search your own network locally, from the CLI or Claude Code`

**URL:** the GitHub repository (not a landing page).

**First comment (draft):**

> I built openstance because the people I know were spread across Gmail, LinkedIn, contacts and calendar, and the tools that search across them run in someone else's cloud and meter the searching.
>
> openstance imports the exports you can already download (Takeout mbox, LinkedIn Connections.csv, .vcf), resolves them into one person per human, and scores each relationship from recency, frequency, reciprocity, meetings and channel breadth. Every component is visible and the weights are a TOML file. Search is SQLite FTS5 plus local embeddings (bge-small, on CPU), so it's free, offline and fast. The same binary runs an MCP server, so Claude Code or Cursor can ask "who do I know at X, and who's my warmest way in?"
>
> Defaults: email headers only (no bodies), no enrichment, nothing leaves the machine. Next week adds optional per-call enrichment for people outside your graph, priced before it runs and logged.
>
> It's Rust, MIT, one SQLite file. Rough edges: entity resolution is deliberately conservative, so expect a short review queue. I'd love bug reports from real exports (please don't paste real contacts into issues; the issue template shows how to reproduce with synthetic data).

---

## Launch checklist

Everything here is checked off before the post goes live. It doubles as Gate A in the [roadmap](roadmap.md#gate-a-launch-readiness-end-of-phase-2).

**Product**

- [ ] The [Phase 2 exit criteria](phase-2.md#exit-criteria) are met.
- [ ] A clean macOS arm64 and Linux x86_64 install works from the README instructions alone (cargo install, or prebuilt binaries if they are ready).
- [ ] The demo dataset `fixtures/demo/` and the 60-second recording reproduce on a clean machine.
- [ ] The network-blocked integration test passes with the default config.
- [ ] `openstance mcp` works in Claude Code and Cursor, using the README snippets.
- [ ] Error messages for the top 5 likely failures have been reviewed: unknown file type, model download failed offline, malformed mbox, empty results, ambiguous name.

**Naming and legal**

- [ ] ADR-0001 (name) is decided or deliberately deferred, and the decision is noted in the README.
- [ ] No Happenstance name, logo, screenshots, or copy in the repository or the launch assets, apart from the one factual comparison page with sources and dates.
- [ ] LICENSE (MIT) is present and the crate metadata matches.
- [ ] All sample people and companies are fictional, and the README says so.

**Repository hygiene**

- [ ] README meets every requirement below.
- [ ] `CONTRIBUTING.md` covers how to add an importer and how to report a bad merge without sharing real data.
- [ ] Issue templates: bug, importer request (format and a sample generated synthetically), bad merge (with a redaction step), feature request.
- [ ] `SECURITY.md` with a private reporting channel.
- [ ] `CODE_OF_CONDUCT.md`.
- [ ] GitHub Discussions enabled, with a "Show your setup" category.
- [ ] `good-first-issue` labels on at least 5 importer, fixture, or doc tasks (for example P5-IMP-08 `csv` with a column map).
- [ ] The docs in `docs/` are linked from the README.

**Launch assets**

- [ ] Show HN title and first comment are final.
- [ ] Demo GIF/MP4 under 10 MB, and an asciinema link.
- [ ] Product Hunt gallery images and tagline.
- [ ] MCP and skill directory submissions are prepared.
- [ ] The comparison page is factual, dated, and sourced, and lists where the competitor is stronger.

**People**

- [ ] The maintainer has a clear calendar for launch day and the next 48 hours.
- [ ] Two or three founders have tried it on their own exports before launch, and their feedback has been addressed or listed as known issues.

---

## README requirements

The README is the landing page. It must contain, in this order:

1. **Name and one-liner.** "openstance: your network as a file. Free local search, works in Claude Code." Include the MIT badge and the CI badge.
2. **The 60-second demo** (GIF inline, with an asciinema link).
3. **Install** in one or two lines (`cargo install --git ...` at launch, the installer script and Homebrew from v1.0).
4. **Quickstart** in 5 commands: `init`, `import`, `review`, `search`, `who`. Show expected output for each.
5. **Where to get your exports.** Short, accurate steps for Google Takeout (mail), LinkedIn data export, and Apple or Google Contacts, with the typical wait time for each.
6. **Use from an agent.** Claude Code, Cursor, and Claude Desktop MCP snippets, plus the list of tools.
7. **Privacy and what costs money.** A table of every path by which data can leave the machine (only the enricher and hosted models, both off by default), what is sent, and how to audit it (`openstance enrich log`). Note the one-time model download.
8. **How the score works.** The formula, the five components, the presets, and the TOML snippet. Link to [scoring](../tech-spec/scoring.md).
9. **Status and limits.** What works today, what is coming in Phases 3 and 4, and known limits (conservative merges, headers only, no live sync).
10. **The file.** Schema overview and the `sqlite3` one-liner. Link to [data model](../tech-spec/data-model.md).
11. **Contributing,** with a pointer to the importer guide and `good-first-issue`s.
12. **License and naming note.** MIT. Say that the name is a working name, if ADR-0001 is still open.

The README must not include unmeasured performance claims, competitor free-tier numbers, testimonials from people who do not exist, or a "vs" table above the fold.

---

## Metrics to watch

openstance has **no telemetry**. The binary never phones home. All metrics come from public or voluntary signals.

| Signal | Source | Why it matters | Check |
|---|---|---|---|
| HN points and comment count, plus the questions people ask | HN | Interest, and the objections we have not answered | Launch day, day 2 |
| Unique visitors and clones | GitHub traffic insights (14-day window, so take a snapshot) | Reach beyond the stars | Days 1, 7, 14 |
| Release and binary download counts | GitHub Releases API, crates.io (once published) | Installs, roughly | Weekly |
| **Non-maintainer issues that mention a real import** (format bugs, merge reports) | GitHub Issues | The strongest signal that strangers ran it on their own data | Daily for 2 weeks |
| **External PRs**, especially importers | GitHub | Contributor pull, which keeps maintenance sustainable | Weekly |
| Importer requests, ranked by count | Issues and Discussions labelled `importer` | Orders the Phase 5 WP1 work | Weekly |
| Directory listings and agent-skill installs (where the directories show counts) | MCP and skill directories | Whether "agent-native" lands | Weekly |
| **"Forgotten person found" reports** | A pinned Discussion asking: "did openstance surface someone relevant you had forgotten, and did it lead to a conversation?" | The product's core validation question | Weekly |
| Stars | GitHub | A vanity metric, but it tracks reach. Weigh it last | Weekly |

Write a short weekly note (in Discussions or the changelog) for the first four weeks. It feeds Gate B and Gate C.

---

## Park criterion

From the roadmap: **if there is no meaningful Show HN response within a month of launch, park the project.**

"Meaningful" is defined before launch, so the decision cannot be rationalised afterwards. **Proposed thresholds** (confirm or adjust them in an ADR before the post goes live):

Within 30 days of the Show HN post, the project continues if **at least two** of the following are true:

1. At least **10 different non-maintainer users** report running openstance on their own exports (issues, Discussions, or direct messages that are verifiable).
2. At least **3 external pull requests** are merged or under active review, or at least 5 well-specified importer requests come with format details.
3. At least **3 "forgotten person found" reports** that led to an actual conversation or introduction.
4. Inbound interest from teams in federation or sync, from at least 3 separate groups.

Stars, points, and downloads are recorded but are not enough on their own to continue.

If the criterion fails, **park**: pin a README status note ("parked as of YYYY-MM-DD; MIT; PRs reviewed best effort"), keep the release binaries up, close the Phase 5 planning issues with a link to the note, and record the outcome in [decisions](../decisions/README.md). If it passes, go to [Phase 5](phase-5-and-beyond.md) and schedule [Gate C](roadmap.md#gate-c-cloud-decision-about-4-weeks-after-v10-only-if-gate-b-passes).

---

## Post-launch cadence

| When | What | Channel |
|---|---|---|
| Launch day (end of Phase 2) | Show HN, social video, MCP and skill directory submissions | HN, X/Bluesky/LinkedIn, directories |
| Launch +1 to 7 days | Product Hunt. Fixes from HN feedback as a `v0.3.x` | PH, GitHub Releases |
| End of Phase 3 | "Path to a stranger: quoted first, cents per call, logged." `v0.4.0` | A comment on our own Show HN thread, X, founder Slacks where relevant |
| End of Phase 4 | "v1.0: one-line install, local UI, bring your own model." `v1.0.0` | GitHub Release, r/selfhosted, This Week in Rust, PH update |
| Launch +30 days | Gate B review and a public status note | Discussions |
| v1.0 +about 4 weeks | Gate C (cloud decision) if Gate B passed | ADR in [decisions](../decisions/README.md) |
