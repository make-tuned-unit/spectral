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
            // Repair the mode on LOAD, not only on create. A key written by an
            // earlier build, by a different tool, or restored by a copy that
            // did not preserve permissions stays wrong forever otherwise —
            // which is exactly what happened in production, where a brain.key
            // sat at 0644 for months while the create path was correct.
            Self::set_key_permissions(&key_path)?;
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

    /// `as_bytes` must return the id's real bytes: it is what
    /// `memory_signing_payload_v2` splices into the signed payload to bind the
    /// origin brain. Found by mutation — returning a constant array passed
    /// every other test, because `verify` compares `BrainId` values directly
    /// and never goes through `as_bytes`.
    #[test]
    fn brain_id_as_bytes_returns_the_real_bytes() {
        let a = BrainIdentity::generate();
        let b = BrainIdentity::generate();
        assert_ne!(
            a.brain_id().as_bytes(),
            b.brain_id().as_bytes(),
            "two independent brains share as_bytes output"
        );
        // And it agrees with the canonical hex rendering.
        let hex: String = a
            .brain_id()
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(hex, a.brain_id().to_string());
    }

    /// The origin brain must actually reach the signed payload, so two brains
    /// signing identical memory fields produce different payloads.
    #[test]
    fn the_signing_payload_binds_the_origin_brain() {
        let a = BrainIdentity::generate();
        let b = BrainIdentity::generate();
        assert_ne!(
            memory_signing_payload_v2(a.brain_id(), "k", "h", "t", "team"),
            memory_signing_payload_v2(b.brain_id(), "k", "h", "t", "team"),
            "the payload does not bind the origin brain id"
        );
    }

    /// `Debug` is what lands in error messages and logs, so it must carry the
    /// id rather than eliding it.
    #[test]
    fn brain_id_debug_contains_the_full_hex_id() {
        let id = BrainIdentity::generate();
        let shown = format!("{:?}", id.brain_id());
        assert!(
            shown.contains(&id.brain_id().to_string()),
            "Debug elided the id: {shown}"
        );
    }

    // ── federation attestation payloads ────────────────────────────
    //
    // Found by `cargo mutants`: replacing `length_prefixed`,
    // `federation_object_signing_payload` or `federation_tombstone_signing_payload`
    // with a constant left the **entire 1194-test workspace green**. With an
    // empty payload every object and every retraction signs identical bytes, so
    // under `ImportPolicy::RequireSigned` one valid signature from any accepted
    // key authenticates any object and any retraction in any wing.
    //
    // Two reasons nothing caught it. The length-prefix rule is implemented
    // twice — inline in `memory_signing_payload_v2`, and here as
    // `length_prefixed` — and only the inline copy was tested. And every
    // existing federation test signs and verifies through the *same* function,
    // so a compatibly-wrong pair round-trips happily.
    //
    // These tests are therefore all attack-shaped or structural. None of them
    // is a round trip.

    fn sig_bytes(s: &Signature) -> Vec<u8> {
        s.to_bytes().to_vec()
    }

    /// A signature over one object must not authenticate a different object.
    /// This is the mutant's actual consequence, stated as the attack.
    #[test]
    fn an_object_signature_does_not_authenticate_a_different_object() {
        let id = BrainIdentity::generate();
        let sig_a = id.sign_federation_object("hash-aaaa");

        assert!(
            verify(
                id.brain_id(),
                id.verifying_key(),
                &federation_object_signing_payload("hash-aaaa"),
                &sig_a
            ),
            "precondition: the signature should verify its own object"
        );
        assert!(
            !verify(
                id.brain_id(),
                id.verifying_key(),
                &federation_object_signing_payload("hash-bbbb"),
                &sig_a
            ),
            "a signature over one object authenticated a DIFFERENT object —              the payload does not bind the object hash"
        );
    }

    /// Domain separation: an object attestation must not be replayable as a
    /// retraction. Without distinct domain tags, publishing a signed object
    /// would hand every peer a valid retraction for it.
    #[test]
    fn an_object_signature_cannot_be_replayed_as_a_retraction() {
        let id = BrainIdentity::generate();
        let target = "hash-aaaa";
        let obj_sig = id.sign_federation_object(target);

        assert_ne!(
            federation_object_signing_payload(target),
            federation_tombstone_signing_payload("wing", target, "2026-01-01T00:00:00Z"),
            "object and tombstone payloads must be domain-separated"
        );
        assert!(
            !verify(
                id.brain_id(),
                id.verifying_key(),
                &federation_tombstone_signing_payload("wing", target, "2026-01-01T00:00:00Z"),
                &obj_sig
            ),
            "an object attestation was accepted as a retraction"
        );
    }

    /// A retraction is wing-scoped: one authorised for `wing-a` must not
    /// suppress the same object in `wing-b`.
    #[test]
    fn a_retraction_does_not_replay_into_another_wing() {
        let id = BrainIdentity::generate();
        let ts = "2026-01-01T00:00:00Z";
        let sig = id.sign_federation_tombstone("wing-a", "hash-aaaa", ts);

        assert!(verify(
            id.brain_id(),
            id.verifying_key(),
            &federation_tombstone_signing_payload("wing-a", "hash-aaaa", ts),
            &sig
        ));
        assert!(
            !verify(
                id.brain_id(),
                id.verifying_key(),
                &federation_tombstone_signing_payload("wing-b", "hash-aaaa", ts),
                &sig
            ),
            "a retraction authorised for wing-a was valid in wing-b"
        );
    }

    /// Each of the retraction's three fields must be bound independently.
    #[test]
    fn changing_any_retraction_field_invalidates_the_signature() {
        let id = BrainIdentity::generate();
        let (w, t, ts) = ("wing-a", "hash-aaaa", "2026-01-01T00:00:00Z");
        let sig = id.sign_federation_tombstone(w, t, ts);

        for (label, payload) in [
            ("wing", federation_tombstone_signing_payload("other", t, ts)),
            (
                "target",
                federation_tombstone_signing_payload(w, "hash-bbbb", ts),
            ),
            (
                "timestamp",
                federation_tombstone_signing_payload(w, t, "2027-06-06T00:00:00Z"),
            ),
        ] {
            assert!(
                !verify(id.brain_id(), id.verifying_key(), &payload, &sig),
                "changing the {label} did not invalidate the retraction signature"
            );
        }
    }

    /// Field-boundary unambiguity for the three-field retraction payload — the
    /// same property `payload_is_unambiguous_across_field_boundaries` pins for
    /// memories, applied to the second, previously untested implementation.
    #[test]
    fn retraction_field_boundaries_are_unambiguous() {
        let shifted = [
            (("ab", "c", "t"), ("a", "bc", "t")),
            (("w", "ab", "c"), ("w", "a", "bc")),
        ];
        for ((w1, t1, s1), (w2, t2, s2)) in shifted {
            assert_ne!(
                federation_tombstone_signing_payload(w1, t1, s1),
                federation_tombstone_signing_payload(w2, t2, s2),
                "({w1:?},{t1:?},{s1:?}) and ({w2:?},{t2:?},{s2:?}) share a payload"
            );
        }
    }

    /// Structural: the payload must actually carry its domain tag and its
    /// field bytes. Kills any constant-returning implementation directly,
    /// rather than only through its consequences.
    #[test]
    fn federation_payloads_carry_their_domain_tag_and_fields() {
        let obj = federation_object_signing_payload("hash-aaaa");
        assert!(
            obj.starts_with(FEDERATION_OBJ_SIG_DOMAIN),
            "object payload lost its domain tag"
        );
        assert!(
            obj.windows(9).any(|w| w == b"hash-aaaa"),
            "object payload does not contain the object hash"
        );
        // domain + 4-byte length prefix + the field itself.
        assert_eq!(obj.len(), FEDERATION_OBJ_SIG_DOMAIN.len() + 4 + 9);

        let tomb = federation_tombstone_signing_payload("wing-a", "hash-aaaa", "ts");
        assert!(
            tomb.starts_with(FEDERATION_TOMBSTONE_SIG_DOMAIN),
            "tombstone payload lost its domain tag"
        );
        assert_eq!(
            tomb.len(),
            FEDERATION_TOMBSTONE_SIG_DOMAIN.len() + (4 + 6) + (4 + 9) + (4 + 2)
        );
    }

    /// Distinct inputs must give distinct payloads — a blanket guard against
    /// any constant-returning implementation.
    #[test]
    fn distinct_inputs_never_share_a_federation_payload() {
        let payloads = [
            federation_object_signing_payload("a"),
            federation_object_signing_payload("b"),
            federation_tombstone_signing_payload("w", "a", "t"),
            federation_tombstone_signing_payload("w", "b", "t"),
        ];
        for (i, a) in payloads.iter().enumerate() {
            assert!(!a.is_empty(), "payload {i} is empty");
            for (j, b) in payloads.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "payloads {i} and {j} collide");
            }
        }
    }

    /// Two brains signing the same object produce different signatures, and
    /// neither verifies under the other's identity.
    #[test]
    fn a_foreign_key_cannot_attest_a_federation_object() {
        let mine = BrainIdentity::generate();
        let theirs = BrainIdentity::generate();
        let payload = federation_object_signing_payload("hash-aaaa");

        let their_sig = theirs.sign_federation_object("hash-aaaa");
        assert_ne!(
            sig_bytes(&their_sig),
            sig_bytes(&mine.sign_federation_object("hash-aaaa"))
        );
        assert!(
            !verify(mine.brain_id(), mine.verifying_key(), &payload, &their_sig),
            "another brain's attestation verified as mine"
        );
    }

    /// Loading an existing key must REPAIR its mode, not just trust it.
    ///
    /// The create path was always correct; the load path was not, so a key
    /// written by an earlier build or restored by a `cp` that dropped
    /// permissions stayed world-readable indefinitely. Found in production: a
    /// real `brain.key` sat at 0644 for four months.
    #[cfg(unix)]
    #[test]
    fn loading_an_existing_key_repairs_world_readable_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let created = BrainIdentity::load_or_create(dir.path()).unwrap();
        let path = dir.path().join("brain.key");

        // Simulate the damage: a restore or an older build leaves it readable.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644,
            "precondition: the key should be world-readable before the reload"
        );

        let reloaded = BrainIdentity::load_or_create(dir.path()).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600,
            "loading an existing key did not repair its permissions"
        );
        // And it is still the same identity, not a silently regenerated one.
        assert_eq!(reloaded.brain_id(), created.brain_id());
    }

    /// The private key file must not be group- or world-readable. Found by
    /// mutation: `set_key_permissions` could be replaced with `Ok(())` and
    /// nothing failed, so the 0600 mode was never actually checked.
    #[cfg(unix)]
    #[test]
    fn a_persisted_signing_key_is_not_readable_by_others() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let _id = BrainIdentity::load_or_create(dir.path()).unwrap();
        let path = dir.path().join("brain.key");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "signing key at {} has mode {mode:o}, expected 600 — the private              key is readable beyond its owner",
            path.display()
        );
    }
}
