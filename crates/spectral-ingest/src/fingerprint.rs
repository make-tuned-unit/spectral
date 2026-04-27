//! SHA-256 constellation fingerprint generation.
//!
//! CRITICAL: The hash algorithm must be byte-identical to production
//! `constellation.py` and `tact_retrieval.py`.
//!
//! Input:  `"{anchor_hall}|{target_hall}|{wing}|{bucket}"` encoded as UTF-8.
//! Output: first 16 hex characters of the SHA-256 digest (lowercase).

use sha2::{Digest, Sha256};

use crate::TimeBucket;

/// Compute a constellation fingerprint hash.
///
/// Byte-identical to `make_fingerprint_hash()` in `constellation.py`.
pub fn make_fingerprint_hash(
    anchor_hall: &str,
    target_hall: &str,
    wing: &str,
    bucket: TimeBucket,
) -> String {
    let raw = format!(
        "{}|{}|{}|{}",
        anchor_hall,
        target_hall,
        wing,
        bucket.as_str()
    );
    let digest = Sha256::digest(raw.as_bytes());
    format!(
        "{:016x}",
        u64::from_be_bytes(
            digest[..8]
                .try_into()
                .expect("SHA-256 always produces >= 8 bytes"),
        )
    )
}

/// All known hall types used to generate query fingerprint hashes.
pub const ALL_HALLS: &[&str] = &["fact", "preference", "discovery", "advice", "event", "none"];

/// All time buckets.
pub const ALL_BUCKETS: &[TimeBucket] = &[
    TimeBucket::SameDay,
    TimeBucket::SameWeek,
    TimeBucket::SameMonth,
    TimeBucket::Older,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden test: this exact input/output is verified against production brain.db.
    #[test]
    fn golden_hash_fact_fact_atlas_atlantic_same_day() {
        let hash = make_fingerprint_hash("fact", "fact", "atlas-atlantic", TimeBucket::SameDay);
        assert_eq!(hash, "4c355a4f544a52f5");
    }

    #[test]
    fn hash_deterministic() {
        let h1 = make_fingerprint_hash("fact", "discovery", "jesse", TimeBucket::SameWeek);
        let h2 = make_fingerprint_hash("fact", "discovery", "jesse", TimeBucket::SameWeek);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_different_inputs() {
        let h1 = make_fingerprint_hash("fact", "discovery", "jesse", TimeBucket::SameWeek);
        let h2 = make_fingerprint_hash("discovery", "fact", "jesse", TimeBucket::SameWeek);
        assert_ne!(h1, h2, "order of anchor/target hall matters");
    }

    #[test]
    fn hash_length_always_16() {
        for hall in ALL_HALLS {
            for bucket in ALL_BUCKETS {
                let h = make_fingerprint_hash(hall, hall, "test-wing", *bucket);
                assert_eq!(h.len(), 16, "hash for {hall}/{bucket} has wrong length");
            }
        }
    }

    #[test]
    fn hash_is_lowercase_hex() {
        let h = make_fingerprint_hash("fact", "event", "polybot", TimeBucket::Older);
        assert!(
            h.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "hash must be lowercase hex: {h}"
        );
    }
}
