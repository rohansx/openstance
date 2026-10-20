# ADR-0010: Email headers only by default; bodies opt-in

- **Status:** Accepted (default may be revisited; see open question)
- **Date:** 2026-10-20
- **Related:** [tech-spec/ingestion.md](../tech-spec/ingestion.md), [tech-spec/configuration.md](../tech-spec/configuration.md), [risks-and-open-questions.md](../risks-and-open-questions.md)

## Context

Email is the richest relationship source. Headers (From, To, Cc, Date, Subject, Message-ID, In-Reply-To, List-Unsubscribe) are enough for identities, interactions, reciprocity, thread membership, ties and bulk detection. Bodies add signatures (title, company, phone) and topical content that would improve semantic search, but they are by far the most sensitive data and multiply database size and import time.

Happenstance states it uses communication metadata, not message bodies. openstance's local-first story would be undermined if it stored more than a cloud competitor by default, even locally.

## Decision

- `mbox` (and later `imap`) importers read **headers only** by default: `From, To, Cc, Date, Subject, Message-ID, In-Reply-To, References, List-Unsubscribe`.
- `[import] email_bodies = false` is the default. `openstance import --bodies <file>` or `email_bodies = true` opts in.
- When bodies are enabled, v1 extracts **signatures only** (title/company/phone hints via regex) into `RawIdentity` hints. Full body text is not stored in v1. Storing and indexing bodies is a separate future decision.
- `interaction.subject` stores the subject (or meeting title / note heading), never body text.

## Consequences

**Positive**

- Smaller file, faster import (5 GB mbox under 2 min target is only realistic headers-only).
- Lower blast radius if the file leaks.
- Honest comparison with competitors.

**Negative**

- Weaker search for topical queries ("people I discussed pricing with"). Partly offset by subjects in `person_fts.subjects` and embeddings.
- Title/company coverage depends on LinkedIn CSV and vcf unless bodies are opted in.

## Alternatives considered

| Option | Why not (as default) |
|---|---|
| Bodies on by default | Privacy story worse; larger file; slower import |
| Bodies on, stored encrypted | Still increases sensitive data; adds key management |
| Subjects off too | Loses the best cheap evidence line ("thread: Q3 fundraising") |

**Open question:** whether search quality without bodies is good enough is measured in Phase 2 evals. See [risks-and-open-questions.md](../risks-and-open-questions.md).
