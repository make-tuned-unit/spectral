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

use std::sync::OnceLock;

use regex::Regex;

use spectral_graph::cascade_layers::CascadePipelineConfig;

/// Define a compile-once accessor for a static classifier pattern.
///
/// [`QuestionShape::classify`] runs on every routed question, and every arm it
/// falls through compiles another pattern. Building them per call re-introduced
/// exactly the read-path defect measured and fixed for `tact::classifier` and
/// `spectrogram::dimensions` in
/// `docs/internal/read-path-regex-cache-2026-07-25.md` — this policy module was
/// migrated in from the benchmark harness afterwards and did not inherit the
/// fix. `unwrap` is retained (the patterns are static literals, so a failure to
/// compile is a build-time authoring bug) but now happens at most once.
macro_rules! classifier_pattern {
    ($(#[$meta:meta])* $name:ident, $pattern:expr) => {
        $(#[$meta])*
        fn $name() -> &'static Regex {
            static CELL: OnceLock<Regex> = OnceLock::new();
            CELL.get_or_init(|| Regex::new($pattern).unwrap())
        }
    };
}

classifier_pattern!(
    /// Temporal-counting ("how many days ago", "how old") — checked before
    /// general counting so date arithmetic wins over tallying.
    re_temporal_counting,
    r"how many (?:days|weeks|months|years) (?:ago|since|passed|before|after|between|had passed|have passed|did it take)|how old"
);
classifier_pattern!(re_counting, r"how many|how much|total|in total|altogether");
classifier_pattern!(
    /// Recency sub-gate for Counting.
    re_counting_recency,
    r"\b(currently|right now|most recent|latest|newest|do i still|now)\b"
);
classifier_pattern!(re_where, r"^where\b");
classifier_pattern!(
    /// Recency sub-gate for `where`. Deliberately NOT the same pattern as
    /// [`re_factual_recency`]: this arm also admits bare `recent`.
    re_where_recency,
    r"\b(currently|right now|most recent|latest|newest|do i still|now|recent)\b"
);
classifier_pattern!(
    re_temporal,
    r"when did|how long|(?:^|\W)first\b|(?:^|\W)last\b|before|after|ago|since|order.+(?:earliest|latest)|from earliest|chronological|(?:^|\W)order of\b"
);
classifier_pattern!(re_factual, r"^(?:what|where|who|which)\b");
classifier_pattern!(
    /// Recency sub-gate for Factual. Deliberately NOT the same pattern as
    /// [`re_where_recency`]: this arm also admits `most recently`.
    re_factual_recency,
    r"\b(currently|right now|most recent|most recently|latest|newest|do i still|now)\b"
);
classifier_pattern!(
    re_general_preference,
    r"\b(suggest|recommend|tips?|advice|recommendations?|what should i)\b"
);
classifier_pattern!(
    re_general_preference_any,
    r"\bany (tips?|advice|suggestions?|ideas?|thoughts?|recommendations?)\b"
);
classifier_pattern!(
    /// V2 recency sub-gate for Counting: adds bare `current`.
    re_counting_recency_v2,
    r"\b(current|currently|right now|most recent|latest|newest|do i still|now)\b"
);
classifier_pattern!(
    /// V2 recency sub-gate for `where`: adds bare `current`.
    re_where_recency_v2,
    r"\b(current|currently|right now|most recent|latest|newest|do i still|now|recent)\b"
);
classifier_pattern!(
    /// V2 recency sub-gate for Factual: adds bare `current`.
    re_factual_recency_v2,
    r"\b(current|currently|right now|most recent|most recently|latest|newest|do i still|now)\b"
);
classifier_pattern!(
    re_general_recall,
    r"\b(remind me|going back to|previous|earlier conversation|we (discussed|talked about)|can you remind me)\b"
);

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
    /// V1 with two classifier defects repaired. **Unproven** — see
    /// `docs/internal/policy-v2-prereg-2026-08-02.md`. Not the default.
    ///
    /// 1. The recency sub-gates admit bare `current`, not only `currently`.
    ///    `FactualCurrentState` is documented as *"What is my current X"*, and
    ///    that exact phrasing did not match it.
    /// 2. The `GeneralPreference` gates are checked **before** the Factual
    ///    branch. In V1 they sit after `^(?:what|where|who|which)`, so
    ///    `what should i` — an alternative listed in the preference pattern
    ///    itself — is unreachable for any question starting with "what".
    V2Fixed,
}

impl RetrievalPolicyVersion {
    /// Stable string form for receipts, logs, and published results.
    pub fn as_str(&self) -> &'static str {
        match self {
            RetrievalPolicyVersion::V1 => "retrieval-v1",
            RetrievalPolicyVersion::V2Fixed => "retrieval-v2-fixed",
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
        Self::classify_with(question, RetrievalPolicyVersion::V1)
    }

    /// Classify under an explicit policy version.
    ///
    /// [`RetrievalPolicyVersion::V1`] is byte-for-byte the routing behind the
    /// published number and must never change. `V2Fixed` repairs two defects
    /// found by the classifier pinning corpus; it is unproven and not the
    /// default.
    pub fn classify_with(question: &str, version: RetrievalPolicyVersion) -> Self {
        let v2 = matches!(version, RetrievalPolicyVersion::V2Fixed);
        let q = question.to_lowercase();

        // V2 defect 2: the preference gates are unreachable behind the Factual
        // branch for any question starting with "what", which is most of them.
        if v2 && (re_general_preference().is_match(&q) || re_general_preference_any().is_match(&q))
        {
            return Self::GeneralPreference;
        }

        // ── Level 1: top-level shape ──

        // Temporal-counting ("how many days/weeks ago", "how old") → Temporal
        if re_temporal_counting().is_match(&q) {
            return Self::Temporal;
        }

        // General counting → Counting (with sub-gate)
        if re_counting().is_match(&q) {
            // Level 2: recency sub-gate for Counting
            let recency = if v2 {
                re_counting_recency_v2()
            } else {
                re_counting_recency()
            };
            if recency.is_match(&q) {
                return Self::CountingCurrentState;
            }
            return Self::Counting;
        }

        // Location questions: "where" → Factual, even with temporal modifiers.
        // Temporal modifiers in "where" questions provide context, not focus.
        if re_where().is_match(&q) {
            let recency = if v2 {
                re_where_recency_v2()
            } else {
                re_where_recency()
            };
            if recency.is_match(&q) {
                return Self::FactualCurrentState;
            }
            return Self::Factual;
        }

        // Temporal — includes explicit ordering phrases ("order ... earliest/latest")
        if re_temporal().is_match(&q) {
            return Self::Temporal;
        }

        // Factual (with sub-gate)
        if re_factual().is_match(&q) {
            // Level 2: recency sub-gate for Factual
            let recency = if v2 {
                re_factual_recency_v2()
            } else {
                re_factual_recency()
            };
            if recency.is_match(&q) {
                return Self::FactualCurrentState;
            }
            return Self::Factual;
        }

        // ── Level 2: General sub-gates ──

        if re_general_preference().is_match(&q) {
            return Self::GeneralPreference;
        }
        if re_general_preference_any().is_match(&q) {
            return Self::GeneralPreference;
        }
        if re_general_recall().is_match(&q) {
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

    /// Behaviour pin for the classifier. Written against the pre-cache
    /// implementation (11 `Regex::new` calls per invocation) so the
    /// compile-once refactor is provably behaviour-preserving: every arm of
    /// the two-level classifier is exercised, including the near-duplicate
    /// recency sub-gates, which differ between the `where` branch (`recent`)
    /// and the general factual branch (`most recently`).
    #[test]
    fn classification_corpus_is_pinned() {
        use QuestionShape::*;
        let cases: &[(&str, QuestionShape)] = &[
            // Temporal-counting pre-empts general counting.
            ("How many days ago did I book the flight?", Temporal),
            ("how many weeks since the last deploy", Temporal),
            ("How old is my nephew?", Temporal),
            // General counting, with and without the recency sub-gate.
            ("How many times did I go running last month?", Counting),
            ("how much did I spend altogether", Counting),
            ("How many pets do I currently have?", CountingCurrentState),
            ("how many books do i still own", CountingCurrentState),
            // `where` routes to Factual even with temporal modifiers, and has
            // its own recency gate that (unlike Factual's) includes `recent`.
            ("Where did I park last week?", Factual),
            ("Where do I currently live?", FactualCurrentState),
            ("Where was my most recent trip?", FactualCurrentState),
            // Temporal.
            ("When did I start the new job?", Temporal),
            ("How long was the drive?", Temporal),
            ("What happened before the merger?", Temporal),
            ("List the events in chronological order", Temporal),
            // Factual, with the recency sub-gate.
            ("What is my sister's name?", Factual),
            ("Who is the team lead?", Factual),
            // NOTE: bare "current" does NOT hit the recency sub-gate — the
            // pattern only lists "currently". See
            // `bare_current_misses_the_recency_sub_gate` below.
            ("What is my current job title?", Factual),
            ("What is my job title currently?", FactualCurrentState),
            ("What did I most recently order?", FactualCurrentState),
            // General sub-gates.
            ("Any tips for staying focused?", GeneralPreference),
            ("Can you recommend a good restaurant?", GeneralPreference),
            // NOTE: leading "what" is captured by the Factual branch first, so
            // the `what should i` alternative never fires. See
            // `what_should_i_is_shadowed_by_the_factual_branch` below.
            ("What should I get my mum for her birthday?", Factual),
            (
                "I should get my mum something — any ideas?",
                GeneralPreference,
            ),
            (
                "Remind me what we discussed about the budget",
                GeneralRecall,
            ),
            ("Going back to the roadmap conversation", GeneralRecall),
            // Catch-all.
            ("Tell me about my hiking trip", General),
            ("I need help planning dinner", General),
        ];

        for (question, expected) in cases {
            assert_eq!(
                QuestionShape::classify(question),
                *expected,
                "classification changed for {question:?}"
            );
        }
    }

    /// KNOWN GAP, pinned deliberately rather than fixed.
    ///
    /// `FactualCurrentState` is documented as *"What is my current X" —
    /// most-recent-wins factual*, but the sub-gate pattern lists `currently`,
    /// not `current`, so the exact phrasing in the doc comment falls through
    /// to plain `Factual` and loses the recency priority.
    ///
    /// This is **not** fixed here: the routing decision is load-bearing on the
    /// published 81.5% LongMemEval-S number, so widening the gate has to be
    /// pre-registered and measured on the $0 retrieval oracle like any other
    /// retrieval lever. Filed as a Phase 3 candidate. This test exists so the
    /// gap cannot be closed *accidentally* by an unrelated refactor.
    #[test]
    fn bare_current_misses_the_recency_sub_gate() {
        assert_eq!(
            QuestionShape::classify("What is my current employer?"),
            QuestionShape::Factual,
            "gate widened without a measured prereg — see doc comment"
        );
        assert_eq!(
            QuestionShape::classify("How many pets do I current have?"),
            QuestionShape::Counting,
        );
    }

    /// KNOWN GAP, pinned deliberately rather than fixed.
    ///
    /// The `GeneralPreference` gate lists `what should i`, but it is checked
    /// *after* the Factual branch `^(?:what|where|who|which)\b`, so any
    /// question beginning with "what" — i.e. every natural phrasing of it —
    /// routes to `Factual` and the alternative is dead.
    ///
    /// Not fixed here for the same reason as
    /// [`bare_current_misses_the_recency_sub_gate`]: reordering the branches
    /// changes routing on the published benchmark and needs its own prereg.
    /// `single-session-preference` is the weakest measured category (56.0%),
    /// which makes this a strong Phase 3 candidate.
    #[test]
    fn what_should_i_is_shadowed_by_the_factual_branch() {
        assert_eq!(
            QuestionShape::classify("What should I cook tonight?"),
            QuestionShape::Factual,
            "branch order changed without a measured prereg — see doc comment"
        );
    }

    #[test]
    fn v2_repairs_the_bare_current_gate() {
        use RetrievalPolicyVersion::V2Fixed;
        for q in [
            "What is my current employer?",
            "What is my current job title?",
            "Where is my current office?",
        ] {
            assert_eq!(
                QuestionShape::classify_with(q, V2Fixed),
                QuestionShape::FactualCurrentState,
                "V2 should route {q:?} to the recency variant"
            );
        }
        assert_eq!(
            QuestionShape::classify_with("How many pets do I current have?", V2Fixed),
            QuestionShape::CountingCurrentState
        );
    }

    #[test]
    fn v2_unshadows_the_preference_gate() {
        use RetrievalPolicyVersion::V2Fixed;
        for q in [
            "What should I cook tonight?",
            "What should I get my mum for her birthday?",
        ] {
            assert_eq!(
                QuestionShape::classify_with(q, V2Fixed),
                QuestionShape::GeneralPreference,
                "V2 should reach the preference gate for {q:?}"
            );
        }
    }

    /// V2 must change **only** the two defects. Everything else in the pinned
    /// corpus has to route identically, or this is a rewrite rather than a fix
    /// and the measured comparison would be uninterpretable.
    #[test]
    fn v2_changes_nothing_else_in_the_pinned_corpus() {
        use RetrievalPolicyVersion::V2Fixed;
        let unaffected = [
            "How many days ago did I book the flight?",
            "How many times did I go running last month?",
            "How many pets do I currently have?",
            "Where did I park last week?",
            "Where do I currently live?",
            "When did I start the new job?",
            "How long was the drive?",
            "What is my sister's name?",
            "Who is the team lead?",
            "What did I most recently order?",
            "Remind me what we discussed about the budget",
            "Tell me about my hiking trip",
        ];
        for q in unaffected {
            assert_eq!(
                QuestionShape::classify_with(q, V2Fixed),
                QuestionShape::classify(q),
                "V2 changed routing for an unaffected question: {q:?}"
            );
        }
    }

    #[test]
    fn v1_is_the_default_and_is_unchanged_by_the_v2_work() {
        assert_eq!(
            RetrievalPolicyVersion::default(),
            RetrievalPolicyVersion::V1
        );
        assert_eq!(RetrievalPolicyVersion::V1.as_str(), "retrieval-v1");
        assert_eq!(
            QuestionShape::classify("What is my current employer?"),
            QuestionShape::classify_with(
                "What is my current employer?",
                RetrievalPolicyVersion::V1
            )
        );
    }

    /// The classifier lowercases internally; casing must not change routing.
    #[test]
    fn classification_is_case_insensitive() {
        assert_eq!(
            QuestionShape::classify("WHERE DO I CURRENTLY LIVE?"),
            QuestionShape::classify("where do i currently live?")
        );
    }
}
