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
}
