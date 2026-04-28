//! Memory ingestion pipeline for TACT (Topic-Aware Context Triage).
//!
//! Takes raw text, classifies it (wing/hall), computes signal_score,
//! generates constellation fingerprints, and writes to a [`MemoryStore`].
//! Fingerprint hashes are byte-identical to the production Python
//! implementation in `constellation.py` / `tact_retrieval.py`.

pub mod classifier;
pub mod fingerprint;
pub mod ingest;
pub mod signal;
#[cfg(feature = "sqlite")]
pub mod sqlite_store;

pub use classifier::{default_hall_rule_strings, default_wing_rule_strings};

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
