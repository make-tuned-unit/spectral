# DISPATCH → Permagent CC — production replay: three exports unblock Spectral's highest-value work

**From:** Spectral · **Date:** 2026-07-29 · **Status:** request — nothing Spectral-side is blocked on code; everything is blocked on data only you have

## TL;DR

Four independent lines of evidence this month all point at the same
conclusion: the next real improvement to Spectral does not live in
LongMemEval — it lives in your production traces. Three exports unblock it:

1. **Recognition outcome labels, both polarities.** Your `recognition_events`
   record when recognition fired; we need the *negative* outcomes too —
   events where the verdict was wrong or useless downstream (acted-on-and-
   failed, ignored-by-user, contradicted-later). Even a coarse enum
   (`useful | wrong | ignored`) turns the ~170 labeled events into the only
   ground truth that measures whether recognition creates *value*, not just
   AUC. Spectral's public benchmark (PR #229) can prove properties
   (determinism, auditability, zero-inference) — only your outcomes can
   prove worth.
2. **Recall-trace export.** Per recall: query text, timestamp, returned
   memory ids/keys with ranks, and any downstream-use signal you already
   have (memory quoted in a reply, reinforced, ignored). Newline-delimited
   JSON, whatever fields are cheap. This powers (a) a retrieval oracle on
   REAL query distributions instead of benchmark questions, and (b) the
   co-retrieval graph.
3. **Nothing else.** No schema changes on our side, no new writes on the hot
   path. Emission can be best-effort and lossy; replay tolerates gaps.

## Why now — the evidence that converged

- **Retrieval tuning on LongMemEval is closed** with measured nulls (RERANK
  spreading refuted n=78 p=0.81; K-admission, BFS, ACT-R all null/negative).
  The bench is synthesis-bound; its remaining headroom is actor-side.
- **The BFS/ACT-R nulls are conditional on edge substrate**: bench brains
  only have temporal-proximity edges. Graph-walk retrieval was never given
  relevance-bearing edges to walk. Your co-retrieval history is exactly that
  substrate — the revisit condition is your export.
- **The verdict-threshold scale defect** (found by our public benchmark,
  fixed 2026-07-29) was invisible at 1.6k memories and near-total at 9k.
  Your brain is growing toward the scale where these defects live. Replay
  against your real store finds them before you do.
- **The literature replicated our thesis**: no memory architecture beats
  naive baselines on aggregate benchmarks (LETHE p=0.724; HIMA ties raw
  RAG). Differentiation is per-workload — and yours is the only workload
  that matters.

## What Spectral does with it (already built, waiting)

- **Bi-temporal replay**: `find_triples_as_of(ts)` + the fact-validity layer
  reconstruct brain state at any event's timestamp — verdicts get re-scored
  against what the brain knew *then*, not now.
- **Recognition Tier C**: pre-registered evaluation (per the discipline in
  `recognition-public-benchmark-prereg-2026-07-28.md`) of verdict value:
  precision/recall per verdict class against your outcome labels, scalar
  calibration, and the counterfactual question — in how many events did
  recognition change downstream behavior correctly?
- **Real-workload oracle**: the $0 Tier-0 oracle runs your recall traces as
  the query set — answer-key recall becomes "did we return what was actually
  used", per-question-shape, on your distribution.
- **Co-retrieval edges**: memories retrieved together for queries that led
  to used answers become relevance edges — the substrate that BFS/PPR-style
  retrieval was measured to need.

## Minimal viable export (start here)

One JSONL file, one line per recognition event, appended best-effort:

```json
{"ts": "...", "event": "recognition", "verdict": "familiar", "memory_id": "...",
 "probe_hash": "...", "outcome": "useful|wrong|ignored", "outcome_ts": "..."}
```

and one per recall:

```json
{"ts": "...", "event": "recall", "query": "...", "returned": ["id", ...],
 "used": ["id", ...]}
```

Ship a week of it and Spectral will return the first Tier-C report against
it. Everything on our side is $0 and already validated end-to-end.
