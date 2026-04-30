//! Configurable signal scoring for memory ingestion.
//!
//! Scores memories by cognitive significance (0.0–1.0) based on hall
//! classification and content keyword analysis. Ported from the
//! production-validated reference implementation (`signal_scorer.py`).
//!
//! # Algorithm
//!
//! The default scorer computes:
//!
//! 1. **Hall base score** — looked up by hall name (case-insensitive):
//!
//!    | Hall         | Base score |
//!    |--------------|-----------|
//!    | `fact`       | 0.70      |
//!    | `discovery`  | 0.65      |
//!    | `preference` | 0.60      |
//!    | `advice`     | 0.55      |
//!    | `event`      | 0.50      |
//!    | *(other)*    | 0.50      |
//!
//! 2. **Content boosters** — each group adds its boost if *any* keyword
//!    appears in the content (case-insensitive, checked once):
//!
//!    | Group      | Keywords                                    | Boost |
//!    |------------|---------------------------------------------|-------|
//!    | decisions  | decided, chose, switched, approved, rejected | +0.15 |
//!    | errors     | error, bug, failed, broke, crash             | +0.10 |
//!    | learnings  | learned, realized, breakthrough, insight     | +0.10 |
//!    | rules      | always, never, rule, policy, must            | +0.10 |
//!    | urgency    | deadline, urgent, critical, blocker          | +0.10 |
//!
//! 3. **Cap** — final score is clamped to `max_score` (default 1.0).
//!
//! # Usage
//!
//! **Pattern A: Consumer scores explicitly.**
//!
//! ```
//! use spectral_ingest::signal_scorer::{DefaultSignalScorer, SignalScorer};
//!
//! let scorer = DefaultSignalScorer::new();
//! let score = scorer.score("Alice decided to use Clerk", Some("fact"));
//! assert!((score - 0.85).abs() < 0.001);
//! ```
//!
//! **Pattern B: Brain auto-scores (future PR).**
//!
//! In a future PR, `Brain` will optionally hold a `Box<dyn SignalScorer>`
//! and auto-compute `signal_score` when `RememberOpts::signal_score` is
//! `None`. This avoids requiring consumers to wire up scoring manually.
//!
//! # Custom scorers
//!
//! Implement the [`SignalScorer`] trait for domain-specific scoring:
//!
//! ```
//! use spectral_ingest::signal_scorer::SignalScorer;
//!
//! struct AlwaysHighScorer;
//!
//! impl SignalScorer for AlwaysHighScorer {
//!     fn score(&self, _content: &str, _hall: Option<&str>) -> f64 {
//!         0.9
//!     }
//! }
//! ```

use std::collections::HashMap;

/// Computes a signal score (0.0–1.0) for a memory based on its content
/// and classification. Higher scores indicate higher cognitive significance.
pub trait SignalScorer: Send + Sync {
    /// Score a memory. Returns a value in \[0.0, 1.0\].
    fn score(&self, content: &str, hall: Option<&str>) -> f64;
}

/// A keyword group that adds a boost when any keyword matches.
#[derive(Debug, Clone)]
pub struct KeywordBooster {
    /// Descriptive name for this booster group (e.g. "decisions").
    pub name: String,
    /// Keywords to match (case-insensitive substring match).
    pub keywords: Vec<String>,
    /// Score boost added when any keyword matches.
    pub boost: f64,
}

/// Configuration for [`DefaultSignalScorer`].
#[derive(Debug, Clone)]
pub struct SignalScorerConfig {
    /// Base score for memories without a recognized hall classification.
    pub default_base_score: f64,
    /// Hall-based base scores. Keys must be lowercase.
    pub hall_base_scores: HashMap<String, f64>,
    /// Booster keyword groups. Each group adds its boost if any keyword matches.
    pub boosters: Vec<KeywordBooster>,
    /// Maximum signal score cap.
    pub max_score: f64,
}

impl Default for SignalScorerConfig {
    fn default() -> Self {
        let mut hall_base_scores = HashMap::new();
        hall_base_scores.insert("fact".into(), 0.7);
        hall_base_scores.insert("discovery".into(), 0.65);
        hall_base_scores.insert("preference".into(), 0.6);
        hall_base_scores.insert("advice".into(), 0.55);
        hall_base_scores.insert("event".into(), 0.5);

        let boosters = vec![
            KeywordBooster {
                name: "decisions".into(),
                keywords: vec![
                    "decided".into(),
                    "chose".into(),
                    "switched".into(),
                    "approved".into(),
                    "rejected".into(),
                ],
                boost: 0.15,
            },
            KeywordBooster {
                name: "errors".into(),
                keywords: vec![
                    "error".into(),
                    "bug".into(),
                    "failed".into(),
                    "broke".into(),
                    "crash".into(),
                ],
                boost: 0.1,
            },
            KeywordBooster {
                name: "learnings".into(),
                keywords: vec![
                    "learned".into(),
                    "realized".into(),
                    "breakthrough".into(),
                    "insight".into(),
                ],
                boost: 0.1,
            },
            KeywordBooster {
                name: "rules".into(),
                keywords: vec![
                    "always".into(),
                    "never".into(),
                    "rule".into(),
                    "policy".into(),
                    "must".into(),
                ],
                boost: 0.1,
            },
            KeywordBooster {
                name: "urgency".into(),
                keywords: vec![
                    "deadline".into(),
                    "urgent".into(),
                    "critical".into(),
                    "blocker".into(),
                ],
                boost: 0.1,
            },
        ];

        Self {
            default_base_score: 0.5,
            hall_base_scores,
            boosters,
            max_score: 1.0,
        }
    }
}

/// Default heuristic signal scorer based on hall classification and
/// content keyword analysis.
///
/// Ported from the production reference implementation (`signal_scorer.py`).
/// Pure heuristic — no embeddings, no LLM calls, sub-microsecond per memory.
pub struct DefaultSignalScorer {
    config: SignalScorerConfig,
}

impl DefaultSignalScorer {
    /// Create a scorer with the default configuration matching the
    /// reference implementation.
    pub fn new() -> Self {
        Self {
            config: SignalScorerConfig::default(),
        }
    }

    /// Create a scorer with a custom configuration.
    pub fn with_config(config: SignalScorerConfig) -> Self {
        Self { config }
    }
}

impl Default for DefaultSignalScorer {
    fn default() -> Self {
        Self::new()
    }
}

impl SignalScorer for DefaultSignalScorer {
    fn score(&self, content: &str, hall: Option<&str>) -> f64 {
        let mut score = match hall {
            Some(h) => {
                let h_lower = h.to_lowercase();
                *self
                    .config
                    .hall_base_scores
                    .get(&h_lower)
                    .unwrap_or(&self.config.default_base_score)
            }
            None => self.config.default_base_score,
        };

        let content_lower = content.to_lowercase();

        for booster in &self.config.boosters {
            if booster
                .keywords
                .iter()
                .any(|kw| content_lower.contains(kw.as_str()))
            {
                score += booster.boost;
            }
        }

        score.min(self.config.max_score)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_pure_event_content() {
        let scorer = DefaultSignalScorer::new();
        let score = scorer.score("", Some("event"));
        assert!((score - 0.5).abs() < 0.001, "event base = 0.5, got {score}");
    }

    #[test]
    fn score_decision_content() {
        let scorer = DefaultSignalScorer::new();
        // fact(0.7) + decided(0.15) = 0.85
        let score = scorer.score("Alice decided to use Clerk for auth", Some("fact"));
        assert!(
            (score - 0.85).abs() < 0.001,
            "fact(0.7) + decided(0.15) = 0.85, got {score}"
        );
    }

    #[test]
    fn score_critical_bug() {
        let scorer = DefaultSignalScorer::new();
        // discovery(0.65) + error(0.1) + urgency(0.1) = 0.85
        let score = scorer.score("Found a critical bug in auth flow", Some("discovery"));
        assert!(
            (score - 0.85).abs() < 0.001,
            "discovery(0.65) + bug(0.1) + critical(0.1) = 0.85, got {score}"
        );
    }

    #[test]
    fn score_caps_at_one() {
        let scorer = DefaultSignalScorer::new();
        // fact(0.7) + decided(0.15) + error(0.1) + learned(0.1) + rule(0.1) + critical(0.1) = 1.25 → 1.0
        let score = scorer.score(
            "decided the rule: always report crash errors — learned this is critical",
            Some("fact"),
        );
        assert!(
            (score - 1.0).abs() < 0.001,
            "should cap at 1.0, got {score}"
        );
    }

    #[test]
    fn score_unknown_hall() {
        let scorer = DefaultSignalScorer::new();
        let score = scorer.score("hello", Some("banana"));
        assert!(
            (score - 0.5).abs() < 0.001,
            "unknown hall = default 0.5, got {score}"
        );
    }

    #[test]
    fn score_no_hall() {
        let scorer = DefaultSignalScorer::new();
        let score = scorer.score("hello", None);
        assert!(
            (score - 0.5).abs() < 0.001,
            "None hall = default 0.5, got {score}"
        );
    }

    #[test]
    fn score_case_insensitive_keywords() {
        let scorer = DefaultSignalScorer::new();
        // event(0.5) + urgency(0.1) = 0.6
        let score = scorer.score("CRITICAL Issue found", Some("event"));
        assert!(
            (score - 0.6).abs() < 0.001,
            "event(0.5) + critical(0.1) = 0.6, got {score}"
        );
    }

    #[test]
    fn score_case_insensitive_hall() {
        let scorer = DefaultSignalScorer::new();
        let score = scorer.score("something happened", Some("FACT"));
        assert!(
            (score - 0.7).abs() < 0.001,
            "FACT should match fact(0.7), got {score}"
        );
    }

    #[test]
    fn custom_config_replaces_defaults() {
        let mut hall_scores = HashMap::new();
        hall_scores.insert("custom".into(), 0.9);

        let config = SignalScorerConfig {
            default_base_score: 0.1,
            hall_base_scores: hall_scores,
            boosters: vec![KeywordBooster {
                name: "test".into(),
                keywords: vec!["magic".into()],
                boost: 0.05,
            }],
            max_score: 0.95,
        };
        let scorer = DefaultSignalScorer::with_config(config);

        assert!(
            (scorer.score("hello", Some("custom")) - 0.9).abs() < 0.001,
            "custom hall = 0.9"
        );
        assert!(
            (scorer.score("hello", Some("unknown")) - 0.1).abs() < 0.001,
            "unknown hall = custom default 0.1"
        );
        assert!(
            (scorer.score("magic trick", Some("custom")) - 0.95).abs() < 0.001,
            "0.9 + 0.05 = 0.95 (at cap)"
        );
        // decision keywords from default config should NOT apply
        assert!(
            (scorer.score("decided something", Some("custom")) - 0.9).abs() < 0.001,
            "default boosters should not apply in custom config"
        );
    }

    /// Verify exact match with reference Python implementation for 5 canonical inputs.
    ///
    /// Python: score_memory(content, key="", wing="", hall=hall, room="")
    /// The Python impl only uses content and hall; key/wing/room are unused for scoring.
    #[test]
    fn score_matches_reference_implementation() {
        let scorer = DefaultSignalScorer::new();

        // 1. Pure event, no keywords → event(0.5) = 0.5
        let s1 = scorer.score("User logged in from new device", Some("event"));
        assert!(
            (s1 - 0.5).abs() < 0.001,
            "ref case 1: expected 0.5, got {s1}"
        );

        // 2. Fact with decision → fact(0.7) + decided(0.15) = 0.85
        let s2 = scorer.score("Alice decided to use Clerk for auth", Some("fact"));
        assert!(
            (s2 - 0.85).abs() < 0.001,
            "ref case 2: expected 0.85, got {s2}"
        );

        // 3. Discovery with learning → discovery(0.65) + learned(0.1) = 0.75
        let s3 = scorer.score(
            "I learned a breakthrough insight about caching",
            Some("discovery"),
        );
        assert!(
            (s3 - 0.75).abs() < 0.001,
            "ref case 3: expected 0.75, got {s3}"
        );

        // 4. Preference with rule → preference(0.6) + rule(0.1) = 0.7
        let s4 = scorer.score("always use snake_case as a rule", Some("preference"));
        assert!(
            (s4 - 0.7).abs() < 0.001,
            "ref case 4: expected 0.7, got {s4}"
        );

        // 5. All boosters + fact → fact(0.7) + 0.15 + 0.1 + 0.1 + 0.1 + 0.1 = 1.25 → 1.0
        let s5 = scorer.score(
            "decided to fix the critical crash bug, learned the policy must always apply",
            Some("fact"),
        );
        assert!(
            (s5 - 1.0).abs() < 0.001,
            "ref case 5: expected 1.0, got {s5}"
        );
    }
}
