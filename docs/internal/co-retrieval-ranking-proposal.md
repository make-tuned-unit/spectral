# Co-Retrieval Signal in Cascade Ranking

**Date**: 2026-05-12
**Branch**: `feat/co-retrieval-ranking-signal`
**Backlog**: Item #2, Tier 1
**Status**: Proposal v2. Addressing review feedback.

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
4.5 Co-retrieval boost (additive)    <-- NEW
5. Recency decay (multiplicative)
```

**Rationale**: Co-retrieval should fire after ambient boost and declarative density (both establish baseline relevance signals) but before recency decay (which is multiplicative and should apply to the co-retrieval-adjusted score). Placing it before the final sort ensures the signal contributes to the composite score alongside all other signals.

## New RerankingConfig Field

```rust
pub struct RerankingConfig {
    // ... existing fields ...
    pub co_retrieval_weight: f64,
}
```

**Default**: `co_retrieval_weight: 0.10`.

No `apply_co_retrieval_boost: bool` flag. An empty `co_retrieval_boosts` map passed to the pipeline is the disable signal. The bool flag would be redundant ceremony.

**Why 0.10?** At 0.05, a candidate scoring ~0.7 gets a ~7% relative boost — likely below the threshold to cross K=20 cutoff positions that change actor consumption. At 0.10, the relative boost is ~14%, enough to move candidates 2-3 positions. Still bounded: max 0.10 additive vs FTS range 0.0-1.0. If bench shows more headroom, 0.15-0.20 is available without risking FTS override.

## Pipeline Function Signature Change

```rust
pub fn apply_reranking_pipeline(
    candidates: Vec<MemoryHit>,
    config: &RerankingConfig,
    context: &RecognitionContext,
    co_retrieval_boosts: &HashMap<String, f64>,  // NEW
) -> Vec<MemoryHit>
```

**Why a separate parameter, not on RerankingConfig?** Co-retrieval boosts are runtime data (pre-computed from DB queries), not configuration. Mixing runtime data into a config struct is architecturally unclean. An empty HashMap means "no co-retrieval data available" — the stage becomes a no-op.

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
| Empty `co_retrieval_pairs` table (fresh install) | `related_memories` returns empty vecs -> empty boost map -> no effect |
| Empty boost map passed | Stage is a no-op (all candidates get +0.0) |
| Anchor has no co-retrieval data | That anchor contributes nothing to the map |
| Candidate not in map | Gets +0.0 (identity) |
| Zero candidates | `compute_co_retrieval_boosts` returns empty map without panic |

**No new failure modes.** `related_memories` is already battle-tested (PR #76). The boost computation is pure arithmetic.

## Test Plan

### Unit tests (in `ranking.rs`)

**(a) Co-retrieval boost changes ranking.** Two candidates at similar FTS rank. One has high co-retrieval affinity (map value 1.0), the other has none (0.0). With co-retrieval, the high-affinity candidate ranks higher.

**(a2) Co-retrieval boost is directional.** Same setup as (a), but reverse the affinities (candidate that had 1.0 now has 0.0, and vice versa). Verify ordering reverses. Proves the boost actually moves things, not an artifact of initial ordering.

**(b) Empty map produces identical results.** Run pipeline with non-empty co-retrieval weight and an empty HashMap. Compare output to pipeline with same config. Ordering must be identical — empty map is identity.

**(c) Co-retrieval and ambient boost compose without swamping.** Candidate A has high ambient boost (wing match) but no co-retrieval. Candidate B has no ambient boost but high co-retrieval. Neither signal completely dominates — both present in top results, relative ordering determined by composite.

### Integration test (in `brain_tests.rs`)

**(d) End-to-end co-retrieval in cascade.** Ingest memories, create retrieval events establishing co-retrieval pairs, rebuild index, run cascade. Verify co-retrieved memories rank higher than without the signal.

### Compute helper tests

**(e) `compute_co_retrieval_boosts` normalization.** Verify output is normalized to [0.0, 1.0]. Verify anchor_count > candidates.len() doesn't panic.

**(e2) Empty candidates.** `compute_co_retrieval_boosts(brain, &[], 3)` returns empty map without panic.

## Telemetry

No new field on `MemoryHit`. The co-retrieval boost is folded into the composite `signal_score` alongside all other signals. For deeper debugging, the boost map size and max value can be logged at the pipeline level (a one-line debug log, not a schema change).

## Call Site Changes

### `run_cascade_pipeline` (cascade_layers.rs)

```rust
let co_boosts = crate::ranking::compute_co_retrieval_boosts(brain, &candidates, 3);
```

### `recall_topk_fts` (brain.rs)

```rust
let co_boosts = crate::ranking::compute_co_retrieval_boosts(self, &candidates, 3);
```

Both paths compute co-retrieval boosts. Co-retrieval is query-pattern affinity, not ambient context — excluding it from topk_fts would create a confound on bench (can't tell if Temporal stays flat because co-retrieval doesn't help or because we never enabled it).

### Ranking tests

All existing tests pass empty `HashMap::new()` as the new parameter. No behavioral change.
