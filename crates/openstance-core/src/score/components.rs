//! Pure component math. Spec: `docs/tech-spec/scoring.md` section 2.
//!
//! Owned by the "score" stream. Golden values: R(180) = 0.5 exactly at the
//! default half-life; the worked example in section 9 (Priya 0.8853,
//! Marcus 0.6066, Dana 0.3683).

use super::{Components, ScoreConfig, ScoreInputs, Weights};

/// `exp(-ln(2) * days / half_life_days)`; future timestamps clamp to 1.
pub fn recency(days_since_last: Option<f64>, half_life_days: f64) -> f64 {
    match days_since_last {
        None => 0.0,
        Some(d) => decay(d, half_life_days),
    }
}

/// `exp(-ln(2) * max(0, days) / half_life_days)`, shared with tie weights.
pub(crate) fn decay(days: f64, half_life_days: f64) -> f64 {
    let d = if days.is_nan() { 0.0 } else { days.max(0.0) };
    (-std::f64::consts::LN_2 * d / half_life_days).exp()
}

/// `1 - exp(-n / scale)`.
fn saturate(n: u32, scale: f64) -> f64 {
    1.0 - (-f64::from(n) / scale).exp()
}

/// `1 - exp(-n / scale)`.
pub fn frequency(n_interactions: u32, scale: f64) -> f64 {
    saturate(n_interactions, scale)
}

/// `0` when `n_out = 0`, else `min(1, (n_replies + m * n_threads_started) / n_out)`.
pub fn reciprocity(n_out: u32, n_replies: u32, n_threads_started: u32, multiplier: f64) -> f64 {
    if n_out == 0 {
        return 0.0;
    }
    let num = f64::from(n_replies) + multiplier * f64::from(n_threads_started);
    (num / f64::from(n_out)).min(1.0)
}

/// `1 - exp(-n_meetings / scale)`.
pub fn meetings(n_meetings: u32, scale: f64) -> f64 {
    saturate(n_meetings, scale)
}

/// `channels_seen / 4`.
pub fn breadth(channels: usize) -> f64 {
    // At most four channels exist; the clamp guards a future fifth.
    (channels.min(4) as f64) / 4.0
}

/// All five components.
pub fn components(i: &ScoreInputs, cfg: &ScoreConfig) -> Components {
    Components {
        recency: recency(i.days_since_last, cfg.half_life_days),
        frequency: frequency(i.n_interactions, cfg.frequency_scale),
        reciprocity: reciprocity(
            i.n_out,
            i.n_replies,
            i.n_threads_started,
            cfg.thread_start_multiplier,
        ),
        meetings: meetings(i.n_meetings, cfg.meetings_scale),
        breadth: breadth(i.channels.len()),
    }
}

/// `S = sum(w_k * c_k)`.
pub fn total(c: &Components, w: &Weights) -> f64 {
    w.recency * c.recency
        + w.frequency * c.frequency
        + w.reciprocity * c.reciprocity
        + w.meetings * c.meetings
        + w.breadth * c.breadth
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScoreSection;
    use crate::score::Channel;

    fn cfg() -> ScoreConfig {
        ScoreConfig::from_config(&ScoreSection::default())
            .unwrap()
            .0
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn recency_golden_points() {
        assert_eq!(recency(Some(0.0), 180.0), 1.0);
        assert!(close(recency(Some(180.0), 180.0), 0.5, 1e-9));
        assert!(close(recency(Some(360.0), 180.0), 0.25, 1e-9));
        assert!(close(recency(Some(540.0), 180.0), 0.125, 1e-9));
        assert!(close(recency(Some(30.0), 180.0), 0.890_898_718, 1e-9));
        assert!(close(
            recency(Some(90.0), 180.0),
            std::f64::consts::FRAC_1_SQRT_2,
            1e-12
        ));
    }

    #[test]
    fn recency_none_is_zero_and_future_clamps_to_one() {
        assert_eq!(recency(None, 180.0), 0.0);
        assert_eq!(recency(Some(-5.0), 180.0), 1.0);
    }

    #[test]
    fn recency_is_monotone_non_increasing() {
        let mut prev = 1.0;
        for d in 0..2000 {
            let r = recency(Some(f64::from(d) * 0.75), 180.0);
            assert!(r <= prev && (0.0..=1.0).contains(&r));
            prev = r;
        }
    }

    #[test]
    fn frequency_and_meetings_saturate() {
        assert_eq!(frequency(0, 20.0), 0.0);
        assert!(close(frequency(20, 20.0), 1.0 - (-1.0f64).exp(), 1e-12));
        assert!(close(frequency(42, 20.0), 0.877_543_572, 1e-9));
        assert_eq!(meetings(0, 3.0), 0.0);
        assert!(close(meetings(4, 3.0), 0.736_402_862, 1e-9));
        let mut prev = 0.0;
        for n in 0..500 {
            let f = frequency(n, 20.0);
            let m = meetings(n, 3.0);
            assert!(f >= prev && (0.0..=1.0).contains(&f) && (0.0..=1.0).contains(&m));
            prev = f;
        }
    }

    #[test]
    fn reciprocity_rules() {
        // n_out = 0 guard: cold inbound sender.
        assert_eq!(reciprocity(0, 10, 5, 2.0), 0.0);
        // Capped at 1.
        assert_eq!(reciprocity(18, 15, 3, 2.0), 1.0);
        assert!(close(reciprocity(30, 24, 0, 2.0), 0.8, 1e-12));
        // A thread they started counts double.
        assert!(close(reciprocity(10, 1, 2, 2.0), 0.5, 1e-12));
        assert!(close(reciprocity(10, 1, 2, 3.0), 0.7, 1e-12));
        assert_eq!(reciprocity(1, 0, 0, 2.0), 0.0);
    }

    #[test]
    fn breadth_is_channels_over_four() {
        assert_eq!(breadth(0), 0.0);
        assert_eq!(breadth(3), 0.75);
        assert_eq!(breadth(4), 1.0);
        assert_eq!(breadth(9), 1.0);
    }

    /// The worked example in scoring.md section 9.
    #[test]
    fn worked_example_priya_marcus_dana() {
        let c = cfg();
        let priya = ScoreInputs {
            last_contact: Some(0),
            days_since_last: Some(30.0),
            n_interactions: 42,
            n_out: 18,
            n_replies: 15,
            n_threads_started: 3,
            n_meetings: 4,
            channels: vec![Channel::Email, Channel::Calendar, Channel::Linkedin],
        };
        let marcus = ScoreInputs {
            last_contact: Some(0),
            days_since_last: Some(450.0),
            n_interactions: 60,
            n_out: 30,
            n_replies: 24,
            n_threads_started: 0,
            n_meetings: 6,
            channels: vec![Channel::Email, Channel::Calendar],
        };
        let dana = ScoreInputs {
            last_contact: Some(0),
            days_since_last: Some(10.0),
            n_interactions: 2,
            n_out: 1,
            n_replies: 0,
            n_threads_started: 0,
            n_meetings: 0,
            channels: vec![Channel::Linkedin],
        };
        let p = components(&priya, &c);
        assert!(close(p.recency, 0.8909, 1e-4));
        assert!(close(p.frequency, 0.8775, 1e-4));
        assert_eq!(p.reciprocity, 1.0);
        assert!(close(p.meetings, 0.7364, 1e-4));
        assert_eq!(p.breadth, 0.75);
        assert!(close(total(&p, &c.weights), 0.8853, 1e-4));
        assert!(close(
            total(&components(&marcus, &c), &c.weights),
            0.6066,
            1e-4
        ));
        assert!(close(
            total(&components(&dana, &c), &c.weights),
            0.3683,
            1e-4
        ));

        let expect = [
            ("founder", [0.8837, 0.6335, 0.3154]),
            ("recruiter", [0.8610, 0.5584, 0.4058]),
            ("seller", [0.8760, 0.6679, 0.2673]),
            ("community", [0.8833, 0.7431, 0.2074]),
        ];
        for (name, s) in expect {
            let w = crate::score::presets::preset(name).unwrap();
            for (inp, want) in [&priya, &marcus, &dana].iter().zip(s) {
                let got = total(&components(inp, &c), &w);
                assert!(close(got, want, 1e-4), "{name}: {got} vs {want}");
            }
        }
    }

    #[test]
    fn empty_inputs_score_zero() {
        let c = cfg();
        let z = components(&ScoreInputs::default(), &c);
        assert_eq!(z, Components::default());
        assert_eq!(total(&z, &c.weights), 0.0);
    }

    proptest::proptest! {
        #[test]
        fn components_stay_in_unit_interval(
            days in proptest::option::of(-1000.0f64..5000.0),
            n in 0u32..10_000,
            out in 0u32..500,
            replies in 0u32..500,
            starts in 0u32..500,
            meets in 0u32..500,
            chans in 0usize..5,
        ) {
            let all = [Channel::Email, Channel::Calendar, Channel::Linkedin, Channel::Social];
            let i = ScoreInputs {
                last_contact: None,
                days_since_last: days,
                n_interactions: n,
                n_out: out,
                n_replies: replies,
                n_threads_started: starts,
                n_meetings: meets,
                channels: all[..chans.min(4)].to_vec(),
            };
            let c = cfg();
            let k = components(&i, &c);
            for v in [k.recency, k.frequency, k.reciprocity, k.meetings, k.breadth] {
                proptest::prop_assert!((0.0..=1.0).contains(&v));
            }
            let s = total(&k, &c.weights);
            proptest::prop_assert!((0.0..=1.0 + 1e-12).contains(&s));
        }

        #[test]
        fn monotone_in_days_and_counts(d in 0.0f64..3000.0, step in 0.0f64..100.0, n in 0u32..1000) {
            proptest::prop_assert!(recency(Some(d + step), 180.0) <= recency(Some(d), 180.0));
            proptest::prop_assert!(frequency(n + 1, 20.0) >= frequency(n, 20.0));
            proptest::prop_assert!(meetings(n + 1, 3.0) >= meetings(n, 3.0));
        }
    }
}
