//! Spectral core primitives: identity, content-addressed IDs, visibility.
//!
//! This crate is intentionally minimal. It defines the types that every
//! other Spectral crate (and every brain that ever federates) must agree on.

pub mod device_id;
pub mod entity_id;
pub mod error;
pub mod identity;
pub mod visibility;

pub use error::Error;

/// Decode a single ASCII hex digit (`0-9`, `a-f`, `A-F`) to its 0..=15 value.
/// Returns `None` for any other byte — the canonical, panic-free primitive that
/// `DeviceId`/`EntityId` parsing builds on (rejects `+`, whitespace, non-ASCII).
#[inline]
pub(crate) fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
