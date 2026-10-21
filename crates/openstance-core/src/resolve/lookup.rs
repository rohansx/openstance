//! Person references: an id (or a former anchor id), an email, a LinkedIn
//! URL or slug, a phone, an X handle, or a name. A name matching several
//! people is `Error::Ambiguous` with candidates.
//! Spec: `docs/tech-spec/entity-resolution.md` section 9 and
//! `interfaces.md` section 4.
//!
//! Owned by the "resolve" stream.

use openstance_import::HandleKind;
use rusqlite::{Connection, OptionalExtension};

use super::normalize::{name_key, normalize_handle, NormalizeContext};
use crate::error::{Error, Result};
use crate::PersonId;

/// A parsed person reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonKey {
    /// Person or identity id (`812`).
    Id(i64),
    /// A handle (`jane@example.com`, `in/jane-doe`, `@jdoe`, `+1...`).
    Handle(HandleKind, String),
    /// A display name.
    Name(String),
}

impl PersonKey {
    /// Classify user input: digits -> id, `kind:value` -> that handle,
    /// `@`-with-dot -> email, linkedin URL or `in/` -> slug, leading `@` ->
    /// X handle, leading `+` -> phone, else name.
    pub fn parse(input: &str) -> PersonKey {
        let t = input.trim();
        if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(n) = t.parse::<i64>() {
                return PersonKey::Id(n);
            }
        }
        if let Some((kind, value)) = t.split_once(':') {
            if let Some(k) = HandleKind::parse(kind.trim()) {
                return PersonKey::Handle(k, value.trim().to_string());
            }
        }
        let lower = t.to_ascii_lowercase();
        if lower.contains("linkedin.com/") {
            return PersonKey::Handle(HandleKind::LinkedinUrl, t.to_string());
        }
        if let Some(slug) = lower.strip_prefix("in/") {
            return PersonKey::Handle(
                HandleKind::LinkedinUrl,
                format!("linkedin.com/in/{}", slug.trim_end_matches('/')),
            );
        }
        if lower.starts_with("x.com/") || lower.starts_with("twitter.com/") {
            return PersonKey::Handle(HandleKind::XHandle, t.to_string());
        }
        if let Some(rest) = t.strip_prefix('@') {
            if !rest.is_empty() && !rest.contains('@') && !rest.contains(char::is_whitespace) {
                return PersonKey::Handle(HandleKind::XHandle, rest.to_string());
            }
        }
        if let Some(rest) = t.strip_prefix('+') {
            if !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_digit() || " -.()".contains(c))
            {
                return PersonKey::Handle(HandleKind::Phone, t.to_string());
            }
        }
        if t.contains('@') && !t.contains(char::is_whitespace) {
            return PersonKey::Handle(HandleKind::Email, t.to_string());
        }
        PersonKey::Name(t.to_string())
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
    match key {
        PersonKey::Id(n) => {
            let p: Option<Option<i64>> = conn
                .prepare_cached("SELECT person_id FROM identity WHERE id = ?1")?
                .query_row([n], |r| r.get(0))
                .optional()?;
            match p.flatten() {
                Some(p) => Ok(Resolved {
                    person: p,
                    resolved_from: (p != *n).then_some(*n),
                }),
                None => {
                    let exists: bool = conn
                        .prepare_cached("SELECT EXISTS(SELECT 1 FROM person WHERE id = ?1)")?
                        .query_row([n], |r| r.get(0))?;
                    if exists {
                        Ok(Resolved {
                            person: *n,
                            resolved_from: None,
                        })
                    } else {
                        Err(Error::NotFound(format!("person {n}")))
                    }
                }
            }
        }
        PersonKey::Handle(kind, raw) => {
            let value = normalize_handle(*kind, raw, &NormalizeContext::default())
                .ok_or_else(|| Error::BadRequest(format!("not a valid {kind} reference")))?;
            let p: Option<Option<i64>> = conn
                .prepare_cached("SELECT person_id FROM identity WHERE kind = ?1 AND value = ?2")?
                .query_row(rusqlite::params![kind.as_str(), value], |r| r.get(0))
                .optional()?;
            match p.flatten() {
                Some(p) => Ok(Resolved {
                    person: p,
                    resolved_from: None,
                }),
                None => Err(Error::NotFound(format!("no person with that {kind}"))),
            }
        }
        PersonKey::Name(name) => {
            let key = name_key(name);
            if key.is_empty() {
                return Err(Error::BadRequest("empty person reference".into()));
            }
            let mut st = conn.prepare_cached(
                "SELECT id, display_name FROM person WHERE display_name IS NOT NULL ORDER BY id",
            )?;
            let rows = st
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let hits: Vec<(i64, String)> = rows
                .into_iter()
                .filter(|(_, d)| name_key(d) == key)
                .collect();
            match hits.as_slice() {
                [] => Err(Error::NotFound("no person with that name".into())),
                [(p, _)] => Ok(Resolved {
                    person: *p,
                    resolved_from: None,
                }),
                _ => Err(Error::Ambiguous {
                    reference: name.clone(),
                    candidates: hits,
                }),
            }
        }
    }
}
