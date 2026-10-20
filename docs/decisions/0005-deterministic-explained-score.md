# ADR-0005: Deterministic, explained relationship score

- **Status:** Accepted
- **Date:** 2026-10-20
- **Related:** [tech-spec/scoring.md](../tech-spec/scoring.md), [tech-spec/configuration.md](../tech-spec/configuration.md), [ADR-0012](0012-spec-corrections.md)

## Context

Ranking people by "how well do I know them" is central to every query ("who do I know at Stripe" is useless if a newsletter sender ranks first). Options range from an opaque learned model to plain arithmetic. Users of existing tools ask "is everyone in my inbox a warm intro?" and have no way to see or correct the logic. The v0.1 prototype used a hand-entered `relationship` field (Strong/Warm/Weak/Unknown), which does not scale and is not derived from evidence.

## Decision

The relationship score is a **deterministic weighted sum of five components**, each in `[0, 1]`, all stored and shown per person, with weights from a TOML preset the user can edit.

```
S = w_r*R + w_f*F + w_c*C + w_m*M + w_b*B
```

| Component | Measures | Formula |
|---|---|---|
| R, recency | how recently you interacted | `exp(-ln(2) * days_since_last / half_life_days)`, `half_life_days = 180` (R = 0.5 at 180 days) |
| F, frequency | how often, diminishing returns | `1 - exp(-n_interactions / 20)` |
| C, reciprocity | did they engage back | `min(1, replies_from_them / max(1, messages_from_you))`; a thread they started counts double |
| M, meetings | calendar / in-person signal | `1 - exp(-n_meetings / 3)` |
| B, breadth | channels seen | `channels_seen / 4` (email, calendar, linkedin dm, social) |

Weights (R, F, C, M, B):

| Preset | R | F | C | M | B |
|---|---|---|---|---|---|
| **default** (used when config is empty; founder-leaning) | 0.35 | 0.20 | 0.25 | 0.15 | 0.05 |
| founder | 0.30 | 0.15 | 0.30 | 0.20 | 0.05 |
| recruiter | 0.35 | 0.20 | 0.15 | 0.10 | 0.20 |
| seller | 0.25 | 0.15 | 0.30 | 0.25 | 0.05 |
| community | 0.15 | 0.40 | 0.25 | 0.10 | 0.10 |

Bulk senders (50+ messages and zero replies either way, or `List-Unsubscribe` on most messages) get `person.is_bulk = 1` and score 0, and are excluded from search by default.

Every search result and `who` output includes the five components, the weights used, and the inputs (`days_since_last`, `n_interactions`, replies, meetings, channels) so the score is explainable as "why 0.81".

## Consequences

**Positive**

- A number users can argue with: every component is visible and every weight is editable.
- Reproducible: the same sources plus overrides give the same scores (property-tested via `openstance rebuild`).
- Golden tests: ten hand-computed people in `fixtures/`; changing weights requires an intentional golden update.
- Costs nothing per query; no model required.

**Negative**

- Hand-picked constants (20, 3, 4, 180 days) are guesses. Mitigation: they are config keys; tune against the search eval set in Phase 2.
- Does not capture sentiment or relationship quality beyond counts.
- Heuristic bulk detection can mislabel real people who never reply. Mitigation: the user can see bulk-flagged people (`openstance search --include-bulk`) and correct them with `openstance edit <p> --not-bulk`, which writes an `is_bulk` scalar override (an extension of the canonical scalar override fields; see [tech-spec/data-model.md](../tech-spec/data-model.md#47-override-edit)).

## Alternatives considered

| Option | Why not |
|---|---|
| Learned ranking model | No labels, not explainable, not reproducible across versions |
| LLM-judged relationship strength | Costs money per person, non-deterministic, unverifiable |
| Manual strength field (v0.1) | Does not scale beyond a few dozen people; not evidence-based |
| Single opaque number | Fails the core product promise of evidence over prose |
