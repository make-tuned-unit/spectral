//! Error types for spectral-graph.

use thiserror::Error;

/// Errors produced by the spectral-graph crate.
#[derive(Debug, Error)]
pub enum Error {
    /// Error from spectral-core.
    #[error(transparent)]
    Core(#[from] spectral_core::Error),

    /// Kuzu database error.
    #[error("kuzu: {0}")]
    Kuzu(#[from] kuzu::Error),

    /// I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// TOML deserialization error.
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),

    /// Ontology validation error.
    #[error("ontology: {0}")]
    Ontology(String),

    /// Schema or data conversion error.
    #[error("schema: {0}")]
    Schema(String),

    /// Subject or object could not be resolved to an ontology entity.
    #[error("unresolved mention: '{mention}'")]
    UnresolvedMention {
        /// The mention text that could not be resolved.
        mention: String,
        /// Canonical name of nearest match, if any was within 0.5 similarity.
        nearest: Option<String>,
    },

    /// Predicate is unknown or types don't match domain/range constraints.
    #[error("invalid predicate '{predicate}' for {subject_type} -> {object_type}")]
    InvalidPredicate {
        /// The predicate name.
        predicate: String,
        /// The subject entity type.
        subject_type: String,
        /// The object entity type.
        object_type: String,
    },

    /// No LLM client configured for an operation that requires one.
    #[error("no LLM client configured; ingest_text requires a client. Set BrainConfig.llm_client or use BrainBuilder::llm_client().")]
    MissingLlmClient,

    /// LLM call failed.
    #[error("llm: {0}")]
    Llm(String),
}
