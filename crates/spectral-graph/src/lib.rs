//! Graph layer for Spectral.
//!
//! Stores entities, triples, and provenance in an embedded SQLite graph store.
//! Canonicalizes mentions through a TOML ontology.

pub mod activity;
pub mod brain;
pub mod canonicalize;
pub mod cascade_layers;
pub mod error;
pub mod extract;
pub mod federation;
pub mod graph_store;
pub mod ontology;
pub mod provenance;
pub mod ranking;

pub use error::Error;
pub use spectral_cascade::RecognitionContext;
