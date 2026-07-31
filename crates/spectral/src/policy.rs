//! Versioned retrieval policy — question-shape classification and the
//! per-shape retrieval configuration that follows from it.
//!
//! # Why this is in the library
//!
//! This logic lived in `spectral-bench-accuracy` (the benchmark harness), which
//! meant every published accuracy number described a *harness* configuration
//! that no consumer could execute. The library's own `recall_*` entry points
//! applied none of it. That is a credibility problem, not a packaging one: a
//! result is only meaningful if the configuration that produced it ships.
//!
//! Everything here is deterministic — regex classification and config
//! selection, no LLM, no I/O, no randomness. The same question always yields
//! the same shape and the same config.
//!
//! # What deliberately did NOT move
//!
//! Actor prompt templates stay in the harness. They describe how to talk to a
//! model, not how memory retrieves, and embedding them here would make the
//! library carry benchmark-specific prompt engineering.
//!
//! # Versioning
//!
//! [`RetrievalPolicyVersion`] names the policy so a published result can cite
//! the exact executable configuration behind it. The current behaviour is
//! [`RetrievalPolicyVersion::V1`], moved **verbatim** from the harness so the
//! migration is provably behaviour-preserving — the $0 retrieval oracle emits
//! the same ordered hits and context hashes before and after.

use regex::Regex;

use spectral_graph::cascade_layers::CascadePipelineConfig;

/// Which retrieval path a question routes to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalRoute {
    /// Top-K FTS with re-ranking.
    #[default]
    TopkFts,
    /// TACT/FTS recall (legacy).
    Tact,
    /// Graph traversal.
    Graph,
    /// Cascade (L1→L2→L3).
    Cascade,
}

/// Version of the retrieval policy. Cite this alongside any published result
/// so the configuration behind the number is unambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum RetrievalPolicyVersion {
    /// The shape routing and per-shape profiles as measured on LongMemEval-S,
    /// moved verbatim from the benchmark harness.
    #[default]
    V1,
}

impl RetrievalPolicyVersion {
    /// Stable string form for receipts, logs, and published results.
    pub fn as_str(&self) -> &'static str {
        match self {
            RetrievalPolicyVersion::V1 => "retrieval-v1",
        }
    }
}

/// Question shape, determined by structural analysis of the query.
///
/// Two-level classification:
/// - Level 1: top-level shape (Counting, Temporal, Factual, General)
/// - Level 2: sub-shape within Counting, Factual, and General
///
/// Temporal intentionally has NO sub-gate. Date arithmetic is a single
/// coherent strategy; adding a current-state variant would fragment it
/// without evidence of benefit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionShape {
    /// "How many", "how much", "total" — exhaustive session scan, no recency signal.
    Counting,
    /// "How many ... currently/still" — current count, recency-priority.
    CountingCurrentState,
    /// Date arithmetic, ordering, duration. No sub-gate — single coherent strategy.
    Temporal,
    /// "What is", "where", "who" — single-entity retrieval.
    Factual,
    /// "What is my current X" — most-recent-wins factual.
    FactualCurrentState,
    /// "Suggest/recommend/tips/advice" — preference inference.
    GeneralPreference,
    /// "Remind me/going back to/we discussed" — assistant recall.
    GeneralRecall,
    /// Catch-all fallback.
    General,
}

impl QuestionShape {
    /// Classify a question string into a routing shape.
    ///
    /// Level 1: top-level classifier (Counting/Temporal/Factual/General).
    /// Level 2: sub-gates for Counting (recency), Factual (recency), General
    /// (preference/recall). Temporal has no sub-gate by design.
    pub fn classify(question: &str) -> Self {
        let q = question.to_lowercase();

        // ── Level 1: top-level shape ──

        // Temporal-counting ("how many days/weeks ago", "how old") → Temporal
        if Regex::new(r"how many (?:days|weeks|months|years) (?:ago|since|passed|before|after|between|had passed|have passed|did it take)|how old")
            .unwrap()
            .is_match(&q)
        {
            return Self::Temporal;
        }

        // General counting → Counting (with sub-gate)
        if Regex::new(r"how many|how much|total|in total|altogether")
            .unwrap()
            .is_match(&q)
        {
            // Level 2: recency sub-gate for Counting
            if Regex::new(r"\b(currently|right now|most recent|latest|newest|do i still|now)\b")
                .unwrap()
                .is_match(&q)
            {
                return Self::CountingCurrentState;
            }
            return Self::Counting;
        }

        // Location questions: "where" → Factual, even with temporal modifiers.
        // Temporal modifiers in "where" questions provide context, not focus.
        if Regex::new(r"^where\b").unwrap().is_match(&q) {
            if Regex::new(
                r"\b(currently|right now|most recent|latest|newest|do i still|now|recent)\b",
            )
            .unwrap()
            .is_match(&q)
            {
                return Self::FactualCurrentState;
            }
            return Self::Factual;
        }

        // Temporal — includes explicit ordering phrases ("order ... earliest/latest")
        if Regex::new(r"when did|how long|(?:^|\W)first\b|(?:^|\W)last\b|before|after|ago|since|order.+(?:earliest|latest)|from earliest|chronological|(?:^|\W)order of\b")
            .unwrap()
            .is_match(&q)
        {
            return Self::Temporal;
        }

        // Factual (with sub-gate)
        if Regex::new(r"^(?:what|where|who|which)\b")
            .unwrap()
            .is_match(&q)
        {
            // Level 2: recency sub-gate for Factual
            if Regex::new(
                r"\b(currently|right now|most recent|most recently|latest|newest|do i still|now)\b",
            )
            .unwrap()
            .is_match(&q)
            {
                return Self::FactualCurrentState;
            }
            return Self::Factual;
        }

        // ── Level 2: General sub-gates ──

        if Regex::new(r"\b(suggest|recommend|tips?|advice|recommendations?|what should i)\b")
            .unwrap()
            .is_match(&q)
        {
            return Self::GeneralPreference;
        }
        if Regex::new(r"\bany (tips?|advice|suggestions?|ideas?|thoughts?|recommendations?)\b")
            .unwrap()
            .is_match(&q)
        {
            return Self::GeneralPreference;
        }
        if Regex::new(r"\b(remind me|going back to|previous|earlier conversation|we (discussed|talked about)|can you remind me)\b")
            .unwrap()
            .is_match(&q)
        {
            return Self::GeneralRecall;
        }

        Self::General
    }

    /// The cascade pipeline config tuned for this shape.
    /// Sub-shapes inherit their parent shape's profile.
    pub fn cascade_profile(&self) -> CascadePipelineConfig {
        match self {
            Self::Counting | Self::CountingCurrentState => CascadePipelineConfig {
                k: 60,
                max_per_episode: 3,
                recency_half_life_days: 730.0, // don't penalize any memories
                ..CascadePipelineConfig::default()
            },
            Self::Temporal => CascadePipelineConfig {
                k: 40,
                max_per_episode: 5,
                recency_half_life_days: 60.0, // aggressive recency
                ..CascadePipelineConfig::default()
            },
            Self::Factual | Self::FactualCurrentState => CascadePipelineConfig {
                k: 30,
                max_per_episode: 8,
                ..CascadePipelineConfig::default()
            },
            Self::GeneralPreference | Self::GeneralRecall | Self::General => {
                CascadePipelineConfig::default()
            }
        }
    }

    /// Per-shape retrieval route. Temporal routes to top-k FTS (cascade hurts
    /// temporal by ~-15pp); all other shapes use cascade.
    pub fn retrieval_route(&self) -> RetrievalRoute {
        match self {
            Self::Temporal => RetrievalRoute::TopkFts,
            _ => RetrievalRoute::Cascade,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The profiles are load-bearing on published numbers; pin them so a
    /// refactor cannot silently retune retrieval.
    #[test]
    fn per_shape_profiles_are_pinned() {
        let counting = QuestionShape::Counting.cascade_profile();
        assert_eq!((counting.k, counting.max_per_episode), (60, 3));
        assert_eq!(counting.recency_half_life_days, 730.0);

        let temporal = QuestionShape::Temporal.cascade_profile();
        assert_eq!((temporal.k, temporal.max_per_episode), (40, 5));
        assert_eq!(temporal.recency_half_life_days, 60.0);

        let factual = QuestionShape::Factual.cascade_profile();
        assert_eq!((factual.k, factual.max_per_episode), (30, 8));

        let general = QuestionShape::General.cascade_profile();
        assert_eq!(general.k, CascadePipelineConfig::default().k);
    }

    #[test]
    fn temporal_is_the_only_shape_routed_off_cascade() {
        assert_eq!(
            QuestionShape::Temporal.retrieval_route(),
            RetrievalRoute::TopkFts
        );
        for shape in [
            QuestionShape::Counting,
            QuestionShape::CountingCurrentState,
            QuestionShape::Factual,
            QuestionShape::FactualCurrentState,
            QuestionShape::GeneralPreference,
            QuestionShape::GeneralRecall,
            QuestionShape::General,
        ] {
            assert_eq!(
                shape.retrieval_route(),
                RetrievalRoute::Cascade,
                "{shape:?}"
            );
        }
    }

    #[test]
    fn classification_is_deterministic() {
        let q = "how many times did I go running last month";
        let first = QuestionShape::classify(q);
        for _ in 0..5 {
            assert_eq!(QuestionShape::classify(q), first);
        }
    }
}
