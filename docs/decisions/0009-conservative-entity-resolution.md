# ADR-0009: Conservative entity resolution with manual merge and split

- **Status:** Accepted
- **Date:** 2026-10-20
- **Related:** [tech-spec/entity-resolution.md](../tech-spec/entity-resolution.md), [tech-spec/data-model.md](../tech-spec/data-model.md)

## Context

The same human appears as several handles across sources: two email addresses, a LinkedIn URL, an X handle, a phone number in a vcf, a name in a WhatsApp export. Resolving them into one `person` is necessary for scoring and search. Errors are asymmetric: a **wrong merge** puts one person's evidence on another and the user immediately loses trust; a **missed merge** only shows two rows that the user can merge in one command. The v0.1 prototype already enforced "never merge by name alone".

## Decision

Rules run in order; the first match wins and records a confidence the UI shows.

| Rule | Join on | Confidence | Auto-merge |
|---|---|---|---|
| Exact identity | same normalized email, LinkedIn slug, phone (E.164) or X handle | 1.0 | yes |
| Gmail aliasing | `a.b@gmail.com` = `ab@gmail.com` = `ab+x@gmail.com`; corporate plus-addressing | 0.98 | yes |
| Name + company | same normalized name and same company domain or string | 0.85 | yes, flagged |
| Name + mutual thread | same name, both on the same email thread or meeting | 0.75 | yes, flagged |
| Name only, rare | identical name not in the top-5k names list | 0.5 | no, suggested |
| Name only, common | identical common name | 0 | never |

- Normalization: NFKC, lowercase, strip honorifics and emoji, collapse whitespace; company strings strip "Inc", "Ltd", "Pvt" and similar, and known domains map to names.
- Jaro-Winkler at 0.92 or above **only suggests**, never auto-merges.
- **Overrides win over rules every time.** `override(field='merged_into')` forces two people together; `override(field='split_from')` keeps them apart even if a later import matches exactly.
- `openstance review` lists flagged merges (0.75 to 0.85) and suggestions for bulk confirm or split.
- `openstance rebuild` replays all sources, then all overrides; the graph is a pure function of sources plus edits. Property test: rebuild is idempotent.

## Consequences

**Positive**

- Few visible wrong merges; every merge has a reason and a confidence.
- User corrections are durable across re-imports and rebuilds.
- Missed merges are cheap to fix (`openstance merge <a> <b>`).

**Negative**

- More duplicate rows on first import, especially for people known only by name (WhatsApp without vcf, X handles). Mitigation: `review` queue and suggestions.
- The top-5k common-names list must be curated and is culture-biased; it lives in `openstance-core` as data and accepts contributions.
- `review` is a required step for a clean graph, which adds onboarding friction.

## Alternatives considered

| Option | Why not |
|---|---|
| Aggressive fuzzy auto-merge | Wrong merges are visible and trust-destroying |
| ML-based record linkage | No labels; not explainable; overkill at 20k people |
| No auto-merge at all | Too many duplicates; exact email match is safe |
| Name-only merging (common CRM default) | Two people can share a name; v0.1 tests already forbid it |
