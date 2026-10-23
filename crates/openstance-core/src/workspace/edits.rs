//! `POST /edits` (interfaces.md 3.2): a batch of override operations, merges,
//! splits and review decisions applied in **one** write transaction. Each op
//! does exactly what the matching CLI command does (`edit`, `merge`,
//! `merge --undo`, `split`, `review --confirm`, `review --reject`), so the
//! override rows are the same whichever surface wrote them.

use std::collections::BTreeSet;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::json::{PersonRef, V};
use crate::overrides::{self, Edit, OverrideField};
use crate::resolve::lookup::{resolve_person, PersonKey};
use crate::search::person::person_ref;
use crate::{rebuild, resolve, Config, Graph, IdentityId, PersonId};

/// Most ops accepted in one request.
pub const MAX_OPS: usize = 500;

/// A person reference in an op: an id or any string `PersonKey::parse`
/// accepts (email, LinkedIn slug, @handle, name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PersonArg {
    /// A person (anchor) id.
    Id(i64),
    /// A reference string.
    Ref(String),
}

impl PersonArg {
    /// The lookup key.
    pub fn key(&self) -> PersonKey {
        match self {
            PersonArg::Id(n) => PersonKey::Id(*n),
            PersonArg::Ref(s) => PersonKey::parse(s),
        }
    }
}

/// One edit operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    /// Set a scalar field (`display_name`, `title`, `company`, `location`, `bio`).
    Set {
        /// Person.
        person: PersonArg,
        /// Field.
        field: String,
        /// New value.
        value: String,
    },
    /// Restore the derived value of a scalar field (writes a tombstone).
    Unset {
        /// Person.
        person: PersonArg,
        /// Field.
        field: String,
    },
    /// Add a tag.
    Tag {
        /// Person.
        person: PersonArg,
        /// Tag.
        value: String,
    },
    /// Remove a tag.
    Untag {
        /// Person.
        person: PersonArg,
        /// Tag.
        value: String,
    },
    /// Add a note.
    Note {
        /// Person.
        person: PersonArg,
        /// Note text.
        value: String,
    },
    /// Set a flag (`do_not_share`, `no_enrich`, `is_bulk`).
    Flag {
        /// Person.
        person: PersonArg,
        /// Flag name.
        field: String,
        /// On or off.
        value: bool,
    },
    /// Merge two people (writes `merged_into`).
    Merge {
        /// First person.
        a: PersonArg,
        /// Second person.
        b: PersonArg,
    },
    /// Remove a forced merge between two anchor ids.
    Unmerge {
        /// First anchor.
        a: PersonArg,
        /// Second anchor.
        b: PersonArg,
    },
    /// Move identities out of a person into a new one (writes `split_from`).
    Split {
        /// Person.
        person: PersonArg,
        /// Identity ids to move.
        identities: Vec<IdentityId>,
    },
    /// Accept a flagged merge.
    Confirm {
        /// The flagged person.
        person: PersonArg,
    },
    /// Keep two suggested people apart.
    Reject {
        /// First person.
        a: PersonArg,
        /// Second person.
        b: PersonArg,
    },
}

/// The `POST /edits` body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditsRequest {
    /// Ops, applied in order.
    pub ops: Vec<EditOp>,
}

/// The `POST /edits` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditsResponse {
    /// Schema version.
    pub v: u32,
    /// Ops applied.
    pub applied: usize,
    /// Every person the batch touched, after the change (by id).
    pub people: Vec<PersonRef>,
}

fn scalar(field: &str) -> Result<OverrideField> {
    let f = match field.trim() {
        "name" => Some(OverrideField::DisplayName),
        f => OverrideField::parse(f),
    };
    f.filter(|f| f.is_scalar() && !is_flag(*f)).ok_or_else(|| {
        Error::BadRequest(format!(
            "field {field:?} is not one of display_name, title, company, location, bio"
        ))
    })
}

fn is_flag(f: OverrideField) -> bool {
    matches!(
        f,
        OverrideField::DoNotShare | OverrideField::NoEnrich | OverrideField::IsBulk
    )
}

fn person(tx: &Connection, p: &PersonArg) -> Result<PersonId> {
    Ok(resolve_person(tx, &p.key())?.person)
}

/// An anchor id as given (undo ops address anchors), else the person.
fn anchor(tx: &Connection, p: &PersonArg) -> Result<PersonId> {
    match p {
        PersonArg::Id(n) => Ok(*n),
        PersonArg::Ref(s) => match s.trim().parse::<i64>() {
            Ok(n) => Ok(n),
            Err(_) => person(tx, p),
        },
    }
}

fn touched(ids: &[PersonId]) -> crate::ingest::Touched {
    crate::ingest::Touched {
        identities: Default::default(),
        people: ids.iter().copied().collect(),
    }
}

/// Apply one op; returns the people it touched (before re-resolution).
fn apply(tx: &Connection, cfg: &Config, op: &EditOp, now: i64) -> Result<Vec<PersonId>> {
    let field_edit = |p: &PersonArg, e: Edit| -> Result<Vec<PersonId>> {
        let id = person(tx, p)?;
        overrides::apply_edit(tx, id, &e, now)?;
        rebuild::refresh_people(tx, cfg, &[id], now)?;
        Ok(vec![id])
    };
    match op {
        EditOp::Set {
            person: p,
            field,
            value,
        } => field_edit(
            p,
            Edit {
                set: vec![(scalar(field)?, value.clone())],
                ..Edit::default()
            },
        ),
        EditOp::Unset { person: p, field } => field_edit(
            p,
            Edit {
                unset: vec![scalar(field)?],
                ..Edit::default()
            },
        ),
        EditOp::Tag { person: p, value } => field_edit(
            p,
            Edit {
                add_tags: vec![non_empty("tag", value)?],
                ..Edit::default()
            },
        ),
        EditOp::Untag { person: p, value } => field_edit(
            p,
            Edit {
                remove_tags: vec![value.clone()],
                ..Edit::default()
            },
        ),
        EditOp::Note { person: p, value } => field_edit(
            p,
            Edit {
                add_notes: vec![non_empty("note", value)?],
                ..Edit::default()
            },
        ),
        EditOp::Flag {
            person: p,
            field,
            value,
        } => {
            let mut e = Edit::default();
            match OverrideField::parse(field.trim()) {
                Some(OverrideField::DoNotShare) => e.do_not_share = Some(*value),
                Some(OverrideField::NoEnrich) => e.no_enrich = Some(*value),
                Some(OverrideField::IsBulk) => e.is_bulk = Some(*value),
                _ => {
                    return Err(Error::BadRequest(format!(
                        "flag {field:?} is not one of do_not_share, no_enrich, is_bulk"
                    )))
                }
            }
            field_edit(p, e)
        }
        EditOp::Merge { a, b } => {
            let ids = [person(tx, a)?, person(tx, b)?];
            let out = resolve::commands::merge(tx, &ids, now)?;
            rebuild::incremental(tx, cfg, &touched(&ids), now)?;
            Ok(vec![out.person])
        }
        EditOp::Unmerge { a, b } => {
            let (x, y) = (anchor(tx, a)?, anchor(tx, b)?);
            resolve::commands::unmerge(tx, x, y)?;
            rebuild::incremental(tx, cfg, &touched(&[x, y]), now)?;
            Ok(vec![x, y])
        }
        EditOp::Split {
            person: p,
            identities,
        } => {
            if identities.is_empty() {
                return Err(Error::BadRequest(
                    "split needs at least one identity".into(),
                ));
            }
            let id = person(tx, p)?;
            let out = resolve::commands::split(tx, id, identities, now)?;
            rebuild::incremental(tx, cfg, &touched(&[out.kept, out.moved]), now)?;
            Ok(vec![out.kept, out.moved])
        }
        EditOp::Confirm { person: p } => {
            let id = person(tx, p)?;
            resolve::review::confirm(tx, id, now)?;
            rebuild::incremental(tx, cfg, &touched(&[id]), now)?;
            Ok(vec![id])
        }
        EditOp::Reject { a, b } => {
            let (x, y) = (person(tx, a)?, person(tx, b)?);
            resolve::review::reject(tx, x, y, now)?;
            rebuild::incremental(tx, cfg, &touched(&[x, y]), now)?;
            Ok(vec![x, y])
        }
    }
}

fn non_empty(what: &str, v: &str) -> Result<String> {
    if v.trim().is_empty() {
        return Err(Error::BadRequest(format!("{what} must not be empty")));
    }
    Ok(v.to_string())
}

impl Graph {
    /// Apply `ops` in one write transaction (all or nothing), then refresh
    /// embeddings for the touched people.
    pub fn apply_edits(&mut self, ops: &[EditOp]) -> Result<EditsResponse> {
        if ops.is_empty() || ops.len() > MAX_OPS {
            return Err(Error::BadRequest(format!(
                "ops must hold 1 to {MAX_OPS} items (got {})",
                ops.len()
            )));
        }
        let now = self.now();
        let ids = self.write(|tx, cfg| {
            let mut ids = Vec::new();
            for (i, op) in ops.iter().enumerate() {
                let t = apply(tx, cfg, op, now).map_err(|e| match e {
                    Error::BadRequest(m) => Error::BadRequest(format!("ops[{i}]: {m}")),
                    e => e,
                })?;
                ids.extend(t);
            }
            Ok(ids)
        })?;
        self.refresh_embeddings(None)?;
        let mut seen = BTreeSet::new();
        let mut people = Vec::new();
        for id in ids {
            let Ok(r) = resolve_person(self.conn(), &PersonKey::Id(id)) else {
                continue;
            };
            if seen.insert(r.person) {
                if let Some(p) = person_ref(self.conn(), r.person)? {
                    people.push(p);
                }
            }
        }
        Ok(EditsResponse {
            v: V,
            applied: ops.len(),
            people,
        })
    }
}
