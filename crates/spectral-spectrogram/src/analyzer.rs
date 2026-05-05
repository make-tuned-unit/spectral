//! SpectrogramAnalyzer: computes cognitive fingerprints for memories.

use std::collections::HashMap;

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

/// Diagnostic introspection from the analysis pipeline (audit-only path).
#[derive(Debug, Clone)]
pub struct AnalysisIntrospection {
    /// Why the action_type was selected (e.g., "matched 'decided' keyword").
    pub action_type_rationale: String,
    /// Entities detected by the entity_density classifier.
    pub entities_detected: Vec<String>,
    /// Per-dimension calculation details.
    pub dimension_calculations: HashMap<String, String>,
    /// Rationale for which dimensions were selected as peaks.
    pub peak_selection_rationale: String,
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

    /// Analyze with full introspection for audit purposes.
    /// Returns the fingerprint plus diagnostic details about every classification decision.
    pub fn analyze_with_introspection(
        &self,
        memory: &Memory,
        context: &AnalysisContext,
    ) -> (SpectralFingerprint, AnalysisIntrospection) {
        let content = &memory.content;
        let at = dimensions::action_type(content);
        let ed = dimensions::entity_density(content);
        let dp = dimensions::decision_polarity(content, at);
        let cd = dimensions::causal_depth(content);
        let ev = dimensions::emotional_valence(content);
        let ts = dimensions::temporal_specificity(content);
        let nv = dimensions::novelty(content, &context.wing_corpus);

        // Action type rationale
        let lower = content.to_lowercase();
        let action_type_rationale = match at {
            crate::ActionType::Decision => {
                let keywords: Vec<&str> = ["decided", "chose", "going with", "locked in", "picked"]
                    .iter()
                    .filter(|k| lower.contains(**k))
                    .copied()
                    .collect();
                format!("matched keyword(s): {}", keywords.join(", "))
            }
            crate::ActionType::Discovery => {
                let keywords: Vec<&str> = [
                    "found that",
                    "noticed",
                    "realized",
                    "discovered",
                    "learned that",
                ]
                .iter()
                .filter(|k| lower.contains(**k))
                .copied()
                .collect();
                format!("matched keyword(s): {}", keywords.join(", "))
            }
            crate::ActionType::Advice => {
                let keywords: Vec<&str> = ["should", "recommend", "suggest", "advise"]
                    .iter()
                    .filter(|k| lower.contains(**k))
                    .copied()
                    .collect();
                format!("matched keyword(s): {}", keywords.join(", "))
            }
            crate::ActionType::Reflection => {
                let keywords: Vec<&str> =
                    ["thinking about", "considering", "reflecting", "wondering"]
                        .iter()
                        .filter(|k| lower.contains(**k))
                        .copied()
                        .collect();
                format!("matched keyword(s): {}", keywords.join(", "))
            }
            crate::ActionType::Task => {
                let keywords: Vec<&str> = ["build", "implement", "fix", "deploy", "ship"]
                    .iter()
                    .filter(|k| lower.contains(**k))
                    .copied()
                    .collect();
                format!("matched keyword(s): {}", keywords.join(", "))
            }
            crate::ActionType::Observation => "no keyword matched; defaulted to observation".into(),
        };

        // Entity detection
        let entities_detected: Vec<String> = content
            .split_whitespace()
            .filter(|w| {
                let is_abbrev = w.len() >= 2
                    && w.chars()
                        .all(|c| c.is_ascii_uppercase() || matches!(c, '-'));
                let is_capitalized = w.chars().next().is_some_and(|c| c.is_uppercase())
                    && w.len() > 1
                    && !w.ends_with('.');
                is_abbrev || is_capitalized
            })
            .map(|s| s.to_string())
            .collect();

        // Dimension calculations
        let mut dimension_calculations = HashMap::new();
        dimension_calculations.insert(
            "entity_density".into(),
            format!(
                "{ed:.3} ({} entities / sqrt({} chars))",
                entities_detected.len(),
                content.len()
            ),
        );
        dimension_calculations.insert(
            "decision_polarity".into(),
            format!("{dp:.3} (action_type={at}, polarity scoring applied)"),
        );
        dimension_calculations.insert(
            "causal_depth".into(),
            format!("{cd:.3} (causal markers per sentence)"),
        );
        dimension_calculations.insert(
            "emotional_valence".into(),
            format!("{ev:.3} (positive - negative sentiment words)"),
        );
        dimension_calculations.insert(
            "temporal_specificity".into(),
            format!("{ts:.3} (time markers per sentence)"),
        );
        dimension_calculations.insert(
            "novelty".into(),
            format!(
                "{nv:.3} (novel terms / total terms, corpus_len={})",
                context.wing_corpus.len()
            ),
        );

        // Peak selection
        let mut dims: Vec<(&str, f64)> = vec![
            ("entity_density", ed),
            ("decision_polarity", dp.abs()),
            ("causal_depth", cd),
            ("emotional_valence", ev.abs()),
            ("temporal_specificity", ts),
            ("novelty", nv),
        ];
        dims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let selected: Vec<String> = dims
            .iter()
            .take(self.config.peak_dimension_count)
            .map(|(name, val)| format!("{name}={val:.3}"))
            .collect();
        let rejected: Vec<String> = dims
            .iter()
            .skip(self.config.peak_dimension_count)
            .map(|(name, val)| format!("{name}={val:.3}"))
            .collect();
        let peak_selection_rationale = format!(
            "selected [{}]; rejected [{}]",
            selected.join(", "),
            rejected.join(", ")
        );

        let peak_dimensions: Vec<String> = dims
            .iter()
            .take(self.config.peak_dimension_count)
            .map(|(name, _)| name.to_string())
            .collect();

        let fp = SpectralFingerprint {
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
        };

        let introspection = AnalysisIntrospection {
            action_type_rationale,
            entities_detected,
            dimension_calculations,
            peak_selection_rationale,
        };

        (fp, introspection)
    }
}

impl Default for SpectrogramAnalyzer {
    fn default() -> Self {
        Self::new(AnalyzerConfig::default())
    }
}
