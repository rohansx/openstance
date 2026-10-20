//! Compiled-in weight presets. Spec: `docs/tech-spec/scoring.md` section 4.
//!
//! Owned by the "score" stream. Every preset sums to 1.00:
//!
//! | preset    | R    | F    | C    | M    | B    |
//! |-----------|------|------|------|------|------|
//! | default   | 0.35 | 0.20 | 0.25 | 0.15 | 0.05 |
//! | founder   | 0.30 | 0.15 | 0.30 | 0.20 | 0.05 |
//! | recruiter | 0.35 | 0.20 | 0.15 | 0.10 | 0.20 |
//! | seller    | 0.25 | 0.15 | 0.30 | 0.25 | 0.05 |
//! | community | 0.15 | 0.40 | 0.25 | 0.10 | 0.10 |

use super::Weights;

/// Preset names, in documentation order.
pub const PRESET_NAMES: [&str; 5] = ["default", "founder", "recruiter", "seller", "community"];

/// Weights of a named preset.
pub fn preset(name: &str) -> Option<Weights> {
    let [recency, frequency, reciprocity, meetings, breadth] = match name {
        "default" => [0.35, 0.20, 0.25, 0.15, 0.05],
        "founder" => [0.30, 0.15, 0.30, 0.20, 0.05],
        "recruiter" => [0.35, 0.20, 0.15, 0.10, 0.20],
        "seller" => [0.25, 0.15, 0.30, 0.25, 0.05],
        "community" => [0.15, 0.40, 0.25, 0.10, 0.10],
        _ => return None,
    };
    Some(Weights {
        recency,
        frequency,
        reciprocity,
        meetings,
        breadth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_sums_to_one() {
        for name in PRESET_NAMES {
            let w = preset(name).expect(name);
            let sum = w.recency + w.frequency + w.reciprocity + w.meetings + w.breadth;
            assert!((sum - 1.0).abs() < 1e-12, "{name} sums to {sum}");
        }
    }

    #[test]
    fn default_and_founder_match_the_spec_table() {
        let d = preset("default").unwrap();
        assert_eq!(
            [d.recency, d.frequency, d.reciprocity, d.meetings, d.breadth],
            [0.35, 0.20, 0.25, 0.15, 0.05]
        );
        let f = preset("founder").unwrap();
        assert_eq!(
            [f.recency, f.frequency, f.reciprocity, f.meetings, f.breadth],
            [0.30, 0.15, 0.30, 0.20, 0.05]
        );
        let r = preset("recruiter").unwrap();
        assert_eq!(
            [r.recency, r.frequency, r.reciprocity, r.meetings, r.breadth],
            [0.35, 0.20, 0.15, 0.10, 0.20]
        );
        let s = preset("seller").unwrap();
        assert_eq!(
            [s.recency, s.frequency, s.reciprocity, s.meetings, s.breadth],
            [0.25, 0.15, 0.30, 0.25, 0.05]
        );
        let c = preset("community").unwrap();
        assert_eq!(
            [c.recency, c.frequency, c.reciprocity, c.meetings, c.breadth],
            [0.15, 0.40, 0.25, 0.10, 0.10]
        );
    }

    #[test]
    fn unknown_preset_is_none() {
        assert!(preset("custom").is_none());
        assert!(preset("Default").is_none());
        assert!(preset("").is_none());
    }
}
