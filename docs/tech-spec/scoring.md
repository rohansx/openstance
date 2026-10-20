# Relationship scoring

Status: build-ready spec for Phase 1 (scoring, presets, bulk detection) and Phase 2 (tie weights and path strength used by `how_connected`).
Crate: `openstance-core` (`core::score`, `core::ties`). No model and no network are involved.

Related: [data model](data-model.md), [search](search.md), [playbooks](playbooks.md), [configuration](configuration.md), [ADR-0005](../decisions/0005-deterministic-explained-score.md), [testing strategy](../engineering/testing-strategy.md).

---

## 1. Goals and non-goals

The relationship score answers one question: "how well do I know this person, according to the evidence on disk?" It is used to:

- order structured searches (no text) and lift text matches during fusion ([search](search.md#63-fusion));
- pick mutuals for paths (`how_connected`, `path_to`);
- drive the `reconnect` playbook (high frequency, low recency).

Requirements:

1. **Deterministic.** Same sources + same overrides + same `now` gives the same score, bit for bit (up to float formatting). `openstance rebuild` is idempotent.
2. **Explainable.** Every component is stored and returned so the UI and CLI can show "why 0.81".
3. **Editable.** Weights come from a preset or an explicit weight table in `config.toml`.
4. **Cheap.** A full rescore of 20k people and 500k interactions runs well inside the 60 s rebuild budget; the incremental recency refresh (section 7) runs in under 200 ms.

Non-goals: sentiment, learned ranking, LLM judgments of closeness. See [ADR-0005](../decisions/0005-deterministic-explained-score.md) for why.

---

## 2. Formula

```
S = w_r*R + w_f*F + w_c*C + w_m*M + w_b*B
```

Each component is in `[0, 1]`. Weights are non-negative and sum to 1 (they are normalized on load; see section 5), so `S` is also in `[0, 1]`.

| Symbol | Name | Formula | Inputs (per person) |
|---|---|---|---|
| R | recency | `exp(-ln(2) * days_since_last / half_life_days)` | `days_since_last`, `half_life_days` (default 180) |
| F | frequency | `1 - exp(-n_interactions / 20)` | `n_interactions` |
| C | reciprocity | `min(1, (n_replies + 2 * n_threads_started) / max(1, n_out))`, and `0` when `n_out = 0` | `n_out`, `n_replies`, `n_threads_started` |
| M | meetings | `1 - exp(-n_meetings / 3)` | `n_meetings` |
| B | breadth | `channels_seen / 4` | set of channels with at least one interaction |

> **Differs from the original spec PDF:** the PDF wrote recency as `exp(-days/half_life)`, which gives R = 0.37 (not 0.5) at the half-life. The fixed formula uses `ln(2)` so `half_life_days` means what it says: R = 0.5 at 180 days, 0.25 at 360, 0.125 at 540. The PDF's reciprocity formula also let a cold inbound sender reach C = 1 without the owner ever writing back; the `n_out = 0 => C = 0` guard below closes that.

### 2.1 Which interactions count

Scoring reads `interaction` joined with `interaction_person` for one person. The owner's own person row (the "self person", resolved from the owner's identities at import) is never scored and never appears in search results or ties.

Interaction kinds (from [data model](data-model.md)) are split into two groups:

| Group | Kinds | Counts toward |
|---|---|---|
| contact | `email`, `meeting`, `dm`, `connection` | R, F, C (email and dm only), M (meeting only), B |
| weak | `follow`, `mention` | B only |

A LinkedIn `connection` row counts as one contact event at the connection date. `follow` and `mention` (X archive and later social importers) only prove that a channel exists; they never move R or F, because following someone is not knowing them.

Interactions are counted once per person, no matter how many roles the person had on them (`from` and `cc` on the same message is one interaction).

### 2.2 Recency, R

```
last_contact   = max(interaction.at) over contact-group interactions with the person
days_since_last = max(0, (now - last_contact) / 86400)        -- fractional days, clamped
R = exp(-ln(2) * days_since_last / half_life_days)
```

- `now` is the scoring time (unix seconds), passed in explicitly so tests are reproducible (`Scorer::score(&self, now: i64)`); never read the clock deep inside core.
- **Future timestamps** (clock skew in mail headers, future calendar events): clamped to 0 days, so R = 1. Calendar events in the future do not set `last_contact`; only `at <= now` counts. They still count for M once they are in the past.
- **No contact interactions** (only a vCard entry, or only `follow`): `last_contact = NULL`, R = 0.
- `person.last_contact` is the same value, stored for sorting and for the `last_contact_before` filter in [search](search.md#52-per-field-clauses).

| days_since_last | R (half-life 180) |
|---|---|
| 0 | 1.000 |
| 30 | 0.891 |
| 90 | 0.707 |
| 180 | 0.500 |
| 365 | 0.245 |
| 540 | 0.125 |
| 720 | 0.063 |

### 2.3 Frequency, F

```
n_interactions = count(distinct interaction.id) over contact-group interactions
F = 1 - exp(-n_interactions / 20)
```

F = 0.39 at 10 interactions, 0.63 at 20, 0.92 at 50, ~1 by 100. Zero interactions gives F = 0. The constant 20 is a tunable (`score.frequency_scale`, see section 5).

Known weakness, accepted for Phase 1: a 40-recipient announcement email counts as one interaction for every recipient. Ingestion already keeps only headers; group dampening is tracked as a tuning question in section 10.

### 2.4 Reciprocity, C

Counts over `email` and `dm` interactions only (meetings are M's job):

| Count | Definition |
|---|---|
| `n_out` | interactions with `direction = 'out'` where the person is a `to`/`cc`/`recipient` |
| `n_replies` | interactions with `direction = 'in'` where the person is `from`/`sender` and the message is a reply in a thread that contains an earlier `out` message to them |
| `n_threads_started` | interactions with `direction = 'in'` where the person is `from`/`sender` and the message starts a thread (no `In-Reply-To` / `References`, or the DM conversation's first message) |

```
if n_out == 0:  C = 0
else:           C = min(1, (n_replies + 2 * n_threads_started) / n_out)
```

- "A thread they started counts double" (from the spec): someone who reaches out to you is a stronger signal than someone who only answers.
- **`n_out = 0` guard:** without any outbound message from the owner (on any channel), there is no reciprocity to measure. This keeps cold inbound senders and recruiters at C = 0. They can still have R and F.
- **Thread identity:** email threads are keyed by the root Message-ID derived from `In-Reply-To` / `References` at import; DMs by conversation id. The importer records this on the interaction (see [ingestion](ingestion.md) and [data model](data-model.md); field name owned there). If an importer cannot provide thread identity, fall back to normalized subject (strip `Re:`, `Fwd:`, `AW:`, whitespace, case) plus a 30-day window.
- DMs: a reply is an inbound DM within 14 days after an outbound DM in the same conversation.
- Auto-replies: messages with `Auto-Submitted: auto-replied` (when the importer captures it) are not replies.
- `replied` in search means `C > 0` ([search](search.md#52-per-field-clauses)).

### 2.5 Meetings, M

```
n_meetings = count(distinct interaction.id) where kind = 'meeting' and at <= now and the person is an attendee
M = 1 - exp(-n_meetings / 3)
```

M = 0.28 at 1 meeting, 0.49 at 2, 0.74 at 4, 0.95 at 9. Declined events are dropped by the `ics` importer and never become interactions. Meeting-only contacts (no email) are common for founders; M plus R carries them.

### 2.6 Breadth, B

Channels are derived from `source.kind` of the interaction's source:

| Channel | Source kinds |
|---|---|
| `email` | `mbox`, `imap` |
| `calendar` | `ics` |
| `linkedin` | `linkedin_csv` (connection or dm) |
| `social` | `x_archive`, `whatsapp`, and later `slack`, `discord`, `telegram` |

```
B = |channels with >= 1 interaction (contact or weak group)| / 4
```

`vcf`, `csv` and `enrich:*` sources are address books and lookups, not channels; they contribute nothing to B.

---

## 3. Bulk senders

Bulk people are flagged before scoring and get `S = 0` with all components stored as 0.

`person.is_bulk INTEGER NOT NULL DEFAULT 0` is set to 1 when either rule matches:

1. **Volume without conversation:** 50 or more email interactions with the person, and zero replies in either direction (`n_out = 0` and `n_replies = 0` and no outbound message from the owner on any thread they appear in).
2. **List mail:** more than 50% of their inbound messages carried a `List-Unsubscribe` header (the mbox and imap importers record this flag per interaction; see [ingestion](ingestion.md)).

Effects:

- `score = 0`, components 0, `is_bulk = 1`.
- Excluded from search unless `include_bulk = true` (`--include-bulk` on the CLI); see [search](search.md).
- Excluded from tie derivation (a newsletter does not connect its 40k recipients) and from embeddings (saves time; see [models](models.md#6-embedding-text-and-re-indexing)).
- Still visible in `openstance who <email>` with `"bulk": true` and the rule that fired, so a user can see why.

A false positive is corrected with `openstance edit <p> --not-bulk`, which writes an `is_bulk = '0'` scalar override ([data model](data-model.md#47-override-edit)); like every override it always wins over the derived flag.

---

## 4. Presets

| Preset | Emphasis | R | F | C | M | B |
|---|---|---|---|---|---|---|
| `default` | used when config sets nothing; founder-leaning | 0.35 | 0.20 | 0.25 | 0.15 | 0.05 |
| `founder` | people who reply and meet | 0.30 | 0.15 | 0.30 | 0.20 | 0.05 |
| `recruiter` | breadth and recency | 0.35 | 0.20 | 0.15 | 0.10 | 0.20 |
| `seller` | meetings and replies | 0.25 | 0.15 | 0.30 | 0.25 | 0.05 |
| `community` | frequency over recency | 0.15 | 0.40 | 0.25 | 0.10 | 0.10 |

Every preset sums to 1.00. Presets are compiled into the binary (`core::score::presets`), so changing one is a code change that updates the golden file.

---

## 5. Configuration

`~/.openstance/config.toml` (full key list in [configuration](configuration.md)):

```toml
[score]
preset = "default"            # default | founder | recruiter | seller | community
half_life_days = 180          # R = 0.5 at this many days
# Explicit weights replace the preset entirely. Mutually exclusive with `preset`
# being set explicitly; missing keys are an error, not 0.
# weights = { recency = 0.30, frequency = 0.15, reciprocity = 0.30, meetings = 0.20, breadth = 0.05 }

# Tunables (rarely changed; defaults shown)
frequency_scale = 20          # F = 1 - exp(-n / frequency_scale)
meetings_scale = 3            # M = 1 - exp(-n / meetings_scale)
tie_scale = 3                 # tie weight normalization, section 8
tie_max_participants = 20     # interactions with more people do not create ties
```

Validation rules (errors print the key path):

- `half_life_days` > 0; `frequency_scale`, `meetings_scale`, `tie_scale` > 0.
- `weights`: all five keys present, each >= 0, sum > 0. If the sum is not 1.0 (tolerance 1e-6), weights are divided by their sum and a `tracing::warn!` names the normalized values.
- `preset` and `weights` both set explicitly: error.
- Changing any score key marks scores stale; the next command that reads scores runs a full rescore (section 7). No re-import is needed.

Rust shape:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Weights { pub recency: f64, pub frequency: f64, pub reciprocity: f64, pub meetings: f64, pub breadth: f64 }

#[derive(Debug, Clone)]
pub struct ScoreConfig {
    pub weights: Weights,          // resolved from preset or explicit table, normalized
    pub preset: Option<String>,    // None when explicit weights were given
    pub half_life_days: f64,
    pub frequency_scale: f64,
    pub meetings_scale: f64,
    pub tie_scale: f64,
    pub tie_max_participants: usize,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Components { pub recency: f64, pub frequency: f64, pub reciprocity: f64, pub meetings: f64, pub breadth: f64 }

#[derive(Debug, Clone, Default, Serialize)]
pub struct ScoreInputs {
    pub last_contact: Option<i64>,
    pub days_since_last: Option<f64>,
    pub n_interactions: u32,
    pub n_out: u32,
    pub n_replies: u32,
    pub n_threads_started: u32,
    pub n_meetings: u32,
    pub channels: Vec<Channel>,   // sorted, deduped
}

pub fn components(i: &ScoreInputs, cfg: &ScoreConfig) -> Components;
pub fn total(c: &Components, w: &Weights) -> f64;
```

`components` and `total` are pure functions; all SQL lives in `ScoreInputs` loaders. Property tests assert each component is in `[0, 1]`, R is monotone non-increasing in days, F and M are monotone non-decreasing in counts.

---

## 6. Storage and exposure ("why 0.81")

### 6.1 Stored columns

The cached score and its five components live on `person` so search can filter and sort in SQL without recomputing:

| Column | Type | Meaning |
|---|---|---|
| `score` | REAL | S, 0 for bulk and for the self person |
| `score_r` | REAL | R (recency) |
| `score_f` | REAL | F (frequency) |
| `score_c` | REAL | C (reciprocity) |
| `score_m` | REAL | M (meetings) |
| `score_b` | REAL | B (breadth) |
| `last_contact` | INTEGER | unix seconds, input to R |
| `is_bulk` | INTEGER NOT NULL DEFAULT 0 | section 3 |

The exact DDL is owned by [data model](data-model.md). The raw counts (`n_out`, `n_replies`, ...) are not stored: `who` recomputes them from `interaction_person` for one person in well under the 20 ms `who` budget, and storing them would create a second source of truth.

Scoring metadata (`scored_at`, the resolved weights and `half_life_days`, a hash of the score config) is kept in the `meta` table ([data model](data-model.md)) so a stale score can be detected.

### 6.2 JSON shape

Every search result and every `get_person` / `openstance who` response carries:

```json
{
  "score": 0.885,
  "score_detail": {
    "preset": "default",
    "weights":    { "recency": 0.35, "frequency": 0.20, "reciprocity": 0.25, "meetings": 0.15, "breadth": 0.05 },
    "components": { "recency": 0.891, "frequency": 0.878, "reciprocity": 1.0, "meetings": 0.736, "breadth": 0.75 },
    "contributions": { "recency": 0.312, "frequency": 0.176, "reciprocity": 0.25, "meetings": 0.110, "breadth": 0.038 },
    "inputs": {
      "last_contact": "2026-09-20", "days_since_last": 30,
      "n_interactions": 42, "n_out": 18, "n_replies": 15, "n_threads_started": 3,
      "n_meetings": 4, "channels": ["calendar", "email", "linkedin"]
    },
    "bulk": false,
    "half_life_days": 180,
    "scored_at": "2026-10-20T09:12:00Z"
  }
}
```

Search results include only `score` and `components` by default (keeps result lists small); `who`/`get_person` include the full `score_detail`. `inputs` are computed on demand. The envelope and field naming rules are owned by [interfaces](interfaces.md).

CLI rendering for `openstance who priya@acme.io`:

```
Priya Raman  ·  VP Engineering, Acme  ·  score 0.885 (default preset)
  recency      0.891 x 0.35 = 0.312   last contact 2026-09-20 (30 days)
  frequency    0.878 x 0.20 = 0.176   42 interactions
  reciprocity  1.000 x 0.25 = 0.250   15 replies + 3 threads started (x2) / 18 sent
  meetings     0.736 x 0.15 = 0.110   4 meetings
  breadth      0.750 x 0.05 = 0.038   email, calendar, linkedin
```

---

## 7. When scores are computed

| Trigger | Work |
|---|---|
| End of every `openstance import` | rescore people touched by the import, then rebuild their ties |
| `openstance rebuild` | full rescore of every person, full tie rebuild |
| Score config changed (config hash differs from meta) | full rescore on next command that reads scores |
| `scored_at` older than 24 h | recency refresh: recompute R and S for all people from stored components and `last_contact` (one UPDATE pass, < 200 ms for 20k people); F, C, M, B do not change with time |
| `openstance edit`, `merge`, `split` | rescore the affected people (merge/split change the interaction sets) |

The recency refresh keeps cached scores honest for people who reopen the tool after weeks: without it, "last contact 30 days ago" would stay frozen at import time.

Implementation note: rescoring is one aggregate query per pass (group by `person_id` over `interaction_person` joined with `interaction` and `source`), evaluated in Rust, then a batched `UPDATE person SET score=?, score_r=?, ... WHERE id=?` in one transaction.

---

## 8. Ties and path strength

Ties connect two people (neither is the owner) who appear together on interactions. They are used by `how_connected`, `path_to` and `brief` ([playbooks](playbooks.md)).

### 8.1 Derivation

For every interaction with between 2 and `tie_max_participants` (default 20) non-owner, non-bulk participants, every unordered pair `(a, b)` with `a < b` shares that interaction. Larger interactions (all-hands invites, big announcement threads) create no ties; without the cap a single 200-person email would create about 20k pair rows.

Tie kind comes from the interaction kind:

| interaction.kind | tie.kind |
|---|---|
| `email` | `email` (cc'd together, on the same thread) |
| `meeting` | `meeting` (same calendar event) |
| `dm` (group DM) | `dm` |

### 8.2 Weight

For each `(a, b, kind)`:

```
D        = sum over shared interactions i of exp(-ln(2) * age_days(i) / half_life_days)
weight   = 1 - exp(-D / tie_scale)            -- tie_scale default 3, weight in [0, 1)
n_shared = count of shared interactions
last_at  = max(i.at)
```

The tie row stores `kind, weight, last_at, n_shared` (see [data model](data-model.md)). Evidence is never stored: it is the join of `interaction_person` on both people.

```sql
-- evidence for a tie between :a and :b, most recent first
SELECT i.id, i.kind, i.at, i.subject
FROM interaction i
JOIN interaction_person pa ON pa.interaction_id = i.id AND pa.person_id = :a
JOIN interaction_person pb ON pb.interaction_id = i.id AND pb.person_id = :b
GROUP BY i.id
ORDER BY i.at DESC
LIMIT 3;
```

When two people have ties of more than one kind, path computations combine them as independent signals:

```
tie_strength(a, b) = 1 - product over kinds k of (1 - weight_k(a, b))
```

Tie weights are recomputed together with scores (section 7), so they decay too.

### 8.3 Path strength

A path is `me -> mutual -> target` (one intermediary). Direct contacts are reported as a direct relationship, not a path.

```
path_strength(mutual, target) = S(mutual) * tie_strength(mutual, target)
```

`how_connected(target)` returns the top 3 mutuals by `path_strength`, each with the tie evidence query above. Bulk people and the owner are never mutuals. `path_to` adds employer/school intersection for targets not in the graph; see [playbooks](playbooks.md).

If the target is itself in the graph with `S(target) > 0`, the response also includes `"direct": {"score": S(target), ...}` first, because a direct relationship always beats an intro.

---

## 9. Worked example

Scoring time: `now = 2026-10-20`, default preset, `half_life_days = 180`.

### 9.1 Inputs

| Person | days since last | n_interactions | n_out | n_replies | n_threads_started | n_meetings | channels |
|---|---|---|---|---|---|---|---|
| Priya (close colleague) | 30 | 42 | 18 | 15 | 3 | 4 | email, calendar, linkedin |
| Marcus (old co-founder friend) | 450 | 60 | 30 | 24 | 0 | 6 | email, calendar |
| Dana (new LinkedIn connection) | 10 | 2 (connection + 1 DM out) | 1 | 0 | 0 | 0 | linkedin |
| news@vendor.io | 3 | 120 inbound | 0 | 0 | 0 | 0 | email |

### 9.2 Components

| Person | R | F | C | M | B |
|---|---|---|---|---|---|
| Priya | `2^(-30/180)` = 0.8909 | `1-e^(-42/20)` = 0.8775 | `min(1, (15+2*3)/18)` = min(1, 1.167) = 1.0000 | `1-e^(-4/3)` = 0.7364 | 3/4 = 0.7500 |
| Marcus | `2^(-450/180)` = 0.1768 | `1-e^(-60/20)` = 0.9502 | 24/30 = 0.8000 | `1-e^(-6/3)` = 0.8647 | 2/4 = 0.5000 |
| Dana | `2^(-10/180)` = 0.9622 | `1-e^(-2/20)` = 0.0952 | 0/1 = 0.0000 | 0.0000 | 1/4 = 0.2500 |
| news@vendor.io | bulk (rule 1 and 2) | | | | |

### 9.3 Scores

Default preset, contributions `w * component`:

| Person | R | F | C | M | B | **S** |
|---|---|---|---|---|---|---|
| Priya | 0.3118 | 0.1755 | 0.2500 | 0.1105 | 0.0375 | **0.8853** |
| Marcus | 0.0619 | 0.1900 | 0.2000 | 0.1297 | 0.0250 | **0.6066** |
| Dana | 0.3368 | 0.0190 | 0.0000 | 0.0000 | 0.0125 | **0.3683** |
| news@vendor.io | | | | | | **0** (bulk) |

Same people under every preset:

| Person | default | founder | recruiter | seller | community |
|---|---|---|---|---|---|
| Priya | 0.8853 | 0.8837 | 0.8610 | 0.8760 | 0.8833 |
| Marcus | 0.6066 | 0.6335 | 0.5584 | 0.6679 | 0.7431 |
| Dana | 0.3683 | 0.3154 | 0.4058 | 0.2673 | 0.2074 |

Reading it: `community` rewards Marcus's long history; `recruiter` rewards Dana's recency and breadth. Marcus has F = 0.95 and R = 0.18, so he matches `lost_touch` (F >= 0.5 and R < 0.2) and is a `reconnect` candidate.

### 9.4 Path example

Target Sam is not a direct contact. Shared interactions:

| Mutual | Shared interactions with Sam (age in days) | D | tie weight | path strength |
|---|---|---|---|---|
| Priya | 5 emails (20, 45, 90, 200, 400) | 3.1511 | `1-e^(-3.1511/3)` = 0.6502 | 0.8853 x 0.6502 = **0.5756** |
| Dana | 1 meeting (5) | 0.9809 | 0.2789 | 0.3683 x 0.2789 = **0.1027** |
| Marcus | 2 meetings (500, 600) | 0.2450 | 0.0784 | 0.6066 x 0.0784 = **0.0476** |

`how_connected("Sam")` returns Priya, Dana, Marcus in that order, each with up to three evidence interactions.

These numbers are the seed for `fixtures/golden/scoring.toml` (see [testing strategy](../engineering/testing-strategy.md)); the golden test asserts components and S to 1e-4.

---

## 10. Tuning and open questions

Constants are guesses until the Phase 2 search eval exists ([phase 2](../plan/phase-2.md)). Candidates to revisit, in order:

1. Group dampening for F: count an interaction with more than N participants as `1/2` or less.
2. Large-meeting dampening for M (all-hands invites).
3. A `noreply@` / `notifications@` local-part rule for bulk detection.
4. Two-hop paths (`me -> a -> b -> target`) for `path_to`; Phase 3 at the earliest.

Any change to a formula, constant or preset updates the golden file in the same PR. Tracked in [risks and open questions](../risks-and-open-questions.md).
