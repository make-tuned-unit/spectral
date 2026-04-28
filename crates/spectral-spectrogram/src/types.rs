//! Core types for the cognitive spectrogram.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The action type of a memory, classified by keyword patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionType {
    Decision,
    Discovery,
    Task,
    Observation,
    Advice,
    Reflection,
}

impl ActionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Discovery => "discovery",
            Self::Task => "task",
            Self::Observation => "observation",
            Self::Advice => "advice",
            Self::Reflection => "reflection",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "decision" => Self::Decision,
            "discovery" => Self::Discovery,
            "task" => Self::Task,
            "advice" => Self::Advice,
            "reflection" => Self::Reflection,
            _ => Self::Observation,
        }
    }
}

impl std::fmt::Display for ActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A cognitive fingerprint of a memory across seven dimensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectralFingerprint {
    pub memory_id: String,
    pub entity_density: f64,
    pub action_type: ActionType,
    pub decision_polarity: f64,
    pub causal_depth: f64,
    pub emotional_valence: f64,
    pub temporal_specificity: f64,
    pub novelty: f64,
    /// Top 2-3 dimensions by magnitude.
    pub peak_dimensions: Vec<String>,
    pub created_at: DateTime<Utc>,
}
