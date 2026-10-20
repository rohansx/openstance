//! Pure component math. Spec: `docs/tech-spec/scoring.md` section 2.
//!
//! Owned by the "score" stream. Golden values: R(180) = 0.5 exactly at the
//! default half-life; the worked example in section 9 (Priya 0.8853,
//! Marcus 0.6066, Dana 0.3683).

use super::{Components, ScoreConfig, ScoreInputs, Weights};

/// `exp(-ln(2) * days / half_life_days)`; future timestamps clamp to 1.
pub fn recency(days_since_last: Option<f64>, half_life_days: f64) -> f64 {
    let _ = (days_since_last, half_life_days);
    todo!("score stream: recency")
}

/// `1 - exp(-n / scale)`.
pub fn frequency(n_interactions: u32, scale: f64) -> f64 {
    let _ = (n_interactions, scale);
    todo!("score stream: frequency")
}

/// `0` when `n_out = 0`, else `min(1, (n_replies + m * n_threads_started) / n_out)`.
pub fn reciprocity(n_out: u32, n_replies: u32, n_threads_started: u32, multiplier: f64) -> f64 {
    let _ = (n_out, n_replies, n_threads_started, multiplier);
    todo!("score stream: reciprocity")
}

/// `1 - exp(-n_meetings / scale)`.
pub fn meetings(n_meetings: u32, scale: f64) -> f64 {
    let _ = (n_meetings, scale);
    todo!("score stream: meetings")
}

/// `channels_seen / 4`.
pub fn breadth(channels: usize) -> f64 {
    let _ = channels;
    todo!("score stream: breadth")
}

/// All five components.
pub fn components(i: &ScoreInputs, cfg: &ScoreConfig) -> Components {
    let _ = (i, cfg);
    todo!("score stream: components")
}

/// `S = sum(w_k * c_k)`.
pub fn total(c: &Components, w: &Weights) -> f64 {
    let _ = (c, w);
    todo!("score stream: total")
}
