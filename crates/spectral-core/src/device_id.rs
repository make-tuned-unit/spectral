//! Content-addressed device identifier.
//!
//! A [`DeviceId`] is a blake3 hash derived from a stable device descriptor
//! string (hostname, hardware UUID, MAC address — caller's choice).
//! Same descriptor always produces the same ID.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::Error;

/// Versioned salt for device ID hashing.
const DEVICE_SALT: &[u8] = b"spectral-device-v1:";

/// A content-addressed device identifier (blake3 hash).
///
/// # Same descriptor produces the same ID
///
/// ```
/// use spectral_core::device_id::DeviceId;
///
/// let a = DeviceId::from_descriptor("laptop-abc");
/// let b = DeviceId::from_descriptor("laptop-abc");
/// assert_eq!(a, b);
/// ```
///
/// # Different descriptors produce different IDs
///
/// ```
/// use spectral_core::device_id::DeviceId;
///
/// let a = DeviceId::from_descriptor("laptop-abc");
/// let b = DeviceId::from_descriptor("desktop-xyz");
/// assert_ne!(a, b);
/// ```
///
/// # Hex round-trip
///
/// ```
/// use spectral_core::device_id::DeviceId;
///
/// let id = DeviceId::from_descriptor("my-host");
/// let hex = id.to_string();
/// let parsed: DeviceId = hex.parse().unwrap();
/// assert_eq!(id, parsed);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceId([u8; 32]);

impl DeviceId {
    /// Construct a DeviceId from raw bytes (e.g. loaded from storage).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw 32-byte hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Generate a content-addressed DeviceId from a stable device descriptor.
    ///
    /// Use the system's hardware UUID, MAC address, or hostname — whichever
    /// is most stable for the deployment context. This is a pure function;
    /// no platform-specific detection is performed.
    pub fn from_descriptor(descriptor: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(DEVICE_SALT);
        hasher.update(descriptor.as_bytes());
        Self(*hasher.finalize().as_bytes())
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({})", self)
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

impl FromStr for DeviceId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 64 {
            return Err(Error::InvalidDeviceId(format!(
                "expected 64 hex chars, got {}",
                s.len()
            )));
        }
        // Operate on raw bytes: `s` is len 64 but may not be 64 ASCII chars, so
        // slicing `&s[..]` at 2-byte offsets would panic on a char boundary.
        // Requiring canonical ASCII hex digits also rejects the `+`/whitespace
        // that `u8::from_str_radix` would otherwise accept (breaks round-trip).
        let hex = s.as_bytes();
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            let hi = crate::hex_val(hex[i * 2]).ok_or_else(|| {
                Error::InvalidDeviceId(format!("invalid hex at position {}", i * 2))
            })?;
            let lo = crate::hex_val(hex[i * 2 + 1]).ok_or_else(|| {
                Error::InvalidDeviceId(format!("invalid hex at position {}", i * 2 + 1))
            })?;
            *byte = (hi << 4) | lo;
        }
        Ok(DeviceId(bytes))
    }
}

impl Serialize for DeviceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod parse_tests {
    use super::DeviceId;
    use std::str::FromStr;

    #[test]
    fn rejects_non_ascii_64_bytes_without_panic() {
        // 64 bytes but not 64 chars: a 3-byte '€' plus 61 ASCII. Must Err, not panic.
        let s = format!("{}{}", "\u{20AC}", "a".repeat(61));
        assert_eq!(s.len(), 64);
        assert!(DeviceId::from_str(&s).is_err());
    }

    #[test]
    fn rejects_leading_plus_non_canonical() {
        // `u8::from_str_radix` used to accept "+f"; canonical hex must reject it.
        let s = format!("+f{}", "0".repeat(62));
        assert_eq!(s.len(), 64);
        assert!(DeviceId::from_str(&s).is_err());
    }

    #[test]
    fn round_trips_canonical_hex() {
        let s = "ab".repeat(32);
        let id = DeviceId::from_str(&s).expect("valid hex");
        assert_eq!(id.to_string(), s);
    }
}
