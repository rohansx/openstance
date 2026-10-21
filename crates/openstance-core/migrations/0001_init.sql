-- ============================================================
-- openstance schema v1
-- Times are unix seconds (INTEGER, UTC). Booleans are INTEGER 0/1.
-- Money is integer micro-USD. JSON columns are TEXT (UTF-8 JSON).
-- Authoritative copy: docs/tech-spec/data-model.md section 3.
-- person_vec ships separately as 0002_vectors.sql in Phase 2.
-- ============================================================

-- ------------------------------------------------------------
-- Housekeeping: schema version, migration history, index fingerprints.
-- ------------------------------------------------------------
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;
-- keys written by the migrator and passes:
--   schema_version            '1'
--   migration.0001_init       '<applied_at unix seconds>:<blake3 of sql>'
--   created_by                'openstance 0.2.0'
--   embedder_id               'fastembed:bge-small-en-v1.5'
--   embedder_dims             '384'
--   embedder_state            'ready' | 'building'   (see models.md)
--   scored_at                 '<unix seconds>'       (see scoring.md)
--   score_config_hash         '<blake3 of effective [score] config>'
--   normalizer_version        '1'   (see entity-resolution.md)
--   last_rebuild_at           '<unix seconds>'
--   self_identity_ids         '[12, 873]'  (derived cache, see section 4.3)

-- ------------------------------------------------------------
-- RAW: where data came from. One row per import run or enrichment run.
-- ------------------------------------------------------------
CREATE TABLE source (
  id               INTEGER PRIMARY KEY,
  kind             TEXT    NOT NULL,     -- mbox | imap | linkedin_csv | x_archive | vcf | ics
                                         -- | whatsapp | csv | enrich:<backend>
  label            TEXT,                 -- 'work gmail', 'linkedin 2026-10'
  path             TEXT,                 -- path as given on the command line (display only)
  imported_at      INTEGER NOT NULL,
  file_hash        TEXT,                 -- 'blake3:<hex>'; NULL for imap and enrich:*
  importer_version INTEGER NOT NULL DEFAULT 1,
  stats_json       TEXT                  -- ImportStats, see ingestion.md
);
CREATE UNIQUE INDEX source_file_hash ON source(file_hash) WHERE file_hash IS NOT NULL;

-- ------------------------------------------------------------
-- RAW (+ derived columns): every handle that points at a human.
-- A pure dictionary of normalized handles; one row per (kind, value) ever seen.
-- ------------------------------------------------------------
CREATE TABLE identity (
  id          INTEGER PRIMARY KEY,
  kind        TEXT    NOT NULL CHECK (kind IN
                ('email','phone','linkedin_url','x_handle','x_user_id',
                 'ig_handle','name_company','name_local')),
  value       TEXT    NOT NULL,           -- normalized, see entity-resolution.md section 2
  source_id   INTEGER NOT NULL REFERENCES source(id),  -- first source that produced it
  -- derived (written only by resolution):
  person_id   INTEGER REFERENCES person(id) DEFERRABLE INITIALLY DEFERRED,
  confidence  REAL,                       -- confidence of the rule that attached it; 1.0 for the anchor
  match_rule  TEXT,                       -- anchor | exact | gmail_alias | name_company
                                          -- | name_thread | override | enrich
  UNIQUE (kind, value)
);
CREATE INDEX identity_person ON identity(person_id);

-- ------------------------------------------------------------
-- RAW (+ one derived column): an event between me and one or more people.
-- ------------------------------------------------------------
CREATE TABLE interaction (
  id          INTEGER PRIMARY KEY,
  source_id   INTEGER NOT NULL REFERENCES source(id),
  kind        TEXT    NOT NULL CHECK (kind IN
                ('email','meeting','dm','mention','follow','connection','contact')),
  at          INTEGER NOT NULL,           -- event time; for 'contact' the card REV or import time
  subject     TEXT,                       -- email subject, meeting title; never a body
  external_id TEXT    NOT NULL,           -- Message-ID, event UID[/RECURRENCE-ID], or a
                                          -- synthesized stable hash (ingestion.md section 5)
  thread_key  TEXT,                       -- Gmail X-GM-THRID, else root of References, else Message-ID;
                                          -- LinkedIn CONVERSATION ID; X conversationId
  dir_hint    TEXT CHECK (dir_hint IN ('in','out','mutual','none')),  -- importer's own knowledge
  list_unsub  INTEGER NOT NULL DEFAULT 0, -- 1 if List-Unsubscribe or Precedence: bulk|list|junk
  -- derived:
  direction   TEXT CHECK (direction IN ('in','out','mutual','none')),
  UNIQUE (source_id, external_id)
);
CREATE INDEX interaction_at        ON interaction(at);
CREATE INDEX interaction_global_id ON interaction(kind, external_id);   -- cross-source dedupe
CREATE INDEX interaction_thread    ON interaction(thread_key) WHERE thread_key IS NOT NULL;

-- ------------------------------------------------------------
-- RAW (+ one derived column): who took part, by handle.
-- ------------------------------------------------------------
CREATE TABLE interaction_person (
  interaction_id INTEGER NOT NULL REFERENCES interaction(id) ON DELETE CASCADE,
  identity_id    INTEGER NOT NULL REFERENCES identity(id),
  role           TEXT    NOT NULL CHECK (role IN
                   ('from','to','cc','bcc','attendee','organizer','sender','recipient',
                    'member','subject')),   -- 'subject' = the person a contact/connection/follow is about
  slot           INTEGER NOT NULL DEFAULT 0, -- same slot in one interaction = same human (source-asserted)
  name_hint      TEXT,                       -- display name as shown in this record
  title_hint     TEXT,
  company_hint   TEXT,
  location_hint  TEXT,
  -- derived:
  person_id      INTEGER REFERENCES person(id) DEFERRABLE INITIALLY DEFERRED,
  PRIMARY KEY (interaction_id, identity_id, role)
) WITHOUT ROWID;
CREATE INDEX interaction_person_person   ON interaction_person(person_id, interaction_id);
CREATE INDEX interaction_person_identity ON interaction_person(identity_id);

-- ------------------------------------------------------------
-- DERIVED: one row per human after resolution. id = anchor identity id.
-- ------------------------------------------------------------
CREATE TABLE person (
  id             INTEGER PRIMARY KEY,       -- = MIN(identity.id) of the cluster, never AUTOINCREMENT
  display_name   TEXT,
  title          TEXT,
  company        TEXT,
  location       TEXT,
  bio            TEXT,
  first_seen     INTEGER,                   -- earliest interaction.at (contact rows excluded)
  last_contact   INTEGER,                   -- latest interaction.at with direction in/out/mutual
  n_interactions INTEGER NOT NULL DEFAULT 0,
  score          REAL    NOT NULL DEFAULT 0, -- weighted sum, see scoring.md
  score_r        REAL    NOT NULL DEFAULT 0, -- recency      [0,1]
  score_f        REAL    NOT NULL DEFAULT 0, -- frequency    [0,1]
  score_c        REAL    NOT NULL DEFAULT 0, -- reciprocity  [0,1]
  score_m        REAL    NOT NULL DEFAULT 0, -- meetings     [0,1]
  score_b        REAL    NOT NULL DEFAULT 0, -- breadth      [0,1]
  is_bulk        INTEGER NOT NULL DEFAULT 0, -- newsletter / bulk sender: score 0, hidden from search
  is_self        INTEGER NOT NULL DEFAULT 0, -- the user; never in results, never in ties
  do_not_share   INTEGER NOT NULL DEFAULT 0, -- materialized from override
  no_enrich      INTEGER NOT NULL DEFAULT 0  -- materialized from override
);
CREATE INDEX person_score        ON person(score DESC);
CREATE INDEX person_last_contact ON person(last_contact DESC);
CREATE INDEX person_company      ON person(company COLLATE NOCASE);

-- ------------------------------------------------------------
-- DERIVED: person <-> person edges from shared interactions.
-- Evidence = interactions where both a and b appear in interaction_person.
-- ------------------------------------------------------------
CREATE TABLE tie (
  a        INTEGER NOT NULL REFERENCES person(id) DEFERRABLE INITIALLY DEFERRED,
  b        INTEGER NOT NULL REFERENCES person(id) DEFERRABLE INITIALLY DEFERRED,
  kind     TEXT    NOT NULL CHECK (kind IN ('email','meeting','dm')),
  weight   REAL    NOT NULL,   -- [0,1]: 1 - exp(-D / tie_scale), D = decay-weighted shared count; see scoring.md
  last_at  INTEGER NOT NULL,   -- most recent shared interaction
  n_shared INTEGER NOT NULL,   -- count of shared interactions of this kind
  PRIMARY KEY (a, b, kind),
  CHECK (a < b)
) WITHOUT ROWID;
CREATE INDEX tie_b ON tie(b);  -- tie(a) is served by the primary key prefix

-- ------------------------------------------------------------
-- EDIT: user corrections that survive rebuilds.
-- person_id is an ANCHOR IDENTITY ID (see section 5), deliberately not a FK to person.
-- ------------------------------------------------------------
CREATE TABLE override (
  id        INTEGER PRIMARY KEY,
  person_id INTEGER NOT NULL,
  field     TEXT    NOT NULL CHECK (field IN
              ('display_name','title','company','location','bio','do_not_share','no_enrich',
               'is_bulk',                                       -- scalars
               'tag','note','merged_into','split_from')),        -- multi-valued
  value     TEXT,          -- NULL = tombstone (scalars only)
  set_at    INTEGER NOT NULL
);
CREATE INDEX override_person ON override(person_id, field, set_at);
CREATE INDEX override_field  ON override(field);
CREATE UNIQUE INDEX override_multi_unique ON override(person_id, field, value)
  WHERE field IN ('tag','merged_into','split_from');

-- ------------------------------------------------------------
-- EDIT: saved queries, the unit of sharing. See sharing-and-views.md.
-- ------------------------------------------------------------
CREATE TABLE view (
  id               INTEGER PRIMARY KEY,
  name             TEXT    NOT NULL UNIQUE,
  view_id          TEXT    UNIQUE,     -- ULID, assigned on first export; meta.view_id in the .ost file
  query_json       TEXT    NOT NULL,   -- serialized Query, see search.md
  created_at       INTEGER NOT NULL,
  expires_at       INTEGER,            -- NULL = no TTL
  recipient_pubkey TEXT,               -- age X25519 recipients ('age1...'), newline-separated
  origin           TEXT    NOT NULL DEFAULT 'local' CHECK (origin IN ('local','imported')),
  owner_pubkey     TEXT,               -- for imported views
  file_path        TEXT,               -- for imported views: decrypted copy under ~/.openstance/views/
  last_exported_at INTEGER
);

-- ------------------------------------------------------------
-- RAW: audit of every request that left the machine. Written by the egress gate.
-- Column semantics are owned by enrichment.md section 6.
-- person_id is an ANCHOR IDENTITY ID, deliberately not a FK to person.
-- ------------------------------------------------------------
CREATE TABLE enrich_log (
  id               INTEGER PRIMARY KEY,
  run_id           TEXT    NOT NULL,   -- ULID, one per playbook invocation
  at               INTEGER NOT NULL,
  backend          TEXT    NOT NULL,   -- 'treg' | 'ownkeys' | 'mock' | 'model:<provider>'
  provider         TEXT,
  playbook         TEXT,               -- 'path' | 'refresh' | ... | 'quote_enrich'
  person_id        INTEGER,            -- NULL for company / web search
  op               TEXT    NOT NULL,   -- 'profile' | 'find_email' | 'verify_email' | 'company' | 'web_search'
                                       -- | 'parse' | 'rerank' | 'embed' (hosted model calls)
  request_json     TEXT    NOT NULL,   -- exactly the outbound fields
  quoted_micro_usd INTEGER NOT NULL,
  cost_micro_usd   INTEGER,
  status           TEXT    NOT NULL CHECK (status IN ('ok','not_found','error')),
  error            TEXT,
  source_id        INTEGER REFERENCES source(id),
  response_json    TEXT                -- normalized EnrichData; replayed by rebuild
);
CREATE INDEX enrich_log_run    ON enrich_log(run_id);
CREATE INDEX enrich_log_person ON enrich_log(person_id);
CREATE INDEX enrich_log_at     ON enrich_log(at);

-- ------------------------------------------------------------
-- DERIVED: search indexes.
-- Regular (content-storing) FTS5, rowid = person.id. Updated by delete+insert
-- whenever a person row changes; fully rebuilt by `openstance rebuild`.
-- ------------------------------------------------------------
CREATE VIRTUAL TABLE person_fts USING fts5(
  display_name, title, company, location, bio,
  tags,        -- space-joined override tags
  handles,     -- emails, linkedin slugs, x handles (so 'stripe.com' and 'jdoe' match)
  subjects,    -- space-joined bag of interaction subjects, newest first, capped (section 4.10)
  tokenize = 'unicode61 remove_diacritics 2',
  prefix = '2 3'
);

INSERT INTO meta(key, value) VALUES ('schema_version', '1'), ('normalizer_version', '1');
