//! The synthetic perf graph and fixed query sets behind `benches/search.rs`
//! (P2-CORE-07), shared with `tests/bench_graph.rs`.
//!
//! The graph is written with direct SQL (seconds, not the minutes a full
//! import of `gen-graph` output takes) in the shape performance-targets.md
//! describes: people with titles, companies and locations from a synthetic
//! vocabulary, about 3% bulk senders, Zipf-like interactions per person
//! (`u^3` skew) over six years with recency skew, one in ten a meeting,
//! subjects from a topic vocabulary so FTS sees realistic term frequencies,
//! then the real FTS index and a real embedding pass with the configured
//! embedder. Every name and domain is fictional. Deterministic per seed.

#![allow(dead_code)]

use openstance_core::embed::IndexOptions;
use openstance_core::search::{fts, today_start, SearchRequest};
use openstance_core::{Config, FixedClock, Graph, Providers, QueryPatch};
use openstance_model::embed::{embedder_from_spec, EmbedderOptions};
use rusqlite::params;

/// 2026-10-20T12:00:00Z, the "now" of the graph and every query.
pub const NOW: i64 = 1_792_454_400 + 12 * 3600;

const DAY: i64 = 86_400;

/// Generator parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerfParams {
    /// People including the owner (person 1).
    pub people: i64,
    /// Interactions (emails and meetings).
    pub interactions: i64,
    /// PRNG seed.
    pub seed: u64,
    /// `model.embedder` spec (`hash:384`, `fastembed:bge-small-en-v1.5`).
    pub embedder: String,
}

impl PerfParams {
    /// The perf fixture (20k people, 500k interactions, 384-dim hash
    /// vectors), overridable with `OPENSTANCE_BENCH_PEOPLE`,
    /// `OPENSTANCE_BENCH_INTERACTIONS` and `OPENSTANCE_BENCH_EMBEDDER`.
    pub fn from_env() -> Self {
        let num = |k: &str, d: i64| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d)
        };
        PerfParams {
            people: num("OPENSTANCE_BENCH_PEOPLE", 20_000),
            interactions: num("OPENSTANCE_BENCH_INTERACTIONS", 500_000),
            seed: 42,
            embedder: std::env::var("OPENSTANCE_BENCH_EMBEDDER")
                .unwrap_or_else(|_| "hash:384".into()),
        }
    }
}

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

const FIRST: &[&str] = &[
    "Ada", "Bram", "Cleo", "Dev", "Esme", "Farid", "Greta", "Hugo", "Ines", "Jonah", "Kira",
    "Luca", "Mina", "Nils", "Oona", "Pavel", "Quinn", "Rosa", "Teo", "Uma", "Vera", "Wes", "Yara",
    "Zane",
];
const LAST: &[&str] = &[
    "Arden", "Brook", "Calder", "Dune", "Ellery", "Frost", "Garnet", "Hollis", "Ives", "Juniper",
    "Kestrel", "Lark", "Merrow", "Nash", "Orrin", "Pike", "Quill", "Rook", "Sable", "Thorne",
];
const TITLES: &[&str] = &[
    "Software Engineer",
    "Staff Engineer",
    "Engineering Manager",
    "Product Manager",
    "Designer",
    "Founder",
    "Co-founder & CEO",
    "CTO",
    "Partner",
    "Principal",
    "Data Scientist",
    "Recruiter",
    "Head of Growth",
    "Account Executive",
    "Researcher",
    "Angel Investor",
];
const COMPANY_A: &[&str] = &[
    "North", "Blue", "Iron", "Cedar", "Lumen", "Quartz", "Harbor", "Echo", "Atlas", "Ember",
    "Willow", "Summit",
];
const COMPANY_B: &[&str] = &[
    "wind", "labs", "works", "ventures", "systems", "forge", "logic", "field", "stack", "path",
    "bridge", "grid", "data", "cloud", "health", "pay", "ware",
];
const CITIES: &[&str] = &[
    "San Francisco",
    "New York",
    "London",
    "Berlin",
    "Paris",
    "Bangalore",
    "Toronto",
    "Austin",
    "Seattle",
    "Amsterdam",
    "Singapore",
    "Sydney",
    "Lisbon",
    "Stockholm",
    "Tel Aviv",
    "Boston",
];
const TOPICS: &[&str] = &[
    "devtools",
    "fundraising",
    "seed round",
    "hiring",
    "kubernetes",
    "rust",
    "design review",
    "board deck",
    "pricing",
    "launch",
    "onboarding",
    "observability",
    "postgres",
    "security audit",
    "roadmap",
    "angel investing",
    "climate",
    "payments",
    "machine learning",
    "open source",
    "conference",
    "dinner",
    "coffee",
    "intro",
    "offsite",
    "partnership",
    "contract",
    "customer feedback",
];
const SUBJECT_FORMS: &[&str] = &[
    "{t} sync",
    "Re: {t}",
    "{t} follow-up",
    "Quick question about {t}",
    "{t} notes",
    "Intro: {t}",
];

/// A searchable graph at `p`'s size, FTS-indexed and embedded.
pub fn build(p: &PerfParams) -> Graph {
    let opts = EmbedderOptions {
        cache_dir: std::env::var_os("OPENSTANCE_MODEL_DIR")
            .map(Into::into)
            .unwrap_or_else(|| "models".into()),
        offline: std::env::var_os("OPENSTANCE_OFFLINE").is_some(),
        ..EmbedderOptions::default()
    };
    let mut providers = Providers::null();
    providers.models.embedder =
        embedder_from_spec(&p.embedder, &opts).expect("OPENSTANCE_BENCH_EMBEDDER");
    let mut g = Graph::open_in_memory(Config::default(), providers, Box::new(FixedClock(NOW)))
        .expect("open graph");
    let mut rng = Rng(p.seed);
    let c = g.conn();
    c.execute_batch(
        "BEGIN;
         INSERT INTO source(id, kind, label, imported_at) VALUES (1, 'mbox', 'perf mail', 0);
         INSERT INTO source(id, kind, label, imported_at) VALUES (2, 'ics', 'perf calendar', 0);",
    )
    .unwrap();
    {
        let mut person = c
            .prepare(
                "INSERT INTO person(id, display_name, title, company, location, bio,
                    first_seen, last_contact, n_interactions, score,
                    score_r, score_f, score_c, score_m, score_b, is_bulk, is_self)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            )
            .unwrap();
        let mut ident = c
            .prepare(
                "INSERT INTO identity(id, kind, value, source_id, person_id, confidence, match_rule)
                 VALUES (?1, 'email', ?2, 1, ?1, 1.0, 'anchor')",
            )
            .unwrap();
        for id in 1..=p.people {
            let is_self = id == 1;
            let is_bulk = !is_self && rng.below(100) < 3;
            let name = if is_self {
                "Owner Perf".to_string()
            } else if is_bulk {
                format!("{} Digest {id}", rng.pick(COMPANY_A))
            } else {
                format!("{} {} {id}", rng.pick(FIRST), rng.pick(LAST))
            };
            let known = !is_bulk && rng.below(10) < 8;
            let title = known.then(|| rng.pick(TITLES).to_string());
            let company = known.then(|| format!("{}{}", rng.pick(COMPANY_A), rng.pick(COMPANY_B)));
            let location = (known && rng.below(10) < 7).then(|| rng.pick(CITIES).to_string());
            let bio = (known && rng.below(10) < 2)
                .then(|| format!("Works on {} and {}", rng.pick(TOPICS), rng.pick(TOPICS)));
            let comps: Vec<f64> = (0..5).map(|_| rng.below(1000) as f64 / 1000.0).collect();
            let score = if is_bulk || is_self {
                0.0
            } else {
                0.35 * comps[0]
                    + 0.2 * comps[1]
                    + 0.25 * comps[2]
                    + 0.15 * comps[3]
                    + 0.05 * comps[4]
            };
            let last = NOW - (rng.unit() * rng.unit() * 2000.0 * DAY as f64) as i64;
            person
                .execute(params![
                    id,
                    name,
                    title,
                    company,
                    location,
                    bio,
                    last - 400 * DAY,
                    last,
                    score,
                    comps[0],
                    comps[1],
                    comps[2],
                    comps[3],
                    comps[4],
                    is_bulk,
                    is_self
                ])
                .unwrap();
            ident
                .execute(params![id, format!("p{id}@perf.example")])
                .unwrap();
        }
        let mut ix = c
            .prepare(
                "INSERT INTO interaction(id, source_id, kind, at, subject, external_id, direction)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?1, ?6)",
            )
            .unwrap();
        let mut ip = c
            .prepare(
                "INSERT OR IGNORE INTO interaction_person
                    (interaction_id, identity_id, role, person_id, name_hint)
                 VALUES (?1, ?2, ?3, ?2, NULL)",
            )
            .unwrap();
        let six_years = 6.0 * 365.0 * DAY as f64;
        for id in 1..=p.interactions {
            let u = rng.unit();
            let who = 2 + ((u * u * u) * (p.people - 2) as f64) as i64;
            let at = NOW - (rng.unit() * rng.unit() * six_years) as i64;
            let topic = rng.pick(TOPICS);
            let subject = rng.pick(SUBJECT_FORMS).replace("{t}", topic);
            if rng.below(10) == 0 {
                ix.execute(params![id, 2, "meeting", at, subject, "mutual"])
                    .unwrap();
                ip.execute(params![id, 1, "organizer"]).unwrap();
                ip.execute(params![id, who, "attendee"]).unwrap();
            } else {
                let out = rng.below(2) == 0;
                let dir = if out { "out" } else { "in" };
                ix.execute(params![id, 1, "email", at, subject, dir])
                    .unwrap();
                let (me, them) = if out { ("from", "to") } else { ("to", "from") };
                ip.execute(params![id, 1, me]).unwrap();
                ip.execute(params![id, who, them]).unwrap();
                if rng.below(5) == 0 {
                    let cc = 2 + rng.below((p.people - 1) as usize) as i64;
                    ip.execute(params![id, cc, "cc"]).unwrap();
                }
            }
        }
    }
    c.execute_batch(
        "UPDATE person SET n_interactions =
           (SELECT COUNT(*) FROM interaction_person ip WHERE ip.person_id = person.id);
         COMMIT;",
    )
    .unwrap();
    c.execute_batch("BEGIN").unwrap();
    fts::reindex_all(c).unwrap();
    c.execute_batch("COMMIT").unwrap();
    let t = std::time::Instant::now();
    let r = g
        .embed(&IndexOptions {
            force: true,
            ..IndexOptions::default()
        })
        .expect("embedding pass");
    assert!(r.error.is_none(), "embedding failed: {:?}", r.error);
    // P2-MODEL-03 throughput (first-run embedding), printed for the record.
    eprintln!(
        "perf graph: embedded {} people with {} in {} ms",
        r.embedded,
        r.embedder,
        t.elapsed().as_millis()
    );
    g
}

fn request(text: Option<&str>, filters: QueryPatch) -> SearchRequest {
    SearchRequest {
        text: text.map(str::to_string),
        filters,
        no_parse: false,
        explain: false,
        today: today_start(NOW, chrono_tz::UTC),
        now: NOW,
    }
}

/// Free-text queries (FTS + vector fusion, including the query embedding).
pub fn text_queries() -> Vec<(String, SearchRequest)> {
    [
        "devtools dinner",
        "fundraising seed round",
        "kubernetes observability",
        "rust open source",
        "angel investing",
        "design review notes",
        "payments partnership",
        "machine learning researcher",
        "people I talked to about pricing",
        "climate conference",
    ]
    .iter()
    .map(|t| (t.replace(' ', "_"), request(Some(t), QueryPatch::default())))
    .collect()
}

/// Structured-only queries (no text: filters, score order, evidence).
pub fn structured_queries() -> Vec<(String, SearchRequest)> {
    let p = |f: fn(&mut QueryPatch)| {
        let mut q = QueryPatch::default();
        f(&mut q);
        q
    };
    vec![
        ("company", p(|q| q.company = Some(vec!["Northwind".into()]))),
        (
            "company_met",
            p(|q| {
                q.company = Some(vec!["Lumenlabs".into()]);
                q.met_in_person = Some(true);
            }),
        ),
        ("title", p(|q| q.title = Some(vec!["Founder".into()]))),
        ("location", p(|q| q.location = Some(vec!["Berlin".into()]))),
        (
            "title_location",
            p(|q| {
                q.title = Some(vec!["Partner".into()]);
                q.location = Some(vec!["London".into()]);
            }),
        ),
        ("met", p(|q| q.met_in_person = Some(true))),
        ("replied", p(|q| q.replied = Some(true))),
        ("since", p(|q| q.since = Some(NOW - 90 * DAY))),
        ("lost_touch", p(|q| q.lost_touch = Some(true))),
        ("min_score", p(|q| q.min_score = Some(0.7))),
    ]
    .into_iter()
    .map(|(id, f)| (id.to_string(), request(None, f)))
    .collect()
}
