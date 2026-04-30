//! Memory-quality maintenance for Spectral brains.
//!
//! Ports the production-validated Librarian maintenance agent to Rust.
//! Five algorithmic passes keep memory quality high over time:
//!
//! 1. **Deduplication** — Jaccard similarity on word sets per wing
//! 2. **Gap detection** — wings missing summaries, facts, or people
//! 3. **Reclassification** — general-wing memories that belong elsewhere
//! 4. **Signal decay/boost** — stale memories decay, recently-used get boosted
//! 5. **Consolidation candidates** — pairs eligible for LLM-mediated merge
//!
//! Two LLM-mediated passes (full consolidation, index generation) are
//! shipped as pluggable traits with no-op defaults. Consumers can wire
//! their own LLM clients in Phase 2.
//!
//! # Usage
//!
//! ```no_run
//! use spectral_archivist::Archivist;
//! use std::path::Path;
//!
//! let archivist = Archivist::open(Path::new("./my-brain/memory.db")).unwrap();
//!
//! // Dry-run report (no mutations)
//! let report = archivist.report().unwrap();
//! println!("Duplicates: {}", report.duplicates.len());
//! println!("Gaps: {:?}", report.gaps);
//!
//! // Apply signal decay (the only mutation pass)
//! let stats = archivist.apply_decay().unwrap();
//! println!("Decayed: {}, Boosted: {}", stats.decayed, stats.boosted);
//! ```

pub mod archivist;
pub mod candidates;
pub mod decay;
pub mod duplicates;
pub mod gaps;
pub mod reclassify;
pub mod traits;

pub use archivist::{Archivist, ArchivistConfig, ArchivistReport, ArchivistRunReport};
pub use candidates::ConsolidationCandidate;
pub use decay::DecayStats;
pub use duplicates::DuplicatePair;
pub use gaps::GapReport;
pub use reclassify::ReclassificationSuggestion;
pub use traits::{Consolidator, Indexer, NoOpConsolidator, NoOpIndexer};
