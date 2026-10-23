# Playbooks

A playbook is a small, fixed sequence of steps over the local graph. It may include metered egress steps through the enricher. It returns a typed result and a cost receipt. Playbooks are what a person (or an agent) does *after* a search: find a path to a stranger, map accounts, refresh stale rows, watch for job changes, write an intro brief, pick people to reconnect with, check an address before sending.

There is **no open-ended agent loop**. Each playbook has a fixed step list, and the only branch is "enough local data, skip egress". The CLI, the MCP server and the HTTP API call the same Rust function.

Related: [enrichment.md](enrichment.md) (egress gate, budget, quote), [scoring.md](scoring.md) (R, F, C, M, B, tie weight), [search.md](search.md), [interfaces.md](interfaces.md) (how each surface exposes playbooks), [configuration.md](configuration.md#playbooks).

| Playbook | Phase | Egress (max per run) | Surfaces |
| --- | --- | --- | --- |
| `path` | 3 | 1 profile, 1 find_email, 1 verify_email | CLI `path`, MCP `path_to`, HTTP `POST /playbooks/path` |
| `brief` | 3 | 1 web_search | CLI `brief`, MCP `brief`, HTTP |
| `accounts` | 4 | 0 | CLI `accounts`, HTTP |
| `refresh` | 4 | n profiles, capped by budget | CLI `refresh`, HTTP |
| `watch` | 4 | up to `playbooks.watch.top_n` (100) profiles | CLI `watch`, HTTP |
| `reconnect` | 4 | 0 | CLI `reconnect`, HTTP |
| `verify` | 4 | 1 verify_email | CLI `verify`, HTTP |

Only `path` and `brief` are MCP tools. Agents cover `accounts` and `reconnect` with `search_people` (company filter, `lost_touch`) plus `how_connected`. The skill files in section 10 tell them how. The other playbooks are human, cron or HTTP workflows. This keeps the MCP tool list at the eight canonical tools ([interfaces.md](interfaces.md#2-mcp-server)).

---

## 1. Types

```rust
// openstance-core/src/playbook/mod.rs

pub trait Playbook: Send + Sync {
    fn name(&self) -> &'static str;
    /// JSON Schema of the input, used by MCP tool registration and HTTP validation.
    fn input_schema(&self) -> &'static serde_json::Value;
    /// Steps and quoted cost, before running. Pure local reads plus EgressGate::quote; no egress.
    fn plan(&self, g: &Graph, gate: &mut EgressGate, input: &Value) -> Result<Plan>;
    /// Execute. Egress only through `gate` (which wraps the configured Enricher), only if ctx.allow_egress.
    fn run(&self, g: &Graph, gate: &mut EgressGate, input: &Value, ctx: &RunCtx) -> Result<Outcome>;
}

pub struct RunCtx {
    pub allow_egress: bool,
    pub budget_micro_usd: MicroUsd,   // already resolved per enrichment.md section 5
    pub now: i64,                     // injected for deterministic tests
    pub run_id: String,               // ULID
}

#[derive(Serialize)]
pub struct Plan {
    pub playbook: String,
    pub run_id: String,
    pub steps: Vec<PlanStep>,
    pub quote: Quote,                 // enrichment.md; empty lines if no egress planned
    pub egress_max: Vec<EgressMax>,   // static ceiling for this playbook
}

#[derive(Serialize)]
pub struct PlanStep {
    pub n: u8,
    pub description: String,          // "lookup target in graph"
    pub egress: Option<String>,       // Some("profile") if this step may call out
    pub will_run: StepDecision,       // run | skip_local_sufficient | skip_no_enricher | skip_no_egress
}

#[derive(Serialize)]
pub struct EgressMax { pub op: String, pub max: u32 }

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus { Done, Partial, Quote, NothingToDo }

#[derive(Serialize)]
pub struct Outcome {
    pub playbook: String,
    pub status: RunStatus,
    pub plan: Plan,
    pub result: Option<Value>,        // playbook-specific, typed per section below
    pub receipt: Receipt,
    pub notes: Vec<String>,           // human-readable: "target not in graph; enricher is none"
}

#[derive(Serialize)]
pub struct Receipt {
    pub run_id: String,
    pub backend: String,
    pub requests_sent: u32,
    pub quoted_micro_usd: MicroUsd,
    pub spent_micro_usd: MicroUsd,
    pub budget_micro_usd: MicroUsd,
    pub skipped: Vec<Skipped>,        // no_enrich, over_budget, ...
    pub enrich_log_ids: Vec<i64>,
}
```

Rules every playbook follows:

1. `plan` does not send contact data anywhere. It may call `EgressGate::quote`, which filters and prices jobs by op name and never sends contact data.
2. When `allow_egress == false`, `run` runs all local steps and returns `status: "quote"`. The result holds whatever the local steps produced, and the quote shows what the egress steps would add. For example, `path` with `allow_egress: false` still returns in-graph paths. A caller who only wants the plan without running local steps uses `--plan` / `"plan_only": true`.
3. Egress steps go through `EgressGate` ([enrichment.md](enrichment.md#4-the-egress-gate)).
4. Results are deterministic for a fixed graph, fixed `now` and fixed enricher responses. Tests depend on this.
5. Output always carries evidence (interaction ids, subjects, dates) for any claim about a relationship.

---

## 2. `path` - path to a stranger

Find the best warm route to a person, whether or not they are in the graph.

**Input**

```json
{
  "target": "Priya Raman",          // name, email, linkedin url, or person id
  "company": "Stripe",              // optional, disambiguates and seeds egress
  "linkedin": null,                 // optional
  "find_email": false,              // add find + verify steps
  "limit": 3,
  "allow_egress": false,
  "max_usd": null
}
```

**Steps**

| # | Step | Egress |
| --- | --- | --- |
| 1 | Resolve the target in the graph by identity (email, LinkedIn slug), then by name+company. If found, compute in-graph paths with `how_connected` and **skip step 2**. | - |
| 2 | If the target is missing or the profile is thin (no company), enrich the profile: current employer, past employers, schools. | 1 `profile` |
| 3 | Intersect: people in the graph whose current company equals the target's company (affinity 1.0), whose current or past company (from LinkedIn export history) equals a past employer of the target (0.6), or whose bio or title mentions the target's school (0.4). Exclude `is_bulk`. | - |
| 4 | Rank mutuals by `strength = my_score(m) * w(m, target)`, where `w` is the tie weight if the target is in the graph, else the affinity from step 3. Keep the top `limit`, each with evidence. | - |
| 5 | Optional (`find_email: true`): find the target's email at the company domain and verify it. Skipped if a verified email identity already exists. | 1 `find_email`, 1 `verify_email` |

**Output** (`result`; `Path` items use the shared shape in [interfaces.md](interfaces.md#17-path-and-howconnected))

```json
{
  "target": { "person_id": null, "display_name": "Priya Raman", "company": "Stripe",
              "in_graph": false, "profile_source": "enrich:treg" },
  "paths": [
    {
      "via": { "id": 812, "display_name": "Tom Okafor", "company": "Stripe", "score": 0.78 },
      "strength": 0.78,
      "reason": "same_company",
      "my_score": 0.78,
      "link_weight": 1.0,
      "evidence": [
        { "kind": "interaction", "interaction_id": 99120, "type": "meeting", "direction": "mutual",
          "at": "2026-09-14T16:02:00Z", "subject": "devtools dinner", "role": "attendee",
          "source": { "id": 4, "kind": "ics", "label": "work calendar" } },
        { "kind": "field", "field": "company", "value": "Stripe", "matched": "target.company" }
      ]
    }
  ],
  "email": { "address": "priya@stripe.com", "status": "deliverable", "source": "enrich:treg" }
}
```

**CLI**

```console
$ openstance path "Priya Raman" --company Stripe --find-email
Plan for path (run 01JC4Z...)
  1 lookup target in graph ............ not found
  2 enrich profile .................... profile x1      $0.0100
  3 intersect employers/school ........ local
  4 rank mutuals ...................... local
  5 find + verify email ............... find x1, verify x1  $0.0150
  Total $0.0250 (budget $0.50) via treg. Sends: name, company, domain, email.
Send 3 requests for $0.0250? [y/N] y

Priya Raman - Staff Engineer, Stripe (enrich:treg)
  1. Tom Okafor (Stripe)        strength 0.78  met 2026-09-14 "devtools dinner"
  2. Ana Lee (ex-Stripe 2024)   strength 0.41  12 emails, last 2026-06-02 "Re: infra hiring"
  3. Sam Ito (Stanford)         strength 0.22  last 2025-11-20 "Re: alumni panel"
  email: priya@stripe.com (deliverable)
Spent $0.0250 of $0.50. Log: openstance enrich log --run 01JC4Z...
```

The prices above are illustrative. Real numbers come from the backend's quote.

---

## 3. `accounts` - account mapping

**Input:** `{ "companies": ["stripe.com", "Anthropic", "Ramp"], "one_hop": true, "limit_per_company": 10 }`. The CLI reads a file with one company name or domain per line.

**Steps** (egress 0)

1. **Current:** people whose `company` matches (normalized name, or a company domain from email identities).
2. **Alumni:** people whose company appeared in an **older** `linkedin_csv` source but differs in the latest one. Every LinkedIn export is kept as its own source, so this falls out of history.
3. **One hop:** people with a tie to someone in 1 or 2 (`tie.n_shared >= 2`), ranked by `my_score(m) * tie.weight`.
4. **Coverage table:** one row per company with counts and the best contact.

**Output**

```json
{
  "companies": [
    { "company": "Stripe", "domain": "stripe.com",
      "current": [{ "id": 812, "display_name": "Tom Okafor", "title": "EM", "score": 0.78 }],
      "alumni":  [{ "id": 77, "display_name": "Ana Lee", "left_seen": "2024-08", "score": 0.61 }],
      "one_hop": [{ "id": 1203, "display_name": "Kai Wu", "via": 812, "strength": 0.33 }],
      "coverage": "warm", "best": 812 }
  ]
}
```

`coverage` is `warm` if any current contact has a score of at least `0.6`, `known` if anyone is found, else `none`.

Implementation notes (P4): a company may be a name or a domain; a domain matches the company whose key is its first label (or the bundled domain map). A name target learns its domains from the work addresses of the current people it finds (freemail never counts). Current also includes people with no company and a work address at such a domain. Each alumnus carries `seen_in`: `linkedin_csv` (an older `Connections.csv` export; `left_seen` is the `YYYY-MM` of the first later export that shows them elsewhere), `enrich` (a committed profile's past employers) or `email_domain` (an old work address while their company is now something else). Each row also has `counts` (`current`, `alumni`, `one_hop`, before `limit_per_company`) and `best_score`. The CLI prints the table, or CSV with `--csv` (`company,domain,current,alumni,one_hop,coverage,best_id,best_contact,best_score`).

```console
$ openstance accounts targets.txt
company     current  alumni  one-hop  coverage  best contact
Stripe            3       1        5  warm      Tom Okafor (0.78)
Anthropic         0       2        1  known     Ana Lee (0.61, alumni)
Ramp              0       0        0  none      -
```

---

## 4. `refresh` - fill stale rows

**Input:** `{ "min_interactions": 5, "limit": 50, "allow_egress": false, "max_usd": null }`

**Steps**

1. Select people with an empty `title` **or** `company`, at least `min_interactions` interactions, `is_bulk = 0`, `no_enrich = 0`, and no override on those fields. Order by score, descending.
2. Quote one `profile` request each.
3. Enrich in score order until the budget runs out.
4. Commit under `source.kind = 'enrich:<backend>'`. Overrides win ([enrichment.md](enrichment.md#7-writing-results-without-clobbering-edits)).

**Egress max:** `limit` profiles, capped by budget.

**Output:** `{ "updated": [{ "id": 31, "fields": { "title": "Head of Data", "company": "Ramp" }, "enrich_log_id": 9 }], "unchanged": [], "not_found": [44], "failed": [], "skipped": [...] }`. `fields` lists only the fields that were empty and the profile filled; `unchanged` are answers that filled nothing; `skipped` is the receipt's list (`over_budget`, `no_enrich`).

```console
$ openstance refresh --limit 40
40 people have no title/company and 5+ interactions (3 skipped: no_enrich).
Quote: 40 x profile = $0.40 via treg (budget $0.50). Sends: name, company, linkedin slug.
Send 40 requests for $0.40? [y/N]
```

---

## 5. `watch` - job-change watch

**Input:** `{ "targets": ["Anthropic", "OpenAI"], "check_profiles": false, "top_n": 100, "allow_egress": false, "max_usd": null }`

**Steps**

1. Diff the latest two `linkedin_csv` sources. A person whose company or title changed is flagged `move` with `from`, `to` and `seen_between`. Egress 0.
2. If `check_profiles`: for the top `top_n` by score (not `no_enrich`, and not already moved in step 1), quote and run a `profile` check.
3. Flag company changes. Highlight moves **into** a company on `targets` (from `--targets` or `playbooks.watch.targets`).

**Egress max:** `top_n` profiles (default 100), capped by budget. With the default `$0.50` budget, that is far fewer than 100 at typical prices. The quote shows exactly how many fit.

**Output:** `{ "moves": [{ "id": 77, "display_name": "Ana Lee", "from": "Stripe", "to": "Anthropic", "detected_by": "linkedin_diff", "target_hit": true }], "checked": 0 }`

The full result also has `title_changes` (same company, new title), `new_connections` and `removed_connections` (in only one of the two exports), `targets` and `sources` (`previous`, `latest`: the two exports compared). Each move also carries `from_title`, `to_title`, `seen_between`, `score`, `evidence` (both export rows) or `enrich_log_id` (profile checks). Only `Connections.csv` sources count as exports (`messages.csv` and `Invitations.csv` share the `linkedin_csv` kind); they are ordered by import time. With fewer than two exports the diff is empty and a note says so.

"Weekly" is up to the user. The OSS build has no scheduler, so use cron:

```cron
# every Monday 08:00, diff-only (free), JSON to a file
0 8 * * 1  openstance watch --json > ~/.openstance/reports/watch-$(date +\%F).json
```

On macOS, a launchd agent does the same (`~/Library/LaunchAgents/dev.openstance.watch.plist`, then `launchctl load` it):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.openstance.watch</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/sh</string><string>-c</string>
    <string>openstance watch --json &gt; "$HOME/.openstance/reports/watch-$(date +%F).json"</string>
  </array>
  <key>StartCalendarInterval</key>
  <dict><key>Weekday</key><integer>1</integer><key>Hour</key><integer>8</integer><key>Minute</key><integer>0</integer></dict>
</dict>
</plist>
```

---

## 6. `brief` - intro brief

**Input:** `{ "person": "812", "web": false, "allow_egress": false, "max_usd": null }`

**Steps**

1. Last 5 interactions (subject, date, direction, channel).
2. Mutuals ranked by `my_score(m) * tie.weight`, top 3.
3. Optional (`web: true`): one `web_search` for recent posts (`"<name>" "<company>"`). Skipped for `no_enrich` people.
4. Render a markdown brief of at most 10 lines. The format is deterministic. No model is involved unless `model.reranker` is an API model *and* `playbooks.brief.prose = true`.

**Egress max:** 1 `web_search`.

**Output:** `{ "markdown": "...", "person": {Person}, "interactions": [...], "mutuals": [...], "web": [] }`

```console
$ openstance brief "Tom Okafor"
**Tom Okafor** - Engineering Manager, Stripe (score 0.78: R .87 F .68 C .76 M .74 B .75)
- Last contact 2026-09-14 (meeting "devtools dinner"); 23 interactions since 2024-02.
- Last thread: "Re: infra hiring" 2026-08-30, he replied within a day.
- Met 4 times (calendar), most recent 2026-09-14.
- Mutuals: Ana Lee (0.61, 9 shared), Kai Wu (0.44, 3 shared).
- Channels: email, calendar, linkedin.
- Tags: devtools, investor-intro. Note: "prefers short asks".
```

---

## 7. `reconnect` - reconnect digest

**Input:** `{ "limit": 10, "min_frequency": 0.5, "max_recency": 0.2 }` (egress 0)

**Steps**

1. People with `F > min_frequency` and `R < max_recency`, `is_bulk = 0`.
2. Sort by **past score**: the score evaluated with `R = 1`, which approximates how strong the relationship was at its last contact.
3. Top `limit`, each with the last subject and date.

**Output:** `{ "people": [{ "id": 5, "display_name": "...", "title": null, "company": null, "past_score": 0.74, "score": 0.41, "last_contact": "2025-10-02T09:00:00Z", "last_subject": "Re: seed round notes", "last_interaction_id": 1203 }] }`. The components are the stored ones (the numbers `who` and search show); the last subject is the latest past interaction of any kind except `contact`.

```console
$ openstance reconnect
 #  person          was   now   last contact  last subject
 1  Maya Chen       0.74  0.41  2025-10-02    Re: seed round notes
 2  Luis Ortega     0.70  0.38  2025-08-19    lunch?
```

---

## 8. `verify` - verify before send

**Input:** `{ "person": "priya@stripe.com", "allow_egress": false, "max_usd": null }`

**Steps**

1. Pick the best email from identities: highest confidence, then most recent interaction, then not `enrich:`-sourced.
2. Verify it (1 `verify_email`). Skipped if it was verified within `playbooks.verify.fresh_days` (30).
3. Attach the freshest mutual who **replied**: the mutual with `C > 0` and the most recent `tie.last_at`.

**Egress max:** 1 `verify_email`. If the person has no email identity, the result is `status: "nothing_to_do"` with the note `"no email in graph; try path --find-email"`.

**Output:** `{ "person_id": 31, "email": "priya@stripe.com", "status": "deliverable", "verified_at": "...", "source": "enrich:treg", "enrich_log_id": 12, "mutual": { "id": 812, "display_name": "Tom Okafor", "last_reply": "2026-08-30T10:02:00Z", "last_subject": "Re: infra hiring", "interaction_id": 4410 } }`. `status` is `deliverable | undeliverable | risky | unknown`, or `unverified` without egress. The verdict is committed under `enrich:<backend>`; a committed verdict within `fresh_days` is reused (step 2 `skip_local_sufficient`, no request). `last_reply` is the mutual's latest inbound message, or the tie's last shared interaction when there is none.

```console
$ openstance verify "Priya Raman" --yes
priya@stripe.com  deliverable (treg, $0.0050)
freshest mutual who replied: Tom Okafor, 2026-08-30 "Re: infra hiring"
```

---

## 9. Egress maximums at a glance

| Playbook | profile | find_email | verify_email | web_search |
| --- | --- | --- | --- | --- |
| path | 1 | 1 | 1 | 0 |
| accounts | 0 | 0 | 0 | 0 |
| refresh | `limit` | 0 | 0 | 0 |
| watch | `top_n` | 0 | 0 | 0 |
| brief | 0 | 0 | 0 | 1 |
| reconnect | 0 | 0 | 0 | 0 |
| verify | 0 | 0 | 1 | 0 |

`refresh` and `watch` are also capped by the run budget, and every egress column is skipped entirely when `allow_egress` is false or `enrich.backend = "none"`.

---

## 10. Agent skill files (`playbooks/`)

Each playbook also ships as a markdown skill in the repo's top-level `playbooks/` directory. An agent that reads skills (Claude Code, Cursor rules, OpenClaw-style agents) then knows when to call which MCP tool. These files are **openstance's own text**. They contain no third-party promotion, no "star this repo", and no instructions to use a particular enrichment vendor. treg's skill file is **not** vendored.

```
playbooks/
  openstance.md     # umbrella skill: what the graph is, the 8 tools, egress etiquette
  path.md           # -> path_to, how_connected
  accounts.md       # -> search_people(company=...) + how_connected
  reconnect.md      # -> search_people(lost_touch=true)
  brief.md          # -> brief
  refresh.md        # -> CLI only: tells the agent to suggest `openstance refresh`
  watch.md          # -> CLI only
  verify.md         # -> CLI only (or HTTP POST /playbooks/verify)
```

Every skill file uses this frontmatter and these sections:

```markdown
---
name: openstance-path
description: Find a warm introduction path to a specific person using the user's local openstance graph. Use when the user asks "how can I get to X", "who can intro me to X", or "do I know anyone at <company> who knows X".
tools: [how_connected, path_to]
egress: optional (profile, find_email, verify_email)
---

## When to use
- The user names one target person (optionally with a company).
- Not for "who do I know at <company>" - use search_people with a company filter.

## Steps
1. Call `how_connected` with the target. If it returns paths, present the top 3 with their evidence lines and stop.
2. Otherwise call `path_to` with `allow_egress: false`. Show the user the quote: total cost, and what will be sent (name, company).
3. Only if the user agrees, call `path_to` again with `allow_egress: true` and the same `max_usd`.
4. Present paths as: mutual, strength, the dated evidence line. Never invent a relationship that has no evidence line.

## Rules
- Never set `allow_egress: true` without the user's explicit consent in this conversation.
- Never retry a failed paid call more than once.
- Do not paste email addresses into other tools unless the user asks.
- If the result says a person is `no_enrich` or skipped, tell the user; do not work around it.
```

CI lints every file in `playbooks/`. It checks the frontmatter, checks that the tool names match the registered MCP tools, and rejects URLs other than the openstance repo and docs.

---

## 11. Acceptance criteria

- [ ] All seven playbooks implement `Playbook`. CLI, MCP (`path_to`, `brief`) and HTTP (`POST /playbooks/{name}`) produce byte-identical `result` JSON for the same input and `now`.
- [x] `plan()` for every playbook sends nothing. A test runs it with a `MockEnricher` and asserts that `calls` is empty.
- [x] With `enrich.backend = "none"`, every playbook completes. Egress steps show `skip_no_enricher`, and the notes say why.
- [ ] `path` against a target in the graph makes zero egress calls, even with `allow_egress: true`.
- [x] `accounts`, `reconnect` and `watch` (without `--check-profiles`) make zero egress calls in all configurations.
- [x] Golden tests: fixed fixture graph plus `now = 2026-10-20T00:00:00Z` gives a fixed `reconnect` top 10 and a fixed `brief` markdown.
- [ ] The `playbooks/*.md` lint passes in CI.
