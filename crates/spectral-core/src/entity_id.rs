//! Content-addressed entity ID derivation.
//!
//! An [`EntityId`] is a blake3 hash derived from an entity type and its
//! canonical representation, salted with a versioned prefix. The same
//! input always produces the same ID.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::Error;

/// Versioned salt prepended to every entity hash input.
const ENTITY_SALT: &[u8] = b"spectral-entity-v1:";

/// Separator between entity type and canonical form.
const SEPARATOR: u8 = b':';

/// A content-addressed entity identifier (blake3 hash).
///
/// # Examples
///
/// Same input always produces the same ID:
/// ```
/// use spectral_core::entity_id::{EntityId, entity_id};
///
/// let a = entity_id("note", "hello world");
/// let b = entity_id("note", "hello world");
/// assert_eq!(a, b);
/// ```
///
/// Different inputs produce different IDs:
/// ```
/// use spectral_core::entity_id::{EntityId, entity_id};
///
/// let a = entity_id("note", "hello");
/// let b = entity_id("note", "goodbye");
/// assert_ne!(a, b);
/// ```
///
/// Hex round-trip:
/// ```
/// use spectral_core::entity_id::{EntityId, entity_id};
///
/// let id = entity_id("task", "buy milk");
/// let hex = id.to_string();
/// let parsed: EntityId = hex.parse().unwrap();
/// assert_eq!(id, parsed);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityId([u8; 32]);

impl EntityId {
    /// Returns the raw 32-byte hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Derive a content-addressed entity ID from a type and canonical form.
///
/// The hash is computed as `blake3(SALT + entity_type + ":" + canonical)`.
pub fn entity_id(entity_type: &str, canonical: &str) -> EntityId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ENTITY_SALT);
    hasher.update(entity_type.as_bytes());
    hasher.update(&[SEPARATOR]);
    hasher.update(canonical.as_bytes());
    EntityId(*hasher.finalize().as_bytes())
}

impl fmt::Debug for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EntityId({})", self)
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

impl FromStr for EntityId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 64 {
            return Err(Error::InvalidEntityId(format!(
                "expected 64 hex chars, got {}",
                s.len()
            )));
        }
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| {
                Error::InvalidEntityId(format!("invalid hex at position {}", i * 2))
            })?;
        }
        Ok(EntityId(bytes))
    }
}

impl Serialize for EntityId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for EntityId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}
