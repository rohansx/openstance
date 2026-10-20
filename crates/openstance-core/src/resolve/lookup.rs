//! Person references: an id (or a former anchor id), an email, a LinkedIn
//! URL or slug, a phone, an X handle, or a name. A name matching several
//! people is `Error::Ambiguous` with candidates.
//! Spec: `docs/tech-spec/entity-resolution.md` section 9 and
//! `interfaces.md` section 4.
//!
//! Owned by the "resolve" stream.

use rusqlite::Connection;

use crate::error::Result;
use crate::PersonId;

/// A parsed person reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonKey {
    /// Person or identity id (`812`).
    Id(i64),
    /// A handle (`jane@example.com`, `in/jane-doe`, `@jdoe`, `+1...`).
    Handle(openstance_import::HandleKind, String),
    /// A display name.
    Name(String),
}

impl PersonKey {
    /// Classify user input: digits -> id, `@`-with-dot -> email, linkedin
    /// URL or `in/` -> slug, leading `@` -> X handle, leading `+` -> phone,
    /// else name.
    pub fn parse(input: &str) -> PersonKey {
        let _ = input;
        todo!("resolve stream: PersonKey::parse")
    }
}

/// A resolved reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    /// Current person id.
    pub person: PersonId,
    /// Set when the given id was an absorbed anchor.
    pub resolved_from: Option<i64>,
}

/// Resolve a reference to a person.
pub fn resolve_person(conn: &Connection, key: &PersonKey) -> Result<Resolved> {
    let _ = (conn, key);
    todo!("resolve stream: resolve_person")
}
