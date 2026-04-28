//! Error types for spectral-core.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("signature error: {0}")]
    Signature(#[from] ed25519_dalek::SignatureError),

    #[error("invalid entity ID: {0}")]
    InvalidEntityId(String),

    #[error("invalid brain ID: {0}")]
    InvalidBrainId(String),

    #[error("invalid device ID: {0}")]
    InvalidDeviceId(String),
}
