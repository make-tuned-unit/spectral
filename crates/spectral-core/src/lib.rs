//! Spectral core primitives: identity, content-addressed IDs, visibility.
//!
//! This crate is intentionally minimal. It defines the types that every
//! other Spectral crate (and every brain that ever federates) must agree on.

pub mod identity;
pub mod entity_id;
pub mod visibility;
pub mod error;

pub use error::Error;
