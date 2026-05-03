//! Cascade orchestrator for Spectral's ordered recognition layers.
//!
//! Routes queries through cheap deterministic layers first (L1 AAAK,
//! L3 constellation) and only falls through to heavier layers when needed.
//! Token budgets control how much context each layer contributes.

pub mod orchestrator;
pub mod result;

use serde::{Deserialize, Serialize};
use spectral_ingest::MemoryHit;

/// Layer identifier following the Spectral whitepaper ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LayerId {
    /// L0: filesystem ground truth (consumer-defined).
    L0,
    /// L1: Always-Active Agent Knowledge (foundational facts).
    L1,
    /// L2: curated episode summaries.
    L2,
    /// L3: constellation fingerprint matching (TACT).
    L3,
    /// L4: vector similarity search (deferred by design).
    L4,
    /// L5: ambient activity recognition.
    L5,
}

impl std::fmt::Display for LayerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::L0 => write!(f, "L0"),
            Self::L1 => write!(f, "L1"),
            Self::L2 => write!(f, "L2"),
            Self::L3 => write!(f, "L3"),
            Self::L4 => write!(f, "L4"),
            Self::L5 => write!(f, "L5"),
        }
    }
}

/// Result returned by a single layer.
pub enum LayerResult {
    /// Layer found sufficient context; cascade can stop if confidence
    /// meets the threshold.
    Sufficient {
        hits: Vec<MemoryHit>,
        tokens_used: usize,
        confidence: f64,
    },
    /// Layer found partial context; cascade should continue.
    Partial {
        hits: Vec<MemoryHit>,
        tokens_used: usize,
        confidence: f64,
    },
    /// Layer determined the query doesn't apply to it.
    Skipped { reason: String, confidence: f64 },
}

impl LayerResult {
    pub fn tokens_used(&self) -> usize {
        match self {
            Self::Sufficient { tokens_used, .. } | Self::Partial { tokens_used, .. } => {
                *tokens_used
            }
            Self::Skipped { .. } => 0,
        }
    }

    pub fn hits(&self) -> &[MemoryHit] {
        match self {
            Self::Sufficient { hits, .. } | Self::Partial { hits, .. } => hits,
            Self::Skipped { .. } => &[],
        }
    }

    pub fn confidence(&self) -> f64 {
        match self {
            Self::Sufficient { confidence, .. }
            | Self::Partial { confidence, .. }
            | Self::Skipped { confidence, .. } => *confidence,
        }
    }
}

/// A recognition layer in the cascade.
pub trait Layer: Send + Sync {
    /// Layer identifier (L0–L5).
    fn id(&self) -> LayerId;

    /// Run this layer's query. `budget_remaining` is the token budget
    /// still available across the cascade.
    fn query(
        &self,
        query: &str,
        budget_remaining: usize,
    ) -> Result<LayerResult, Box<dyn std::error::Error + Send + Sync>>;
}
