-- index enrich_log_at on enrich_log
CREATE INDEX enrich_log_at     ON enrich_log(at);

-- index enrich_log_person on enrich_log
CREATE INDEX enrich_log_person ON enrich_log(person_id);

-- index enrich_log_run on enrich_log
CREATE INDEX enrich_log_run    ON enrich_log(run_id);

-- index identity_person on identity
CREATE INDEX identity_person ON identity(person_id);

-- index interaction_at on interaction
CREATE INDEX interaction_at        ON interaction(at);

-- index interaction_global_id on interaction
CREATE INDEX interaction_global_id ON interaction(kind, external_id);

-- index interaction_person_identity on interaction_person
CREATE INDEX interaction_person_identity ON interaction_person(identity_id);

-- index interaction_person_person on interaction_person
CREATE INDEX interaction_person_person   ON interaction_person(person_id, interaction_id);

-- index interaction_thread on interaction
CREATE INDEX interaction_thread    ON interaction(thread_key) WHERE thread_key IS NOT NULL;

-- index override_field on override
CREATE INDEX override_field  ON override(field);

-- index override_multi_unique on override
CREATE UNIQUE INDEX override_multi_unique ON override(person_id, field, value)
  WHERE field IN ('tag','merged_into','split_from');

-- index override_person on override
CREATE INDEX override_person ON override(person_id, field, set_at);

-- index person_company on person
CREATE INDEX person_company      ON person(company COLLATE NOCASE);

-- index person_last_contact on person
CREATE INDEX person_last_contact ON person(last_contact DESC);

-- index person_score on person
CREATE INDEX person_score        ON person(score DESC);

-- index source_file_hash on source
CREATE UNIQUE INDEX source_file_hash ON source(file_hash) WHERE file_hash IS NOT NULL;

-- index sqlite_autoindex_identity_1 on identity
-- (auto)

-- index sqlite_autoindex_interaction_1 on interaction
-- (auto)

-- index sqlite_autoindex_person_vec_info_1 on person_vec_info
-- (auto)

-- index sqlite_autoindex_person_vec_vector_chunks00_1 on person_vec_vector_chunks00
-- (auto)

-- index sqlite_autoindex_view_1 on view
-- (auto)

-- index sqlite_autoindex_view_2 on view
-- (auto)

-- index tie_b on tie
CREATE INDEX tie_b ON tie(b);

-- table enrich_log on enrich_log
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

-- table identity on identity
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

-- table interaction on interaction
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

-- table interaction_person on interaction_person
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

-- table meta on meta
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;

-- table override on override
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

-- table person on person
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

-- table person_embed on person_embed
CREATE TABLE person_embed (
  person_id INTEGER PRIMARY KEY,
  text_hash TEXT    NOT NULL,
  at        INTEGER NOT NULL
);

-- table person_fts on person_fts
CREATE VIRTUAL TABLE person_fts USING fts5(
  display_name, title, company, location, bio,
  tags,        -- space-joined override tags
  handles,     -- emails, linkedin slugs, x handles (so 'stripe.com' and 'jdoe' match)
  subjects,    -- space-joined bag of interaction subjects, newest first, capped (section 4.10)
  tokenize = 'unicode61 remove_diacritics 2',
  prefix = '2 3'
);

-- table person_fts_config on person_fts_config
CREATE TABLE 'person_fts_config'(k PRIMARY KEY, v) WITHOUT ROWID;

-- table person_fts_content on person_fts_content
CREATE TABLE 'person_fts_content'(id INTEGER PRIMARY KEY, c0, c1, c2, c3, c4, c5, c6, c7);

-- table person_fts_data on person_fts_data
CREATE TABLE 'person_fts_data'(id INTEGER PRIMARY KEY, block BLOB);

-- table person_fts_docsize on person_fts_docsize
CREATE TABLE 'person_fts_docsize'(id INTEGER PRIMARY KEY, sz BLOB);

-- table person_fts_idx on person_fts_idx
CREATE TABLE 'person_fts_idx'(segid, term, pgno, PRIMARY KEY(segid, term)) WITHOUT ROWID;

-- table person_vec on person_vec
CREATE VIRTUAL TABLE person_vec USING vec0(
  person_id INTEGER PRIMARY KEY,
  embedding float[384]
);

-- table person_vec_chunks on person_vec_chunks
CREATE TABLE "person_vec_chunks"(chunk_id INTEGER PRIMARY KEY AUTOINCREMENT,size INTEGER NOT NULL,validity BLOB NOT NULL,rowids BLOB NOT NULL);

-- table person_vec_info on person_vec_info
CREATE TABLE "person_vec_info" (key text primary key, value any);

-- table person_vec_rowids on person_vec_rowids
CREATE TABLE "person_vec_rowids"(rowid INTEGER PRIMARY KEY AUTOINCREMENT,id,chunk_id INTEGER,chunk_offset INTEGER);

-- table person_vec_vector_chunks00 on person_vec_vector_chunks00
CREATE TABLE "person_vec_vector_chunks00"(rowid PRIMARY KEY,vectors BLOB NOT NULL);

-- table source on source
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

-- table sqlite_sequence on sqlite_sequence
CREATE TABLE sqlite_sequence(name,seq);

-- table tie on tie
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

-- table view on view
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

