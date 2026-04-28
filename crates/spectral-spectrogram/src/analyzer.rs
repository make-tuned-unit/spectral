//! SpectrogramAnalyzer: computes cognitive fingerprints for memories.

use chrono::Utc;
use spectral_ingest::Memory;

use crate::dimensions;
use crate::types::SpectralFingerprint;

/// Context for analysis, providing the existing corpus for novelty computation.
#[derive(Default)]
pub struct AnalysisContext {
    /// Concatenated content of existing memories in the same wing.
    /// Used for novelty scoring. Empty string if no context available.
    pub wing_corpus: String,
}

/// Configuration for the analyzer.
#[derive(Debug, Clone)]
pub struct AnalyzerConfig {
    /// Number of peak dimensions to include in the fingerprint.
    pub peak_dimension_count: usize,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self {
            peak_dimension_count: 3,
        }
    }
}

/// Analyzes memories and produces cognitive spectral fingerprints.
pub struct SpectrogramAnalyzer {
    config: AnalyzerConfig,
}

impl SpectrogramAnalyzer {
    pub fn new(config: AnalyzerConfig) -> Self {
        Self { config }
    }

    /// Analyze a memory and produce its spectral fingerprint.
    pub fn analyze(&self, memory: &Memory, context: &AnalysisContext) -> SpectralFingerprint {
        let at = dimensions::action_type(&memory.content);
        let ed = dimensions::entity_density(&memory.content);
        let dp = dimensions::decision_polarity(&memory.content, at);
        let cd = dimensions::causal_depth(&memory.content);
        let ev = dimensions::emotional_valence(&memory.content);
        let ts = dimensions::temporal_specificity(&memory.content);
        let nv = dimensions::novelty(&memory.content, &context.wing_corpus);

        // Pick peak dimensions by absolute magnitude
        let mut dims: Vec<(&str, f64)> = vec![
            ("entity_density", ed),
            ("decision_polarity", dp.abs()),
            ("causal_depth", cd),
            ("emotional_valence", ev.abs()),
            ("temporal_specificity", ts),
            ("novelty", nv),
        ];
        dims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let peak_dimensions: Vec<String> = dims
            .iter()
            .take(self.config.peak_dimension_count)
            .map(|(name, _)| name.to_string())
            .collect();

        SpectralFingerprint {
            memory_id: memory.id.clone(),
            entity_density: ed,
            action_type: at,
            decision_polarity: dp,
            causal_depth: cd,
            emotional_valence: ev,
            temporal_specificity: ts,
            novelty: nv,
            peak_dimensions,
            created_at: Utc::now(),
        }
    }
}

impl Default for SpectrogramAnalyzer {
    fn default() -> Self {
        Self::new(AnalyzerConfig::default())
    }
}
