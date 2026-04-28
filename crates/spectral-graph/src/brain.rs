//! High-level Brain API for Spectral.
//!
//! A [`Brain`] is the primary interface: asserting facts in the knowledge
//! graph, remembering free-text observations via TACT ingestion, and
//! recalling relevant context from both stores.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::Utc;

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
/// }).unwrap();
/// println!("Brain ID: {}", brain.brain_id());
/// ```
pub struct Brain {
    identity: BrainIdentity,
    device_id: DeviceId,
    ontology: Ontology,
    store: KuzuStore,
    memory_store: Box<dyn MemoryStore>,
    llm_client: Option<Box<dyn LlmClient>>,
    enable_spectrogram: bool,
    spectrogram_analyzer: SpectrogramAnalyzer,
    tact_config: TactConfig,
    ingest_config: spectral_ingest::ingest::IngestConfig,
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
        let ontology = Ontology::load(&config.ontology_path)?;
        let store = KuzuStore::open(&config.data_dir.join("graph.kz"))?;

        let memory_db_path = config
            .memory_db_path
            .unwrap_or_else(|| config.data_dir.join("memory.db"));
        let memory_store: Box<dyn MemoryStore> =
            Box::new(SqliteStore::open(&memory_db_path).map_err(|e| Error::Schema(e.to_string()))?);

        // Resolve wing/hall rules — shared between ingest and TACT retrieval.
        let wing_rules = config
            .wing_rules
            .unwrap_or_else(spectral_ingest::default_wing_rule_strings);
        let hall_rules = config
            .hall_rules
            .unwrap_or_else(spectral_ingest::default_hall_rule_strings);

        let tact_config = TactConfig {
            wing_rules: wing_rules.clone(),
            hall_rules: hall_rules.clone(),
            ..TactConfig::default()
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
            store,
            memory_store,
            llm_client: config.llm_client,
            enable_spectrogram: config.enable_spectrogram,
            spectrogram_analyzer: SpectrogramAnalyzer::default(),
            tact_config,
            ingest_config,
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
    /// Both subject and object are canonicalized through the ontology.
    /// The predicate is validated against ontology domain/range constraints.
    pub fn assert(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: f64,
        visibility: Visibility,
    ) -> Result<AssertResult, Error> {
        let canonicalizer = Canonicalizer::new(&self.ontology);

        let subject_match = canonicalizer.resolve_one(subject).ok_or_else(|| {
            let nearest = canonicalizer.find_nearest(subject).map(|n| n.canonical);
            Error::UnresolvedMention {
                mention: subject.to_string(),
                nearest,
            }
        })?;

        let object_match = canonicalizer.resolve_one(object).ok_or_else(|| {
            let nearest = canonicalizer.find_nearest(object).map(|n| n.canonical);
            Error::UnresolvedMention {
                mention: object.to_string(),
                nearest,
            }
        })?;

        self.ontology
            .validate_triple(
                predicate,
                &subject_match.entity_type,
                &object_match.entity_type,
            )
            .map_err(|_| Error::InvalidPredicate {
                predicate: predicate.to_string(),
                subject_type: subject_match.entity_type.clone(),
                object_type: object_match.entity_type.clone(),
            })?;

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
            subject: subject_match,
            predicate: predicate.to_string(),
            object: object_match,
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
