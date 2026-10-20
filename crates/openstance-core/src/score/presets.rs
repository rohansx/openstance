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
    let _ = name;
    todo!("score stream: P1-CORE-11 presets")
}
