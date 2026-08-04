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

/// Recommended cap on constellation fingerprint fan-out per write, for
/// callers who opt into [`IngestConfig::max_fingerprint_peers`].
///
/// Measured on this repo's own corpus: 508 memories produced 70,796 edges
/// (~278 peers/memory) because the wing key collapses ~73% of memories into
/// `general`. That near-clique is what made per-write cost grow linearly with
/// corpus size. 64 keeps a dense recent neighbourhood while making ingest
/// cost flat. Not applied by default — see the field docs for why.
pub const DEFAULT_MAX_FINGERPRINT_PEERS: usize = 64;

/// Configuration for the ingestion pipeline.
#[derive(Debug, Clone)]
pub struct IngestConfig {
    /// Wing classification rules.
    pub wing_rules: Vec<(Regex, String)>,
    /// Hall classification rules.
    pub hall_rules: Vec<(Regex, String)>,
    /// Minimum signal_score for fingerprint generation (default 0.5).
    pub signal_threshold: f64,
    /// Maximum number of existing wing peers a new memory is paired with when
    /// generating constellation fingerprints. Default
    /// [`DEFAULT_MAX_FINGERPRINT_PEERS`] (64); `None` = unbounded, the legacy
    /// behaviour. Formerly the `SPECTRAL_MAX_FINGERPRINT_PEERS` env var
    /// (`0` = unbounded); env overrides now live only in the bench harness
    /// (`spectral_bench_accuracy::apply_env_levers`).
    ///
    /// Unbounded pairing is O(peers) per write and O(N^2) in stored rows,
    /// because ~73% of memories classify into the `general` wing and form a
    /// near-clique. Setting this to [`DEFAULT_MAX_FINGERPRINT_PEERS`] makes
    /// ingest cost flat in corpus size (measured 12.5x -> 1.6x growth over
    /// 800 writes, ~73% fewer stored edges).
    ///
    /// This is opt-in, not the default, because it is NOT retrieval-neutral:
    /// `time_delta_bucket` is part of the fingerprint hash, so bounding
    /// fan-out changes which (hall, bucket) hashes exist and therefore what
    /// `fingerprint_search` returns. Enable it after an end-to-end A/B on
    /// your own workload — the affected reader is the TACT tier-1 path, which
    /// this repo has separately measured at no retrieval effect.
    pub max_fingerprint_peers: Option<usize>,
    /// Generate constellation fingerprints at all. Default `true`
    /// (behaviour-preserving).
    ///
    /// Fingerprints cost **~39% of a write** and **~57% of store-layer bytes**
    /// (26.4 -> 11.6 KB/event), and their only production reader is TACT
    /// tier 1, which:
    ///
    /// - requires BOTH a wing and a hall to be detected on the *query*, which
    ///   holds for **3.2%** of LongMemEval-S questions (16/500), and only via
    ///   coincidental overlap with the demo-derived default wing keywords; and
    /// - has been separately measured at **0 wins, 2 losses, 9 ties** against
    ///   plain FTS when it does fire.
    ///
    /// Setting this to `false` trades that tier for a large ingest and storage
    /// win. See `docs/internal/fingerprint-retirement-2026-08-03.md`.
    pub fingerprints: bool,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            wing_rules: classifier::default_wing_rules(),
            hall_rules: classifier::default_hall_rules(),
            signal_threshold: 0.5,
            max_fingerprint_peers: Some(DEFAULT_MAX_FINGERPRINT_PEERS),
            fingerprints: true,
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
    pub write_outcome: crate::WriteOutcome,
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
        declarative_density: None, // Computed by Brain after ingest
        description: None,
        description_generated_at: None,
        content_hash: None,    // Computed by store.write()
        source_brain_id: None, // Stamped by Brain after write (signing)
        signature: None,
    };

    let fingerprints = if signal_score >= config.signal_threshold {
        generate_fingerprints(&memory, config, store).await?
    } else {
        Vec::new()
    };

    let write_outcome = store.write(&memory, &fingerprints).await?;

    Ok(IngestResult {
        memory,
        fingerprints,
        write_outcome,
    })
}

async fn generate_fingerprints(
    new_memory: &Memory,
    config: &IngestConfig,
    store: &dyn MemoryStore,
) -> anyhow::Result<Vec<Fingerprint>> {
    if !config.fingerprints {
        return Ok(Vec::new());
    }
    let wing = new_memory.wing.as_deref().unwrap_or("general");
    let new_hall = new_memory.hall.as_deref().unwrap_or("none");

    let peers = match config.max_fingerprint_peers {
        // +1: the new memory may itself already be in the wing listing and is
        // skipped below, so ask for one extra to still fill the cap.
        Some(cap) => {
            store
                .list_wing_memories_capped(wing, config.signal_threshold, cap.saturating_add(1))
                .await?
        }
        None => {
            store
                .list_wing_memories(wing, config.signal_threshold)
                .await?
        }
    };

    let mut fingerprints = Vec::with_capacity(peers.len());

    // Parse new memory's created_at for time-delta bucket computation
    let new_created_at = new_memory
        .created_at
        .as_deref()
        .and_then(parse_timestamp_secs);

    for peer in &peers {
        if peer.id == new_memory.id {
            continue;
        }
        let peer_hall = peer.hall.as_deref().unwrap_or("none");
        let fp_id = make_fp_id(&peer.id, &new_memory.id);
        let bucket = match (
            new_created_at,
            peer.created_at.as_deref().and_then(parse_timestamp_secs),
        ) {
            (Some(new_ts), Some(peer_ts)) => TimeBucket::from_delta_secs(new_ts - peer_ts),
            _ => TimeBucket::Older, // Default to Older (not Unknown) when timestamps unavailable
        };
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

    // The peer read asked for cap+1 so that a re-write of an already-stored
    // key (which appears in its own wing listing and is skipped above) still
    // fills the cap. Trim so the cap is exact in both cases.
    if let Some(cap) = config.max_fingerprint_peers {
        fingerprints.truncate(cap);
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

/// Parse a timestamp string to epoch seconds. Handles both SQLite datetime
/// format ("YYYY-MM-DD HH:MM:SS") and RFC3339.
fn parse_timestamp_secs(s: &str) -> Option<f64> {
    // SQLite datetime format
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp() as f64);
    }
    // RFC3339 (from opts.created_at)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp() as f64);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_timestamp_secs_sqlite_format() {
        let ts = parse_timestamp_secs("2024-06-15 12:00:00");
        assert!(ts.is_some());
    }

    #[test]
    fn parse_timestamp_secs_rfc3339_format() {
        let ts = parse_timestamp_secs("2024-06-15T12:00:00+00:00");
        assert!(ts.is_some());
    }

    #[test]
    fn time_bucket_from_timestamps() {
        let now = parse_timestamp_secs("2024-06-15 12:00:00").unwrap();
        let same_day = parse_timestamp_secs("2024-06-15 08:00:00").unwrap();
        let same_week = parse_timestamp_secs("2024-06-12 12:00:00").unwrap();
        let same_month = parse_timestamp_secs("2024-06-01 12:00:00").unwrap();
        let older = parse_timestamp_secs("2024-01-15 12:00:00").unwrap();

        assert_eq!(
            TimeBucket::from_delta_secs(now - same_day),
            TimeBucket::SameDay
        );
        assert_eq!(
            TimeBucket::from_delta_secs(now - same_week),
            TimeBucket::SameWeek
        );
        assert_eq!(
            TimeBucket::from_delta_secs(now - same_month),
            TimeBucket::SameMonth
        );
        assert_eq!(TimeBucket::from_delta_secs(now - older), TimeBucket::Older);
    }

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
