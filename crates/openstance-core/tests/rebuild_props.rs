//! P1-CORE-09 property tests (entity-resolution.md section 11,
//! testing-strategy.md "Property"): over random small graphs built from a
//! fictional cast and random override sequences,
//! - rebuild is idempotent (byte-identical derived tables);
//! - importing source by source with incremental resolution, then applying
//!   merge / split / edit commands, equals one full rebuild (incremental
//!   equals full, and overrides survive the rebuild);
//! - a `split_from` pair is never in one person after any further import;
//! - adding a `split_from` never grows any person.
//!
//! Case count: `PROPTEST_CASES` (default 48 to keep PR CI fast).

mod resolve_support;

use openstance_core::import::{
    Direction, Handle, HandleKind, Hints, InteractionKind, Participant, RawIdentity,
    RawInteraction, Role, Sink,
};
use openstance_core::ingest::Touched;
use openstance_core::overrides::{Edit, OverrideField};
use openstance_core::rebuild;
use openstance_core::resolve::lookup::{resolve_person, PersonKey};
use openstance_core::resolve::{self as res};
use openstance_core::{FixedClock, Graph, Providers};
use proptest::prelude::*;
use resolve_support::*;
use rusqlite::Connection;

struct Cast {
    name: &'static str,
    company: &'static str,
    emails: [&'static str; 2],
    slug: &'static str,
    phone: &'static str,
}

const CAST: [Cast; 6] = [
    Cast {
        name: "Ines Quillfeather",
        company: "Halcyon",
        emails: ["ines@halcyon.example", "ines.q@gmail.com"],
        slug: "ines-q",
        phone: "+12015550160",
    },
    Cast {
        name: "Priya Sharma",
        company: "Northwind",
        emails: ["priya@northwind.example", "priya.s@gmail.com"],
        slug: "priya-sharma",
        phone: "+12015550161",
    },
    Cast {
        name: "Priya Sharma",
        company: "Contoso",
        emails: ["psharma@contoso.example", "p.sharma@gmail.com"],
        slug: "priya-sharma-2",
        phone: "+12015550162",
    },
    Cast {
        name: "Jon Quillby",
        company: "Brightwater",
        emails: ["jon@brightwater.example", "jonq+x@gmail.com"],
        slug: "jon-quillby",
        phone: "+12015550163",
    },
    Cast {
        name: "John Quillby",
        company: "Brightwater",
        emails: ["john@brightwater.example", "john.q@gmail.com"],
        slug: "john-quillby",
        phone: "+12015550164",
    },
    Cast {
        name: "Tariq Bellweather",
        company: "Fabrikam",
        emails: ["tariq@fabrikam.example", "tariq.b@gmail.com"],
        slug: "tariq-b",
        phone: "+12015550165",
    },
];
const OWNER: &str = "owner@example.com";

#[derive(Debug, Clone)]
enum Msg {
    /// (person, email index, inbound?, cc: Option<(person, email)>, thread)
    Mail(usize, usize, bool, Option<(usize, usize)>, u8),
}

#[derive(Debug, Clone)]
enum Src {
    Mbox(Vec<Msg>),
    /// person, handle mask (bit 0 email0, 1 email1, 2 phone, 3 slug)
    Card(usize, u8),
    /// person, include email0
    Linkedin(usize, bool),
}

#[derive(Debug, Clone)]
enum Op {
    Merge(usize, usize),
    Split(usize),
    Title(usize, u8),
    Tag(usize),
}

fn msg() -> impl Strategy<Value = Msg> {
    (
        0..6usize,
        0..2usize,
        any::<bool>(),
        proptest::option::of((0..6usize, 0..2usize)),
        0..4u8,
    )
        .prop_map(|(p, e, inb, cc, t)| Msg::Mail(p, e, inb, cc, t))
}

fn src() -> impl Strategy<Value = Src> {
    prop_oneof![
        proptest::collection::vec(msg(), 1..8).prop_map(Src::Mbox),
        (0..6usize, 1..16u8).prop_map(|(p, m)| Src::Card(p, m)),
        (0..6usize, any::<bool>()).prop_map(|(p, e)| Src::Linkedin(p, e)),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0..6usize, 0..6usize).prop_map(|(a, b)| Op::Merge(a, b)),
        (0..6usize).prop_map(Op::Split),
        (0..6usize, 0..3u8).prop_map(|(p, t)| Op::Title(p, t)),
        (0..6usize).prop_map(Op::Tag),
    ]
}

fn email_handle(addr: &str) -> Handle {
    Handle::new(HandleKind::Email, addr)
}

fn stage(c: &Connection, n: usize, s: &Src) -> Touched {
    match s {
        Src::Mbox(msgs) => with_sink(c, "mbox", |sink| {
            for (k, Msg::Mail(p, e, inbound, cc, t)) in msgs.iter().enumerate() {
                let who = &CAST[*p];
                let them = Participant {
                    handles: vec![email_handle(who.emails[*e])],
                    role: if *inbound { Role::From } else { Role::To },
                    hints: hints(who.name, None),
                };
                let me = Participant {
                    handles: vec![email_handle(OWNER)],
                    role: if *inbound { Role::To } else { Role::From },
                    hints: hints("Owen Ryder", None),
                };
                let mut participants = if *inbound {
                    vec![them, me]
                } else {
                    vec![me, them]
                };
                if let Some((q, f)) = cc {
                    participants.push(Participant {
                        handles: vec![email_handle(CAST[*q].emails[*f])],
                        role: Role::Cc,
                        hints: hints(CAST[*q].name, None),
                    });
                }
                sink.interaction(RawInteraction {
                    kind: InteractionKind::Email,
                    at: NOW - (k as i64 + 1) * 7 * DAY,
                    dir_hint: None,
                    subject: Some(format!("note {n}-{k}")),
                    external_id: Some(format!("s{n}m{k}@example.com")),
                    thread_key: Some(format!("thread-{t}")),
                    list_unsub: false,
                    participants,
                });
            }
        }),
        Src::Card(p, mask) => with_sink(c, "vcf", |sink| {
            let who = &CAST[*p];
            let mut handles = Vec::new();
            for (bit, h) in [
                (0, Handle::new(HandleKind::Email, who.emails[0])),
                (1, Handle::new(HandleKind::Email, who.emails[1])),
                (2, Handle::new(HandleKind::Phone, who.phone)),
                (
                    3,
                    Handle::new(
                        HandleKind::LinkedinUrl,
                        format!("linkedin.com/in/{}", who.slug),
                    ),
                ),
            ] {
                if mask & (1 << bit) != 0 {
                    handles.push(h);
                }
            }
            for h in handles {
                sink.identity(RawIdentity {
                    handle: h,
                    hints: hints(who.name, Some(who.company)),
                    record: format!("card-{n}"),
                    observed_at: Some(NOW - 30 * DAY),
                });
            }
        }),
        Src::Linkedin(p, with_email) => with_sink(c, "linkedin_csv", |sink| {
            let who = &CAST[*p];
            let mut handles = vec![Handle::new(
                HandleKind::LinkedinUrl,
                format!("linkedin.com/in/{}", who.slug),
            )];
            if *with_email {
                handles.push(email_handle(who.emails[0]));
            }
            sink.interaction(RawInteraction {
                kind: InteractionKind::Connection,
                at: NOW - 200 * DAY,
                dir_hint: Some(Direction::Mutual),
                subject: None,
                external_id: Some(format!("conn:{n}:{}", who.slug)),
                thread_key: None,
                list_unsub: false,
                participants: vec![Participant {
                    handles,
                    role: Role::Subject,
                    hints: Hints {
                        name: Some(who.name.into()),
                        title: Some("Engineer".into()),
                        company: Some(who.company.into()),
                        location: None,
                    },
                }],
            });
        }),
    }
}

/// Some handle of cast member `p` that exists in the graph.
fn some_handle(c: &Connection, p: usize) -> Option<i64> {
    let who = &CAST[p];
    let slug = who.slug.to_string();
    for (kind, value) in [
        ("email", who.emails[0].to_string()),
        ("email", who.emails[1].to_string()),
        ("phone", who.phone.to_string()),
        ("linkedin_url", slug),
    ] {
        let id: Option<i64> = c
            .query_row(
                "SELECT id FROM identity WHERE kind = ?1 AND value = ?2 AND person_id IS NOT NULL",
                [kind, value.as_str()],
                |r| r.get(0),
            )
            .ok();
        if id.is_some() {
            return id;
        }
    }
    None
}

fn apply(g: &mut Graph, op: &Op) {
    let person = |g: &Graph, p: usize| -> Option<i64> {
        let h = some_handle(g.conn(), p)?;
        resolve_person(g.conn(), &PersonKey::Id(h))
            .ok()
            .map(|r| r.person)
    };
    match op {
        Op::Merge(a, b) => {
            if let (Some(x), Some(y)) = (person(g, *a), person(g, *b)) {
                if x != y {
                    g.merge(&[PersonKey::Id(x), PersonKey::Id(y)]).unwrap();
                }
            }
        }
        Op::Split(a) => {
            if let Some(p) = person(g, *a) {
                let ids: Vec<i64> = g
                    .conn()
                    .prepare("SELECT id FROM identity WHERE person_id = ?1 ORDER BY id")
                    .unwrap()
                    .query_map([p], |r| r.get(0))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                if ids.len() > 1 {
                    g.split(&PersonKey::Id(p), &ids[ids.len() - 1..]).unwrap();
                }
            }
        }
        Op::Title(a, t) => {
            if let Some(p) = person(g, *a) {
                g.edit(
                    &PersonKey::Id(p),
                    &Edit {
                        set: vec![(
                            OverrideField::Title,
                            ["CTO", "Founder", ""][*t as usize].into(),
                        )],
                        ..Edit::default()
                    },
                )
                .unwrap();
            }
        }
        Op::Tag(a) => {
            if let Some(p) = person(g, *a) {
                g.edit(
                    &PersonKey::Id(p),
                    &Edit {
                        add_tags: vec!["investor".into()],
                        ..Edit::default()
                    },
                )
                .unwrap();
            }
        }
    }
}

fn graph() -> Graph {
    Graph::open_in_memory(
        cfg_with_me(&["email:owner@example.com"]),
        Providers::null(),
        Box::new(FixedClock(NOW)),
    )
    .unwrap()
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(48)
}

/// Identity pairs that a `split_from` row separates.
fn split_pairs(c: &Connection) -> Vec<(i64, i64)> {
    c.prepare("SELECT person_id, CAST(value AS INTEGER) FROM override WHERE field = 'split_from'")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn person_of_identity(c: &Connection, id: i64) -> Option<i64> {
    c.query_row("SELECT person_id FROM identity WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    #[test]
    fn rebuild_is_idempotent(srcs in proptest::collection::vec(src(), 1..6), ops in proptest::collection::vec(op(), 0..5)) {
        let mut g = graph();
        for (n, s) in srcs.iter().enumerate() {
            stage(g.conn(), n, s);
        }
        g.rebuild().unwrap();
        for o in &ops {
            apply(&mut g, o);
        }
        g.rebuild().unwrap();
        let first = derived_dump(g.conn());
        g.rebuild().unwrap();
        prop_assert_eq!(derived_dump(g.conn()), first);
    }

    #[test]
    fn incremental_imports_and_commands_equal_a_full_rebuild(
        srcs in proptest::collection::vec(src(), 1..6),
        ops in proptest::collection::vec(op(), 0..5),
        later in proptest::collection::vec(src(), 0..3),
    ) {
        let mut g = graph();
        for (n, s) in srcs.iter().enumerate() {
            g.write(|tx, cfg| { let t = stage(tx, n, s); rebuild::incremental(tx, cfg, &t, NOW) }).unwrap();
        }
        for o in &ops {
            apply(&mut g, o);
        }
        for (k, s) in later.iter().enumerate() {
            let n = srcs.len() + k;
            g.write(|tx, cfg| { let t = stage(tx, n, s); rebuild::incremental(tx, cfg, &t, NOW) }).unwrap();
        }
        let incremental = derived_dump(g.conn());
        let pairs = split_pairs(g.conn());
        g.rebuild().unwrap();
        prop_assert_eq!(derived_dump(g.conn()), incremental);
        // Split holds: no split_from pair shares a person after later imports.
        for (a, b) in pairs {
            let (pa, pb) = (person_of_identity(g.conn(), a), person_of_identity(g.conn(), b));
            prop_assert!(pa.is_none() || pa != pb, "split_from {} / {} violated", a, b);
        }
    }

    #[test]
    fn adding_a_split_from_never_grows_a_person(
        srcs in proptest::collection::vec(src(), 1..6),
        pick in (any::<proptest::sample::Index>(), any::<proptest::sample::Index>()),
    ) {
        let c = new_db();
        for (n, s) in srcs.iter().enumerate() {
            stage(&c, n, s);
        }
        let cfg = cfg_with_me(&["email:owner@example.com"]);
        let before = res::resolve(&c, &cfg).unwrap();
        let ids: Vec<i64> = before.assignments.keys().copied().collect();
        prop_assume!(ids.len() >= 2);
        let (a, b) = (ids[pick.0.index(ids.len())], ids[pick.1.index(ids.len())]);
        prop_assume!(a != b);
        add_override(&c, a, "split_from", Some(&b.to_string()), 1);
        let after = res::resolve(&c, &cfg).unwrap();
        let size = |r: &res::Resolution, id: i64| -> usize {
            let p = r.assignments[&id].person_id;
            r.assignments.values().filter(|x| x.person_id == p).count()
        };
        for id in ids {
            prop_assert!(size(&after, id) <= size(&before, id));
        }
        prop_assert_ne!(after.assignments[&a].person_id, after.assignments[&b].person_id);
    }
}
