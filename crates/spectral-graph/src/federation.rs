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
use spectral_ingest::MemoryHit;

use crate::brain::Brain;
use crate::cascade_layers::CascadePipelineConfig;
use crate::error::Error;

/// Default provenance weight applied to a child's hits: rank purely on the
/// child's own `signal_score`.
pub const DEFAULT_BRAIN_WEIGHT: f64 = 1.0;

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
        self.children.push(Child { brain, weight });
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
    /// Each child is queried live (no cache), so any child-side change is
    /// reflected here on the next call. Ranking is by **provenance-weighted
    /// score** descending; ties break by origin [`BrainId`] bytes (it is not
    /// `Ord`) then by memory id, yielding a total deterministic order.
    ///
    /// v1 queries children **sequentially**. Because `recall_cascade` is
    /// `&self`, this is trivially parallelizable (one thread per child) if the
    /// measured latency warrants it.
    pub fn fan_out_recall(
        &self,
        query: &str,
        context: &RecognitionContext,
        config: &CascadePipelineConfig,
    ) -> Result<FanoutResult, Error> {
        let mut ranked: Vec<LabeledHit> = Vec::new();
        let mut per_brain: Vec<(BrainId, usize)> = Vec::with_capacity(self.children.len());
        let mut recognition_token_cost = 0usize;

        for child in &self.children {
            let origin = *child.brain.brain_id();
            let result = child.brain.recall_cascade(query, context, config)?;
            recognition_token_cost += result.total_recognition_token_cost;
            per_brain.push((origin, result.merged_hits.len()));
            for hit in result.merged_hits {
                let effective_score = hit.signal_score * child.weight;
                ranked.push(LabeledHit {
                    origin,
                    effective_score,
                    hit,
                });
            }
        }

        // Merge-and-rank. Primary key: provenance-weighted score, descending.
        // Tiebreak by origin BrainId bytes (BrainId is NOT Ord) then memory id
        // — both ascending — for a total, deterministic order.
        ranked.sort_by(|a, b| {
            b.effective_score
                .total_cmp(&a.effective_score)
                .then_with(|| a.origin.as_bytes().cmp(b.origin.as_bytes()))
                .then_with(|| a.hit.id.cmp(&b.hit.id))
        });

        // Dedup seam (intentionally a no-op in v1): overlapping memories from
        // different brains are BOTH kept — distinct provenance, defer-to-
        // ranking. A future content-hash dedup would slot HERE, collapsing
        // equal-content hits and keeping the highest-ranked provenance.
        // `MemoryHit` carries no content hash today, so v1 cannot dedup by
        // content without a core change; this is out of scope by design.

        Ok(FanoutResult {
            ranked,
            per_brain,
            recognition_token_cost,
        })
    }
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
        // non-increasing, ties broken by origin bytes then memory id.
        for w in first.ranked.windows(2) {
            let (x, y) = (&w[0], &w[1]);
            let ord = y
                .effective_score
                .total_cmp(&x.effective_score)
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
        let _ = henry.fan_out_recall("latency probe", &ctx, &cfg).unwrap();

        let start = std::time::Instant::now();
        let result = henry.fan_out_recall("latency probe", &ctx, &cfg).unwrap();
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
}
