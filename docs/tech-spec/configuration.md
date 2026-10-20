# Configuration

All configuration lives in one TOML file, `~/.openstance/config.toml`. **Every key has a default, so the file can be empty or missing.** `openstance init` writes the file with every key commented out at its default value, so users can see what exists. `openstance config` prints the effective configuration and, for each key, where its value came from.

Related: [enrichment.md](enrichment.md), [scoring.md](scoring.md), [search.md](search.md), [models.md](models.md), [sharing-and-views.md](sharing-and-views.md), [interfaces.md](interfaces.md), [../architecture/security-and-threat-model.md](../architecture/security-and-threat-model.md).

---

## 1. File locations

| Path | Purpose | Created by | Mode |
| --- | --- | --- | --- |
| `~/.openstance/` | data directory (`$OPENSTANCE_HOME`) | `init` | 0700 |
| `~/.openstance/config.toml` | this file | `init` | 0600 |
| `~/.openstance/graph.db` (+ `-wal`, `-shm`) | the graph, one SQLite file | `init` | 0600 |
| `~/.openstance/age.key` | age X25519 identity for views | `init` | 0600 |
| `~/.openstance/api.token` | HTTP API / HTTP MCP bearer token | `init` | 0600 |
| `~/.openstance/models/` | fastembed model cache (about 33 MB for bge-small int8) | first embedding run | 0700 |
| `~/.openstance/views/<view_id>.db` | imported, decrypted views | `view import` | 0600 |
| `~/.openstance/sync/<remote>/` | git working copies for sync (P5) | `sync` | 0700 |
| `~/.openstance/logs/openstance.log` | rotating log file (10 MB x 5) | any command | 0600 |
| `~/.openstance/reports/` | suggested place for cron output (not created automatically) | user | - |

On Windows `~` is `%USERPROFILE%`. openstance does not follow XDG directories in v1: one directory is easier to back up and to explain. Set `OPENSTANCE_HOME` to relocate everything. Paths in the config file may use `~` and `$OPENSTANCE_HOME`. Relative paths resolve against `$OPENSTANCE_HOME`.

---

## 2. Precedence and environment variables

Highest wins:

1. CLI flags (`--db`, `--max-usd`, `--port`, ...)
2. Environment variables
3. `config.toml` (or the file given by `--config` / `OPENSTANCE_CONFIG`)
4. Built-in defaults (this document)

| Variable | Effect |
| --- | --- |
| `OPENSTANCE_HOME` | data directory (default `~/.openstance`) |
| `OPENSTANCE_CONFIG` | config file path |
| `OPENSTANCE_DB` | same as `db.path` |
| `OPENSTANCE_MODEL_DIR` | model cache directory (same as `model.cache_dir`; for air-gapped installs point it at a pre-downloaded copy) |
| `OPENSTANCE_OFFLINE` | `1` forbids model downloads (`HF_HUB_OFFLINE=1` is honored too); search degrades to FTS-only if a model is missing |
| `OPENSTANCE_LOG` | stderr log filter in `tracing` env-filter syntax, e.g. `openstance_core=debug` (overrides `log.level`) |
| `OPENSTANCE__<SECTION>__<KEY>` | any key. Double underscores separate path segments, e.g. `OPENSTANCE__ENRICH__MAX_USD_PER_RUN=0.25`, `OPENSTANCE__SCORE__WEIGHTS__RECENCY=0.4`. Values are parsed as TOML scalars, and arrays as TOML (`'["a","b"]'`) |
| `OPENSTANCE_TREG_TOKEN` | treg token (name configurable via `enrich.treg.token_env`) |
| `OPENAI_API_KEY`, `ANTHROPIC_API_KEY` | API-backed embedder / parser / reranker |
| `PDL_API_KEY`, `APOLLO_API_KEY`, `CLEARBIT_API_KEY` | OwnKeys enricher |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_PROFILE`, `AWS_REGION` | S3 sync (P5) |
| `NO_COLOR` | disable colors |

**Secrets are never read from `config.toml`.** Keys that need a secret take the *name* of an environment variable (`token_env`, `key_env`). `openstance config` prints any value that looks like a secret as `***`, and so does `GET /config`.

---

## 3. Complete reference file

This is the file `openstance init` writes, uncommented, and it shows every default. The per-section tables in section 4 define the allowed values.

```toml
# ~/.openstance/config.toml - every key optional; values shown are defaults.

[db]
path = "~/.openstance/graph.db"
busy_timeout_ms = 5000

[score]
preset = "default"            # default | founder | recruiter | seller | community
# weights = { recency = 0.35, frequency = 0.20, reciprocity = 0.25, meetings = 0.15, breadth = 0.05 }
half_life_days = 180          # R = exp(-ln(2) * days_since_last / half_life_days)
frequency_scale = 20          # F = 1 - exp(-n_interactions / frequency_scale)
meetings_scale = 3            # M = 1 - exp(-n_meetings / meetings_scale)
thread_start_multiplier = 2.0 # C: a thread they started counts double
tie_scale = 3                 # tie weight = 1 - exp(-D / tie_scale)
tie_max_participants = 20     # interactions with more people create no ties

[score.bulk]
min_messages = 50             # 50+ messages from them and zero replies either way -> bulk
list_unsubscribe_ratio = 0.5  # or List-Unsubscribe on more than this share of their messages

[resolution]
suggest_jaro_winkler = 0.92   # name similarity at or above this is suggested in review, never auto-merged
common_names_top_n = 5000     # names in the top-N list never merge on name alone
large_company_threshold = 50  # common name at a company this big in your graph -> suggestion, not merge
company_aliases = {}          # e.g. { "stripe inc" = "stripe" }
freemail_extra = []           # extra freemail domains (never used as a company signal)
role_localparts_extra = []    # extra role addresses (never glue people together)

[search]
default_limit = 30
candidates_fts = 200
candidates_vec = 200
text_weight = 0.5             # fused = text_weight * fts_norm + (1 - text_weight) * vec_norm
relationship_lift = 0.5       # final = fused * ((1 - relationship_lift) + relationship_lift * score)
evidence_per_result = 3
include_bulk = false

[model]
embedder = "fastembed:bge-small-en-v1.5"
parser = "rules"
reranker = "none"
cache_dir = "~/.openstance/models"   # env OPENSTANCE_MODEL_DIR
timeout_local_ms = 5000
timeout_remote_ms = 10000
rerank_top_n = 50

[model.ollama]
base_url = "http://127.0.0.1:11434"  # a non-loopback host counts as remote

[model.openai]
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
json_mode = "schema"          # schema | object | prompt
# dimensions = 512            # optional, embeddings only

[model.anthropic]
api_key_env = "ANTHROPIC_API_KEY"

[enrich]
backend = "none"              # none | treg | ownkeys
max_usd_per_run = 0.50
allow_egress_default = false
respect_do_not_share = true
timeout_secs = 20

[enrich.treg]
endpoint = "https://api.treg.to"
token_env = "OPENSTANCE_TREG_TOKEN"

[enrich.ownkeys]
order = ["pdl", "apollo", "clearbit"]

[enrich.ownkeys.prices]       # micro-USD per call; unset op = quoted as 0 with a warning
# profile = 10000
# find_email = 10000
# verify_email = 5000
# company = 5000

# [enrich.ownkeys.verify]     # optional generic email verifier
# url = "https://verifier.example/v1/check?email={email}"
# key_env = "VERIFIER_API_KEY"
# key_header = "Authorization"

[me]
identities = []               # your own handles, e.g. ["email:rohan@example.com", "linkedin_url:rohan-sharma"];
                              # empty = proposed from the first import (Gmail "Sent" label, LinkedIn Profile.csv)

[import]
email_bodies = false          # headers only; --bodies to opt in per run
max_message_mb = 50           # mbox messages larger than this are skipped (header still read)
default_region = ""           # ISO country for phone numbers without +, e.g. "US", "IN"
timezone = ""                 # for naive timestamps; empty = system local
ics_max_attendees = 50        # larger events are skipped

[share]
keypair = "~/.openstance/age.key"
default_ttl = "30d"
owner_label = ""              # shown to recipients as the claimed owner; empty = $USER
armor = false
max_import_mb = 100
bucket_strong = 0.60
bucket_medium = 0.30

[share.recipients]            # alias = "age1..."
# alice = "age1..."

[serve]
host = "127.0.0.1"
port = 7411
token_path = "~/.openstance/api.token"
ui = true
cors_origins = []
max_upload_mb = 2048

[mcp]
read_only = false
default_limit = 20

[playbooks.path]
limit = 3

[playbooks.refresh]
min_interactions = 5
limit = 50

[playbooks.watch]
targets = []
top_n = 100

[playbooks.reconnect]
limit = 10
min_frequency = 0.5
max_recency = 0.2

[playbooks.brief]
prose = false

[playbooks.verify]
fresh_days = 30

[log]
level = "warn"                # stderr: error | warn | info | debug | trace
file = "~/.openstance/logs/openstance.log"
file_level = "info"
format = "text"               # text | json

# [sync.remotes.work]         # Phase 5
# url = "git+ssh://git@github.com/acme/openstance-views.git"
```

---

## 4. Key reference

### `[db]`

| Key | Type | Default | Allowed | Notes |
| --- | --- | --- | --- | --- |
| `path` | path | `~/.openstance/graph.db` | any writable path | `--db` / `OPENSTANCE_DB` override |
| `busy_timeout_ms` | int | `5000` | 0 to 60000 | SQLite busy timeout. The CLI and `serve` can run at the same time (WAL) |

### `[score]`

| Key | Type | Default | Allowed | Notes |
| --- | --- | --- | --- | --- |
| `preset` | string | `"default"` | `default`, `founder`, `recruiter`, `seller`, `community` | ignored if `weights` is set (shown as preset `custom`) |
| `weights` | table | unset | `recency`, `frequency`, `reciprocity`, `meetings`, `breadth`, each >= 0 | normalized to sum to 1. A warning is printed if the input sum is off by more than 0.01 |
| `half_life_days` | float | `180` | > 0 | recency halves every this many days |
| `frequency_scale` | float | `20` | > 0 | F saturation |
| `meetings_scale` | float | `3` | > 0 | M saturation |
| `thread_start_multiplier` | float | `2.0` | >= 1 | C: a thread they start counts this many times |
| `bulk.min_messages` | int | `50` | >= 1 | bulk rule 1 (with zero replies either way) |
| `bulk.list_unsubscribe_ratio` | float | `0.5` | 0 to 1 | bulk rule 2. Bulk people get `is_bulk = 1` and score 0. A false positive is corrected per person with `openstance edit <p> --not-bulk` (an `is_bulk` override) |
| `tie_scale` | float | `3` | > 0 | tie weight saturation, see [scoring.md](scoring.md#82-weight) |
| `tie_max_participants` | int | `20` | >= 2 | interactions with more participants create no ties |

Presets (weights R / F / C / M / B):

| Preset | R | F | C | M | B |
| --- | --- | --- | --- | --- | --- |
| `default` | 0.35 | 0.20 | 0.25 | 0.15 | 0.05 |
| `founder` | 0.30 | 0.15 | 0.30 | 0.20 | 0.05 |
| `recruiter` | 0.35 | 0.20 | 0.15 | 0.10 | 0.20 |
| `seller` | 0.25 | 0.15 | 0.30 | 0.25 | 0.05 |
| `community` | 0.15 | 0.40 | 0.25 | 0.10 | 0.10 |

The tech spec PDF's example config shows `preset = "founder"` and separately lists default weights of `[0.35, 0.20, 0.25, 0.15, 0.05]`. Those two do not agree. The resolved behavior is a `default` preset with the spec's default weights, and `founder` as defined in the preset table. See [scoring.md](scoring.md).

Changing any `[score]` key changes scores. openstance stores a hash of the effective score config in the database, notices a change on the next command, and re-scores in place. This takes a few seconds at 20k people. It is the same as `openstance rebuild --scores-only`.

### `[resolution]`

| Key | Type | Default | Allowed | Notes |
| --- | --- | --- | --- | --- |
| `suggest_jaro_winkler` | float | `0.92` | 0.80 to 1.0 | suggestion threshold only. Fuzzy names never auto-merge |
| `common_names_top_n` | int | `5000` | 0 to 50000 | size of the bundled common-names list (given names and surnames) used to block name-only merges |
| `large_company_threshold` | int | `50` | >= 2 | a common name at a company with this many people in the graph becomes a 0.6 suggestion, not a merge |
| `company_aliases` | table | `{}` | `"alias" = "canonical"` | company-name normalization additions |
| `freemail_extra` | array | `[]` | domains | added to the bundled freemail list |
| `role_localparts_extra` | array | `[]` | local parts | added to the bundled role-address list |

The confidence values of the merge rules are fixed in code, not configurable. See [entity-resolution.md](entity-resolution.md).

### `[search]`

| Key | Type | Default | Allowed | Notes |
| --- | --- | --- | --- | --- |
| `default_limit` | int | `30` | 1 to 1000 | CLI default `--limit` |
| `candidates_fts` | int | `200` | 10 to 5000 | top-N FTS5 hits before filtering |
| `candidates_vec` | int | `200` | 10 to 5000 | top-N vector hits before filtering |
| `text_weight` | float | `0.5` | 0 to 1 | FTS vs vector share of the fused score |
| `relationship_lift` | float | `0.5` | 0 to 1 | `final = fused * ((1 - lift) + lift * score)`. At 0.5 this is the spec's `0.5 + 0.5 * score` |
| `evidence_per_result` | int | `3` | 0 to 20 | most recent interactions shown per result |
| `include_bulk` | bool | `false` | | bulk senders are excluded from search unless true or `--include-bulk` |

### `[model]`

| Key | Type | Default | Allowed | Notes |
| --- | --- | --- | --- | --- |
| `embedder` | string | `"fastembed:bge-small-en-v1.5"` | `none`, `fastembed:<model>`, `ollama:<model>` (e.g. `nomic-embed-text`), `openai:<model>` (e.g. `text-embedding-3-small`) | the embedder id and dims are recorded in the DB. On a change, vector search is disabled (FTS-only, with a notice) until `openstance rebuild --embeddings` |
| `parser` | string | `"rules"` | `rules`, `ollama:<model>` (e.g. `qwen2.5:7b`), `anthropic:<model-id>`, `openai:<model-id>` | model output is validated against `Query`. On failure it falls back to rules |
| `reranker` | string | `"none"` | `none`, `fastembed:bge-reranker-base`, `ollama:<model>`, `anthropic:<model-id>`, `openai:<model-id>` | API rerankers may also write the one-line `reason` |
| `cache_dir` | path | `~/.openstance/models` | | fastembed downloads; `OPENSTANCE_MODEL_DIR` overrides |
| `timeout_local_ms` | int | `5000` | 100 to 300000 | per call to a local model |
| `timeout_remote_ms` | int | `10000` | 100 to 300000 | per call to a remote model; on timeout the parser falls back to rules |
| `rerank_top_n` | int | `50` | 1 to 200 | candidates sent to the reranker |
| `ollama.base_url` | url | `http://127.0.0.1:11434` | | a non-loopback host counts as remote (egress) |
| `openai.base_url` | url | `https://api.openai.com/v1` | any OpenAI-compatible endpoint | lets users point at local servers (LM Studio, vLLM) |
| `openai.api_key_env` | string | `"OPENAI_API_KEY"` | env var name | |
| `openai.json_mode` | string | `"schema"` | `schema`, `object`, `prompt` | structured-output mode for the parser |
| `openai.dimensions` | int | unset | | embeddings only |
| `anthropic.api_key_env` | string | `"ANTHROPIC_API_KEY"` | env var name | |

Model id strings are written `<backend>:<model>`. Pin exact provider model ids when Phase 4 is implemented. The tech spec's `anthropic:claude-haiku` means "a Haiku-class model" and is not a literal id.

**Privacy note.** Remote model calls are egress, under the same rules as enrichment ([models.md](models.md#9-privacy-and-egress-rules-for-models)). An API-backed `parser` sends the query text. An API-backed `reranker` sends title, company, location, tags and score bucket for at most `rerank_top_n` candidates, never names, emails or subjects. An API-backed `embedder` sends every non-bulk person's embedding text. Therefore:

- the defaults are all local
- setting any remote model prints a one-time notice explaining what it sends, and `openstance config` marks those keys `(remote)`
- every remote call is logged to `enrich_log` with backend `model:<provider>` (op `parse`, `rerank` or `embed`) and its estimated cost
- bulk remote embedding is quoted first and needs confirmation (or `--yes`); per-query parse and rerank calls are consented to by the config itself and are not quoted per call
- `no_enrich` people are never sent to a remote model

See [models.md](models.md) and [../architecture/security-and-threat-model.md](../architecture/security-and-threat-model.md).

### `[enrich]`

| Key | Type | Default | Allowed | Notes |
| --- | --- | --- | --- | --- |
| `backend` | string | `"none"` | `none`, `treg`, `ownkeys` | `none` means no egress is possible |
| `max_usd_per_run` | float | `0.50` | >= 0 | per playbook run. CLI `--max-usd` can raise it for one run. MCP/HTTP `max_usd` can only lower it |
| `allow_egress_default` | bool | `false` | | the value of `allow_egress` when an MCP/HTTP caller omits it. The CLI still prompts unless `--yes` |
| `respect_do_not_share` | bool | `true` | | also skip `do_not_share` people in enrichment. `no_enrich` is always respected and has no switch |
| `timeout_secs` | int | `20` | 1 to 120 | per request |
| `treg.endpoint` | url | `https://api.treg.to` | | point at a self-hosted treg router |
| `treg.token_env` | string | `"OPENSTANCE_TREG_TOKEN"` | env var name | |
| `ownkeys.order` | array | `["pdl","apollo","clearbit"]` | subset of those | preference when several keys can serve an op |
| `ownkeys.prices.<op>` | int | unset | micro-USD | `profile`, `find_email`, `verify_email`, `company`, `web_search` |
| `ownkeys.verify.url` | string | unset | URL with `{email}` | generic verifier |
| `ownkeys.verify.key_env` | string | unset | env var name | |
| `ownkeys.verify.key_header` | string | `"Authorization"` | | sent as `Bearer <key>` when the header is `Authorization`, else as a raw value |

### `[import]`

| Key | Type | Default | Allowed | Notes |
| --- | --- | --- | --- | --- |
| `email_bodies` | bool | `false` | | headers only by default. When on, bodies are read for signature title/company; what is persisted is defined in [ingestion.md](ingestion.md) |
| `max_message_mb` | int | `50` | 1 to 1000 | larger mbox messages are skipped after the header |
| `default_region` | string | `""` | ISO 3166 alpha-2 | phone parsing for numbers without `+` |
| `timezone` | string | `""` (system local) | IANA name | for naive timestamps (some CSV/ics) |
| `ics_max_attendees` | int | `50` | >= 2 | larger calendar events are skipped |
| `csv.<name>` | table | none | column map | named column maps for `import --kind csv --map <name>` (Phase 5 importer) |

### `[me]`

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `identities` | array | `[]` | your own handles as `kind:value` (`email:...`, `linkedin_url:...`, `x_handle:...`). They are must-linked into the self person, which never appears in results or ties and decides `interaction.direction`. If empty, the first import proposes candidates (Gmail `Sent` label, LinkedIn `Profile.csv`) and writes them here after confirmation. `openstance init --me <email>` sets it directly |

Column maps can also be passed per run (`--map`). See [ingestion.md](ingestion.md).

### `[share]`

| Key | Type | Default | Allowed | Notes |
| --- | --- | --- | --- | --- |
| `keypair` | path | `~/.openstance/age.key` | | |
| `default_ttl` | duration | `"30d"` | `Nd`, `Nw`, `Nm`, `Ny`, or `never` | `never` requires `--yes` |
| `owner_label` | string | `""` (uses `$USER`) | | written to `meta.owner_label` (a claim, not proof) |
| `armor` | bool | `false` | | ASCII-armored `.ost` by default |
| `max_import_mb` | int | `100` | 1 to 2048 | decrypted size limit for `view import` |
| `bucket_strong` | float | `0.60` | 0 to 1 | score at or above this is `strong` |
| `bucket_medium` | float | `0.30` | 0 to `bucket_strong` | at or above this is `medium`, else `weak` |
| `recipients.<alias>` | string | none | `age1...` | `--to <alias>` |

### `[serve]`

| Key | Type | Default | Allowed | Notes |
| --- | --- | --- | --- | --- |
| `host` | IP | `127.0.0.1` | any | a non-loopback address also requires the `--i-understand-this-exposes-my-graph` flag at start; config alone is not enough |
| `port` | int | `7411` | 1 to 65535 | |
| `token_path` | path | `~/.openstance/api.token` | | |
| `ui` | bool | `true` | | serve the embedded UI |
| `cors_origins` | array | `[]` | origins | e.g. `["http://localhost:5173"]` for UI development |
| `max_upload_mb` | int | `2048` | 1 to 20000 | `POST /import`, `POST /views/import` |

### `[mcp]`

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `read_only` | bool | `false` | force `allow_egress = false` on every tool, and refuse edits. Same as `openstance mcp --read-only` |
| `default_limit` | int | `20` | `search_people` default limit |

### `[playbooks.*]`

Defaults for playbook inputs. CLI flags and MCP/HTTP input fields override them. See [playbooks.md](playbooks.md).

| Key | Default | Notes |
| --- | --- | --- |
| `path.limit` | `3` | paths returned |
| `refresh.min_interactions` | `5` | |
| `refresh.limit` | `50` | max people per run (also capped by budget) |
| `watch.targets` | `[]` | companies whose incoming moves are highlighted |
| `watch.top_n` | `100` | profile checks with `--check-profiles` |
| `reconnect.limit` | `10` | |
| `reconnect.min_frequency` | `0.5` | F threshold |
| `reconnect.max_recency` | `0.2` | R threshold (about 418 days at half-life 180) |
| `brief.prose` | `false` | allow a configured API reranker model to rewrite the brief as prose |
| `verify.fresh_days` | `30` | skip re-verification within this window |

### `[log]`

| Key | Default | Allowed | Notes |
| --- | --- | --- | --- |
| `level` | `"warn"` | `error`, `warn`, `info`, `debug`, `trace` | stderr. `-v` / `-q` / `OPENSTANCE_LOG` override |
| `file` | `~/.openstance/logs/openstance.log` | path or `""` to disable | rotated 10 MB x 5 |
| `file_level` | `"info"` | as above | |
| `format` | `"text"` | `text`, `json` | file format |

Logs never contain email bodies, tokens or API keys. At `debug` they may contain names and subjects, so the log file is mode 0600.

### `[sync]` (Phase 5)

| Key | Default | Notes |
| --- | --- | --- |
| `remotes.<name>.url` | none | `git+ssh://...`, `git+https://...`, `s3://bucket/prefix?region=..&endpoint=..` |
| `remotes.<name>.default` | `false` | used when `sync` is run without `--remote` |

See [sharing-and-views.md](sharing-and-views.md#8-sync-and-team-federation-phase-5).

---

## 5. Validation

- Unknown keys are a **warning**, not an error, so newer configs still load in older binaries. `openstance config --strict` turns them into errors (useful in CI).
- Type errors and out-of-range values are errors. The message names the key, the value, the allowed range and the source (file line or env var).
- Validation runs at startup of every command. `serve` and `mcp` do not hot-reload. Restart them after editing.

```console
$ openstance config
[score]
preset = "founder"               # file:3
half_life_days = 120             # env OPENSTANCE__SCORE__HALF_LIFE_DAYS
weights (from preset founder)    = R 0.30 F 0.15 C 0.30 M 0.20 B 0.05
[enrich]
backend = "treg"                 # file:9
max_usd_per_run = 0.5            # default
treg.token_env = "OPENSTANCE_TREG_TOKEN" (set: ***)
[model]
parser = "ollama:qwen2.5:7b"     # file:14   (local)
reranker = "none"                # default
```

---

## 6. Acceptance criteria

- [ ] An empty or missing `config.toml` yields exactly the defaults in section 3 (snapshot test of `openstance config --json`).
- [ ] Every key in section 3 can be set through `OPENSTANCE__...` and shows `env` as its origin.
- [ ] No secret value appears in `openstance config`, `GET /config`, logs or `enrich_log` (test with sentinel values).
- [ ] Changing `score.preset` re-scores on the next command, and `who` shows the new weights.
- [ ] Changing `model.embedder` disables vector search with a notice until `rebuild --embeddings`. Searches still return FTS results.
- [ ] `serve.host = "0.0.0.0"` without the explicit flag refuses to start.
