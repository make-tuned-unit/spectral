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

pub use spectral_core::visibility::Visibility;
pub use spectral_graph::brain::{
    AssertResult, HybridRecallResult, IngestResult, RecallResult, RememberResult,
};
pub use spectral_graph::Error;
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

    /// Ingest a document: hash content, create document node, link mentions.
    pub fn ingest_document(
        &self,
        source: &str,
        content: &str,
        visibility: Visibility,
    ) -> Result<IngestResult, Error> {
        self.inner.ingest_document(source, content, visibility)
    }

    /// Direct access to the underlying graph store.
    pub fn store(&self) -> &spectral_graph::kuzu_store::KuzuStore {
        self.inner.store()
    }

    /// Direct access to the ontology.
    pub fn ontology(&self) -> &spectral_graph::ontology::Ontology {
        self.inner.ontology()
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
        };

        let inner = spectral_graph::brain::Brain::open(config)?;
        Ok(Brain { inner })
    }
}
