# Show HN draft (P2-DOC-04)

Draft of the launch post and the first comment, following [launch.md](launch.md#show-hn). Status: **draft, not final.** Everything here gets checked against the release build before it is posted (see the checklist at the end).

---

## Title

Pick one that is 80 characters or fewer, with no superlatives. Options 1 and 2 are the launch.md titles, kept for reference.

1. `Show HN: Openstance – your network as a SQLite file, free search, works in Claude Code` (86 characters, **too long**)
2. `Show HN: Openstance – search your own network locally, from the CLI or Claude Code` (82 characters, **too long**)
3. `Show HN: Openstance – your network as a SQLite file, searchable from Claude Code` (80 characters)
4. `Show HN: Openstance – search your contacts locally, from the CLI or Claude Code` (79 characters)

Recommendation: **option 3.** It carries all three claims in short form (the file, local search, agent-native) and fits the limit. The two titles in launch.md are 86 and 82 characters long, so they need trimming. Recount after any edit, because the en dash counts as one character.

**URL:** the GitHub repository, not a landing page.

**Text field:** leave it empty. HN shows the URL. The explanation goes in the first comment, posted as soon as the submission is live.

---

## First comment

> I built openstance because the people I know were spread across Gmail, LinkedIn, contacts and calendar, and the tools that search across them run in someone else's cloud and meter the searching.
>
> openstance imports the exports you can already download (Takeout mbox, LinkedIn Connections.csv, .vcf), resolves them into one person per human, and scores each relationship from recency, frequency, reciprocity, meetings and channel breadth. Every component is visible (`ost who "Maya Chen"` prints the five numbers and their weights) and the weights are a TOML file. Search is SQLite FTS5 plus local embeddings (bge-small, on CPU), so it is free and offline, and every result comes with the evidence lines it was found on rather than a summary. The same binary runs an MCP server, so Claude Code or Cursor can ask "who do I know at X, and who's my warmest way in?" and answer from those evidence lines.
>
> Defaults: email headers only (bodies are never read), no enrichment, no telemetry. Nothing leaves your machine except a one-time download of the embedding model, unless you turn on an enricher or a hosted model. Next week adds optional per-call enrichment for people outside your graph, priced before it runs and logged.
>
> It's Rust, Apache-2.0 [CHECK: licence, see below], one SQLite file with a documented schema; `sqlite3 ~/.openstance/graph.db` works. The demo in the README runs on a fictional dataset that ships in the repo (`scripts/demo.sh`), so you can try it before exporting anything of your own.
>
> Rough edges: entity resolution is deliberately conservative, so expect a short review queue. Calendar import arrives next week, so meetings count as zero until then. I'd love bug reports from real exports (please don't paste real contacts into issues; CONTRIBUTING.md shows how to reproduce with synthetic data).

Notes on this draft, compared with the launch.md version:

- It adds the `who` example, the evidence-lines point and the fictional demo dataset. Each of these answers an objection people are likely to raise early ("is the score a black box?", "is the agent making things up?", "do I have to hand over my data to try it?").
- It changes "nothing leaves the machine" to the accurate statement from [launch.md, What we do not say](launch.md#what-we-do-not-say), and names the model download.
- It says that meetings are zero until calendar import lands, because the first person to read `who` output will notice `M 0.000`.
- It names no competitor. If someone asks for a comparison, link the dated, sourced comparison page (launch checklist) and say where the other product is stronger.

---

## Prepared replies

Short answers to the questions most likely to come up in the first hour. Each one is a starting point and gets adapted to the actual comment. Do not paste them verbatim.

**"How is this different from <hosted product>?"**
> It runs on your machine against exports you download yourself, and searching is free and unmetered. The hosted tools have things this doesn't: live connectors (no re-exporting), mobile apps, and searching across a group's combined networks. There's a dated comparison here: <link>.

**"Does it read my email?"**
> Headers only: From/To/Cc, date, subject, message ids and threading. The mbox importer skips bodies entirely and they are never stored. There's a test that plants a sentinel string in every body and checks it never reaches the database.

**"Why do I need an embedding model? Can I turn it off?"**
> It's what lets "people who build developer tools" match someone whose title says "SDK engineer". It runs on CPU and is downloaded once (about 33 MB). `model.embedder = "none"` turns it off, and search falls back to keyword search.

**"The score is arbitrary."**
> It is a heuristic, and that's why it is shown in full: five components, each with its formula, and weights you can change in TOML. `who` prints the breakdown for any person. If a number looks wrong to you, that's a bug report I want.

**"Entity resolution merged two different people / missed an obvious duplicate."**
> It merges automatically only on exact handle matches. Same-name-plus-same-company merges are applied but flagged for review, and fuzzy name matches are only ever suggestions. `ost split` undoes a merge and the decision survives re-import. If you can reproduce it with made-up names, please open a "bad merge" issue.

**"What's the business model?"**
> Local search is free and stays free. The paid part, from next week, is optional enrichment for people outside your graph: per call, priced before it runs, logged. There is no account and no subscription.

**"Why Rust / why SQLite?"**
> One static binary, no runtime, and a single file you can open with any SQLite tool, back up or delete. FTS5 and sqlite-vec do the search inside the same file.

---

## Before posting

- [ ] Title recounted (80 characters or fewer) and chosen.
- [ ] Every command and number in the first comment checked against the release build: `who` output shape, the model size, the `scripts/demo.sh` path.
- [ ] **Licence:** launch.md and the demo title card say MIT, while `Cargo.toml` and the current README say Apache-2.0, and there is no LICENSE file yet. Settle it and fix the comment, the README badge and the crate metadata together.
- [ ] Name: ADR-0001 decided or deliberately deferred. If deferred, the README says it is a working name.
- [ ] "Next week" is still accurate (the Phase 3 dates in the [roadmap](roadmap.md)).
- [ ] The comparison page exists, is dated and sourced, and lists where the other product is stronger.
- [ ] No performance figure appears that was not measured on the release build with the hardware named.
- [ ] The maintainer is free for the next 6 or more hours to answer comments.
