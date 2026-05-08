//! Memory ingestion pipeline for TACT (Topic-Aware Context Triage).
//!
//! Takes raw text, classifies it (wing/hall), computes signal_score,
//! generates constellation fingerprints, and writes to a [`MemoryStore`].
//! Fingerprint hashes are byte-identical to the production Python
//! implementation in `constellation.py` / `tact_retrieval.py`.

pub mod activity;
pub mod classifier;
pub mod fingerprint;
pub mod ingest;
pub mod signal;
pub mod signal_scorer;
#[cfg(feature = "sqlite")]
pub mod sqlite_store;

pub use classifier::{default_hall_rule_strings, default_wing_rule_strings};
pub use signal_scorer::{DefaultSignalScorer, KeywordBooster, SignalScorer, SignalScorerConfig};

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

// ── Memory ──────────────────────────────────────────────────────────

/// A single memory record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub key: String,
    pub content: String,
    pub wing: Option<String>,
    pub hall: Option<String>,
    pub signal_score: f64,
    /// Visibility level. Defaults to `"private"` for fail-safe.
    #[serde(default = "default_visibility_str")]
    pub visibility: String,
    /// Where this memory came from (e.g. "native", "openbird_sidecar", "manual", "import").
    #[serde(default)]
    pub source: Option<String>,
    /// Which device originated this memory (raw 32-byte blake3 hash).
    #[serde(default)]
    pub device_id: Option<[u8; 32]>,
    /// Classification confidence, 0.0–1.0. Defaults to 1.0.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// When this memory was created (ISO-8601 string from SQLite).
    #[serde(default)]
    pub created_at: Option<String>,
    /// When this memory was last reinforced via the Memify feedback loop.
    #[serde(default)]
    pub last_reinforced_at: Option<String>,
    /// Episode this memory belongs to (if any).
    #[serde(default)]
    pub episode_id: Option<String>,
    /// Compaction tier for lifecycle management. `None` = untiered.
    #[serde(default)]
    pub compaction_tier: Option<CompactionTier>,
    /// Pre-computed declarative density (ratio of first-person declarative
    /// sentences). `None` = not yet computed (pre-backfill memories).
    #[serde(default)]
    pub declarative_density: Option<f64>,
}

/// Compaction tier for memory lifecycle management.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTier {
    Raw,
    HourlyRollup,
    DailyRollup,
    WeeklyRollup,
}

impl CompactionTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::HourlyRollup => "hourly_rollup",
            Self::DailyRollup => "daily_rollup",
            Self::WeeklyRollup => "weekly_rollup",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "raw" => Some(Self::Raw),
            "hourly_rollup" => Some(Self::HourlyRollup),
            "daily_rollup" => Some(Self::DailyRollup),
            "weekly_rollup" => Some(Self::WeeklyRollup),
            _ => None,
        }
    }
}

fn default_confidence() -> f64 {
    1.0
}

fn default_visibility_str() -> String {
    "private".to_string()
}

// ── Fingerprint ─────────────────────────────────────────────────────

/// A constellation fingerprint linking two memories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fingerprint {
    pub id: String,
    pub hash: String,
    pub anchor_memory_id: String,
    pub target_memory_id: String,
    pub wing: String,
    pub anchor_hall: String,
    pub target_hall: String,
    pub time_delta_bucket: String,
}

// ── MemoryHit ───────────────────────────────────────────────────────

/// A memory hit from any search method, with match quality metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHit {
    pub id: String,
    pub key: String,
    pub content: String,
    pub wing: Option<String>,
    pub hall: Option<String>,
    pub signal_score: f64,
    /// Visibility level.
    #[serde(default = "default_visibility_str")]
    pub visibility: String,
    /// Number of fingerprint/keyword matches that produced this hit.
    pub hits: usize,
    /// Where this memory came from.
    #[serde(default)]
    pub source: Option<String>,
    /// Which device originated this memory.
    #[serde(default)]
    pub device_id: Option<[u8; 32]>,
    /// Classification confidence, 0.0–1.0.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// When this memory was created.
    #[serde(default)]
    pub created_at: Option<String>,
    /// When this memory was last reinforced.
    #[serde(default)]
    pub last_reinforced_at: Option<String>,
    /// Episode this memory belongs to (if any).
    #[serde(default)]
    pub episode_id: Option<String>,
    /// Pre-computed declarative density. `None` = not yet computed.
    #[serde(default)]
    pub declarative_density: Option<f64>,
}

// ── Episode ────────────────────────────────────────────────────────

/// An episode groups temporally-related memories within a wing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: String,
    pub started_at: String,
    pub ended_at: String,
    pub memory_count: usize,
    pub wing: String,
    /// First ~200 chars of the highest-signal memory in the episode.
    pub summary_preview: Option<String>,
}

// ── Annotation ─────────────────────────────────────────────────────

/// A canonical entity reference. Spectral stores the string as-is and
/// does not validate format. Convention is consumer-defined (Permagent
/// uses prefixes like "person:", "project:", "did:chitin:").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntityRef {
    /// Canonical identifier stored as-provided. Consumer is responsible
    /// for canonicalization consistency (e.g., format stability, case
    /// normalization, alias resolution). Spectral does not validate or
    /// transform this value.
    pub canonical_id: String,
    /// Human-readable display name. May change without affecting
    /// canonical_id resolution. Used for UI rendering only.
    pub display_name: String,
}

/// A contextual annotation on a memory. Stores who/where/why/how
/// metadata produced by external agents (e.g., Permagent's Librarian).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryAnnotation {
    pub id: String,
    pub memory_id: String,
    pub description: String,
    pub who: Vec<EntityRef>,
    pub why: String,
    pub where_: Option<String>,
    pub when_: chrono::DateTime<chrono::Utc>,
    pub how: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Input for creating an annotation (without id/created_at).
#[derive(Debug, Clone)]
pub struct AnnotationInput {
    pub description: String,
    pub who: Vec<EntityRef>,
    pub why: String,
    pub where_: Option<String>,
    pub when_: chrono::DateTime<chrono::Utc>,
    pub how: String,
}

// ── SpectrogramRow ─────────────────────────────────────────────────

/// A row from the memory_spectrogram table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectrogramRow {
    pub memory_id: String,
    pub wing: Option<String>,
    pub entity_density: f64,
    pub action_type: String,
    pub decision_polarity: f64,
    pub causal_depth: f64,
    pub emotional_valence: f64,
    pub temporal_specificity: f64,
    pub novelty: f64,
    pub peak_dimensions: String,
}

// ── MemoryStore trait ───────────────────────────────────────────────

/// Unified trait abstracting the memory storage backend.
///
/// Combines write-side operations (used by ingestion) and read-side
/// operations (used by TACT retrieval).
pub trait MemoryStore: Send + Sync {
    // ── Write side ──

    /// Write a memory and its fingerprints to the store.
    fn write(
        &self,
        memory: &Memory,
        fingerprints: &[Fingerprint],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// List memories in the given wing with signal_score >= threshold.
    fn list_wing_memories(
        &self,
        wing: &str,
        min_signal: f64,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>>;

    /// List all memories with signal_score >= threshold, ordered by signal_score DESC.
    fn list_memories_by_signal(
        &self,
        min_signal: f64,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>>;

    // ── Read side ──

    /// Search by fingerprint hashes within a wing.
    fn fingerprint_search(
        &self,
        wing: &str,
        hall: &str,
        hashes: &[String],
        max_results: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryHit>>> + Send + '_>>;

    /// Retrieve high-signal memories for a wing with query-term boosting.
    fn wing_search(
        &self,
        wing: &str,
        query_terms: &[String],
        max_results: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryHit>>> + Send + '_>>;

    /// Full-text search fallback.
    fn fts_search(
        &self,
        query_words: &[String],
        max_results: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryHit>>> + Send + '_>>;

    /// Fetch full memory records by ID.
    fn fetch_by_ids(
        &self,
        ids: &[String],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>>;

    // ── Feedback ──

    /// Reinforce a memory by key: add `strength` to its signal_score (clamped to 1.0)
    /// and set last_reinforced_at to now. Returns the memory's wing (for cache invalidation)
    /// or None if the key was not found.
    fn reinforce_memory(
        &self,
        key: &str,
        strength: f64,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<String>>> + Send + '_>>;

    // ── Spectrogram ──

    /// Write a spectrogram record for a memory.
    #[allow(clippy::too_many_arguments)]
    fn write_spectrogram(
        &self,
        memory_id: &str,
        entity_density: f64,
        action_type: &str,
        decision_polarity: f64,
        causal_depth: f64,
        emotional_valence: f64,
        temporal_specificity: f64,
        novelty: f64,
        peak_dimensions: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Load spectrogram for a single memory. Returns None if no spectrogram exists.
    fn load_spectrogram(
        &self,
        memory_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<SpectrogramRow>>> + Send + '_>>;

    /// Load spectrograms, optionally filtering by wing. Returns (memory_id, wing, spectrogram data).
    fn load_spectrograms(
        &self,
        wing_filter: Option<&str>,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<SpectrogramRow>>> + Send + '_>>;

    /// List memory IDs that have no spectrogram yet.
    fn memories_without_spectrogram(
        &self,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<String>>> + Send + '_>>;

    // ── Activity / retention ──

    /// List memories in a wing created after `since` (ISO-8601), ordered by
    /// created_at DESC, up to `limit`.
    fn list_wing_memories_since(
        &self,
        wing: &str,
        since: &str,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>>;

    /// Delete memories in a wing created before `before` (ISO-8601).
    /// Returns the number of deleted rows.
    fn delete_wing_memories_before(
        &self,
        wing: &str,
        before: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>>;

    /// For each distinct `source` in a wing, keep only the most recent
    /// `keep` memories (by created_at), deleting the rest.
    /// Returns the total number of deleted rows.
    fn prune_wing_keeping_recent_per_source(
        &self,
        wing: &str,
        keep: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>>;

    // ── Episodes ──

    /// Write or update an episode record.
    fn write_episode(
        &self,
        episode: &Episode,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Find the most recent episode in a wing that ended within the given
    /// time window (ISO-8601 cutoff).
    fn find_recent_episode(
        &self,
        wing: &str,
        since: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<Episode>>> + Send + '_>>;

    /// List episodes, optionally filtered by wing.
    fn list_episodes(
        &self,
        wing: Option<&str>,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Episode>>> + Send + '_>>;

    /// Get all memories belonging to an episode.
    fn list_memories_by_episode(
        &self,
        episode_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>>;

    // ── Annotations ──

    /// Write an annotation on a memory.
    fn write_annotation(
        &self,
        annotation: &MemoryAnnotation,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// List all annotations for a memory.
    fn list_annotations(
        &self,
        memory_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryAnnotation>>> + Send + '_>>;

    // ── Compaction ──

    /// Set the compaction tier for a memory.
    fn set_compaction_tier(
        &self,
        memory_id: &str,
        tier: CompactionTier,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Backfill time_delta_bucket on existing fingerprints.
    /// Recomputes bucket from anchor/target memory timestamps.
    /// Returns number of fingerprints updated.
    fn backfill_fingerprint_time_buckets(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>>;

    // ── Retrieval events ──

    /// Log a retrieval event. Best-effort: failures should never block retrieval.
    fn log_retrieval_event(
        &self,
        event: &RetrievalEvent,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Count total retrieval events (for testing/diagnostics).
    fn count_retrieval_events(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>>;

    /// Set the declarative_density for a memory.
    fn set_declarative_density(
        &self,
        memory_id: &str,
        density: f64,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Count retrieval events filtered by method (for testing).
    fn count_retrieval_events_by_method(
        &self,
        method: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>>;
}

// ── RetrievalEvent ──────────────────────────────────────────────────

/// A recorded retrieval event for the recall→recognition feedback loop.
///
/// Captures what was retrieved, when, how, and for what query — enabling
/// downstream analysis of retrieval patterns, co-access mining, and
/// signal score evolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalEvent {
    /// Hash of the query string (for grouping similar queries without storing raw text).
    pub query_hash: String,
    /// ISO-8601 timestamp of retrieval.
    pub timestamp: String,
    /// Memory IDs returned by the retrieval (JSON array).
    pub memory_ids_json: String,
    /// Retrieval method: "cascade", "topk_fts", "tact", "graph", "probe".
    pub method: String,
    /// Classified wing (if any).
    pub wing: Option<String>,
    /// Question type from routing (if cascade): "Counting", "Temporal", etc.
    pub question_type: Option<String>,
}

/// Hash a query string for retrieval event grouping.
///
/// Returns full blake3 hex (64 chars). Used as a grouping key for
/// co-access mining, not a security primitive.
pub fn hash_query(query: &str) -> String {
    blake3::hash(query.as_bytes()).to_hex().to_string()
}

// ── TimeBucket ──────────────────────────────────────────────────────

/// Time delta buckets matching the production `constellation.py` algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeBucket {
    SameDay,
    SameWeek,
    SameMonth,
    Older,
    Unknown,
}

impl TimeBucket {
    /// Bucket the absolute time delta (in seconds) between two timestamps.
    pub fn from_delta_secs(delta_secs: f64) -> Self {
        let abs = delta_secs.abs();
        if abs < 86400.0 {
            Self::SameDay
        } else if abs < 604800.0 {
            Self::SameWeek
        } else if abs < 2592000.0 {
            Self::SameMonth
        } else {
            Self::Older
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SameDay => "same_day",
            Self::SameWeek => "same_week",
            Self::SameMonth => "same_month",
            Self::Older => "older",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for TimeBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
