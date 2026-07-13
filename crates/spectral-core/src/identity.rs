//! Brain identity via Ed25519 keypair.
//!
//! Each brain has a unique [`BrainIdentity`] consisting of an Ed25519
//! signing key, its corresponding verifying key, and a [`BrainId`]
//! derived as the blake3 hash of the public key bytes.

use std::fmt;
use std::path::Path;

use ed25519_dalek::SigningKey;
pub use ed25519_dalek::{Signature, VerifyingKey};

use crate::error::Error;

/// Unique identifier for a brain, derived as blake3 of the Ed25519 public key.
///
/// # Examples
///
/// ```
/// use spectral_core::identity::BrainIdentity;
///
/// let identity = BrainIdentity::generate();
/// let hex = identity.brain_id().to_string();
/// assert_eq!(hex.len(), 64);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BrainId([u8; 32]);

impl BrainId {
    /// Derive a BrainId from a verifying (public) key.
    fn from_verifying_key(vk: &VerifyingKey) -> Self {
        Self(*blake3::hash(vk.as_bytes()).as_bytes())
    }

    /// Construct a BrainId from raw bytes (e.g. loaded from storage).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw 32-byte hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for BrainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BrainId({})", self)
    }
}

impl fmt::Display for BrainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

/// A brain's Ed25519 identity: signing key, verifying key, and derived brain ID.
///
/// # Sign and verify round-trip
///
/// ```
/// use spectral_core::identity::{BrainIdentity, verify};
///
/// let identity = BrainIdentity::generate();
/// let msg = b"hello spectral";
/// let sig = identity.sign(msg);
/// assert!(verify(identity.brain_id(), identity.verifying_key(), msg, &sig));
/// ```
///
/// # Deterministic BrainId from persisted key
///
/// ```
/// use spectral_core::identity::BrainIdentity;
///
/// let dir = std::env::temp_dir().join(format!("spectral-doctest-{}", std::process::id()));
/// std::fs::create_dir_all(&dir).unwrap();
/// let a = BrainIdentity::load_or_create(&dir).unwrap();
/// let b = BrainIdentity::load_or_create(&dir).unwrap();
/// assert_eq!(a.brain_id(), b.brain_id());
/// std::fs::remove_dir_all(&dir).unwrap();
/// ```
///
/// # Verification fails with wrong message
///
/// ```
/// use spectral_core::identity::{BrainIdentity, verify};
///
/// let identity = BrainIdentity::generate();
/// let sig = identity.sign(b"original");
/// assert!(!verify(identity.brain_id(), identity.verifying_key(), b"tampered", &sig));
/// ```
pub struct BrainIdentity {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
    brain_id: BrainId,
}

impl BrainIdentity {
    /// Generate a new random brain identity.
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();
        let brain_id = BrainId::from_verifying_key(&verifying_key);
        Self {
            signing_key,
            verifying_key,
            brain_id,
        }
    }

    /// Load an existing identity from `dir/brain.key`, or create a new one.
    ///
    /// When creating, writes `brain.key` (mode 0600), `brain.pub`, and
    /// `brain.id` into the given directory.
    pub fn load_or_create(dir: &Path) -> Result<Self, Error> {
        let key_path = dir.join("brain.key");

        if key_path.exists() {
            let key_bytes = std::fs::read(&key_path)?;
            let key_array: [u8; 32] = key_bytes.try_into().map_err(|v: Vec<u8>| {
                Error::InvalidBrainId(format!("brain.key must be 32 bytes, got {}", v.len()))
            })?;
            let signing_key = SigningKey::from_bytes(&key_array);
            let verifying_key = signing_key.verifying_key();
            let brain_id = BrainId::from_verifying_key(&verifying_key);
            Ok(Self {
                signing_key,
                verifying_key,
                brain_id,
            })
        } else {
            let identity = Self::generate();

            std::fs::create_dir_all(dir)?;
            std::fs::write(&key_path, identity.signing_key.to_bytes())?;
            Self::set_key_permissions(&key_path)?;
            std::fs::write(dir.join("brain.pub"), identity.verifying_key.to_bytes())?;
            std::fs::write(dir.join("brain.id"), identity.brain_id.to_string())?;

            Ok(identity)
        }
    }

    #[cfg(unix)]
    fn set_key_permissions(path: &Path) -> Result<(), Error> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn set_key_permissions(_path: &Path) -> Result<(), Error> {
        Ok(())
    }

    /// Sign a message with this brain's signing key.
    pub fn sign(&self, msg: &[u8]) -> Signature {
        use ed25519_dalek::Signer;
        self.signing_key.sign(msg)
    }

    /// Sign a memory contribution: binds this brain's identity to the
    /// memory's content, creation time, and visibility. The signature
    /// authenticates *who* contributed the memory and that its content /
    /// visibility have not been altered — the trust anchor for a shared,
    /// multi-contributor project brain. The signed payload is produced by
    /// [`memory_signing_payload`] with `source_brain_id = self.brain_id()`.
    pub fn sign_memory(
        &self,
        content_hash: &str,
        created_at: &str,
        visibility: &str,
    ) -> Signature {
        let payload =
            memory_signing_payload(&self.brain_id, content_hash, created_at, visibility);
        self.sign(&payload)
    }

    /// Returns the brain's unique identifier.
    pub fn brain_id(&self) -> &BrainId {
        &self.brain_id
    }

    /// Returns the brain's public verifying key.
    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }
}

/// Verify a message signature against a brain's public key and identity.
///
/// Returns `true` only if the `brain_id` matches the `public_key` and the
/// signature is valid for the given message.
pub fn verify(brain_id: &BrainId, public_key: &VerifyingKey, msg: &[u8], sig: &Signature) -> bool {
    use ed25519_dalek::Verifier;
    let expected = BrainId::from_verifying_key(public_key);
    if expected != *brain_id {
        return false;
    }
    public_key.verify(msg, sig).is_ok()
}

/// Domain-separated version tag for the memory-signing payload. Bumping this
/// invalidates old signatures — change only on a deliberate scheme change.
pub const MEMORY_SIG_DOMAIN: &[u8] = b"spectral-memory-sig-v1";

/// Build the canonical byte payload signed for a memory contribution.
///
/// Layout (length-prefixed to prevent field-boundary ambiguity):
/// `DOMAIN ‖ source_brain_id(32) ‖ len(content_hash)‖content_hash ‖
///  len(created_at)‖created_at ‖ len(visibility)‖visibility`.
/// Every field is recoverable from a stored/returned memory at verify time,
/// so no signed field needs to be transmitted separately.
pub fn memory_signing_payload(
    source_brain_id: &BrainId,
    content_hash: &str,
    created_at: &str,
    visibility: &str,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        MEMORY_SIG_DOMAIN.len() + 32 + content_hash.len() + created_at.len() + visibility.len() + 12,
    );
    buf.extend_from_slice(MEMORY_SIG_DOMAIN);
    buf.extend_from_slice(source_brain_id.as_bytes());
    for field in [content_hash, created_at, visibility] {
        buf.extend_from_slice(&(field.len() as u32).to_le_bytes());
        buf.extend_from_slice(field.as_bytes());
    }
    buf
}

/// Verify a memory contribution's signature.
///
/// Returns `true` only if `public_key` matches `source_brain_id` (so the
/// claimed origin owns the key) **and** the signature is valid over the
/// canonical payload for the given content hash, creation time, and
/// visibility. Any tampering with the content (hash), timestamp, visibility,
/// or origin fails verification.
///
/// The caller supplies `public_key` — resolve it from `source_brain_id` via
/// the contributor grant set (a `BrainId` is `blake3(public_key)` and cannot
/// be inverted, so the key must be known out of band).
pub fn verify_memory_signature(
    source_brain_id: &BrainId,
    public_key: &VerifyingKey,
    content_hash: &str,
    created_at: &str,
    visibility: &str,
    sig: &Signature,
) -> bool {
    let payload = memory_signing_payload(source_brain_id, content_hash, created_at, visibility);
    verify(source_brain_id, public_key, &payload, sig)
}

#[cfg(test)]
mod memory_sig_tests {
    use super::*;

    #[test]
    fn sign_and_verify_memory_roundtrip() {
        let id = BrainIdentity::generate();
        let sig = id.sign_memory("abc123", "2026-07-10T12:00:00Z", "team");
        assert!(verify_memory_signature(
            id.brain_id(),
            id.verifying_key(),
            "abc123",
            "2026-07-10T12:00:00Z",
            "team",
            &sig,
        ));
    }

    #[test]
    fn tampering_any_signed_field_fails() {
        let id = BrainIdentity::generate();
        let sig = id.sign_memory("abc123", "2026-07-10T12:00:00Z", "team");
        // Wrong content hash (content was altered).
        assert!(!verify_memory_signature(
            id.brain_id(), id.verifying_key(),
            "TAMPERED", "2026-07-10T12:00:00Z", "team", &sig,
        ));
        // Wrong timestamp.
        assert!(!verify_memory_signature(
            id.brain_id(), id.verifying_key(),
            "abc123", "2026-07-11T00:00:00Z", "team", &sig,
        ));
        // Visibility escalation (team -> public) must not verify.
        assert!(!verify_memory_signature(
            id.brain_id(), id.verifying_key(),
            "abc123", "2026-07-10T12:00:00Z", "public", &sig,
        ));
    }

    #[test]
    fn foreign_key_cannot_impersonate_origin() {
        let alice = BrainIdentity::generate();
        let mallory = BrainIdentity::generate();
        let sig = alice.sign_memory("abc123", "2026-07-10T12:00:00Z", "team");
        // Mallory presents Alice's brain_id but her own key: pubkey doesn't
        // match the claimed origin -> reject.
        assert!(!verify_memory_signature(
            alice.brain_id(), mallory.verifying_key(),
            "abc123", "2026-07-10T12:00:00Z", "team", &sig,
        ));
        // Mallory re-signs under her own identity but claims Alice's id ->
        // brain_id/pubkey mismatch -> reject.
        let forged = mallory.sign_memory("abc123", "2026-07-10T12:00:00Z", "team");
        assert!(!verify_memory_signature(
            alice.brain_id(), mallory.verifying_key(),
            "abc123", "2026-07-10T12:00:00Z", "team", &forged,
        ));
    }

    #[test]
    fn payload_is_unambiguous_across_field_boundaries() {
        let id = BrainIdentity::generate();
        // Length-prefixing means ("ab","c") and ("a","bc") sign differently.
        let a = memory_signing_payload(id.brain_id(), "ab", "c", "team");
        let b = memory_signing_payload(id.brain_id(), "a", "bc", "team");
        assert_ne!(a, b, "field boundaries must be unambiguous");
    }
}
