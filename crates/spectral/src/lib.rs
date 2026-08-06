//! # Spectral
//!
//! Deterministic, embedding-free memory for AI agents: recall, recognition,
//! and adaptive feedback. Local-first and federation-ready, on one SQLite
//! file — no vector DB, no LLM on the recall path.
//!
//! Spectral gives your agent complementary recall, recognition, relational,
//! episodic, adaptive, and federated memory over one embedded SQLite
//! database, accessible through a single [`Brain`] handle.
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
//! ## Agent turns: learning from outcomes, not exposure
//!
//! [`Brain::turn`] is the recommended path for an agent loop. It retrieves
//! **read-only** and hands back a receipt; nothing is reinforced until you say
//! what the agent actually used. The plain `recall_*` methods auto-reinforce
//! every hit at retrieval time, which credits exposure rather than usefulness.
//!
//! ```no_run
//! use spectral::{Brain, MemoryOutcome, TurnRequest, Visibility};
//!
//! let brain = Brain::open("./my-brain")?;
//!
//! // 1. Retrieve. Nothing is written; recognition runs only over observations.
//! let turn = brain.turn(
//!     &TurnRequest::query("what was the auth decision", Visibility::Private)
//!         .with_observations(&["user just pasted a Clerk config file"]),
//! )?;
//!
//! // 2. ... the agent answers, using some of what it got back ...
//!
//! // 3. Report outcomes. Only `Used` is reinforced.
//! if let Some(hit) = turn.hits.first() {
//!     brain.record_turn_outcome(
//!         &turn.receipt,
//!         &[(hit.key.as_str(), MemoryOutcome::Used)],
//!     )?;
//! }
//! # Ok::<(), spectral::Error>(())
//! ```
//!
//! A turn that is never committed leaves memory state completely unchanged.
//! See the [`turn`] module for the full contract.
//!
//! ## Crate architecture
//!
//! This umbrella crate re-exports the public API. Internally:
//!
//! | Crate | Role |
//! |---|---|
//! | `spectral-core` | Content-addressed entity IDs, Ed25519 brain identity, device IDs, visibility levels |
//! | `spectral-graph` | SQLite graph store, ontology, canonicalization, federation coordinator, and the `Brain` implementation |
//! | `spectral-ingest` | Memory ingestion: classify, signal-score, fingerprint, store (memories, FTS, episodes) |
//! | `spectral-tact` | Retrieval; the production recall path is deterministic FTS5 + BM25 with re-ranking — the fingerprint/wing tiers are measured as adding nothing over FTS |
//! | `spectral-cascade` | Recognition context and result types for the retrieval pipeline |
//! | `spectral-recognition` | Embedding-free recognition ("have I seen this before?") with auditable verdicts; no accuracy claim over classical baselines |
//! | `spectral-spectrogram` | Retired as a recall path (0/500 retrieval contexts changed); behind the off-by-default `spectrogram-legacy` feature, retained for the recognition experiments' history |
//! | `spectral-archivist` | Opt-in maintenance: dedup, gap detection, reclassification, decay, consolidation candidates |
//! | `spectral-bench-accuracy`, `spectral-bench-real` | Benchmark harnesses; not part of the library API |
//!
//! The measured record behind these one-liners — including the negative
//! results — is indexed in `docs/MEASURED_RECORD.md`.

pub mod answerability;
#[cfg(feature = "http-llm")]
pub mod llm;
pub mod policy;
pub mod render;
pub mod retrieve;
pub mod supersession;
pub mod temporal;
pub mod turn;

pub use answerability::{AnswerType, AnswerabilityConfig};
pub use policy::{QuestionShape, RetrievalPolicyVersion, RetrievalRoute};
pub use render::{RenderOptions, SessionOrder};
pub use retrieve::{retrieve, RetrievePlan, Retrieved};
pub use supersession::{SupersessionConfig, SupersessionReport};
pub use temporal::{resolve_relative_dates, Certainty, ResolvedDate};

pub use turn::{
    DeliveredHit, MemoryOutcome, OutcomeReceipt, TurnPolicyVersion, TurnReceipt, TurnRequest,
    TurnResult,
};

// `TurnRequest` carries a `RecognitionContext`, so it must be reachable from
// the crate root rather than only via `spectral::graph`.
pub use spectral_graph::RecognitionContext;

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
    AaakOpts, AaakResult, AssertResult, DerivationHealthReport, DerivationRepairReport,
    EntityPolicy, HybridRecallResult, IngestResult, IngestTextOpts, IngestTextResult, RecallResult,
    RecallTopKConfig, ReinforceOpts, ReinforceResult, RejectedTriple, RejectionReason,
    RememberOpts, RememberResult, VerificationStatus, WingReclassifyReport,
};
// Spectrogram-as-recall is retired: 0/500 contexts changed in the Tier-0
// retrieval oracle (docs/internal/ORACLE_TIER0.md). The facade re-exports of
// `CrossWingRecallResult` and `ResonantMemoryHit` were deleted outright —
// Permagent, the only consumer, never used them. The deprecated types remain
// in `spectral_graph::brain` behind the off-by-default `spectrogram-legacy`
// feature for historical experiments.
pub use spectral_graph::Error;
pub use spectral_ingest::{DefaultSignalScorer, KeywordBooster, SignalScorer, SignalScorerConfig};
pub use spectral_recognition::{Evidence, RecognitionResult, TraceMatch, Verdict};
pub use spectral_tact::LlmClient;

// Re-export chrono types used in the public API surface (recall_at, recall_local_at)
// so consumers don't need to pin chrono as a direct dependency.
pub use chrono::{DateTime, Utc};

/// Stable retrieval profiles for [`Brain::recall_with`].
///
/// Profiles are intentionally coarse. Callers needing experimental tuning can
/// still use `recall_cascade_scoped` with a full `CascadePipelineConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallProfile {
    /// Lowest-complexity deterministic cascade with adaptive context disabled.
    Fast,
    /// Validated default cascade: signal, recency, density, and dedup enabled.
    Balanced,
    /// Balanced retrieval plus caller-provided ambient recognition context.
    Adaptive,
}

/// Options for the canonical, visibility-safe integrated recall path.
#[derive(Debug, Clone)]
pub struct RecallOptions {
    /// Required visibility boundary. There is deliberately no `Default` impl,
    /// so external/federated callers cannot accidentally omit this decision.
    pub visibility: Visibility,
    pub profile: RecallProfile,
    pub context: spectral_graph::RecognitionContext,
}

impl RecallOptions {
    pub fn new(visibility: Visibility) -> Self {
        Self {
            visibility,
            profile: RecallProfile::Balanced,
            context: spectral_graph::RecognitionContext::empty(),
        }
    }

    pub fn profile(mut self, profile: RecallProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn context(mut self, context: spectral_graph::RecognitionContext) -> Self {
        self.context = context;
        self
    }
}

// Sub-crate access for advanced users
pub use spectral_core as core;
pub use spectral_graph as graph;
pub use spectral_ingest as ingest;
pub use spectral_recognition as recognition;
// Retired as a recall path (0/500 contexts changed, ORACLE_TIER0); only
// re-exported behind the off-by-default `spectrogram-legacy` feature.
#[cfg(feature = "spectrogram-legacy")]
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
    /// Uses `<path>/memory.db` for graph, memories, and full-text indexes,
    /// plus `<path>/recognition.db` for the recognition sidecar,
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

    /// Returns this brain's public verifying key (for peers to verify its
    /// signed contributions).
    pub fn verifying_key(&self) -> &spectral_core::identity::VerifyingKey {
        self.inner.verifying_key()
    }

    /// Verify a memory hit's signed provenance against a contributor's public
    /// key. `true` only if the hit is signed, the key matches its
    /// `source_brain_id`, and the signature covers its content, timestamp,
    /// and visibility. See [`spectral_graph::brain::Brain::verify_hit`].
    pub fn verify_hit(
        hit: &spectral_ingest::MemoryHit,
        pubkey: &spectral_core::identity::VerifyingKey,
    ) -> bool {
        spectral_graph::brain::Brain::verify_hit(hit, pubkey)
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
    ///
    /// **Time anchor defaults to `Utc::now()`** — correct for live queries,
    /// wrong for historical replay. Use [`recall_at()`](Self::recall_at) to
    /// anchor recency decay to the query date.
    pub fn recall(
        &self,
        query: &str,
        context_visibility: Visibility,
    ) -> Result<HybridRecallResult, Error> {
        self.inner
            .recall(query, context_visibility)
            .map(|r| Self::regroup_context_block(r, None))
    }

    /// Hybrid recall with an explicit time anchor for recency decay.
    ///
    /// Identical to [`recall()`](Self::recall) but uses `now` instead of
    /// `Utc::now()` for signal-score decay.
    pub fn recall_at(
        &self,
        query: &str,
        context_visibility: Visibility,
        now: DateTime<Utc>,
    ) -> Result<HybridRecallResult, Error> {
        self.inner
            .recall_at(query, context_visibility, now)
            .map(|r| Self::regroup_context_block(r, Some(now)))
    }

    /// R11 (BREAKING for consumers parsing the old block): `context_block` is
    /// now [`render::session_grouped`] — dated, session-grouped, role-tagged —
    /// instead of the undated TACT bundle. Measured on held-out LoCoMo with
    /// byte-identical retrieval: +19.2pp (dev) / **+14.2pp (disjoint
    /// validation, McNemar p=4.9e-4)**, the entire effect temporal-reasoning
    /// (a temporal question over an undated context is guesswork). The hits
    /// themselves are untouched; only the rendering changes.
    /// See `docs/internal/r11-render-ab-stage2-result-2026-08-06.md`.
    fn regroup_context_block(
        mut result: HybridRecallResult,
        now: Option<DateTime<Utc>>,
    ) -> HybridRecallResult {
        let now_str = now.map(|n| n.to_rfc3339());
        let mut opts = render::RenderOptions::published();
        opts.question_date = now_str.as_deref();
        result.tact.context_block =
            render::session_grouped(&result.tact.memories, &opts).join("\n");
        result
    }

    /// Convenience: recall with maximally-permissive context (returns everything).
    ///
    /// **Time anchor defaults to `Utc::now()`** — use
    /// [`recall_local_at()`](Self::recall_local_at) for historical queries.
    /// The latest `created_at` in this brain — a deterministic time anchor for
    /// recency decay. See
    /// [`spectral_graph::brain::Brain::latest_interaction_time`].
    ///
    /// Pass it to [`retrieve::RetrievePlan::with_time_anchor`] (or use
    /// [`retrieve::RetrievePlan::reproducible`]) so the decayed scores a
    /// caller reads depend on what the brain contains rather than on when the
    /// query ran. Ordering is already time-invariant — measured, see
    /// `docs/internal/decay-time-invariance-2026-08-03.md`.
    pub fn latest_interaction_time(&self) -> Result<Option<DateTime<Utc>>, Error> {
        self.inner.latest_interaction_time()
    }

    /// Re-run wing classification with the current rules. `apply = false` is a
    /// dry run. See [`spectral_graph::brain::Brain::reclassify_wings`].
    pub fn reclassify_wings(&self, apply: bool) -> Result<WingReclassifyReport, Error> {
        self.inner.reclassify_wings(apply)
    }

    /// Reclassify only memories currently in `only_wings` — the safe form of
    /// taxonomy repair. See
    /// [`spectral_graph::brain::Brain::reclassify_wings_in`].
    pub fn reclassify_wings_in(
        &self,
        only_wings: &[&str],
        apply: bool,
    ) -> Result<WingReclassifyReport, Error> {
        self.inner.reclassify_wings_in(only_wings, apply)
    }

    pub fn recall_local(&self, query: &str) -> Result<HybridRecallResult, Error> {
        self.inner
            .recall_local(query)
            .map(|r| Self::regroup_context_block(r, None))
    }

    /// Convenience: [`recall_at()`](Self::recall_at) with `Visibility::Private`.
    pub fn recall_local_at(
        &self,
        query: &str,
        now: DateTime<Utc>,
    ) -> Result<HybridRecallResult, Error> {
        self.inner
            .recall_local_at(query, now)
            .map(|r| Self::regroup_context_block(r, Some(now)))
    }

    /// Canonical integrated recall path. Unlike the legacy [`recall`](Self::recall)
    /// compatibility method, this applies the complete cascade and requires an
    /// explicit visibility boundary through [`RecallOptions`].
    pub fn recall_with(
        &self,
        query: &str,
        options: &RecallOptions,
    ) -> Result<spectral_cascade::result::CascadeResult, Error> {
        let mut config = spectral_graph::cascade_layers::CascadePipelineConfig::default();
        match options.profile {
            RecallProfile::Fast => {
                config.apply_ambient_boost = false;
                config.apply_declarative_boost = false;
                config.apply_episode_diversity = false;
                config.co_retrieval_weight = 0.0;
                config.spread = spectral_graph::spreading::AssocSpreadConfig::default();
            }
            RecallProfile::Balanced => {
                config.apply_ambient_boost = false;
            }
            RecallProfile::Adaptive => {}
        }
        self.inner
            .recall_cascade_scoped(query, &options.context, &config, options.visibility)
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

    // Note: the facade wrappers `recall_cross_wing`, `backfill_spectrograms`,
    // and `audit_spectrogram` were deleted outright: spectrogram-as-recall is
    // retired (0/500 contexts changed, docs/internal/ORACLE_TIER0.md) and
    // Permagent — the only consumer — never called them. Historical
    // experiments reach the deprecated graph-level equivalents through
    // `spectral::graph::brain::Brain` with the `spectrogram-legacy` feature.

    /// Hard-delete a memory by key across every substrate and verify it is
    /// gone (right-to-be-forgotten). Returns a [`ForgetReport`] with
    /// per-substrate deletion counts and post-delete recall/recognize probes.
    /// Unlike consolidation (soft hide), the content becomes unrecoverable.
    pub fn forget(&self, key: &str) -> Result<spectral_graph::brain::ForgetReport, Error> {
        self.inner.forget(key)
    }

    /// Complete physical erasure of already-`forget`-ed content: FTS
    /// `'optimize'` + truncating WAL checkpoint + `VACUUM` + a second
    /// checkpoint, across `memory.db`, `recognition.db`, and `graph.sqlite`.
    ///
    /// `forget` makes a memory logically unreachable immediately; SQLite
    /// still retains the deleted bytes in FTS5 segment b-trees, WAL frames,
    /// and free pages until this runs. See `docs/DELETION_GUARANTEES.md`.
    pub fn vacuum(&self) -> Result<(), Error> {
        self.inner.vacuum()
    }

    /// Recognition: "have I encountered this before — and what happened
    /// last time?" Deterministic, no LLM, sub-millisecond; the result
    /// carries the exact matched features behind the verdict
    /// ([`RecognitionResult`] with [`Verdict`], familiarity, novelty,
    /// traces, and evidence).
    ///
    /// Distinct from recall: recall retrieves what's relevant to a query;
    /// recognize judges whether a stimulus is a re-encounter and of what.
    pub fn recognize(&self, stimulus: &str) -> Result<RecognitionResult, Error> {
        self.inner.recognize(stimulus)
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

    /// Run the integrated retrieval pipeline with ambient boost.
    ///
    /// Unlike [`recall()`](Brain::recall), this path takes a
    /// [`RecognitionContext`](spectral_graph::RecognitionContext) and applies
    /// wing-match, recency, and ambient boost in the re-ranking pipeline.
    pub fn recall_cascade(
        &self,
        query: &str,
        context: &spectral_graph::RecognitionContext,
        config: &spectral_graph::cascade_layers::CascadePipelineConfig,
    ) -> Result<spectral_cascade::result::CascadeResult, Error> {
        self.inner.recall_cascade(query, context, config)
    }

    /// Integrated cascade recall with an explicit visibility boundary.
    pub fn recall_cascade_scoped(
        &self,
        query: &str,
        context: &spectral_graph::RecognitionContext,
        config: &spectral_graph::cascade_layers::CascadePipelineConfig,
        visibility: Visibility,
    ) -> Result<spectral_cascade::result::CascadeResult, Error> {
        self.inner
            .recall_cascade_scoped(query, context, config, visibility)
    }

    /// Rebuild the co-retrieval pairs index from accumulated retrieval events.
    ///
    /// Full recompute (not incremental). Atomic replace via single transaction —
    /// concurrent reads are safe. Idempotent. Returns the number of pairs written.
    pub fn rebuild_co_retrieval_index(&self) -> Result<usize, Error> {
        self.inner.rebuild_co_retrieval_index()
    }

    /// Rebuild the co-retrieval pairs index from retrieval events whose
    /// `method` starts with one of `method_prefixes`. An empty slice means
    /// every method — identical to [`Brain::rebuild_co_retrieval_index`].
    ///
    /// The event log is not homogeneous: `cascade` rows record the **full
    /// returned set** (exposure — every hit the caller was shown), while
    /// `turn:*` rows record only the subset the caller reported as **used**.
    /// Rebuilding over the union blends the two, and because exposure rows are
    /// far denser they dominate the counts — so an evaluation of
    /// outcome-credited co-retrieval run over everything cannot be
    /// interpreted. Pass [`spectral_ingest::TURN_EVENT_METHOD_PREFIX`] to
    /// build from turn events alone.
    pub fn rebuild_co_retrieval_index_for_methods(
        &self,
        method_prefixes: &[String],
    ) -> Result<usize, Error> {
        self.inner
            .rebuild_co_retrieval_index_for_methods(method_prefixes)
    }

    /// Bound constellation fingerprint fan-out per write. `None` (default) is
    /// unbounded. Setting a cap makes ingest cost flat in corpus size instead
    /// of growing with it; see the inner method for the measured trade-off.
    pub fn set_max_fingerprint_peers(&mut self, cap: Option<usize>) {
        self.inner.set_max_fingerprint_peers(cap);
    }

    /// Report bounded coverage of derived memory state without mutating it.
    pub fn derivation_health(&self, limit: usize) -> Result<DerivationHealthReport, Error> {
        self.inner.derivation_health(limit)
    }

    /// Idempotently repair derived state after interrupted or legacy ingests.
    pub fn repair_derivations(&self, limit: usize) -> Result<DerivationRepairReport, Error> {
        self.inner.repair_derivations(limit)
    }

    /// Direct access to the underlying graph store.
    pub fn store(&self) -> &spectral_graph::graph_store::GraphStore {
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

    /// Set the description on a graph entity. Idempotent: setting the same value
    /// twice is a no-op. Descriptions improve over time as understanding deepens.
    pub fn set_entity_description(
        &self,
        entity_id: &spectral_core::entity_id::EntityId,
        description: &str,
    ) -> Result<(), Error> {
        self.inner.set_entity_description(entity_id, description)
    }

    /// Write (insert-or-update) a typed field on an entity, with provenance.
    /// An `Enriched` write never overwrites a `Manual` field (enforced in the
    /// store). Returns `false` when suppressed, `true` when applied.
    pub fn set_entity_field(
        &self,
        entity_id: &spectral_core::entity_id::EntityId,
        field_name: &str,
        value: &str,
        source: spectral_ingest::FieldSource,
        source_url: Option<&str>,
    ) -> Result<bool, Error> {
        self.inner
            .set_entity_field(entity_id, field_name, value, source, source_url)
    }

    /// Read all typed fields for an entity (provenance included).
    pub fn get_entity_fields(
        &self,
        entity_id: &spectral_core::entity_id::EntityId,
    ) -> Result<Vec<spectral_ingest::EntityField>, Error> {
        self.inner.get_entity_fields(entity_id)
    }

    /// List memories where description IS NULL, ordered by created_at DESC.
    pub fn list_undescribed(&self, limit: usize) -> Result<Vec<spectral_ingest::Memory>, Error> {
        self.inner.list_undescribed(limit)
    }

    /// Mark source memories as consolidated into a target summary.
    /// Target must exist. Idempotent on same source→target pair.
    /// Flattens chains on write and merges signal scores (capped at 1.0).
    pub fn consolidate_into(
        &self,
        source_keys: &[String],
        target_key: &str,
        opts: &spectral_ingest::ConsolidateOpts,
    ) -> Result<spectral_ingest::ConsolidationResult, Error> {
        self.inner.consolidate_into(source_keys, target_key, opts)
    }

    /// List consolidation edges, optionally filtered to a specific target.
    pub fn list_consolidated(
        &self,
        target_key: Option<&str>,
    ) -> Result<Vec<spectral_ingest::ConsolidationEdge>, Error> {
        self.inner.list_consolidated(target_key)
    }

    /// List memory keys not consolidated as sources.
    pub fn list_unconsolidated(&self, limit: usize) -> Result<Vec<String>, Error> {
        self.inner.list_unconsolidated(limit)
    }

    /// Layered / provenance-linked recall: each hit paired with its
    /// ground-truth source memories (drill-down through `consolidation_edges`).
    /// See [`spectral_graph::brain::Brain::recall_with_provenance`].
    pub fn recall_with_provenance(
        &self,
        query: &str,
        config: &RecallTopKConfig,
        visibility: Visibility,
        max_sources_per_hit: usize,
    ) -> Result<Vec<spectral_graph::brain::LayeredHit>, Error> {
        self.inner
            .recall_with_provenance(query, config, visibility, max_sources_per_hit)
    }

    /// Recognition/co-retrieval-driven consolidation candidates (recurring
    /// clusters worth abstracting). See
    /// [`spectral_graph::brain::Brain::consolidation_candidates`].
    pub fn consolidation_candidates(
        &self,
        min_co_count: u64,
        scan_limit: usize,
    ) -> Result<Vec<spectral_graph::brain::ConsolidationCandidate>, Error> {
        self.inner
            .consolidation_candidates(min_co_count, scan_limit)
    }

    /// Consolidate sources into a higher-tier abstraction whose content comes
    /// from `summarize` (the one optional-LLM seam). See
    /// [`spectral_graph::brain::Brain::consolidate_with`].
    pub fn consolidate_with<F>(
        &self,
        source_keys: &[String],
        target_key: &str,
        tier: spectral_ingest::CompactionTier,
        summarize: F,
    ) -> Result<spectral_graph::brain::RememberResult, Error>
    where
        F: FnOnce(&[String]) -> String,
    {
        self.inner
            .consolidate_with(source_keys, target_key, tier, summarize)
    }

    /// Deterministic `$0` extractive consolidation (no LLM). See
    /// [`spectral_graph::brain::Brain::consolidate_extractive`].
    pub fn consolidate_extractive(
        &self,
        source_keys: &[String],
        target_key: &str,
        tier: spectral_ingest::CompactionTier,
    ) -> Result<spectral_graph::brain::RememberResult, Error> {
        self.inner
            .consolidate_extractive(source_keys, target_key, tier)
    }

    /// Store a pre-computed abstraction (e.g. a Librarian-generated atom) over
    /// the given sources. See
    /// [`spectral_graph::brain::Brain::consolidate_as`].
    pub fn consolidate_as(
        &self,
        source_keys: &[String],
        target_key: &str,
        tier: spectral_ingest::CompactionTier,
        content: &str,
    ) -> Result<spectral_graph::brain::RememberResult, Error> {
        self.inner
            .consolidate_as(source_keys, target_key, tier, content)
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

    /// Anticipatory recall: recommend memories associated with `memory_id`,
    /// ranked by **lift** (context-specific association) rather than raw
    /// co-retrieval count — a recommender over the user's own memories that
    /// surfaces what their current context is *specifically* associated with,
    /// suppressing globally-popular blobs. Deterministic, no LLM. See
    /// [`spectral_graph::brain::Brain::recommend`].
    pub fn recommend(
        &self,
        memory_id: &str,
        limit: usize,
        min_co_count: u64,
    ) -> Result<Vec<spectral_ingest::RelatedMemory>, Error> {
        self.inner.recommend(memory_id, limit, min_co_count)
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
    entity_policy: Option<EntityPolicy>,
    fts_tokenizer: Option<String>,
    read_only: bool,
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

    // Note: `enable_spectrogram` was deleted outright from the builder:
    // spectrogram-as-recall is retired (0/500 contexts changed, ORACLE_TIER0)
    // and Permagent — the only consumer — never called it. Historical
    // experiments use `spectral::graph::brain::BrainConfig` directly with the
    // `spectrogram-legacy` feature.

    /// Set the FTS5 tokenizer for the memories full-text index.
    ///
    /// Default (unset): `"porter unicode61"` — deterministic stemming that
    /// bridges plural/inflected queries to singular content. Pass
    /// `"unicode61"` to disable stemming. A brain built with a different
    /// tokenizer is migrated (one-time FTS index rebuild) on open.
    pub fn fts_tokenizer(mut self, tokenizer: impl Into<String>) -> Self {
        self.fts_tokenizer = Some(tokenizer.into());
        self
    }

    /// Open the brain strictly read-only: opening never mutates the brain
    /// (no directory/identity creation, no migrations, no FTS rebuild),
    /// recall paths skip their ambient writes, and write APIs return
    /// `Error::ReadOnly`. Required mode for federated read-time fan-out
    /// over a brain you don't own. Fails if the brain does not exist.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
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
            // Spectrogram-as-recall is retired (0/500, ORACLE_TIER0); the
            // facade never enables it.
            enable_spectrogram: false,
            entity_policy: self.entity_policy.unwrap_or_default(),
            sqlite_mmap_size: None,
            fts_tokenizer: self.fts_tokenizer,
            read_only: self.read_only,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
            ..Default::default()
        };

        let inner = spectral_graph::brain::Brain::open(config)?;
        Ok(Brain { inner })
    }
}
