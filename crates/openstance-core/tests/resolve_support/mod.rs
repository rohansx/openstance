//! Shared builders for the resolve/rebuild integration tests: write raw
//! rows through the real `GraphSink`, one source per call. All handles are
//! fictional (`*.example`, 555-01xx).
#![allow(dead_code)]

use openstance_core::config::Config;
use openstance_core::db::{migrate, open_in_memory};
use openstance_core::import::{
    Direction, Handle, HandleKind, Hints, InteractionKind, Participant, RawIdentity,
    RawInteraction, Role, Sink,
};
use openstance_core::ingest::{GraphSink, SinkOptions, Touched};
use openstance_core::resolve::{self, MatchRule, Resolution};
use openstance_core::{IdentityId, PersonId};
use rusqlite::Connection;

/// 2026-10-01T00:00:00Z.
pub const NOW: i64 = 1_790_812_800;
pub const DAY: i64 = 86_400;

pub fn new_db() -> Connection {
    let mut c = open_in_memory().unwrap();
    migrate(&mut c, None, 0).unwrap();
    c
}

pub fn cfg_with_me(me: &[&str]) -> Config {
    let mut cfg = Config::default();
    cfg.me.identities = me.iter().map(|s| s.to_string()).collect();
    cfg
}

fn new_source(c: &Connection, kind: &str) -> i64 {
    c.execute(
        "INSERT INTO source(kind, label, imported_at) VALUES (?1, ?1, ?2)",
        rusqlite::params![kind, NOW],
    )
    .unwrap();
    c.last_insert_rowid()
}

pub fn handle(spec: &str) -> Handle {
    let (kind, raw) = spec.split_once(':').unwrap();
    Handle::new(HandleKind::parse(kind).unwrap(), raw)
}

pub fn hints(name: &str, company: Option<&str>) -> Hints {
    Hints {
        name: (!name.is_empty()).then(|| name.to_string()),
        company: company.map(str::to_string),
        ..Hints::default()
    }
}

/// Run `f` against a fresh GraphSink for a new source of `kind`.
pub fn with_sink(c: &Connection, kind: &str, f: impl FnOnce(&mut GraphSink<'_>)) -> Touched {
    let src = new_source(c, kind);
    let mut sink = GraphSink::new(
        c,
        src,
        SinkOptions {
            default_region: Some("US".into()),
            importer_kind: kind.into(),
        },
        NOW,
    );
    f(&mut sink);
    sink.finish().unwrap().touched
}

/// One card (vCard / CSV row): every handle in one record.
pub fn card(
    c: &Connection,
    kind: &str,
    handles: &[&str],
    name: &str,
    company: Option<&str>,
) -> Touched {
    with_sink(c, kind, |s| {
        for h in handles {
            s.identity(RawIdentity {
                handle: handle(h),
                hints: hints(name, company),
                record: "card".into(),
                observed_at: None,
            });
        }
    })
}

/// A LinkedIn connection row: slug (+ email) in one participant slot.
pub fn connection(c: &Connection, handles: &[&str], name: &str, company: &str) -> Touched {
    with_sink(c, "linkedin_csv", |s| {
        s.interaction(RawInteraction {
            kind: InteractionKind::Connection,
            at: NOW - 400 * DAY,
            dir_hint: Some(Direction::Mutual),
            subject: None,
            external_id: Some(format!("conn:{}", handles[0])),
            thread_key: None,
            list_unsub: false,
            participants: vec![Participant {
                handles: handles.iter().map(|h| handle(h)).collect(),
                role: Role::Subject,
                hints: Hints {
                    name: Some(name.into()),
                    company: Some(company.into()),
                    title: Some("Engineer".into()),
                    ..Hints::default()
                },
            }],
        })
    })
}

/// One email message. Participants are `(email, display name)`.
pub struct Mail<'a> {
    pub id: &'a str,
    pub thread: &'a str,
    pub at: i64,
    pub from: (&'a str, &'a str),
    pub to: Vec<(&'a str, &'a str)>,
    pub cc: Vec<(&'a str, &'a str)>,
    pub list_unsub: bool,
}

impl<'a> Mail<'a> {
    pub fn new(id: &'a str, from: (&'a str, &'a str), to: (&'a str, &'a str)) -> Self {
        Mail {
            id,
            thread: id,
            at: NOW - 10 * DAY,
            from,
            to: vec![to],
            cc: vec![],
            list_unsub: false,
        }
    }
    pub fn thread(mut self, t: &'a str) -> Self {
        self.thread = t;
        self
    }
    pub fn cc(mut self, cc: (&'a str, &'a str)) -> Self {
        self.cc.push(cc);
        self
    }
    pub fn raw(&self) -> RawInteraction {
        let p = |(addr, name): (&str, &str), role| Participant {
            handles: vec![Handle::new(HandleKind::Email, addr)],
            role,
            hints: hints(name, None),
        };
        let mut participants = vec![p(self.from, Role::From)];
        participants.extend(self.to.iter().map(|x| p(*x, Role::To)));
        participants.extend(self.cc.iter().map(|x| p(*x, Role::Cc)));
        RawInteraction {
            kind: InteractionKind::Email,
            at: self.at,
            dir_hint: None,
            subject: Some(format!("subject {}", self.id)),
            external_id: Some(self.id.to_string()),
            thread_key: Some(self.thread.to_string()),
            list_unsub: self.list_unsub,
            participants,
        }
    }
}

pub fn mbox(c: &Connection, mails: &[Mail<'_>]) -> Touched {
    with_sink(c, "mbox", |s| {
        for m in mails {
            s.interaction(m.raw());
        }
    })
}

pub fn identity_id(c: &Connection, spec: &str) -> IdentityId {
    let (kind, value) = spec.split_once(':').unwrap();
    c.query_row(
        "SELECT id FROM identity WHERE kind = ?1 AND value = ?2",
        [kind, value],
        |r| r.get(0),
    )
    .unwrap()
}

pub fn resolve(c: &Connection, cfg: &Config) -> Resolution {
    resolve::resolve(c, cfg).unwrap()
}

/// Person of an identity in a resolution.
pub fn person_of(c: &Connection, res: &Resolution, spec: &str) -> Option<PersonId> {
    res.assignments
        .get(&identity_id(c, spec))
        .map(|a| a.person_id)
}

pub fn rule_of(c: &Connection, res: &Resolution, spec: &str) -> (MatchRule, f64) {
    let a = res.assignments[&identity_id(c, spec)];
    (a.rule, a.confidence)
}

pub fn same(c: &Connection, res: &Resolution, a: &str, b: &str) -> bool {
    let (x, y) = (person_of(c, res, a), person_of(c, res, b));
    x.is_some() && x == y
}

pub fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

pub fn add_override(
    c: &Connection,
    person: i64,
    field: &str,
    value: Option<&str>,
    set_at: i64,
) -> i64 {
    c.execute(
        "INSERT INTO override(person_id, field, value, set_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![person, field, value, set_at],
    )
    .unwrap();
    c.last_insert_rowid()
}
