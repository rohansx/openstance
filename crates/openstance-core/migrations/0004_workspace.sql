-- ============================================================
-- openstance schema v4: the user's own follow-through workspace (Phase 4).
-- Both tables are user-authored (like `override`): rebuild never touches
-- them. person_id / connector_id store the anchor identity id the user
-- picked; reads resolve it through identity.person_id, so a later merge
-- or split keeps every row pointing at the right person.
-- Spec: docs/tech-spec/data-model.md sections 4.11 and 4.12.
-- ============================================================

-- People the user marked as worth a conversation.
CREATE TABLE shortlist (
  person_id INTEGER PRIMARY KEY,           -- anchor identity id
  note      TEXT,
  added_at  INTEGER NOT NULL
);

-- Introductions the user is tracking by hand. Nothing is ever sent.
CREATE TABLE intro (
  id           INTEGER PRIMARY KEY,
  person_id    INTEGER NOT NULL,           -- who the user wants to meet (anchor identity id)
  connector_id INTEGER,                    -- who could introduce them; NULL = direct
  stage        TEXT    NOT NULL DEFAULT 'draft' CHECK (stage IN
                 ('draft','asked','intro_made','met','closed')),
  note         TEXT,
  follow_up    TEXT,                       -- 'YYYY-MM-DD' (a local calendar date)
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);
CREATE INDEX intro_person ON intro(person_id);
CREATE INDEX intro_stage  ON intro(stage);
