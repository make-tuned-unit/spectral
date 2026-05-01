//! Top-level ingestion pipeline.

use regex::Regex;

use chrono::{DateTime, Utc};
use spectral_core::device_id::DeviceId;

use crate::classifier;
use crate::fingerprint;
use crate::signal;
use crate::{Fingerprint, Memory, MemoryStore, TimeBucket};

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
    let wing = classifier::classify_wing(key, content, category, &config.wing_rules);
    let hall = classifier::classify_hall(content, &config.hall_rules);
    let signal_score = signal::score_memory(content, &hall);

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
