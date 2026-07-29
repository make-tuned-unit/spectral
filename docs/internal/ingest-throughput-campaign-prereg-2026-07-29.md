# PRE-REGISTRATION — ingest throughput campaign (2026-07-29)

Committed before optimization. Addenda only. The one systems axis where Spectral
loses ugliest vs the null alternative (MinHash+BM25 ~21k ev/s); the goal is to
narrow that gap without touching recall/recognition correctness.

## Measured baseline (write_path_cost, N=800, release, this machine)

| path | ms/mem | growth first→last bucket |
|---|---|---|
| `remember` UNCAPPED (legacy) | 14.78 | **12.5×** (2.4 → 30.6) |
| `remember` + cap | 5.56 | 3.4× (62% faster than uncapped) |
| `ingest_with` only | 3.47 | 1.9× |
| `MemoryStore::write` only | 0.12 | flat |
| recognition enroll only | ~3.1× growth | — |

**Diagnosis.** Derived-write overhead (remember − store-write) is **99% of the
public path**, and it grows *superlinearly* with corpus size — the steady-state
killer (a large brain sits deep in the 12.5× regime; that is why "~43 ev/s
steady-state" is so far below the ~288 ev/s that `ingest_with` alone would
allow). The cap recovers most of the superlinearity, implying an unbounded
fan-out (fingerprint peer-matching and/or recognition enrollment scanning a
set that grows with the corpus).

## Claims / questions under test

1. **Is the cap on by default in production?** (`max_fingerprint_peers`
   default became `Some(64)` in #231.) If yes, the shipped path is the 3.4×
   line, not 12.5× — establish the true current default number first.
2. **What remains superlinear under the cap?** Attribute the residual 3.4×
   growth to a specific stage (fingerprint match, recognition enroll, wing
   cache, FTS) via the per-stage breakdown.
3. **Can steady-state throughput improve ≥2× without correctness regression?**

## Pre-registered target

- **Primary:** steady-state (bucket 700..800) capped `remember` from **7.93
  ms/mem → ≤ 4.0 ms/mem** (≥2×), OR growth factor from 3.4× → ≤ 2.0×.
- **Hard constraints (any violation ⇒ revert):** every existing spectral-graph,
  spectral-ingest, spectral-recognition, and the deletion/recognition proof
  suites stay green. Recall answer-key parity on the Tier-0 oracle (byte-identical
  context hashes on a sample) — a throughput change must not alter what is
  retrieved. No new default that changes recall/recognition output.
- **Batch-write API** (`remember_batch`) is in-scope as an additive path if the
  per-transaction floor (B−C measurement) shows headroom; it must not change
  single-write semantics.

## Method

Profile-guided, measured each step: (a) confirm current default cap; (b) per-stage
timing under the cap to localize the residual growth; (c) one targeted change at
a time, re-running write_path_cost + the correctness gates after each; (d) keep
only changes that move the primary metric with all constraints green. Every
attempt (kept or reverted) recorded in the addendum.

## Out of scope

Recall-path latency (separate concern), federation write path, anything
requiring an index redesign (SQLite FTS5 + BM25 stays).
