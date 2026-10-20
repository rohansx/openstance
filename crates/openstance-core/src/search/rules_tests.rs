//! RuleParser phrase table (search.md section 4): one phrase per row, fixed
//! `today` (Tuesday 2026-10-20, UTC) and a fixed lexicon.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use openstance_model::{Query, QueryParser};

use super::*;

fn d(y: i32, m: u32, day: u32) -> i64 {
    NaiveDate::from_ymd_opt(y, m, day)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp()
}

fn today() -> i64 {
    d(2026, 10, 20)
}

fn set(xs: &[&str]) -> BTreeSet<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

fn parser() -> RuleParser {
    RuleParser::new(
        Lexicon {
            companies: set(&[
                "stripe",
                "google",
                "anthropic",
                "northwind",
                "contoso",
                "jane street",
                "notion",
            ]),
            locations: set(&["san francisco", "bengaluru", "berlin", "london", "new york"]),
            tags: set(&["angels", "investor-intro"]),
        },
        chrono_tz::UTC,
    )
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn q(f: impl FnOnce(&mut Query)) -> Query {
    let mut q = Query::default();
    f(&mut q);
    q
}

#[track_caller]
fn check(input: &str, expected: Query) {
    let got = parser().parse(input, today()).unwrap();
    assert_eq!(got, expected, "input: {input:?}");
}

#[test]
fn golden_fintech_founders_met_since_2025() {
    check(
        "fintech founders I've met since 2025",
        q(|q| {
            q.title = s(&["founder"]);
            q.industry = s(&["fintech"]);
            q.met_in_person = Some(true);
            q.since = Some(d(2025, 1, 1));
        }),
    );
}

/// The examples of search.md section 4.7.
#[test]
fn spec_examples() {
    check("who do I know at Stripe", q(|q| q.company = s(&["stripe"])));
    check(
        "fintech founders I've actually met",
        q(|q| {
            q.title = s(&["founder"]);
            q.industry = s(&["fintech"]);
            q.met_in_person = Some(true);
        }),
    );
    check(
        "fintech founders I met last year",
        q(|q| {
            q.title = s(&["founder"]);
            q.industry = s(&["fintech"]);
            q.met_in_person = Some(true);
            q.since = Some(d(2025, 1, 1));
            q.until = Some(d(2026, 1, 1));
        }),
    );
    check(
        "people I lost touch with who are now at Anthropic",
        q(|q| {
            q.company = s(&["anthropic"]);
            q.lost_touch = Some(true);
        }),
    );
    check(
        "ml researchers in sf",
        q(|q| {
            q.title = s(&["researcher"]);
            q.location = s(&["san francisco"]);
            q.text = Some("ml".into());
        }),
    );
    check(
        "devtools dinner",
        q(|q| q.text = Some("developer tools dinner".into())),
    );
    check(
        "investors who replied #angels limit:10",
        q(|q| {
            q.title = s(&["investor"]);
            q.replied = Some(true);
            q.tags = s(&["angels"]);
            q.limit = 10;
        }),
    );
    check(
        "haven't talked to since 2024 at google",
        q(|q| {
            q.company = s(&["google"]);
            q.last_contact_before = Some(d(2024, 1, 1));
        }),
    );
}

#[test]
fn explicit_field_syntax() {
    let rows: Vec<(&str, Query)> = vec![
        (
            "company:stripe met",
            q(|q| {
                q.company = s(&["stripe"]);
                q.met_in_person = Some(true);
            }),
        ),
        (
            "company:\"jane street\"",
            q(|q| q.company = s(&["jane street"])),
        ),
        ("at:stripe", q(|q| q.company = s(&["stripe"]))),
        (
            "title:founder location:sf",
            q(|q| {
                q.title = s(&["founder"]);
                q.location = s(&["san francisco"]);
            }),
        ),
        (
            "role:\"product manager\"",
            q(|q| q.title = s(&["product manager"])),
        ),
        (
            "in:\"new york\" industry:fintech",
            q(|q| {
                q.location = s(&["new york"]);
                q.industry = s(&["fintech"]);
            }),
        ),
        ("tag:investor", q(|q| q.tags = s(&["investor"]))),
        ("#Angels", q(|q| q.tags = s(&["angels"]))),
        (
            "since:2025-01 until:2026-06",
            q(|q| {
                q.since = Some(d(2025, 1, 1));
                q.until = Some(d(2026, 6, 1));
            }),
        ),
        ("since:2025-01-15", q(|q| q.since = Some(d(2025, 1, 15)))),
        (
            "before:2024",
            q(|q| q.last_contact_before = Some(d(2024, 1, 1))),
        ),
        ("met:yes", q(|q| q.met_in_person = Some(true))),
        ("met:no", q(|q| q.met_in_person = Some(false))),
        ("replied", q(|q| q.replied = Some(true))),
        ("replied:no", q(|q| q.replied = Some(false))),
        ("lost-touch", q(|q| q.lost_touch = Some(true))),
        ("min-score:0.5", q(|q| q.min_score = Some(0.5))),
        ("limit:500", q(|q| q.limit = 200)),
        ("bulk:yes", q(|q| q.include_bulk = true)),
    ];
    for (input, expected) in rows {
        check(input, expected);
    }
}

#[test]
fn explicit_syntax_wins_over_phrases() {
    check(
        "people at Google company:stripe",
        q(|q| q.company = s(&["stripe"])),
    );
    check("never met met:yes", q(|q| q.met_in_person = Some(true)));
}

#[test]
fn date_phrases() {
    let rows: Vec<(&str, Option<i64>, Option<i64>)> = vec![
        ("today", Some(d(2026, 10, 20)), Some(d(2026, 10, 21))),
        ("yesterday", Some(d(2026, 10, 19)), Some(d(2026, 10, 20))),
        ("this week", Some(d(2026, 10, 19)), None),
        ("last week", Some(d(2026, 10, 12)), Some(d(2026, 10, 19))),
        ("this month", Some(d(2026, 10, 1)), None),
        ("last month", Some(d(2026, 9, 1)), Some(d(2026, 10, 1))),
        ("this year", Some(d(2026, 1, 1)), None),
        ("last year", Some(d(2025, 1, 1)), Some(d(2026, 1, 1))),
        ("in 2025", Some(d(2025, 1, 1)), Some(d(2026, 1, 1))),
        ("during 2024", Some(d(2024, 1, 1)), Some(d(2025, 1, 1))),
        ("in march", Some(d(2026, 3, 1)), Some(d(2026, 4, 1))),
        ("in november", Some(d(2025, 11, 1)), Some(d(2025, 12, 1))),
        ("in march 2025", Some(d(2025, 3, 1)), Some(d(2025, 4, 1))),
        ("in december", Some(d(2025, 12, 1)), Some(d(2026, 1, 1))),
        ("since march", Some(d(2026, 3, 1)), None),
        ("since 2025-03", Some(d(2025, 3, 1)), None),
        ("since last year", Some(d(2025, 1, 1)), None),
        ("after 2025", Some(d(2025, 1, 1)), None),
        ("before 2024", None, Some(d(2024, 1, 1))),
        ("until 2024-06", None, Some(d(2024, 6, 1))),
        ("last 30 days", Some(d(2026, 9, 20)), None),
        ("past 2 weeks", Some(d(2026, 10, 6)), None),
        ("in the last 6 months", Some(d(2026, 4, 20)), None),
        ("last two years", Some(d(2024, 10, 20)), None),
        ("over the past twelve months", Some(d(2025, 10, 20)), None),
        ("recently", Some(d(2026, 7, 22)), None),
        ("lately", Some(d(2026, 7, 22)), None),
    ];
    for (input, since, until) in rows {
        check(
            input,
            q(|q| {
                q.since = since;
                q.until = until;
            }),
        );
    }
}

#[test]
fn not_since_sets_last_contact_before() {
    check(
        "not since 2025",
        q(|q| q.last_contact_before = Some(d(2025, 1, 1))),
    );
    check(
        "haven't spoken to since march",
        q(|q| q.last_contact_before = Some(d(2026, 3, 1))),
    );
}

#[test]
fn unparseable_date_stays_text_with_warning() {
    let out = parser().parse_detailed("since Q5", today());
    assert_eq!(out.query.text.as_deref(), Some("q5"));
    assert_eq!(out.query.since, None);
    assert_eq!(out.warnings, vec!["could not parse date: 'Q5'".to_string()]);
}

#[test]
fn flag_phrases() {
    let met = |v: bool| q(move |q| q.met_in_person = Some(v));
    let replied = |v: bool| q(move |q| q.replied = Some(v));
    let lost = || q(|q| q.lost_touch = Some(true));
    let rows: Vec<(&str, Query)> = vec![
        ("met", met(true)),
        ("people I've met", met(true)),
        ("people I have met", met(true)),
        ("met in person", met(true)),
        ("in person", met(true)),
        ("had a meeting with", met(true)),
        ("never met", met(false)),
        ("people I haven't met", met(false)),
        ("not met", met(false)),
        ("who replied", replied(true)),
        ("responded", replied(true)),
        ("wrote back", replied(true)),
        ("people I actually talk to", replied(true)),
        ("never replied", replied(false)),
        ("didn't reply", replied(false)),
        ("ghosted", replied(false)),
        ("no reply", replied(false)),
        ("lost touch", lost()),
        ("drifted", lost()),
        ("haven't talked to in a while", lost()),
        ("fell out of touch", lost()),
        ("should reconnect", lost()),
        ("know well", q(|q| q.min_score = Some(0.4))),
        ("warm", q(|q| q.min_score = Some(0.4))),
        (
            "top 5 founders",
            q(|q| {
                q.limit = 5;
                q.title = s(&["founder"]);
            }),
        ),
        (
            "10 people at Google",
            q(|q| {
                q.limit = 10;
                q.company = s(&["google"]);
            }),
        ),
        ("first three", q(|q| q.limit = 3)),
        ("top 500", q(|q| q.limit = 200)),
    ];
    for (input, expected) in rows {
        check(input, expected);
    }
}

#[test]
fn entity_phrases() {
    let rows: Vec<(&str, Query)> = vec![
        (
            "close friends at Stripe",
            q(|q| {
                q.min_score = Some(0.4);
                q.company = s(&["stripe"]);
                q.text = Some("friends".into());
            }),
        ),
        ("people in fintech", q(|q| q.industry = s(&["fintech"]))),
        (
            "engineers based in Berlin",
            q(|q| {
                q.title = s(&["engineer"]);
                q.location = s(&["berlin"]);
            }),
        ),
        (
            "vcs from London",
            q(|q| {
                q.title = s(&["investor"]);
                q.location = s(&["london"]);
            }),
        ),
        (
            "people from Northwind",
            q(|q| q.company = s(&["northwind"])),
        ),
        ("@stripe", q(|q| q.company = s(&["stripe"]))),
        (
            "who knows someone at Contoso",
            q(|q| q.company = s(&["contoso"])),
        ),
        ("@ contoso", q(|q| q.company = s(&["contoso"]))),
        (
            "works at Acme Robotics",
            q(|q| q.company = s(&["acme robotics"])),
        ),
        ("at jane street", q(|q| q.company = s(&["jane street"]))),
        (
            "at the conference",
            q(|q| q.text = Some("conference".into())),
        ),
        ("notion", q(|q| q.text = Some("notion".into()))),
        ("Northwind people", q(|q| q.company = s(&["northwind"]))),
        (
            "cto at a startup",
            q(|q| {
                q.title = s(&["cto"]);
                q.text = Some("startup".into());
            }),
        ),
        (
            "co-founders in bengaluru",
            q(|q| {
                q.title = s(&["founder"]);
                q.location = s(&["bengaluru"]);
            }),
        ),
        (
            "designers near bangalore",
            q(|q| {
                q.title = s(&["designer"]);
                q.location = s(&["bengaluru"]);
            }),
        ),
        ("head of sales", q(|q| q.title = s(&["head of", "sales"]))),
        (
            "vice president of product at Google",
            q(|q| {
                q.title = s(&["vp"]);
                q.company = s(&["google"]);
                q.text = Some("product".into());
            }),
        ),
        ("investor-intro", q(|q| q.tags = s(&["investor-intro"]))),
        ("crypto and web3 people", q(|q| q.industry = s(&["crypto"]))),
        ("Priya Raman", q(|q| q.text = Some("priya raman".into()))),
    ];
    for (input, expected) in rows {
        check(input, expected);
    }
}

#[test]
fn empty_and_stopword_only_inputs_give_no_text() {
    check("", Query::default());
    check("   ", Query::default());
    check("the people who", Query::default());
}

#[test]
fn hostile_input_never_fails() {
    for input in [
        "\"quoted ) OR * NEAR(",
        "company:",
        "company:\"unterminated",
        "since:",
        "limit:abc",
        "min-score:-3",
        "#",
        "@",
        "at",
        "in",
        "from",
        "top",
        "last",
        "since",
        "💥 émoji ñ 名前",
    ] {
        let out = parser().parse(input, today());
        assert!(out.is_ok(), "{input:?}");
    }
}

#[test]
fn parser_identity() {
    let p = parser();
    assert_eq!(p.id(), "rules");
    assert!(!p.is_remote());
}

#[test]
fn date_args_accept_cli_forms() {
    let tz = chrono_tz::UTC;
    let t = today();
    assert_eq!(parse_date_arg("2025", t, tz), Some(d(2025, 1, 1)));
    assert_eq!(parse_date_arg("2025-03", t, tz), Some(d(2025, 3, 1)));
    assert_eq!(parse_date_arg("2025-03-09", t, tz), Some(d(2025, 3, 9)));
    assert_eq!(parse_date_arg("90d", t, tz), Some(d(2026, 7, 22)));
    assert_eq!(parse_date_arg("2w", t, tz), Some(d(2026, 10, 6)));
    assert_eq!(parse_date_arg("6m", t, tz), Some(d(2026, 4, 20)));
    assert_eq!(parse_date_arg("1y", t, tz), Some(d(2025, 10, 20)));
    assert_eq!(parse_date_arg("Q5", t, tz), None);
    assert_eq!(parse_date_arg("2025-13", t, tz), None);
}

#[test]
fn local_timezone_decides_day_boundaries() {
    let tz: chrono_tz::Tz = "Asia/Kolkata".parse().unwrap();
    let p = RuleParser::new(Lexicon::default(), tz);
    // 2026-10-20 00:00 IST = 2026-10-19 18:30 UTC.
    let today_ist = d(2026, 10, 20) - 5 * 3600 - 1800;
    let got = p.parse("since 2025", today_ist).unwrap();
    assert_eq!(got.since, Some(d(2025, 1, 1) - 5 * 3600 - 1800));
}

#[test]
fn lexicon_loads_companies_locations_and_tags_from_graph() {
    let mut c = crate::db::open_in_memory().unwrap();
    crate::db::migrate(&mut c, None, 0).unwrap();
    c.execute_batch(
        "INSERT INTO source(id, kind, imported_at) VALUES (1, 'mbox', 0);
         INSERT INTO person(id, company, location) VALUES
           (1, 'Northwind Traders Inc.', 'San Francisco, CA'),
           (2, 'The Contoso Group', NULL),
           (3, NULL, 'Berlin');
         INSERT INTO identity(id, kind, value, source_id, person_id) VALUES
           (1, 'email', 'a@fabrikam.example', 1, 1),
           (2, 'email', 'b@gmail.com', 1, 2),
           (3, 'email', 'c@mail.tailspin.example', 1, 3);
         INSERT INTO override(person_id, field, value, set_at) VALUES
           (1, 'tag', 'Investor-Intro', 1), (2, 'note', 'not a tag', 1);",
    )
    .unwrap();
    let lex = Lexicon::load(&c).unwrap();
    assert!(lex.companies.contains("northwind traders"), "{lex:?}");
    assert!(lex.companies.contains("contoso group"), "{lex:?}");
    assert!(lex.companies.contains("fabrikam"), "{lex:?}");
    assert!(lex.companies.contains("tailspin"), "{lex:?}");
    assert!(!lex.companies.contains("gmail"), "{lex:?}");
    assert!(lex.locations.contains("san francisco"), "{lex:?}");
    assert!(lex.locations.contains("berlin"), "{lex:?}");
    assert_eq!(lex.tags, set(&["investor-intro"]));
}

mod never_fails {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn any_string_parses(input in "\\PC{0,60}") {
            let out = parser().parse(&input, today());
            prop_assert!(out.is_ok());
            let q = out.unwrap();
            prop_assert!(q.limit >= 1 && q.limit <= 200);
        }
    }
}
