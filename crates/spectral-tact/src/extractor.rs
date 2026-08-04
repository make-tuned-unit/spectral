//! Fingerprint generation and search orchestration.

use sha2::{Digest, Sha256};

use crate::classifier::extract_query_terms;
use crate::{MemoryHit, MemoryStore, RetrievalMethod, TactConfig};

const ALL_HALLS: &[&str] = &["fact", "preference", "discovery", "advice", "event"];
const TIME_BUCKETS: &[&str] = &["same_day", "same_week", "same_month", "older"];

/// Generate a deterministic fingerprint hash.
pub fn make_fingerprint_hash(
    anchor_hall: &str,
    target_hall: &str,
    wing: &str,
    bucket: &str,
) -> String {
    let raw = format!("{anchor_hall}|{target_hall}|{wing}|{bucket}");
    let hash = Sha256::digest(raw.as_bytes());
    hex_encode(&hash[..8])
}

/// Run the multi-tier search pipeline.
pub async fn search(
    user_msg: &str,
    wing: &Option<String>,
    hall: &Option<String>,
    config: &TactConfig,
    store: &dyn MemoryStore,
) -> anyhow::Result<(Vec<MemoryHit>, RetrievalMethod)> {
    let max = config.max_results;

    // Tier 1: Fingerprint search.
    //
    // Gated on wing AND hall by default. `tier1_requires_hall = false` fires on
    // wing alone and enumerates every anchor hall, because a *question* rarely
    // declares the memory type that answers it — the conjunction suppressed
    // this tier to 0.9% of real queries.
    let tier1_wing = match (wing, hall, config.tier1_requires_hall) {
        (Some(w), Some(_), _) => Some(w),
        (Some(w), None, false) => Some(w),
        _ => None,
    };
    if let Some(w) = tier1_wing {
        // Anchor hall: the query's own when known, otherwise every hall.
        let anchors: Vec<&str> = match hall {
            Some(h) => vec![h.as_str()],
            None => ALL_HALLS.to_vec(),
        };
        let mut hashes: Vec<String> = anchors
            .iter()
            .flat_map(|a| generate_query_hashes(a, w))
            .collect();
        hashes.sort();
        hashes.dedup();
        // `fingerprint_search`'s hall argument scopes the anchor; with no hall
        // on the query the hashes already span every anchor, so pass the first.
        let probe_hall = anchors[0];
        let results = store
            .fingerprint_search(w, probe_hall, &hashes, max)
            .await?;

        if !results.is_empty() {
            let query_words = extract_fts_words(user_msg);
            let fts_results = store.fts_search(&query_words, max).await?;
            let merged = merge_dedup(results, fts_results, max);
            return Ok((merged, RetrievalMethod::Fingerprint));
        }
    }

    // Tier 2: Wing-only search.
    if let Some(w) = wing {
        let terms = extract_query_terms(user_msg);
        let results = store.wing_search(w, &terms, max).await?;
        if !results.is_empty() {
            return Ok((results, RetrievalMethod::WingOnly));
        }
    }

    // Tier 3: FTS fallback.
    let query_words = extract_fts_words(user_msg);
    if !query_words.is_empty() {
        let results = store.fts_search(&query_words, max).await?;
        if !results.is_empty() {
            return Ok((results, RetrievalMethod::Fts));
        }
    }

    Ok((Vec::new(), RetrievalMethod::Empty))
}

/// Public for diagnostics: the hash set a (anchor_hall, wing) pair probes.
pub fn query_hashes_for(hall: &str, wing: &str) -> Vec<String> {
    generate_query_hashes(hall, wing)
}

fn generate_query_hashes(hall: &str, wing: &str) -> Vec<String> {
    let mut hashes = Vec::new();
    for target_hall in ALL_HALLS {
        for bucket in TIME_BUCKETS {
            hashes.push(make_fingerprint_hash(hall, target_hall, wing, bucket));
            hashes.push(make_fingerprint_hash(target_hall, hall, wing, bucket));
        }
    }
    hashes.sort();
    hashes.dedup();
    hashes
}

fn extract_fts_words(msg: &str) -> Vec<String> {
    // Static pattern: compile once, not once per query.
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"\w+").unwrap());
    re.find_iter(&msg.to_lowercase())
        .map(|m| m.as_str().to_string())
        .collect()
}

fn merge_dedup(primary: Vec<MemoryHit>, secondary: Vec<MemoryHit>, max: usize) -> Vec<MemoryHit> {
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();

    for hit in primary.into_iter().chain(secondary) {
        if seen.insert(hit.id.clone()) {
            merged.push(hit);
            if merged.len() >= max {
                break;
            }
        }
    }

    merged
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_hash_is_deterministic() {
        let h1 = make_fingerprint_hash("fact", "discovery", "apollo", "same_week");
        let h2 = make_fingerprint_hash("fact", "discovery", "apollo", "same_week");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn fingerprint_hash_varies_with_inputs() {
        let h1 = make_fingerprint_hash("fact", "discovery", "apollo", "same_week");
        let h2 = make_fingerprint_hash("fact", "discovery", "apollo", "same_month");
        assert_ne!(h1, h2);
    }

    #[test]
    fn merge_dedup_preserves_primary_order() {
        let primary = vec![
            MemoryHit {
                id: "a".into(),
                key: "k".into(),
                content: "c".into(),
                wing: None,
                hall: None,
                signal_score: 0.9,
                visibility: "private".into(),
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
                hits: 3,
                source_brain_id: None,
                signature: None,
            },
            MemoryHit {
                id: "b".into(),
                key: "k".into(),
                content: "c".into(),
                wing: None,
                hall: None,
                signal_score: 0.8,
                visibility: "private".into(),
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
                hits: 2,
                source_brain_id: None,
                signature: None,
            },
        ];
        let secondary = vec![
            MemoryHit {
                id: "b".into(),
                key: "k".into(),
                content: "c".into(),
                wing: None,
                hall: None,
                signal_score: 0.8,
                visibility: "private".into(),
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
                hits: 1,
                source_brain_id: None,
                signature: None,
            },
            MemoryHit {
                id: "c".into(),
                key: "k".into(),
                content: "c".into(),
                wing: None,
                hall: None,
                signal_score: 0.7,
                visibility: "private".into(),
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
                hits: 1,
                source_brain_id: None,
                signature: None,
            },
        ];

        let merged = merge_dedup(primary, secondary, 10);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].id, "a");
        assert_eq!(merged[1].id, "b");
        assert_eq!(merged[2].id, "c");
    }
}
