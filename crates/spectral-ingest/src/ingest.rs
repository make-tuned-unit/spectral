//! Top-level ingestion pipeline.

use regex::Regex;

use chrono::{DateTime, Utc};
use spectral_core::device_id::DeviceId;

use crate::classifier;
use crate::fingerprint;
use crate::signal;
use crate::{Episode, Fingerprint, Memory, MemoryStore, TimeBucket};

/// Default time gap for auto-detecting episode boundaries (30 minutes).
const EPISODE_GAP_MINUTES: i64 = 30;

/// Configuration for the ingestion pipeline.
#[derive(Debug, Clone)]
pub struct IngestConfig {
    /// Wing classification rules.
    pub wing_rules: Vec<(Regex, String)>,
    /// Hall classification rules.
    pub hall_rules: Vec<(Regex, String)>,
    /// Minimum signal_score for fingerprint generation (default 0.5).
    pub signal_threshold: f64,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            wing_rules: classifier::default_wing_rules(),
            hall_rules: classifier::default_hall_rules(),
            signal_threshold: 0.5,
        }
    }
}

/// Optional provenance metadata for ingestion.
#[derive(Debug, Clone, Default)]
pub struct IngestOpts {
    pub source: Option<String>,
    pub device_id: Option<DeviceId>,
    /// Classification confidence override. `None` = default 1.0.
    pub confidence: Option<f64>,
    /// Override the memory's creation timestamp. `None` means use the
    /// database default (`datetime('now')`). Use this when ingesting
    /// historical memories with known dates.
    pub created_at: Option<DateTime<Utc>>,
    /// Assign the memory to this episode. `None` = auto-detect via
    /// time-gap heuristic (join recent episode in same wing if within
    /// 30 min, otherwise create a new episode).
    pub episode_id: Option<String>,
    /// Compaction tier for ambient stream memories. Set to `Some(Raw)` when
    /// ingesting raw activity events; the Librarian (or other consumer-side
    /// compaction process) updates this to `HourlyRollup`, `DailyRollup`, or
    /// `WeeklyRollup` as memories are aggregated over time. `None` means the
    /// memory is not part of the ambient stream. Spectral uses
    /// `compaction_tier.is_some()` as the canonical signal that a memory
    /// belongs to the ambient stream.
    pub compaction_tier: Option<crate::CompactionTier>,
    /// Wing override. When `Some(value)`, the classifier is bypassed and
    /// the value is stored as-is (no normalization, no prefix stripping).
    /// Callers are responsible for passing the canonical slug form.
    /// When `None`, wing is derived by the classifier from key+content+category.
    pub wing: Option<String>,
}

/// Result of the ingestion pipeline.
#[derive(Debug)]
pub struct IngestResult {
    pub memory: Memory,
    pub fingerprints: Vec<Fingerprint>,
}

/// Run the ingestion pipeline: classify, score, generate fingerprints, write.
#[allow(clippy::too_many_arguments)]
pub async fn ingest(
    id: &str,
    key: &str,
    content: &str,
    category: &str,
    _created_at_epoch: f64,
    visibility: &str,
    config: &IngestConfig,
    store: &dyn MemoryStore,
) -> anyhow::Result<IngestResult> {
    ingest_with(
        id,
        key,
        content,
        category,
        _created_at_epoch,
        visibility,
        config,
        store,
        IngestOpts::default(),
    )
    .await
}

/// Strip `[Memory context] - key:` reference chains from the front of content.
/// These are ingest artifacts from nested memory retrieval and pollute classification.
fn clean_memory_context_prefixes(content: &str) -> String {
    let mut cleaned = content.trim().to_string();

    while cleaned.starts_with("[Memory context]") {
        if let Some(colon_pos) = cleaned.find(": ") {
            cleaned = cleaned[colon_pos + 2..].trim().to_string();
        } else {
            break;
        }
    }

    // If stripping left too little content, preserve the original.
    if cleaned.len() < 20 {
        return content.to_string();
    }

    cleaned
}

/// Run the ingestion pipeline with full metadata control.
#[allow(clippy::too_many_arguments)]
pub async fn ingest_with(
    id: &str,
    key: &str,
    content: &str,
    category: &str,
    _created_at_epoch: f64,
    visibility: &str,
    config: &IngestConfig,
    store: &dyn MemoryStore,
    opts: IngestOpts,
) -> anyhow::Result<IngestResult> {
    let content = clean_memory_context_prefixes(content);
    let content = content.as_str();
    let wing = opts
        .wing
        .unwrap_or_else(|| classifier::classify_wing(key, content, category, &config.wing_rules));
    let hall = classifier::classify_hall(content, &config.hall_rules);
    let signal_score = signal::score_memory(content, &hall);

    let now = Utc::now();
    let now_str = now.format("%Y-%m-%d %H:%M:%S").to_string();

    // Resolve episode_id: consumer-provided or auto-detected
    let episode_id = if let Some(ep_id) = opts.episode_id {
        // Consumer-provided episode_id — join or create that episode
        let existing = store
            .find_recent_episode(&wing, "1970-01-01 00:00:00")
            .await?;
        let is_existing = existing.as_ref().is_some_and(|e| e.id == ep_id);

        if is_existing {
            let mut ep = existing.unwrap();
            ep.memory_count += 1;
            ep.ended_at = now_str.clone();
            if signal_score > 0.5 {
                if let Some(ref prev) = ep.summary_preview {
                    if prev.len() < 10 || signal_score > 0.8 {
                        ep.summary_preview = Some(content.chars().take(200).collect());
                    }
                }
            }
            store.write_episode(&ep).await?;
        } else {
            let ep = Episode {
                id: ep_id.clone(),
                started_at: now_str.clone(),
                ended_at: now_str.clone(),
                memory_count: 1,
                wing: wing.clone(),
                summary_preview: Some(content.chars().take(200).collect()),
            };
            store.write_episode(&ep).await?;
        }
        Some(ep_id)
    } else {
        // Auto-detect: find recent episode in same wing within time gap
        let since = (now - chrono::Duration::minutes(EPISODE_GAP_MINUTES))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let recent = store.find_recent_episode(&wing, &since).await?;

        if let Some(mut ep) = recent {
            ep.memory_count += 1;
            ep.ended_at = now_str.clone();
            if signal_score > 0.5 {
                if let Some(ref prev) = ep.summary_preview {
                    if prev.len() < 10 || signal_score > 0.8 {
                        ep.summary_preview = Some(content.chars().take(200).collect());
                    }
                }
            }
            store.write_episode(&ep).await?;
            Some(ep.id)
        } else {
            // Create new episode with a deterministic ID from the memory ID
            let ep_id = format!("ep-{id}");
            let ep = Episode {
                id: ep_id.clone(),
                started_at: now_str.clone(),
                ended_at: now_str.clone(),
                memory_count: 1,
                wing: wing.clone(),
                summary_preview: Some(content.chars().take(200).collect()),
            };
            store.write_episode(&ep).await?;
            Some(ep_id)
        }
    };

    let memory = Memory {
        id: id.to_string(),
        key: key.to_string(),
        content: content.to_string(),
        wing: Some(wing.clone()),
        hall: Some(hall.clone()),
        signal_score,
        visibility: visibility.to_string(),
        source: opts.source,
        device_id: opts.device_id.map(|d| *d.as_bytes()),
        confidence: opts.confidence.unwrap_or(1.0),
        created_at: opts.created_at.map(|dt| dt.to_rfc3339()),
        last_reinforced_at: None,
        episode_id,
        compaction_tier: opts.compaction_tier,
    };

    let fingerprints = if signal_score >= config.signal_threshold {
        generate_fingerprints(&memory, config, store).await?
    } else {
        Vec::new()
    };

    store.write(&memory, &fingerprints).await?;

    Ok(IngestResult {
        memory,
        fingerprints,
    })
}

async fn generate_fingerprints(
    new_memory: &Memory,
    config: &IngestConfig,
    store: &dyn MemoryStore,
) -> anyhow::Result<Vec<Fingerprint>> {
    let wing = new_memory.wing.as_deref().unwrap_or("general");
    let new_hall = new_memory.hall.as_deref().unwrap_or("none");

    let peers = store
        .list_wing_memories(wing, config.signal_threshold)
        .await?;

    let mut fingerprints = Vec::with_capacity(peers.len());

    for peer in &peers {
        if peer.id == new_memory.id {
            continue;
        }
        let peer_hall = peer.hall.as_deref().unwrap_or("none");
        let fp_id = make_fp_id(&peer.id, &new_memory.id);
        let bucket = TimeBucket::Unknown;
        let hash = fingerprint::make_fingerprint_hash(peer_hall, new_hall, wing, bucket);

        fingerprints.push(Fingerprint {
            id: fp_id,
            hash,
            anchor_memory_id: peer.id.clone(),
            target_memory_id: new_memory.id.clone(),
            wing: wing.to_string(),
            anchor_hall: peer_hall.to_string(),
            target_hall: new_hall.to_string(),
            time_delta_bucket: bucket.to_string(),
        });
    }

    Ok(fingerprints)
}

/// Deterministic fingerprint row ID from two memory IDs.
fn make_fp_id(id_a: &str, id_b: &str) -> String {
    use sha2::{Digest, Sha256};
    let (first, second) = if id_a <= id_b {
        (id_a, id_b)
    } else {
        (id_b, id_a)
    };
    let raw = format!("fp|{}|{}", first, second);
    let digest = Sha256::digest(raw.as_bytes());
    format!(
        "{:016x}",
        u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 >= 8 bytes")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_memory_context_prefixes_strips_single() {
        let input = "[Memory context] - some_key: Decided to use Rust for the backend";
        let cleaned = clean_memory_context_prefixes(input);
        assert_eq!(cleaned, "Decided to use Rust for the backend");
    }

    #[test]
    fn clean_memory_context_prefixes_strips_double() {
        let input = "[Memory context] - outer_key: [Memory context] - inner_key: The actual content of this memory is here";
        let cleaned = clean_memory_context_prefixes(input);
        assert_eq!(cleaned, "The actual content of this memory is here");
    }

    #[test]
    fn clean_memory_context_prefixes_strips_triple() {
        let input = "[Memory context] - a: [Memory context] - b: [Memory context] - c: Real cognitive content about architecture decisions";
        let cleaned = clean_memory_context_prefixes(input);
        assert_eq!(
            cleaned,
            "Real cognitive content about architecture decisions"
        );
    }

    #[test]
    fn clean_memory_context_prefixes_leaves_clean_content() {
        let input = "Decided to use PostgreSQL for the production database";
        let cleaned = clean_memory_context_prefixes(input);
        assert_eq!(cleaned, input);
    }

    #[test]
    fn clean_memory_context_prefixes_fallback_when_too_short() {
        // After stripping, only "hi" remains (< 20 chars) — preserve original
        let input = "[Memory context] - key: hi";
        let cleaned = clean_memory_context_prefixes(input);
        assert_eq!(cleaned, input);
    }
}
