//! Shared seeding helper for the score/bulk/tie integration tests. Rows are
//! inserted directly against the 0001 schema, as resolution would leave
//! them: every person is a single email identity whose id is the person id,
//! `interaction.direction` is already derived and `interaction_person.person_id`
//! is filled. All addresses are fictional (`.example`).
#![allow(dead_code)]

use std::collections::BTreeMap;

use openstance_core::db::{migrate, open_in_memory};
use rusqlite::{params, Connection};

pub const DAY: i64 = 86_400;
/// 2026-10-20T00:00:00Z, the golden clock from testing-strategy.md.
pub const NOW: i64 = 1_792_540_800;

pub struct Seed {
    pub conn: Connection,
    sources: BTreeMap<String, i64>,
    next_ext: i64,
}

impl Default for Seed {
    fn default() -> Self {
        Self::new()
    }
}

impl Seed {
    pub fn new() -> Self {
        let mut conn = open_in_memory().unwrap();
        migrate(&mut conn, None, 0).unwrap();
        Seed {
            conn,
            sources: BTreeMap::new(),
            next_ext: 0,
        }
    }

    /// The source row for a kind (one per kind).
    pub fn source(&mut self, kind: &str) -> i64 {
        if let Some(id) = self.sources.get(kind) {
            return *id;
        }
        self.conn
            .execute(
                "INSERT INTO source(kind, label, imported_at) VALUES (?1, ?1, 0)",
                [kind],
            )
            .unwrap();
        let id = self.conn.last_insert_rowid();
        self.sources.insert(kind.to_string(), id);
        id
    }

    /// A person with one email identity; returns the person id.
    pub fn person(&mut self, email: &str) -> i64 {
        self.person_with(email, false)
    }

    /// The owner.
    pub fn owner(&mut self, email: &str) -> i64 {
        self.person_with(email, true)
    }

    fn person_with(&mut self, email: &str, is_self: bool) -> i64 {
        let src = self.source("mbox");
        self.conn
            .execute(
                "INSERT INTO identity(kind, value, source_id, confidence, match_rule)
                 VALUES ('email', ?1, ?2, 1.0, 'anchor')",
                params![email, src],
            )
            .unwrap();
        let id = self.conn.last_insert_rowid();
        self.conn
            .execute(
                "INSERT INTO person(id, display_name, is_self) VALUES (?1, ?2, ?3)",
                params![id, email, is_self as i64],
            )
            .unwrap();
        self.conn
            .execute("UPDATE identity SET person_id = ?1 WHERE id = ?1", [id])
            .unwrap();
        id
    }

    /// Add another email identity to an existing person.
    pub fn alias(&mut self, person: i64, email: &str) -> i64 {
        let src = self.source("mbox");
        self.conn
            .execute(
                "INSERT INTO identity(kind, value, source_id, person_id, confidence, match_rule)
                 VALUES ('email', ?1, ?2, ?3, 1.0, 'exact')",
                params![email, src, person],
            )
            .unwrap();
        self.conn.last_insert_rowid()
    }

    /// One interaction. `parts` are `(person id, role)`; the identity used is
    /// the person's anchor identity (id = person id).
    #[allow(clippy::too_many_arguments)]
    pub fn interaction(
        &mut self,
        source_kind: &str,
        kind: &str,
        at: i64,
        direction: &str,
        thread_key: Option<&str>,
        list_unsub: bool,
        subject: Option<&str>,
        parts: &[(i64, &str)],
    ) -> i64 {
        let src = self.source(source_kind);
        self.next_ext += 1;
        let ext = format!("ext-{}", self.next_ext);
        self.conn
            .execute(
                "INSERT INTO interaction(source_id, kind, at, subject, external_id, thread_key,
                                         list_unsub, direction)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    src,
                    kind,
                    at,
                    subject,
                    ext,
                    thread_key,
                    list_unsub as i64,
                    direction
                ],
            )
            .unwrap();
        let id = self.conn.last_insert_rowid();
        for (slot, (p, role)) in parts.iter().enumerate() {
            self.conn
                .execute(
                    "INSERT INTO interaction_person(interaction_id, identity_id, role, slot, person_id)
                     VALUES (?1, ?2, ?3, ?4, ?2)",
                    params![id, p, role, slot as i64],
                )
                .unwrap();
        }
        id
    }

    /// Owner -> them email (`out`).
    pub fn email_out(&mut self, owner: i64, them: i64, at: i64, thread: &str) -> i64 {
        self.interaction(
            "mbox",
            "email",
            at,
            "out",
            Some(thread),
            false,
            Some("hello"),
            &[(owner, "from"), (them, "to")],
        )
    }

    /// Them -> owner email (`in`).
    pub fn email_in(&mut self, owner: i64, them: i64, at: i64, thread: &str) -> i64 {
        self.interaction(
            "mbox",
            "email",
            at,
            "in",
            Some(thread),
            false,
            Some("hello"),
            &[(them, "from"), (owner, "to")],
        )
    }

    /// A meeting organized by the owner.
    pub fn meeting(&mut self, owner: i64, attendees: &[i64], at: i64) -> i64 {
        let mut parts = vec![(owner, "organizer")];
        parts.extend(attendees.iter().map(|a| (*a, "attendee")));
        self.interaction(
            "ics",
            "meeting",
            at,
            "out",
            None,
            false,
            Some("sync"),
            &parts,
        )
    }

    pub fn add_override(&mut self, person: i64, field: &str, value: Option<&str>, set_at: i64) {
        self.conn
            .execute(
                "INSERT INTO override(person_id, field, value, set_at) VALUES (?1, ?2, ?3, ?4)",
                params![person, field, value, set_at],
            )
            .unwrap();
    }

    pub fn is_bulk(&self, person: i64) -> bool {
        self.conn
            .query_row("SELECT is_bulk FROM person WHERE id = ?1", [person], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
            == 1
    }

    /// `(score, r, f, c, m, b)` as stored.
    pub fn stored(&self, person: i64) -> [f64; 6] {
        self.conn
            .query_row(
                "SELECT score, score_r, score_f, score_c, score_m, score_b FROM person WHERE id = ?1",
                [person],
                |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?]),
            )
            .unwrap()
    }

    /// Tie rows `(a, b, kind, weight, last_at, n_shared)`, ordered.
    pub fn ties(&self) -> Vec<(i64, i64, String, f64, i64, i64)> {
        let mut st = self
            .conn
            .prepare("SELECT a, b, kind, weight, last_at, n_shared FROM tie ORDER BY a, b, kind")
            .unwrap();
        st.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }

    pub fn meta(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .ok()
    }
}
