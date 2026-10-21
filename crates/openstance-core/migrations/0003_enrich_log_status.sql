-- ============================================================
-- openstance schema v3: egress gate statuses (Phase 3, P3-CORE-01).
-- enrich_log now records every egress attempt through its lifecycle:
--   quoted    priced by the gate; nothing left the machine (yet)
--   sent      the request is on the wire (a crash leaves this behind)
--   ok | not_found | error   the backend answered (or failed)
--   blocked   the gate refused to send it (no_enrich, over budget, run aborted)
-- SQLite cannot alter a CHECK constraint, so the table is rebuilt with every
-- existing row kept. Column semantics: docs/tech-spec/enrichment.md section 6.
-- ============================================================

CREATE TABLE enrich_log_v3 (
  id               INTEGER PRIMARY KEY,
  run_id           TEXT    NOT NULL,   -- ULID, one per playbook invocation
  at               INTEGER NOT NULL,   -- quote time, then send time
  backend          TEXT    NOT NULL,   -- 'treg' | 'ownkeys' | 'mock' | 'model:<provider>'
  provider         TEXT,
  playbook         TEXT,               -- 'path' | 'brief' | ... | 'quote_enrich'
  person_id        INTEGER,            -- anchor identity id; NULL for company / web search
  op               TEXT    NOT NULL,   -- 'profile' | 'find_email' | 'verify_email' | 'company' | 'web_search'
                                       -- | 'parse' | 'rerank' | 'embed' (hosted model calls)
  request_json     TEXT    NOT NULL,   -- exactly the outbound fields
  quoted_micro_usd INTEGER NOT NULL,
  cost_micro_usd   INTEGER,            -- actual; NULL until the backend answered
  status           TEXT    NOT NULL CHECK (status IN
                     ('quoted','sent','ok','not_found','error','blocked')),
  error            TEXT,
  source_id        INTEGER REFERENCES source(id),
  response_json    TEXT                -- normalized EnrichData; replayed by rebuild
);
INSERT INTO enrich_log_v3 (id, run_id, at, backend, provider, playbook, person_id, op,
                           request_json, quoted_micro_usd, cost_micro_usd, status, error,
                           source_id, response_json)
  SELECT id, run_id, at, backend, provider, playbook, person_id, op,
         request_json, quoted_micro_usd, cost_micro_usd, status, error,
         source_id, response_json
  FROM enrich_log;
DROP TABLE enrich_log;
ALTER TABLE enrich_log_v3 RENAME TO enrich_log;
CREATE INDEX enrich_log_run    ON enrich_log(run_id);
CREATE INDEX enrich_log_person ON enrich_log(person_id);
CREATE INDEX enrich_log_at     ON enrich_log(at);
