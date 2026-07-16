//! High-level Brain API for Spectral.
//!
//! A [`Brain`] is the primary interface: asserting facts in the knowledge
//! graph, remembering free-text observations via TACT ingestion, and
//! recalling relevant context from both stores.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use spectral_core::device_id::DeviceId;
use spectral_core::entity_id::EntityId;
use spectral_core::identity::{BrainId, BrainIdentity};
use spectral_core::visibility::Visibility;
use spectral_ingest::sqlite_store::SqliteStore;
use spectral_ingest::{MemoryHit, MemoryStore};
use spectral_tact::{LlmClient, TactConfig, TactResult};

use spectral_spectrogram::{AnalysisContext, SpectrogramAnalyzer};

use crate::canonicalize::{Canonicalizer, MatchedMention};
use crate::extract::{ExtractedTriple, ExtractionPrompt};
use crate::graph_store::{Entity, GraphStore, Neighborhood, Triple};
use crate::ontology::Ontology;
use crate::Error;

/// Controls how `Brain::assert()` handles entities not found in the ontology.
#[derive(Default)]
pub enum EntityPolicy {
    /// Strict: assert() fails on unknown entities. Default.
    #[default]
    Strict,
    /// AutoCreate: assert() creates new entities using mention text as canonical name.
    /// Entity type is inferred from the predicate's domain/range.
    AutoCreate,
    /// AutoCreateWithCanonicalizer: assert() creates new entities, applying the
    /// provided function to derive canonical form from mention text.
    AutoCreateWithCanonicalizer(Arc<dyn Fn(&str) -> String + Send + Sync>),
}

impl std::fmt::Debug for EntityPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Strict => write!(f, "Strict"),
            Self::AutoCreate => write!(f, "AutoCreate"),
            Self::AutoCreateWithCanonicalizer(_) => write!(f, "AutoCreateWithCanonicalizer(...)"),
        }
    }
}

/// Configuration for opening a brain.
pub struct BrainConfig {
    /// Directory for brain data (identity files, graph database).
    pub data_dir: PathBuf,
    /// Path to the ontology TOML file.
    pub ontology_path: PathBuf,
    /// Path to the SQLite memory database (default: data_dir/memory.db).
    pub memory_db_path: Option<PathBuf>,
    /// Optional LLM client for TACT classification.
    pub llm_client: Option<Box<dyn LlmClient>>,
    /// Wing detection rules as `(regex_pattern, wing_name)` pairs.
    /// `None` uses the defaults from `spectral_ingest::default_wing_rule_strings()`.
    pub wing_rules: Option<Vec<(String, String)>>,
    /// Hall detection rules as `(regex_pattern, hall_name)` pairs.
    /// `None` uses the defaults from `spectral_ingest::default_hall_rule_strings()`.
    pub hall_rules: Option<Vec<(String, String)>>,
    /// Optional device identifier. `None` = derive from hostname.
    pub device_id: Option<DeviceId>,
    /// Enable cognitive spectrogram computation on ingest. Default false.
    pub enable_spectrogram: bool,
    /// Controls how assert() handles unknown entities. Default Strict.
    pub entity_policy: EntityPolicy,
    /// SQLite memory-map size override. See [`spectral_ingest::sqlite_store::SqliteStoreConfig::mmap_size`].
    ///
    /// - `None` (default): adaptive mmap (50 MB – 1 GB based on file size)
    /// - `Some(0)`: disable mmap
    /// - `Some(n)`: use exactly *n* bytes
    pub sqlite_mmap_size: Option<u64>,
    /// FTS5 tokenizer for the memories full-text index. See
    /// [`spectral_ingest::sqlite_store::SqliteStoreConfig::fts_tokenizer`].
    ///
    /// - `None` (default): `SPECTRAL_FTS_TOKENIZER` env var if set, else
    ///   `"porter unicode61"` — deterministic stemming that bridges
    ///   plural/inflected queries to singular content at zero runtime cost.
    /// - `Some("unicode61")`: explicit no-stemming tokenizer.
    ///
    /// A brain built with a different tokenizer is migrated (one-time FTS
    /// index rebuild) on open.
    pub fts_tokenizer: Option<String>,
    /// Open the brain strictly read-only. Default false.
    ///
    /// Read-only open **never mutates the brain**: no directory or identity
    /// creation, no schema DDL, no migrations, no FTS tokenizer rebuild, no
    /// fingerprint backfill. Recall paths skip their ambient writes
    /// (auto-reinforce, retrieval-event logging, recognition enroll), and
    /// write APIs return [`Error::ReadOnly`]. Underlying stores are opened
    /// with driver-level read-only flags as defense in depth.
    ///
    /// This is the required mode for federated read-time fan-out over a
    /// brain you don't own: without it, merely opening a peer's brain runs
    /// migrations against it and every recall bumps its signal scores and
    /// logs your query metadata into its store.
    pub read_only: bool,
    /// Wing name for activity episodes. Default "activity".
    pub activity_wing: String,
    /// Redaction policy applied to activity episodes before storage.
    /// Default: [`DefaultRedactionPolicy`](crate::activity::DefaultRedactionPolicy).
    pub redaction_policy: Option<Box<dyn crate::activity::RedactionPolicy>>,
    /// Override TACT pipeline configuration. `None` uses defaults.
    /// When `Some`, the provided config's `max_results`, `min_words`,
    /// and `max_context_chars` are used; `wing_rules` and `hall_rules`
    /// are still derived from `BrainConfig::wing_rules`/`hall_rules`
    /// so consumers don't have to duplicate them.
    pub tact_config: Option<TactConfig>,
}

/// Outcome of [`Brain::forget`]: the per-substrate SQLite deletion receipt,
/// whether the recognition sidecar dropped the memory, and a verification
/// probe confirming the content is no longer recall- or recognize-able.
/// "Verified forgetting" is the receipt plus the probe: deletion that is
/// checked across substrates rather than assumed from a single boolean.
#[derive(Debug, Clone)]
pub struct ForgetReport {
    /// Per-substrate SQLite deletion counts.
    pub store: spectral_ingest::ForgetReceipt,
    /// Whether the recognition sidecar had the memory enrolled and removed it.
    pub recognition_removed: bool,
    /// Post-delete probe: `recall_topk_fts` for the deleted content returned
    /// zero hits carrying the forgotten key. `true` = verified gone.
    pub recall_clear: bool,
    /// Post-delete probe: `recognize` on the deleted content did not return a
    /// `Recognized` verdict naming the forgotten memory. `true` = verified gone.
    pub recognize_clear: bool,
}

impl ForgetReport {
    /// The memory existed and every substrate + probe confirms it is gone.
    pub fn fully_forgotten(&self) -> bool {
        self.store.existed && self.recall_clear && self.recognize_clear
    }
}

/// Result of a successful assertion.
#[derive(Debug)]
pub struct AssertResult {
    pub triple_written: bool,
    pub subject: MatchedMention,
    pub predicate: String,
    pub object: MatchedMention,
}

/// Result of a graph-only recall query.
#[derive(Debug)]
pub struct RecallResult {
    pub seed_entities: Vec<EntityId>,
    pub triples: Vec<Triple>,
    pub neighborhood: Neighborhood,
}

/// Result of hybrid recall (memory + graph).
#[derive(Debug)]
pub struct HybridRecallResult {
    /// TACT memory hits.
    pub memory_hits: Vec<MemoryHit>,
    /// TACT retrieval result.
    pub tact: TactResult,
    /// Graph neighborhood result.
    pub graph: RecallResult,
}

/// Result of document ingestion.
#[derive(Debug)]
pub struct IngestResult {
    pub document_id: [u8; 32],
    pub matched: Vec<MatchedMention>,
    pub unresolved_count: usize,
}

/// Options for `Brain::remember_with()`.
#[derive(Debug, Default)]
pub struct RememberOpts {
    pub source: Option<String>,
    pub device_id: Option<DeviceId>,
    /// Classification confidence override. `None` = default 1.0.
    pub confidence: Option<f64>,
    pub visibility: Visibility,
    /// Override the memory's creation timestamp. `None` means use
    /// `Utc::now()` (database default). Use this when ingesting historical
    /// memories with known dates (e.g., migrating from external systems,
    /// importing dated conversation history).
    pub created_at: Option<DateTime<Utc>>,
    /// Assign the memory to this episode. `None` = auto-detect via
    /// time-gap heuristic.
    pub episode_id: Option<String>,
    /// Compaction tier for ambient stream memories. Set to `Some(Raw)` when
    /// ingesting raw activity events; the Librarian (or other consumer-side
    /// compaction process) updates this to `HourlyRollup`, `DailyRollup`, or
    /// `WeeklyRollup` as memories are aggregated over time. `None` means the
    /// memory is not part of the ambient stream (e.g., core or semantic facts
    /// written via direct user interaction). Spectral uses
    /// `compaction_tier.is_some()` as the canonical signal that a memory
    /// belongs to the ambient stream.
    pub compaction_tier: Option<spectral_ingest::CompactionTier>,
    /// Wing override. When `Some(value)`, the classifier is bypassed and the
    /// value is stored as-is. Callers pass the canonical slug form (e.g.,
    /// `"permagent"` not `"project:permagent"`). When `None`, wing is derived
    /// by the classifier from key+content+category.
    pub wing: Option<String>,
}

/// Result of remembering a memory.
#[derive(Debug)]
pub struct RememberResult {
    pub memory_id: String,
    pub wing: Option<String>,
    pub hall: Option<String>,
    pub signal_score: f64,
    pub fingerprints_created: usize,
    pub source: Option<String>,
    pub device_id: Option<DeviceId>,
    pub confidence: f64,
    pub write_outcome: spectral_ingest::WriteOutcome,
    /// Set when the written content re-encountered an existing memory
    /// (ambient recurrence feedback). The prior memory has been reinforced,
    /// and consumers (e.g. a Librarian) can use this to consolidate the
    /// near-duplicate. `None` when recurrence feedback is off or no prior
    /// match crossed the familiarity threshold.
    pub recurrence: Option<Recurrence>,
}

/// A detected content recurrence: the incoming memory re-encountered a prior
/// memory (recognition, not exact hash — this catches paraphrase/degraded
/// restatements). Recurrence is an ambient importance signal — the system
/// strengthens what keeps coming up, deterministically and without an LLM.
#[derive(Debug, Clone)]
pub struct Recurrence {
    /// The prior memory this content re-encountered (strongest match).
    pub matched_memory_id: String,
    /// Recognition familiarity of the re-encounter, in [0, 1].
    pub familiarity: f64,
}

/// A recalled memory paired with its ground-truth source memories — the
/// provenance chain from `consolidation_edges`. This is layered recall: the
/// actor receives the compact/abstract memory it matched, plus (on demand) the
/// exact source turns that memory was distilled from, so it can verify or
/// re-aggregate without re-deriving from scattered raw turns. Deterministic
/// "abstraction → ground truth" drill-down, no extra LLM cost.
#[derive(Debug, Clone)]
pub struct LayeredHit {
    /// The matched memory (may be an abstract/consolidated summary or a raw turn).
    pub hit: spectral_ingest::MemoryHit,
    /// Source memories consolidated into `hit`. Empty when `hit` is itself raw
    /// (no consolidation edges point at it).
    pub sources: Vec<spectral_ingest::MemoryHit>,
}

/// A cluster of related memories worth abstracting into a single higher-tier
/// memory. Selected by the deterministic ambient signals — recognition
/// recurrence (the spectrogram/MinHash engine flagging re-encounters) and/or
/// co-retrieval (what the user's usage pulls together) — so any downstream
/// summarizer (an extractive `$0` default or a sparse LLM) only ever runs on
/// high-value, already-recurring groups.
#[derive(Debug, Clone)]
pub struct ConsolidationCandidate {
    /// Member memory keys in the cluster (always ≥ 2).
    pub member_keys: Vec<String>,
    /// Cluster cohesion in [0, 1] — normalized strength of the ambient signal
    /// that grouped these (co-retrieval affinity or recognition familiarity).
    pub cohesion: f64,
    /// Which ambient signal produced this candidate.
    pub signal: &'static str,
}

/// Options for `Brain::ingest_text()`.
#[derive(Debug)]
pub struct IngestTextOpts {
    pub source: Option<String>,
    pub device_id: Option<DeviceId>,
    pub visibility: Visibility,
    /// Memory key for the original text. `None` = auto-generate from blake3 of content.
    pub memory_key: Option<String>,
    /// Confidence threshold below which extracted triples are rejected. Default 0.5.
    pub min_confidence: f64,
}

impl Default for IngestTextOpts {
    fn default() -> Self {
        Self {
            source: None,
            device_id: None,
            visibility: Visibility::Private,
            memory_key: None,
            min_confidence: 0.5,
        }
    }
}

/// Result of `Brain::ingest_text()`.
#[derive(Debug)]
pub struct IngestTextResult {
    pub memory: RememberResult,
    pub triples_extracted: usize,
    pub triples_asserted: usize,
    pub triples_rejected: Vec<RejectedTriple>,
}

/// A triple that was extracted but rejected during validation.
#[derive(Debug)]
pub struct RejectedTriple {
    pub raw: ExtractedTriple,
    pub reason: RejectionReason,
}

/// Why an extracted triple was rejected.
#[derive(Debug)]
pub enum RejectionReason {
    BelowConfidenceThreshold,
    UnresolvedSubject,
    UnresolvedObject,
    InvalidPredicate(String),
}

/// Result of cross-wing recall.
#[derive(Debug)]
pub struct CrossWingRecallResult {
    /// Best match for the seed query in its own wing.
    pub seed_memory: Option<MemoryHit>,
    /// Memories from other wings that resonate with the seed.
    pub resonant_memories: Vec<ResonantMemoryHit>,
}

/// A memory from another wing that resonates with the seed.
#[derive(Debug)]
pub struct ResonantMemoryHit {
    pub memory: MemoryHit,
    pub resonance_score: f64,
    pub matched_dimensions: Vec<String>,
}

/// Options for `Brain::reinforce()`.
#[derive(Debug)]
pub struct ReinforceOpts {
    /// Memory keys to reinforce (matched against recall result memory_hits).
    pub memory_keys: Vec<String>,
    /// Reinforcement strength, 0.0 to 1.0. Default 0.1.
    /// Each call adds this to signal_score (clamped to 1.0).
    pub strength: f64,
}

impl Default for ReinforceOpts {
    fn default() -> Self {
        Self {
            memory_keys: Vec::new(),
            strength: 0.1,
        }
    }
}

/// Result of `Brain::reinforce()`.
#[derive(Debug)]
pub struct ReinforceResult {
    pub memories_reinforced: usize,
    pub memories_not_found: Vec<String>,
}

/// Options for [`Brain::aaak()`] — Always-Active Agent Knowledge retrieval.
///
/// AAAK is the agent's foundational identity: a token-budgeted, ranked set
/// of the most important facts, suitable for injection into every system prompt.
/// Corresponds to the L1 "Curated Memory" layer in the TACT whitepaper (~170 tokens).
#[derive(Debug, Clone)]
pub struct AaakOpts {
    /// Maximum tokens for the returned string. Default 170.
    pub max_tokens: usize,
    /// Minimum signal_score for inclusion. Default 0.7.
    pub min_signal_score: f64,
    /// Halls to include. Default: fact, preference, decision, rule.
    pub include_halls: Vec<String>,
    /// Wings to include (None = all wings). Default None.
    pub include_wings: Option<Vec<String>>,
    /// Token estimation: characters per token. Default 4.0.
    pub chars_per_token: f64,
}

impl Default for AaakOpts {
    fn default() -> Self {
        Self {
            max_tokens: 170,
            min_signal_score: 0.7,
            include_halls: vec![
                "fact".into(),
                "preference".into(),
                "decision".into(),
                "rule".into(),
            ],
            include_wings: None,
            chars_per_token: 4.0,
        }
    }
}

/// Result of [`Brain::aaak()`].
#[derive(Debug, Clone)]
pub struct AaakResult {
    /// Formatted string ready for system prompt injection.
    pub formatted: String,
    /// Estimated token count.
    pub estimated_tokens: usize,
    /// Number of facts included.
    pub fact_count: usize,
    /// Number of facts excluded due to budget.
    pub excluded_count: usize,
    /// Wings represented in the result.
    pub wings_represented: Vec<String>,
}

/// Configuration for [`Brain::recall_topk_fts`].
#[derive(Debug, Clone)]
pub struct RecallTopKConfig {
    /// Number of results to return. Default 40.
    pub k: usize,
    /// Candidate-pool multiplier: fetch `k × fetch_mult` FTS candidates as
    /// the re-ranking pool, then truncate to `k` after re-ranking. Default 3.
    ///
    /// Broad FTS matching (porter stemming) can bury true evidence below a
    /// fixed bm25 LIMIT; a wider pool lets the deterministic re-ranker
    /// (signal, recency, entity) recover it. Validated at 3 on the temporal
    /// slice (+0.8pp session recall, ~+4% context tokens; 5 over-widens) —
    /// see docs/internal/TIER1_PORTER_WIDEN.md. Set to 1 to disable widening.
    pub fetch_mult: usize,
    /// Blend signal_score into FTS ranking. Default true.
    pub apply_signal_score_weighting: bool,
    /// Apply exponential recency decay. Default true.
    pub apply_recency_weighting: bool,
    /// Half-life for recency decay in days. Default 365.0.
    pub recency_half_life_days: f64,
    /// Boost top candidate within entity/wing clusters. Default true.
    pub apply_entity_resolution: bool,
    /// Additive boost for first-person declarative content (answer-bearing
    /// user-fact turns). Default false — kept off historically because the
    /// signal blends into cascade; enabled selectively for the topk_fts path
    /// where broad FTS matching (e.g. porter stemming) surfaces generic
    /// distractor turns that this signal down-weights.
    pub apply_declarative_boost: bool,
    /// Collapse `[Memory context]` reference duplicates. Default true.
    pub apply_context_dedup: bool,
    /// Time anchor for recency decay.
    ///
    /// **`None` silently falls back to `Utc::now()` at re-ranking time.**
    /// This is correct for live queries but silently wrong for historical
    /// replay — recency decay will measure distance from wall-clock rather
    /// than from the query's temporal context.
    ///
    /// Callers scoring historical data (bench, import replay, time-travel
    /// queries) **must** set this to the question/query date.
    pub now: Option<chrono::DateTime<chrono::Utc>>,
}

impl Default for RecallTopKConfig {
    fn default() -> Self {
        Self {
            k: 40,
            fetch_mult: 3,
            apply_signal_score_weighting: true,
            apply_recency_weighting: true,
            recency_half_life_days: 365.0,
            apply_entity_resolution: true,
            apply_declarative_boost: false,
            apply_context_dedup: true,
            now: None,
        }
    }
}

/// Result of [`Brain::audit_spectrogram()`].
#[derive(Debug, Clone)]
pub struct AuditReport {
    pub memory_id: String,
    pub memory_key: String,
    pub wing: Option<String>,
    pub content_excerpt: String,
    pub fingerprint: spectral_spectrogram::SpectralFingerprint,
    pub introspection: spectral_spectrogram::AnalysisIntrospection,
    pub signal_score: f64,
    pub created_at: Option<DateTime<Utc>>,
}

/// A Spectral brain: identity + ontology + knowledge graph + memory store.
///
/// # Open a brain
///
/// ```no_run
/// use spectral_graph::brain::{Brain, BrainConfig};
/// use std::path::PathBuf;
///
/// let brain = Brain::open(BrainConfig {
///     data_dir: PathBuf::from("/tmp/my-brain"),
///     ontology_path: PathBuf::from("ontology.toml"),
///     memory_db_path: None,
///     llm_client: None,
///     wing_rules: None,
///     hall_rules: None,
///     device_id: None,
///     enable_spectrogram: false,
///     entity_policy: spectral_graph::brain::EntityPolicy::Strict,
///     sqlite_mmap_size: None,
///     fts_tokenizer: None,
///     read_only: false,
///     activity_wing: "activity".into(),
///     redaction_policy: None,
///     tact_config: None,
/// }).unwrap();
/// println!("Brain ID: {}", brain.brain_id());
/// ```
pub struct Brain {
    identity: BrainIdentity,
    device_id: DeviceId,
    ontology: Ontology,
    /// Entities created at runtime via AutoCreate policy. Checked alongside the ontology.
    runtime_entities: Mutex<Vec<crate::ontology::OntologyEntity>>,
    ontology_path: PathBuf,
    store: GraphStore,
    memory_store: Box<dyn MemoryStore>,
    llm_client: Option<Box<dyn LlmClient>>,
    entity_policy: EntityPolicy,
    enable_spectrogram: bool,
    spectrogram_analyzer: SpectrogramAnalyzer,
    tact_config: TactConfig,
    ingest_config: spectral_ingest::ingest::IngestConfig,
    activity_wing: String,
    redaction_policy: Box<dyn crate::activity::RedactionPolicy>,
    /// Recognition engine (sidecar `recognition.db`). Memories are enrolled
    /// at write time; `recognize()` answers "have I encountered this?".
    recognition: Mutex<
        spectral_recognition::RecognitionEngine<spectral_recognition::SqliteRecognitionStore>,
    >,
    /// Opened with `read_only = true`: write APIs return [`Error::ReadOnly`]
    /// and recall paths skip their ambient writes (auto-reinforce,
    /// retrieval-event logging).
    read_only: bool,
    /// Ambient recurrence feedback: when new content re-encounters an existing
    /// memory (recognition), reinforce that prior memory. Enabled via
    /// `SPECTRAL_RECURRENCE_FEEDBACK=1`. Off by default (measure before
    /// defaulting — the co-retrieval lesson).
    recurrence_feedback: bool,
    rt: tokio::runtime::Runtime,
}

/// Minimum recognition familiarity for an incoming write to count as a
/// recurrence of a prior memory (recurrence feedback). Calibrated for
/// high-confidence re-encounters — restatements that reuse the salient
/// specifics (the recognition strong regime), not loose paraphrases (its weak
/// regime, which needs embeddings) — to avoid false reinforcement.
const RECURRENCE_MIN_FAMILIARITY: f64 = 0.4;
/// Reinforcement strength applied to a recurring prior memory. Bounded and
/// small: recurrence nudges importance, it does not saturate it.
const RECURRENCE_STRENGTH: f64 = 0.05;

impl std::fmt::Debug for Brain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Brain")
            .field("brain_id", self.identity.brain_id())
            .finish_non_exhaustive()
    }
}

impl Brain {
    /// Open or create a brain.
    ///
    /// Creates `data_dir` if missing, generates identity on first run,
    /// opens both the graph database and memory store.
    ///
    /// # Known issue: schema-creation abort on Linux
    ///
    /// `Brain::open()` may abort with SIGABRT on Linux during schema
    /// creation. The abort fires inside `create_schema` when kuzu
    /// throws a C++ exception that the kuzu cxxbridge FFI layer fails
    /// to convert into a Rust `Result::Err` — `std::terminate` fires
    /// instead. macOS is unaffected.
    ///
    /// This is a kuzu FFI correctness issue (cxxbridge `trycatch`
    /// should never call `std::terminate`). The platform difference
    /// is not yet fully diagnosed; the underlying query appears to
    /// behave differently on Linux.
    ///
    /// Earlier diagnoses attributed this to glibc 2.39 pthread
    /// teardown behavior or to instance-count thresholds. Both were
    /// incorrect; a single `Brain::open()` is sufficient to trigger
    /// the abort.
    ///
    /// Reproducers (both `#[ignore]`d, in the `kuzu_schema_abort_repro`
    /// module below): `single_brain_open_aborts_on_linux` and
    /// `n_brains_coresident_recall_varied_drop_order`. The latter also
    /// settles whether federation read-time fan-out can co-locate N
    /// brains in one process. Run them on Linux via
    /// `.github/workflows/kuzu-abort-diagnostic.yml`.
    ///
    /// Tracked at: <https://github.com/make-tuned-unit/spectral/issues/153>.
    /// Upstream kuzu issue: not yet filed — it needs the Linux abort
    /// backtrace produced by the diagnostic workflow (pending the
    /// Ubuntu run / Permagent-collaborator hand-off).
    pub fn open(config: BrainConfig) -> Result<Self, Error> {
        if config.read_only {
            if !config.data_dir.is_dir() {
                return Err(Error::Schema(format!(
                    "read-only open requires an existing brain: {} not found",
                    config.data_dir.display()
                )));
            }
        } else {
            std::fs::create_dir_all(&config.data_dir)?;
        }

        // `load_or_create` only writes when identity files are absent — a
        // state a read-only open must reject rather than repair.
        if config.read_only && !config.data_dir.join("brain.id").exists() {
            return Err(Error::Schema(format!(
                "read-only open requires an existing brain identity in {}",
                config.data_dir.display()
            )));
        }
        let identity = BrainIdentity::load_or_create(&config.data_dir).map_err(Error::Core)?;
        let ontology_path = config.ontology_path.clone();
        let ontology = Ontology::load(&config.ontology_path)?;
        let graph_path = config.data_dir.join("graph.sqlite");
        let store = if config.read_only {
            GraphStore::open_read_only(&graph_path)?
        } else {
            GraphStore::open(&graph_path)?
        };

        let memory_db_path = config
            .memory_db_path
            .unwrap_or_else(|| config.data_dir.join("memory.db"));
        let sqlite_config = spectral_ingest::sqlite_store::SqliteStoreConfig {
            mmap_size: config.sqlite_mmap_size,
            fts_tokenizer: config.fts_tokenizer.clone(),
            read_only: config.read_only,
            // Opt-in via SPECTRAL_FTS_FUSION (resolved inside the store); no
            // BrainConfig field yet — keeps this an env-gated experimental lever
            // like the stopword/anticipatory levers.
            fts_fusion: false,
        };
        let memory_store: Box<dyn MemoryStore> = Box::new(
            SqliteStore::open_with_config(&memory_db_path, &sqlite_config)
                .map_err(|e| Error::Schema(e.to_string()))?,
        );
        // Resolve wing/hall rules — shared between ingest and TACT retrieval.
        let wing_rules = config
            .wing_rules
            .unwrap_or_else(spectral_ingest::default_wing_rule_strings);
        let hall_rules = config
            .hall_rules
            .unwrap_or_else(spectral_ingest::default_hall_rule_strings);

        let tact_config = match config.tact_config {
            Some(custom) => TactConfig {
                wing_rules: wing_rules.clone(),
                hall_rules: hall_rules.clone(),
                ..custom
            },
            None => TactConfig {
                wing_rules: wing_rules.clone(),
                hall_rules: hall_rules.clone(),
                ..TactConfig::default()
            },
        };

        let ingest_config = spectral_ingest::ingest::IngestConfig {
            wing_rules: wing_rules
                .iter()
                .map(|(p, w)| (regex::Regex::new(p).expect("invalid wing regex"), w.clone()))
                .collect(),
            hall_rules: hall_rules
                .iter()
                .map(|(p, h)| (regex::Regex::new(p).expect("invalid hall regex"), h.clone()))
                .collect(),
            ..spectral_ingest::ingest::IngestConfig::default()
        };

        let rt = tokio::runtime::Runtime::new().map_err(|e| Error::Schema(e.to_string()))?;

        let device_id = config.device_id.unwrap_or_else(|| {
            let hostname = hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown-device".to_string());
            DeviceId::from_descriptor(&hostname)
        });

        // One-time backfill: fix legacy fingerprints with "unknown" time_delta_bucket (PR #65).
        // Idempotent — returns 0 on subsequent opens once all buckets are valid.
        // Skipped read-only: opening must not mutate the brain.
        if !config.read_only {
            match rt.block_on(memory_store.backfill_fingerprint_time_buckets()) {
                Ok(0) => {}
                Ok(n) => tracing::info!(count = n, "backfilled legacy fingerprint time buckets"),
                Err(e) => tracing::warn!("fingerprint backfill failed (non-fatal): {e}"),
            }
        }

        // Recognition engine: sidecar store next to the graph and memory DBs.
        // Read-only mode opens an existing sidecar without DDL; a brain that
        // predates the sidecar gets an empty in-memory index (recognize()
        // returns Novel) rather than a file created in someone else's brain.
        let recognition_db = config.data_dir.join("recognition.db");
        let recognition_store = if config.read_only {
            if recognition_db.exists() {
                spectral_recognition::SqliteRecognitionStore::open_read_only(&recognition_db)
            } else {
                spectral_recognition::SqliteRecognitionStore::open(std::path::Path::new(
                    ":memory:",
                ))
            }
        } else {
            spectral_recognition::SqliteRecognitionStore::open(&recognition_db)
        }
        .map_err(|e| Error::Schema(format!("recognition store: {e}")))?;
        let recognition = Mutex::new(spectral_recognition::RecognitionEngine::new(
            recognition_store,
            spectral_recognition::RecognitionConfig::default(),
        ));

        Ok(Self {
            identity,
            device_id,
            ontology,
            runtime_entities: Mutex::new(Vec::new()),
            ontology_path,
            store,
            memory_store,
            llm_client: config.llm_client,
            entity_policy: config.entity_policy,
            enable_spectrogram: config.enable_spectrogram,
            spectrogram_analyzer: SpectrogramAnalyzer::default(),
            tact_config,
            ingest_config,
            activity_wing: config.activity_wing,
            redaction_policy: config
                .redaction_policy
                .unwrap_or_else(|| Box::new(crate::activity::DefaultRedactionPolicy::default())),
            recognition,
            read_only: config.read_only,
            recurrence_feedback: std::env::var("SPECTRAL_RECURRENCE_FEEDBACK")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            rt,
        })
    }

    /// Whether this brain was opened read-only.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Guard for write APIs: returns [`Error::ReadOnly`] naming the blocked
    /// operation when the brain was opened with `read_only = true`.
    fn ensure_writable(&self, op: &'static str) -> Result<(), Error> {
        if self.read_only {
            Err(Error::ReadOnly(op))
        } else {
            Ok(())
        }
    }

    /// Recognition: "have I encountered this before — and what happened
    /// last time?" Deterministic, no LLM, sub-millisecond; the result
    /// carries the exact matched features behind the verdict.
    ///
    /// Distinct from recall: recall retrieves what's relevant to a query;
    /// recognize judges whether a stimulus is a re-encounter and of what.
    pub fn recognize(
        &self,
        stimulus: &str,
    ) -> Result<spectral_recognition::RecognitionResult, Error> {
        let engine = self
            .recognition
            .lock()
            .map_err(|e| Error::Schema(format!("recognition lock poisoned: {e}")))?;
        engine
            .recognize(stimulus)
            .map_err(|e| Error::Schema(format!("recognize: {e}")))
    }

    /// Returns this brain's stable identifier.
    pub fn brain_id(&self) -> &BrainId {
        self.identity.brain_id()
    }

    /// Returns this brain's public verifying key (for signature verification
    /// by federated peers).
    pub fn verifying_key(&self) -> &spectral_core::identity::VerifyingKey {
        self.identity.verifying_key()
    }

    /// Verify a memory hit's signed provenance against a contributor's public
    /// key. Returns `true` only if the hit carries a signature and
    /// source-brain id, the key matches that id, and the signature is valid
    /// over the hit's content hash, creation time, and visibility.
    ///
    /// `pubkey` must be resolved from the hit's `source_brain_id` out of band
    /// (via the contributor grant set) — a `BrainId` cannot be inverted to
    /// its key. Returns `false` for unsigned/legacy hits.
    pub fn verify_hit(
        hit: &spectral_ingest::MemoryHit,
        pubkey: &spectral_core::identity::VerifyingKey,
    ) -> bool {
        let (Some(sbid_bytes), Some(sig_bytes), Some(created_at)) =
            (hit.source_brain_id, hit.signature.as_deref(), hit.created_at.as_deref())
        else {
            return false;
        };
        let Ok(sig) = spectral_core::identity::Signature::from_slice(sig_bytes) else {
            return false;
        };
        let source_id = BrainId::from_bytes(sbid_bytes);
        let content_hash = blake3::hash(hit.content.as_bytes()).to_hex().to_string();
        spectral_core::identity::verify_memory_signature(
            &source_id,
            pubkey,
            &content_hash,
            created_at,
            &hit.visibility,
            &sig,
        )
    }

    /// Returns the device ID associated with this brain instance.
    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    /// Assert a fact: subject text, predicate name, object text.
    ///
    /// Both subject and object are resolved through the ontology. Under
    /// `EntityPolicy::Strict` (default), unknown entities cause an error.
    /// Under `AutoCreate` or `AutoCreateWithCanonicalizer`, unknown entities
    /// are created with types inferred from the predicate's domain/range.
    ///
    /// Returns `Error::AmbiguousEntityType` if the predicate has multiple
    /// valid domain or range types and an entity needs to be created.
    /// Use `assert_typed()` in that case.
    pub fn assert(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: f64,
        visibility: Visibility,
    ) -> Result<AssertResult, Error> {
        self.ensure_writable("assert")?;
        let pred_def = self
            .ontology
            .predicates
            .iter()
            .find(|p| p.name == predicate);

        // Validate predicate exists in the ontology
        let pd =
            pred_def.ok_or_else(|| Error::Ontology(format!("unknown predicate: '{predicate}'")))?;

        let subject_match =
            self.resolve_or_create(subject, Some(pd.domain.as_slice()), predicate, visibility)?;
        let object_match =
            self.resolve_or_create(object, Some(pd.range.as_slice()), predicate, visibility)?;

        if !pd.domain.iter().any(|d| d == &subject_match.entity_type) {
            return Err(Error::InvalidPredicate {
                predicate: predicate.to_string(),
                subject_type: subject_match.entity_type.clone(),
                object_type: object_match.entity_type.clone(),
            });
        }
        if !pd.range.iter().any(|r| r == &object_match.entity_type) {
            return Err(Error::InvalidPredicate {
                predicate: predicate.to_string(),
                subject_type: subject_match.entity_type.clone(),
                object_type: object_match.entity_type.clone(),
            });
        }

        self.write_triple(
            &subject_match,
            predicate,
            &object_match,
            confidence,
            visibility,
        )
    }

    /// Assert a triple with explicit types for subject and object.
    ///
    /// Use when the predicate has multiple valid domain/range types, or
    /// when overriding predicate-derived inference.
    pub fn assert_typed(
        &self,
        subject: (&str, &str), // (entity_type, mention)
        predicate: &str,
        object: (&str, &str),
        confidence: f64,
        visibility: Visibility,
    ) -> Result<AssertResult, Error> {
        self.ensure_writable("assert_typed")?;
        let (subject_type, subject_mention) = subject;
        let (object_type, object_mention) = object;

        let subject_match =
            self.resolve_or_create_typed(subject_mention, subject_type, visibility)?;
        let object_match = self.resolve_or_create_typed(object_mention, object_type, visibility)?;

        // Validate predicate if it exists in the ontology
        if let Some(pd) = self
            .ontology
            .predicates
            .iter()
            .find(|p| p.name == predicate)
        {
            if !pd.domain.iter().any(|d| d == subject_type) {
                return Err(Error::InvalidPredicate {
                    predicate: predicate.to_string(),
                    subject_type: subject_type.to_string(),
                    object_type: object_type.to_string(),
                });
            }
            if !pd.range.iter().any(|r| r == object_type) {
                return Err(Error::InvalidPredicate {
                    predicate: predicate.to_string(),
                    subject_type: subject_type.to_string(),
                    object_type: object_type.to_string(),
                });
            }
        }

        self.write_triple(
            &subject_match,
            predicate,
            &object_match,
            confidence,
            visibility,
        )
    }

    /// Resolve a mention to an entity, creating it if the policy allows.
    fn resolve_or_create(
        &self,
        mention: &str,
        allowed_types: Option<&[String]>,
        predicate: &str,
        visibility: Visibility,
    ) -> Result<MatchedMention, Error> {
        let canonicalizer = Canonicalizer::new(&self.ontology);

        // Try ontology match first
        if let Some(m) = canonicalizer.resolve_one(mention) {
            return Ok(m);
        }
        // Try runtime entities
        if let Some(m) = self.resolve_from_runtime(mention) {
            return Ok(m);
        }

        match &self.entity_policy {
            EntityPolicy::Strict => {
                let nearest = canonicalizer.find_nearest(mention).map(|n| n.canonical);
                Err(Error::UnresolvedMention {
                    mention: mention.to_string(),
                    nearest,
                })
            }
            EntityPolicy::AutoCreate => {
                let entity_type = infer_single_type(mention, allowed_types, predicate)?;
                self.auto_create_entity(mention, mention, &entity_type, visibility)
            }
            EntityPolicy::AutoCreateWithCanonicalizer(f) => {
                let canonical = f(mention);
                // Check if canonicalized form matches
                if let Some(m) = canonicalizer.resolve_one(&canonical) {
                    self.ensure_alias(&m.canonical, &m.entity_type, mention)?;
                    return Ok(m);
                }
                if let Some(m) = self.resolve_from_runtime(&canonical) {
                    self.ensure_alias(&m.canonical, &m.entity_type, mention)?;
                    return Ok(m);
                }
                let entity_type = infer_single_type(mention, allowed_types, predicate)?;
                self.auto_create_entity(&canonical, mention, &entity_type, visibility)
            }
        }
    }

    /// Resolve a mention with an explicit type, creating if policy allows.
    fn resolve_or_create_typed(
        &self,
        mention: &str,
        entity_type: &str,
        visibility: Visibility,
    ) -> Result<MatchedMention, Error> {
        let canonicalizer = Canonicalizer::new(&self.ontology);

        if let Some(m) = canonicalizer.resolve_one(mention) {
            return Ok(m);
        }
        if let Some(m) = self.resolve_from_runtime(mention) {
            return Ok(m);
        }

        match &self.entity_policy {
            EntityPolicy::Strict => {
                let nearest = canonicalizer.find_nearest(mention).map(|n| n.canonical);
                Err(Error::UnresolvedMention {
                    mention: mention.to_string(),
                    nearest,
                })
            }
            EntityPolicy::AutoCreate => {
                self.auto_create_entity(mention, mention, entity_type, visibility)
            }
            EntityPolicy::AutoCreateWithCanonicalizer(f) => {
                let canonical = f(mention);
                if let Some(m) = canonicalizer.resolve_one(&canonical) {
                    self.ensure_alias(&m.canonical, &m.entity_type, mention)?;
                    return Ok(m);
                }
                if let Some(m) = self.resolve_from_runtime(&canonical) {
                    self.ensure_alias(&m.canonical, &m.entity_type, mention)?;
                    return Ok(m);
                }
                self.auto_create_entity(&canonical, mention, entity_type, visibility)
            }
        }
    }

    /// Try to resolve a mention from runtime-created entities.
    fn resolve_from_runtime(&self, mention: &str) -> Option<MatchedMention> {
        let lower = mention.to_lowercase();
        let entities = self.runtime_entities.lock().ok()?;
        entities.iter().find_map(|e| {
            let matches = e.canonical.to_lowercase() == lower
                || e.aliases.iter().any(|a| a.to_lowercase() == lower);
            if matches {
                let eid = spectral_core::entity_id::entity_id(&e.entity_type, &e.canonical);
                Some(MatchedMention {
                    mention: mention.to_string(),
                    span: (0, mention.len()),
                    entity_id: eid,
                    entity_type: e.entity_type.clone(),
                    canonical: e.canonical.clone(),
                    match_kind: crate::canonicalize::MatchKind::Exact,
                })
            } else {
                None
            }
        })
    }

    /// Create a new entity and persist it.
    fn auto_create_entity(
        &self,
        canonical: &str,
        mention: &str,
        entity_type: &str,
        visibility: Visibility,
    ) -> Result<MatchedMention, Error> {
        let entity_id = spectral_core::entity_id::entity_id(entity_type, canonical);
        let aliases = if mention != canonical {
            vec![mention.to_string()]
        } else {
            vec![]
        };

        // Persist to ontology file
        self.append_entity_to_ontology(entity_type, canonical, &aliases, visibility)?;

        // Add to runtime entity list
        if let Ok(mut rt_entities) = self.runtime_entities.lock() {
            rt_entities.push(crate::ontology::OntologyEntity {
                entity_type: entity_type.to_string(),
                canonical: canonical.to_string(),
                aliases: aliases.clone(),
                visibility,
            });
        }

        Ok(MatchedMention {
            mention: mention.to_string(),
            span: (0, mention.len()),
            entity_id,
            entity_type: entity_type.to_string(),
            canonical: canonical.to_string(),
            match_kind: crate::canonicalize::MatchKind::Exact,
        })
    }

    /// Add an alias to an existing runtime entity.
    fn ensure_alias(&self, canonical: &str, entity_type: &str, alias: &str) -> Result<(), Error> {
        if let Ok(mut rt_entities) = self.runtime_entities.lock() {
            if let Some(entity) = rt_entities
                .iter_mut()
                .find(|e| e.canonical == canonical && e.entity_type == entity_type)
            {
                if !entity.aliases.iter().any(|a| a.eq_ignore_ascii_case(alias)) {
                    entity.aliases.push(alias.to_string());
                }
            }
        }
        Ok(())
    }

    /// Append a new entity to the ontology TOML file for persistence across restarts.
    fn append_entity_to_ontology(
        &self,
        entity_type: &str,
        canonical: &str,
        aliases: &[String],
        visibility: Visibility,
    ) -> Result<(), Error> {
        use std::fmt::Write;
        let mut block = String::new();
        writeln!(block).unwrap();
        writeln!(block, "[[entity]]").unwrap();
        writeln!(block, "type = \"{}\"", entity_type).unwrap();
        writeln!(block, "canonical = \"{}\"", canonical).unwrap();
        let alias_strs: Vec<String> = aliases.iter().map(|a| format!("\"{}\"", a)).collect();
        writeln!(block, "aliases = [{}]", alias_strs.join(", ")).unwrap();
        writeln!(block, "visibility = \"{}\"", visibility_to_str(visibility)).unwrap();

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.ontology_path)?;
        std::io::Write::write_all(&mut file, block.as_bytes())?;
        Ok(())
    }

    /// Write a triple to the graph store (shared by assert and assert_typed).
    fn write_triple(
        &self,
        subject_match: &MatchedMention,
        predicate: &str,
        object_match: &MatchedMention,
        confidence: f64,
        visibility: Visibility,
    ) -> Result<AssertResult, Error> {
        let now = Utc::now();

        self.store.upsert_entity(&Entity {
            id: subject_match.entity_id,
            entity_type: subject_match.entity_type.clone(),
            canonical: subject_match.canonical.clone(),
            visibility,
            created_at: now,
            updated_at: now,
            weight: 1.0,
            description: None,
        })?;

        self.store.upsert_entity(&Entity {
            id: object_match.entity_id,
            entity_type: object_match.entity_type.clone(),
            canonical: object_match.canonical.clone(),
            visibility,
            created_at: now,
            updated_at: now,
            weight: 1.0,
            description: None,
        })?;

        self.store.insert_triple(&Triple {
            from: subject_match.entity_id,
            to: object_match.entity_id,
            predicate: predicate.to_string(),
            confidence,
            source_doc_id: None,
            source_brain_id: *self.identity.brain_id(),
            asserted_at: now,
            visibility,
            weight: 1.0,
        })?;

        Ok(AssertResult {
            triple_written: true,
            subject: subject_match.clone(),
            predicate: predicate.to_string(),
            object: object_match.clone(),
        })
    }

    /// Ingest free text: classify, score, fingerprint, store in memory DB.
    ///
    /// The `visibility` parameter controls who can see this memory during recall.
    /// Equivalent to `remember_with(key, content, RememberOpts { visibility, ..Default::default() })`.
    pub fn remember(
        &self,
        key: &str,
        content: &str,
        visibility: Visibility,
    ) -> Result<RememberResult, Error> {
        self.remember_with(
            key,
            content,
            RememberOpts {
                visibility,
                ..Default::default()
            },
        )
    }

    /// Ingest free text with full metadata control.
    pub fn remember_with(
        &self,
        key: &str,
        content: &str,
        opts: RememberOpts,
    ) -> Result<RememberResult, Error> {
        self.ensure_writable("remember_with")?;
        let memory_id = format!(
            "{:016x}",
            u64::from_be_bytes(
                blake3::hash(key.as_bytes()).as_bytes()[..8]
                    .try_into()
                    .unwrap()
            )
        );

        let vis_str = visibility_to_str(opts.visibility);
        let ingest_opts = spectral_ingest::ingest::IngestOpts {
            source: opts.source,
            device_id: opts.device_id,
            confidence: opts.confidence,
            created_at: opts.created_at,
            episode_id: opts.episode_id,
            compaction_tier: opts.compaction_tier,
            wing: opts.wing,
        };
        let result = self
            .rt
            .block_on(spectral_ingest::ingest::ingest_with(
                &memory_id,
                key,
                content,
                "core",
                Utc::now().timestamp() as f64,
                &vis_str,
                &self.ingest_config,
                self.memory_store.as_ref(),
                ingest_opts,
            ))
            .map_err(|e| Error::Schema(e.to_string()))?;

        // Compute and store declarative density
        let density = crate::ranking::declarative_density(content);
        let _ = self.rt.block_on(
            self.memory_store
                .set_declarative_density(&result.memory.id, density),
        );

        // Sign the contribution (best-effort — a signing failure degrades
        // provenance but must not block the write). Signs over the STORED
        // content hash, creation time, and visibility, so verification later
        // recomputes the exact payload. Read the row back to get the values
        // the store actually persisted (created_at default, cleaned content).
        if let Ok(Some(stored)) = self
            .rt
            .block_on(self.memory_store.fetch_by_ids(std::slice::from_ref(&result.memory.id)))
            .map(|v| v.into_iter().next())
        {
            if let (Some(content_hash), Some(created_at)) =
                (stored.content_hash.as_deref(), stored.created_at.as_deref())
            {
                let sig = self
                    .identity
                    .sign_memory(content_hash, created_at, &stored.visibility);
                let sbid = *self.identity.brain_id().as_bytes();
                let _ = self.rt.block_on(self.memory_store.set_signature(
                    &result.memory.id,
                    &sbid,
                    &sig.to_bytes(),
                ));
            }
        }

        // Ambient recurrence feedback: BEFORE enrolling the new memory, check
        // whether its content re-encounters an EXISTING memory (recognition
        // sees only priors — the new one isn't enrolled yet). If so, strengthen
        // that prior (recurrence = importance) and surface the match so a
        // consumer can consolidate the near-duplicate. Content-driven and
        // deterministic — the opposite of the co-retrieval popularity signal.
        // Only on genuinely new writes; a self-match is impossible pre-enroll.
        let mut recurrence = None;
        if self.recurrence_feedback
            && matches!(result.write_outcome, spectral_ingest::WriteOutcome::Inserted)
        {
            let rec = self
                .recognition
                .lock()
                .ok()
                .and_then(|engine| engine.recognize(content).ok());
            if let Some(rec) = rec {
                if rec.familiarity >= RECURRENCE_MIN_FAMILIARITY {
                    if let Some(top) = rec.traces.first() {
                        if top.memory_id != result.memory.id {
                            if let Ok(Some(prior)) = self.get_memory(&top.memory_id) {
                                let _ = self.reinforce_by_id(&prior.key, RECURRENCE_STRENGTH);
                            }
                            recurrence = Some(Recurrence {
                                matched_memory_id: top.memory_id.clone(),
                                familiarity: rec.familiarity,
                            });
                        }
                    }
                }
            }
        }

        // Enroll in the recognition index (idempotent per memory id;
        // failures are non-fatal — recognition degrades, writes don't).
        if let Ok(mut engine) = self.recognition.lock() {
            if let Err(e) = engine.enroll(&result.memory.id, content) {
                tracing::warn!("recognition enroll failed (non-fatal): {e}");
            }
        }

        // Compute and store spectrogram if enabled
        if self.enable_spectrogram {
            let context =
                self.spectrogram_context(result.memory.wing.as_deref(), &result.memory.id);
            let fp = self.spectrogram_analyzer.analyze(&result.memory, &context);
            let peak_json = serde_json::to_string(&fp.peak_dimensions).unwrap_or_default();
            let _ = self.rt.block_on(self.memory_store.write_spectrogram(
                &result.memory.id,
                fp.entity_density,
                fp.action_type.as_str(),
                fp.decision_polarity,
                fp.causal_depth,
                fp.emotional_valence,
                fp.temporal_specificity,
                fp.novelty,
                &peak_json,
            ));
        }

        Ok(RememberResult {
            memory_id: result.memory.id,
            wing: result.memory.wing,
            hall: result.memory.hall,
            signal_score: result.memory.signal_score,
            fingerprints_created: result.fingerprints.len(),
            source: result.memory.source,
            device_id: result.memory.device_id.map(DeviceId::from_bytes),
            confidence: result.memory.confidence,
            write_outcome: result.write_outcome,
            recurrence,
        })
    }

    /// Hybrid recall filtered by visibility context.
    ///
    /// Returns only content where `content.visibility.allows(context_visibility)`
    /// is true. A `Private` context sees everything; a `Public` context sees
    /// only `Public` content.
    ///
    /// Recall results are scored using time-decayed signal scores. Memories that
    /// have not been reinforced recently receive a gentle penalty (1% per week,
    /// capped at 50% of the original score). Reinforce useful results via
    /// `Brain::reinforce()` to lift them back up.
    ///
    /// **Time anchor defaults to `Utc::now()`**, which is correct for live
    /// queries but wrong for historical replay. Use [`recall_at()`](Self::recall_at)
    /// to anchor decay to a specific point in time.
    pub fn recall(
        &self,
        query: &str,
        context_visibility: Visibility,
    ) -> Result<HybridRecallResult, Error> {
        self.recall_at(query, context_visibility, Utc::now())
    }

    /// Hybrid recall with an explicit time anchor for recency decay.
    ///
    /// Identical to [`recall()`](Self::recall) but uses `now` instead of
    /// `Utc::now()` for signal-score decay, so historical/replay queries
    /// measure recency from the query date rather than wall-clock.
    pub fn recall_at(
        &self,
        query: &str,
        context_visibility: Visibility,
        now: DateTime<Utc>,
    ) -> Result<HybridRecallResult, Error> {
        let tact = self
            .rt
            .block_on(spectral_tact::retrieve(
                query,
                &self.tact_config,
                self.memory_store.as_ref(),
            ))
            .map_err(|e| Error::Schema(e.to_string()))?;

        // Filter by visibility, then apply time-based decay to signal scores.
        // Consolidated sources are already excluded at the SQL layer
        // (NOT IN consolidation_edges on fingerprint_search and fts_search).
        let memory_hits: Vec<_> = tact
            .memories
            .iter()
            .filter(|m| str_to_vis(&m.visibility).allows(context_visibility))
            .cloned()
            .map(|mut hit| {
                hit.signal_score = decayed_signal_score(
                    hit.signal_score,
                    &hit.created_at,
                    &hit.last_reinforced_at,
                    &now,
                );
                hit
            })
            .collect();

        let graph = self.recall_graph(query, context_visibility)?;

        Ok(HybridRecallResult {
            memory_hits,
            tact,
            graph,
        })
    }

    /// Convenience: recall with maximally-permissive context (returns everything).
    ///
    /// **Time anchor defaults to `Utc::now()`** — see [`recall()`](Self::recall).
    /// Use [`recall_local_at()`](Self::recall_local_at) for historical queries.
    pub fn recall_local(&self, query: &str) -> Result<HybridRecallResult, Error> {
        self.recall(query, Visibility::Private)
    }

    /// Convenience: [`recall_at()`](Self::recall_at) with `Visibility::Private`.
    pub fn recall_local_at(
        &self,
        query: &str,
        now: DateTime<Utc>,
    ) -> Result<HybridRecallResult, Error> {
        self.recall_at(query, Visibility::Private, now)
    }

    /// Run TACT retrieval with a custom max_results (overriding the Brain's
    /// default TactConfig). Used by cascade to get K=40 through TACT's tiered
    /// search (fingerprint → wing → FTS) instead of bypassing it.
    pub fn tact_retrieve_with_k(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<spectral_ingest::MemoryHit>, Error> {
        let mut config = self.tact_config.clone();
        config.max_results = max_results;
        let result = self
            .rt
            .block_on(spectral_tact::retrieve(
                query,
                &config,
                self.memory_store.as_ref(),
            ))
            .map_err(|e| Error::Schema(e.to_string()))?;
        Ok(result.memories)
    }

    /// Combined TACT + FTS retrieval for cascade. Calls TACT first, then
    /// supplements with raw FTS if TACT returned fewer than K results.
    /// Deduplicates by memory key across both sources.
    pub fn cascade_retrieve(
        &self,
        query: &str,
        k: usize,
    ) -> Result<Vec<spectral_ingest::MemoryHit>, Error> {
        // Step 1: TACT retrieval
        let mut hits = self.tact_retrieve_with_k(query, k)?;

        // Step 2: If TACT returned fewer than K, supplement with FTS
        if hits.len() < k {
            let words = fts_query_words(query);

            if !words.is_empty() {
                let fts_hits = self.fts_search_direct(&words, k)?;

                // Dedup: only add FTS hits not already in TACT results
                let existing_keys: std::collections::HashSet<String> =
                    hits.iter().map(|h| h.key.clone()).collect();

                for fts_hit in fts_hits {
                    if hits.len() >= k {
                        break;
                    }
                    if !existing_keys.contains(&fts_hit.key) {
                        hits.push(fts_hit);
                    }
                }
            }
        }

        Ok(hits)
    }

    /// Direct FTS search bypassing TACT pipeline. Used by topk_fts
    /// for raw FTS access without TACT classification overhead.
    pub fn fts_search_direct(
        &self,
        words: &[String],
        max_results: usize,
    ) -> Result<Vec<spectral_ingest::MemoryHit>, Error> {
        self.rt
            .block_on(self.memory_store.fts_search(words, max_results))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Top-K FTS retrieval with additive re-ranking signals. No LLM cost.
    ///
    /// Retrieves `config.k × config.fetch_mult` candidates via full-text
    /// search, applies configurable re-ranking (signal score weighting,
    /// recency decay, entity clustering boost, context chain dedup), and
    /// returns the top `config.k` by final blended score. The widened fetch
    /// pool lets re-ranking recover evidence that broad (porter-stemmed)
    /// matching would otherwise bury below the bm25 LIMIT.
    pub fn recall_topk_fts(
        &self,
        query: &str,
        config: &RecallTopKConfig,
        visibility: Visibility,
    ) -> Result<Vec<spectral_ingest::MemoryHit>, Error> {
        // Sanitize: strip possessives / FTS5 special characters / short words.
        let words = fts_query_words(query);

        if words.is_empty() {
            return Ok(Vec::new());
        }

        let fetch_k = config.k.saturating_mul(config.fetch_mult.max(1));
        let mut candidates = self
            .rt
            .block_on(self.memory_store.fts_search(&words, fetch_k))
            .map_err(|e| Error::Schema(e.to_string()))?;

        // Filter by visibility
        candidates.retain(|m| str_to_vis(&m.visibility).allows(visibility));

        // Unified re-ranking pipeline
        let reranking_config = crate::ranking::RerankingConfig {
            apply_signal_score: config.apply_signal_score_weighting,
            signal_score_weight: 0.3,
            apply_recency: config.apply_recency_weighting,
            recency_half_life_days: config.recency_half_life_days,
            apply_entity_boost: config.apply_entity_resolution,
            entity_boost_weight: 0.05,
            apply_ambient_boost: false,
            ambient_weights: crate::cascade_layers::AmbientBoostWeights::default(),
            apply_declarative_boost: config.apply_declarative_boost,
            declarative_weight: 0.10,
            // Disabled: co-retrieval degrades real-workload relevance.
            // See docs/internal/tickets/coretrieval-regression.md.
            co_retrieval_weight: 0.0,
            apply_episode_diversity: false,
            max_per_episode: 5,
            apply_context_dedup: config.apply_context_dedup,
        };
        let ctx = match config.now {
            Some(dt) => spectral_cascade::RecognitionContext::empty().with_now(dt),
            None => spectral_cascade::RecognitionContext::empty(),
        };
        // Skip the co-retrieval DB queries (one per anchor) unless the boost is
        // actually weighted; at weight 0.0 the result is discarded downstream.
        let co_boosts = if reranking_config.co_retrieval_weight > 0.0 {
            crate::ranking::compute_co_retrieval_boosts(self, &candidates, 3)
        } else {
            std::collections::HashMap::new()
        };
        let mut results = crate::ranking::apply_reranking_pipeline(
            candidates,
            &reranking_config,
            &ctx,
            &co_boosts,
        );
        // Truncate the widened re-rank pool back to the requested k.
        results.truncate(config.k);

        // Best-effort retrieval event logging. Skipped read-only: recall
        // over a brain you don't own must not write your query metadata
        // into its store.
        if !self.read_only {
            let memory_ids: Vec<&str> = results.iter().map(|h| h.id.as_str()).collect();
            let event = spectral_ingest::RetrievalEvent {
                query_hash: spectral_ingest::hash_query(query),
                timestamp: chrono::Utc::now().to_rfc3339(),
                memory_ids_json: serde_json::to_string(&memory_ids).unwrap_or_default(),
                method: "topk_fts".into(),
                wing: None,
                question_type: None,
                session_id: None,
            };
            let _ = self.log_retrieval_event(&event);
        }

        // Anticipatory augmentation (opt-in, `SPECTRAL_ANTICIPATORY_RECALL=1`).
        // Surface memories the query did NOT match but that the top hits are
        // specifically associated with (lift over co-retrieval history) — "what
        // you need before you ask". Appended AFTER the k query-matches (they
        // supplement, never displace) and AFTER event logging (anticipated
        // memories aren't logged as retrieved, avoiding a feedback runaway).
        // Visibility-filtered like everything else.
        if !results.is_empty() && fts_anticipatory_enabled() {
            self.append_anticipated(&mut results, visibility);
        }

        Ok(results)
    }

    /// Append lift-associated memories (anticipatory recall) to a result set,
    /// skipping ones already present, respecting `visibility`, capped small.
    fn append_anticipated(
        &self,
        results: &mut Vec<spectral_ingest::MemoryHit>,
        visibility: Visibility,
    ) {
        const MAX_ANTICIPATED: usize = 3;
        const MIN_LIFT: f64 = 1.0; // above-baseline association only
        const SEED_HITS: usize = 3;

        let mut present: std::collections::HashSet<String> =
            results.iter().map(|h| h.id.clone()).collect();
        let seeds: Vec<String> = results.iter().take(SEED_HITS).map(|h| h.id.clone()).collect();
        let mut added = 0usize;
        for seed in seeds {
            if added >= MAX_ANTICIPATED {
                break;
            }
            let recs = self
                .rt
                .block_on(self.memory_store.recommend_by_lift(&seed, MAX_ANTICIPATED, 2))
                .unwrap_or_default();
            for r in recs {
                if added >= MAX_ANTICIPATED {
                    break;
                }
                if r.lift < MIN_LIFT || present.contains(&r.memory_id) {
                    continue;
                }
                if let Ok(Some(m)) = self.get_memory(&r.memory_id) {
                    if !str_to_vis(&m.visibility).allows(visibility) {
                        continue;
                    }
                    present.insert(r.memory_id.clone());
                    results.push(memory_to_hit(m));
                    added += 1;
                }
            }
        }
    }

    /// Run the integrated retrieval pipeline with ambient boost.
    ///
    /// TACT tiered search (fingerprint → wing → FTS fallback) supplemented
    /// by raw FTS, then unified re-ranking: signal blend, ambient boost,
    /// declarative density, co-retrieval, recency decay, entity cluster
    /// boost, episode diversity cap, context chain dedup.
    pub fn recall_cascade(
        &self,
        query: &str,
        context: &spectral_cascade::RecognitionContext,
        pipeline_config: &crate::cascade_layers::CascadePipelineConfig,
    ) -> Result<spectral_cascade::result::CascadeResult, Error> {
        self.recall_cascade_with_pipeline(query, context, pipeline_config)
    }

    /// Run the retrieval pipeline with a caller-supplied pipeline config.
    ///
    /// The bench harness uses this to pass question-type-tuned profiles
    /// (varying K, episode diversity, recency half-life per question shape).
    pub fn recall_cascade_with_pipeline(
        &self,
        query: &str,
        context: &spectral_cascade::RecognitionContext,
        pipeline_config: &crate::cascade_layers::CascadePipelineConfig,
    ) -> Result<spectral_cascade::result::CascadeResult, Error> {
        let hits =
            crate::cascade_layers::run_cascade_pipeline(self, query, context, pipeline_config)?;

        let tokens_used = hits.iter().map(|h| h.content.len() / 4 + 5).sum();
        let max_confidence = hits
            .first()
            .map(|h| h.signal_score.min(0.85))
            .unwrap_or(0.0);

        Ok(spectral_cascade::result::CascadeResult {
            merged_hits: hits,
            total_tokens_used: tokens_used,
            max_confidence,
            total_recognition_token_cost: 0,
        })
    }

    /// Graph-only recall filtered by visibility context.
    pub fn recall_graph(
        &self,
        query: &str,
        context_visibility: Visibility,
    ) -> Result<RecallResult, Error> {
        let canonicalizer = Canonicalizer::new(&self.ontology);
        let result = canonicalizer.canonicalize(query);

        let seed_entities: Vec<EntityId> = result.matched.iter().map(|m| m.entity_id).collect();

        if seed_entities.is_empty() {
            return Ok(RecallResult {
                seed_entities: vec![],
                triples: vec![],
                neighborhood: Neighborhood {
                    entities: vec![],
                    triples: vec![],
                    documents: vec![],
                },
            });
        }

        let mut all_entity_ids = HashSet::new();
        let mut all_entities = Vec::new();
        let mut all_triples = Vec::new();
        let mut seen_edges: HashSet<(EntityId, EntityId, String)> = HashSet::new();
        let mut seen_docs: HashSet<[u8; 32]> = HashSet::new();
        let mut all_documents = Vec::new();

        for seed in &seed_entities {
            let hood = self.store.neighborhood(seed, 2)?;
            for entity in hood.entities {
                if all_entity_ids.insert(entity.id) {
                    all_entities.push(entity);
                }
            }
            for triple in hood.triples {
                let key = (triple.from, triple.to, triple.predicate.clone());
                if seen_edges.insert(key) {
                    all_triples.push(triple);
                }
            }
            for doc in hood.documents {
                if seen_docs.insert(doc.id) {
                    all_documents.push(doc);
                }
            }
        }

        // Filter by visibility
        let all_entities: Vec<_> = all_entities
            .into_iter()
            .filter(|e| e.visibility.allows(context_visibility))
            .collect();
        let all_triples: Vec<_> = all_triples
            .into_iter()
            .filter(|t| t.visibility.allows(context_visibility))
            .collect();
        let all_documents: Vec<_> = all_documents
            .into_iter()
            .filter(|d| d.visibility.allows(context_visibility))
            .collect();
        let triples_clone = all_triples.clone();

        Ok(RecallResult {
            seed_entities,
            triples: triples_clone,
            neighborhood: Neighborhood {
                entities: all_entities,
                triples: all_triples,
                documents: all_documents,
            },
        })
    }

    /// Ingest a document: hash content, upsert Document node, link mentions.
    pub fn ingest_document(
        &self,
        source: &str,
        content: &str,
        visibility: Visibility,
    ) -> Result<IngestResult, Error> {
        self.ensure_writable("ingest_document")?;
        let document_id = *blake3::hash(content.as_bytes()).as_bytes();

        self.store
            .upsert_document(&document_id, source, visibility)?;

        let canonicalizer = Canonicalizer::new(&self.ontology);
        let canon_result = canonicalizer.canonicalize(content);

        let now = Utc::now();
        for mention in &canon_result.matched {
            self.store.upsert_entity(&Entity {
                id: mention.entity_id,
                entity_type: mention.entity_type.clone(),
                canonical: mention.canonical.clone(),
                visibility,
                created_at: now,
                updated_at: now,
                weight: 1.0,
                description: None,
            })?;

            self.store.insert_mention(
                &document_id,
                &mention.entity_id,
                mention.span.0 as i64,
                mention.span.1 as i64,
            )?;
        }

        let unresolved_count = canon_result.unresolved.len();

        Ok(IngestResult {
            document_id,
            matched: canon_result.matched,
            unresolved_count,
        })
    }

    /// Direct access to the underlying graph store.
    pub fn store(&self) -> &GraphStore {
        &self.store
    }

    /// Count retrieval events in the database (for testing the feedback loop).
    pub fn count_retrieval_events(&self) -> Result<usize, Error> {
        self.rt
            .block_on(self.memory_store.count_retrieval_events())
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Count retrieval events filtered by method (for testing).
    pub fn count_retrieval_events_by_method(&self, method: &str) -> Result<usize, Error> {
        self.rt
            .block_on(self.memory_store.count_retrieval_events_by_method(method))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Fetch a memory by ID. Returns None if not found.
    pub fn get_memory(&self, id: &str) -> Result<Option<spectral_ingest::Memory>, Error> {
        self.rt
            .block_on(self.memory_store.get_memory(id))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Set the description field on a memory and update description_generated_at to now.
    pub fn set_description(&self, id: &str, description: &str) -> Result<(), Error> {
        self.ensure_writable("set_description")?;
        self.rt
            .block_on(self.memory_store.set_description(id, description))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Set the description on a graph entity. Idempotent.
    pub fn set_entity_description(
        &self,
        entity_id: &EntityId,
        description: &str,
    ) -> Result<(), Error> {
        self.ensure_writable("set_entity_description")?;
        self.store.set_entity_description(entity_id, description)
    }

    /// Write (insert-or-update) a typed field on an entity, with provenance.
    ///
    /// Enforces the manual-not-clobbered rule in the store: an `Enriched`
    /// write never overwrites a `Manual` field. Returns `false` when such a
    /// write was suppressed, `true` when applied.
    pub fn set_entity_field(
        &self,
        entity_id: &EntityId,
        field_name: &str,
        value: &str,
        source: spectral_ingest::FieldSource,
        source_url: Option<&str>,
    ) -> Result<bool, Error> {
        self.rt
            .block_on(self.memory_store.set_entity_field(
                &entity_id.to_string(),
                field_name,
                value,
                source,
                source_url,
            ))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Read all typed fields for an entity (provenance included).
    pub fn get_entity_fields(
        &self,
        entity_id: &EntityId,
    ) -> Result<Vec<spectral_ingest::EntityField>, Error> {
        self.rt
            .block_on(self.memory_store.get_entity_fields(&entity_id.to_string()))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// List memories where description IS NULL, ordered by created_at DESC.
    pub fn list_undescribed(&self, limit: usize) -> Result<Vec<spectral_ingest::Memory>, Error> {
        self.rt
            .block_on(self.memory_store.list_undescribed(limit))
            .map_err(|e| Error::Schema(e.to_string()))
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
        self.ensure_writable("consolidate_into")?;
        self.rt
            .block_on(
                self.memory_store
                    .consolidate_into(source_keys, target_key, opts),
            )
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Hard-delete a memory by key across every substrate, then verify it is
    /// gone — the "verified forgetting" / right-to-be-forgotten primitive.
    ///
    /// Unlike [`consolidate_into`](Self::consolidate_into) (which only hides a
    /// source from two read paths while the row and all its fingerprints,
    /// spectrograms, and recognition features persist), this removes:
    /// the `memories` row and its FTS shadow; constellation fingerprints;
    /// spectrogram; annotations; consolidation edges; co-retrieval pairs;
    /// retrieval-event references; and the recognition-sidecar pair/gram
    /// index. It then re-probes recall and recognition for the deleted
    /// content and reports whether both are clear.
    ///
    /// Graph triples (from `assert`/`ingest_*`) are a separate provenance
    /// substrate keyed by entity/document, not by memory key, and are not
    /// touched here.
    ///
    /// Returns a [`ForgetReport`]; `report.store.existed == false` means no
    /// such memory. Errors only on store failure.
    pub fn forget(&self, key: &str) -> Result<ForgetReport, Error> {
        self.ensure_writable("forget")?;

        // Capture the content before deletion so the verification probe has
        // something to search for; also gives us the id for recognition.
        let memory_id = format!(
            "{:016x}",
            u64::from_be_bytes(
                blake3::hash(key.as_bytes()).as_bytes()[..8]
                    .try_into()
                    .unwrap()
            )
        );
        let content = self
            .get_memory(&memory_id)?
            .map(|m| m.content)
            .unwrap_or_default();

        let store = self
            .rt
            .block_on(self.memory_store.delete_memory_by_key(key))
            .map_err(|e| Error::Schema(e.to_string()))?;

        // Recognition sidecar is a separate DB with no FK to memories.
        let recognition_removed = match self.recognition.lock() {
            Ok(mut engine) => engine.forget(&memory_id).unwrap_or(false),
            Err(_) => false,
        };

        // Verification probe. If the memory never existed, treat probes as
        // vacuously clear so `fully_forgotten()` keys off `existed`.
        let (recall_clear, recognize_clear) = if !store.existed || content.is_empty() {
            (true, true)
        } else {
            let recall_clear = self
                .recall_topk_fts(&content, &RecallTopKConfig::default(), Visibility::Private)
                .map(|hits| !hits.iter().any(|h| h.key == key))
                .unwrap_or(true);
            let recognize_clear = match self.recognize(&content) {
                Ok(r) => !matches!(
                    r.verdict,
                    spectral_recognition::Verdict::Recognized { memory_id: ref mid } if *mid == memory_id
                ),
                Err(_) => true,
            };
            (recall_clear, recognize_clear)
        };

        Ok(ForgetReport {
            store,
            recognition_removed,
            recall_clear,
            recognize_clear,
        })
    }

    /// List consolidation edges, optionally filtered to a specific target.
    pub fn list_consolidated(
        &self,
        target_key: Option<&str>,
    ) -> Result<Vec<spectral_ingest::ConsolidationEdge>, Error> {
        self.rt
            .block_on(self.memory_store.list_consolidated(target_key))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// List memory keys not consolidated as sources.
    pub fn list_unconsolidated(&self, limit: usize) -> Result<Vec<String>, Error> {
        self.rt
            .block_on(self.memory_store.list_unconsolidated(limit))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    // ── Layered / provenance-linked recall + ambient consolidation ──────────
    //
    // Deterministic, LLM-optional layered memory. Recall surfaces compact
    // abstract memories and (on demand) their ground-truth sources; the
    // recognition/co-retrieval ambient signals pick what recurs enough to
    // abstract; the summarizer that produces an abstraction is a pluggable
    // closure (extractive `$0` default, or a sparse consumer-supplied LLM).

    /// Recall with provenance-linked drill-down: run normal recall (which
    /// surfaces abstract/consolidated memories and hides their now-consolidated
    /// sources), then attach each hit's source memories from
    /// `consolidation_edges`, visibility-filtered and capped at
    /// `max_sources_per_hit`. The actor gets a compact context *plus* the exact
    /// evidence each summary rests on — the deterministic analogue of
    /// abstraction→ground-truth drill-down, at no extra LLM cost.
    pub fn recall_with_provenance(
        &self,
        query: &str,
        config: &RecallTopKConfig,
        visibility: Visibility,
        max_sources_per_hit: usize,
    ) -> Result<Vec<LayeredHit>, Error> {
        let hits = self.recall_topk_fts(query, config, visibility)?;
        let mut out = Vec::with_capacity(hits.len());
        for hit in hits {
            let mut sources = Vec::new();
            if max_sources_per_hit > 0 {
                for edge in self.list_consolidated(Some(&hit.key))? {
                    if sources.len() >= max_sources_per_hit {
                        break;
                    }
                    if let Ok(Some(m)) = self.get_memory(&key_to_id(&edge.source_key)) {
                        if str_to_vis(&m.visibility).allows(visibility) {
                            sources.push(memory_to_hit(m));
                        }
                    }
                }
            }
            out.push(LayeredHit { hit, sources });
        }
        Ok(out)
    }

    /// Surface consolidation candidates from the **co-retrieval ambient signal**:
    /// cluster unconsolidated memories that the user's usage repeatedly pulls
    /// together (`co_count ≥ min_co_count`). These recurring groups are the
    /// high-value targets for abstraction, so a downstream summarizer only ever
    /// runs on them. Deterministic, `$0`; empty until co-retrieval history
    /// exists (rebuild it with [`rebuild_co_retrieval_index`](Self::rebuild_co_retrieval_index)).
    /// Recognition recurrence (the spectrogram/MinHash re-encounter signal,
    /// surfaced at write time via [`RememberResult::recurrence`]) is the
    /// complementary content-similarity signal.
    pub fn consolidation_candidates(
        &self,
        min_co_count: u64,
        scan_limit: usize,
    ) -> Result<Vec<ConsolidationCandidate>, Error> {
        use std::collections::HashMap;
        let keys = self.list_unconsolidated(scan_limit)?;
        if keys.len() < 2 {
            return Ok(Vec::new());
        }
        // Union-find over the scan set; union two memories when they co-occur in
        // retrievals at least `min_co_count` times. Track summed co_count per
        // cluster for a cohesion score.
        let in_set: std::collections::HashSet<&str> = keys.iter().map(|s| s.as_str()).collect();
        let mut parent: HashMap<String, String> = keys.iter().map(|k| (k.clone(), k.clone())).collect();
        fn find(parent: &mut HashMap<String, String>, k: &str) -> String {
            let p = parent.get(k).cloned().unwrap_or_else(|| k.to_string());
            if p == k {
                return p;
            }
            let root = find(parent, &p);
            parent.insert(k.to_string(), root.clone());
            root
        }
        let mut edge_strength: HashMap<(String, String), u64> = HashMap::new();
        for key in &keys {
            let related = self
                .related_memories(&key_to_id(key), 20)
                .unwrap_or_default();
            for rel in related {
                if rel.co_count < min_co_count {
                    continue;
                }
                // related_memories keys by memory_id; recover the key via the row.
                let other_key = match self.get_memory(&rel.memory_id) {
                    Ok(Some(m)) => m.key,
                    _ => continue,
                };
                if !in_set.contains(other_key.as_str()) || other_key == *key {
                    continue;
                }
                let pair = if *key < other_key {
                    (key.clone(), other_key.clone())
                } else {
                    (other_key.clone(), key.clone())
                };
                edge_strength.insert(pair.clone(), rel.co_count);
                let a = find(&mut parent, key);
                let b = find(&mut parent, &other_key);
                if a != b {
                    parent.insert(a, b);
                }
            }
        }
        // Group keys by cluster root.
        let mut clusters: HashMap<String, Vec<String>> = HashMap::new();
        for key in &keys {
            let root = find(&mut parent, key);
            clusters.entry(root).or_default().push(key.clone());
        }
        let max_co = edge_strength.values().copied().max().unwrap_or(1) as f64;
        let mut out: Vec<ConsolidationCandidate> = clusters
            .into_values()
            .filter(|members| members.len() >= 2)
            .map(|mut members| {
                members.sort();
                // Cohesion: mean of the cluster's internal edge strengths, normalized.
                let strengths: Vec<u64> = edge_strength
                    .iter()
                    .filter(|((a, b), _)| members.contains(a) && members.contains(b))
                    .map(|(_, &c)| c)
                    .collect();
                let cohesion = if strengths.is_empty() {
                    0.0
                } else {
                    (strengths.iter().sum::<u64>() as f64 / strengths.len() as f64) / max_co
                };
                ConsolidationCandidate {
                    member_keys: members,
                    cohesion,
                    signal: "co_retrieval",
                }
            })
            .collect();
        out.sort_by(|a, b| b.cohesion.partial_cmp(&a.cohesion).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out)
    }

    /// Consolidate `source_keys` into a single higher-tier memory at
    /// `target_key`, whose content is produced by `summarize` (given the source
    /// contents in order). This is the one seam where an LLM *may* be used —
    /// pass a sparse consumer-supplied closure for a real abstraction, or the
    /// deterministic extractive default (see
    /// [`consolidate_extractive`](Self::consolidate_extractive)) for `$0`. The
    /// resulting memory is tagged `compaction_tier` and the sources are linked
    /// to it via `consolidation_edges` (hiding them from ordinary recall while
    /// keeping them reachable through [`recall_with_provenance`](Self::recall_with_provenance)).
    /// Returns the abstraction's `RememberResult`.
    pub fn consolidate_with<F>(
        &self,
        source_keys: &[String],
        target_key: &str,
        tier: spectral_ingest::CompactionTier,
        summarize: F,
    ) -> Result<RememberResult, Error>
    where
        F: FnOnce(&[String]) -> String,
    {
        self.ensure_writable("consolidate_with")?;
        // Gather source contents (skip missing).
        let mut contents = Vec::with_capacity(source_keys.len());
        for k in source_keys {
            if let Some(m) = self.get_memory(&key_to_id(k))? {
                contents.push(m.content);
            }
        }
        if contents.is_empty() {
            return Err(Error::Schema("no valid source memories to consolidate".into()));
        }
        let summary = summarize(&contents);
        // Write the abstraction, tagged with its compaction tier.
        let result = self.remember_with(
            target_key,
            &summary,
            RememberOpts {
                visibility: Visibility::Private,
                compaction_tier: Some(tier),
                ..Default::default()
            },
        )?;
        // Link sources → target (hides sources from ordinary recall; provenance
        // preserved).
        self.consolidate_into(
            source_keys,
            target_key,
            &spectral_ingest::ConsolidateOpts::default(),
        )?;
        Ok(result)
    }

    /// Deterministic `$0` extractive summary: the longest source content (a
    /// reasonable "most complete restatement" heuristic), used as the default
    /// abstraction when no LLM summarizer is supplied. Convenience wrapper over
    /// [`consolidate_with`](Self::consolidate_with).
    pub fn consolidate_extractive(
        &self,
        source_keys: &[String],
        target_key: &str,
        tier: spectral_ingest::CompactionTier,
    ) -> Result<RememberResult, Error> {
        self.consolidate_with(source_keys, target_key, tier, |contents| {
            contents
                .iter()
                .max_by_key(|c| c.len())
                .cloned()
                .unwrap_or_default()
        })
    }

    /// Store a **pre-computed** abstraction over `source_keys` — the entry point
    /// for an external Librarian that generated the atom offline with a strong
    /// model. Identical storage semantics to
    /// [`consolidate_with`](Self::consolidate_with) (higher-tier memory + source
    /// linkage), but the caller supplies the final `content` directly instead of
    /// a closure. Because the sources stay reachable via
    /// [`recall_with_provenance`](Self::recall_with_provenance), the atom is an
    /// additive hint, never an authoritative lossy replacement.
    pub fn consolidate_as(
        &self,
        source_keys: &[String],
        target_key: &str,
        tier: spectral_ingest::CompactionTier,
        content: &str,
    ) -> Result<RememberResult, Error> {
        self.consolidate_with(source_keys, target_key, tier, |_| content.to_string())
    }

    /// Return memories most frequently co-retrieved with the given memory_id.
    pub fn related_memories(
        &self,
        memory_id: &str,
        limit: usize,
    ) -> Result<Vec<spectral_ingest::RelatedMemory>, Error> {
        self.rt
            .block_on(self.memory_store.related_memories(memory_id, limit))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Anticipatory recall: recommend memories associated with `memory_id`,
    /// ranked by **lift** (context-specific association) rather than raw
    /// co-retrieval count. This is the ambient read-time signal — surfacing
    /// the memories the user's current context is *specifically* associated
    /// with, deterministically and with no LLM. Lift suppresses
    /// globally-popular memories (the bias that sank raw co-retrieval), the
    /// same way recommender systems avoid recommending bestsellers to everyone.
    /// `min_co_count` filters low-evidence pairs (2 is a sensible floor).
    pub fn recommend(
        &self,
        memory_id: &str,
        limit: usize,
        min_co_count: u64,
    ) -> Result<Vec<spectral_ingest::RelatedMemory>, Error> {
        self.rt
            .block_on(
                self.memory_store
                    .recommend_by_lift(memory_id, limit, min_co_count),
            )
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Rebuild the co_retrieval_pairs index from retrieval_events data.
    /// Returns the number of pairs written.
    pub fn rebuild_co_retrieval_index(&self) -> Result<usize, Error> {
        self.ensure_writable("rebuild_co_retrieval_index")?;
        self.rt
            .block_on(self.memory_store.rebuild_co_retrieval_index())
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Backfill content_hash for all rows with NULL content_hash.
    /// Returns count of rows updated. Idempotent — safe to re-run.
    pub fn backfill_content_hashes(&self) -> Result<usize, Error> {
        self.ensure_writable("backfill_content_hashes")?;
        self.rt
            .block_on(self.memory_store.backfill_content_hashes())
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// List retrieval events for a given session, ordered by timestamp ASC.
    pub fn events_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<spectral_ingest::RetrievalEvent>, Error> {
        self.rt
            .block_on(self.memory_store.events_for_session(session_id, limit))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// List unique memory IDs that surfaced in a session, ordered by first appearance.
    pub fn memories_for_session(&self, session_id: &str) -> Result<Vec<String>, Error> {
        self.rt
            .block_on(self.memory_store.memories_for_session(session_id))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Direct access to the ontology.
    pub fn ontology(&self) -> &Ontology {
        &self.ontology
    }

    /// Find memories across wings that resonate with a query memory's cognitive fingerprint.
    ///
    /// Flow: recall the best memory for the seed query, compute or load its spectrogram,
    /// load spectrograms from other wings, find resonant matches, and return seed + resonant
    /// memories with scores. Requires `enable_spectrogram = true` in BrainConfig.
    /// Uses the default [`MatchTolerances`](spectral_spectrogram::matching::MatchTolerances);
    /// call [`recall_cross_wing_with`](Self::recall_cross_wing_with) to tune the
    /// precision/recall frontier.
    pub fn recall_cross_wing(
        &self,
        seed_query: &str,
        visibility: Visibility,
        max_results: usize,
    ) -> Result<CrossWingRecallResult, Error> {
        self.recall_cross_wing_with(
            seed_query,
            visibility,
            max_results,
            &spectral_spectrogram::matching::MatchTolerances::default(),
        )
    }

    /// [`recall_cross_wing`](Self::recall_cross_wing) with explicit resonance
    /// tolerances. Tighter tolerances / higher `min_matching_dimensions` raise
    /// precision (only near-identical cognitive shapes resonate); looser ones
    /// raise recall. See the `spectrogram_resonance_scale` bench for the swept
    /// frontier.
    pub fn recall_cross_wing_with(
        &self,
        seed_query: &str,
        visibility: Visibility,
        max_results: usize,
        tolerances: &spectral_spectrogram::matching::MatchTolerances,
    ) -> Result<CrossWingRecallResult, Error> {
        // Recall the best match for seed_query
        let recall = self.recall(seed_query, visibility)?;
        let seed_memory = recall.memory_hits.into_iter().next();

        let seed = match &seed_memory {
            Some(m) => m,
            None => {
                return Ok(CrossWingRecallResult {
                    seed_memory: None,
                    resonant_memories: vec![],
                })
            }
        };

        // Get or compute the seed's spectrogram
        let seed_fp = {
            let existing = self
                .rt
                .block_on(self.memory_store.load_spectrogram(&seed.id))
                .map_err(|e| Error::Schema(e.to_string()))?;

            if let Some(row) = existing {
                row_to_fingerprint(&row)
            } else {
                // Compute on the fly
                let mem = spectral_ingest::Memory {
                    id: seed.id.clone(),
                    key: seed.key.clone(),
                    content: seed.content.clone(),
                    wing: seed.wing.clone(),
                    hall: seed.hall.clone(),
                    signal_score: seed.signal_score,
                    visibility: seed.visibility.clone(),
                    source: seed.source.clone(),
                    device_id: seed.device_id,
                    confidence: seed.confidence,
                    created_at: seed.created_at.clone(),
                    last_reinforced_at: seed.last_reinforced_at.clone(),
                    episode_id: seed.episode_id.clone(),
                    compaction_tier: None,
                    declarative_density: seed.declarative_density,
                    description: None,
                    description_generated_at: None,
                    content_hash: None,
                    source_brain_id: None,
                    signature: None,
                };
                let context = self.spectrogram_context(mem.wing.as_deref(), &mem.id);
                self.spectrogram_analyzer.analyze(&mem, &context)
            }
        };

        // Load spectrograms from OTHER wings
        let all_spectrograms = self
            .rt
            .block_on(self.memory_store.load_spectrograms(None, 500))
            .map_err(|e| Error::Schema(e.to_string()))?;

        let seed_wing = seed.wing.as_deref();
        let other_wing_fps: Vec<spectral_spectrogram::SpectralFingerprint> = all_spectrograms
            .iter()
            .filter(|row| {
                // Exclude same wing
                match (row.wing.as_deref(), seed_wing) {
                    (Some(rw), Some(sw)) => rw != sw,
                    _ => true,
                }
            })
            .map(row_to_fingerprint)
            .collect();

        // Find resonant matches
        let resonant = spectral_spectrogram::matching::find_resonant(
            &seed_fp,
            &other_wing_fps,
            max_results,
            tolerances,
        );

        // Fetch full memories for resonant matches
        let resonant_ids: Vec<String> = resonant.iter().map(|r| r.memory_id.clone()).collect();
        let resonant_mems = self
            .rt
            .block_on(self.memory_store.fetch_by_ids(&resonant_ids))
            .map_err(|e| Error::Schema(e.to_string()))?;

        let mut resonant_memories = Vec::new();
        for rmatch in &resonant {
            if let Some(mem) = resonant_mems.iter().find(|m| m.id == rmatch.memory_id) {
                // Visibility filter
                if !str_to_vis(&mem.visibility).allows(visibility) {
                    continue;
                }
                resonant_memories.push(ResonantMemoryHit {
                    memory: MemoryHit {
                        id: mem.id.clone(),
                        key: mem.key.clone(),
                        content: mem.content.clone(),
                        wing: mem.wing.clone(),
                        hall: mem.hall.clone(),
                        signal_score: mem.signal_score,
                        visibility: mem.visibility.clone(),
                        hits: 0,
                        source: mem.source.clone(),
                        device_id: mem.device_id,
                        confidence: mem.confidence,
                        created_at: mem.created_at.clone(),
                        last_reinforced_at: mem.last_reinforced_at.clone(),
                        episode_id: mem.episode_id.clone(),
                        declarative_density: mem.declarative_density,
                        description: mem.description.clone(),
                        source_brain_id: None,
                        signature: None,
                    },
                    resonance_score: rmatch.resonance_score,
                    matched_dimensions: rmatch.matched_dimensions.clone(),
                });
            }
        }

        Ok(CrossWingRecallResult {
            seed_memory,
            resonant_memories,
        })
    }

    /// Compute and store spectrograms for memories that don't have one.
    /// Returns count of spectrograms generated. Idempotent.
    pub fn backfill_spectrograms(&self) -> Result<usize, Error> {
        self.ensure_writable("backfill_spectrograms")?;
        let mut total = 0;
        loop {
            let ids = self
                .rt
                .block_on(self.memory_store.memories_without_spectrogram(100))
                .map_err(|e| Error::Schema(e.to_string()))?;

            if ids.is_empty() {
                break;
            }

            let memories = self
                .rt
                .block_on(self.memory_store.fetch_by_ids(&ids))
                .map_err(|e| Error::Schema(e.to_string()))?;

            for mem in &memories {
                let context = self.spectrogram_context(mem.wing.as_deref(), &mem.id);
                let fp = self.spectrogram_analyzer.analyze(mem, &context);
                let peak_json = serde_json::to_string(&fp.peak_dimensions).unwrap_or_default();
                self.rt
                    .block_on(self.memory_store.write_spectrogram(
                        &mem.id,
                        fp.entity_density,
                        fp.action_type.as_str(),
                        fp.decision_polarity,
                        fp.causal_depth,
                        fp.emotional_valence,
                        fp.temporal_specificity,
                        fp.novelty,
                        &peak_json,
                    ))
                    .map_err(|e| Error::Schema(e.to_string()))?;
                total += 1;
            }
        }
        Ok(total)
    }

    /// Backfill declarative_density for memories that don't have one.
    /// Computes density from content and stores the result. Returns count updated.
    pub fn backfill_declarative_density(&self) -> Result<usize, Error> {
        self.ensure_writable("backfill_declarative_density")?;
        // Fetch memories with NULL declarative_density in batches
        let mut total = 0;
        loop {
            let memories = self
                .rt
                .block_on(async { self.memory_store.list_memories_by_signal(0.0, 200).await })
                .map_err(|e| Error::Schema(e.to_string()))?;

            let batch: Vec<_> = memories
                .into_iter()
                .filter(|m| m.declarative_density.is_none())
                .collect();

            if batch.is_empty() {
                break;
            }

            for mem in &batch {
                let density = crate::ranking::declarative_density(&mem.content);
                self.rt
                    .block_on(self.memory_store.set_declarative_density(&mem.id, density))
                    .map_err(|e| Error::Schema(e.to_string()))?;
                total += 1;
            }

            // If we processed fewer than 200, we've seen all memories
            if batch.len() < 200 {
                break;
            }
        }
        Ok(total)
    }

    /// Backfill time_delta_bucket on existing constellation fingerprints.
    /// Recomputes bucket from anchor/target memory created_at timestamps and
    /// updates the fingerprint hash to match. Returns count of updated rows.
    pub fn backfill_fingerprint_time_buckets(&self) -> Result<usize, Error> {
        self.ensure_writable("backfill_fingerprint_time_buckets")?;
        self.rt
            .block_on(self.memory_store.backfill_fingerprint_time_buckets())
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Reinforce memories that the caller found useful from a recall result.
    ///
    /// Increases signal_score by `strength` (clamped to 1.0) and updates
    /// `last_reinforced_at` to now. This resets the decay clock for those
    /// memories, causing them to rank higher in future recalls.
    pub fn reinforce(&self, opts: ReinforceOpts) -> Result<ReinforceResult, Error> {
        self.ensure_writable("reinforce")?;
        let mut memories_reinforced = 0;
        let mut memories_not_found = Vec::new();

        for key in &opts.memory_keys {
            let wing = self
                .rt
                .block_on(self.memory_store.reinforce_memory(key, opts.strength))
                .map_err(|e| Error::Schema(e.to_string()))?;

            match wing {
                Some(_) => memories_reinforced += 1,
                None => memories_not_found.push(key.clone()),
            }
        }

        Ok(ReinforceResult {
            memories_reinforced,
            memories_not_found,
        })
    }

    /// Reinforce a single memory by key. Convenience for auto-reinforce.
    /// Returns Ok(()) on success or if memory not found (best-effort).
    pub(crate) fn reinforce_by_id(&self, key: &str, strength: f64) -> Result<(), Error> {
        let _ = self
            .rt
            .block_on(self.memory_store.reinforce_memory(key, strength))
            .map_err(|e| Error::Schema(e.to_string()))?;
        Ok(())
    }

    /// Log a retrieval event (best-effort). Failures are silently ignored.
    pub(crate) fn log_retrieval_event(
        &self,
        event: &spectral_ingest::RetrievalEvent,
    ) -> Result<(), Error> {
        self.rt
            .block_on(self.memory_store.log_retrieval_event(event))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Extract triples from natural-language text, validate against ontology,
    /// assert valid triples, and store the original text as a memory.
    ///
    /// Requires a configured `LlmClient`.
    pub fn ingest_text(&self, text: &str, opts: IngestTextOpts) -> Result<IngestTextResult, Error> {
        self.ensure_writable("ingest_text")?;
        let llm = self.llm_client.as_ref().ok_or(Error::MissingLlmClient)?;

        // Build prompt with ontology predicates
        let predicate_names: Vec<String> = self
            .ontology
            .predicates
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let prompt = ExtractionPrompt::build(text, &predicate_names);

        // Call LLM
        let response = self
            .rt
            .block_on(llm.complete(&prompt))
            .map_err(|e| Error::Llm(e.to_string()))?;

        // Parse response
        let extracted = ExtractionPrompt::parse(&response);
        let triples_extracted = extracted.len();

        let mut triples_asserted = 0;
        let mut triples_rejected = Vec::new();

        for triple in extracted {
            // Check confidence threshold
            if triple.confidence < opts.min_confidence {
                triples_rejected.push(RejectedTriple {
                    raw: triple,
                    reason: RejectionReason::BelowConfidenceThreshold,
                });
                continue;
            }

            // Try to assert — uses existing canonicalization + ontology validation
            match self.assert(
                &triple.subject,
                &triple.predicate,
                &triple.object,
                triple.confidence,
                opts.visibility,
            ) {
                Ok(_) => {
                    triples_asserted += 1;
                }
                Err(Error::UnresolvedMention { mention, .. }) => {
                    let reason = if mention == triple.subject {
                        RejectionReason::UnresolvedSubject
                    } else {
                        RejectionReason::UnresolvedObject
                    };
                    triples_rejected.push(RejectedTriple {
                        raw: triple,
                        reason,
                    });
                }
                Err(Error::InvalidPredicate { predicate, .. }) => {
                    triples_rejected.push(RejectedTriple {
                        raw: triple,
                        reason: RejectionReason::InvalidPredicate(predicate),
                    });
                }
                Err(Error::Ontology(_)) => {
                    triples_rejected.push(RejectedTriple {
                        raw: triple.clone(),
                        reason: RejectionReason::InvalidPredicate(triple.predicate),
                    });
                }
                Err(e) => return Err(e),
            }
        }

        // Store original text as memory
        let memory_key = opts.memory_key.unwrap_or_else(|| {
            format!(
                "ingest:{:016x}",
                u64::from_be_bytes(
                    blake3::hash(text.as_bytes()).as_bytes()[..8]
                        .try_into()
                        .unwrap(),
                )
            )
        });

        let memory = self.remember_with(
            &memory_key,
            text,
            RememberOpts {
                source: opts.source,
                device_id: opts.device_id,
                visibility: opts.visibility,
                ..Default::default()
            },
        )?;

        Ok(IngestTextResult {
            memory,
            triples_extracted,
            triples_asserted,
            triples_rejected,
        })
    }

    // ── Activity ingestion ──────────────────────────────────────────

    /// Ingest a batch of activity episodes. Idempotent on episode.id (UPSERT).
    /// Applies the configured RedactionPolicy before storage.
    pub fn ingest_activity(
        &self,
        episodes: &[crate::activity::ActivityEpisode],
    ) -> Result<crate::activity::IngestActivityStats, Error> {
        self.ensure_writable("ingest_activity")?;
        use crate::activity::IngestActivityStats;

        let mut stats = IngestActivityStats {
            episodes_received: episodes.len(),
            ..Default::default()
        };

        for episode in episodes {
            // Apply redaction policy
            let redacted = match self.redaction_policy.redact(episode.clone()) {
                Some(ep) => {
                    if ep.window_title != episode.window_title
                        || ep.url != episode.url
                        || ep.excerpt != episode.excerpt
                    {
                        stats.episodes_redacted += 1;
                    }
                    ep
                }
                None => {
                    stats.episodes_rejected += 1;
                    continue;
                }
            };

            let content = redacted.to_content();
            let signal_score = redacted.compute_signal_score();
            let memory_id = format!(
                "{:016x}",
                u64::from_be_bytes(
                    blake3::hash(redacted.id.as_bytes()).as_bytes()[..8]
                        .try_into()
                        .unwrap()
                )
            );

            let memory = spectral_ingest::Memory {
                id: memory_id,
                key: redacted.id.clone(),
                content,
                wing: Some(self.activity_wing.clone()),
                hall: Some(redacted.source.clone()),
                signal_score,
                visibility: "private".into(),
                source: Some(redacted.bundle_id.clone()),
                device_id: None,
                confidence: 1.0,
                created_at: Some(redacted.started_at.to_rfc3339()),
                last_reinforced_at: None,
                episode_id: None,
                compaction_tier: None,
                declarative_density: None, // Activity episodes don't need density
                description: None,
                description_generated_at: None,
                content_hash: None, // Computed by store.write()
                source_brain_id: None,
                signature: None,
            };

            let _outcome = self
                .rt
                .block_on(self.memory_store.write(&memory, &[]))
                .map_err(|e| Error::Schema(e.to_string()))?;
            stats.episodes_inserted += 1;
        }

        Ok(stats)
    }

    /// Single-shot recognition. Given a context string, returns memories
    /// that pattern-match without requiring an explicit query.
    pub fn probe(
        &self,
        context: &str,
        opts: crate::activity::ProbeOpts,
    ) -> Result<Vec<crate::activity::RecognizedMemory>, Error> {
        if context.is_empty() {
            return Ok(Vec::new());
        }

        // Use recall as the retrieval backbone
        let recall_result = self.recall(context, Visibility::Private)?;

        let mut recognized: Vec<crate::activity::RecognizedMemory> = recall_result
            .memory_hits
            .into_iter()
            .filter(|hit| {
                if let Some(ref wing_filter) = opts.wing_filter {
                    hit.wing.as_deref() == Some(wing_filter.as_str())
                } else {
                    true
                }
            })
            .map(|hit| {
                let relevance =
                    (hit.signal_score * 0.4 + (hit.hits as f64).min(5.0) / 5.0 * 0.6).min(1.0);
                crate::activity::RecognizedMemory {
                    id: hit.id,
                    key: hit.key,
                    content: hit.content,
                    wing: hit.wing,
                    hall: hit.hall,
                    signal_score: hit.signal_score,
                    relevance,
                    hits: hit.hits,
                }
            })
            .filter(|r| r.relevance >= opts.min_relevance)
            .collect();

        recognized.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        recognized.truncate(opts.max_results);
        Ok(recognized)
    }

    /// Recognition over the recent activity window. Reads recent episodes,
    /// synthesizes them into a context string, then calls probe().
    pub fn probe_recent(
        &self,
        window: crate::activity::ProbeWindow,
        opts: crate::activity::ProbeOpts,
    ) -> Result<Vec<crate::activity::RecognizedMemory>, Error> {
        let since = match window {
            crate::activity::ProbeWindow::Duration(d) => (Utc::now() - d).to_rfc3339(),
            crate::activity::ProbeWindow::Since(dt) => dt.to_rfc3339(),
            crate::activity::ProbeWindow::Count(_) => {
                // For count, use a very old timestamp and let the limit handle it
                "2000-01-01T00:00:00Z".to_string()
            }
        };

        let limit = match window {
            crate::activity::ProbeWindow::Count(n) => n,
            _ => 100,
        };

        let recent_memories = self
            .rt
            .block_on(self.memory_store.list_wing_memories_since(
                &self.activity_wing,
                &since,
                limit,
            ))
            .map_err(|e| Error::Schema(e.to_string()))?;

        if recent_memories.is_empty() {
            return Ok(Vec::new());
        }

        // Synthesize context from recent activity
        let context: String = recent_memories
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join(" | ");

        self.probe(&context, opts)
    }

    // ── Activity retention ──────────────────────────────────────────

    /// Prune activity episodes older than the cutoff. Returns count pruned.
    pub fn prune_activity_older_than(&self, cutoff: DateTime<Utc>) -> Result<usize, Error> {
        self.ensure_writable("prune_activity_older_than")?;
        let before = cutoff.to_rfc3339();
        self.rt
            .block_on(
                self.memory_store
                    .delete_wing_memories_before(&self.activity_wing, &before),
            )
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Keep only the most recent `per_bundle` activity episodes per bundle_id.
    /// Returns count pruned.
    pub fn prune_activity_keep_recent(&self, per_bundle: usize) -> Result<usize, Error> {
        self.ensure_writable("prune_activity_keep_recent")?;
        self.rt
            .block_on(
                self.memory_store
                    .prune_wing_keeping_recent_per_source(&self.activity_wing, per_bundle),
            )
            .map_err(|e| Error::Schema(e.to_string()))
    }

    // ── AAAK (Always-Active Agent Knowledge) ────────────────────────

    /// Returns the agent's foundational facts as a token-budgeted context
    /// string suitable for system prompt injection.
    ///
    /// AAAK is the L1 "Curated Memory" layer from the TACT whitepaper.
    /// It selects the highest-signal facts from qualifying halls,
    /// formats them as a bulleted list, and truncates to the token budget.
    /// The result is deterministic given the same brain state and options.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use spectral_graph::brain::{Brain, BrainConfig, AaakOpts, EntityPolicy};
    /// # let brain: Brain = todo!();
    /// let result = brain.aaak(AaakOpts::default()).unwrap();
    /// println!("System context (~{} tokens):\n{}", result.estimated_tokens, result.formatted);
    /// ```
    /// List episodes, optionally filtered by wing.
    pub fn list_episodes(
        &self,
        wing: Option<&str>,
        limit: usize,
    ) -> Result<Vec<spectral_ingest::Episode>, Error> {
        self.rt
            .block_on(self.memory_store.list_episodes(wing, limit))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Get all memories belonging to an episode.
    pub fn list_memories_by_episode(
        &self,
        episode_id: &str,
    ) -> Result<Vec<spectral_ingest::Memory>, Error> {
        self.rt
            .block_on(self.memory_store.list_memories_by_episode(episode_id))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Annotate a memory with contextual who/where/why/how metadata.
    pub fn annotate(
        &self,
        memory_id: &str,
        input: spectral_ingest::AnnotationInput,
    ) -> Result<spectral_ingest::MemoryAnnotation, Error> {
        self.ensure_writable("annotate")?;
        let annotation = spectral_ingest::MemoryAnnotation {
            id: format!(
                "ann-{:016x}",
                u64::from_be_bytes(
                    blake3::hash(
                        format!(
                            "{memory_id}-{}",
                            Utc::now().timestamp_nanos_opt().unwrap_or(0)
                        )
                        .as_bytes()
                    )
                    .as_bytes()[..8]
                        .try_into()
                        .unwrap()
                )
            ),
            memory_id: memory_id.to_string(),
            description: input.description,
            who: input.who,
            why: input.why,
            where_: input.where_,
            when_: input.when_,
            how: input.how,
            created_at: Utc::now(),
        };
        self.rt
            .block_on(self.memory_store.write_annotation(&annotation))
            .map_err(|e| Error::Schema(e.to_string()))?;
        Ok(annotation)
    }

    /// List all annotations for a memory.
    pub fn list_annotations(
        &self,
        memory_id: &str,
    ) -> Result<Vec<spectral_ingest::MemoryAnnotation>, Error> {
        self.rt
            .block_on(self.memory_store.list_annotations(memory_id))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Update the compaction_tier on an existing memory. Used by rollup
    /// consumers (e.g., Permagent's Librarian) to track compaction state.
    pub fn set_compaction_tier(
        &self,
        memory_id: &str,
        tier: spectral_ingest::CompactionTier,
    ) -> Result<(), Error> {
        self.ensure_writable("set_compaction_tier")?;
        self.rt
            .block_on(self.memory_store.set_compaction_tier(memory_id, tier))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// List all memories sorted by signal_score descending, up to `limit`.
    pub fn list_all_memories(&self, limit: usize) -> Result<Vec<spectral_ingest::Memory>, Error> {
        self.rt
            .block_on(self.memory_store.list_memories_by_signal(0.0, limit))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// List memories in a specific wing with minimum signal score.
    pub fn list_wing_memories(
        &self,
        wing: &str,
        min_signal: f64,
    ) -> Result<Vec<spectral_ingest::Memory>, Error> {
        self.rt
            .block_on(self.memory_store.list_wing_memories(wing, min_signal))
            .map_err(|e| Error::Schema(e.to_string()))
    }

    /// Build the novelty-analysis context for a memory: the concatenated
    /// content of *other* memories in the same wing. Without this corpus the
    /// novelty dimension degenerates to 1.0 for every memory — the measured
    /// failure in docs/internal/SPECTROGRAM_AUDIT.md ("novelty 1.0 for
    /// 497/497"). The corpus is capped to bound ingest cost on large wings;
    /// `list_wing_memories` returns high-signal-first, so the cap keeps the
    /// most representative content.
    fn spectrogram_context(&self, wing: Option<&str>, exclude_id: &str) -> AnalysisContext {
        const MAX_CORPUS_CHARS: usize = 65_536;
        const MAX_CORPUS_MEMORIES: usize = 256;
        let Some(wing) = wing else {
            return AnalysisContext::default();
        };
        // wing_search is bounded (top-signal N) and LRU-cached, so ingest
        // cost stays flat on large wings — unlike list_wing_memories, which
        // would materialize the whole wing per remember().
        let mems = self
            .rt
            .block_on(
                self.memory_store
                    .wing_search(wing, &[], MAX_CORPUS_MEMORIES),
            )
            .unwrap_or_default();
        let mut corpus = String::new();
        for m in &mems {
            if m.id == exclude_id {
                continue;
            }
            if corpus.len() + m.content.len() + 1 > MAX_CORPUS_CHARS {
                break;
            }
            corpus.push_str(&m.content);
            corpus.push(' ');
        }
        AnalysisContext {
            wing_corpus: corpus,
        }
    }

    /// Audit a single memory's spectrogram with full introspection.
    pub fn audit_spectrogram(&self, memory_id: &str) -> Result<AuditReport, Error> {
        let mems = self
            .rt
            .block_on(self.memory_store.list_memories_by_signal(0.0, 10000))
            .map_err(|e| Error::Schema(e.to_string()))?;

        let mem = mems
            .iter()
            .find(|m| m.id == memory_id)
            .ok_or_else(|| Error::Schema(format!("memory not found: {memory_id}")))?;

        let context = self.spectrogram_context(mem.wing.as_deref(), &mem.id);
        let (fingerprint, introspection) = self
            .spectrogram_analyzer
            .analyze_with_introspection(mem, &context);

        Ok(AuditReport {
            memory_id: mem.id.clone(),
            memory_key: mem.key.clone(),
            wing: mem.wing.clone(),
            content_excerpt: mem.content.chars().take(500).collect(),
            fingerprint,
            introspection,
            signal_score: mem.signal_score,
            created_at: mem
                .created_at
                .as_deref()
                .and_then(crate::ranking::parse_created_at),
        })
    }

    pub fn aaak(&self, opts: AaakOpts) -> Result<AaakResult, Error> {
        let max_chars = (opts.max_tokens as f64 * opts.chars_per_token) as usize;
        let hall_set: std::collections::HashSet<&str> =
            opts.include_halls.iter().map(|s| s.as_str()).collect();

        // Fetch high-signal memories
        let memories = if let Some(ref wings) = opts.include_wings {
            let mut all = Vec::new();
            for wing in wings {
                let mut wing_mems = self
                    .rt
                    .block_on(
                        self.memory_store
                            .list_wing_memories(wing, opts.min_signal_score),
                    )
                    .map_err(|e| Error::Schema(e.to_string()))?;
                all.append(&mut wing_mems);
            }
            // Re-sort across wings by signal_score descending
            all.sort_by(|a, b| {
                b.signal_score
                    .partial_cmp(&a.signal_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            all
        } else {
            self.rt
                .block_on(
                    self.memory_store
                        .list_memories_by_signal(opts.min_signal_score, 1000),
                )
                .map_err(|e| Error::Schema(e.to_string()))?
        };

        // Filter by hall
        let candidates: Vec<_> = memories
            .iter()
            .filter(|m| {
                m.hall
                    .as_deref()
                    .map(|h| hall_set.contains(h))
                    .unwrap_or(false)
            })
            .collect();

        let mut lines = Vec::new();
        let mut total_chars = 0;
        let mut wings_seen = std::collections::HashSet::new();
        let mut included = 0;

        for mem in &candidates {
            let content = mem.content.split_whitespace().collect::<Vec<_>>().join(" ");
            let line = format!("- {content}\n");
            if total_chars + line.len() > max_chars && !lines.is_empty() {
                break;
            }
            total_chars += line.len();
            lines.push(line);
            included += 1;
            if let Some(ref w) = mem.wing {
                wings_seen.insert(w.clone());
            }
        }

        let formatted = lines.join("");
        let estimated_tokens = (formatted.len() as f64 / opts.chars_per_token).ceil() as usize;
        let excluded_count = candidates.len().saturating_sub(included);

        let mut wings_represented: Vec<String> = wings_seen.into_iter().collect();
        wings_represented.sort();

        Ok(AaakResult {
            formatted,
            estimated_tokens,
            fact_count: included,
            excluded_count,
            wings_represented,
        })
    }
}

fn visibility_to_str(v: Visibility) -> String {
    match v {
        Visibility::Private => "private",
        Visibility::Team => "team",
        Visibility::Org => "org",
        Visibility::Public => "public",
    }
    .to_string()
}

/// Conservative FTS stopword set: pure function words with negligible
/// content-homograph risk. Deliberately EXCLUDES ambiguous tokens that are
/// often content in personal memory (e.g. "it"/"IT", "us"/"US", "can",
/// "may", "will", "march", "in", "on", "at") — dropping those would lose real
/// matches. These only ever pollute the candidate pool (a turn matching only
/// "is" has nothing to do with the query).
const FTS_STOPWORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "am",
    "of", "to", "and", "or", "what", "which", "how", "did", "does", "do",
    "that", "this", "these", "those", "for", "with", "there", "here",
];

fn is_fts_stopword(w: &str) -> bool {
    FTS_STOPWORDS.contains(&w)
}

/// Whether to drop stopwords from FTS queries (`SPECTRAL_FTS_STOPWORDS=1`).
/// Off by default — measure on the real bench before defaulting (the porter /
/// co-retrieval discipline). The possessive fix below is unconditional (a
/// clear bug fix, no downside); stopword removal is a behavior change with
/// tradeoffs, so it is gated.
fn fts_stopwords_enabled() -> bool {
    std::env::var("SPECTRAL_FTS_STOPWORDS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Whether recall should append anticipatory (lift-associated) memories to its
/// results (`SPECTRAL_ANTICIPATORY_RECALL=1`). Off by default — it only helps
/// once real co-retrieval history exists, and it changes the result contract
/// (a few extras beyond the requested k), so it is opt-in per the same
/// measure-before-defaulting discipline as the stopword lever.
fn fts_anticipatory_enabled() -> bool {
    std::env::var("SPECTRAL_ANTICIPATORY_RECALL")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Project a stored [`Memory`] into a [`MemoryHit`] for anticipatory
/// augmentation. `hits = 0` marks it as not query-matched (surfaced by
/// association, not keyword overlap) so a consumer can distinguish it.
/// Derive a memory's stable id from its key (blake3 of the key, first 8 bytes as
/// hex) — the same derivation `remember`/`forget` use, so a `consolidation_edges`
/// key can be resolved to its row via `get_memory`.
fn key_to_id(key: &str) -> String {
    format!(
        "{:016x}",
        u64::from_be_bytes(blake3::hash(key.as_bytes()).as_bytes()[..8].try_into().unwrap())
    )
}

fn memory_to_hit(m: spectral_ingest::Memory) -> spectral_ingest::MemoryHit {
    spectral_ingest::MemoryHit {
        id: m.id,
        key: m.key,
        content: m.content,
        wing: m.wing,
        hall: m.hall,
        signal_score: m.signal_score,
        visibility: m.visibility,
        hits: 0,
        source: m.source,
        device_id: m.device_id,
        confidence: m.confidence,
        created_at: m.created_at,
        last_reinforced_at: m.last_reinforced_at,
        episode_id: m.episode_id,
        declarative_density: m.declarative_density,
        description: m.description,
        source_brain_id: m.source_brain_id,
        signature: m.signature,
    }
}

/// Sanitize a natural-language query into FTS search terms. Reads the
/// stopword flag; see [`fts_query_words_opts`]. Applies number-word bridging
/// when `SPECTRAL_NUMBER_NORMALIZE` is set.
pub(crate) fn fts_query_words(query: &str) -> Vec<String> {
    let mut words = fts_query_words_opts(query, fts_stopwords_enabled());
    if number_normalize_enabled() {
        expand_number_words(query, &mut words);
    }
    expand_aliases(&mut words, query_aliases());
    words
}

/// Consumer-curated query alias/synonym table, loaded once from the JSON file at
/// `SPECTRAL_QUERY_ALIASES` (`{"term": ["expansion", ...], ...}`); empty (a
/// no-op) if the var is unset or the file is unreadable. This is the deterministic
/// answer to the semantic-bridging gap that pure lexical matching cannot close
/// (`CEO`↔`chief executive`, `k8s`↔`kubernetes`): the near-duplicate literature
/// endorses exactly ONE synonym approach — a **controlled vocabulary** the
/// consumer owns, not a general thesaurus (which harms precision). Spectral
/// supplies the mechanism; Permagent supplies the (bounded, domain) table.
fn query_aliases() -> &'static std::collections::HashMap<String, Vec<String>> {
    static QUERY_ALIASES: std::sync::OnceLock<std::collections::HashMap<String, Vec<String>>> =
        std::sync::OnceLock::new();
    QUERY_ALIASES.get_or_init(|| {
        std::env::var("SPECTRAL_QUERY_ALIASES")
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    })
}

/// Expand query terms with consumer-curated aliases: for each term matching an
/// alias key (case-insensitive), append the alias's expansion terms, each
/// re-tokenized into individual FTS terms (so multi-word expansions like
/// `"chief executive"` become `chief`, `executive`). Additive recall expansion;
/// the re-ranker handles precision. No-op on an empty table.
fn expand_aliases(
    words: &mut Vec<String>,
    aliases: &std::collections::HashMap<String, Vec<String>>,
) {
    if aliases.is_empty() {
        return;
    }
    let mut additions: Vec<String> = Vec::new();
    for w in words.iter() {
        if let Some(expansions) = aliases.get(&w.to_lowercase()) {
            for exp in expansions {
                for t in exp.split(|c: char| !c.is_alphanumeric()) {
                    if t.len() > 1
                        && !words.iter().any(|x| x.eq_ignore_ascii_case(t))
                        && !additions.iter().any(|x| x.eq_ignore_ascii_case(t))
                    {
                        additions.push(t.to_string());
                    }
                }
            }
        }
    }
    words.extend(additions);
}

/// Bidirectional digit ↔ word pairs for conservative number bridging. Deliberately
/// a **closed, unambiguous** set — cardinals 2–12 only. `one`/`zero` are excluded
/// (article / rare) and words like `second`/`quarter`/`half` are excluded because
/// they are common non-numeric content (time unit, fraction), the same homograph
/// discipline as the stopword set. Number words are universal English, not a
/// domain vocabulary, so this is safe where a general synonym table would not be.
const NUMBER_PAIRS: &[(&str, &str)] = &[
    ("2", "two"),
    ("3", "three"),
    ("4", "four"),
    ("5", "five"),
    ("6", "six"),
    ("7", "seven"),
    ("8", "eight"),
    ("9", "nine"),
    ("10", "ten"),
    ("11", "eleven"),
    ("12", "twelve"),
];

/// Whether to bridge digit/number-word query terms (`SPECTRAL_NUMBER_NORMALIZE=1`).
/// Off by default — measure on the real bench before defaulting, per the
/// porter/stopword discipline. Directly targets LongMemEval's counting/number
/// category: `three dogs` and `3 dogs` should retrieve each other.
fn number_normalize_enabled() -> bool {
    std::env::var("SPECTRAL_NUMBER_NORMALIZE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Ensure both surface forms of any number in the query are present as MATCH
/// terms. Scans the **raw** query (not the length-filtered word list) because a
/// single digit like `3` is dropped by the `len > 1` filter before it can
/// bridge; both forms are then appended (bypassing that filter, since `3` is a
/// valid FTS token). Additive recall expansion; the re-ranker handles precision.
fn expand_number_words(raw_query: &str, words: &mut Vec<String>) {
    let ensure = |words: &mut Vec<String>, term: &str| {
        if !words.iter().any(|x| x.eq_ignore_ascii_case(term)) {
            words.push(term.to_string());
        }
    };
    for tok in raw_query.split(|c: char| !c.is_alphanumeric()) {
        if tok.is_empty() {
            continue;
        }
        let lt = tok.to_lowercase();
        for (digit, word) in NUMBER_PAIRS {
            if lt == *digit || lt == *word {
                ensure(words, digit);
                ensure(words, word);
            }
        }
    }
}

/// Sanitize a query into FTS terms.
///
/// **Splits on the characters FTS5/unicode61 treats as token separators**
/// (everything except alphanumerics and the intra-token `-` / `_` / apostrophe),
/// so the query is tokenized the same way the stored content was. The previous
/// version *deleted* those characters, which silently **merged** adjacent tokens
/// and dropped the answer from the candidate pool entirely: `alice@acme.io`
/// became `aliceacmeio` (matches nothing), `and/or` became `andor`, and — the
/// original symptom — the possessive `Marcus's` became `Marcuss`. Splitting
/// fixes the whole class; the possessive is then just a trailing one-char token.
/// A trailing possessive `'s` (ASCII or Unicode apostrophe) is still stripped so
/// `Marcus's` → `Marcus` matches the entity `Marcus`. Words shorter than two
/// characters are dropped.
///
/// When `drop_stopwords`, also removes conservative function words
/// ([`FTS_STOPWORDS`]) that only pollute the pool (a turn matching solely on
/// "is" is unrelated to the query). Guard: if that would empty the query, the
/// unfiltered words are kept so an all-function-word query still recalls.
pub(crate) fn fts_query_words_opts(query: &str, drop_stopwords: bool) -> Vec<String> {
    let words: Vec<String> = query
        .split(|c: char| {
            !(c.is_alphanumeric() || c == '_' || c == '-' || c == '\'' || c == '\u{2019}')
        })
        .map(|w| {
            // Strip a trailing possessive: "Marcus's" / "Marcus’s" -> "Marcus".
            let base = w
                .strip_suffix("'s")
                .or_else(|| w.strip_suffix("\u{2019}s"))
                .unwrap_or(w);
            // Trim apostrophes stranded at token edges by the split.
            base.trim_matches(|c: char| c == '\'' || c == '\u{2019}')
                .to_string()
        })
        .filter(|w| w.len() > 1)
        .collect();

    if !drop_stopwords {
        return words;
    }
    let filtered: Vec<String> = words
        .iter()
        .filter(|w| !is_fts_stopword(&w.to_lowercase()))
        .cloned()
        .collect();
    // Fallback: never let stopword removal empty a query.
    if filtered.is_empty() {
        words
    } else {
        filtered
    }
}

pub(crate) fn str_to_vis(s: &str) -> Visibility {
    match s {
        "team" => Visibility::Team,
        "org" => Visibility::Org,
        "public" => Visibility::Public,
        _ => Visibility::Private,
    }
}

/// Apply time-based decay to a signal score.
///
/// Uses `last_reinforced_at` if present, otherwise `created_at`.
/// Decay rate: 1% per week, maximum decay of 50% (old memories never fully fade).
/// This is applied to the in-memory representation only — the stored score is unchanged.
fn decayed_signal_score(
    raw_score: f64,
    created_at: &Option<String>,
    last_reinforced_at: &Option<String>,
    now: &chrono::DateTime<Utc>,
) -> f64 {
    let last_touch = last_reinforced_at
        .as_deref()
        .or(created_at.as_deref())
        .and_then(crate::ranking::parse_created_at);

    let last_touch = match last_touch {
        Some(t) => t,
        None => return raw_score, // No timestamp available, no decay
    };

    let days_since = (*now - last_touch).num_days().max(0) as f64;
    let decay = (days_since / 7.0) * 0.01;
    let decay_factor = (1.0 - decay).max(0.5);

    raw_score * decay_factor
}

/// Infer a single entity type from a predicate's allowed types.
/// Returns an error if there are 0 or 2+ allowed types.
fn infer_single_type(
    mention: &str,
    allowed_types: Option<&[String]>,
    predicate: &str,
) -> Result<String, Error> {
    match allowed_types {
        None => Err(Error::Ontology(format!(
            "predicate '{}' not found in ontology",
            predicate
        ))),
        Some([]) => Err(Error::Ontology(format!(
            "predicate '{}' has no valid types",
            predicate
        ))),
        Some([single]) => Ok(single.clone()),
        Some(types) => Err(Error::AmbiguousEntityType {
            mention: mention.to_string(),
            predicate: predicate.to_string(),
            allowed: types.to_vec(),
        }),
    }
}

/// Convert a SpectrogramRow to a SpectralFingerprint.
fn row_to_fingerprint(
    row: &spectral_ingest::SpectrogramRow,
) -> spectral_spectrogram::SpectralFingerprint {
    spectral_spectrogram::SpectralFingerprint {
        memory_id: row.memory_id.clone(),
        entity_density: row.entity_density,
        action_type: spectral_spectrogram::ActionType::from_str_lossy(&row.action_type),
        decision_polarity: row.decision_polarity,
        causal_depth: row.causal_depth,
        emotional_valence: row.emotional_valence,
        temporal_specificity: row.temporal_specificity,
        novelty: row.novelty,
        peak_dimensions: serde_json::from_str(&row.peak_dimensions).unwrap_or_default(),
        created_at: Utc::now(),
    }
}

#[cfg(test)]
mod fts_query_words_tests {
    use super::*;

    #[test]
    fn strips_possessive_ascii_and_unicode() {
        assert_eq!(fts_query_words_opts("Marcus's title", false), vec!["Marcus", "title"]);
        assert_eq!(fts_query_words_opts("Marcus\u{2019}s title", false), vec!["Marcus", "title"]);
    }

    #[test]
    fn splits_on_separators_instead_of_merging() {
        // Regression: separators were deleted, merging adjacent tokens so the
        // query never matched the content-tokenized form. They must SPLIT.
        assert_eq!(
            fts_query_words_opts("alice@acme.io", false),
            vec!["alice", "acme", "io"],
            "@ and . must split, not merge into 'aliceacmeio'"
        );
        assert_eq!(fts_query_words_opts("and/or clause", false), vec!["and", "or", "clause"]);
        assert_eq!(fts_query_words_opts("api.acme.dev cert", false), vec!["api", "acme", "dev", "cert"]);
        // Intra-token hyphen/underscore are preserved (FTS re-tokenizes them).
        assert_eq!(fts_query_words_opts("blue-green deploy", false), vec!["blue-green", "deploy"]);
        // Possessive still handled after the split change.
        assert_eq!(fts_query_words_opts("Sarah's role", false), vec!["Sarah", "role"]);
    }

    #[test]
    fn alias_expansion_bridges_and_tokenizes_multiword() {
        let mut aliases = std::collections::HashMap::new();
        aliases.insert("ceo".to_string(), vec!["chief executive officer".to_string()]);
        aliases.insert("k8s".to_string(), vec!["kubernetes".to_string()]);
        let mut w = vec!["ceo".to_string(), "budget".to_string()];
        expand_aliases(&mut w, &aliases);
        // Multi-word expansion is tokenized into individual FTS terms.
        for t in ["chief", "executive", "officer"] {
            assert!(w.iter().any(|x| x == t), "{t} should be added: {w:?}");
        }
        // Case-insensitive key match.
        let mut w2 = vec!["K8s".to_string()];
        expand_aliases(&mut w2, &aliases);
        assert!(w2.iter().any(|x| x == "kubernetes"), "K8s should bridge: {w2:?}");
        // Empty table is a no-op.
        let mut w3 = vec!["ceo".to_string()];
        expand_aliases(&mut w3, &std::collections::HashMap::new());
        assert_eq!(w3, vec!["ceo".to_string()]);
    }

    #[test]
    fn query_aliases_empty_by_default() {
        // With no SPECTRAL_QUERY_ALIASES file, the table is empty (no-op).
        assert!(query_aliases().is_empty());
    }

    #[test]
    fn number_words_bridge_both_directions_conservatively() {
        // digit -> word (single digit survives via the raw scan)
        let mut w = vec!["puppies".to_string()];
        expand_number_words("3 puppies", &mut w);
        assert!(w.iter().any(|x| x == "three"), "3 should bridge to three: {w:?}");
        // word -> digit
        let mut w2 = vec!["kids".to_string()];
        expand_number_words("five kids", &mut w2);
        assert!(w2.iter().any(|x| x == "5"), "five should bridge to 5: {w2:?}");
        // Excluded homographs must NOT bridge (ambiguous content words).
        let mut w3 = vec!["thing".to_string()];
        expand_number_words("one second", &mut w3);
        assert!(!w3.iter().any(|x| x == "1"), "'one' must not bridge (article): {w3:?}");
        assert!(!w3.iter().any(|x| x == "2"), "'second' must not bridge (time unit): {w3:?}");
    }

    #[test]
    fn drops_stopwords_only_when_enabled() {
        // Off: function words survive (legacy behavior).
        assert_eq!(
            fts_query_words_opts("What is the deploy status", false),
            vec!["What", "is", "the", "deploy", "status"]
        );
        // On: pure function words dropped, content kept.
        assert_eq!(
            fts_query_words_opts("What is the deploy status", true),
            vec!["deploy", "status"]
        );
    }

    #[test]
    fn stopword_removal_never_empties_query() {
        // An all-function-word query falls back to the unfiltered terms so it
        // still recalls something rather than returning nothing.
        let out = fts_query_words_opts("what is that", true);
        assert!(!out.is_empty(), "must not empty an all-stopword query");
    }

    #[test]
    fn does_not_drop_content_homographs() {
        // Ambiguous tokens that are often content are deliberately NOT stopwords.
        let out = fts_query_words_opts("the IT team can march in May", true);
        for kept in ["IT", "team", "can", "march", "in", "May"] {
            assert!(out.iter().any(|w| w == kept), "{kept} must be kept, got {out:?}");
        }
        assert!(!out.iter().any(|w| w == "the"), "'the' should be dropped");
    }
}

#[cfg(test)]
mod kuzu_schema_abort_repro {
    use super::*;
    use std::path::PathBuf;

    /// Reproducer for the kuzu schema-creation abort on Linux.
    ///
    /// A single `Brain::open()` aborts during schema creation on Linux
    /// (a C++ exception thrown by kuzu inside create_schema is not
    /// converted to a Rust Result by cxxbridge — std::terminate fires
    /// and the process aborts with SIGABRT).
    ///
    /// Marked `#[ignore]` so it does not run in normal CI. Invoke:
    ///
    /// ```bash
    /// cargo test -p spectral-graph \
    ///     -- --ignored --nocapture single_brain_open_aborts_on_linux
    /// ```
    ///
    /// Expected behavior:
    /// - macOS: test passes, process exits cleanly (exit code 0)
    /// - Ubuntu 24.04+: process aborts during Brain::open() with
    ///   SIGABRT (exit code 134). The test will not complete.
    ///
    /// See `Brain::open` doc comment for context and issue links.
    #[test]
    #[ignore = "diagnostic reproducer; runs only with --ignored"]
    fn single_brain_open_aborts_on_linux() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let data_dir: PathBuf = tempdir.path().join("brain");
        std::mem::forget(tempdir);

        let config = BrainConfig {
            data_dir,
            ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
            memory_db_path: None,
            llm_client: None,
            wing_rules: None,
            hall_rules: None,
            device_id: None,
            enable_spectrogram: false,
            entity_policy: EntityPolicy::Strict,
            sqlite_mmap_size: None,
            fts_tokenizer: None,
            read_only: false,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        };

        // On Linux this never returns successfully — abort fires
        // inside Brain::open during schema creation.
        let _brain = Brain::open(config).expect("brain open");
    }

    /// Federation topology probe + multi-brain extension of issue #153.
    ///
    /// Opens N (>= 3) independent `Brain` handles on N distinct `data_dir`s
    /// in a SINGLE process, keeps them all alive simultaneously, runs
    /// `recall_cascade` on each while its siblings are live, then drops them
    /// in a deliberately non-stack (non-LIFO) order.
    ///
    /// Why this exists — two questions, one experiment:
    ///  1. Federation v1 (read-time fan-out) wants a coordinator to open N
    ///     child brains in one process and query each. This test is the
    ///     empirical settler for "is in-process fan-out viable, or must the
    ///     coordinator shard brains across processes + local IPC?".
    ///  2. Issue #153 (kuzu Linux SIGABRT in create_schema): co-resident
    ///     `Database` instances probe whether the FFI abort is sensitive to
    ///     multiple live kuzu Databases / teardown order, beyond the
    ///     single-brain `single_brain_open_aborts_on_linux` reproducer.
    ///
    /// Expected behavior:
    /// - macOS: passes, clean exit. The Mac does NOT reproduce #153, so a
    ///   green run here is NOT meaningful evidence — it only confirms the
    ///   test logic compiles and that N brains can coexist on this platform.
    /// - Ubuntu (glibc 2.39+): if #153 is purely a single-open schema abort,
    ///   the FIRST `Brain::open` already aborts (SIGABRT, exit 134) and the
    ///   test never reaches the multi-brain phases. If open instead succeeds,
    ///   this verifies N co-resident recalls and a varied-order teardown —
    ///   which would itself refine the #153 diagnosis.
    ///
    /// MUST run on Linux to be meaningful. Wired into
    /// `.github/workflows/kuzu-abort-diagnostic.yml`. Running it needs the
    /// Permagent collaborator's Ubuntu box — the #153 hand-off that has been
    /// pending since the teardown-vs-schema diagnosis was corrected.
    ///
    /// ```bash
    /// cargo test -p spectral-graph \
    ///     -- --ignored --nocapture n_brains_coresident_recall
    /// ```
    #[test]
    #[ignore = "diagnostic reproducer; Linux-only, runs only with --ignored"]
    fn n_brains_coresident_recall_varied_drop_order() {
        const N: usize = 3;

        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path().to_path_buf();
        std::mem::forget(tempdir);

        let make_config = |i: usize| BrainConfig {
            data_dir: root.join(format!("brain-{i}")),
            ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
            memory_db_path: None,
            llm_client: None,
            wing_rules: None,
            hall_rules: None,
            device_id: None,
            enable_spectrogram: false,
            entity_policy: EntityPolicy::Strict,
            sqlite_mmap_size: None,
            fts_tokenizer: None,
            read_only: false,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        };

        // Phase 1 — open N brains on N distinct paths, all kept alive.
        // On Linux, if #153 is a single-open schema abort, the process
        // aborts HERE on i = 0 and never returns.
        let mut brains: Vec<Option<Brain>> = (0..N)
            .map(|i| Some(Brain::open(make_config(i)).expect("brain open")))
            .collect();

        // Distinct identities — confirms N truly independent handles with no
        // global/static DB collision (per the fan-out feasibility audit).
        let ids: std::collections::HashSet<String> = brains
            .iter()
            .map(|b| b.as_ref().unwrap().brain_id().to_string())
            .collect();
        assert_eq!(ids.len(), N, "expected N distinct brain identities");

        // Phase 2 — seed + recall on each brain WHILE all siblings are live.
        // Exercises the per-brain kuzu Connection::query path under
        // co-residence (the federation fan-out read pattern).
        for (i, slot) in brains.iter().enumerate() {
            let brain = slot.as_ref().unwrap();
            brain
                .remember(
                    &format!("fed-probe-{i}"),
                    &format!("federation fan-out probe memory for brain {i}"),
                    Visibility::Private,
                )
                .expect("remember");
            let pipeline_config = crate::cascade_layers::CascadePipelineConfig::default();
            let result = brain
                .recall_cascade(
                    "federation fan-out probe",
                    &spectral_cascade::RecognitionContext::empty(),
                    &pipeline_config,
                )
                .expect("recall_cascade");
            assert!(
                !result.merged_hits.is_empty(),
                "brain {i} should recall its own seeded memory while siblings are live"
            );
        }

        // Phase 3 — drop in a deliberately NON-LIFO order to probe
        // teardown-order sensitivity (the original, since-disproven, #153
        // teardown theory). Reaching the end on Linux without SIGABRT would
        // itself be a notable result.
        let drop_order = [1usize, 0, 2];
        assert_eq!(drop_order.len(), N, "drop order must cover all N brains");
        for &i in &drop_order {
            drop(brains[i].take());
        }
    }
}
