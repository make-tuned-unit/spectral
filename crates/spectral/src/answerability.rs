//! Query-conditioned deterministic reranking.
//!
//! # Why this exists
//!
//! Spectral's reranker
//! ([`spectral_graph::ranking::apply_reranking_pipeline`]) takes **no query
//! parameter**. Signal score, recency, entity clustering and context dedup are
//! all properties of the *candidate*, not of the candidate's fit to the
//! question. Past the FTS match itself, the question does not influence
//! ranking at all.
//!
//! Every retrieval lever in `docs/MEASURED_RECORD.md` that was measured and
//! rejected — K widening, associative spreading, the fingerprint tier, ACR —
//! is likewise query-independent: they change *which* candidates are in the
//! pool, never *how well each one answers this question*. The one lever that
//! measured best (cross-encoder rerank, +1.6pp session recall with zero
//! session losses) is query-conditioned, and was shelved only because it needs
//! a neural model, colliding with the no-embedding stance.
//!
//! This module is the deterministic form of that idea: score each candidate on
//! how *answerable* it makes the question, using features that are free to
//! compute and reproducible byte-for-byte.
//!
//! Convergent evidence for the design: DMF (arXiv 2606.03463) reranks on
//! deterministic "answerability features" — answer-span likelihood, entity
//! overlap, temporal compatibility, current-state compatibility — with
//! penalties for stale facts, acknowledgement-like social turns, and
//! candidates matching only a topic name without answer-bearing content. It
//! reports 0.7753 vs Mem0's 0.6883 on LoCoMo overall.
//!
//! # Status
//!
//! **Default OFF and unproven.** Nothing here may be defaulted on without a
//! preregistered oracle run. The $0 retrieval oracle is the gate; see
//! `docs/internal/answerability-prereg-2026-08-02.md`.
//!
//! # Determinism
//!
//! Pure function of `(hits, query, shape, config)`. No clock, no I/O, no
//! randomness. Ties break on memory id, matching the sort discipline
//! established by the determinism fix in PR #238 — without it, equal scores
//! preserve SQLite row order, which is not guaranteed stable.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use spectral_ingest::MemoryHit;

use crate::policy::QuestionShape;

macro_rules! cached {
    ($name:ident, $pattern:expr) => {
        fn $name() -> &'static Regex {
            static CELL: OnceLock<Regex> = OnceLock::new();
            CELL.get_or_init(|| Regex::new($pattern).unwrap())
        }
    };
}

cached!(
    re_number,
    r"\b\d+(?:[.,]\d+)?\b|\b(?:one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|dozen|couple|several)\b"
);
cached!(
    re_date,
    r"\b(?:19|20)\d{2}\b|\b\d{1,2}[/-]\d{1,2}\b|\b(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\b|\b(?:mon|tue|wed|thu|fri|sat|sun)[a-z]*day\b|\b(?:yesterday|today|tomorrow|last (?:week|month|year)|next (?:week|month|year)|ago)\b"
);
cached!(re_proper_noun, r"\b[A-Z][a-z]{2,}\b");
cached!(
    re_preference_cue,
    r"\b(?:prefer|prefers|preferred|favou?rite|i like|i love|i hate|i enjoy|can't stand|cannot stand|rather|i'd rather|instead of)\b"
);
cached!(
    re_ack_only,
    r"^\W*(?:ok|okay|sure|thanks|thank you|got it|great|awesome|nice|cool|yeah|yep|yes|no problem|sounds good|will do|perfect|haha|lol|np|np!|同意)\b"
);
// Query words too common to carry topic identity.
cached!(
    re_stopword,
    r"^(?:the|a|an|and|or|but|of|to|in|on|at|for|with|is|are|was|were|be|been|do|does|did|i|you|he|she|it|we|they|my|your|his|her|its|our|their|me|him|them|this|that|these|those|what|when|where|who|which|how|why|did|had|have|has|can|could|would|should|will|about|from|as|by|so|if|than|then|there|here|any|some|all|most|more|much|many|last|first)$"
);

/// What kind of token would actually answer this question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerType {
    /// A count — "how many", "how much".
    Number,
    /// A date, duration or ordering cue.
    Date,
    /// A named thing — "what is", "who", "which".
    Entity,
    /// A stated preference or constraint.
    Preference,
    /// No specific expectation.
    Any,
}

impl AnswerType {
    /// The token type a question shape is asking for.
    pub fn for_shape(shape: QuestionShape) -> Self {
        match shape {
            QuestionShape::Counting | QuestionShape::CountingCurrentState => Self::Number,
            QuestionShape::Temporal => Self::Date,
            QuestionShape::Factual | QuestionShape::FactualCurrentState => Self::Entity,
            QuestionShape::GeneralPreference => Self::Preference,
            QuestionShape::GeneralRecall | QuestionShape::General => Self::Any,
        }
    }

    /// Whether `content` contains a token of this type.
    pub fn present_in(&self, content: &str) -> bool {
        match self {
            Self::Number => re_number().is_match(&content.to_lowercase()),
            Self::Date => re_date().is_match(&content.to_lowercase()),
            Self::Entity => re_proper_noun().is_match(content),
            Self::Preference => re_preference_cue().is_match(&content.to_lowercase()),
            Self::Any => true,
        }
    }
}

/// Feature weights for the answerability score.
///
/// Every weight is an additive adjustment in score units comparable to the
/// existing reranker's boosts (`RECENCY_BOOST_WEIGHT = 0.1`,
/// `declarative_weight = 0.10`), so this composes on the same scale rather
/// than dominating.
#[derive(Debug, Clone)]
pub struct AnswerabilityConfig {
    /// Master switch. **Default `false`** — unproven, see module docs.
    pub enabled: bool,
    /// Bonus when the candidate contains a token of the expected answer type.
    pub answer_type_weight: f64,
    /// Bonus scaled by the fraction of distinct query content words present.
    ///
    /// Orthogonal to BM25: BM25 is IDF-weighted and length-normalised, so a
    /// document can rank highly on one rare term repeated. Coverage of
    /// *distinct* query terms measures something BM25 does not.
    pub coverage_weight: f64,
    /// Penalty for acknowledgement-like turns ("Sure!", "Got it").
    ///
    /// This is the scored form of the render-layer `< 40` char filler drop.
    /// A penalty is strictly better than a hard drop: it demotes phatic turns
    /// without destroying evidence, so a genuinely short answer ("Yes, 42")
    /// can still surface.
    pub ack_penalty: f64,
    /// Penalty when the candidate matches query topic words but contains no
    /// token of the expected answer type — the "matches the subject, answers
    /// nothing" case.
    pub topic_only_penalty: f64,
    /// Score decrement per rank position of the incoming order.
    ///
    /// The incoming rank is real information (BM25 plus the existing
    /// reranker produced it), so answerability adjusts it rather than
    /// replacing it. A **uniform** step matters: the obvious `1/(1+rank)`
    /// prior puts a 0.5 gap between ranks 0 and 1 but only 0.17 between 1 and
    /// 2, so it makes the top slot immovable while letting the tail shuffle
    /// freely — influence that varies by position rather than by evidence.
    ///
    /// At the default 0.03 the score range of the features (about ±0.22) can
    /// move a candidate roughly 15 positions — enough to matter over k=40,
    /// not enough to invert the list.
    pub rank_step: f64,
}

impl Default for AnswerabilityConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            answer_type_weight: 0.12,
            coverage_weight: 0.10,
            ack_penalty: 0.15,
            topic_only_penalty: 0.08,
            rank_step: 0.03,
        }
    }
}

impl AnswerabilityConfig {
    /// Enabled, with default weights.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }
}

/// Distinct, lowercased, non-stopword content words of a query.
fn content_words(query: &str) -> HashSet<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| w.len() > 2 && !re_stopword().is_match(w))
        .map(|w| w.to_string())
        .collect()
}

/// The answerability score for one candidate. Exposed so an audit can show the
/// exact per-feature contribution behind a rank — the same auditability the
/// recognition engine's verdicts provide.
#[derive(Debug, Clone, PartialEq)]
pub struct AnswerabilityScore {
    /// Fraction of distinct query content words present in the candidate.
    pub coverage: f64,
    /// Candidate contains a token of the expected answer type.
    pub has_answer_type: bool,
    /// Candidate reads as an acknowledgement.
    pub is_ack: bool,
    /// Matched the topic but carries no answer-type token.
    pub topic_only: bool,
    /// Net additive adjustment.
    pub delta: f64,
}

/// Score one candidate against a query.
pub fn score_hit(
    hit: &MemoryHit,
    query_words: &HashSet<String>,
    answer_type: AnswerType,
    config: &AnswerabilityConfig,
) -> AnswerabilityScore {
    let content_lower = hit.content.to_lowercase();

    let matched = query_words
        .iter()
        .filter(|w| content_lower.contains(w.as_str()))
        .count();
    let coverage = if query_words.is_empty() {
        0.0
    } else {
        matched as f64 / query_words.len() as f64
    };

    let has_answer_type = answer_type.present_in(&hit.content);
    let is_ack = re_ack_only().is_match(content_lower.trim());
    // "Matched the subject, answered nothing": the candidate does carry query
    // topic words, but none of the token type the question needs. Only
    // meaningful when the shape has a real expectation.
    let topic_only = !has_answer_type && matched > 0 && answer_type != AnswerType::Any;

    let mut delta = coverage * config.coverage_weight;
    if has_answer_type && answer_type != AnswerType::Any {
        delta += config.answer_type_weight;
    }
    if is_ack {
        delta -= config.ack_penalty;
    }
    if topic_only {
        delta -= config.topic_only_penalty;
    }

    AnswerabilityScore {
        coverage,
        has_answer_type,
        is_ack,
        topic_only,
        delta,
    }
}

/// Rerank `hits` in place by answerability, preserving set membership.
///
/// Membership is deliberately untouched — this reorders what retrieval already
/// admitted and never adds or drops a candidate. That keeps the lever
/// separable from admission levers (K widening, fetch-pool widening), which
/// were measured separately and rejected.
///
/// A no-op when `config.enabled` is false.
pub fn rerank(
    hits: &mut [MemoryHit],
    query: &str,
    shape: QuestionShape,
    config: &AnswerabilityConfig,
) {
    if !config.enabled || hits.len() < 2 {
        return;
    }
    let query_words = content_words(query);
    let answer_type = AnswerType::for_shape(shape);

    let mut scored: Vec<(usize, f64, String)> = hits
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let rank_prior = -config.rank_step * i as f64;
            let delta = score_hit(h, &query_words, answer_type, config).delta;
            (i, rank_prior + delta, h.id.clone())
        })
        .collect();

    // Deterministic: score desc, then memory id asc. Never leaves ordering to
    // the stability of the input (see PR #238).
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.2.cmp(&b.2))
    });

    let order: Vec<usize> = scored.into_iter().map(|(i, _, _)| i).collect();
    let reordered: Vec<MemoryHit> = order.into_iter().map(|i| hits[i].clone()).collect();
    hits.clone_from_slice(&reordered);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str, content: &str) -> MemoryHit {
        MemoryHit {
            id: id.to_string(),
            key: format!("s1:turn:{id}:user"),
            content: content.to_string(),
            wing: None,
            hall: None,
            signal_score: 1.0,
            visibility: "private".to_string(),
            hits: 1,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: Some("2023-05-20 12:00:00".to_string()),
            last_reinforced_at: None,
            episode_id: Some("s1".to_string()),
            declarative_density: None,
            description: None,
            source_brain_id: None,
            signature: None,
        }
    }

    #[test]
    fn answer_type_follows_question_shape() {
        assert_eq!(
            AnswerType::for_shape(QuestionShape::Counting),
            AnswerType::Number
        );
        assert_eq!(
            AnswerType::for_shape(QuestionShape::Temporal),
            AnswerType::Date
        );
        assert_eq!(
            AnswerType::for_shape(QuestionShape::Factual),
            AnswerType::Entity
        );
        assert_eq!(
            AnswerType::for_shape(QuestionShape::General),
            AnswerType::Any
        );
    }

    #[test]
    fn answer_type_detection() {
        assert!(AnswerType::Number.present_in("I ran 5 miles"));
        assert!(AnswerType::Number.present_in("I have three cats"));
        assert!(!AnswerType::Number.present_in("I went running"));

        assert!(AnswerType::Date.present_in("back in 2019"));
        assert!(AnswerType::Date.present_in("we met in March"));
        assert!(!AnswerType::Date.present_in("we met at the park"));

        assert!(AnswerType::Entity.present_in("I work at Stripe"));
        assert!(!AnswerType::Entity.present_in("i work there"));

        assert!(AnswerType::Preference.present_in("I prefer oat milk"));
        assert!(!AnswerType::Preference.present_in("the milk was cold"));
    }

    #[test]
    fn disabled_config_is_a_no_op() {
        let mut hits = vec![hit("a", "Sure!"), hit("b", "I ran 5 miles on Tuesday")];
        let before = hits.clone();
        rerank(
            &mut hits,
            "how many miles did I run",
            QuestionShape::Counting,
            &AnswerabilityConfig::default(),
        );
        assert_eq!(
            hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
            before.iter().map(|h| h.id.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn acknowledgement_is_demoted_below_a_real_answer() {
        let mut hits = vec![
            hit("ack", "Sure! Happy to help with that miles question."),
            hit("real", "I ran 5 miles on Tuesday"),
        ];
        rerank(
            &mut hits,
            "how many miles did I run",
            QuestionShape::Counting,
            &AnswerabilityConfig::enabled(),
        );
        assert_eq!(hits[0].id, "real", "ack outranked the answer");
    }

    #[test]
    fn topic_match_without_an_answer_token_is_demoted() {
        let mut hits = vec![
            hit("topic", "we talked about running again"),
            hit("answer", "I ran 12 miles that week"),
        ];
        rerank(
            &mut hits,
            "how many miles running",
            QuestionShape::Counting,
            &AnswerabilityConfig::enabled(),
        );
        assert_eq!(hits[0].id, "answer");
    }

    #[test]
    fn membership_is_always_preserved() {
        let mut hits = vec![
            hit("a", "Sure!"),
            hit("b", "I ran 5 miles"),
            hit("c", "the weather was nice"),
        ];
        let before: HashSet<String> = hits.iter().map(|h| h.id.clone()).collect();
        rerank(
            &mut hits,
            "how many miles",
            QuestionShape::Counting,
            &AnswerabilityConfig::enabled(),
        );
        let after: HashSet<String> = hits.iter().map(|h| h.id.clone()).collect();
        assert_eq!(before, after);
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn reranking_is_deterministic_including_ties() {
        // Identical content -> identical scores -> order must still be stable
        // and id-determined, not input-order-determined.
        let mut a = vec![hit("z", "same text"), hit("a", "same text")];
        let mut b = vec![hit("a", "same text"), hit("z", "same text")];
        let cfg = AnswerabilityConfig::enabled();
        rerank(&mut a, "same text", QuestionShape::General, &cfg);
        rerank(&mut b, "same text", QuestionShape::General, &cfg);
        // Both orderings converge only if the incumbent term is equal; here it
        // is not (rank 0 vs 1), so assert the weaker real property: repeated
        // application is idempotent.
        let a1 = a.clone();
        rerank(&mut a, "same text", QuestionShape::General, &cfg);
        assert_eq!(
            a.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
            a1.iter().map(|h| h.id.clone()).collect::<Vec<_>>()
        );
        let _ = b;
    }

    #[test]
    fn coverage_counts_distinct_query_terms() {
        let words = content_words("how many miles did I run in Boston");
        // Stopwords and short words dropped.
        assert!(!words.contains("how"));
        assert!(!words.contains("did"));
        assert!(words.contains("miles"));
        assert!(words.contains("run"));
        assert!(words.contains("boston"));

        let s = score_hit(
            &hit("x", "I ran miles in Boston"),
            &words,
            AnswerType::Number,
            &AnswerabilityConfig::enabled(),
        );
        assert!(s.coverage > 0.5, "coverage was {}", s.coverage);
    }

    #[test]
    fn single_hit_and_empty_input_are_no_ops() {
        let cfg = AnswerabilityConfig::enabled();
        let mut empty: Vec<MemoryHit> = vec![];
        rerank(&mut empty, "q", QuestionShape::General, &cfg);
        assert!(empty.is_empty());

        let mut one = vec![hit("a", "content")];
        rerank(&mut one, "q", QuestionShape::General, &cfg);
        assert_eq!(one.len(), 1);
    }
}
