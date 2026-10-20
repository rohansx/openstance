# ADR-0004: Export-first ingestion over OAuth connectors

- **Status:** Accepted
- **Date:** 2026-10-20
- **Related:** [tech-spec/ingestion.md](../tech-spec/ingestion.md), [ADR-0010](0010-email-headers-only-by-default.md), [plan/phase-5-and-beyond.md](../plan/phase-5-and-beyond.md)

## Context

A people-search engine is only as good as the data it ingests. There are two ways to get it:

1. **Live OAuth connectors** (Gmail, Outlook, Google Calendar, Contacts, LinkedIn). This is how Happenstance works. It gives continuous freshness, but requires registered OAuth apps, Google's restricted-scope verification (a CASA security assessment for Gmail scopes), token storage, refresh logic, quota handling and ongoing maintenance per provider. LinkedIn does not offer general third-party graph access at all.
2. **User-downloaded exports** (Google Takeout mbox, LinkedIn `Connections.csv` and messages, X/Twitter archive, `.vcf`, `.ics`, WhatsApp/Slack/Discord/Telegram exports, notes folders, arbitrary CSV). The user gets these themselves; no app registration, no tokens, works offline.

Target outcome from the product overview: value in five minutes with zero API keys.

## Decision

v1 ingests **exports only**. The `Importer` trait turns a file into a stream of `RawIdentity` and `RawInteraction` records and never touches `person` directly:

```rust
pub trait Importer {
    fn kind(&self) -> &'static str;                 // "linkedin_csv"
    fn detect(&self, path: &Path) -> bool;          // sniff, so `openstance import <file>` needs no flag
    fn import(&self, path: &Path, sink: &mut dyn Sink) -> Result<ImportStats>;
}
```

Importer order by phase (see [plan/roadmap.md](../plan/roadmap.md)):

| Phase | Importers |
|---|---|
| 1 | `mbox` (Takeout), `linkedin_csv`, `vcf` |
| 3 | `ics`, `x_archive` |
| 5 | `imap` (app password, incremental by UID), `whatsapp`, `slack`, `discord`, `telegram`, `notes` (Obsidian/Granola/Apple Notes markdown), `pst`; optional OAuth |
| 5 | generic `csv` with a column map (P5-IMP-08; a good first issue that may be pulled into an earlier phase) (CRM exports, event attendee lists). Not in the four-week plan; a stretch item that can reuse the v0.1 header aliases (see [prototype-v0.1.md](../prototype-v0.1.md)). Tracked in [risks-and-open-questions.md](../risks-and-open-questions.md). |

Every import is a `source` row with a `file_hash`; re-importing the same file is a no-op. Raw source rows are never edited; people and ties are derived and can be rebuilt.

Live OAuth sync is **deferred to Phase 5** and, if built, is optional: bring-your-own OAuth app in the open-source binary, managed connectors only as a possible cloud feature.

## Consequences

**Positive**

- No Google CASA audit or Microsoft publisher verification blocks launch.
- No tokens on disk, no refresh failures, no rate limits; works offline.
- Importers are small, testable against synthetic fixtures, and community-contributable.
- Keeping every LinkedIn export as its own source makes job changes diffable (the `watch` playbook).

**Negative**

- Data goes stale between exports; users must re-export. Mitigation: `refresh` and `watch` playbooks, and documented one-minute export instructions per source.
- Onboarding has a "go download your Takeout" step that can take Google hours to prepare. Mitigation: start with LinkedIn CSV and `.vcf` (minutes), add mbox later.
- Some sources (Instagram, live Slack) are weaker via export.
- Happenstance keeps the advantage on live connector breadth; this is accepted ([product/competitive-analysis.md](../product/competitive-analysis.md)).

## Alternatives considered

| Option | Why not (for v1) |
|---|---|
| Gmail/Outlook OAuth first | Verification and security review timeline exceeds the four-week plan; token handling adds attack surface |
| IMAP with app passwords first | Viable and in scope for Phase 5; not first because Takeout covers the same data without credentials |
| Browser extension scraping LinkedIn | Violates LinkedIn terms; fragile |
| Only CSV (like the v0.1 prototype) | Loses the email and calendar interactions that drive scoring |
