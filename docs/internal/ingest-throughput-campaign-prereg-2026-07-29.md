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

---

## ADDENDUM — results (2026-07-29, this machine, N=800, release)

### Q1 — is the cap on by default? YES.
`BrainConfig::default()` sets `max_fingerprint_peers: Some(64)`
(brain.rs:224) and `IngestConfig::default()` sets `Some(64)` (ingest.rs:63).
The shipped `Brain::open` path is the **capped** line. The honest
current-default steady-state (bucket 700..800) is **7.79 ms/mem**, growth
**3.4×** — NOT the 12.5× uncapped legacy (which is only reachable by
explicitly calling `set_max_fingerprint_peers(None)`, as the bench's arm A
does). The 12.5× row is a synthetic "before"; it is not what production runs.

### Q2 — what remains superlinear under the cap? (per-stage attribution)
Temporary per-stage timers in `remember_with` (removed before commit),
capped path, µs/mem at bucket 0..100 → 700..800:

| stage | first | last | growth | share of last bucket |
|---|---|---|---|---|
| recognition **enroll** | 1387 | 4739 | **3.4×** | **63%** |
| **ingest** (`ingest_with`) | 764 | 2517 | 3.3× | 33% |
| declarative density | 77 | 120 | flat | ~2% |
| signature (fetch+sign) | 30 | 42 | flat | <1% |

Two growing stages. `recurrence_feedback` defaults to **false**, so the
ambient `recognize()` call is OFF by default — enroll is the only recognition
work per write. Enroll growth is intrinsic B-tree insert cost: each write
inserts ~116 pair + ~5 gram hashes into `WITHOUT ROWID` fingerprint tables
whose size grows with the corpus. Ingest growth was the fingerprint peer read
`list_wing_memories_capped`: `ORDER BY datetime(created_at) DESC, id DESC` —
`EXPLAIN` showed `USE TEMP B-TREE FOR ORDER BY`, i.e. the LIMIT 64 was
satisfied by sorting the **whole wing** (grows with corpus).

### Changes tried (before → after, capped remember bucket 700..800 ms/mem)

| # | change | 700..800 | ingest growth | verdict |
|---|---|---|---|---|
| baseline | (shipped default, cap=64) | 7.79 | 1.9× | — |
| 1 | recognition store PRAGMAs (`synchronous=NORMAL`, `temp_store=MEMORY`, `mmap_size`) to match memory store | 7.81 | 1.9× | **KEPT** (neutral on metric, but crash-durability + mmap parity; harmless) |
| 2 | **`idx_memories_wing_recency` expression index** `(wing, datetime(created_at) DESC, id DESC)` — removes the temp-B-tree sort; LIMIT satisfied by index walk. Byte-identical peer ordering. | **6.92** | **1.5×** | **KEPT** (−11%, the only metric-mover) |
| 3 | sorted-order inserts in `index_memory`/`index_minhash` (PK-order to reduce WITHOUT-ROWID B-tree page thrash) | 7.11 (noise) | — | **REVERTED** (no measurable effect at N=800; SQLite page cache absorbs it) |
| — | `remember_batch` | not built | — | **SKIPPED** — B−C floor is **negative** (store-write 0.12 < ingest 3.46 ms/mem): the cost is per-memory derived work, not per-transaction overhead, so batching cannot help steady-state. Prereg gated batch on "if the floor shows headroom"; it does not. |

### Final vs target
- Kept diff: change #1 (PRAGMAs) + change #2 (recency index). 18 net lines.
- Capped `remember` steady-state: **7.79 → 6.92 ms/mem (−11%)**.
- `ingest_with` growth first→last: **1.9× → 1.5×**.
- Capped `remember` growth first→last: 3.4× → **3.2×**.
- **Neither pre-registered target was met** (≤4.0 ms/mem OR ≤2.0× growth).

### Honest finding — the mechanism blocking ≥2×
The floor is structural: at bucket 700..800, `ingest_with` alone costs
~3.5 ms/mem and enroll's *first* bucket already costs ~1.2 ms/mem, so the
irreducible per-memory floor is **~4.7 ms/mem** even with a perfectly flat
enroll. Hitting ≤4.0 requires shrinking work that is fixed by the algorithms:
- **Enroll (63% of cost, 3.4× growth):** ~116 fingerprint inserts/memory is
  set by `fingerprint_stimulus` (landmark pairing). Reducing insert count,
  changing the `WITHOUT ROWID` PK, or bounding the pair fan-out all change
  **what is stored and matched** → alters `recognize()`/recall output →
  out of scope (write-path-only, recall-parity constraint).
- The remaining superlinearity is inherent B-tree insert cost on tables that
  must grow with the corpus. Bounding it needs a recognition index redesign
  (LSH-only / capped posting lists), explicitly **out of scope** per prereg.

The recency index is the correct, safe, kept win: it removes a full-wing sort
per write with byte-identical peer selection. All hard-constraint gates green
(spectral-graph / spectral-ingest / spectral-recognition suites incl.
deletion_guarantees + recognition invariants: 0 failed; fmt + clippy clean).
