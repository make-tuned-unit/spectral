//! High-level Brain API for Spectral.
//!
//! A [`Brain`] is the primary interface for knowledge graph operations:
//! asserting facts, recalling related information, and ingesting documents.
//! It composes identity, ontology, canonicalization, and storage into a
//! cohesive API.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::Utc;

use spectral_core::entity_id::EntityId;
use spectral_core::identity::{BrainId, BrainIdentity};
use spectral_core::visibility::Visibility;

use crate::canonicalize::{Canonicalizer, MatchedMention};
use crate::kuzu_store::{Entity, KuzuStore, Neighborhood, Triple};
use crate::ontology::Ontology;
use crate::Error;

/// Configuration for opening a brain.
#[derive(Debug, Clone)]
pub struct BrainConfig {
    /// Directory for brain data (identity files, graph database).
    pub data_dir: PathBuf,
    /// Path to the ontology TOML file.
    pub ontology_path: PathBuf,
}

/// Result of a successful assertion.
#[derive(Debug)]
pub struct AssertResult {
    /// Whether the triple was written (always true on success).
    pub triple_written: bool,
    /// The resolved subject.
    pub subject: MatchedMention,
    /// The predicate name.
    pub predicate: String,
    /// The resolved object.
    pub object: MatchedMention,
}

/// Result of a recall query.
#[derive(Debug)]
pub struct RecallResult {
    /// Seed entities found from the query text.
    pub seed_entities: Vec<EntityId>,
    /// All triples in the neighborhood.
    pub triples: Vec<Triple>,
    /// Full neighborhood traversal result.
    pub neighborhood: Neighborhood,
}

/// Result of document ingestion.
#[derive(Debug)]
pub struct IngestResult {
    /// Blake3 hash of the document content.
    pub document_id: [u8; 32],
    /// Entities mentioned in the document.
    pub matched: Vec<MatchedMention>,
    /// Count of unresolved mentions.
    pub unresolved_count: usize,
}

/// A Spectral brain: identity + ontology + knowledge graph.
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
/// }).unwrap();
///
/// println!("Brain ID: {}", brain.brain_id());
/// ```
pub struct Brain {
    identity: BrainIdentity,
    ontology: Ontology,
    store: KuzuStore,
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
    /// opens the graph database, and validates the ontology.
    pub fn open(config: BrainConfig) -> Result<Self, Error> {
        std::fs::create_dir_all(&config.data_dir)?;

        let identity = BrainIdentity::load_or_create(&config.data_dir).map_err(Error::Core)?;
        let ontology = Ontology::load(&config.ontology_path)?;
        let store = KuzuStore::open(&config.data_dir.join("graph.kz"))?;

        Ok(Self {
            identity,
            ontology,
            store,
        })
    }

    /// Returns this brain's stable identifier.
    pub fn brain_id(&self) -> &BrainId {
        self.identity.brain_id()
    }

    /// Assert a fact: subject text, predicate name, object text.
    ///
    /// Both subject and object are canonicalized through the ontology.
    /// The predicate is validated against ontology domain/range constraints.
    ///
    /// # Errors
    ///
    /// - [`Error::UnresolvedMention`] if subject or object can't be resolved
    /// - [`Error::InvalidPredicate`] if the predicate doesn't fit the types
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

        // Validate predicate against ontology
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

        // Upsert both entities
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

        // Insert the triple
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

    /// Recall: find entities matching the query text, then return their
    /// 2-hop neighborhood.
    pub fn recall(&self, query: &str) -> Result<RecallResult, Error> {
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

        // Collect neighborhoods for all seeds, deduplicating
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

        // Upsert document node
        self.store
            .upsert_document(&document_id, source, visibility)?;

        // Canonicalize content
        let canonicalizer = Canonicalizer::new(&self.ontology);
        let canon_result = canonicalizer.canonicalize(content);

        // Upsert matched entities and create Mentions edges
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

    /// Direct access to the underlying store.
    pub fn store(&self) -> &KuzuStore {
        &self.store
    }

    /// Direct access to the ontology.
    pub fn ontology(&self) -> &Ontology {
        &self.ontology
    }
}
