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
use crate::kuzu_store::{Entity, KuzuStore, Neighborhood, Triple};
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
    /// Number of FTS candidates to retrieve. Default 40.
    pub k: usize,
    /// Blend signal_score into FTS ranking. Default true.
    pub apply_signal_score_weighting: bool,
    /// Apply exponential recency decay. Default true.
    pub apply_recency_weighting: bool,
    /// Half-life for recency decay in days. Default 365.0.
    pub recency_half_life_days: f64,
    /// Boost top candidate within entity/wing clusters. Default true.
    pub apply_entity_resolution: bool,
    /// Collapse `[Memory context]` reference duplicates. Default true.
    pub apply_context_dedup: bool,
}

impl Default for RecallTopKConfig {
    fn default() -> Self {
        Self {
            k: 40,
            apply_signal_score_weighting: true,
            apply_recency_weighting: true,
            recency_half_life_days: 365.0,
            apply_entity_resolution: true,
            apply_context_dedup: true,
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
    store: KuzuStore,
    memory_store: Box<dyn MemoryStore>,
    llm_client: Option<Box<dyn LlmClient>>,
    entity_policy: EntityPolicy,
    enable_spectrogram: bool,
    spectrogram_analyzer: SpectrogramAnalyzer,
    tact_config: TactConfig,
    ingest_config: spectral_ingest::ingest::IngestConfig,
    activity_wing: String,
    redaction_policy: Box<dyn crate::activity::RedactionPolicy>,
    rt: tokio::runtime::Runtime,
}

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
    pub fn open(config: BrainConfig) -> Result<Self, Error> {
        std::fs::create_dir_all(&config.data_dir)?;

        let identity = BrainIdentity::load_or_create(&config.data_dir).map_err(Error::Core)?;
        let ontology_path = config.ontology_path.clone();
        let ontology = Ontology::load(&config.ontology_path)?;
        let store = KuzuStore::open(&config.data_dir.join("graph.kz"))?;

        let memory_db_path = config
            .memory_db_path
            .unwrap_or_else(|| config.data_dir.join("memory.db"));
        let sqlite_config = spectral_ingest::sqlite_store::SqliteStoreConfig {
            mmap_size: config.sqlite_mmap_size,
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
            rt,
        })
    }

    /// Returns this brain's stable identifier.
    pub fn brain_id(&self) -> &BrainId {
        self.identity.brain_id()
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
        })?;

        self.store.upsert_entity(&Entity {
            id: object_match.entity_id,
            entity_type: object_match.entity_type.clone(),
            canonical: object_match.canonical.clone(),
            visibility,
            created_at: now,
            updated_at: now,
            weight: 1.0,
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

        // Compute and store spectrogram if enabled
        if self.enable_spectrogram {
            let context = AnalysisContext::default();
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
    pub fn recall(
        &self,
        query: &str,
        context_visibility: Visibility,
    ) -> Result<HybridRecallResult, Error> {
        let tact = self
            .rt
            .block_on(spectral_tact::retrieve(
                query,
                &self.tact_config,
                self.memory_store.as_ref(),
            ))
            .map_err(|e| Error::Schema(e.to_string()))?;

        // Filter by visibility, then apply time-based decay to signal scores
        let now = Utc::now();
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
    /// Equivalent to `recall(query, Visibility::Private)`.
    pub fn recall_local(&self, query: &str) -> Result<HybridRecallResult, Error> {
        self.recall(query, Visibility::Private)
    }

    /// Direct FTS search bypassing TACT pipeline. Used by cascade and topk_fts
    /// to avoid TACT's max_results=5 cap.
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
    /// Retrieves `config.k` candidates via full-text search, then applies
    /// configurable re-ranking (signal score weighting, recency decay,
    /// entity clustering boost, context chain dedup). Returns results
    /// sorted by final blended score.
    pub fn recall_topk_fts(
        &self,
        query: &str,
        config: &RecallTopKConfig,
        visibility: Visibility,
    ) -> Result<Vec<spectral_ingest::MemoryHit>, Error> {
        // Sanitize: strip FTS5 special characters and short words
        let words: Vec<String> = query
            .split_whitespace()
            .filter(|w| w.len() > 1)
            .map(|w| {
                w.chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                    .collect::<String>()
            })
            .filter(|w| w.len() > 1)
            .collect();

        if words.is_empty() {
            return Ok(Vec::new());
        }

        let mut candidates = self
            .rt
            .block_on(self.memory_store.fts_search(&words, config.k))
            .map_err(|e| Error::Schema(e.to_string()))?;

        // Filter by visibility
        candidates.retain(|m| str_to_vis(&m.visibility).allows(visibility));

        // Apply re-ranking signals in order
        if config.apply_signal_score_weighting {
            crate::ranking::apply_signal_score_weight(&mut candidates, 0.3);
        }

        // Tuned 2026-05-06: 90d → 365d softens recency demotion of older-but-still-correct
        // memories (multi-session synthesis was regressing under aggressive recency)
        if config.apply_recency_weighting {
            crate::ranking::apply_recency_weight(
                &mut candidates,
                config.recency_half_life_days,
                Utc::now(),
            );
        }

        // Tuned 2026-05-06: 0.15 → 0.05
        // Entity grouping was pulling cross-session noise into top results
        if config.apply_entity_resolution {
            crate::ranking::boost_entity_clusters(&mut candidates, 0.05);
        }

        if config.apply_context_dedup {
            candidates = crate::ranking::dedup_context_chains(candidates);
        }

        Ok(candidates)
    }

    /// Run the integrated cascade pipeline: FTS K=40 → ambient boost →
    /// signal/recency re-ranking → episode diversity → dedup.
    ///
    /// Single retrieval path using all Spectral subsystems. No redundant FTS,
    /// no AAAK contamination, no episode truncation.
    pub fn recall_cascade(
        &self,
        query: &str,
        context: &spectral_cascade::RecognitionContext,
        config: &spectral_cascade::orchestrator::CascadeConfig,
    ) -> Result<spectral_cascade::result::CascadeResult, Error> {
        let pipeline_config = crate::cascade_layers::CascadePipelineConfig {
            k: config.total_budget.clamp(20, 40),
            ..Default::default()
        };

        let hits =
            crate::cascade_layers::run_cascade_pipeline(self, query, context, &pipeline_config)?;

        // Wrap in CascadeResult for backwards compatibility
        let tokens_used = hits.iter().map(|h| h.content.len() / 4 + 5).sum();
        let max_confidence = hits
            .first()
            .map(|h| h.signal_score.min(0.85))
            .unwrap_or(0.0);

        Ok(spectral_cascade::result::CascadeResult {
            layer_outcomes: Vec::new(), // Pipeline doesn't use layer abstraction
            merged_hits: hits,
            total_tokens_used: tokens_used,
            stopped_at: None,
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
                },
            });
        }

        let mut all_entity_ids = HashSet::new();
        let mut all_entities = Vec::new();
        let mut all_triples = Vec::new();
        let mut seen_edges: HashSet<(EntityId, EntityId, String)> = HashSet::new();

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
        let triples_clone = all_triples.clone();

        Ok(RecallResult {
            seed_entities,
            triples: triples_clone,
            neighborhood: Neighborhood {
                entities: all_entities,
                triples: all_triples,
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
    pub fn store(&self) -> &KuzuStore {
        &self.store
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
    pub fn recall_cross_wing(
        &self,
        seed_query: &str,
        visibility: Visibility,
        max_results: usize,
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
                };
                self.spectrogram_analyzer
                    .analyze(&mem, &AnalysisContext::default())
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
        let tolerances = spectral_spectrogram::matching::MatchTolerances::default();
        let resonant = spectral_spectrogram::matching::find_resonant(
            &seed_fp,
            &other_wing_fps,
            max_results,
            &tolerances,
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
                let context = AnalysisContext::default();
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

    /// Backfill time_delta_bucket on existing constellation fingerprints.
    /// Recomputes bucket from anchor/target memory created_at timestamps and
    /// updates the fingerprint hash to match. Returns count of updated rows.
    pub fn backfill_fingerprint_time_buckets(&self) -> Result<usize, Error> {
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

    /// Extract triples from natural-language text, validate against ontology,
    /// assert valid triples, and store the original text as a memory.
    ///
    /// Requires a configured `LlmClient`.
    pub fn ingest_text(&self, text: &str, opts: IngestTextOpts) -> Result<IngestTextResult, Error> {
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
            };

            self.rt
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

        let context = AnalysisContext::default();
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
            created_at: mem.created_at.as_deref().and_then(|s| {
                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                    .ok()
                    .map(|dt| dt.and_utc())
            }),
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

fn str_to_vis(s: &str) -> Visibility {
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
        .and_then(|s| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|dt| dt.and_utc())
        });

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
