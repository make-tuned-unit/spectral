# Cascade `max_confidence` Plateau Investigation

**Date**: 2026-05-11
**Branch**: `investigate/cascade-max-confidence-plateau`
**Status**: Bug confirmed. Fix deferred to follow-up PR.

## Summary

Every cascade run in the May 11 bench produces identical `max_confidence=0.85` and `stopped_at=None`. This is a real bug, not a statistical artifact or intentional behavior.

## Where `max_confidence` Is Computed

### The cascade orchestrator (not used)

`crates/spectral-cascade/src/orchestrator.rs:91-94`:
```rust
let max_confidence = layer_outcomes
    .iter()
    .map(|(_, r)| r.confidence())
    .fold(0.0_f64, f64::max);
```

This is the *correct* computation: it takes the max confidence across all layer outcomes. It is never reached in practice.

### What actually runs: `brain.rs` manual construction

`crates/spectral-graph/src/brain.rs:1189-1192` (`recall_cascade`):
```rust
let max_confidence = hits
    .first()
    .map(|h| h.signal_score.min(0.85))
    .unwrap_or(0.0);
```

Identical code at `brain.rs:1219-1222` (`recall_cascade_with_pipeline`).

Both methods bypass the `Cascade` orchestrator entirely. They call `run_cascade_pipeline()` directly, which returns `Vec<MemoryHit>` (not a `CascadeResult`). They then manually construct a `CascadeResult` with:

- `layer_outcomes: Vec::new()` — no layer data
- `stopped_at: None` — hardcoded
- `max_confidence` — clamped to 0.85

## Three Problems in the Computation

### 1. Hardcoded ceiling: `.min(0.85)`

The first hit's `signal_score` is clamped to 0.85. This value happens to match `CascadeConfig::confidence_threshold` (also 0.85), suggesting a copy-paste from the threshold constant. Since the top-ranked hit virtually always has `signal_score >= 0.85` after re-ranking, the output is always exactly 0.85.

### 2. Single-hit sampling: `.first()`

Only the first hit contributes. This ignores the distribution of scores across all results. A cascade that returns 60 high-quality memories produces the same `max_confidence` as one that returns a single borderline hit.

### 3. Bypassed orchestrator

`recall_cascade()` and `recall_cascade_with_pipeline()` never instantiate a `Cascade` or call `Cascade::query()`. The well-designed confidence computation in the orchestrator (max across layer confidences, early-stop on Sufficient) is dead code in practice. The `CascadeResult` is a compatibility shim, not a real cascade outcome.

## Empirical Evidence

### Distribution of `max_confidence` across May 11 bench run

| Category                 | n  | `max_confidence` values |
|--------------------------|---:|-------------------------|
| cost-check               |  5 | all 0.85                |
| diag-k20-cascade         | 20 | all 0.85                |
| knowledge-update         | 20 | all 0.85                |
| multi-session            | 20 | all 0.85                |
| single-session-assistant | 20 | all 0.85                |
| single-session-preference| 20 | all 0.85                |
| single-session-user      | 20 | all 0.85                |
| smoke                    |  1 | all 0.85                |
| temporal-reasoning       | 20 | all 0.85                |

**166 out of 166 cascade runs produce `max_confidence=0.85`.**

Every run also shows `stopped_at: null` and `layer_outcomes: []`, consistent with the orchestrator never being invoked.

## Conclusion: Bug (Hypothesis 1)

The `max_confidence` field in cascade telemetry is non-functional. It does not discriminate between high-quality and low-quality cascade runs. The value 0.85 is hardcoded via the `.min(0.85)` clamp, making it useless for:

- Confidence-based cascade stopping (the orchestrator's early-stop logic is bypassed)
- Quality assessment of retrieval results
- Comparison across question types or retrieval strategies

## Proposed Fix (Separate PR)

The fix has two possible scopes:

### Option A: Fix the shim (minimal)

Replace the `.min(0.85)` clamp with a meaningful aggregate — e.g., mean or max of all hit signal scores, or a coverage heuristic based on hit count vs. K. Remove the single-hit sampling. This preserves the current architecture where `recall_cascade()` bypasses the orchestrator.

### Option B: Wire through the orchestrator (structural)

Make `recall_cascade()` actually use the `Cascade` orchestrator with real `Layer` implementations. This would make `layer_outcomes`, `stopped_at`, and `max_confidence` reflect genuine cascade behavior. This is more work but aligns telemetry with the cascade design.

### Recommendation

Option A first — it's a targeted fix that unblocks meaningful telemetry without re-architecting the retrieval path. Option B can follow as a separate design decision about whether the multi-layer cascade should replace the single-pipeline approach.

## Impact on PR #86 Routing Decision

The routing rule "Temporal -> topk_fts" was introduced because cascade appeared to underperform on temporal questions. With `max_confidence` broken, we cannot use it to assess cascade quality. However, the routing decision was based on **accuracy** (70% cascade vs. higher FTS on temporal), not on `max_confidence`. The accuracy difference is real and independent of this bug.

Fixing `max_confidence` will not change cascade accuracy on temporal questions. It will, however, enable future work on confidence-driven routing (e.g., "if cascade confidence is low, fall through to FTS") which could replace the hardcoded routing rules.
