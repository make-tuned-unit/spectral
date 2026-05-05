//! Cognitive Spectrogram: cross-wing fingerprint matching for Spectral.
//!
//! Classifies memories along seven cognitive dimensions (entity density, action
//! type, decision polarity, causal depth, emotional valence, temporal specificity,
//! novelty) and finds "resonant" memories across wings whose dimensions align.
//!
//! See `DESIGN.md` in this crate for the algorithm details.

pub mod analyzer;
pub mod dimensions;
pub mod matching;
pub mod types;

pub use analyzer::{AnalysisContext, AnalysisIntrospection, AnalyzerConfig, SpectrogramAnalyzer};
pub use matching::ResonantMatch;
pub use types::{ActionType, SpectralFingerprint};
