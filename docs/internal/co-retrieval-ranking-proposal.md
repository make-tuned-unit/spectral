# Co-Retrieval Signal in Cascade Ranking

**Date**: 2026-05-11
**Branch**: `feat/co-retrieval-ranking-signal`
**Backlog**: Item #2, Tier 1
**Status**: Proposal. Implementation follows after approval.

---

## Signal Definition

**Intuition**: Memories frequently retrieved together in the past are likely relevant to the same topics. If the top-ranked hits for a query have high co-retrieval affinity with a lower-ranked candidate, that candidate is probably more relevant than its FTS rank suggests.

**Mechanism**:

1. **Select anchor hits.** Take the top-3 candidates by initial FTS rank position (before re-ranking). These are the highest-confidence matches from the retrieval engine.

2. **Look up co-retrieval counts.** For each anchor, call `Brain::related_memories(anchor_id, 50)` to get the co-retrieval pairs with highest `co_count`. Collect all results into a map: `HashMap<String, f64>` where key = candidate memory ID, value = sum of `co_count` across all anchors.

3. **Normalize.** Divide all values by the maximum value in the map. Result: each candidate's co-retrieval affinity with the anchors, normalized to [0.0, 1.0]. Candidates not in the map get 0.0.

4. **Apply boost.** Additive: `scores[i] += co_retrieval_weight * normalized_affinity`.

**Why top-3 anchors, not all candidates?** Querying `related_memories` for all 40 candidates would be 40 DB queries (one per candidate). Top-3 limits to 3 queries while capturing the strongest signal — the highest-FTS-ranked hits are the most reliable anchors. Empirically, if a memory is co-retrieved with the top 3, it's strongly associated; co-retrieval with rank-30 hits is noise.

**Why additive, not multiplicative?** Multiplicative boosts (like ambient boost) can zero out scores or create runaway amplification. Additive keeps the signal bounded and predictable — co-retrieval can lift a candidate by at most `co_retrieval_weight` points. This follows the pattern used by entity boost and declarative density.

## Pipeline Integration Point

Current pipeline order in `apply_reranking_pipeline()`:

```
1. FTS rank normalization
2. Signal score blending (additive)
3. Ambient boost (multiplicative)
4. Declarative density (additive)
5. Recency decay (multiplicative)
6. Entity boost (additive)
7. Sort
8. Episode diversity (post-rank)
9. Context dedup (post-rank)
```

**Co-retrieval fires at position 4.5** — after declarative density, before recency decay:

```
4. Declarative density (additive)
4.5 Co-retrieval boost (additive)    ← NEW
5. Recency decay (multiplicative)
```

**Rationale**: Co-retrieval should fire after ambient boost and declarative density (both establish baseline relevance signals) but before recency decay (which is multiplicative and should apply to the co-retrieval-adjusted score). Placing it before the final sort ensures the signal contributes to the composite score alongside all other signals.

## New RerankingConfig Fields

```rust
pub struct RerankingConfig {
    // ... existing fields ...
    pub apply_co_retrieval_boost: bool,
    pub co_retrieval_weight: f64,
}
```

**Default**: `apply_co_retrieval_boost: false`, `co_retrieval_weight: 0.05`.

**Why `false` by default?** The `topk_fts` path doesn't currently pass co-retrieval data. Only callers that explicitly compute co-retrieval boosts should enable it. The cascade pipeline enables it in its `RerankingConfig` construction.

**Why 0.05?** Same magnitude as `entity_boost_weight`. This is a weak signal — enough to break ties and lift marginally-ranked candidates by 1-2 positions, not enough to override FTS rank or signal score. Conservative start; bench validates whether to increase.

## Pipeline Function Signature Change

```rust
pub fn apply_reranking_pipeline(
    candidates: Vec<MemoryHit>,
    config: &RerankingConfig,
    context: &RecognitionContext,
    co_retrieval_boosts: &HashMap<String, f64>,  // NEW
) -> Vec<MemoryHit>
```

**Why a separate parameter, not on RerankingConfig?** Co-retrieval boosts are runtime data (pre-computed from DB queries), not configuration. Mixing runtime data into a config struct is architecturally unclean. An empty HashMap means "no co-retrieval data available" — identical to `apply_co_retrieval_boost: false`.

**Helper function** for callers:

```rust
/// Compute co-retrieval boost map for candidates using top-N anchors.
/// Returns normalized [0.0, 1.0] affinity scores keyed by memory ID.
/// Empty map when co_retrieval_pairs index is empty or anchors have no data.
pub fn compute_co_retrieval_boosts(
    brain: &Brain,
    candidates: &[MemoryHit],
    anchor_count: usize,
) -> HashMap<String, f64>
```

Lives in `ranking.rs` alongside other ranking functions. Both `run_cascade_pipeline` and `recall_topk_fts` call this before invoking the pipeline.

## Graceful Degradation

| Condition | Behavior |
|-----------|----------|
| Empty `co_retrieval_pairs` table (fresh install) | `related_memories` returns empty vecs → empty boost map → no effect |
| `apply_co_retrieval_boost: false` | Stage skipped entirely |
| Empty boost map passed | Stage is a no-op (all candidates get +0.0) |
| Anchor has no co-retrieval data | That anchor contributes nothing to the map |
| Candidate not in map | Gets +0.0 (identity) |

**No new failure modes.** `related_memories` is already battle-tested (PR #76). The boost computation is pure arithmetic.

## Test Plan

### Unit tests (in `ranking.rs`)

**(a) Co-retrieval boost changes ranking.** Two candidates at similar FTS rank. One has high co-retrieval affinity with anchors (pre-computed map value 1.0), the other has none (0.0). With co-retrieval enabled, the high-affinity candidate should rank higher.

**(b) Empty map produces identical results.** Run pipeline with `apply_co_retrieval_boost: true` and an empty `HashMap`. Compare output to `apply_co_retrieval_boost: false`. Ordering must be identical.

**(c) Co-retrieval and ambient boost compose without swamping.** Candidate A has high ambient boost (wing match) but no co-retrieval. Candidate B has no ambient boost but high co-retrieval. Neither signal should completely dominate — both candidates should be present in the top results, with relative ordering determined by the composite.

### Integration test (in `brain_tests.rs`)

**(d) End-to-end co-retrieval in cascade.** Ingest memories, create retrieval events that establish co-retrieval pairs, rebuild index, then run cascade. Verify that co-retrieved memories rank higher than they would without the signal.

### Compute helper test

**(e) `compute_co_retrieval_boosts` normalization.** Verify output is normalized to [0.0, 1.0]. Verify anchor_count > candidates.len() doesn't panic.

## Telemetry

No new field on `MemoryHit`. The co-retrieval boost is folded into the composite `signal_score` alongside all other signals. Adding a per-hit `co_retrieval_boost_applied: f64` would require a schema change to `MemoryHit` for a debugging signal that's only useful during development.

Instead: the fact that `apply_co_retrieval_boost: true` is set in `RerankingConfig` is sufficient to know the signal was active. For deeper debugging, log the boost map size and max value in the pipeline (a one-line debug log, not a schema change).

## CascadePipelineConfig Change

```rust
pub struct CascadePipelineConfig {
    // ... existing fields ...
    pub apply_co_retrieval_boost: bool,
}
```

Default `true`. The cascade pipeline is the primary beneficiary of this signal.

## Call Site Changes

### `run_cascade_pipeline` (cascade_layers.rs)

```rust
// After retrieval, before pipeline:
let co_boosts = if config.apply_co_retrieval_boost {
    crate::ranking::compute_co_retrieval_boosts(brain, &candidates, 3)
} else {
    HashMap::new()
};
// ... pass co_boosts to apply_reranking_pipeline
```

### `recall_topk_fts` (brain.rs)

```rust
// topk_fts doesn't use co-retrieval in v1 (no ambient context)
let co_boosts = HashMap::new();
```

### Ranking tests

All existing tests pass empty `HashMap::new()` as the new parameter. No behavioral change.
