//! # Spectral
//!
//! A frequency-domain memory system for AI agents, designed for federation.
//!
//! Spectral gives your agent two complementary memory systems:
//! - A **knowledge graph** (Kuzu) for typed entity relationships
//! - A **fingerprint store** (SQLite + FTS5) for fast topical retrieval
//!
//! Both are accessible through a single [`Brain`] handle.
//!
//! ## Quick start
//!
//! ```no_run
//! use spectral::{Brain, Visibility};
//!
//! // Open (or create) a brain with one line
//! let brain = Brain::open("./my-brain")?;
//!
//! // Remember free-text observations
//! brain.remember("auth-decision", "Decided to use Clerk for auth", Visibility::Private)?;
//!
//! // Recall with hybrid search (memory + graph)
//! let result = brain.recall_local("what was the auth decision")?;
//! for hit in &result.memory_hits {
//!     println!("[{}] {}", hit.key, hit.content);
//! }
//! # Ok::<(), spectral::Error>(())
//! ```
//!
//! ## With an ontology and graph assertions
//!
//! ```no_run
//! use spectral::{Brain, BrainBuilder, Visibility};
//!
//! let brain = Brain::builder()
//!     .data_dir("./my-brain")
//!     .ontology_path("./ontology.toml")
//!     .build()?;
//!
//! brain.assert("Alice", "knows", "Bob", 1.0, Visibility::Private)?;
//! let result = brain.recall_graph("Alice", Visibility::Private)?;
//! println!("{} triples", result.triples.len());
//! # Ok::<(), spectral::Error>(())
//! ```
//!
//! ## Crate architecture
//!
//! This umbrella crate re-exports the public API. Internally:
//!
//! | Crate | Role |
//! |---|---|
//! | `spectral-core` | Content-addressed IDs, identity, visibility |
//! | `spectral-graph` | Kuzu graph store, ontology, canonicalization, Brain API |
//! | `spectral-ingest` | Memory ingestion: classify, score, fingerprint (Constellation) |
//! | `spectral-tact` | TACT retrieval: fingerprint → wing → FTS search |
//! | `spectral-spectrogram` | *(reserved)* Phase 2 cognitive cross-wing matching |

#[cfg(feature = "http-llm")]
pub mod llm;

use std::path::{Path, PathBuf};

// ── Re-exports ──────────────────────────────────────────────────────

pub use spectral_core::device_id::DeviceId;
pub use spectral_core::visibility::Visibility;
pub use spectral_graph::activity::{
    ActivityEpisode, ComposeRedaction, DefaultRedactionPolicy, ExcludeBundlesPolicy,
    IngestActivityStats, NoOpRedactionPolicy, ProbeOpts, ProbeWindow, RecognizedMemory,
    RedactionPolicy, RollupStats,
};
pub use spectral_graph::brain::{
    AaakOpts, AaakResult, AssertResult, CrossWingRecallResult, EntityPolicy, HybridRecallResult,
    IngestResult, IngestTextOpts, IngestTextResult, RecallResult, RecallTopKConfig, ReinforceOpts,
    ReinforceResult, RejectedTriple, RejectionReason, RememberOpts, RememberResult,
    ResonantMemoryHit,
};
pub use spectral_graph::Error;
pub use spectral_ingest::{DefaultSignalScorer, KeywordBooster, SignalScorer, SignalScorerConfig};
pub use spectral_tact::LlmClient;

// Sub-crate access for advanced users
pub use spectral_core as core;
pub use spectral_graph as graph;
pub use spectral_ingest as ingest;
pub use spectral_spectrogram as spectrogram;
pub use spectral_tact as tact;

// ── Brain ───────────────────────────────────────────────────────────

/// A Spectral brain: knowledge graph + fingerprint memory store.
///
/// This is a thin wrapper around [`spectral_graph::brain::Brain`] that
/// provides a simpler constructor and re-exports all operations.
///
/// # Open with defaults
///
/// ```no_run
/// let brain = spectral::Brain::open("./my-brain").unwrap();
/// println!("Brain ID: {}", brain.brain_id());
/// ```
///
/// # Open with builder
///
/// ```no_run
/// let brain = spectral::Brain::builder()
///     .data_dir("./my-brain")
///     .ontology_path("./ontology.toml")
///     .build()
///     .unwrap();
/// ```
pub struct Brain {
    inner: spectral_graph::brain::Brain,
}

impl std::fmt::Debug for Brain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl Brain {
    /// Open or create a brain at the given path with sensible defaults.
    ///
    /// Uses `<path>/graph.kz` for the graph, `<path>/memory.db` for memories,
    /// `<path>/ontology.toml` if present (empty ontology otherwise),
    /// default wing/hall rules, and no LLM client.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        BrainBuilder::new()
            .data_dir(path.as_ref())
            .auto_ontology()
            .build()
    }

    /// Start building a brain with custom configuration.
    pub fn builder() -> BrainBuilder {
        BrainBuilder::new()
    }

    /// Returns this brain's stable identifier.
    pub fn brain_id(&self) -> &spectral_core::identity::BrainId {
        self.inner.brain_id()
    }

    /// Assert a fact: subject, predicate, object.
    ///
    /// Both sides are canonicalized through the ontology; the predicate
    /// is validated against domain/range constraints.
    pub fn assert(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: f64,
        visibility: Visibility,
    ) -> Result<AssertResult, Error> {
        self.inner
            .assert(subject, predicate, object, confidence, visibility)
    }

    /// Assert a triple with explicit types for subject and object.
    pub fn assert_typed(
        &self,
        subject: (&str, &str),
        predicate: &str,
        object: (&str, &str),
        confidence: f64,
        visibility: Visibility,
    ) -> Result<AssertResult, Error> {
        self.inner
            .assert_typed(subject, predicate, object, confidence, visibility)
    }

    /// Returns the device ID associated with this brain instance.
    pub fn device_id(&self) -> &DeviceId {
        self.inner.device_id()
    }

    /// Remember free-text content: classify, score, fingerprint, store.
    ///
    /// The `visibility` parameter controls who can see this memory during recall.
    pub fn remember(
        &self,
        key: &str,
        content: &str,
        visibility: Visibility,
    ) -> Result<RememberResult, Error> {
        self.inner.remember(key, content, visibility)
    }

    /// Remember free-text content with full metadata control.
    pub fn remember_with(
        &self,
        key: &str,
        content: &str,
        opts: RememberOpts,
    ) -> Result<RememberResult, Error> {
        self.inner.remember_with(key, content, opts)
    }

    /// Hybrid recall filtered by visibility context.
    ///
    /// A `Private` context sees everything; a `Public` context sees only
    /// `Public` content. See [`Visibility::allows`] for the full matrix.
    pub fn recall(
        &self,
        query: &str,
        context_visibility: Visibility,
    ) -> Result<HybridRecallResult, Error> {
        self.inner.recall(query, context_visibility)
    }

    /// Convenience: recall with maximally-permissive context (returns everything).
    pub fn recall_local(&self, query: &str) -> Result<HybridRecallResult, Error> {
        self.inner.recall_local(query)
    }

    /// Graph-only recall filtered by visibility context.
    pub fn recall_graph(
        &self,
        query: &str,
        context_visibility: Visibility,
    ) -> Result<RecallResult, Error> {
        self.inner.recall_graph(query, context_visibility)
    }

    /// Extract triples from natural-language text via LLM, validate against
    /// ontology, assert valid triples, and store the original text as a memory.
    pub fn ingest_text(&self, text: &str, opts: IngestTextOpts) -> Result<IngestTextResult, Error> {
        self.inner.ingest_text(text, opts)
    }

    /// Find memories across wings that resonate with a query memory's cognitive fingerprint.
    pub fn recall_cross_wing(
        &self,
        seed_query: &str,
        visibility: Visibility,
        max_results: usize,
    ) -> Result<CrossWingRecallResult, Error> {
        self.inner
            .recall_cross_wing(seed_query, visibility, max_results)
    }

    /// Compute and store spectrograms for memories that don't have one.
    pub fn backfill_spectrograms(&self) -> Result<usize, Error> {
        self.inner.backfill_spectrograms()
    }

    /// Reinforce memories that the caller found useful from a recall result.
    pub fn reinforce(&self, opts: ReinforceOpts) -> Result<ReinforceResult, Error> {
        self.inner.reinforce(opts)
    }

    /// Returns the agent's foundational facts as a token-budgeted context
    /// string suitable for system prompt injection (AAAK / L1 curated memory).
    pub fn aaak(&self, opts: AaakOpts) -> Result<AaakResult, Error> {
        self.inner.aaak(opts)
    }

    /// Ingest a document: hash content, create document node, link mentions.
    pub fn ingest_document(
        &self,
        source: &str,
        content: &str,
        visibility: Visibility,
    ) -> Result<IngestResult, Error> {
        self.inner.ingest_document(source, content, visibility)
    }

    /// Probe: given a context string (e.g., recent activity text), find
    /// memories that are relevant to the current cognitive state.
    ///
    /// This is the recognition-mode entry point. Unlike `recall` (which is
    /// query-initiated: "what do I know about X?"), probe is system-initiated:
    /// "given what the user is doing, what related knowledge exists?"
    pub fn probe(&self, context: &str, opts: ProbeOpts) -> Result<Vec<RecognizedMemory>, Error> {
        self.inner.probe(context, opts)
    }

    /// Probe recent activity: synthesizes recent activity-wing memories into
    /// a context string and probes the brain for related knowledge.
    ///
    /// This is the ambient-awareness entry point. Consumers call this
    /// periodically (e.g., on each chat turn) to surface relevant memories
    /// from the user's recent activity without an explicit query.
    pub fn probe_recent(
        &self,
        window: ProbeWindow,
        opts: ProbeOpts,
    ) -> Result<Vec<RecognizedMemory>, Error> {
        self.inner.probe_recent(window, opts)
    }

    /// Top-K FTS retrieval with additive re-ranking. Zero LLM cost.
    pub fn recall_topk_fts(
        &self,
        query: &str,
        config: &RecallTopKConfig,
        visibility: Visibility,
    ) -> Result<Vec<spectral_ingest::MemoryHit>, Error> {
        self.inner.recall_topk_fts(query, config, visibility)
    }

    /// Run the integrated cascade pipeline with ambient boost.
    ///
    /// Unlike [`recall()`](Brain::recall), this path takes a
    /// [`RecognitionContext`](spectral_graph::RecognitionContext) and applies
    /// wing-match, recency, and ambient boost in the re-ranking pipeline.
    pub fn recall_cascade(
        &self,
        query: &str,
        context: &spectral_graph::RecognitionContext,
        config: &spectral_cascade::orchestrator::CascadeConfig,
    ) -> Result<spectral_cascade::result::CascadeResult, Error> {
        self.inner.recall_cascade(query, context, config)
    }

    /// Rebuild the co-retrieval pairs index from accumulated retrieval events.
    ///
    /// Full recompute (not incremental). Atomic replace via single transaction —
    /// concurrent reads are safe. Idempotent. Returns the number of pairs written.
    pub fn rebuild_co_retrieval_index(&self) -> Result<usize, Error> {
        self.inner.rebuild_co_retrieval_index()
    }

    /// Direct access to the underlying graph store.
    pub fn store(&self) -> &spectral_graph::kuzu_store::KuzuStore {
        self.inner.store()
    }

    /// Direct access to the ontology.
    pub fn ontology(&self) -> &spectral_graph::ontology::Ontology {
        self.inner.ontology()
    }

    /// Fetch a memory by ID. Returns None if not found.
    pub fn get_memory(&self, id: &str) -> Result<Option<spectral_ingest::Memory>, Error> {
        self.inner.get_memory(id)
    }

    /// Set the description field on a memory and update description_generated_at.
    pub fn set_description(&self, id: &str, description: &str) -> Result<(), Error> {
        self.inner.set_description(id, description)
    }

    /// List memories where description IS NULL, ordered by created_at DESC.
    pub fn list_undescribed(&self, limit: usize) -> Result<Vec<spectral_ingest::Memory>, Error> {
        self.inner.list_undescribed(limit)
    }

    /// Annotate a memory with contextual who/where/why/how metadata.
    ///
    /// Writes a [`spectral_ingest::MemoryAnnotation`] row to the
    /// `memory_annotations` table. Idempotent on
    /// `(memory_id, description, when_)`: if an identical annotation
    /// already exists the call is a no-op and the existing row is returned.
    pub fn annotate(
        &self,
        memory_id: &str,
        input: spectral_ingest::AnnotationInput,
    ) -> Result<spectral_ingest::MemoryAnnotation, Error> {
        self.inner.annotate(memory_id, input)
    }

    /// List all annotations for a memory. Read-only, returns an empty
    /// Vec when no annotations exist for the given memory_id.
    pub fn list_annotations(
        &self,
        memory_id: &str,
    ) -> Result<Vec<spectral_ingest::MemoryAnnotation>, Error> {
        self.inner.list_annotations(memory_id)
    }

    /// Update the `compaction_tier` on an existing memory.
    ///
    /// Used by rollup consumers (e.g., Permagent's Librarian) to track
    /// compaction state as ambient-stream memories are aggregated from
    /// `Raw` → `HourlyRollup` → `DailyRollup` → `WeeklyRollup`.
    /// Idempotent: setting the same tier twice is a no-op. Writes a
    /// single UPDATE to the `memories` table.
    pub fn set_compaction_tier(
        &self,
        memory_id: &str,
        tier: spectral_ingest::CompactionTier,
    ) -> Result<(), Error> {
        self.inner.set_compaction_tier(memory_id, tier)
    }

    /// List episodes, optionally filtered by wing.
    ///
    /// Read-only scan of the `episodes` table, ordered by `started_at`
    /// descending, up to `limit` rows. Pass `None` for `wing` to list
    /// across all wings. Cost is O(limit) — bounded by the limit parameter.
    pub fn list_episodes(
        &self,
        wing: Option<&str>,
        limit: usize,
    ) -> Result<Vec<spectral_ingest::Episode>, Error> {
        self.inner.list_episodes(wing, limit)
    }

    /// Get all memories belonging to an episode.
    ///
    /// Read-only. Returns memories ordered by `created_at` ascending.
    /// Returns an empty Vec if the episode_id does not exist.
    pub fn list_memories_by_episode(
        &self,
        episode_id: &str,
    ) -> Result<Vec<spectral_ingest::Memory>, Error> {
        self.inner.list_memories_by_episode(episode_id)
    }

    /// Return memories most frequently co-retrieved with the given memory_id.
    ///
    /// Reads from the `co_retrieval_pairs` table (populated by
    /// [`rebuild_co_retrieval_index`](Brain::rebuild_co_retrieval_index)).
    /// Returns up to `limit` results ordered by co-occurrence count
    /// descending. Returns an empty Vec if the memory_id has no
    /// co-retrieval data or if the index has not been built yet.
    pub fn related_memories(
        &self,
        memory_id: &str,
        limit: usize,
    ) -> Result<Vec<spectral_ingest::RelatedMemory>, Error> {
        self.inner.related_memories(memory_id, limit)
    }

    /// Count all retrieval events in the database.
    ///
    /// Read-only. Scans the `retrieval_events` table. Useful for
    /// verifying that the feedback loop is logging events.
    pub fn count_retrieval_events(&self) -> Result<usize, Error> {
        self.inner.count_retrieval_events()
    }

    /// Count retrieval events filtered by method (e.g., `"cascade"`,
    /// `"topk_fts"`).
    ///
    /// Read-only. Scans `retrieval_events` with a WHERE clause on
    /// the `method` column.
    pub fn count_retrieval_events_by_method(&self, method: &str) -> Result<usize, Error> {
        self.inner.count_retrieval_events_by_method(method)
    }

    /// List retrieval events for a given session, ordered by timestamp ASC.
    ///
    /// Read-only. Queries the `retrieval_events` table filtered by
    /// `session_id`, up to `limit` rows. Returns an empty Vec if no
    /// events exist for the session.
    pub fn events_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<spectral_ingest::RetrievalEvent>, Error> {
        self.inner.events_for_session(session_id, limit)
    }

    /// List unique memory IDs that surfaced in a session, ordered by
    /// first appearance.
    ///
    /// Read-only. Extracts distinct memory IDs from `retrieval_events`
    /// for the given `session_id`. Returns an empty Vec if no events
    /// exist for the session.
    pub fn memories_for_session(&self, session_id: &str) -> Result<Vec<String>, Error> {
        self.inner.memories_for_session(session_id)
    }
}

// ── BrainBuilder ────────────────────────────────────────────────────

/// Builder for configuring a [`Brain`].
///
/// ```no_run
/// let brain = spectral::Brain::builder()
///     .data_dir("./my-brain")
///     .ontology_path("./ontology.toml")
///     .build()
///     .unwrap();
/// ```
#[derive(Default)]
pub struct BrainBuilder {
    data_dir: Option<PathBuf>,
    ontology_path: Option<PathBuf>,
    memory_db_path: Option<PathBuf>,
    llm_client: Option<Box<dyn LlmClient>>,
    wing_rules: Option<Vec<(String, String)>>,
    hall_rules: Option<Vec<(String, String)>>,
    device_id: Option<DeviceId>,
    enable_spectrogram: bool,
    entity_policy: Option<EntityPolicy>,
    auto_ontology: bool,
}

impl std::fmt::Debug for BrainBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrainBuilder")
            .field("data_dir", &self.data_dir)
            .field("ontology_path", &self.ontology_path)
            .finish_non_exhaustive()
    }
}

impl BrainBuilder {
    fn new() -> Self {
        Self::default()
    }

    /// Set the data directory (required).
    pub fn data_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(path.into());
        self
    }

    /// Set the ontology TOML file path.
    pub fn ontology_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.ontology_path = Some(path.into());
        self.auto_ontology = false;
        self
    }

    /// Set the SQLite memory database path (default: `<data_dir>/memory.db`).
    pub fn memory_db_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.memory_db_path = Some(path.into());
        self
    }

    /// Set an LLM client for TACT classification.
    pub fn llm_client(mut self, client: Box<dyn LlmClient>) -> Self {
        self.llm_client = Some(client);
        self
    }

    /// Set custom wing detection rules.
    pub fn wing_rules(mut self, rules: Vec<(String, String)>) -> Self {
        self.wing_rules = Some(rules);
        self
    }

    /// Set custom hall detection rules.
    pub fn hall_rules(mut self, rules: Vec<(String, String)>) -> Self {
        self.hall_rules = Some(rules);
        self
    }

    /// Set a device identifier for this brain instance.
    pub fn device_id(mut self, id: DeviceId) -> Self {
        self.device_id = Some(id);
        self
    }

    /// Set the entity policy for assert(). Default is Strict.
    pub fn entity_policy(mut self, policy: EntityPolicy) -> Self {
        self.entity_policy = Some(policy);
        self
    }

    /// Enable cognitive spectrogram computation on ingest.
    pub fn enable_spectrogram(mut self, enabled: bool) -> Self {
        self.enable_spectrogram = enabled;
        self
    }

    /// Use `<data_dir>/ontology.toml` if it exists, or an empty ontology.
    fn auto_ontology(mut self) -> Self {
        self.auto_ontology = true;
        self
    }

    /// Build and open the brain.
    pub fn build(self) -> Result<Brain, Error> {
        let data_dir = self
            .data_dir
            .ok_or_else(|| Error::Schema("data_dir is required".into()))?;

        let ontology_path = if let Some(p) = self.ontology_path {
            p
        } else if self.auto_ontology {
            let candidate = data_dir.join("ontology.toml");
            if !candidate.exists() {
                std::fs::create_dir_all(&data_dir)?;
                std::fs::write(&candidate, "version = 1\n")?;
            }
            candidate
        } else {
            return Err(Error::Schema(
                "ontology_path is required (use .ontology_path() or Brain::open())".into(),
            ));
        };

        let config = spectral_graph::brain::BrainConfig {
            data_dir,
            ontology_path,
            memory_db_path: self.memory_db_path,
            llm_client: self.llm_client,
            wing_rules: self.wing_rules,
            hall_rules: self.hall_rules,
            device_id: self.device_id,
            enable_spectrogram: self.enable_spectrogram,
            entity_policy: self.entity_policy.unwrap_or_default(),
            sqlite_mmap_size: None,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        };

        let inner = spectral_graph::brain::Brain::open(config)?;
        Ok(Brain { inner })
    }
}
