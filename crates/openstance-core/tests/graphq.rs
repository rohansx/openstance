//! P2-CORE-04 / P2-CORE-05 acceptance through the `Graph` facade:
//! `get_person` and `how_connected` on the real Phase 1 fixtures
//! (`fixtures/EXPECTED.md`), plus the 20 ms latency budget on a synthetic
//! perf graph (`#[ignore]`d; run with `--release -- --ignored`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use openstance_core::graphq::{HowConnectedResponse, HowOptions, NotInGraph};
use openstance_core::import::{registry, ImportOptions};
use openstance_core::ingest::ImportRequest;
use openstance_core::json::{Evidence, HowConnected};
use openstance_core::resolve::lookup::PersonKey;
use openstance_core::search::person::DetailOptions;
use openstance_core::{config, Config, FixedClock, Graph, Providers};
use rusqlite::params;

/// 2026-10-01T00:00:00Z, the fixtures' "today".
const FIXTURE_NOW: i64 = 1_790_812_800;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn imported() -> Graph {
    let text = std::fs::read_to_string(fixtures().join("config.toml")).unwrap();
    let (cfg, _) = config::parse_str(&text, true).unwrap();
    let providers = Providers::with_importers(registry(&ImportOptions::default()));
    let mut g = Graph::open_in_memory(cfg, providers, Box::new(FixedClock(FIXTURE_NOW))).unwrap();
    let files: Vec<PathBuf> = [
        "mbox/small.mbox",
        "linkedin/Connections.csv",
        "linkedin/messages.csv",
        "linkedin/Invitations.csv",
        "vcf/contacts.vcf",
    ]
    .iter()
    .map(|p| fixtures().join(p))
    .collect();
    g.import(&files, &ImportRequest::default()).unwrap();
    g
}

fn key(s: &str) -> PersonKey {
    PersonKey::parse(s)
}

fn found(r: HowConnectedResponse) -> HowConnected {
    match r {
        HowConnectedResponse::Found(h) => h,
        HowConnectedResponse::NotFound(n) => panic!("not found: {n:?}"),
    }
}

#[test]
fn fixture_who_explains_priyas_score() {
    let g = imported();
    let d = g
        .get_person(&key("priya@northwind.example"), DetailOptions::default())
        .unwrap();
    let p = &d.person;
    assert_eq!(p.display_name.as_deref(), Some("Priya Raman"));
    let c = &p.components;
    let w = &c.weights;
    let sum = w.recency * c.recency
        + w.frequency * c.frequency
        + w.reciprocity * c.reciprocity
        + w.meetings * c.meetings
        + w.breadth * c.breadth;
    // Components are rounded to 3 decimals on output.
    assert!((sum - p.score).abs() < 0.003, "{sum} vs {}", p.score);
    let raw = c.raw.as_ref().unwrap();
    assert!(raw.n_interactions > 50, "{raw:?}");
    assert!(raw.replies_from_them > 0);
    // Three handles, all at 1.0.
    assert_eq!(d.identities.len(), 3);
    assert!(d.identities.iter().all(|i| i.confidence == Some(1.0)));
    assert_eq!(d.recent_interactions.len(), 10);
    assert!(d
        .mutuals
        .iter()
        .any(|m| m.person.display_name.as_deref() == Some("Marcus Lee")));
    // Same record by name and by id.
    let by_name = g
        .get_person(&key("Priya Raman"), DetailOptions::default())
        .unwrap();
    assert_eq!(by_name, d);
    assert_eq!(g.who(&key(&p.id.to_string())).unwrap(), d);
}

#[test]
fn fixture_how_connected_reaches_marcus_through_priya() {
    let g = imported();
    let h = found(
        g.how_connected(&key("marcus@contoso.example"), HowOptions::default())
            .unwrap(),
    );
    assert_eq!(h.target.display_name.as_deref(), Some("Marcus Lee"));
    assert!(h.direct.is_some(), "Marcus is a direct contact too");
    let top = &h.paths[0];
    assert_eq!(top.via.display_name.as_deref(), Some("Priya Raman"));
    let n_shared: u32 = top
        .evidence
        .iter()
        .filter_map(|e| match e {
            Evidence::Tie { n_shared, .. } => Some(*n_shared),
            _ => None,
        })
        .sum();
    assert_eq!(n_shared, 3);
    let shared = top
        .evidence
        .iter()
        .filter(|e| matches!(e, Evidence::Interaction { .. }))
        .count();
    assert_eq!(shared, 3 + 1, "3 shared interactions + my last with Priya");
    assert!((top.strength - top.my_score * top.link_weight).abs() < 0.001);
    assert!(h.paths.len() <= 3);
    for w in h.paths.windows(2) {
        assert!(w[0].strength >= w[1].strength);
    }
}

#[test]
fn fixture_how_connected_unknown_target_is_found_false() {
    let g = imported();
    let r = g
        .how_connected(&key("kai@ramp.example"), HowOptions::default())
        .unwrap();
    assert_eq!(r, HowConnectedResponse::NotFound(NotInGraph::default()));
}

// ---------------------------------------------------------------------------
// Latency: who / how_connected under 20 ms (performance-targets.md)
// ---------------------------------------------------------------------------

/// splitmix64, like the fixture generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

const PEOPLE: i64 = 20_000;
const INTERACTIONS: i64 = 500_000;
const TIES: i64 = 100_000;

/// A random non-owner person id (`2..=PEOPLE`).
fn anyone(rng: &mut Rng) -> i64 {
    2 + rng.below((PEOPLE - 1) as u64) as i64
}

/// A perf-sized graph written with direct SQL, shaped like
/// `gen-graph --people 20000 --interactions 500000`: the counterpart of
/// each email is picked with `u^3` skew (person 2 is on about 18k of them,
/// a few hundred people are heavy, a long tail), one in five has a cc, and
/// 100k ties.
fn perf_graph() -> Graph {
    let g = Graph::open_in_memory(
        Config::default(),
        Providers::null(),
        Box::new(FixedClock(FIXTURE_NOW)),
    )
    .unwrap();
    let c = g.conn();
    let mut rng = Rng(42);
    c.execute_batch(
        "BEGIN; INSERT INTO source(id, kind, label, imported_at) VALUES (1, 'mbox', 'perf', 0);",
    )
    .unwrap();
    {
        let mut person = c
            .prepare(
                "INSERT INTO person(id, display_name, last_contact, score,
                    score_r, score_f, score_c, score_m, score_b, is_self)
                 VALUES (?1, ?2, ?3, ?4, 0.5, 0.5, 0.5, 0.5, 0.5, ?5)",
            )
            .unwrap();
        let mut ident = c
            .prepare(
                "INSERT INTO identity(id, kind, value, source_id, person_id, confidence, match_rule)
                 VALUES (?1, 'email', ?2, 1, ?1, 1.0, 'anchor')",
            )
            .unwrap();
        for id in 1..=PEOPLE {
            let score = (rng.below(1000) as f64) / 1000.0;
            person
                .execute(params![
                    id,
                    format!("Person {id}"),
                    FIXTURE_NOW - rng.below(400 * 86_400) as i64,
                    score,
                    i64::from(id == 1)
                ])
                .unwrap();
            ident
                .execute(params![id, format!("p{id}@perf.example")])
                .unwrap();
        }
        let mut ix = c
            .prepare(
                "INSERT INTO interaction(id, source_id, kind, at, subject, external_id, direction)
                 VALUES (?1, 1, 'email', ?2, ?3, ?1, ?4)",
            )
            .unwrap();
        let mut ip = c
            .prepare(
                "INSERT OR IGNORE INTO interaction_person
                    (interaction_id, identity_id, role, person_id, name_hint)
                 VALUES (?1, ?2, ?3, ?2, ?4)",
            )
            .unwrap();
        let six_years = 6.0 * 365.0 * 86_400.0;
        for id in 1..=INTERACTIONS {
            let u = rng.unit();
            let who = 2 + ((u * u * u) * (PEOPLE - 1) as f64) as i64;
            let out = rng.below(2) == 0;
            let at = FIXTURE_NOW - (rng.unit() * rng.unit() * six_years) as i64;
            ix.execute(params![
                id,
                at,
                format!("update {}", id % 997),
                if out { "out" } else { "in" }
            ])
            .unwrap();
            let (me_role, who_role) = if out { ("from", "to") } else { ("to", "from") };
            ip.execute(params![id, 1, me_role, "Person 1"]).unwrap();
            ip.execute(params![id, who, who_role, format!("Person {who}")])
                .unwrap();
            if rng.below(5) == 0 {
                let cc = anyone(&mut rng);
                ip.execute(params![id, cc, "cc", format!("Person {cc}")])
                    .unwrap();
            }
        }
        let mut tie = c
            .prepare(
                "INSERT OR IGNORE INTO tie(a, b, kind, weight, last_at, n_shared)
                 VALUES (?1, ?2, 'email', ?3, ?4, ?5)",
            )
            .unwrap();
        for _ in 0..TIES {
            let u = rng.unit();
            let a = 2 + ((u * u) * (PEOPLE - 1) as f64) as i64;
            let b = anyone(&mut rng);
            if a == b {
                continue;
            }
            tie.execute(params![
                a.min(b),
                a.max(b),
                (rng.below(1000) as f64) / 1000.0,
                FIXTURE_NOW - rng.below(400 * 86_400) as i64,
                1 + rng.below(20) as i64
            ])
            .unwrap();
        }
    }
    c.execute_batch("COMMIT; ANALYZE;").unwrap();
    g
}

fn p95(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[(v.len() * 95 / 100).min(v.len() - 1)]
}

fn timed(f: impl FnOnce()) -> Duration {
    let s = Instant::now();
    f();
    s.elapsed()
}

/// p95 over 200 random people (the bench's population), each warm.
#[test]
#[ignore = "perf: builds a 20k-person graph; run with --release -- --ignored"]
fn who_and_how_connected_stay_under_20ms_on_the_perf_graph() {
    let g = perf_graph();
    let mut rng = Rng(7);
    let targets: Vec<i64> = (0..200).map(|_| anyone(&mut rng)).collect();
    let (mut who, mut how) = (Vec::new(), Vec::new());
    for t in &targets {
        let k = PersonKey::Id(*t);
        who.push(timed(|| {
            g.get_person(&k, DetailOptions::default()).unwrap();
        }));
        how.push(timed(|| {
            found(g.how_connected(&k, HowOptions::default()).unwrap());
        }));
    }
    let (w, h) = (p95(who), p95(how));
    // The heaviest correspondents, reported (not gated): `who` recomputes
    // the raw score counts from every interaction of the person.
    for heavy in [2, 3, 10, 100] {
        let k = PersonKey::Id(heavy);
        let n: i64 = g
            .conn()
            .query_row(
                "SELECT count(*) FROM interaction_person WHERE person_id = ?1",
                [heavy],
                |r| r.get(0),
            )
            .unwrap();
        let tw = timed(|| {
            g.get_person(&k, DetailOptions::default()).unwrap();
        });
        let th = timed(|| {
            found(g.how_connected(&k, HowOptions::default()).unwrap());
        });
        eprintln!("person {heavy} ({n} interactions): who {tw:?}, how_connected {th:?}");
    }
    eprintln!("who p95 {w:?}, how_connected p95 {h:?}");
    assert!(w < Duration::from_millis(20), "who p95 {w:?}");
    assert!(h < Duration::from_millis(20), "how_connected p95 {h:?}");
}
