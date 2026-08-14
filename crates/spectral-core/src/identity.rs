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
        // Entropy is taken straight from the OS rather than through an RNG
        // trait. `SigningKey::generate` is exactly `fill_bytes` into 32 bytes
        // followed by `from_bytes`, so this is equivalent — and it does not
        // depend on which `rand_core` major version dalek happens to speak,
        // which is what broke on the 2.x -> 3.0 bump.
        //
        // `expect` is deliberate: if the kernel cannot supply entropy we must
        // not continue and mint a brain identity from a degraded source.
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret)
            .expect("OS entropy unavailable; refusing to generate a brain identity");
        let signing_key = SigningKey::from_bytes(&secret);
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

    /// Sign a memory contribution: binds this brain's identity to the memory's
    /// key, content, creation time, and visibility. The signature
    /// authenticates *who* contributed the memory, that its content /
    /// visibility have not been altered, and *which key it was filed under* —
    /// the trust anchor for a shared, multi-contributor project brain. The
    /// signed payload is produced by [`memory_signing_payload_v2`] with
    /// `source_brain_id = self.brain_id()`.
    pub fn sign_memory(
        &self,
        key: &str,
        content_hash: &str,
        created_at: &str,
        visibility: &str,
    ) -> Signature {
        let payload =
            memory_signing_payload_v2(&self.brain_id, key, content_hash, created_at, visibility);
        self.sign(&payload)
    }

    /// Sign a federation object: an attestation over its content address.
    /// The object hash already covers author, key, content, timestamp,
    /// visibility, and supersedes, so signing it binds every source field at
    /// once — see [`federation_object_signing_payload`].
    pub fn sign_federation_object(&self, object_hash: &str) -> Signature {
        self.sign(&federation_object_signing_payload(object_hash))
    }

    /// Sign a retraction. The wing is part of the payload because a tombstone
    /// is wing-scoped: a retraction authorised for one wing must not be
    /// replayable against another.
    pub fn sign_federation_tombstone(
        &self,
        wing_id: &str,
        target_hash: &str,
        ts: &str,
    ) -> Signature {
        self.sign(&federation_tombstone_signing_payload(
            wing_id,
            target_hash,
            ts,
        ))
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

/// Domain-separated version tag for the **legacy** memory-signing payload,
/// which did not bind the memory's key. Retained only so signatures written
/// before the v2 scheme still verify; never used for new signatures.
pub const MEMORY_SIG_DOMAIN: &[u8] = b"spectral-memory-sig-v1";

/// Current memory-signing domain. v2 adds the memory **key** to the payload.
///
/// Without the key, a signature authenticates only *what* was said, never
/// *what question it answers*: a peer could re-serve a genuinely signed
/// memory under any key — as the answer to a different question — and
/// verification would still succeed.
pub const MEMORY_SIG_DOMAIN_V2: &[u8] = b"spectral-memory-sig-v2";

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
        MEMORY_SIG_DOMAIN.len()
            + 32
            + content_hash.len()
            + created_at.len()
            + visibility.len()
            + 12,
    );
    buf.extend_from_slice(MEMORY_SIG_DOMAIN);
    buf.extend_from_slice(source_brain_id.as_bytes());
    for field in [content_hash, created_at, visibility] {
        buf.extend_from_slice(&(field.len() as u32).to_le_bytes());
        buf.extend_from_slice(field.as_bytes());
    }
    buf
}

/// Build the canonical v2 payload, which additionally binds the memory `key`:
/// `DOMAIN_V2 ‖ source_brain_id(32) ‖ len(key)‖key ‖ len(content_hash)‖…`.
///
/// The key is length-prefixed like every other field, so a key/content
/// boundary cannot be shifted to forge an equivalent payload.
pub fn memory_signing_payload_v2(
    source_brain_id: &BrainId,
    key: &str,
    content_hash: &str,
    created_at: &str,
    visibility: &str,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        MEMORY_SIG_DOMAIN_V2.len()
            + 32
            + key.len()
            + content_hash.len()
            + created_at.len()
            + visibility.len()
            + 16,
    );
    buf.extend_from_slice(MEMORY_SIG_DOMAIN_V2);
    buf.extend_from_slice(source_brain_id.as_bytes());
    for field in [key, content_hash, created_at, visibility] {
        buf.extend_from_slice(&(field.len() as u32).to_le_bytes());
        buf.extend_from_slice(field.as_bytes());
    }
    buf
}

/// Domain tag for a federation object attestation.
pub const FEDERATION_OBJ_SIG_DOMAIN: &[u8] = b"spectral-federation-obj-sig-v1";
/// Domain tag for a federation retraction (tombstone) attestation.
pub const FEDERATION_TOMBSTONE_SIG_DOMAIN: &[u8] = b"spectral-federation-tombstone-sig-v1";

fn length_prefixed(domain: &[u8], fields: &[&str]) -> Vec<u8> {
    let mut buf =
        Vec::with_capacity(domain.len() + fields.iter().map(|f| f.len() + 4).sum::<usize>());
    buf.extend_from_slice(domain);
    for field in fields {
        buf.extend_from_slice(&(field.len() as u32).to_le_bytes());
        buf.extend_from_slice(field.as_bytes());
    }
    buf
}

/// Canonical payload for a federation object attestation: `DOMAIN ‖
/// len(object_hash)‖object_hash`.
///
/// Signing the content address rather than the individual fields is what makes
/// authorship unforgeable here: `object_hash` is computed over the author id
/// together with every source field, so re-claiming another brain's authorship
/// changes the hash and invalidates the signature.
pub fn federation_object_signing_payload(object_hash: &str) -> Vec<u8> {
    length_prefixed(FEDERATION_OBJ_SIG_DOMAIN, &[object_hash])
}

/// Canonical payload for a retraction: `DOMAIN ‖ wing ‖ target ‖ ts`, each
/// length-prefixed. Wing-scoped so a retraction cannot be replayed into a
/// different wing.
pub fn federation_tombstone_signing_payload(wing_id: &str, target_hash: &str, ts: &str) -> Vec<u8> {
    length_prefixed(FEDERATION_TOMBSTONE_SIG_DOMAIN, &[wing_id, target_hash, ts])
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
/// Accepts the current v2 scheme (key-bound) and, for rows signed before v2
/// existed, falls back to the legacy v1 payload. The fallback is deliberately
/// *narrow*: it is tried only after v2 fails, so a v2-signed memory can never
/// be downgraded by stripping the key from the verification request.
pub fn verify_memory_signature(
    source_brain_id: &BrainId,
    public_key: &VerifyingKey,
    key: &str,
    content_hash: &str,
    created_at: &str,
    visibility: &str,
    sig: &Signature,
) -> bool {
    let v2 = memory_signing_payload_v2(source_brain_id, key, content_hash, created_at, visibility);
    if verify(source_brain_id, public_key, &v2, sig) {
        return true;
    }
    let v1 = memory_signing_payload(source_brain_id, content_hash, created_at, visibility);
    verify(source_brain_id, public_key, &v1, sig)
}

#[cfg(test)]
mod memory_sig_tests {
    use super::*;

    #[test]
    fn sign_and_verify_memory_roundtrip() {
        let id = BrainIdentity::generate();
        let sig = id.sign_memory("mem-key", "abc123", "2026-07-10T12:00:00Z", "team");
        assert!(verify_memory_signature(
            id.brain_id(),
            id.verifying_key(),
            "mem-key",
            "abc123",
            "2026-07-10T12:00:00Z",
            "team",
            &sig,
        ));
    }

    #[test]
    fn tampering_any_signed_field_fails() {
        let id = BrainIdentity::generate();
        let sig = id.sign_memory("mem-key", "abc123", "2026-07-10T12:00:00Z", "team");
        // Wrong content hash (content was altered).
        assert!(!verify_memory_signature(
            id.brain_id(),
            id.verifying_key(),
            "mem-key",
            "TAMPERED",
            "2026-07-10T12:00:00Z",
            "team",
            &sig,
        ));
        // Wrong timestamp.
        assert!(!verify_memory_signature(
            id.brain_id(),
            id.verifying_key(),
            "mem-key",
            "abc123",
            "2026-07-11T00:00:00Z",
            "team",
            &sig,
        ));
        // Visibility escalation (team -> public) must not verify.
        assert!(!verify_memory_signature(
            id.brain_id(),
            id.verifying_key(),
            "mem-key",
            "abc123",
            "2026-07-10T12:00:00Z",
            "public",
            &sig,
        ));
    }

    /// R-11: the signature must bind the memory KEY, not only its content.
    /// Otherwise a genuinely signed memory can be re-served under any key —
    /// as the answer to a question it never answered — and still verify.
    #[test]
    fn resigning_under_a_different_key_fails() {
        let id = BrainIdentity::generate();
        let sig = id.sign_memory("q-refund-policy", "abc123", "2026-07-10T12:00:00Z", "team");
        assert!(verify_memory_signature(
            id.brain_id(),
            id.verifying_key(),
            "q-refund-policy",
            "abc123",
            "2026-07-10T12:00:00Z",
            "team",
            &sig,
        ));
        // Same brain, same content, same everything — filed under a different
        // key. This must NOT verify.
        assert!(
            !verify_memory_signature(
                id.brain_id(),
                id.verifying_key(),
                "q-security-policy",
                "abc123",
                "2026-07-10T12:00:00Z",
                "team",
                &sig,
            ),
            "a signed memory verified under a key it was never signed for"
        );
    }

    /// Rows signed before v2 existed keep verifying, so enabling key-binding
    /// does not invalidate an existing brain's provenance.
    #[test]
    fn legacy_v1_signatures_still_verify() {
        let id = BrainIdentity::generate();
        let legacy_payload =
            memory_signing_payload(id.brain_id(), "abc123", "2026-07-10T12:00:00Z", "team");
        let sig = id.sign(&legacy_payload);
        assert!(verify_memory_signature(
            id.brain_id(),
            id.verifying_key(),
            "any-key-at-all",
            "abc123",
            "2026-07-10T12:00:00Z",
            "team",
            &sig,
        ));
    }

    #[test]
    fn foreign_key_cannot_impersonate_origin() {
        let alice = BrainIdentity::generate();
        let mallory = BrainIdentity::generate();
        let sig = alice.sign_memory("mem-key", "abc123", "2026-07-10T12:00:00Z", "team");
        // Mallory presents Alice's brain_id but her own key: pubkey doesn't
        // match the claimed origin -> reject.
        assert!(!verify_memory_signature(
            alice.brain_id(),
            mallory.verifying_key(),
            "mem-key",
            "abc123",
            "2026-07-10T12:00:00Z",
            "team",
            &sig,
        ));
        // Mallory re-signs under her own identity but claims Alice's id ->
        // brain_id/pubkey mismatch -> reject.
        let forged = mallory.sign_memory("mem-key", "abc123", "2026-07-10T12:00:00Z", "team");
        assert!(!verify_memory_signature(
            alice.brain_id(),
            mallory.verifying_key(),
            "mem-key",
            "abc123",
            "2026-07-10T12:00:00Z",
            "team",
            &forged,
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
