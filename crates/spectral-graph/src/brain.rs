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

use crate::canonicalize::{Canonicalizer, MatchedMention};
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
/// }).unwrap();
/// println!("Brain ID: {}", brain.brain_id());
/// ```
pub struct Brain {
    identity: BrainIdentity,
    device_id: DeviceId,
    ontology: Ontology,
    store: KuzuStore,
    memory_store: Box<dyn MemoryStore>,
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

        // Filter memory hits by visibility
        let memory_hits: Vec<_> = tact
            .memories
            .iter()
            .filter(|m| str_to_vis(&m.visibility).allows(context_visibility))
            .cloned()
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
