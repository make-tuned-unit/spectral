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
