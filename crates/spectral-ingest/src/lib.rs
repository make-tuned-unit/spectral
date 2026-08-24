//! Memory ingestion pipeline for TACT (Topic-Aware Context Triage).
//!
//! Takes raw text, classifies it (wing/hall), computes signal_score,
//! generates constellation fingerprints, and writes to a [`MemoryStore`].
//! Fingerprint hashes are byte-identical to the production Python
//! implementation in `constellation.py` / `tact_retrieval.py`.

pub mod activity;
pub mod classifier;
pub mod federation_sync;
pub mod fingerprint;
pub mod ingest;
pub mod replicated_set;
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
    /// Prose description of this memory (written by external agents like Librarian).
    #[serde(default)]
    pub description: Option<String>,
    /// When the description was generated (ISO-8601).
    #[serde(default)]
    pub description_generated_at: Option<String>,
    /// Content hash for dedup (blake3 hex of content). `None` for pre-backfill rows.
    #[serde(default)]
    pub content_hash: Option<String>,
    /// Brain that authored this memory (32-byte `BrainId`). `None` for
    /// unsigned/legacy memories. Set at write time in a signed brain; the
    /// authenticated origin for multi-contributor federation.
    #[serde(default)]
    pub source_brain_id: Option<[u8; 32]>,
    /// Ed25519 signature over `(source_brain_id, key, content_hash,
    /// created_at, visibility)` (see
    /// `spectral_core::identity::memory_signing_payload_v2`). Binding the key
    /// is what stops a signed memory being re-served under a different key.
    /// `None` for unsigned/legacy memories. 64 bytes when present.
    #[serde(default)]
    pub signature: Option<Vec<u8>>,
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

// ── WriteOutcome ────────────────────────────────────────────────────

/// Outcome of a memory write operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    Inserted,
    NoOp,
    ContentUpdated,
}

// ── Forget receipt ──────────────────────────────────────────────────

/// Per-substrate deletion counts from a hard delete of one memory. Every
/// place a memory leaves a trace in the SQLite store is a field here, so a
/// caller can audit that right-to-be-forgotten actually reached each
/// substrate rather than trusting a single boolean. The recognition sidecar
/// and graph store live outside this store and are reported separately by
/// the `Brain`-level forget.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgetReceipt {
    /// Whether a `memories` row matched the key (the memory existed).
    pub existed: bool,
    /// `memories` rows deleted (0 or 1).
    pub memory_rows: usize,
    /// `constellation_fingerprints` rows removed (anchor or target).
    pub fingerprints: usize,
    /// `memory_spectrogram` rows removed.
    pub spectrograms: usize,
    /// `memory_annotations` rows removed.
    pub annotations: usize,
    /// `consolidation_edges` rows removed (as source or target).
    pub consolidation_edges: usize,
    /// `co_retrieval_pairs` rows removed (as either member).
    pub co_retrieval_pairs: usize,
    /// `retrieval_events` rows scrubbed (JSON referenced this memory id).
    pub retrieval_events: usize,
    /// `episodes` rows touched: summary previews scrubbed because they were
    /// derived from this memory's content (a verbatim 200-char prefix), plus
    /// the episode row itself when this was its last memory. Found by the
    /// deletion-guarantees D1 schema sweep — previews previously outlived
    /// the memory they quoted.
    pub episodes: usize,
    /// `memories_fts` rows removed (via the AFTER DELETE trigger).
    pub fts_rows: usize,
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
    /// Prose description of this memory (written by external agents like Librarian).
    #[serde(default)]
    pub description: Option<String>,
    /// Authoring brain (32-byte `BrainId`). `None` for unsigned/legacy
    /// memories. Carried on the hit so a federated consumer can attribute and
    /// verify it without a second lookup.
    #[serde(default)]
    pub source_brain_id: Option<[u8; 32]>,
    /// Ed25519 signature over the memory's signed payload. `None` for
    /// unsigned/legacy memories.
    #[serde(default)]
    pub signature: Option<Vec<u8>>,
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

// ── Entity fields (typed, with provenance) ──────────────────────────

/// Provenance of an entity field value.
///
/// `Manual` values are author-supplied (UI / direct entry); `Enriched`
/// values come from an automated enrichment pass. The store enforces that an
/// `Enriched` write never overwrites a field whose stored source is `Manual`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldSource {
    Manual,
    Enriched,
}

impl FieldSource {
    /// Canonical lowercase string stored in the DB.
    pub fn as_str(self) -> &'static str {
        match self {
            FieldSource::Manual => "manual",
            FieldSource::Enriched => "enriched",
        }
    }

    /// Parse from the DB string. Unknown values default to `Enriched` (the
    /// non-protected source) so a corrupt row can never masquerade as a
    /// manual value and block legitimate writes.
    pub fn from_db(s: &str) -> FieldSource {
        match s {
            "manual" => FieldSource::Manual,
            _ => FieldSource::Enriched,
        }
    }
}

/// A single typed field on a graph entity, carrying provenance.
///
/// `entity_id` is the entity's 64-hex content-addressed id (stored by value;
/// there is no DB-level foreign key — graph entities live in the SQLite graph store).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityField {
    pub field_name: String,
    pub value: String,
    pub source: FieldSource,
    pub source_url: Option<String>,
    pub updated_at: String,
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
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<WriteOutcome>> + Send + '_>>;

    /// Write a batch of memories, amortizing the per-write commit where the
    /// backend supports it (register row R7: per-event commit measured at 21%
    /// of ingest cost).
    ///
    /// **Explicit API, never a default:** batching changes durability
    /// semantics — a crash mid-batch loses the whole batch, not one event.
    /// Callers choose it knowingly for bulk paths (imports, replays, brain
    /// builds); the per-event `write` remains the default everywhere else.
    ///
    /// The default implementation is a sequential loop of [`write`](Self::write)
    /// with per-event durability — backends without native batching keep
    /// their existing semantics unchanged.
    fn write_batch<'a>(
        &'a self,
        items: &'a [(Memory, Vec<Fingerprint>)],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<WriteOutcome>>> + Send + 'a>> {
        Box::pin(async move {
            let mut outcomes = Vec::with_capacity(items.len());
            for (memory, fingerprints) in items {
                outcomes.push(self.write(memory, fingerprints).await?);
            }
            Ok(outcomes)
        })
    }

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

    /// The latest `created_at` across the corpus, as stored.
    ///
    /// A **deterministic time anchor** for recency decay.
    ///
    /// Recall *ordering* is already time-invariant (measured — see
    /// `docs/internal/decay-time-invariance-2026-08-03.md`), but the decayed
    /// `signal_score` values callers receive are not: they shrink as the clock
    /// advances. Anchoring to the corpus's own newest memory makes those
    /// scores reproducible too, and pins the ordering property against future
    /// changes to the decay function.
    ///
    /// Returns `None` for an empty corpus or when no memory carries a
    /// timestamp, in which case callers fall back to wall-clock.
    ///
    /// Default implementation returns `None` so alternative stores are not
    /// forced to implement it; they simply do not offer the anchor.
    fn latest_created_at(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<String>>> + Send + '_>> {
        Box::pin(async { Ok(None) })
    }

    /// Reassign a memory's wing. Returns `true` if a row changed.
    ///
    /// Exists for taxonomy repair: brains ingested while the library shipped
    /// demo-fixture wing rules carry memories filed into fictional topic areas
    /// (`alice`, `apollo`, `acme`, ...). See
    /// `docs/internal/wing-taxonomy-2026-08-03.md`.
    ///
    /// Default implementation is a no-op returning `false`.
    fn set_wing<'a>(
        &'a self,
        _key: &'a str,
        _wing: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }

    /// Set a memory's hall by id and re-hash the constellation fingerprints it
    /// participates in (R40). Returns `Ok(false)` when the id is unknown.
    /// Default implementation is a no-op returning `false`.
    fn set_hall<'a>(
        &'a self,
        _id: &'a str,
        _hall: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }

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
    /// Full-text search with **no** visibility boundary (equivalently a
    /// `Private` context, which admits every label).
    fn fts_search(
        &self,
        query_words: &[String],
        max_results: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryHit>>> + Send + '_>> {
        self.fts_search_scoped(
            query_words,
            max_results,
            spectral_core::visibility::Visibility::Private,
        )
    }

    /// Full-text search restricted to labels admissible in `visibility`.
    ///
    /// The predicate is applied in SQL **before** `LIMIT`, so `max_results` is
    /// filled from admissible rows. Filtering after the limit lets
    /// inadmissible rows consume the budget and can return zero hits from a
    /// store that holds matching admissible content.
    fn fts_search_scoped(
        &self,
        query_words: &[String],
        max_results: usize,
        visibility: spectral_core::visibility::Visibility,
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

    /// Reinforce many keys in a single transaction (batched auto-reinforce).
    /// Applies the same `MIN(signal_score + strength, 1.0)` nudge as
    /// [`reinforce_memory`](Self::reinforce_memory) to every key and
    /// invalidates the affected wing caches. Returns the number of rows updated.
    /// Default loops `reinforce_memory`; stores override for one round-trip.
    fn reinforce_batch<'a>(
        &'a self,
        keys: &'a [String],
        strength: f64,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + 'a>> {
        Box::pin(async move {
            let mut updated = 0usize;
            for key in keys {
                if self.reinforce_memory(key, strength).await?.is_some() {
                    updated += 1;
                }
            }
            Ok(updated)
        })
    }

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
    /// Like [`list_wing_memories`](Self::list_wing_memories) but returns at
    /// most `limit` peers, highest-signal first with a deterministic tiebreak.
    ///
    /// Used to bound constellation fingerprint fan-out, which is otherwise
    /// O(wing size) per write. The provided implementation is correct but
    /// still reads the full wing; backends should override it with a pushed-
    /// down LIMIT.
    fn list_wing_memories_capped(
        &self,
        wing: &str,
        min_signal: f64,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>> {
        let wing = wing.to_string();
        Box::pin(async move {
            let mut peers = self.list_wing_memories(&wing, min_signal).await?;
            peers.truncate(limit);
            Ok(peers)
        })
    }

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

    /// Hard-delete a single memory by key, purging every SQLite substrate it
    /// touches (row, FTS, fingerprints, spectrogram, annotations,
    /// consolidation edges, co-retrieval pairs, retrieval-event references)
    /// and returning a per-substrate receipt. Unlike `consolidate_into`
    /// (soft hide), the content is gone and unrecoverable from this store.
    /// Non-FK substrates (co-retrieval, retrieval events, consolidation
    /// edges as target) are scrubbed explicitly; FK-CASCADE substrates and
    /// the FTS trigger fire from the row delete.
    fn delete_memory_by_key(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ForgetReceipt>> + Send + '_>>;

    /// Stamp a memory's signed-provenance columns (`source_brain_id`,
    /// `signature`). Called after write, once the signable payload (the
    /// stored content hash, creation time, and visibility) is fixed. No-op
    /// (0 rows) if the id does not exist.
    fn set_signature(
        &self,
        memory_id: &str,
        source_brain_id: &[u8; 32],
        signature: &[u8],
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

    /// Rebuild the co-retrieval pairs index using only retrieval events whose
    /// `method` starts with one of `method_prefixes`. An empty slice means
    /// every method (identical to [`Self::rebuild_co_retrieval_index`]).
    ///
    /// Use [`TURN_EVENT_METHOD_PREFIX`] to build an outcome-credited index from
    /// `turn:*` events alone. See the implementation note on why mixing
    /// exposure-credited and outcome-credited rows makes the result
    /// uninterpretable.
    fn rebuild_co_retrieval_index_for_methods(
        &self,
        method_prefixes: &[String],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>>;

    /// Record a delivery in the turn ledger. Members start `Unreported`.
    fn record_turn_delivery(
        &self,
        delivery: &TurnDelivery,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Commit outcomes for a recorded turn AND reinforce the `Used` members,
    /// in ONE transaction.
    ///
    /// Atomicity is the point: a partial commit that reinforced without
    /// recording, or recorded without reinforcing, would make the ledger
    /// disagree with the signal scores it is supposed to explain. Replaying the
    /// same `occurrence_id` is a no-op — outcomes are already set and
    /// reinforcement is skipped — so retries are safe. Returns the number of
    /// members whose outcome changed (0 on replay).
    fn commit_turn_outcomes(
        &self,
        occurrence_id: &str,
        outcomes: &[(String, LedgerOutcome)],
        reinforce_strength: f64,
        committed_at: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>>;

    /// Void a turn that aborted before adjudication (cancelled reply, voice
    /// early-exit, crash mid-turn). A voided turn keeps its rows for audit
    /// but is EXCLUDED from outcome evidence — its memories were neither
    /// used nor ignored; the turn never finished. Errors if the turn was
    /// already committed (adjudicated evidence must not be erased);
    /// idempotent on an already-voided turn (returns `false`). Committing a
    /// voided turn likewise errors.
    fn void_turn(
        &self,
        occurrence_id: &str,
        voided_at: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>>;

    /// Aggregated delivery/use evidence per memory, most-delivered first.
    fn memory_outcome_evidence(
        &self,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryOutcomeEvidence>>> + Send + '_>>;

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

    // ── Description ──

    /// Fetch a memory by ID. Returns None if not found.
    fn get_memory(
        &self,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<Memory>>> + Send + '_>>;

    /// Set the description field on a memory and update description_generated_at to now.
    /// Returns Ok(()) on success, Err if memory not found or DB error.
    fn set_description(
        &self,
        id: &str,
        description: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// List memories where description IS NULL, ordered by created_at DESC, limited.
    fn list_undescribed(
        &self,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<Memory>>> + Send + '_>>;

    // ── Entity fields ──

    /// Write (insert-or-update) a single typed field on an entity, with
    /// provenance. `entity_id` is the entity's 64-hex content-addressed id.
    ///
    /// Provenance rule (enforced here, so it holds for every caller): an
    /// `Enriched` write must NOT overwrite a field whose stored source is
    /// `Manual`. When such a write is suppressed this returns `Ok(false)`;
    /// an applied write returns `Ok(true)`.
    fn set_entity_field(
        &self,
        entity_id: &str,
        field_name: &str,
        value: &str,
        source: FieldSource,
        source_url: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>>;

    /// Read all typed fields for an entity, ordered by field_name. Returns an
    /// empty vec when the entity has no fields.
    fn get_entity_fields(
        &self,
        entity_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<EntityField>>> + Send + '_>>;

    // ── Co-retrieval ──

    /// Return memories most frequently co-retrieved with the given memory_id,
    /// ordered by co_count DESC. Returns up to `limit` results.
    fn related_memories(
        &self,
        memory_id: &str,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<RelatedMemory>>> + Send + '_>>;

    /// Return memories linked to `memory_id` by constellation-fingerprint
    /// edges (write-time temporal-proximity constellations), ordered by edge
    /// multiplicity DESC then memory_id ASC (deterministic). Returns up to
    /// `limit` results. This is the memory↔memory adjacency substrate that
    /// exists even in a brain with no retrieval history (co-retrieval edges
    /// require prior recalls; fingerprint edges are written at ingest).
    fn fingerprint_neighbors(
        &self,
        memory_id: &str,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<FingerprintNeighbor>>> + Send + '_>>;

    /// Anticipatory-recall recommendation: memories associated with the seed
    /// ranked by **lift** (`co_count · total / (occ(seed) · occ(this))`)
    /// rather than raw `co_count`. Lift is the association measure recommender
    /// systems use to avoid recommending globally-popular items to everyone —
    /// it surfaces memories *specifically* associated with the seed's context,
    /// which is the fix for the popularity bias that sank raw co-retrieval.
    /// `min_co_count` filters noise (pairs seen fewer times). `None` ranks all.
    fn recommend_by_lift(
        &self,
        memory_id: &str,
        limit: usize,
        min_co_count: u64,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<RelatedMemory>>> + Send + '_>>;

    /// Rebuild the co_retrieval_pairs index from scratch using retrieval_events.
    /// Returns the number of pairs written.
    fn rebuild_co_retrieval_index(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>>;

    /// Durably associate a memory with its originating session and immediately
    /// add same-session pairs to the co-retrieval index. Idempotent for an
    /// existing `(memory_id, session_id)` association.
    fn associate_memory_session(
        &self,
        memory_id: &str,
        session_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>>;

    // ── Session queries ──

    /// List retrieval events for a given session, ordered by timestamp ASC.
    fn events_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<RetrievalEvent>>> + Send + '_>>;

    /// List unique memory IDs that surfaced in retrievals for a given session,
    /// ordered by first appearance.
    fn memories_for_session(
        &self,
        session_id: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<String>>> + Send + '_>>;

    /// Backfill content_hash for all rows with NULL content_hash.
    /// Returns count of rows updated. Idempotent — safe to re-run.
    fn backfill_content_hashes(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<usize>> + Send + '_>>;

    // ── Consolidation ──

    /// Mark source memories as consolidated into a target.
    /// Target must exist. Idempotent on same source→target pair.
    /// Flattens chains: if a source was previously a target, re-points its inbound edges.
    fn consolidate_into(
        &self,
        source_keys: &[String],
        target_key: &str,
        opts: &ConsolidateOpts,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ConsolidationResult>> + Send + '_>>;

    /// List consolidation edges, optionally filtered to a specific target.
    fn list_consolidated(
        &self,
        target_key: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ConsolidationEdge>>> + Send + '_>>;

    /// List memory keys that are NOT consolidated sources (available for recall).
    fn list_unconsolidated(
        &self,
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<String>>> + Send + '_>>;

    /// Return the set of source_keys that have been consolidated (for recall filtering).
    fn consolidated_source_keys(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<std::collections::HashSet<String>>> + Send + '_>>;
}

// ── RelatedMemory ──────────────────────────────────────────────────

/// A memory co-retrieved with another memory, with frequency count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedMemory {
    /// The related memory's ID.
    pub memory_id: String,
    /// Number of times the two memories co-occurred in retrievals.
    pub co_count: u64,
    /// **Lift** association: `P(this | seed) / P(this)` =
    /// `co_count · total / (occ(seed) · occ(this))`. >1 means this memory is
    /// *more* associated with the seed than its baseline popularity — the
    /// signal that suppresses globally-popular memories (which raw `co_count`
    /// ranking does not). `0.0` on the raw (`related_memories`) path.
    #[serde(default)]
    pub lift: f64,
    /// Full memory if cheap to join. `None` in v1 — caller fetches via `get_memory`.
    pub memory: Option<Memory>,
}

/// A memory linked to another by constellation-fingerprint edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FingerprintNeighbor {
    /// The neighboring memory's ID.
    pub memory_id: String,
    /// Number of fingerprint edges connecting the two memories.
    pub edge_count: u64,
}

// ── Consolidation types ────────────────────────────────────────────

/// Options for `consolidate_into()`.
#[derive(Debug, Clone)]
pub struct ConsolidateOpts {
    /// How to handle source keys that don't exist or are already consolidated elsewhere.
    pub on_invalid_source: InvalidSourcePolicy,
}

impl Default for ConsolidateOpts {
    fn default() -> Self {
        Self {
            on_invalid_source: InvalidSourcePolicy::SkipAndReport,
        }
    }
}

/// Policy for handling invalid source keys during consolidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidSourcePolicy {
    /// Abort the entire operation if any source is invalid.
    AbortAll,
    /// Skip invalid sources and report them in the result.
    SkipAndReport,
}

/// Result of a consolidation operation.
#[derive(Debug, Clone)]
pub struct ConsolidationResult {
    /// Keys that were successfully consolidated.
    pub consolidated: Vec<String>,
    /// Keys that were skipped with reasons.
    pub skipped: Vec<(String, SkipReason)>,
    /// Target memory's signal_score after merging.
    pub target_score_after: f64,
}

/// Reason a source was skipped during consolidation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// Source key does not exist in memories.
    SourceNotFound,
    /// Source is already consolidated into a different target.
    AlreadyConsolidatedElsewhere(String),
    /// Source key equals target key.
    SourceEqualsTarget,
}

/// A single consolidation edge (source→target).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationEdge {
    pub source_key: String,
    pub target_key: String,
    pub consolidated_at: String,
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
    /// Session/conversation ID for grouping retrievals. Consumer-managed opaque string.
    #[serde(default)]
    pub session_id: Option<String>,
}

/// `RetrievalEvent::method` prefix used by the turn path (`turn:v1`, …).
///
/// Turn events carry only the memories the caller reported as **used**;
/// `cascade` and other legacy methods carry the full returned set (exposure).
/// Filtering on this prefix is how an outcome-credited co-retrieval index is
/// built without dilution from exposure rows.
pub const TURN_EVENT_METHOD_PREFIX: &str = "turn:";

// ── Turn-outcome ledger ─────────────────────────────────────────────

/// One delivered member of a turn, with the outcome the caller reported.
///
/// `Unreported` is a first-class state, not a gap: a turn whose outcome is
/// never committed still recorded an *exposure*, and distinguishing "delivered
/// and explicitly ignored" from "delivered and never adjudicated" is the whole
/// point of persisting members rather than a flat id list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LedgerOutcome {
    Used,
    Wrong,
    Ignored,
    Unreported,
}

impl LedgerOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            LedgerOutcome::Used => "used",
            LedgerOutcome::Wrong => "wrong",
            LedgerOutcome::Ignored => "ignored",
            LedgerOutcome::Unreported => "unreported",
        }
    }

    /// Parse the stored column value. Deliberately infallible and NOT
    /// `FromStr`: an unrecognised value means an outcome we cannot attribute,
    /// and the safe reading of that is `Unreported` — never `Used`, which is
    /// the only variant that grants reinforcement.
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "used" => LedgerOutcome::Used,
            "wrong" => LedgerOutcome::Wrong,
            "ignored" => LedgerOutcome::Ignored,
            _ => LedgerOutcome::Unreported,
        }
    }
}

/// A delivery to record in the ledger, before any outcome is known.
#[derive(Debug, Clone)]
pub struct TurnDelivery {
    /// Identifies this real-world turn. Distinct per occurrence, even when two
    /// turns deliver byte-identical results.
    pub occurrence_id: String,
    /// Content-addressed digest of what was delivered; repeat deliveries share it.
    pub delivery_digest: String,
    pub query_hash: Option<String>,
    pub session_id: Option<String>,
    pub policy: String,
    pub delivered_at: String,
    /// (rank, memory_id, memory_key) in delivered order.
    pub members: Vec<(usize, String, String)>,
}

/// Aggregated outcome evidence for one memory, across every turn that
/// delivered it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryOutcomeEvidence {
    pub memory_id: String,
    pub memory_key: String,
    /// Times this memory was delivered to a caller (exposure).
    pub delivered: u64,
    pub used: u64,
    pub wrong: u64,
    pub ignored: u64,
    /// Delivered in a turn whose outcome was never committed.
    pub unreported: u64,
    /// Best (lowest) rank at which it was ever delivered.
    pub best_rank: Option<u64>,
}

impl MemoryOutcomeEvidence {
    /// Delivered at least `min_deliveries` times and never once used.
    ///
    /// This is the query the flat `retrieval_events` list structurally cannot
    /// answer, and the reason this ledger exists. It is deliberately exposed as
    /// *evidence*, not as an action: a memory can go unused because it was
    /// ranked 40th or duplicated elsewhere in context, so this must not be
    /// wired to automatic decay or forgetting without separate validation.
    pub fn delivered_never_used(&self, min_deliveries: u64) -> bool {
        self.delivered >= min_deliveries && self.used == 0
    }
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
    /// Legacy value from pre-PR#65 code that hardcoded all buckets as Unknown.
    /// Retained only for deserialization of old fingerprints.
    /// `backfill_fingerprint_time_buckets()` replaces these with real values.
    /// New code should never produce this variant.
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

    /// Inverse of [`as_str`](Self::as_str); `None` for an unknown label.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "same_day" => Some(Self::SameDay),
            "same_week" => Some(Self::SameWeek),
            "same_month" => Some(Self::SameMonth),
            "older" => Some(Self::Older),
            "unknown" => Some(Self::Unknown),
            _ => None,
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

#[cfg(test)]
mod time_bucket_tests {
    use super::*;

    /// `parse` must be the exact inverse of `as_str` for every variant, and
    /// reject anything else. Written per-arm because a single happy-path
    /// assertion leaves the other arms deletable without failing anything —
    /// and `set_hall` re-derives a fingerprint hash through this, so a wrong
    /// arm silently changes a stored hash rather than erroring.
    #[test]
    fn parse_round_trips_every_variant_and_rejects_the_rest() {
        for v in [
            TimeBucket::SameDay,
            TimeBucket::SameWeek,
            TimeBucket::SameMonth,
            TimeBucket::Older,
            TimeBucket::Unknown,
        ] {
            assert_eq!(
                TimeBucket::parse(v.as_str()),
                Some(v),
                "as_str/parse must round-trip {v:?} (label {:?})",
                v.as_str()
            );
        }
        // Each label maps to its OWN variant, not merely to something.
        assert_eq!(TimeBucket::parse("same_day"), Some(TimeBucket::SameDay));
        assert_eq!(TimeBucket::parse("same_week"), Some(TimeBucket::SameWeek));
        assert_eq!(TimeBucket::parse("same_month"), Some(TimeBucket::SameMonth));
        assert_eq!(TimeBucket::parse("older"), Some(TimeBucket::Older));
        assert_eq!(TimeBucket::parse("unknown"), Some(TimeBucket::Unknown));

        for bad in ["", "SameDay", "same day", "yesterday", "same_daily"] {
            assert_eq!(
                TimeBucket::parse(bad),
                None,
                "{bad:?} is not a bucket label"
            );
        }
    }
}
