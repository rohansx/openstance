//! P1-CORE-11 golden test: 10 hand-computed people (testing-strategy.md,
//! "Golden tests: scoring"), seeded as raw rows, scored by
//! `score::recompute_all`, asserted to 1e-6 for every component and for S
//! under all five presets.
//!
//! The expected numbers were computed independently of the Rust code from
//! the formulas in scoring.md section 2 (Python, `exp`/`log` in f64):
//!
//! ```text
//! R = exp(-ln2 * days / 180)  F = 1 - exp(-n / 20)  C = 0 if out == 0 else min(1, (replies + 2 * starts) / out)
//! M = 1 - exp(-meetings / 3)  B = channels / 4      S = sum(w * c)
//! ```
//!
//! Changing a formula, weight or default must update this table in the same
//! change.

// The table holds hand-computed decimals; 2^-0.5 appears as a literal.
#![allow(clippy::approx_constant)]

mod score_support;

use openstance_core::config::ScoreSection;
use openstance_core::score::{self, presets, Channel, ScoreConfig};
use score_support::{Seed, DAY, NOW};

/// `(key, [R, F, C, M, B], [S default, founder, recruiter, seller, community])`.
const GOLDEN: &[(&str, [f64; 5], [f64; 5])] = &[
    (
        "maya_fresh_heavy",
        [
            0.9622238369,
            0.7768698399,
            1.0000000000,
            0.4865828810,
            0.7500000000,
        ],
        [
            0.8526397430,
            0.8400142032,
            0.8408105990,
            0.8162321554,
            0.8287397996,
        ],
    ),
    (
        "olga_two_years",
        [
            0.0601389898,
            0.3296799540,
            1.0000000000,
            0.0000000000,
            0.2500000000,
        ],
        [
            0.3494846372,
            0.3799936900,
            0.2869846372,
            0.3769867405,
            0.4158928301,
        ],
    ),
    (
        "mo_meet_only",
        [
            0.8572439829,
            0.1392920236,
            0.0000000000,
            0.6321205588,
            0.2500000000,
        ],
        [
            0.4352118825,
            0.4169911102,
            0.4411058546,
            0.4057349390,
            0.2725154627,
        ],
    ),
    (
        "cole_one_way",
        [
            0.9809300877,
            0.2591817793,
            0.0000000000,
            0.0000000000,
            0.2500000000,
        ],
        [
            0.4076618865,
            0.3456562932,
            0.4451618865,
            0.2966097888,
            0.2758122249,
        ],
    ),
    (
        "tara_thread_starter",
        [
            0.7937005260,
            0.2591817793,
            0.7500000000,
            0.0000000000,
            0.2500000000,
        ],
        [
            0.5296315400,
            0.5144874247,
            0.4921315400,
            0.4748023984,
            0.4352277906,
        ],
    ),
    (
        "lin_linkedin_only",
        [
            0.9258747123,
            0.0951625820,
            0.0000000000,
            0.0000000000,
            0.2500000000,
        ],
        [
            0.3555886657,
            0.3045368010,
            0.3930886657,
            0.2582430654,
            0.2019462396,
        ],
    ),
    (
        "max_multi_channel",
        [
            0.9885140204,
            0.2211992169,
            1.0000000000,
            0.2834686894,
            1.0000000000,
        ],
        [
            0.7327400539,
            0.7364278265,
            0.7685666195,
            0.7011755600,
            0.6151036588,
        ],
    ),
    (
        "otto_override",
        [
            0.7071067812,
            0.1812692469,
            1.0000000000,
            0.0000000000,
            0.5000000000,
        ],
        [
            0.5587412228,
            0.5643224214,
            0.5337412228,
            0.5289670823,
            0.4785737159,
        ],
    ),
    (
        "hal_half_life",
        [
            0.5000000000,
            0.0951625820,
            1.0000000000,
            0.0000000000,
            0.2500000000,
        ],
        [
            0.4565325164,
            0.4767743873,
            0.3940325164,
            0.4517743873,
            0.3880650328,
        ],
    ),
    ("bea_bulk", [0.0; 5], [0.0; 5]),
];

/// Expected raw inputs: `(key, days_since_last, n, out, replies, starts, meetings, channels)`.
#[allow(clippy::type_complexity)]
const INPUTS: &[(&str, Option<f64>, u32, u32, u32, u32, u32, usize)] = &[
    ("maya_fresh_heavy", Some(10.0), 30, 12, 12, 3, 2, 3),
    ("olga_two_years", Some(730.0), 8, 4, 4, 0, 0, 1),
    ("mo_meet_only", Some(40.0), 3, 0, 0, 0, 3, 1),
    ("cole_one_way", Some(5.0), 6, 0, 0, 6, 0, 1),
    ("tara_thread_starter", Some(60.0), 6, 4, 1, 1, 0, 1),
    ("lin_linkedin_only", Some(20.0), 2, 1, 0, 0, 0, 1),
    ("max_multi_channel", Some(3.0), 5, 1, 1, 1, 1, 4),
    ("otto_override", Some(90.0), 4, 2, 2, 0, 0, 2),
    ("hal_half_life", Some(180.0), 2, 1, 1, 0, 0, 1),
    ("bea_bulk", Some(1.0), 55, 0, 0, 55, 0, 1),
];

const PRESETS: [&str; 5] = ["default", "founder", "recruiter", "seller", "community"];

struct Golden {
    seed: Seed,
    owner: i64,
    ids: Vec<(&'static str, i64)>,
}

impl Golden {
    fn id(&self, key: &str) -> i64 {
        self.ids.iter().find(|(k, _)| *k == key).unwrap().1
    }
}

fn seed() -> Golden {
    let mut s = Seed::new();
    let owner = s.owner("owner@example.com");
    let mut ids = Vec::new();

    // 1. Fresh heavy correspondent: 12 threads I started that they answered,
    //    3 threads they started, 2 meetings, a LinkedIn connection.
    let maya = s.person("maya@northwind.example");
    for k in 0..12 {
        let t = format!("maya-{k}");
        s.email_out(owner, maya, NOW - (10 + k) * DAY - 3600, &t);
        s.email_in(owner, maya, NOW - (10 + k) * DAY, &t);
    }
    for j in 0..3 {
        s.email_in(
            owner,
            maya,
            NOW - (30 + j) * DAY,
            &format!("maya-start-{j}"),
        );
    }
    s.meeting(owner, &[maya], NOW - 15 * DAY);
    s.meeting(owner, &[maya], NOW - 25 * DAY);
    s.interaction(
        "linkedin_csv",
        "connection",
        NOW - 200 * DAY,
        "none",
        None,
        false,
        None,
        &[(maya, "subject")],
    );
    ids.push(("maya_fresh_heavy", maya));

    // 2. Two-year-old contact.
    let olga = s.person("olga@contoso.example");
    for k in 0..4 {
        let t = format!("olga-{k}");
        s.email_out(owner, olga, NOW - (730 + k) * DAY - 3600, &t);
        s.email_in(owner, olga, NOW - (730 + k) * DAY, &t);
    }
    ids.push(("olga_two_years", olga));

    // 3. Meet-only contact; a future meeting is ignored.
    let mo = s.person("mo@fabrikam.example");
    for d in [40, 60, 80] {
        s.meeting(owner, &[mo], NOW - d * DAY);
    }
    s.meeting(owner, &[mo], NOW + 10 * DAY);
    ids.push(("mo_meet_only", mo));

    // 4. One-way cold sender: six threads they started, I never wrote.
    let cole = s.person("cole@recruit.example");
    for k in 0..6 {
        s.email_in(owner, cole, NOW - (5 + 7 * k) * DAY, &format!("cole-{k}"));
    }
    ids.push(("cole_one_way", cole));

    // 5. Thread starter: 4 outbound, 1 reply, 1 thread they started (x2).
    let tara = s.person("tara@tailspin.example");
    s.email_out(owner, tara, NOW - 100 * DAY, "tara-1");
    s.email_in(owner, tara, NOW - 99 * DAY, "tara-1");
    s.email_out(owner, tara, NOW - 90 * DAY, "tara-2");
    s.email_out(owner, tara, NOW - 80 * DAY, "tara-3");
    s.email_out(owner, tara, NOW - 70 * DAY, "tara-4");
    s.email_in(owner, tara, NOW - 60 * DAY, "tara-5");
    ids.push(("tara_thread_starter", tara));

    // 6. LinkedIn-only: a connection and one DM I sent.
    let lin = s.person("lin@woodgrove.example");
    s.interaction(
        "linkedin_csv",
        "connection",
        NOW - 300 * DAY,
        "none",
        None,
        false,
        None,
        &[(lin, "subject")],
    );
    s.interaction(
        "linkedin_csv",
        "dm",
        NOW - 20 * DAY,
        "out",
        Some("li-conv-1"),
        false,
        None,
        &[(owner, "sender"), (lin, "recipient")],
    );
    ids.push(("lin_linkedin_only", lin));

    // 7. Multi-channel (B = 1): email, calendar, LinkedIn and an X DM they
    //    started; a vCard contact row counts for nothing.
    let max = s.person("max@litware.example");
    s.email_out(owner, max, NOW - 100 * DAY, "max-1");
    s.email_in(owner, max, NOW - 99 * DAY, "max-1");
    s.meeting(owner, &[max], NOW - 50 * DAY);
    s.interaction(
        "linkedin_csv",
        "connection",
        NOW - 400 * DAY,
        "none",
        None,
        false,
        None,
        &[(max, "subject")],
    );
    s.interaction(
        "x_archive",
        "dm",
        NOW - 3 * DAY,
        "in",
        Some("x-conv-1"),
        false,
        None,
        &[(max, "sender"), (owner, "recipient")],
    );
    s.interaction(
        "vcf",
        "contact",
        NOW - DAY,
        "none",
        None,
        false,
        None,
        &[(max, "subject")],
    );
    ids.push(("max_multi_channel", max));

    // 8. Override that changes nothing in the score, plus a weak `follow`
    //    that adds a channel but moves neither R nor F.
    let otto = s.person("otto@adatum.example");
    for k in 0..2 {
        let t = format!("otto-{k}");
        s.email_out(owner, otto, NOW - (90 + k) * DAY - 3600, &t);
        s.email_in(owner, otto, NOW - (90 + k) * DAY, &t);
    }
    s.interaction(
        "x_archive",
        "follow",
        NOW - DAY,
        "out",
        None,
        false,
        None,
        &[(otto, "subject")],
    );
    s.add_override(otto, "title", Some("Staff Engineer"), NOW - DAY);
    ids.push(("otto_override", otto));

    // 9. Exactly 180 days: R = 0.5.
    let hal = s.person("hal@proseware.example");
    s.email_out(owner, hal, NOW - 200 * DAY, "hal-1");
    s.email_in(owner, hal, NOW - 180 * DAY, "hal-1");
    ids.push(("hal_half_life", hal));

    // 10. Bulk sender: 55 list messages, never answered.
    let bea = s.person("bea@lists.example");
    for k in 0..55 {
        s.interaction(
            "mbox",
            "email",
            NOW - (1 + k) * DAY,
            "in",
            Some(&format!("bea-{k}")),
            true,
            Some("weekly digest"),
            &[(bea, "from"), (owner, "to")],
        );
    }
    ids.push(("bea_bulk", bea));

    Golden {
        seed: s,
        owner,
        ids,
    }
}

fn cfg_for(preset: &str) -> ScoreConfig {
    ScoreConfig::from_config(&ScoreSection {
        preset: Some(preset.to_string()),
        ..ScoreSection::default()
    })
    .unwrap()
    .0
}

fn assert_close(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() <= 1e-6,
        "{what}: got {got:.10}, want {want:.10}"
    );
}

#[test]
fn score_golden_inputs_match_hand_counts() {
    let g = seed();
    for (key, days, n, out, replies, starts, meetings, channels) in INPUTS {
        let i = score::inputs(&g.seed.conn, g.id(key), NOW).unwrap();
        match (i.days_since_last, days) {
            (Some(a), Some(b)) => assert_close(a, *b, &format!("{key} days")),
            (a, b) => assert_eq!(a, *b, "{key} days"),
        }
        assert_eq!(i.n_interactions, *n, "{key} n_interactions");
        assert_eq!(i.n_out, *out, "{key} n_out");
        assert_eq!(i.n_replies, *replies, "{key} n_replies");
        assert_eq!(i.n_threads_started, *starts, "{key} n_threads_started");
        assert_eq!(i.n_meetings, *meetings, "{key} n_meetings");
        assert_eq!(
            i.channels.len(),
            *channels,
            "{key} channels {:?}",
            i.channels
        );
    }
    let max = score::inputs(&g.seed.conn, g.id("max_multi_channel"), NOW).unwrap();
    assert_eq!(
        max.channels,
        vec![
            Channel::Email,
            Channel::Calendar,
            Channel::Linkedin,
            Channel::Social
        ]
    );
}

#[test]
fn score_golden_default_components_and_s_to_1e6() {
    let g = seed();
    let cfg = cfg_for("default");
    let report = score::recompute_all(&g.seed.conn, &cfg, NOW).unwrap();
    assert_eq!(report.scored, 10, "self is not scored");
    assert_eq!(report.bulk, 1);
    for (key, comp, s) in GOLDEN {
        let [score, r, f, c, m, b] = g.seed.stored(g.id(key));
        for (got, want, name) in [
            (r, comp[0], "R"),
            (f, comp[1], "F"),
            (c, comp[2], "C"),
            (m, comp[3], "M"),
            (b, comp[4], "B"),
            (score, s[0], "S"),
        ] {
            assert_close(got, want, &format!("{key} {name}"));
        }
    }
    assert!(g.seed.is_bulk(g.id("bea_bulk")));
    assert_eq!(
        g.seed.stored(g.owner),
        [0.0; 6],
        "self person is never scored"
    );
    assert_eq!(g.seed.meta("scored_at").as_deref(), Some(&*NOW.to_string()));
    assert_eq!(g.seed.meta("score_config_hash"), Some(cfg.hash()));
}

#[test]
fn score_golden_every_preset_to_1e6() {
    let g = seed();
    for (pi, preset) in PRESETS.iter().enumerate() {
        score::recompute_all(&g.seed.conn, &cfg_for(preset), NOW).unwrap();
        for (key, _, s) in GOLDEN {
            let got = g.seed.stored(g.id(key))[0];
            assert_close(got, s[pi], &format!("{key} S under {preset}"));
        }
    }
}

#[test]
fn score_r_is_exactly_half_at_180_days() {
    let g = seed();
    score::recompute_all(&g.seed.conn, &cfg_for("default"), NOW).unwrap();
    let r = g.seed.stored(g.id("hal_half_life"))[1];
    assert!((r - 0.5).abs() < 1e-12, "{r}");
}

#[test]
fn score_preset_weights_are_pinned() {
    // Changing a preset without updating this golden file fails.
    let pinned: [[f64; 5]; 5] = [
        [0.35, 0.20, 0.25, 0.15, 0.05],
        [0.30, 0.15, 0.30, 0.20, 0.05],
        [0.35, 0.20, 0.15, 0.10, 0.20],
        [0.25, 0.15, 0.30, 0.25, 0.05],
        [0.15, 0.40, 0.25, 0.10, 0.10],
    ];
    for (name, want) in PRESETS.iter().zip(pinned) {
        let w = presets::preset(name).unwrap();
        assert_eq!(
            [w.recency, w.frequency, w.reciprocity, w.meetings, w.breadth],
            want,
            "{name}"
        );
    }
}

#[test]
fn score_stored_components_recombine_to_stored_score() {
    let g = seed();
    let cfg = cfg_for("seller");
    score::recompute_all(&g.seed.conn, &cfg, NOW).unwrap();
    for (key, _, _) in GOLDEN {
        let d = score::detail(&g.seed.conn, &cfg, g.id(key), NOW).unwrap();
        let w = &d.weights;
        let s = w.recency * d.recency
            + w.frequency * d.frequency
            + w.reciprocity * d.reciprocity
            + w.meetings * d.meetings
            + w.breadth * d.breadth;
        assert_close(s, g.seed.stored(g.id(key))[0], key);
        assert_eq!(d.preset, "seller");
        assert_eq!(d.half_life_days, 180.0);
        assert!(d.raw.is_some());
    }
}

#[test]
fn score_detail_fills_raw_counts() {
    let g = seed();
    let cfg = cfg_for("default");
    score::recompute_all(&g.seed.conn, &cfg, NOW).unwrap();
    let d = score::detail(&g.seed.conn, &cfg, g.id("tara_thread_starter"), NOW).unwrap();
    let raw = d.raw.unwrap();
    assert_eq!(raw.messages_from_you, 4);
    assert_eq!(raw.replies_from_them, 1);
    assert_eq!(raw.threads_they_started, 1);
    assert_eq!(raw.n_interactions, 6);
    assert_eq!(raw.channels, vec!["email".to_string()]);
    assert_close(raw.days_since_last.unwrap(), 60.0, "days");
    assert!(score::detail(&g.seed.conn, &cfg, 999_999, NOW).is_err());
}

#[test]
fn score_explicit_weights_are_custom() {
    let g = seed();
    let (cfg, _) = ScoreConfig::from_config(&ScoreSection {
        weights: Some(openstance_core::config::WeightsInput {
            recency: 1.0,
            frequency: 0.0,
            reciprocity: 0.0,
            meetings: 0.0,
            breadth: 0.0,
        }),
        ..ScoreSection::default()
    })
    .unwrap();
    score::recompute_all(&g.seed.conn, &cfg, NOW).unwrap();
    let [s, r, ..] = g.seed.stored(g.id("hal_half_life"));
    assert_close(s, 0.5, "S = R under recency-only weights");
    assert_close(r, 0.5, "R");
    let d = score::detail(&g.seed.conn, &cfg, g.id("hal_half_life"), NOW).unwrap();
    assert_eq!(d.preset, "custom");
}

#[test]
fn score_refresh_recency_updates_r_and_s_only() {
    let g = seed();
    let cfg = cfg_for("default");
    score::recompute_all(&g.seed.conn, &cfg, NOW).unwrap();
    let later = NOW + 30 * DAY;
    let n = score::refresh_recency(&g.seed.conn, &cfg, later).unwrap();
    assert_eq!(n, 9, "non-bulk, non-self people");
    let [s, r, f, c, m, b] = g.seed.stored(g.id("hal_half_life"));
    assert_close(r, 2f64.powf(-210.0 / 180.0), "R after 30 more days");
    assert_close(f, 0.0951625820, "F unchanged");
    assert_close(c, 1.0, "C unchanged");
    assert_close(s, 0.35 * r + 0.20 * f + 0.25 * c + 0.15 * m + 0.05 * b, "S");
    // A full rescore at the later time gives the same numbers, except for
    // Mo, whose future meeting (NOW + 10 days) is in the past by then and so
    // legitimately moves F and M (only a full rescore picks that up).
    let refreshed: Vec<[f64; 6]> = GOLDEN
        .iter()
        .map(|(k, _, _)| g.seed.stored(g.id(k)))
        .collect();
    score::recompute_all(&g.seed.conn, &cfg, later).unwrap();
    for ((k, _, _), before) in GOLDEN.iter().zip(refreshed) {
        if *k == "mo_meet_only" {
            assert!(g.seed.stored(g.id(k))[4] > before[4], "4 past meetings now");
            continue;
        }
        let after = g.seed.stored(g.id(k));
        for (x, y) in before.iter().zip(after) {
            assert_close(*x, y, k);
        }
    }
    assert_eq!(g.seed.meta("scored_at"), Some(later.to_string()));
    assert_eq!(g.seed.stored(g.id("bea_bulk")), [0.0; 6], "bulk stays 0");
}

#[test]
fn score_recompute_people_touches_only_those_people() {
    let g = seed();
    let cfg = cfg_for("default");
    let report = score::recompute_people(
        &g.seed.conn,
        &cfg,
        &[g.id("hal_half_life"), g.id("bea_bulk")],
        NOW,
    )
    .unwrap();
    assert_eq!(report.scored, 2);
    assert_eq!(report.bulk, 1);
    assert_close(
        g.seed.stored(g.id("hal_half_life"))[0],
        0.4565325164,
        "hal S",
    );
    assert_eq!(
        g.seed.stored(g.id("maya_fresh_heavy")),
        [0.0; 6],
        "untouched"
    );
    assert!(g.seed.is_bulk(g.id("bea_bulk")));
}

#[test]
fn score_person_stats_columns_are_written() {
    let g = seed();
    score::recompute_all(&g.seed.conn, &cfg_for("default"), NOW).unwrap();
    let (last, first, n): (Option<i64>, Option<i64>, i64) = g
        .seed
        .conn
        .query_row(
            "SELECT last_contact, first_seen, n_interactions FROM person WHERE id = ?1",
            [g.id("max_multi_channel")],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        last,
        Some(NOW - 3 * DAY),
        "the vCard contact row is ignored"
    );
    assert_eq!(first, Some(NOW - 400 * DAY));
    assert_eq!(n, 5);
    let mo_last: Option<i64> = g
        .seed
        .conn
        .query_row(
            "SELECT last_contact FROM person WHERE id = ?1",
            [g.id("mo_meet_only")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        mo_last,
        Some(NOW - 40 * DAY),
        "future meetings do not count"
    );
}

#[test]
fn score_rescore_is_idempotent() {
    let g = seed();
    let cfg = cfg_for("default");
    score::recompute_all(&g.seed.conn, &cfg, NOW).unwrap();
    let a: Vec<[f64; 6]> = g.ids.iter().map(|(_, id)| g.seed.stored(*id)).collect();
    score::recompute_all(&g.seed.conn, &cfg, NOW).unwrap();
    let b: Vec<[f64; 6]> = g.ids.iter().map(|(_, id)| g.seed.stored(*id)).collect();
    assert_eq!(a, b, "bit-identical");
}
