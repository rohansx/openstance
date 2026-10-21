//! P2-DOC-01: the demo dataset under `fixtures/demo/` is committed, equals
//! the generator, and gives the clean, believable answers the 60-second demo
//! (docs/plan/phase-2.md) relies on when imported with nothing but
//! `openstance init --me you@example.com`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use openstance_core::ingest::ImportRequest;
use openstance_core::resolve::lookup::PersonKey;
use openstance_core::search::{timezone, today_start, SearchRequest};
use openstance_core::{config, FixedClock, Graph, Providers, QueryPatch};
use openstance_fixtures::demo;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
fn committed_demo_matches_the_generator() {
    let root = repo().join("demo");
    for (rel, want) in demo::files() {
        let got = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("fixtures/demo/{rel} missing ({e}); run gen-demo"));
        assert!(
            got == want,
            "fixtures/demo/{rel} drifted from the generator; run `cargo run -p openstance-fixtures --bin gen-demo`"
        );
    }
}

fn demo_graph() -> Graph {
    let cfg_text = format!("[me]\nidentities = [\"email:{}\"]\n", demo::OWNER_EMAIL);
    let (cfg, _) = config::parse_str(&cfg_text, true).unwrap();
    let tz = timezone(&cfg);
    let opts = openstance_import::ImportOptions {
        tz,
        ..openstance_import::ImportOptions::default()
    };
    let mut g = Graph::open_in_memory(
        cfg,
        Providers::with_importers(openstance_import::registry(&opts)),
        Box::new(FixedClock(demo::demo_now())),
    )
    .unwrap();
    let dir = repo().join("demo");
    let files: Vec<PathBuf> = demo::EXPORTS.iter().map(|f| dir.join(f)).collect();
    g.import(&files, &ImportRequest::default()).unwrap();
    g
}

fn search(g: &Graph, text: &str) -> Vec<String> {
    let now = demo::demo_now();
    let tz = timezone(g.config());
    g.search(&SearchRequest {
        text: Some(text.into()),
        filters: QueryPatch::default(),
        today: today_start(now, tz),
        now,
        ..SearchRequest::default()
    })
    .unwrap()
    .results
    .into_iter()
    .map(|r| r.person.display_name.unwrap_or_default())
    .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// One import of the three exports (shared by the assertions below, which
/// are one test so the graph is built once).
#[test]
fn demo_graph_answers_the_demo_questions() {
    let g = demo_graph();

    // 0:05-0:15: about 600 people, exactly the planned review queue.
    let c = g.counts().unwrap();
    assert!(
        (590..=640).contains(&c.persons_excluding_self),
        "people {}",
        c.persons_excluding_self
    );
    assert!(c.interactions >= 5_000, "interactions {}", c.interactions);
    let review = g.review().unwrap();
    let flagged: BTreeSet<String> = review
        .flagged
        .iter()
        .filter_map(|f| f.person.display_name.clone())
        .collect();
    assert_eq!(flagged, set(demo::FLAGGED), "flagged merges");
    assert!(
        review.suggestions.is_empty(),
        "unexpected suggestions: {:?}",
        review
            .suggestions
            .iter()
            .map(|s| (&s.a.display_name, &s.b.display_name))
            .collect::<Vec<_>>()
    );
    assert_eq!(c.bulk as usize, demo::NEWSLETTERS.len(), "bulk senders");

    // 0:15-0:27: fintech founders, warmest first.
    let fintech = search(&g, "fintech founders");
    let top: BTreeSet<String> = fintech.iter().take(5).cloned().collect();
    assert!(
        top.is_subset(&set(demo::FINTECH_FOUNDERS)),
        "fintech founders top5 {fintech:?}"
    );
    assert_eq!(
        top.len(),
        demo::FINTECH_FOUNDERS.len().min(5),
        "{fintech:?}"
    );
    assert_eq!(fintech[0], "Maya Chen", "{fintech:?}");

    // 0:27-0:35: Maya Chen is one person with a strong, explainable score.
    let maya = g.who(&PersonKey::Name("Maya Chen".into())).unwrap();
    assert!(maya.person.score >= 0.6, "Maya score {}", maya.person.score);
    let c = &maya.person.components;
    for (name, v) in [
        ("recency", c.recency),
        ("frequency", c.frequency),
        ("reciprocity", c.reciprocity),
        ("breadth", c.breadth),
    ] {
        assert!(v > 0.3, "Maya {name} {v}");
    }

    // 0:35-0:45: Dana Whitfield is in the graph, weakly, with three warm
    // mutuals behind her.
    let dana = g.who(&PersonKey::Name("Dana Whitfield".into())).unwrap();
    assert_eq!(dana.person.company.as_deref(), Some("Lumen Pay"));
    assert_eq!(dana.person.title.as_deref(), Some("CTO"));
    // Weaker than each mutual: the warm way in goes through them.
    for m in dana.mutuals.iter().take(3) {
        assert!(
            m.person.score > dana.person.score,
            "{:?} {} vs Dana {}",
            m.person.display_name,
            m.person.score,
            dana.person.score
        );
    }
    let mutuals: BTreeSet<String> = dana
        .mutuals
        .iter()
        .take(3)
        .filter_map(|m| m.person.display_name.clone())
        .collect();
    assert_eq!(mutuals, set(demo::DANA_MUTUALS), "Dana's top mutuals");

    // 0:45-0:57: who do I know at Lumen Pay.
    let lumen = search(&g, "who do I know at Lumen Pay");
    assert_eq!(
        lumen.iter().cloned().collect::<BTreeSet<_>>(),
        set(demo::LUMEN_PAY),
        "{lumen:?}"
    );

    // Lost touch: a strong relationship that went quiet.
    let lost = g.who(&PersonKey::Name("Hollis Grant".into())).unwrap();
    assert!(lost.person.components.recency < 0.2);

    // The Phase 2 semantic query has keyword-visible candidates too.
    let devtools = search(&g, "developer relations");
    assert!(
        devtools
            .iter()
            .take(5)
            .any(|n| demo::DEVTOOLS.contains(&n.as_str())),
        "{devtools:?}"
    );
}
