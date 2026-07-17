//! Federation v1 — in-process, read-time, single-tenant fan-out.
//!
//! A [`FederationCoordinator`] ("Henry") holds N child [`Brain`] handles and,
//! on each query, calls [`Brain::recall_cascade`] on every child, tags each
//! returned hit with its origin brain coordinator-side, and merges the N
//! result sets into one provenance-ranked list.
//!
//! # Design (per the federation + fan-out feasibility audits)
//!
//! Everything here lives **above the `Brain` API** — no core changes:
//! - `recall_*` are `&self`, so fanning out over N held handles needs no
//!   mutation and is trivially parallelizable (v1 runs sequentially; see
//!   [`FederationCoordinator::fan_out_recall`]).
//! - Distinct brains on distinct `data_dir`s coexist in one process with no
//!   global/static state (confirmed empirically by the `n_brains_coresident`
//!   diagnostic on ubuntu-latest, glibc 2.39+).
//! - [`MemoryHit`] carries **no** brain id, so provenance is owned here: the
//!   coordinator stamps each hit with its source [`BrainId`] via
//!   [`Brain::brain_id`].
//!
//! # Read-time semantics
//!
//! The coordinator holds no result cache. Every fan-out re-reads live child
//! state, so child-side deletions and edits propagate to the next query for
//! free — this is the design's premise, not an added feature.
//!
//! # Out of scope for v1
//!
//! Write-merge into a master brain, cross-machine / IPC transport,
//! multi-tenant isolation, and mandatory content-hash dedup are all out of
//! scope. `Brain::consolidate_into` stays an intra-brain operation. A dedup
//! seam is documented in [`FederationCoordinator::fan_out_recall`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use spectral_cascade::RecognitionContext;
use spectral_core::identity::BrainId;
use spectral_core::visibility::Visibility;
use spectral_ingest::MemoryHit;

use crate::brain::Brain;
use crate::cascade_layers::CascadePipelineConfig;
use crate::error::Error;

/// Default provenance weight applied to a child's hits: rank purely on the
/// child's own `signal_score`.
pub const DEFAULT_BRAIN_WEIGHT: f64 = 1.0;

/// Default Reciprocal Rank Fusion constant. 60 is the value from Cormack et
/// al. (2009), the de-facto standard used across search/RAG systems.
pub const DEFAULT_RRF_K: f64 = 60.0;

/// Default per-child contribution cap. Bounds any single member's footprint in
/// the merged result so a flooding member cannot crowd out the others, while
/// staying generous enough not to clip normal recall (each child returns at most
/// the query's `k`, and the top ~20 by relevance are what a merged view shows).
/// Set [`MergePolicy::per_child_cap`] to `None` for an uncapped, fully-trusted
/// federation.
pub const DEFAULT_PER_CHILD_CAP: usize = 20;

/// How the coordinator fuses each child's ranked results into one list.
#[derive(Debug, Clone, PartialEq)]
pub enum FusionMethod {
    /// **Reciprocal Rank Fusion** (the field-standard rank-fusion method,
    /// Cormack et al. 2009) — the default. Each child contributes
    /// `weight / (k + rank)` for every result, summed across children by
    /// content identity. Because it ranks on *position*, not the member's
    /// self-asserted `signal_score`, it is immune to the score-inflation
    /// poisoning attack; because contributions *sum across* children, content
    /// independently returned by multiple members naturally outranks a lone
    /// assertion (corroboration for free). One widely-accepted primitive
    /// subsumes both the anti-flooding and corroboration guarantees.
    Rrf {
        /// The RRF `k` constant (default [`DEFAULT_RRF_K`]). Larger `k`
        /// flattens the contribution of top ranks.
        k: f64,
    },
    /// Rank purely on the member's raw `signal_score × weight`. This is the
    /// legacy behavior and is **vulnerable** to a member self-asserting max
    /// scores to dominate the merge — kept only for reproducing v1 results
    /// and for the poisoning benchmark's "undefended" arm.
    RawScore,
}

/// How the coordinator merges and trusts hits across children.
///
/// The threat this addresses (verified in the 2026 memory-poisoning
/// literature, see docs/internal/federation-fundamentals-2026-07-10.md): a
/// member fully controls its own memories' `signal_score`, timestamps, and
/// content, so ranking a merged fan-out on raw `signal_score × weight` lets a
/// single malicious peer flood the top of every result by self-asserting
/// score-1.0, keyword-stuffed memories. RRF fusion removes that cheap attack
/// and rewards cross-member agreement; the `per_child_cap` bounds flooding
/// volume. None of this stops a *signed* insider — that is what signed
/// provenance ([`Brain::verify_hit`](crate::brain::Brain::verify_hit)) is for.
///
/// # Residual attacks (know these before federating untrusted members)
///
/// RRF neutralizes the *score-inflation* flood, and the defaults below bound the
/// remaining flooding/tiebreak vectors. What remains needs deployment trust:
/// - **Distinct-content flooding — bounded by default.** RRF's per-origin dedup
///   only collapses identical content, so a member returning many *distinct*
///   items could occupy many merged slots. [`per_child_cap`](Self::per_child_cap)
///   defaults to [`DEFAULT_PER_CHILD_CAP`] (20), capping any member's footprint;
///   set it to `None` for a fully-trusted federation that wants every hit.
/// - **Tiebreak grinding — mitigated.** Ties are broken by a *content* hash
///   first (not the member-chosen `BrainId`), so a member cannot grind one low
///   `BrainId` to win every tie across the federation; it would have to grind
///   each item's content (which changes the content). `BrainId`/`hit.id` remain
///   only as final total-order tiebreakers.
/// - **Sybil-forged corroboration — deployment trust.** Cross-member agreement
///   lifts honest content, but [`add_brain`](FederationCoordinator::add_brain)
///   authenticates no identity, so K colluding brains could manufacture
///   corroboration. This is not defensible at the merge layer — only federate
///   members whose brains you trust to be distinct principals. (Signed
///   provenance via [`Brain::verify_hit`](crate::brain::Brain::verify_hit) is
///   the intended future basis for authenticated corroboration.)
#[derive(Debug, Clone)]
pub struct MergePolicy {
    /// Rank-fusion method across children. Default [`FusionMethod::Rrf`].
    pub fusion: FusionMethod,
    /// Cap on the number of hits contributed per child (its top-N by own
    /// order), applied before fusion. Flooding-volume defense: one member
    /// cannot crowd out the others regardless of how many hits it returns.
    /// **Defaults to [`DEFAULT_PER_CHILD_CAP`]** (`Some(20)`); set `None` for an
    /// uncapped, fully-trusted federation.
    pub per_child_cap: Option<usize>,
}

impl Default for MergePolicy {
    fn default() -> Self {
        Self {
            fusion: FusionMethod::Rrf { k: DEFAULT_RRF_K },
            per_child_cap: Some(DEFAULT_PER_CHILD_CAP),
        }
    }
}

impl MergePolicy {
    /// The legacy v1 behavior: rank on raw `signal_score × weight`, no fusion,
    /// no cap. Kept for reproducing v1 results and as the poisoning
    /// benchmark's undefended baseline; **not** recommended for multi-user
    /// federation.
    pub fn raw_scores() -> Self {
        Self {
            fusion: FusionMethod::RawScore,
            per_child_cap: None,
        }
    }
}

/// Coordinator-side content fingerprint for corroboration: normalized
/// (trimmed, lowercased, whitespace-collapsed) content hashed with blake3.
/// Computed here because `MemoryHit` carries no content hash in v1.
fn content_fingerprint(content: &str) -> [u8; 32] {
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    *blake3::hash(normalized.to_lowercase().as_bytes()).as_bytes()
}

/// A directory of the child brains a coordinator knows about, mapping
/// [`BrainId`] → on-disk `data_dir`.
///
/// Pure record-keeping with CRUD only — **no filesystem scanning** (v1 is
/// single-tenant, N small, local). The recorded `data_dir` is the seam for a
/// future "re-open a child from its path" capability; v1 holds live handles
/// directly (see [`FederationCoordinator`]).
#[derive(Debug, Default, Clone)]
pub struct BrainRegistry {
    entries: HashMap<BrainId, PathBuf>,
}

impl BrainRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) a `brain_id → data_dir` mapping. Returns the
    /// previous path for this id, if any.
    pub fn register(&mut self, brain_id: BrainId, data_dir: impl Into<PathBuf>) -> Option<PathBuf> {
        self.entries.insert(brain_id, data_dir.into())
    }

    /// Remove a brain from the registry, returning its recorded path if known.
    pub fn deregister(&mut self, brain_id: &BrainId) -> Option<PathBuf> {
        self.entries.remove(brain_id)
    }

    /// The recorded `data_dir` for a brain, if registered.
    pub fn get(&self, brain_id: &BrainId) -> Option<&Path> {
        self.entries.get(brain_id).map(PathBuf::as_path)
    }

    /// Whether a brain is registered.
    pub fn contains(&self, brain_id: &BrainId) -> bool {
        self.entries.contains_key(brain_id)
    }

    /// Number of registered brains.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate the known `(brain_id, data_dir)` pairs. Order is unspecified.
    pub fn iter(&self) -> impl Iterator<Item = (&BrainId, &Path)> {
        self.entries.iter().map(|(id, p)| (id, p.as_path()))
    }
}

/// A [`MemoryHit`] tagged with the child brain it came from and the
/// provenance-weighted score the merge ranked it on.
#[derive(Debug, Clone)]
pub struct LabeledHit {
    /// Origin brain — provenance, owned coordinator-side because [`MemoryHit`]
    /// carries no brain id in v1.
    pub origin: BrainId,
    /// `hit.signal_score * brain_weight`; the value used for ranking.
    pub effective_score: f64,
    /// The underlying hit returned by the child's recall.
    pub hit: MemoryHit,
}

/// Result of a fan-out recall: one merged, provenance-ranked list plus
/// per-brain receipts.
#[derive(Debug, Clone)]
pub struct FanoutResult {
    /// All hits from all children, provenance-ranked best-first.
    pub ranked: Vec<LabeledHit>,
    /// `(brain_id, hit_count)` contributed by each child, in fan-out order.
    pub per_brain: Vec<(BrainId, usize)>,
    /// Sum of children's recognition token cost. Structurally `0` — no
    /// `Brain::recall_*` makes an LLM call — so consumers can assert the
    /// federation path adds no LLM cost.
    pub recognition_token_cost: usize,
    /// Children whose recall failed, as `(brain_id, error)`. The fan-out
    /// **degrades gracefully**: a failing child (locked/corrupt/unavailable DB)
    /// is skipped and recorded here rather than aborting the whole query, so a
    /// single unhealthy member cannot deny service to the federation. Consumers
    /// that require a complete result must check this is empty; the common case
    /// (all children healthy) leaves it empty.
    pub failed: Vec<(BrainId, String)>,
}

/// One child brain held by the coordinator: the live handle plus the
/// provenance weight applied to its hits during merge.
struct Child {
    brain: Brain,
    weight: f64,
}

/// In-process, read-time fan-out coordinator ("Henry").
///
/// Owns N child [`Brain`] handles and a parallel [`BrainRegistry`] recording
/// each child's `data_dir`. See the [module docs](self) for the design.
#[derive(Default)]
pub struct FederationCoordinator {
    registry: BrainRegistry,
    children: Vec<Child>,
}

impl FederationCoordinator {
    /// Create a coordinator with no children.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a child brain with the default provenance weight
    /// ([`DEFAULT_BRAIN_WEIGHT`]).
    ///
    /// `data_dir` is recorded in the registry. It is passed explicitly because
    /// `Brain` does not expose its own path — keeping provenance and the
    /// directory record coordinator-side is what lets v1 stay above the
    /// `Brain` API with zero core changes.
    pub fn add_brain(&mut self, brain: Brain, data_dir: impl Into<PathBuf>) -> &mut Self {
        self.add_brain_weighted(brain, data_dir, DEFAULT_BRAIN_WEIGHT)
    }

    /// Add a child brain with an explicit provenance `weight` multiplied into
    /// its hits' `signal_score` during merge. `weight == 1.0` ranks purely on
    /// the child's own score.
    pub fn add_brain_weighted(
        &mut self,
        brain: Brain,
        data_dir: impl Into<PathBuf>,
        weight: f64,
    ) -> &mut Self {
        let id = *brain.brain_id();
        self.registry.register(id, data_dir);
        // Mirror the registry's replace-on-duplicate semantics: re-adding a
        // BrainId swaps the child handle instead of double-counting its hits.
        let child = Child { brain, weight };
        if let Some(existing) = self.children.iter_mut().find(|c| c.brain.brain_id() == &id) {
            *existing = child;
        } else {
            self.children.push(child);
        }
        self
    }

    /// The registry of known child brains.
    pub fn registry(&self) -> &BrainRegistry {
        &self.registry
    }

    /// Number of child brains held.
    pub fn len(&self) -> usize {
        self.children.len()
    }

    /// Whether the coordinator holds no children.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// Fan a recall out across all child brains and return one merged,
    /// provenance-ranked result.
    ///
    /// `visibility` is the **federation boundary**: only hits whose own
    /// visibility label admits this context cross it (`content >= context`,
    /// same rule as single-brain recall). A coordinator merging brains
    /// contributed by different users passes [`Visibility::Team`] (or
    /// stricter) so members' `Private` memories never leave their brain; a
    /// coordinator over one user's own brains may pass
    /// [`Visibility::Private`] to see everything. Enforcement happens here,
    /// coordinator-side, because the underlying cascade path does not
    /// filter — and the child's labels are self-asserted, so this is
    /// honest-participant privacy, not mandatory access control.
    ///
    /// Each child is queried live (no cache), so any child-side change is
    /// reflected here on the next call. Ranking is by **provenance-weighted
    /// score** descending; ties break by origin [`BrainId`] bytes (it is not
    /// `Ord`) then by memory id, yielding a total deterministic order.
    ///
    /// v1 queries children **sequentially**. Because `recall_cascade` is
    /// `&self`, this is trivially parallelizable (one thread per child) if the
    /// measured latency warrants it.
    ///
    /// **Resilience:** a child whose recall errors (locked/corrupt/unavailable
    /// DB) is skipped, not fatal — the fan-out returns the healthy children's
    /// results and records the failed ones in [`FanoutResult::failed`]. A single
    /// unhealthy member cannot deny service to the whole federation; consumers
    /// needing a complete result assert `failed` is empty.
    ///
    /// Merging uses the default [`MergePolicy`] (Reciprocal Rank Fusion). Use
    /// [`fan_out_recall_with_policy`](Self::fan_out_recall_with_policy) to
    /// override.
    pub fn fan_out_recall(
        &self,
        query: &str,
        context: &RecognitionContext,
        config: &CascadePipelineConfig,
        visibility: Visibility,
    ) -> Result<FanoutResult, Error> {
        self.fan_out_recall_with_policy(query, context, config, visibility, &MergePolicy::default())
    }

    /// Fan-out recall with an explicit [`MergePolicy`] controlling how member
    /// scores are trusted and combined (poisoning-resistance knobs).
    pub fn fan_out_recall_with_policy(
        &self,
        query: &str,
        context: &RecognitionContext,
        config: &CascadePipelineConfig,
        visibility: Visibility,
        policy: &MergePolicy,
    ) -> Result<FanoutResult, Error> {
        let mut contributions: Vec<(BrainId, f64, Vec<MemoryHit>)> =
            Vec::with_capacity(self.children.len());
        let mut recognition_token_cost = 0usize;
        let mut failed: Vec<(BrainId, String)> = Vec::new();

        for child in &self.children {
            let origin = *child.brain.brain_id();
            // Degrade gracefully: a child whose recall errors (locked/corrupt/
            // unavailable DB) is skipped and recorded, not propagated — one
            // unhealthy member must not deny service to the whole federation.
            // The failure is surfaced in FanoutResult.failed, never silent.
            let result = match child.brain.recall_cascade(query, context, config) {
                Ok(result) => result,
                Err(e) => {
                    failed.push((origin, e.to_string()));
                    continue;
                }
            };
            recognition_token_cost += result.total_recognition_token_cost;
            let visible = result
                .merged_hits
                .into_iter()
                .filter(|hit| crate::brain::str_to_vis(&hit.visibility).allows(visibility))
                .collect::<Vec<_>>();
            contributions.push((origin, child.weight, visible));
        }

        let (ranked, per_brain) = merge_and_rank(contributions, policy);
        Ok(FanoutResult {
            ranked,
            per_brain,
            recognition_token_cost,
            failed,
        })
    }
}

/// Merge per-child recall contributions into one provenance-ranked list under
/// a [`MergePolicy`]. Pure and side-effect free — the trust/anti-poisoning
/// logic lives here so it is testable on synthetic hits with controlled
/// signal scores, independent of the retrieval stack.
///
/// `contributions` is `(origin, weight, hits_in_child_order)` per child.
/// Returns the ranked hits and the per-brain contribution counts (post-cap).
fn merge_and_rank(
    contributions: Vec<(BrainId, f64, Vec<MemoryHit>)>,
    policy: &MergePolicy,
) -> (Vec<LabeledHit>, Vec<(BrainId, usize)>) {
    let mut ranked: Vec<LabeledHit> = Vec::new();
    let mut per_brain: Vec<(BrainId, usize)> = Vec::with_capacity(contributions.len());

    // Apply the per-child cap first, then record each hit with its
    // within-child rank (needed for RRF).
    let mut capped: Vec<(BrainId, f64, Vec<MemoryHit>)> = Vec::with_capacity(contributions.len());
    for (origin, weight, mut hits) in contributions {
        if let Some(cap) = policy.per_child_cap {
            hits.truncate(cap);
        }
        per_brain.push((origin, hits.len()));
        capped.push((origin, weight, hits));
    }

    match &policy.fusion {
        FusionMethod::RawScore => {
            // Legacy / undefended: rank on the member's self-asserted score.
            // signal_score is member-controlled and unbounded — sanitize it so a
            // NaN/Inf can't pin poison to the top (NaN sorts above +Inf under
            // total_cmp). Clamp to signal_score's documented [0,1] range.
            for (origin, weight, hits) in capped {
                let w = if weight.is_finite() { weight } else { 0.0 };
                for hit in hits {
                    let s = if hit.signal_score.is_finite() {
                        hit.signal_score.clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    ranked.push(LabeledHit {
                        origin,
                        effective_score: s * w,
                        hit,
                    });
                }
            }
        }
        FusionMethod::Rrf { k } => {
            // Reciprocal Rank Fusion, deduplicated per origin. A member
            // contributes AT MOST ONCE per content identity (its best rank),
            // so it cannot self-corroborate by flooding identical copies — the
            // exact attack the poisoning benchmark surfaced. Contributions then
            // sum ACROSS DISTINCT origins, so content independently returned by
            // K members outranks a lone assertion (genuine corroboration).
            // Every surviving copy carries the fused score, preserving each
            // origin's provenance.
            let mut best_rank: HashMap<([u8; 32], [u8; 32]), usize> = HashMap::new();
            for (origin, _weight, hits) in &capped {
                let obytes = *origin.as_bytes();
                for (rank, hit) in hits.iter().enumerate() {
                    let fp = content_fingerprint(&hit.content);
                    best_rank
                        .entry((obytes, fp))
                        .and_modify(|r| *r = (*r).min(rank))
                        .or_insert(rank);
                }
            }
            // weight lookup by origin bytes.
            let weight_of: HashMap<[u8; 32], f64> =
                capped.iter().map(|(o, w, _)| (*o.as_bytes(), *w)).collect();
            let mut fused: HashMap<[u8; 32], f64> = HashMap::new();
            for ((obytes, fp), rank) in &best_rank {
                let w = weight_of.get(obytes).copied().unwrap_or(1.0);
                let denom = k + *rank as f64;
                // Guard a misconfigured k (<=0, NaN) that would make the
                // contribution +inf/NaN and hijack the sort. Well-formed RRF has
                // k > 0, so denom > 0 always; skip the contribution otherwise.
                if !w.is_finite() || !denom.is_finite() || denom <= 0.0 {
                    continue;
                }
                *fused.entry(*fp).or_insert(0.0) += w / denom;
            }
            for (origin, _weight, hits) in capped {
                for hit in hits {
                    let score = *fused
                        .get(&content_fingerprint(&hit.content))
                        .unwrap_or(&0.0);
                    ranked.push(LabeledHit {
                        origin,
                        effective_score: score,
                        hit,
                    });
                }
            }
        }
    }

    // Primary key: effective (fused) score, descending. RRF produces many exact
    // ties (every uncorroborated hit is 1/(k+0)); break them by a CONTENT hash
    // first, not the member-chosen BrainId — otherwise a member could grind one
    // low-sorting BrainId to win every tie across the federation. Content-first
    // means an attacker would have to grind each item's content (which changes
    // the content). BrainId/hit.id remain as final tiebreakers for a total,
    // deterministic order. Fingerprint is precomputed to keep the sort O(n log n).
    let mut keyed: Vec<([u8; 32], LabeledHit)> = ranked
        .into_iter()
        .map(|h| (content_fingerprint(&h.hit.content), h))
        .collect();
    keyed.sort_by(|a, b| {
        b.1.effective_score
            .total_cmp(&a.1.effective_score)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.origin.as_bytes().cmp(b.1.origin.as_bytes()))
            .then_with(|| a.1.hit.id.cmp(&b.1.hit.id))
    });
    let ranked: Vec<LabeledHit> = keyed.into_iter().map(|(_, h)| h).collect();

    // Overlapping memories from different brains are BOTH kept — distinct
    // provenance — but share the fused score, so a corroborated item's copies
    // sit together at the top while each origin's receipt is preserved.
    (ranked, per_brain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::{Brain, BrainConfig, EntityPolicy};
    use spectral_core::visibility::Visibility;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn child_config(data_dir: PathBuf) -> BrainConfig {
        BrainConfig {
            data_dir,
            ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
            memory_db_path: None,
            llm_client: None,
            wing_rules: None,
            hall_rules: None,
            device_id: None,
            enable_spectrogram: false,
            entity_policy: EntityPolicy::Strict,
            sqlite_mmap_size: None,
            fts_tokenizer: None,
            read_only: false,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        }
    }

    fn open_child(tmp: &TempDir, name: &str) -> (Brain, PathBuf) {
        let dir = tmp.path().join(name);
        let brain = Brain::open(child_config(dir.clone())).expect("open child brain");
        (brain, dir)
    }

    fn contents(result: &FanoutResult) -> Vec<String> {
        result
            .ranked
            .iter()
            .map(|h| h.hit.content.clone())
            .collect()
    }

    /// Correctness: a fan-out over N=3 brains returns the UNION of what each
    /// brain would return alone, merged and provenance-ranked — including
    /// overlapping memories kept once per origin (defer-to-ranking, no dedup).
    #[test]
    fn fan_out_unions_disjoint_and_overlapping() {
        let tmp = TempDir::new().unwrap();
        let (a, a_dir) = open_child(&tmp, "a");
        let (b, b_dir) = open_child(&tmp, "b");
        let (c, c_dir) = open_child(&tmp, "c");

        // Disjoint memories, one per brain ...
        a.remember(
            "a-only",
            "shared topic alpha unique to a",
            Visibility::Private,
        )
        .unwrap();
        b.remember(
            "b-only",
            "shared topic beta unique to b",
            Visibility::Private,
        )
        .unwrap();
        c.remember(
            "c-only",
            "shared topic gamma unique to c",
            Visibility::Private,
        )
        .unwrap();
        // ... plus an OVERLAPPING memory present in both a and b.
        let overlap = "shared topic common to a and b";
        a.remember("shared", overlap, Visibility::Private).unwrap();
        b.remember("shared", overlap, Visibility::Private).unwrap();

        let a_id = *a.brain_id();
        let b_id = *b.brain_id();
        let c_id = *c.brain_id();

        let mut henry = FederationCoordinator::new();
        henry.add_brain(a, a_dir);
        henry.add_brain(b, b_dir);
        henry.add_brain(c, c_dir);
        assert_eq!(henry.len(), 3);
        assert_eq!(henry.registry().len(), 3);
        assert!(henry.registry().contains(&a_id));
        assert!(henry.registry().contains(&b_id));
        assert!(henry.registry().contains(&c_id));

        let result = henry
            .fan_out_recall(
                "shared topic",
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
                Visibility::Private,
            )
            .unwrap();

        let found = contents(&result);
        assert!(found.iter().any(|c| c.contains("alpha")), "missing a's hit");
        assert!(found.iter().any(|c| c.contains("beta")), "missing b's hit");
        assert!(found.iter().any(|c| c.contains("gamma")), "missing c's hit");

        // Overlap kept once per origin — both a and b contribute it.
        let overlap_origins: std::collections::HashSet<BrainId> = result
            .ranked
            .iter()
            .filter(|h| h.hit.content == overlap)
            .map(|h| h.origin)
            .collect();
        assert_eq!(
            overlap_origins.len(),
            2,
            "overlapping memory should appear from both a and b (no dedup in v1)"
        );
        assert!(overlap_origins.contains(&a_id) && overlap_origins.contains(&b_id));

        // No LLM cost added by the federation path.
        assert_eq!(result.recognition_token_cost, 0);
    }

    /// Provenance + deterministic tiebreak: every hit carries its origin
    /// brain_id, the ranking is sorted by the documented comparator, and the
    /// order is stable across repeated identical fan-outs.
    #[test]
    fn provenance_labeled_and_deterministic_order() {
        let tmp = TempDir::new().unwrap();
        let (a, a_dir) = open_child(&tmp, "a");
        let (b, b_dir) = open_child(&tmp, "b");
        let (c, c_dir) = open_child(&tmp, "c");

        for (brain, tag) in [(&a, "a"), (&b, "b"), (&c, "c")] {
            brain
                .remember(
                    &format!("m-{tag}"),
                    &format!("recall probe token shared by all brains, brain {tag}"),
                    Visibility::Private,
                )
                .unwrap();
        }

        let known: std::collections::HashSet<BrainId> =
            [*a.brain_id(), *b.brain_id(), *c.brain_id()]
                .into_iter()
                .collect();

        let mut henry = FederationCoordinator::new();
        henry.add_brain(a, a_dir);
        henry.add_brain(b, b_dir);
        henry.add_brain(c, c_dir);

        let run = || {
            henry
                .fan_out_recall(
                    "recall probe token",
                    &RecognitionContext::empty(),
                    &CascadePipelineConfig::default(),
                    Visibility::Private,
                )
                .unwrap()
        };

        let first = run();
        assert!(!first.ranked.is_empty(), "expected hits across brains");

        // Every returned hit is labeled with a known origin brain.
        for h in &first.ranked {
            assert!(known.contains(&h.origin), "hit has unknown origin");
        }

        // The ranking obeys the documented total order: effective_score
        // non-increasing, ties broken by content hash, then origin bytes, then
        // memory id.
        for w in first.ranked.windows(2) {
            let (x, y) = (&w[0], &w[1]);
            let ord = y
                .effective_score
                .total_cmp(&x.effective_score)
                .then_with(|| {
                    content_fingerprint(&x.hit.content).cmp(&content_fingerprint(&y.hit.content))
                })
                .then_with(|| x.origin.as_bytes().cmp(y.origin.as_bytes()))
                .then_with(|| x.hit.id.cmp(&y.hit.id));
            assert_ne!(
                ord,
                std::cmp::Ordering::Greater,
                "ranking violates the documented comparator"
            );
        }

        // Deterministic: a second identical fan-out yields the same sequence.
        let second = run();
        let seq = |r: &FanoutResult| -> Vec<(BrainId, String)> {
            r.ranked
                .iter()
                .map(|h| (h.origin, h.hit.id.clone()))
                .collect()
        };
        assert_eq!(
            seq(&first),
            seq(&second),
            "fan-out order is not deterministic"
        );
    }

    /// Isolation / read-time: removing a memory from one child (via
    /// consolidation, the only read-affecting removal the `Brain` API exposes)
    /// is reflected in the NEXT fan-out — the coordinator holds no cache.
    #[test]
    fn read_time_removal_propagates() {
        let tmp = TempDir::new().unwrap();
        let (a, a_dir) = open_child(&tmp, "a");
        let (b, b_dir) = open_child(&tmp, "b");

        // `anchor` is the consolidation target; `ephemeral` is what we remove.
        a.remember("anchor", "shared topic anchor memory", Visibility::Private)
            .unwrap();
        a.remember(
            "ephemeral",
            "shared topic ephemeral memory",
            Visibility::Private,
        )
        .unwrap();
        b.remember("b-keep", "shared topic kept in b", Visibility::Private)
            .unwrap();

        let mut henry = FederationCoordinator::new();
        henry.add_brain(a, a_dir);
        henry.add_brain(b, b_dir);

        let query = "shared topic ephemeral";
        let before = henry
            .fan_out_recall(
                query,
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
                Visibility::Private,
            )
            .unwrap();
        assert!(
            before
                .ranked
                .iter()
                .any(|h| h.hit.content.contains("ephemeral")),
            "ephemeral memory should be present before removal"
        );

        // Remove `ephemeral` from child a (consolidation filters sources out of
        // recall). The coordinator still holds a's live handle.
        henry.children[0]
            .brain
            .consolidate_into(
                &["ephemeral".to_string()],
                "anchor",
                &spectral_ingest::ConsolidateOpts::default(),
            )
            .unwrap();

        let after = henry
            .fan_out_recall(
                query,
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
                Visibility::Private,
            )
            .unwrap();
        assert!(
            !after
                .ranked
                .iter()
                .any(|h| h.hit.content.contains("ephemeral")),
            "removed memory must not reappear — read-time deletion did not propagate"
        );
    }

    /// Latency receipt: measure wall-clock of a fan-out over N=3 local brains.
    /// Does not assert a tight bound (machine-dependent); prints the figure
    /// and guards only a loose sanity ceiling. The PR reports the real number.
    #[test]
    fn fan_out_latency_n3_is_reported() {
        let tmp = TempDir::new().unwrap();
        let (a, a_dir) = open_child(&tmp, "a");
        let (b, b_dir) = open_child(&tmp, "b");
        let (c, c_dir) = open_child(&tmp, "c");
        for (brain, tag) in [(&a, "a"), (&b, "b"), (&c, "c")] {
            brain
                .remember(
                    &format!("lat-{tag}"),
                    &format!("latency probe memory for brain {tag}"),
                    Visibility::Private,
                )
                .unwrap();
        }

        let mut henry = FederationCoordinator::new();
        henry.add_brain(a, a_dir);
        henry.add_brain(b, b_dir);
        henry.add_brain(c, c_dir);

        // Warm one query (first recall pays one-time setup), then measure.
        let ctx = RecognitionContext::empty();
        let cfg = CascadePipelineConfig::default();
        let _ = henry
            .fan_out_recall("latency probe", &ctx, &cfg, Visibility::Private)
            .unwrap();

        let start = std::time::Instant::now();
        let result = henry
            .fan_out_recall("latency probe", &ctx, &cfg, Visibility::Private)
            .unwrap();
        let elapsed = start.elapsed();

        eprintln!(
            "[federation] fan-out over N=3 brains: {:?} ({} hits)",
            elapsed,
            result.ranked.len()
        );
        if elapsed.as_millis() > 100 {
            eprintln!(
                "[federation] WARNING: fan-out > 100ms — consider parallelizing per-brain recalls"
            );
        }
        assert!(
            elapsed.as_secs() < 5,
            "fan-out latency wildly out of range: {elapsed:?}"
        );
    }

    /// Federation boundary: a Team-context fan-out must not surface any
    /// child's Private memories; the child's Public memories still cross.
    /// A Private-context fan-out (single-user coordinator) sees everything.
    #[test]
    fn fan_out_visibility_boundary_filters_private() {
        let tmp = TempDir::new().unwrap();
        let (a, a_dir) = open_child(&tmp, "a");

        a.remember(
            "priv",
            "shared topic private secret memory",
            Visibility::Private,
        )
        .unwrap();
        a.remember("pub", "shared topic public note memory", Visibility::Public)
            .unwrap();

        let mut henry = FederationCoordinator::new();
        henry.add_brain(a, a_dir);

        let team = henry
            .fan_out_recall(
                "shared topic memory",
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
                Visibility::Team,
            )
            .unwrap();
        assert!(
            !team
                .ranked
                .iter()
                .any(|h| h.hit.content.contains("private secret")),
            "Private memory crossed a Team federation boundary"
        );
        assert!(
            team.ranked
                .iter()
                .any(|h| h.hit.content.contains("public note")),
            "Public memory should cross a Team federation boundary"
        );

        let own = henry
            .fan_out_recall(
                "shared topic memory",
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
                Visibility::Private,
            )
            .unwrap();
        assert!(
            own.ranked
                .iter()
                .any(|h| h.hit.content.contains("private secret")),
            "Private-context fan-out over one's own brains should see private memories"
        );
    }

    /// Regression: `Brain` drives its own runtime with `block_on`, which panics
    /// if nested inside another Tokio runtime — the normal way a memory library
    /// gets embedded (an async server handler). SafeRuntime offloads to a scoped
    /// thread instead, so open + remember + recall all work from inside a runtime.
    #[test]
    fn brain_is_safe_to_call_from_inside_a_runtime() {
        let outer = tokio::runtime::Runtime::new().unwrap();
        outer.block_on(async {
            let tmp = TempDir::new().unwrap();
            // Brain::open itself block_ons during backfill — must not panic here.
            let (brain, _dir) = open_child(&tmp, "async_ctx");
            brain
                .remember("k", "async context probe memory", Visibility::Private)
                .unwrap();
            let r = brain
                .recall_cascade(
                    "async context probe",
                    &RecognitionContext::empty(),
                    &CascadePipelineConfig::default(),
                )
                .unwrap();
            assert!(
                r.merged_hits.iter().any(|h| h.content.contains("probe")),
                "recall from inside a runtime should work, not panic"
            );
        });
    }

    /// Async write-back: with it enabled, `recall_cascade` still applies the
    /// auto-reinforce nudge — just off the critical path. We poll the reinforced
    /// memory's signal_score until the spawned write lands (bounded), proving the
    /// ambient bookkeeping is deferred, not dropped.
    #[test]
    fn async_writeback_still_reinforces_eventually() {
        let tmp = TempDir::new().unwrap();
        let (mut brain, _dir) = open_child(&tmp, "async");
        let remembered = brain
            .remember("m", "asyncwriteback probe distinctive memory", Visibility::Private)
            .unwrap();
        let id = remembered.memory_id;
        let before = brain.get_memory(&id).unwrap().unwrap().signal_score;

        brain.set_async_writeback(true);
        let hits = brain
            .recall_cascade(
                "asyncwriteback probe",
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
            )
            .unwrap();
        assert!(
            hits.merged_hits.iter().any(|h| h.id == id),
            "probe memory must be retrieved so it is a reinforce target"
        );

        // The spawned write lands shortly after recall returns. Poll (bounded).
        let mut reinforced = false;
        for _ in 0..200 {
            let now = brain.get_memory(&id).unwrap().unwrap().signal_score;
            if now > before {
                reinforced = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            reinforced,
            "async write-back never applied the reinforce nudge (deferred write dropped)"
        );
    }

    /// Resilience: one child's recall failing (locked/corrupt DB) must not
    /// abort the whole fan-out. Healthy children still contribute, and the
    /// failure is surfaced in `FanoutResult.failed` rather than propagated or
    /// silently dropped. Fault is injected by dropping the bad child's FTS
    /// table out from under its live connection via a second connection — a
    /// real store error, not a mock.
    #[test]
    fn fan_out_degrades_gracefully_when_a_child_fails() {
        let tmp = TempDir::new().unwrap();
        let (good, good_dir) = open_child(&tmp, "good");
        good.remember("g", "shared topic healthy memory", Visibility::Public)
            .unwrap();

        let (bad, bad_dir) = open_child(&tmp, "bad");
        bad.remember("b", "shared topic doomed memory", Visibility::Public)
            .unwrap();
        let bad_id = *bad.brain_id();

        // Break the bad child: drop the FTS table its recall path queries.
        // SQLite makes the DDL visible to the child's connection, so its next
        // fts_search errors ("no such table").
        {
            let conn = rusqlite::Connection::open(bad_dir.join("memory.db")).unwrap();
            conn.execute_batch("DROP TABLE memories_fts;").unwrap();
        }
        // Confirm the fault is live: the bad child alone now errors.
        assert!(
            bad.recall_cascade(
                "shared topic memory",
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
            )
            .is_err(),
            "fault injection failed — bad child still recalls"
        );

        let mut coord = FederationCoordinator::new();
        coord.add_brain(good, good_dir);
        coord.add_brain(bad, bad_dir);

        // Fan-out must SUCCEED (not Err) despite the broken child.
        let result = coord
            .fan_out_recall(
                "shared topic memory",
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
                Visibility::Team,
            )
            .expect("fan-out must not abort when one child fails");

        // Healthy child's memory survived.
        assert!(
            result
                .ranked
                .iter()
                .any(|h| h.hit.content.contains("healthy")),
            "healthy child's result should survive a sibling's failure"
        );
        // The failure is reported, not silent.
        assert!(
            result.failed.iter().any(|(id, _)| *id == bad_id),
            "the failed child must be recorded in FanoutResult.failed"
        );
        // All-healthy sanity: a fan-out over just the good child reports no failures.
        let mut ok_coord = FederationCoordinator::new();
        let (good2, good2_dir) = open_child(&tmp, "good2");
        good2
            .remember("g2", "shared topic another healthy memory", Visibility::Public)
            .unwrap();
        ok_coord.add_brain(good2, good2_dir);
        let ok = ok_coord
            .fan_out_recall(
                "shared topic memory",
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
                Visibility::Team,
            )
            .unwrap();
        assert!(ok.failed.is_empty(), "healthy fan-out should report no failures");
    }

    /// Single-brain boundary: `recall_cascade_scoped` must enforce the same
    /// visibility filter the fan-out coordinator applies — a Team context over
    /// one's own brain never surfaces Private content, while the unscoped
    /// `recall_cascade_with_pipeline` (Private context) still returns everything.
    #[test]
    fn recall_cascade_scoped_filters_private_at_the_brain() {
        let tmp = TempDir::new().unwrap();
        let (a, _dir) = open_child(&tmp, "a");

        a.remember(
            "priv",
            "shared topic private secret memory",
            Visibility::Private,
        )
        .unwrap();
        a.remember("pub", "shared topic public note memory", Visibility::Public)
            .unwrap();

        let ctx = RecognitionContext::empty();
        let cfg = CascadePipelineConfig::default();

        // Team-scoped: Private content must NOT appear.
        let team = a
            .recall_cascade_scoped("shared topic memory", &ctx, &cfg, Visibility::Team)
            .unwrap();
        assert!(
            !team
                .merged_hits
                .iter()
                .any(|h| h.content.contains("private secret")),
            "Private memory leaked into a Team-scoped single-brain recall"
        );
        assert!(
            team.merged_hits
                .iter()
                .any(|h| h.content.contains("public note")),
            "Public memory should survive a Team-scoped recall"
        );

        // Unscoped (Private context) still returns everything — no regression.
        let all = a
            .recall_cascade_with_pipeline("shared topic memory", &ctx, &cfg)
            .unwrap();
        assert!(
            all.merged_hits
                .iter()
                .any(|h| h.content.contains("private secret")),
            "Unscoped recall over one's own brain should still see private memories"
        );
    }

    /// A child opened read-only participates in fan-out without being
    /// mutated: no signal-score inflation, no retrieval events written, and
    /// write APIs on the handle are rejected.
    #[test]
    fn read_only_child_is_not_mutated_by_fan_out() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("member");

        // Build the member brain read-write, then close it.
        {
            let owner = Brain::open(child_config(dir.clone())).unwrap();
            owner
                .remember(
                    "fact",
                    "shared topic member fact memory",
                    Visibility::Public,
                )
                .unwrap();
        }

        // Reopen read-only and federate over it.
        let ro = Brain::open(BrainConfig {
            read_only: true,
            ..child_config(dir.clone())
        })
        .unwrap();
        assert!(ro.is_read_only());
        // Write APIs are rejected with a dedicated error.
        match ro.remember("nope", "should fail", Visibility::Private) {
            Err(Error::ReadOnly(_)) => {}
            other => panic!("expected Error::ReadOnly, got {other:?}"),
        }

        let baseline_events = ro.count_retrieval_events().unwrap();
        let mut henry = FederationCoordinator::new();
        henry.add_brain(ro, dir.clone());
        let result = henry
            .fan_out_recall(
                "shared topic member fact",
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
                Visibility::Team,
            )
            .unwrap();
        assert!(
            result
                .ranked
                .iter()
                .any(|h| h.hit.content.contains("member fact")),
            "read-only child should still serve hits"
        );
        let signal_after_ro = result.ranked[0].hit.signal_score;
        drop(henry);

        // Reopen read-write: the fan-out must have left no trace.
        let owner = Brain::open(child_config(dir)).unwrap();
        assert_eq!(
            owner.count_retrieval_events().unwrap(),
            baseline_events,
            "fan-out over a read-only child must not log retrieval events into it"
        );
        let hits = owner
            .recall_topk_fts(
                "member fact",
                &crate::brain::RecallTopKConfig::default(),
                Visibility::Private,
            )
            .unwrap();
        let stored_signal = hits
            .iter()
            .find(|h| h.key == "fact")
            .expect("member fact present")
            .signal_score;
        assert!(
            (stored_signal - signal_after_ro).abs() < 1e-9,
            "fan-out over a read-only child must not auto-reinforce its memories \
             (stored {stored_signal}, seen {signal_after_ro})"
        );
    }

    /// Poisoning defense: a member that floods the fan-out with self-asserted
    /// max-signal memories must NOT out-rank an honest answer under the
    /// default RRF policy — whereas under the legacy raw-score policy it buries
    /// it. Tested on the pure [`merge_and_rank`] with controlled signal scores
    /// so the result is deterministic and independent of the retrieval stack.
    #[test]
    fn rrf_blocks_self_asserted_score_dominance() {
        fn hit(id: &str, content: &str, signal: f64) -> MemoryHit {
            MemoryHit {
                id: id.into(),
                key: id.into(),
                content: content.into(),
                wing: None,
                hall: None,
                signal_score: signal,
                visibility: "public".into(),
                hits: 1,
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
                source_brain_id: None,
                signature: None,
            }
        }
        let honest = BrainId::from_bytes([1u8; 32]);
        let attacker = BrainId::from_bytes([2u8; 32]);

        // Honest: one genuinely relevant answer at a moderate signal.
        // Attacker: five poisons all self-asserted to the maximum signal 1.0.
        let contributions = || {
            vec![
                (honest, 1.0, vec![hit("h1", "the authoritative answer", 0.5)]),
                (
                    attacker,
                    1.0,
                    (0..5)
                        .map(|i| hit(&format!("p{i}"), &format!("poison payload {i}"), 1.0))
                        .collect(),
                ),
            ]
        };

        let honest_rank = |ranked: &[LabeledHit]| -> usize {
            ranked.iter().position(|h| h.origin == honest).unwrap()
        };
        let attacker_above_honest = |ranked: &[LabeledHit]| -> usize {
            let hr = honest_rank(ranked);
            ranked[..hr].iter().filter(|h| h.origin == attacker).count()
        };

        // Raw scores: all five poisons (signal 1.0) outrank the honest answer
        // (0.5) — the honest answer is buried at rank 5. This is the attack.
        let (raw, _) = merge_and_rank(contributions(), &MergePolicy::raw_scores());
        assert_eq!(
            attacker_above_honest(&raw),
            5,
            "raw-score merge buries the honest answer under the whole flood"
        );

        // Default policy (RRF): ranks on within-child *position*, not the
        // self-asserted score, so the attacker's flood contributes a decaying
        // 1/(k+rank) series. The honest answer (the top of its own list) ties
        // the attacker's single top poison and wins the deterministic
        // tiebreak; every other poison sinks below it. Flood neutralized.
        let (safe, _) = merge_and_rank(contributions(), &MergePolicy::default());
        assert_eq!(
            attacker_above_honest(&safe),
            0,
            "RRF must lift the honest answer above the whole flood, {} still above",
            attacker_above_honest(&safe)
        );
        assert_eq!(honest_rank(&safe), 0, "honest answer should be at the top under RRF");
    }

    /// RawScore ranks on the member's self-asserted, unbounded signal_score.
    /// A malicious member setting NaN/Inf/huge must not pin poison to the top
    /// (NaN sorts above +Inf under total_cmp). The merge sanitizes non-finite
    /// scores to 0 and clamps to [0,1], so an honest 0.5 stays above the poison.
    #[test]
    fn raw_score_merge_sanitizes_non_finite_poison() {
        fn hit(id: &str, signal: f64) -> MemoryHit {
            MemoryHit {
                id: id.into(),
                key: id.into(),
                content: format!("content {id}"),
                wing: None,
                hall: None,
                signal_score: signal,
                visibility: "public".into(),
                hits: 1,
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
                source_brain_id: None,
                signature: None,
            }
        }
        let honest = BrainId::from_bytes([1u8; 32]);
        let attacker = BrainId::from_bytes([2u8; 32]);
        // Honest asserts a legitimate max (1.0). Attacker asserts NaN/Inf, which
        // under total_cmp would otherwise sort ABOVE any finite score. (A huge
        // finite value clamps to a legitimate 1.0 — that's the documented
        // undefended RawScore behavior, not the F5 bug.)
        let contributions = vec![
            (honest, 1.0, vec![hit("honest", 1.0)]),
            (
                attacker,
                1.0,
                vec![hit("nan", f64::NAN), hit("inf", f64::INFINITY)],
            ),
        ];
        let (ranked, _) = merge_and_rank(contributions, &MergePolicy::raw_scores());
        assert_eq!(
            ranked[0].origin, honest,
            "honest legit-max must outrank NaN/Inf poison after sanitization; got {:?}",
            ranked.iter().map(|h| h.hit.id.as_str()).collect::<Vec<_>>()
        );
        // No poison sits above the honest answer.
        let hr = ranked.iter().position(|h| h.origin == honest).unwrap();
        assert_eq!(ranked[..hr].iter().filter(|h| h.origin == attacker).count(), 0);
    }

    /// RRF self-corroboration defense (regression for the bug the poisoning
    /// benchmark surfaced): a single member flooding MANY identical copies of a
    /// poison must NOT out-rank content genuinely corroborated by two distinct
    /// members. RRF dedupes contributions per origin, so the flood counts once.
    #[test]
    fn rrf_ignores_self_corroboration_by_identical_flood() {
        fn hit(id: &str, content: &str) -> MemoryHit {
            MemoryHit {
                id: id.into(),
                key: id.into(),
                content: content.into(),
                wing: None,
                hall: None,
                signal_score: 1.0,
                visibility: "public".into(),
                hits: 1,
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
                source_brain_id: None,
                signature: None,
            }
        }
        let honest_a = BrainId::from_bytes([1u8; 32]);
        let honest_b = BrainId::from_bytes([2u8; 32]);
        let attacker = BrainId::from_bytes([9u8; 32]);

        // Two honest members independently return the SAME genuine answer;
        // the attacker returns SIX identical copies of a poison.
        let contributions = vec![
            (honest_a, 1.0, vec![hit("ha", "the genuine corroborated answer")]),
            (honest_b, 1.0, vec![hit("hb", "the genuine corroborated answer")]),
            (
                attacker,
                1.0,
                (0..6).map(|i| hit(&format!("p{i}"), "the attacker poison")).collect(),
            ),
        ];

        let (ranked, _) = merge_and_rank(contributions, &MergePolicy::default());
        assert!(
            ranked[0].hit.content.contains("genuine"),
            "genuine 2-member corroboration must outrank a 6x identical self-flood; top was {:?}",
            ranked[0].hit.content
        );
        // The poison must not appear until after the corroborated answer.
        let poison_rank = ranked.iter().position(|h| h.origin == attacker).unwrap();
        let honest_rank = ranked.iter().position(|h| h.origin != attacker).unwrap();
        assert!(honest_rank < poison_rank, "honest content must rank above the flood");
    }

    /// Corroboration boost: content independently contributed by two members
    /// outranks an uncorroborated memory that would otherwise tie, and a lone
    /// member cannot self-corroborate by returning duplicates.
    #[test]
    fn corroboration_boost_rewards_independent_agreement() {
        let tmp = TempDir::new().unwrap();
        let (a, a_dir) = open_child(&tmp, "a");
        let (b, b_dir) = open_child(&tmp, "b");

        let agreed = "shared topic the corroborated fact everyone agrees on";
        a.remember("a-agreed", agreed, Visibility::Public).unwrap();
        b.remember("b-agreed", agreed, Visibility::Public).unwrap();
        // A lone extra memory in a, same text as itself won't self-corroborate.
        a.remember(
            "a-solo",
            "shared topic a solo uncorroborated claim from a only",
            Visibility::Public,
        )
        .unwrap();

        let mut henry = FederationCoordinator::new();
        henry.add_brain(a, a_dir);
        henry.add_brain(b, b_dir);

        let result = henry
            .fan_out_recall(
                "shared topic fact claim",
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
                Visibility::Team,
            )
            .unwrap();

        // The corroborated content (2 independent origins) ranks first.
        assert!(
            result.ranked[0].hit.content.contains("corroborated fact"),
            "independently-agreed content should outrank uncorroborated content, \
             got top: {:?}",
            result.ranked[0].hit.content
        );
        // Both origins of the agreed fact are still present (no dedup).
        let agreed_origins: std::collections::HashSet<BrainId> = result
            .ranked
            .iter()
            .filter(|h| h.hit.content.contains("corroborated fact"))
            .map(|h| h.origin)
            .collect();
        assert_eq!(agreed_origins.len(), 2, "both contributors should be visible");
    }

    /// Per-child cap bounds one member's contribution regardless of how many
    /// hits it returns.
    #[test]
    fn per_child_cap_bounds_contribution() {
        let tmp = TempDir::new().unwrap();
        let (a, a_dir) = open_child(&tmp, "a");
        let (b, b_dir) = open_child(&tmp, "b");

        for i in 0..6 {
            a.remember(
                &format!("a-{i}"),
                &format!("shared topic memory number {i} from a"),
                Visibility::Public,
            )
            .unwrap();
        }
        b.remember("b-1", "shared topic memory from b", Visibility::Public)
            .unwrap();

        let a_id = *a.brain_id();
        let mut henry = FederationCoordinator::new();
        henry.add_brain(a, a_dir);
        henry.add_brain(b, b_dir);

        let policy = MergePolicy {
            per_child_cap: Some(2),
            ..MergePolicy::default()
        };
        let result = henry
            .fan_out_recall_with_policy(
                "shared topic memory",
                &RecognitionContext::empty(),
                &CascadePipelineConfig::default(),
                Visibility::Team,
                &policy,
            )
            .unwrap();

        let a_count = result.ranked.iter().filter(|h| h.origin == a_id).count();
        assert!(
            a_count <= 2,
            "per-child cap should bound a's contribution to 2, got {a_count}"
        );
    }
}
