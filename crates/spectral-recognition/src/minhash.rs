//! Shingle-set (MinHash) channel — a widely-accepted lexical near-duplicate
//! technique, added alongside the peak-pair and winnowed-gram channels.
//!
//! Motivation (docs/internal/RECOGNITION_BASELINE.md): on the lexical
//! re-encounter regime, a token-shingle set sketch separates re-encounters
//! from novel content more sharply (AUC ~0.998) than the peak-pair scalar
//! (~0.941). It is deterministic, embedding-free, and auditable ("containment
//! 0.87 with mem-X"), so it fits the engine's stature while lifting its
//! discrimination. The peak-pair/gram channels still provide the
//! combinatorial-geometry and verbatim-run signals; this channel adds robust
//! token-set overlap.
//!
//! Scoring uses **containment** (`|probe ∩ doc| / |probe|`), not symmetric
//! Jaccard: a re-encounter is usually a *fragment* of what was enrolled, so
//! containment stays high under heavy degradation where Jaccard collapses.
//! The engine blocks candidates with an inverted shingle index (share any
//! shingle → candidate) for maximal recall at sidecar scale; the MinHash
//! signature + LSH banding functions below remain available for larger-scale
//! deployments where exhaustive shingle indexing is too costly.

use sha2::{Digest, Sha256};

/// Parameters for the MinHash channel.
#[derive(Debug, Clone)]
pub struct MinHashConfig {
    /// Signature length (number of independent min-hashes). More = tighter
    /// Jaccard estimate. Must be divisible by `bands`.
    pub num_hashes: usize,
    /// LSH bands. `rows = num_hashes / bands`. Fewer rows/band → more
    /// candidates (higher recall, more work). `bands × rows == num_hashes`.
    pub bands: usize,
    /// Token shingle size (contiguous normalized tokens per shingle).
    pub shingle: usize,
    /// Weight of a shingle-containment match in the recognition score,
    /// relative to a rarity-weighted pair hit. `0.0` disables the channel.
    pub weight: f64,
    /// Minimum shingle-set containment for a match to count as evidence.
    pub min_similarity: f64,
}

impl Default for MinHashConfig {
    fn default() -> Self {
        Self {
            num_hashes: 64,
            bands: 16, // 4 rows/band
            shingle: 2,
            weight: 3.0,
            min_similarity: 0.15,
        }
    }
}

/// Tokenize consistently with the extractor: split on non-word chars,
/// lowercase, drop empties. Kept deliberately simple — MinHash wants raw token
/// overlap, not the stemmed/anchored landmark view.
fn tokens(content: &str) -> Vec<String> {
    content
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .map(|t| t.trim_matches(|c: char| c == '.' || c == '-' || c == '_'))
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// The k-shingle set of a document, as sorted, deduped 64-bit shingle hashes.
pub fn shingle_set(content: &str, shingle: usize) -> Vec<u64> {
    let toks = tokens(content);
    if toks.is_empty() {
        return Vec::new();
    }
    let k = shingle.max(1);
    if toks.len() < k {
        // Short doc: the whole token sequence is one shingle.
        return vec![hash_seeded(&toks.join(" "), 0)];
    }
    let mut out = Vec::with_capacity(toks.len() - k + 1);
    for w in toks.windows(k) {
        out.push(hash_seeded(&w.join(" "), 0));
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// **Containment** of `probe` in `doc`: the fraction of the probe's shingles
/// present in the doc, `|probe ∩ doc| / |probe|`. Unlike symmetric Jaccard,
/// containment is high when the probe is a *fragment* of the doc — the
/// re-encounter case (a partial/degraded re-observation of something enrolled).
/// This is why recognition uses containment, not Jaccard: a re-encounter is
/// usually a subset, not an equal-sized twin.
pub fn containment(probe_set: &[u64], doc_set: &[u64]) -> f64 {
    if probe_set.is_empty() {
        return 0.0;
    }
    // Both are sorted+deduped; merge-count the intersection.
    let (mut i, mut j, mut inter) = (0, 0, 0usize);
    while i < probe_set.len() && j < doc_set.len() {
        match probe_set[i].cmp(&doc_set[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                inter += 1;
                i += 1;
                j += 1;
            }
        }
    }
    inter as f64 / probe_set.len() as f64
}

/// The k-shingle multiset hashes (unsorted) for signature computation.
fn shingles(content: &str, shingle: usize) -> Vec<u64> {
    shingle_set(content, shingle)
}

fn hash_seeded(s: &str, seed: u64) -> u64 {
    let mut h = Sha256::new();
    h.update(seed.to_le_bytes());
    h.update(s.as_bytes());
    let d = h.finalize();
    u64::from_be_bytes(d[..8].try_into().expect("8 bytes"))
}

/// Mix a shingle hash under permutation `i` (xorshift-style, deterministic).
/// Standard MinHash trick: one base hash permuted by `num_hashes` seeds
/// avoids re-hashing the string per permutation.
fn permute(base: u64, i: u64) -> u64 {
    let mut x = base ^ i.wrapping_mul(0x9E3779B97F4A7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58476D1CE4E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

/// Compute the MinHash signature: for each permutation, the min permuted
/// shingle hash. Empty documents get an all-`u64::MAX` signature (matches
/// nothing but itself, Jaccard 0 with any non-empty doc).
pub fn signature(content: &str, config: &MinHashConfig) -> Vec<u64> {
    let sh = shingles(content, config.shingle);
    let mut sig = vec![u64::MAX; config.num_hashes];
    for &s in &sh {
        for (i, slot) in sig.iter_mut().enumerate() {
            let v = permute(s, i as u64);
            if v < *slot {
                *slot = v;
            }
        }
    }
    sig
}

/// Estimated Jaccard similarity: fraction of matching signature positions.
pub fn estimated_jaccard(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let matches = a.iter().zip(b).filter(|(x, y)| x == y).count();
    matches as f64 / a.len() as f64
}

/// LSH band hashes: split the signature into `bands` bands and hash each. Two
/// documents sharing any band hash are near-duplicate candidates. Deterministic.
pub fn band_hashes(sig: &[u64], config: &MinHashConfig) -> Vec<u64> {
    let bands = config.bands.max(1);
    if sig.is_empty() || !sig.len().is_multiple_of(bands) {
        // Fallback: one band over the whole signature.
        return vec![hash_u64s(sig, 0)];
    }
    let rows = sig.len() / bands;
    (0..bands)
        .map(|b| hash_u64s(&sig[b * rows..(b + 1) * rows], b as u64))
        .collect()
}

fn hash_u64s(vals: &[u64], band_idx: u64) -> u64 {
    let mut h = Sha256::new();
    h.update(band_idx.to_le_bytes());
    for v in vals {
        h.update(v.to_le_bytes());
    }
    let d = h.finalize();
    u64::from_be_bytes(d[..8].try_into().expect("8 bytes"))
}

/// Serialize a signature to bytes for storage (little-endian u64s).
pub fn signature_to_bytes(sig: &[u64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(sig.len() * 8);
    for v in sig {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Deserialize a signature from bytes. Returns empty on malformed input.
pub fn signature_from_bytes(bytes: &[u8]) -> Vec<u64> {
    if !bytes.len().is_multiple_of(8) {
        return Vec::new();
    }
    bytes
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_content_has_jaccard_one() {
        let cfg = MinHashConfig::default();
        let s = signature("the deploy failed with exit code 137 OOMKilled", &cfg);
        assert!((estimated_jaccard(&s, &s) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn degraded_reencounter_scores_higher_than_novel() {
        let cfg = MinHashConfig::default();
        let orig = "Pod task-runner-7f9 OOMKilled at 512Mi during the nightly batch reindex job";
        let degraded = "task-runner-7f9 OOMKilled 512Mi nightly batch reindex"; // dropped tokens
        let novel = "The marketing dashboard export button stopped working on Safari today";
        let so = signature(orig, &cfg);
        let sd = signature(degraded, &cfg);
        let sn = signature(novel, &cfg);
        let j_degraded = estimated_jaccard(&so, &sd);
        let j_novel = estimated_jaccard(&so, &sn);
        assert!(
            j_degraded > j_novel + 0.2,
            "degraded ({j_degraded}) should clearly beat novel ({j_novel})"
        );
    }

    #[test]
    fn band_hashes_collide_for_near_duplicates() {
        let cfg = MinHashConfig::default();
        let a = signature(
            "the quarterly budget review moved to thursday afternoon",
            &cfg,
        );
        let b = signature("the quarterly budget review moved to thursday", &cfg);
        let ba: std::collections::HashSet<u64> = band_hashes(&a, &cfg).into_iter().collect();
        let bb = band_hashes(&b, &cfg);
        assert!(
            bb.iter().any(|h| ba.contains(h)),
            "near-duplicates should share at least one band"
        );
    }

    #[test]
    fn signature_roundtrips_through_bytes() {
        let cfg = MinHashConfig::default();
        let s = signature("roundtrip me please and thank you", &cfg);
        assert_eq!(signature_from_bytes(&signature_to_bytes(&s)), s);
    }
}
