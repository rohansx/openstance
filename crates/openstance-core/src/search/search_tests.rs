//! `search()` end to end over the seeded test graph.

use openstance_model::{ModelError, Models, Query, QueryParser, QueryPatch};

use super::testkit::{ago, indexed, NOW};
use super::*;
use crate::json::{Evidence, SearchMode};

fn req(text: Option<&str>) -> SearchRequest {
    SearchRequest {
        text: text.map(str::to_string),
        today: NOW - 12 * 3600,
        now: NOW,
        ..SearchRequest::default()
    }
}

fn patch(f: impl FnOnce(&mut QueryPatch)) -> QueryPatch {
    let mut p = QueryPatch::default();
    f(&mut p);
    p
}

fn run(r: &SearchRequest) -> SearchResponse {
    search(&indexed(), &Config::default(), &Models::null(), r).unwrap()
}

fn ids(r: &SearchResponse) -> Vec<i64> {
    r.results.iter().map(|x| x.person.id).collect()
}

#[test]
fn structured_mode_orders_by_score_with_evidence_and_matches() {
    let r = run(&SearchRequest {
        filters: patch(|p| p.company = Some(vec!["contoso".into()])),
        ..req(None)
    });
    assert_eq!(r.v, 1);
    assert_eq!(r.mode, SearchMode::Structured);
    assert_eq!(ids(&r), [3, 7]);
    assert_eq!(r.total, 2);
    assert_eq!(r.results[0].rank, 1);
    assert!(r.results.iter().all(|x| x.relevance.is_none()));
    assert_eq!(r.results[0].evidence.len(), 3);
    assert!(matches!(
        &r.results[0].evidence[0],
        Evidence::Interaction {
            interaction_id: 107,
            ..
        }
    ));
    let m7 = &r.results[1].matched;
    assert_eq!(m7.len(), 1, "{m7:?}");
    assert_eq!(m7[0].via.as_deref(), Some("email domain contoso.example"));
    assert_eq!(
        r.results[0].matched[0].via.as_deref(),
        Some("person.company")
    );
    assert_eq!(r.query_source.explicit, ["company"]);
    assert_eq!(r.rerun, "openstance search --company contoso");
    assert!(r.timings_ms.is_none());
    assert!(r.warnings.is_empty());
}

#[test]
fn text_mode_fuses_bm25_with_the_score_lift() {
    let r = run(&req(Some("devtools")));
    assert_eq!(r.mode, SearchMode::Text);
    assert_eq!(ids(&r), [7, 3]);
    let rel = r.results[0].relevance.unwrap();
    assert_eq!(rel.fts, 1.0);
    assert_eq!(rel.vector, None);
    assert_eq!(rel.fused, 1.0);
    assert_eq!(rel.final_, 0.725, "1.0 x (0.5 + 0.5 x 0.45)");
    assert_eq!(r.results[1].relevance.unwrap().final_, 0.0);
    let subj = r.results[0]
        .matched
        .iter()
        .find(|m| m.field == "subjects")
        .expect("subjects match");
    assert_eq!(subj.matched.as_deref(), Some("devtools"));
    assert!(subj.snippet.as_deref().unwrap().contains("devtools dinner"));
    assert_eq!(
        r.query.text.as_deref(),
        Some("developer tools"),
        "v0.1 alias map"
    );
    assert_eq!(r.rerun, "openstance search \"developer tools\" --no-parse");
}

#[test]
fn zero_relevance_hits_are_dropped_when_enough_results_remain() {
    let r = run(&SearchRequest {
        filters: patch(|p| p.limit = Some(1)),
        ..req(Some("devtools"))
    });
    assert_eq!(ids(&r), [7]);
    assert_eq!(r.total, 1);
}

#[test]
fn text_candidates_are_restricted_to_the_structured_filter() {
    let r = run(&SearchRequest {
        filters: patch(|p| p.title = Some(vec!["engineer".into()])),
        ..req(Some("devtools"))
    });
    assert_eq!(ids(&r), [3]);
    assert_eq!(r.results[0].relevance.unwrap().fts, 1.0);
}

#[test]
fn no_text_hit_falls_back_to_filter_matches_by_score() {
    let r = run(&SearchRequest {
        filters: patch(|p| p.company = Some(vec!["contoso".into()])),
        ..req(Some("zeppelin"))
    });
    assert_eq!(ids(&r), [3, 7]);
    assert_eq!(r.mode, SearchMode::Structured);
    assert_eq!(
        r.warnings,
        ["no text matches; showing filter matches by score"]
    );
    let r = run(&req(Some("zeppelin")));
    assert!(r.results.is_empty());
}

#[test]
fn rules_parser_runs_by_default_and_flags_win() {
    let r = run(&req(Some("founders at Northwind I've met")));
    assert_eq!(r.query.title, ["founder"]);
    assert_eq!(r.query.company, ["northwind"]);
    assert_eq!(r.query.met_in_person, Some(true));
    assert_eq!(r.query.text, None);
    assert_eq!(r.query_source.parser, "rules");
    assert_eq!(ids(&r), [2]);
    assert_eq!(
        r.rerun,
        "openstance search --company northwind --title founder --met"
    );
    let met = r.results[0]
        .matched
        .iter()
        .find(|m| m.field == "met_in_person")
        .unwrap();
    assert_eq!(met.via.as_deref(), Some("1 meeting"));

    let r = run(&SearchRequest {
        filters: patch(|p| p.company = Some(vec!["northwind".into()])),
        ..req(Some("people at Contoso"))
    });
    assert_eq!(r.query.company, ["northwind"]);
    assert_eq!(r.query_source.explicit, ["company"]);
    assert_eq!(ids(&r), [2]);
}

#[test]
fn no_parse_sends_text_straight_to_fts() {
    let r = run(&SearchRequest {
        no_parse: true,
        ..req(Some("Board deck"))
    });
    assert_eq!(r.query.text.as_deref(), Some("Board deck"));
    assert_eq!(r.query_source.parser, "passthrough");
    let mut got = ids(&r);
    got.sort_unstable();
    assert_eq!(got, [2, 3]);
}

#[test]
fn bulk_people_are_excluded_unless_asked() {
    assert!(!ids(&run(&req(None))).contains(&4));
    let r = run(&SearchRequest {
        filters: patch(|p| p.include_bulk = Some(true)),
        ..req(None)
    });
    assert!(ids(&r).contains(&4));
    assert!(run(&req(Some("digest"))).results.is_empty());
}

#[test]
fn limit_caps_results_but_total_counts_all() {
    let r = run(&SearchRequest {
        filters: patch(|p| p.limit = Some(2)),
        ..req(None)
    });
    assert_eq!(ids(&r), [2, 3]);
    assert_eq!(r.total, 5);
}

#[test]
fn zero_results_suggest_which_filter_to_drop() {
    let r = run(&SearchRequest {
        filters: patch(|p| {
            p.company = Some(vec!["northwind".into()]);
            p.replied = Some(false);
        }),
        ..req(None)
    });
    assert!(r.results.is_empty());
    assert_eq!(
        r.suggestions,
        ["drop company -> 2 results", "drop replied -> 1 result"]
    );
}

#[test]
fn drop_suggestion_count_matches_the_rerun_total() {
    // "devtools" hits 7 (non-zero) and 3 (zero relevance). With limit 1 the
    // rerun without `company` keeps only 7, so the hint must say 1, not 2.
    let narrow = run(&SearchRequest {
        filters: patch(|p| {
            p.company = Some(vec!["no-such-company".into()]);
            p.limit = Some(1);
        }),
        ..req(Some("devtools"))
    });
    assert!(narrow.results.is_empty());
    let rerun = run(&SearchRequest {
        filters: patch(|p| p.limit = Some(1)),
        ..req(Some("devtools"))
    });
    assert_eq!(rerun.total, 1);
    assert_eq!(narrow.suggestions, ["drop company -> 1 result"]);
}

#[test]
fn explain_fills_timings() {
    let r = run(&SearchRequest {
        explain: true,
        ..req(Some("devtools"))
    });
    assert!(r.timings_ms.is_some());
}

#[test]
fn same_request_gives_the_same_results() {
    let a = run(&req(Some("devtools dinner")));
    let b = run(&req(Some("devtools dinner")));
    assert_eq!(a.results, b.results);
    assert_eq!(a.query, b.query);
}

struct FailingParser;
impl QueryParser for FailingParser {
    fn id(&self) -> &str {
        "ollama:test"
    }
    fn is_remote(&self) -> bool {
        false
    }
    fn parse(&self, _: &str, _: i64) -> openstance_model::Result<Query> {
        Err(ModelError::Unavailable("server down".into()))
    }
}

#[test]
fn model_parser_failure_falls_back_to_rules() {
    let mut cfg = Config::default();
    cfg.model.parser = "ollama:test".into();
    let models = Models {
        parser: Box::new(FailingParser),
        ..Models::null()
    };
    let r = search(&indexed(), &cfg, &models, &req(Some("at Northwind"))).unwrap();
    assert_eq!(r.query.company, ["northwind"]);
    assert_eq!(r.query_source.parser, "rules");
    assert_eq!(
        r.query_source.fallback.as_deref(),
        Some("model not available: server down")
    );
    assert_eq!(
        r.warnings,
        ["parser fallback: model not available: server down"]
    );
}

#[test]
fn rerun_command_reproduces_every_field() {
    let q = Query {
        text: Some("devtools dinner".into()),
        company: vec!["jane street".into(), "stripe".into()],
        title: vec!["founder".into()],
        location: vec!["san francisco".into()],
        industry: vec!["fintech".into()],
        tags: vec!["angels".into()],
        since: Some(ago(0) - (ago(0) % 86_400)),
        until: Some(1_767_225_600),
        last_contact_before: Some(1_704_067_200),
        met_in_person: Some(false),
        replied: Some(true),
        min_score: Some(0.4),
        lost_touch: Some(true),
        include_bulk: true,
        limit: 10,
    };
    assert_eq!(
        rerun_command(&q),
        "openstance search \"devtools dinner\" --company \"jane street\" --company stripe \
         --title founder --location \"san francisco\" --industry fintech --tag angels \
         --since 2026-10-20 --until 2026-01-01 --before 2024-01-01 --not-met --replied \
         --min-score 0.4 --lost-touch --include-bulk --limit 10 --no-parse"
    );
    assert_eq!(rerun_command(&Query::default()), "openstance search");
    assert_eq!(
        rerun_command(&Query::text("it's \"odd\" $HOME")),
        "openstance search 'it'\\''s \"odd\" $HOME' --no-parse"
    );
    let q = Query {
        since: Some(1_735_669_800),
        ..Query::default()
    };
    assert_eq!(rerun_command(&q), "openstance search --since @1735669800");
}
