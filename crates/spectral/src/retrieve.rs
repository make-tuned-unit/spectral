//! The unified retrieval pipeline — one call from question to actor context.
//!
//! # Why this exists
//!
//! `Brain` exposes about fifteen retrieval entry points (`recall`, `recall_at`,
//! `recall_local`, `recall_topk_fts`, `recall_cascade{,_scoped,_with_pipeline}`,
//! `recall_graph`, `recall_cross_wing{,_with}`, `tact_retrieve_with_k`,
//! `cascade_retrieve`, `fts_search_direct`, `probe`, `probe_recent`, `turn`).
//! Each composes its own reranking. `recall_topk_fts` gets fetch-pool widening,
//! entity clustering, context dedup and a time anchor; `recall_at` — which
//! `recall_local` calls, and which is the obvious entry point for a new
//! consumer — gets none of them.
//!
//! The practical consequence is that improvements do not compound: a lever
//! landed in one path silently leaves the others where they were, and every
//! measurement describes one route rather than the product. Reproducing the
//! published configuration required a consumer to assemble the policy, pick the
//! matching `Brain` method, and then render the result the same way the
//! benchmark harness does — three separate things to get right, none of them
//! obvious.
//!
//! [`retrieve`] is the single path: **plan → candidates → rerank → truncate →
//! render**. [`RetrievePlan::v1`] is the published configuration, executable in
//! one call.
//!
//! # Relationship to the existing entry points
//!
//! Additive. Nothing is deprecated and no existing method changes behaviour.
//! The legacy entry points remain for callers who want a specific stage.
//!
//! # Determinism
//!
//! Every stage is deterministic. The only clock read is the caller-supplied
//! time anchor; when `RenderOptions::question_date` is set it also anchors
//! recency, so a replay measures distance from the query's temporal context
//! rather than from wall-clock.

use spectral_ingest::MemoryHit;

use crate::answerability::AnswerabilityConfig;
use crate::policy::{QuestionShape, RetrievalRoute};
use crate::render::{self, RenderOptions};
use crate::supersession::SupersessionConfig;
use crate::{Brain, Error, RecallTopKConfig, RecognitionContext, Visibility};

use spectral_graph::cascade_layers::CascadePipelineConfig;

/// Minimum k for the top-k FTS route.
///
/// Floors CLI/config overrides that would otherwise cut temporal evidence
/// ranking at positions 21–40 after reranking. Mirrors the harness constant.
pub const TOPK_MIN_K: usize = 40;

/// Candidate-pool widening applied when a rerank lever is active.
///
/// Widening must reach the *retrieval* config, not slice a wider window off the
/// results: `run_cascade_pipeline_scoped` truncates to `config.k`, so a
/// post-hoc `take(k * widen)` is a no-op — the defect that made two
/// measurements meaningless (see
/// `docs/internal/answerability-result-run2-2026-08-02.md`).
pub const RERANK_POOL_WIDEN: usize = 2;

/// Everything that determines what a question retrieves and how it renders.
#[derive(Debug, Clone)]
pub struct RetrievePlan<'a> {
    /// Question shape, normally from [`QuestionShape::classify`].
    pub shape: QuestionShape,
    /// Which retrieval route runs.
    pub route: RetrievalRoute,
    /// Config for the cascade route.
    pub cascade: CascadePipelineConfig,
    /// Config for the top-k FTS route.
    pub topk: RecallTopKConfig,
    /// Ambient context (recency anchor, focus wing, session).
    pub context: RecognitionContext,
    /// Visibility boundary. No default — an omitted boundary is a leak.
    pub visibility: Visibility,
    /// Query-conditioned rerank. Off by default; a measured null on the $0
    /// oracle (`docs/MEASURED_RECORD.md`).
    pub answerability: AnswerabilityConfig,
    /// Read-time suppression of superseded facts. Off by default; unproven.
    pub supersession: SupersessionConfig,
    /// How the result renders into actor context.
    pub render: RenderOptions<'a>,
}

impl<'a> RetrievePlan<'a> {
    /// The published LongMemEval-S configuration for a question.
    ///
    /// Classifies the question, takes the per-shape route and cascade profile
    /// from [`crate::policy`] (`RetrievalPolicyVersion::V1`), and renders with
    /// [`RenderOptions::published`]. This is the configuration behind the
    /// published accuracy number, executable by a consumer in one call.
    pub fn v1(question: &str, visibility: Visibility) -> Self {
        let shape = QuestionShape::classify(question);
        let cascade = shape.cascade_profile();
        Self {
            shape,
            route: shape.retrieval_route(),
            topk: RecallTopKConfig {
                k: cascade.k.max(TOPK_MIN_K),
                ..RecallTopKConfig::default()
            },
            cascade,
            context: RecognitionContext::empty(),
            visibility,
            answerability: AnswerabilityConfig::default(),
            supersession: SupersessionConfig::default(),
            render: RenderOptions::published(),
        }
    }

    /// Replace the render options.
    pub fn with_render(mut self, render: RenderOptions<'a>) -> Self {
        self.render = render;
        self
    }

    /// Enable the query-conditioned answerability rerank.
    ///
    /// Measured as a null on the $0 retrieval oracle and **not** recommended;
    /// exposed so the result is reproducible and the lever re-testable.
    pub fn with_answerability(mut self, config: AnswerabilityConfig) -> Self {
        self.answerability = config;
        self
    }

    /// Enable read-time supersession suppression.
    ///
    /// Unproven: the $0 oracle can only measure whether this *harms*
    /// retrieval, because the benefit — the actor not having to pick between
    /// a stale and a current version of the same fact — is actor-side.
    pub fn with_supersession(mut self, config: SupersessionConfig) -> Self {
        self.supersession = config;
        self
    }

    /// Override the ambient context (and therefore the recency anchor).
    pub fn with_context(mut self, context: RecognitionContext) -> Self {
        self.context = context;
        self
    }

    /// Anchor recency to a specific time on **both** routes.
    ///
    /// Without this, recency decay measures distance from wall-clock. That is
    /// right for a live query and silently wrong for historical replay or
    /// time-travel — a whole class of quiet bug, because the result still
    /// looks plausible. Sets the cascade context's `now` and the top-k
    /// config's `now` together, so the anchor cannot be applied to one route
    /// and forgotten on the other.
    pub fn with_time_anchor(mut self, now: chrono::DateTime<chrono::Utc>) -> Self {
        self.context = self.context.with_now(now);
        self.topk.now = Some(now);
        self
    }

    /// The published plan, anchored to the brain's newest memory.
    ///
    /// `RetrievePlan::v1` leaves the anchor unset, so recency decay measures
    /// distance from wall-clock. Measured: that does **not** change recall
    /// ordering (the decay is multiplicative on the top-k/cascade path, and
    /// `recall_at` never re-sorts after decaying), but it does change the
    /// decayed `signal_score` values a caller reads, and it is simply wrong
    /// for historical replay where recency should be measured from the query's
    /// own date.
    ///
    /// Use this for replay, audit, and regression tests — and note it also
    /// pins the ordering property, which today holds by construction rather
    /// than by design. See `docs/internal/decay-time-invariance-2026-08-03.md`.
    ///
    /// Falls back to `v1` (wall-clock) when the brain has no timestamped
    /// memories.
    pub fn reproducible(
        brain: &Brain,
        question: &str,
        visibility: Visibility,
    ) -> Result<Self, Error> {
        let plan = Self::v1(question, visibility);
        Ok(match brain.latest_interaction_time()? {
            Some(anchor) => plan.with_time_anchor(anchor),
            None => plan,
        })
    }

    /// The output size this plan's route produces.
    fn output_k(&self) -> usize {
        match self.route {
            RetrievalRoute::TopkFts => self.topk.k,
            _ => self.cascade.k,
        }
    }

    /// Pool multiplier: widen when a stage can use the extra candidates.
    ///
    /// Supersession widens too, so suppressed slots are backfilled rather than
    /// shrinking the output. Without that, suppression is a pure loss on any
    /// set-recall metric even when it improves what the actor reads.
    fn widen(&self) -> usize {
        if self.answerability.enabled || self.supersession.enabled {
            RERANK_POOL_WIDEN
        } else {
            1
        }
    }
}

/// What a retrieval produced, at every stage a caller might need.
#[derive(Debug, Clone)]
pub struct Retrieved {
    /// Hits in final rank order, truncated to the plan's output size.
    pub hits: Vec<MemoryHit>,
    /// Rendered actor context, one line per session header or turn.
    pub lines: Vec<String>,
    /// The shape the question classified as.
    pub shape: QuestionShape,
    /// The route that ran.
    pub route: RetrievalRoute,
    /// Candidates considered before truncation. Equals `hits.len()` unless a
    /// stage widened the pool.
    pub candidates_considered: usize,
    /// `(suppressed_key, superseded_by_key)` for each fact dropped as stale.
    /// Empty unless supersession is enabled. Kept so the decision is auditable
    /// rather than silent.
    pub superseded: Vec<(String, String)>,
}

impl Retrieved {
    /// The rendered context as one block of text.
    pub fn context_block(&self) -> String {
        self.lines.join("\n")
    }
}

/// Run the pipeline: plan → candidates → rerank → truncate → render.
///
/// ```no_run
/// use spectral::{Brain, Visibility};
/// use spectral::retrieve::{retrieve, RetrievePlan};
///
/// let brain = Brain::open("./my-brain")?;
/// let question = "what did I decide about auth";
/// let plan = RetrievePlan::v1(question, Visibility::Private);
/// let result = retrieve(&brain, question, &plan)?;
/// println!("{}", result.context_block());
/// # Ok::<(), spectral::Error>(())
/// ```
pub fn retrieve(brain: &Brain, query: &str, plan: &RetrievePlan<'_>) -> Result<Retrieved, Error> {
    let output_k = plan.output_k();
    let widen = plan.widen();

    // ── Stage 1: candidates ────────────────────────────────────────
    let mut hits: Vec<MemoryHit> = match plan.route {
        RetrievalRoute::TopkFts => {
            let cfg = RecallTopKConfig {
                k: plan.topk.k * widen,
                ..plan.topk.clone()
            };
            brain.recall_topk_fts(query, &cfg, plan.visibility)?
        }
        RetrievalRoute::Graph | RetrievalRoute::Tact | RetrievalRoute::Cascade => {
            // Widening must reach the pipeline config — see RERANK_POOL_WIDEN.
            let cfg = CascadePipelineConfig {
                k: plan.cascade.k * widen,
                ..plan.cascade.clone()
            };
            brain
                .recall_cascade_scoped(query, &plan.context, &cfg, plan.visibility)?
                .merged_hits
        }
    };
    let candidates_considered = hits.len();

    // ── Stage 2: query-conditioned rerank (opt-in) ─────────────────
    if plan.answerability.enabled {
        crate::answerability::rerank(&mut hits, query, plan.shape, &plan.answerability);
    }

    // ── Stage 3: suppress superseded facts (opt-in) ────────────────
    //
    // Before truncation, so the freed slots are backfilled from the widened
    // pool. Suppression never deletes: `partition` returns both halves and the
    // memories themselves are untouched.
    let mut superseded = Vec::new();
    if plan.supersession.enabled {
        let report = crate::supersession::partition(&hits, &plan.supersession);
        superseded = report
            .suppressed
            .iter()
            .map(|(h, by)| (h.key.clone(), by.clone()))
            .collect();
        hits = report.kept;
    }

    // ── Stage 4: truncate to the shape's output size ───────────────
    hits.truncate(output_k);

    // ── Stage 5: render ────────────────────────────────────────────
    let lines = render::session_grouped(&hits, &plan.render);

    Ok(Retrieved {
        hits,
        lines,
        shape: plan.shape,
        route: plan.route,
        candidates_considered,
        superseded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn brain_with_turns() -> (TempDir, Brain) {
        let tmp = TempDir::new().unwrap();
        let brain = Brain::open(tmp.path()).unwrap();
        for (k, c) in [
            (
                "s1:turn:0:user",
                "I switched my laptop to the framework 13 for repairability",
            ),
            ("s1:turn:1:assistant", "Got it."),
            (
                "s2:turn:0:user",
                "the framework laptop battery life is disappointing",
            ),
            ("s2:turn:1:user", "I ran 5 miles on Tuesday near the office"),
        ] {
            brain.remember(k, c, Visibility::Private).unwrap();
        }
        (tmp, brain)
    }

    #[test]
    fn v1_plan_matches_the_published_policy() {
        let q = "when did I switch laptops";
        let plan = RetrievePlan::v1(q, Visibility::Private);
        assert_eq!(plan.shape, QuestionShape::classify(q));
        assert_eq!(plan.route, plan.shape.retrieval_route());
        assert_eq!(plan.cascade.k, plan.shape.cascade_profile().k);
        // Render defaults to the published configuration.
        assert_eq!(
            plan.render.session_order,
            crate::render::SessionOrder::Chronological
        );
        assert!(
            !plan.answerability.enabled,
            "answerability must default off"
        );
    }

    #[test]
    fn topk_route_floors_k_at_the_harness_value() {
        // Temporal routes to TopkFts; its profile k is 40 but a smaller
        // profile must still floor at TOPK_MIN_K.
        let plan = RetrievePlan::v1("what is my current phone", Visibility::Private);
        assert!(plan.topk.k >= TOPK_MIN_K);
    }

    #[test]
    fn retrieve_produces_hits_and_rendered_lines() {
        let (_tmp, brain) = brain_with_turns();
        let q = "framework laptop";
        let plan = RetrievePlan::v1(q, Visibility::Private);
        let out = retrieve(&brain, q, &plan).unwrap();
        assert!(!out.hits.is_empty());
        assert!(!out.lines.is_empty());
        assert!(out.lines.iter().any(|l| l.starts_with("--- Session")));
        assert!(out.context_block().contains("--- Session"));
    }

    #[test]
    fn output_never_exceeds_the_shape_k_even_when_widened() {
        let (_tmp, brain) = brain_with_turns();
        let q = "how many miles did I run";
        let mut plan = RetrievePlan::v1(q, Visibility::Private);
        plan.cascade.k = 2;
        plan.topk.k = 2;
        let plan = plan.with_answerability(AnswerabilityConfig::enabled());
        let out = retrieve(&brain, q, &plan).unwrap();
        assert!(out.hits.len() <= 2, "got {}", out.hits.len());
    }

    #[test]
    fn widening_only_happens_when_a_rerank_can_use_it() {
        let plan = RetrievePlan::v1("q", Visibility::Private);
        assert_eq!(plan.widen(), 1, "no rerank should mean no widening");
        let widened = plan.with_answerability(AnswerabilityConfig::enabled());
        assert_eq!(widened.widen(), RERANK_POOL_WIDEN);
    }

    #[test]
    fn retrieval_is_deterministic() {
        let (_tmp, brain) = brain_with_turns();
        let q = "framework laptop battery";
        let plan = RetrievePlan::v1(q, Visibility::Private);
        let first = retrieve(&brain, q, &plan).unwrap();
        for _ in 0..3 {
            let again = retrieve(&brain, q, &plan).unwrap();
            assert_eq!(again.lines, first.lines);
            assert_eq!(
                again.hits.iter().map(|h| h.key.clone()).collect::<Vec<_>>(),
                first.hits.iter().map(|h| h.key.clone()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn supersession_defaults_off_and_reports_nothing() {
        let (_tmp, brain) = brain_with_turns();
        let q = "framework laptop";
        let plan = RetrievePlan::v1(q, Visibility::Private);
        assert!(!plan.supersession.enabled);
        let out = retrieve(&brain, q, &plan).unwrap();
        assert!(out.superseded.is_empty());
    }

    #[test]
    fn supersession_widens_the_pool_so_output_size_is_preserved() {
        let plan = RetrievePlan::v1("q", Visibility::Private);
        assert_eq!(plan.widen(), 1);
        let with = plan.with_supersession(crate::supersession::SupersessionConfig::enabled());
        assert_eq!(
            with.widen(),
            RERANK_POOL_WIDEN,
            "suppressed slots must be backfillable"
        );
    }

    #[test]
    fn superseded_facts_are_dropped_and_reported() {
        let tmp = TempDir::new().unwrap();
        let brain = Brain::open(tmp.path()).unwrap();
        brain
            .remember_with(
                "s1:turn:0:user",
                "my note-taking app is Notion and I use it daily",
                crate::RememberOpts {
                    visibility: Visibility::Private,
                    created_at: Some(chrono::Utc::now() - chrono::Duration::days(200)),
                    episode_id: Some("s1".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        brain
            .remember_with(
                "s2:turn:0:user",
                "my note-taking app is Obsidian since the migration",
                crate::RememberOpts {
                    visibility: Visibility::Private,
                    created_at: Some(chrono::Utc::now() - chrono::Duration::days(5)),
                    episode_id: Some("s2".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();

        let q = "note-taking app";
        let plan = RetrievePlan::v1(q, Visibility::Private)
            .with_supersession(crate::supersession::SupersessionConfig::enabled());
        let out = retrieve(&brain, q, &plan).unwrap();

        assert_eq!(out.superseded.len(), 1, "{:?}", out.superseded);
        assert_eq!(out.superseded[0].0, "s1:turn:0:user");
        assert_eq!(out.superseded[0].1, "s2:turn:0:user");
        assert!(out.hits.iter().all(|h| h.key != "s1:turn:0:user"));
    }

    #[test]
    fn visibility_is_required_and_honoured() {
        let (_tmp, brain) = brain_with_turns();
        let q = "framework laptop";
        // Public context admits only public-labelled content; these memories
        // are Private, so a stricter boundary must not return them.
        let mut plan = RetrievePlan::v1(q, Visibility::Public);
        plan.visibility = Visibility::Public;
        let out = retrieve(&brain, q, &plan).unwrap();
        assert!(
            out.hits.is_empty(),
            "private content leaked into a public context: {:?}",
            out.hits.iter().map(|h| &h.key).collect::<Vec<_>>()
        );
    }
}
