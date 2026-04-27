//! Per-edge provenance metadata for triples.
//!
//! Captures the identity and timing of every assertion in the graph.
//! This module defines types only — no business logic.

use chrono::{DateTime, Utc};
use spectral_core::identity::BrainId;

/// Per-edge provenance metadata captured on every triple.
///
/// # Create with brain identity
///
/// ```
/// use spectral_core::identity::BrainIdentity;
/// use spectral_graph::provenance::Provenance;
///
/// let identity = BrainIdentity::generate();
/// let prov = Provenance::new(*identity.brain_id());
/// assert!(prov.source_doc_id.is_none());
/// ```
///
/// # Attach a document source
///
/// ```
/// use spectral_core::identity::BrainIdentity;
/// use spectral_graph::provenance::Provenance;
///
/// let identity = BrainIdentity::generate();
/// let doc_hash = [0xABu8; 32];
/// let prov = Provenance::new(*identity.brain_id()).with_doc(doc_hash);
/// assert!(prov.source_doc_id.is_some());
/// ```
///
/// # Timestamp is set automatically
///
/// ```
/// use spectral_core::identity::BrainIdentity;
/// use spectral_graph::provenance::Provenance;
/// use chrono::Utc;
///
/// let identity = BrainIdentity::generate();
/// let prov = Provenance::new(*identity.brain_id());
/// let elapsed = Utc::now() - prov.asserted_at;
/// assert!(elapsed.num_seconds() < 2);
/// ```
///
/// # BrainId is preserved
///
/// ```
/// use spectral_core::identity::BrainIdentity;
/// use spectral_graph::provenance::Provenance;
///
/// let identity = BrainIdentity::generate();
/// let prov = Provenance::new(*identity.brain_id());
/// assert_eq!(&prov.source_brain_id, identity.brain_id());
/// ```
#[derive(Debug, Clone)]
pub struct Provenance {
    /// Blake3 hash of the source document, if any.
    pub source_doc_id: Option<[u8; 32]>,
    /// Identity of the brain that asserted this triple.
    pub source_brain_id: BrainId,
    /// When the assertion was made.
    pub asserted_at: DateTime<Utc>,
}

impl Provenance {
    /// Create new provenance with `asserted_at` set to now.
    pub fn new(source_brain_id: BrainId) -> Self {
        Self {
            source_doc_id: None,
            source_brain_id,
            asserted_at: Utc::now(),
        }
    }

    /// Attach a source document hash.
    pub fn with_doc(mut self, doc_id: [u8; 32]) -> Self {
        self.source_doc_id = Some(doc_id);
        self
    }
}
