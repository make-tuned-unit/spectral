//! Signal score decay and boost based on retrieval recency.
//!
//! Decay: memories not reinforced in `decay_threshold_days`+, signal_score > min,
//! decrement by `decay_amount`.
//!
//! Boost: memories reinforced in last `boost_threshold_days`, signal_score < max,
//! increment by `boost_amount`.

use chrono::Utc;
use rusqlite::{params, Connection};

/// Result of applying decay and boost.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DecayStats {
    pub decayed: usize,
    pub boosted: usize,
}

/// Apply signal score decay and boost. Uses a transaction for atomicity.
///
/// Uses `last_reinforced_at` (set by `Brain::reinforce()`) as the recency
/// signal, analogous to `last_retrieved` in the reference implementation.
pub fn apply_decay(
    conn: &Connection,
    decay_threshold_days: i64,
    decay_amount: f64,
    boost_threshold_days: i64,
    boost_amount: f64,
    min_signal: f64,
    max_signal: f64,
) -> anyhow::Result<DecayStats> {
    let now = Utc::now().to_rfc3339();
    let decay_cutoff = (Utc::now() - chrono::Duration::days(decay_threshold_days)).to_rfc3339();
    let boost_cutoff = (Utc::now() - chrono::Duration::days(boost_threshold_days)).to_rfc3339();

    // Decay: not reinforced in threshold+ days (or never reinforced), signal > min
    let decayed = conn.execute(
        "UPDATE memories SET \
            signal_score = MAX(?1, signal_score - ?2), \
            updated_at = ?3 \
         WHERE signal_score > ?1 \
         AND (last_reinforced_at IS NULL OR last_reinforced_at < ?4)",
        params![min_signal, decay_amount, now, decay_cutoff],
    )?;

    // Boost: reinforced in last N days, signal < max
    let boosted = conn.execute(
        "UPDATE memories SET \
            signal_score = MIN(?1, signal_score + ?2), \
            updated_at = ?3 \
         WHERE signal_score < ?1 \
         AND last_reinforced_at IS NOT NULL AND last_reinforced_at > ?4",
        params![max_signal, boost_amount, now, boost_cutoff],
    )?;

    Ok(DecayStats { decayed, boosted })
}
