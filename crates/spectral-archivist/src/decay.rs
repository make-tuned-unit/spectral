//! Signal score decay and boost based on retrieval recency.
//!
//! Decay: memories not reinforced in `decay_threshold_days`+, signal_score > min,
//! decrement by `decay_amount`.
//!
//! Boost: memories reinforced in last `boost_threshold_days`, signal_score < max,
//! increment by `boost_amount`.

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
    // Compute the cutoffs and the write timestamp IN SQL with `datetime('now', ...)`
    // so they share the exact format the store writes `last_reinforced_at` and
    // `updated_at` in (`%Y-%m-%d %H:%M:%S`). The previous chrono `to_rfc3339()`
    // form mismatched that format: within the same calendar day a space-format
    // stored timestamp always string-compares LESS than an RFC3339 cutoff (space
    // 0x20 < 'T' 0x54), so eligibility was wrong at day boundaries, and each pass
    // also wrote a second, RFC3339-formatted style into `updated_at`, corrupting
    // any later lexicographic compare on that column.
    let decay_offset = format!("-{decay_threshold_days} days");
    let boost_offset = format!("-{boost_threshold_days} days");

    // One transaction for both updates — the doc contract ("uses a transaction for
    // atomicity") was previously not met (two separate autocommits).
    let tx = conn.unchecked_transaction()?;

    // Decay: not reinforced in threshold+ days (or never reinforced), signal > min
    let decayed = tx.execute(
        "UPDATE memories SET \
            signal_score = MAX(?1, signal_score - ?2), \
            updated_at = datetime('now') \
         WHERE signal_score > ?1 \
         AND (last_reinforced_at IS NULL OR last_reinforced_at < datetime('now', ?3))",
        params![min_signal, decay_amount, decay_offset],
    )?;

    // Boost: reinforced in last N days, signal < max
    let boosted = tx.execute(
        "UPDATE memories SET \
            signal_score = MIN(?1, signal_score + ?2), \
            updated_at = datetime('now') \
         WHERE signal_score < ?1 \
         AND last_reinforced_at IS NOT NULL AND last_reinforced_at > datetime('now', ?3)",
        params![max_signal, boost_amount, boost_offset],
    )?;

    tx.commit()?;
    Ok(DecayStats { decayed, boosted })
}
