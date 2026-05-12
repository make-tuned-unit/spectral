# Cascade Architecture Divergence: Decision Document

**Date**: 2026-05-11
**Branch**: `investigate/cascade-architecture-divergence`
**Status**: Research complete. Recommendation below.

---

## Section 1 — History

### Timeline

| Date | PR | Event |
|------|-----|-------|
| May 2 | #43 | Orchestrator born. `spectral-cascade` crate: `Layer` trait, `LayerResult` enum (Sufficient/Partial/Skipped), `Cascade` orchestrator. Layer impls: `AaakLayer` (L1), `ConstellationLayer` (L3). Token-budget stopping. |
| May 2 | #44 | Confidence stopping added. `confidence: f64` on all `LayerResult` variants. `CascadeConfig::confidence_threshold = 0.85`. L1 reports 0.95; L3 reports `signal_score.min(0.85)` (capped below threshold so L3 alone can't trip early stop). |
| May 3 | #46 | L2 `EpisodeLayer` shipped. Cascade is now L1->L2->L3. Episode grouping, dominance detection, `Sufficient` when top episode dominates. |
| May 3 | #48 | AaakLayer calibration fix. L1 was firing spuriously on bench data. Threshold tightened. |
| May 4 | #54 | `RecognitionContext` introduced. Ambient state primitive for context-conditional scoring. |
| May 4 | #55 | Ambient-conditional scoring across all three layers. L1 gates on context; L2/L3 apply `ambient_boost`. Architectural high-water mark for the orchestrator design. |
| May 5 | #59 | `recall_topk_fts` introduced as an alternative retrieval path. Single FTS K=40 with additive re-ranking. No orchestrator, no layers. Becomes bench default. |
| May 7 | #64 | Three bugs found in multi-layer cascade (45.8% vs 73.2% FTS baseline): redundant FTS calls in L2+L3, L1 empty-context skip, budget splitting between L2/L3. Attempted fix: rewired L3 as primary FTS, L2 consumes L3 output. |
| May 7 | `7e0448d` | **Pivot commit.** "Replaces the broken three-layer cascade" with single integrated pipeline. Old layer impls (`AaakLayer`, `EpisodeLayer`, `ConstellationLayer`) deleted. Pipeline: FTS K=40 -> ambient boost -> signal/recency re-ranking -> episode diversity -> dedup. |
| May 7 | #66 | Pivot merged. TACT added as retrieval entry point (replacing raw FTS). |
| May 7 | #67 | Unified re-ranking: both `topk_fts` and cascade share `apply_reranking_pipeline()`. Eliminates structural drift. |
| May 7 | #69 | Cascade fallback: when TACT returns < K, supplement with raw FTS. |

### What drove the pivot

The commit message at `7e0448d` is explicit. Three bugs:

1. **AAAK contamination.** L1 injected synthetic `__aaak__` blocks into the result set. The pipeline replaces this with ambient boost as a re-ranking signal.
2. **TACT max_results=5 cap.** L3 called TACT with a hard cap, limiting the cascade to 5 FTS results regardless of budget. The pipeline calls FTS directly with K=40.
3. **Single-episode truncation.** L2's episode grouping discarded non-dominant episodes entirely. The pipeline caps per-episode at 5 but interleaves rather than discards.

All three bugs were structural consequences of the Layer abstraction: each layer independently decided what to retrieve and how many results to surface, competing for a shared token budget. The fundamental problem was that retrieval and re-ranking were entangled inside each layer.

PR #64 tried to fix this within the orchestrator model (L3 does primary FTS, L2 consumes L3's output). This was architecturally awkward — it broke the Layer contract (layers should be independent) and introduced coupling between L2 and L3. The same-day pivot to a single pipeline was the cleaner resolution.

### Was it deliberate?

Yes. The commit message says "Replaces the broken three-layer cascade" and "Old layer abstractions removed." The `topk_fts` path (PR #59) had already demonstrated that single-pipeline-with-re-ranking worked well. The pivot extended that proven pattern to the cascade path.

---

## Section 2 — Architectural Comparison

### The orchestrator (`spectral-cascade/src/orchestrator.rs`)

**Design**: Ordered sequence of `Layer` trait objects. Each layer receives a query, a token budget, and a `RecognitionContext`. Each layer independently retrieves and scores memories, returning a `LayerResult`. The orchestrator collects results, deduplicates, and optionally stops early.

**Strengths**:
- Clean abstraction. Adding a layer means implementing one trait with one method.
- Early-stop logic: if L1 returns high-confidence Sufficient, L2/L3 never run. Saves work for easy queries.
- Per-layer telemetry: `layer_outcomes` records what each layer did, enabling fine-grained debugging.
- Independent testing: each layer has its own unit tests. Composition tested at orchestrator level.

**Weaknesses (demonstrated empirically)**:
- **Retrieval entanglement.** Each layer calls FTS independently. With 3 layers, that's 3 FTS queries for the same question. Redundant work, inconsistent result sets.
- **Budget competition.** Token budget is a shared resource. L1 consuming budget reduces what L2/L3 can surface. In practice L1 consumed disproportionate budget with low-value AAAK facts.
- **Layer coupling.** PR #64 showed that making L2 depend on L3's output broke the independent-layer contract. Layers can't be truly independent when they share the same underlying data store and retrieval mechanism.
- **Confidence semantics unclear.** L1 confidence (0.95 for "AAAK has facts") and L3 confidence (`signal_score.min(0.85)` for "best FTS hit quality") measure fundamentally different things. Max-across-layers conflates them.

### The single pipeline (`spectral-graph/src/cascade_layers.rs`)

**Design**: One retrieval pass (TACT + FTS supplement), then a sequence of re-ranking stages. All stages always run. No early stopping.

**Strengths**:
- **One retrieval, many re-rankings.** Eliminates redundant FTS calls. All stages operate on the same candidate set.
- **Composable signals.** Each re-ranking stage is a pure function: `Vec<MemoryHit> -> Vec<MemoryHit>` (conceptually). Adding a signal means adding a weight in `RerankingConfig`.
- **Predictable performance.** No conditional branching means consistent latency. Every query pays the same cost.
- **Proven by bench.** The `topk_fts` path (same architecture) reached 73.2% baseline; cascade pipeline inherited the same re-ranking infrastructure.
- **Unified with topk_fts.** Both paths share `apply_reranking_pipeline()` (PR #67). One re-ranking implementation, two entry points (TACT vs raw FTS).

**Weaknesses**:
- **No telemetry.** `max_confidence` is hardcoded (PR #88 finding). `layer_outcomes` is empty. `stopped_at` is always None. The pipeline produces results but no diagnostic metadata.
- **No early stopping.** Easy queries (L1-level AAAK matches) pay the same cost as hard queries. This is cheap today (all deterministic, no LLM) but would matter if heavier layers (vector search, LLM re-ranking) were added.
- **Ambient boost is a single stage.** The orchestrator's per-layer ambient conditioning (L1 gates, L2 boosts episodes, L3 boosts hits) is collapsed to one `ambient_boost_for_hit()` function. Finer-grained ambient logic would need to be added as more stages.

### Side-by-side

| Aspect | Orchestrator | Single Pipeline |
|--------|-------------|-----------------|
| Execution model | Ordered Layer trait calls with early-stop | One retrieval + sequential re-ranking stages |
| Result type | `LayerResult` (Sufficient/Partial/Skipped) | `Vec<MemoryHit>` |
| Confidence | Per-layer, max across layers | Not computed (bug: hardcoded 0.85) |
| Stopping condition | Token budget OR confidence threshold | Run to completion |
| Telemetry | `layer_outcomes`, `stopped_at` | Hits only |
| Extensibility | Add `Layer` impl | Add re-ranking stage to pipeline |
| Performance | Variable (early-stop saves work) | Predictable (always full pipeline) |
| FTS calls | One per layer (N layers = N calls) | One (TACT + FTS supplement) |
| Re-ranking | Per-layer, heterogeneous | Shared `apply_reranking_pipeline()` |

---

## Section 3 — Current State Assessment

### Do the original pivot conditions still hold?

**Yes.** The three bugs that drove the pivot are structural, not incidental:

1. **Retrieval entanglement** — Still true. The Layer trait's `query()` signature (`query: &str, budget: usize, context: &RecognitionContext`) gives each layer full autonomy over retrieval. There's no mechanism to share a candidate set across layers without breaking the abstraction.

2. **Budget competition** — Still true. Token budgets are a coarse control mechanism. The right budget split depends on the query, and we can't know the right split without running all layers first.

3. **Confidence conflation** — Still true. L1 confidence ("I have relevant AAAK facts") and L3 confidence ("my best FTS hit has a high signal score") measure different things. Max-across-layers doesn't produce a meaningful aggregate.

### What's changed since the pivot?

Since May 7 (pivot date), the recognition architecture has shipped:

| PR | Feature | Favors which architecture? |
|----|---------|---------------------------|
| #72 | Declarative density signal boost | Pipeline (new re-ranking weight) |
| #73 | Recall->recognition feedback loop (auto-reinforce, event logging) | Pipeline (operates on final result set) |
| #74 | Declarative density at ingest time | Neutral |
| #75 | Description field on Memory | Neutral |
| #76 | Co-retrieval index | Pipeline (co-retrieval is a re-ranking signal) |
| #79 | Session-aware retrieval events | Pipeline (session_id on context, events on result set) |
| #85 | Content-hash dedup | Neutral |
| #86 | Shape-routed actor strategies | Neutral (routing is above cascade) |

Every behavioral addition since the pivot has been a re-ranking signal or a post-retrieval operation — both fit naturally into the pipeline. None require per-layer independence.

### What about backlog items?

| Backlog item | Orchestrator fit | Pipeline fit |
|--------------|-----------------|--------------|
| #2 Co-retrieval signal in ranking | Awkward — which layer applies it? | Natural — new weight in `RerankingConfig` |
| #8 Compiled-truth boost | Awkward — L1 already has descriptions, L3 has different ones | Natural — boost any hit with `description.is_some()` |
| #11 Session signal in ranking | Awkward — session is ambient context, not a layer | Natural — new weight in `RerankingConfig` |
| #12 L2 episode summaries | Natural — this is what L2 was designed for | Possible — episode grouping is already a pipeline stage |

Item #12 (L2 episode summaries) is the only backlog item that maps cleanly to the orchestrator model. But the current pipeline already has episode diversity as a stage. The question is whether L2's "episode summarization" requires a fundamentally different execution model (retrieve episode, summarize, return summary as context) or can be expressed as a pipeline stage (group by episode, boost dominant episodes, format as session blocks).

The bench evidence suggests the latter: session-grouped formatting (PR #70) improved temporal accuracy without any orchestrator infrastructure.

---

## Section 4 — Forward Projection

### If we re-adopt the orchestrator

**Gains:**
- Proper telemetry: `layer_outcomes`, `stopped_at`, real `max_confidence`. The PR #88 bug goes away by construction.
- Early stopping for easy queries. Saves compute when L1 AAAK has the answer.
- Cleaner mental model for L2 episode summaries as a distinct recognition phase.

**Costs:**
- Must write new Layer impls. The old ones were deleted (PR #66). New ones would need to avoid the three bugs — meaning they can't independently call FTS.
- If layers share a candidate set (to avoid redundant FTS), the Layer trait needs redesign. The current signature `query(&str, budget, context) -> LayerResult` assumes independent retrieval.
- Every re-ranking signal added since the pivot (declarative density, co-retrieval, session signal) would need to be wired into per-layer logic or a shared post-layer stage.
- Regression risk. The pipeline's accuracy is known (73.3% cascade, higher on non-temporal). Re-architecting risks regressions that require bench runs to detect.
- PR #67's unification (shared `apply_reranking_pipeline`) would need to be preserved or rebuilt.

**Estimated effort:** 2-3 days for Layer trait redesign + new impls + wiring + regression bench. Non-trivial.

### If we stay single-pipeline

**Gains:**
- No rework. Proven architecture, known performance.
- Every behavioral backlog item (#2, #8, #11) integrates trivially as a `RerankingConfig` weight.
- Unified with `topk_fts` — one re-ranking codebase to maintain.

**Costs:**
- Need to fix telemetry separately. PR #88 identified the `max_confidence` bug; a targeted fix in `brain.rs` resolves it without architectural change.
- No early stopping. All queries pay full pipeline cost. Acceptable today (all deterministic), but if vector search or LLM re-ranking were added, this would need revisiting.
- `spectral-cascade` crate carries dead code (`Cascade`, `Layer`, `LayerResult`) alongside live code (`RecognitionContext`, `CascadeResult`, `LayerId`). Confusing for new readers.

**Estimated effort for cleanup:** 1-2 hours. Move live types out of `spectral-cascade` or document the orchestrator as dead code.

### If hybrid

The orchestrator becomes the outer shell; the pipeline becomes the inner retrieval mechanism. Concretely: `Cascade::query()` calls one "retrieval layer" that runs the full pipeline, then optional additional layers (L2 episode summarization) that operate on the result set.

This preserves the orchestrator's telemetry and stopping semantics while using the pipeline for retrieval. But it's contrived — the orchestrator adds ceremony (Layer trait, LayerResult wrapping, budget tracking) around what is effectively a pipeline with an optional post-processing step.

---

## Section 5 — Recommendation

**Stay with single-pipeline. Clean up the dead code.**

The evidence is clear:

1. **The pivot was correct.** Three structural bugs demonstrated that independent-retrieval layers are the wrong abstraction for a system with one underlying data store. The bugs weren't implementation mistakes — they were consequences of the architecture.

2. **Nothing has changed to invalidate the pivot.** Every feature shipped since May 7 fits the pipeline model. Every backlog item integrates more naturally as a re-ranking stage than as a Layer impl.

3. **The orchestrator's advantages are achievable without the orchestrator.** Telemetry can be fixed in `brain.rs` (PR #88 follow-up). Early stopping is a premature optimization when all stages are deterministic and sub-millisecond. Episode summaries can be a pipeline stage.

4. **Re-adopting the orchestrator would cost more than it delivers.** 2-3 days of rework plus regression risk, for telemetry and early-stopping benefits that can be achieved more cheaply within the pipeline.

### Concrete next steps

1. **Fix `max_confidence` in `brain.rs`** — Replace the `.min(0.85)` shim with a meaningful aggregate (e.g., weighted mean of top-N hit signal scores). This is the PR #88 follow-up. Scope: hours, not days.

2. **Clean up `spectral-cascade`** — Either:
   - (a) Move `RecognitionContext`, `CascadeResult`, and `LayerId` to `spectral-cascade` as the crate's public API. Delete `Cascade`, `Layer`, `LayerResult`, and the orchestrator module. Rename crate to something accurate (e.g., `spectral-recognition`).
   - (b) Or: add a `# Dead Code` section to the crate-level docs explaining that `Cascade`/`Layer`/`LayerResult` are historical. Less work, less clean.

3. **If L2 episode summaries (#12) are prioritized**, design them as a pipeline stage, not as a Layer impl. The pipeline already has `apply_episode_diversity()`. Episode summarization would be a new stage that runs after retrieval, groups hits by episode, and optionally generates summary context.

### Whitepaper divergence

The TACT whitepaper describes the orchestrator architecture: ordered layers (L0-L5) with per-layer confidence and early stopping. The implementation rejected that design based on empirical evidence. The whitepaper remains aspirational — it correctly identifies the *goals* (layered recognition, confidence-driven stopping, zero-LLM hot path) but prescribes an implementation strategy (independent Layer trait objects with per-layer retrieval) that doesn't work with a single underlying FTS store. The single-pipeline achieves the whitepaper's goals through different means: one retrieval pass, composable re-ranking stages, deterministic signals throughout.

The whitepaper should be updated to reflect the actual architecture. Leaving the orchestrator design as the documented architecture while the implementation pursues a different approach creates confusion for any reader who consults both. The update should preserve the whitepaper's conceptual framing (layered recognition, escalating cost) while describing how the pipeline implements it (one retrieval, staged re-ranking, configurable signal weights).

### L2 episode summaries: deferred design decision

This document recommends "episode summaries can be a pipeline stage" but does not resolve the L2 design question. There are two distinct capabilities conflated under "L2":

1. **Session-grouped formatting.** Retrieving individual memory hits and presenting them grouped by session/episode. PR #70 already does this. It's a formatting concern, not a retrieval concern, and fits trivially in the pipeline.

2. **Session-level summary retrieval.** Retrieving pre-computed episode summaries as first-class retrieval targets — what the TACT whitepaper's L2 originally described. This is a retrieval concern: summaries would be indexed alongside memories, and the pipeline would need to decide when to surface a summary vs. constituent memories.

These require different designs. (1) is already solved. (2) involves schema changes (summary storage), ingest changes (summary generation trigger), and retrieval changes (summary-vs-memory ranking). When backlog item #12 is dispatched, the design decision is whether (2) is a pipeline stage (summaries are just another candidate type in the re-ranking pool) or something structurally different. This document does not resolve that question — it only notes that the pipeline architecture does not preclude either approach.

### What would change this recommendation

- **If a heavy layer is introduced** (vector search, LLM re-ranking) that makes early stopping materially valuable, the orchestrator model becomes relevant again. But per the backlog, L4 vector search is "deferred indefinitely" and LLM-in-loop recognition contradicts the zero-LLM architectural commitment.
- **If pipeline telemetry proves insufficient** for debugging retrieval quality after the PR #88 fix, the orchestrator's per-layer outcome model could be worth revisiting.

Neither condition is imminent.
