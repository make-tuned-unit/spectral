//! Graph layer for Spectral.
//!
//! Stores entities, triples, and provenance in an embedded Kuzu graph DB.
//! Canonicalizes mentions through a TOML ontology.

pub mod brain;
pub mod canonicalize;
pub mod kuzu_store;
pub mod ontology;
pub mod provenance;
pub mod schema;
