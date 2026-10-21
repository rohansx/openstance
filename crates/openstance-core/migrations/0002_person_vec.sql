-- ============================================================
-- openstance schema v2: vectors (Phase 2, P2-CORE-01 / P2-CORE-02).
-- Authoritative description: docs/tech-spec/models.md section 6 and
-- data-model.md section 4.10. Requires sqlite-vec, which core registers on
-- every connection (db::vec::register).
-- ============================================================

-- One vector per non-bulk, non-self person. The dimension is fixed at
-- creation: 384 (the default fastembed bge-small-en-v1.5). An embedder with
-- other dims recreates this table, but only through
-- `openstance rebuild --embeddings` (core::embed).
CREATE VIRTUAL TABLE person_vec USING vec0(
  person_id INTEGER PRIMARY KEY,
  embedding float[384]
);

-- Input hash per embedded person: blake3(embedder_id + "\n" + embed_text).
-- People whose hash is unchanged are not re-embedded.
CREATE TABLE person_embed (
  person_id INTEGER PRIMARY KEY,
  text_hash TEXT    NOT NULL,
  at        INTEGER NOT NULL
);

-- person_vec_dims: the dimension person_vec was created with.
-- embedder_id / embedder_dims / embedder_state are written by the first
-- embedding pass (absent = never indexed).
INSERT INTO meta(key, value) VALUES ('person_vec_dims', '384');
