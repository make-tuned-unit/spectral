//! Shared evaluation harness primitives for recognition replays.
//!
//! Extracted so the deterministic engine replay (`bin/replay.rs`) and the
//! neural embedding baseline (`examples/embedding_baseline.rs`) score the
//! **identical** positives, negatives, split, and label-noise mask — the only
//! variable between them is the familiarity scalar. Without a shared harness a
//! head-to-head AUC comparison would not be honest.

use std::collections::HashSet;

/// Stable 64-bit hash of an id (first 8 bytes of SHA-256, big-endian).
pub fn hash_id(id: &str) -> u64 {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(id.as_bytes());
    u64::from_be_bytes(d[..8].try_into().unwrap())
}

/// Deterministic ~`drop_pct`% token dropout keyed on (id, position). This is
/// the Shazam noisy-fragment condition: the same content with a random-looking
/// but reproducible subset of tokens removed.
pub fn degrade(content: &str, id: &str, drop_pct: u64) -> String {
    content
        .split_whitespace()
        .enumerate()
        .filter(|(i, _)| hash_id(&format!("{id}|{i}")) % 100 >= drop_pct)
        .map(|(_, t)| t)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A list of `(id, content)` memories.
pub type Memories = Vec<(String, String)>;

/// Deterministic 90/10 split by `hash_id(id) % 10`: not a multiple of 10 →
/// enrolled ("known"), else held-out. Returns owned clones for downstream
/// ergonomics.
pub fn split_9010(memories: &[(String, String)]) -> (Memories, Memories) {
    memories
        .iter()
        .cloned()
        .partition(|(id, _)| !hash_id(id).is_multiple_of(10))
}

/// Lowercased token set of terms with length ≥ 3 (matches the replay's
/// near-duplicate detector).
pub fn token_set(s: &str) -> HashSet<String> {
    s.split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| t.len() >= 3)
        .collect()
}

/// Max Jaccard of `probe`'s token set against each enrolled token set.
pub fn max_jaccard(probe: &HashSet<String>, enrolled_sets: &[HashSet<String>]) -> f64 {
    enrolled_sets
        .iter()
        .map(|es| {
            let inter = probe.intersection(es).count() as f64;
            let union = (probe.len() + es.len()) as f64 - inter;
            if union > 0.0 {
                inter / union
            } else {
                0.0
            }
        })
        .fold(0.0f64, f64::max)
}

/// A held-out negative whose token overlap with some enrolled memory is at or
/// above this Jaccard is a genuine near-duplicate: the "novel" label is wrong,
/// so it is excluded from the clean-negative AUC.
pub const LABEL_NOISE_JACCARD: f64 = 0.5;

/// ROC-AUC via the Mann–Whitney rank statistic with ties at half credit.
/// `scores` are `(scalar, is_positive)`. Higher scalar should mean "more
/// familiar / old".
pub fn roc_auc(scores: &[(f64, bool)]) -> f64 {
    let pos: Vec<f64> = scores.iter().filter(|s| s.1).map(|s| s.0).collect();
    let neg: Vec<f64> = scores.iter().filter(|s| !s.1).map(|s| s.0).collect();
    if pos.is_empty() || neg.is_empty() {
        return f64::NAN;
    }
    let mut num = 0.0f64;
    for &p in &pos {
        for &n in &neg {
            num += if p > n {
                1.0
            } else if p == n {
                0.5
            } else {
                0.0
            };
        }
    }
    num / (pos.len() as f64 * neg.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_is_deterministic_and_disjoint() {
        let mems: Vec<(String, String)> = (0..200)
            .map(|i| (format!("m{i}"), format!("content number {i}")))
            .collect();
        let (a, b) = split_9010(&mems);
        let (a2, b2) = split_9010(&mems);
        assert_eq!(a.len() + b.len(), mems.len());
        assert!(!a.is_empty() && !b.is_empty());
        assert_eq!(a, a2, "split must be deterministic");
        assert_eq!(b, b2);
    }

    #[test]
    fn degrade_drops_roughly_the_requested_fraction() {
        let content = (0..1000)
            .map(|i| format!("tok{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let kept = degrade(&content, "id", 30).split_whitespace().count();
        // ~70% retained; allow generous slack for the hash distribution.
        assert!(
            (600..=800).contains(&kept),
            "expected ~700 kept, got {kept}"
        );
    }

    #[test]
    fn roc_auc_perfect_and_inverted() {
        let perfect = vec![(0.9, true), (0.8, true), (0.2, false), (0.1, false)];
        assert!((roc_auc(&perfect) - 1.0).abs() < 1e-9);
        let inverted = vec![(0.1, true), (0.2, true), (0.8, false), (0.9, false)];
        assert!((roc_auc(&inverted) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn roc_auc_ties_half_credit() {
        let tied = vec![(0.5, true), (0.5, false)];
        assert!((roc_auc(&tied) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn jaccard_identity_is_one() {
        let a = token_set("the quick brown fox jumps");
        let sets = vec![a.clone()];
        assert!((max_jaccard(&a, &sets) - 1.0).abs() < 1e-9);
    }
}
