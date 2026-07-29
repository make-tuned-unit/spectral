# Tier-0 oracle: n-hop BFS graph channel + ACT-R activation rerank

**Date:** 2026-07-28
**Cost:** $0.00 (zero LLM calls; four 500-question oracle passes, brains reused)
**Branch:** `bench/oracle-bfs-actr`
**Prior:** retrieval is measured NOT to be the accuracy bottleneck (answers
retrieved in 15/16 failures) — the pre-registered expectation was a null.

## What was measured

Two research-derived retrieval candidates (agentic-memory research items 2
and 4), each env-gated and OFF by default, run through the standard Tier-0
gate (docs/internal/ORACLE_TIER0.md) against the kept oracle brains in
`~/spectral-local-bench/oracle-work`:

1. **`SPECTRAL_BFS_HOPS=N`** — after primary retrieval, expand the hit set by
   N-hop BFS over the brain's memory↔memory edge substrates, size-preserving
   (up to 10 admitted memories displace the weakest tail hits; seeds=5,
   ≤10 neighbors/node, frontier ≤50/hop; deterministic ordering by hop, edge
   weight, id).
2. **`SPECTRAL_ACTR_DECAY=d`** — ACT-R base-level activation
   `B = ln(Σ_j t_j^-d)` computed per hit, min-max normalized, blended into a
   position score at 0.2 weight over a 2× widened pool, truncated back to the
   original output size.

### Substrate reality check (measured before running)

- The only populated memory↔memory edge table in these brains is
  **`constellation_fingerprints`** (write-time temporal-proximity edges,
  ~70k rows / ~140 edges per memory in a 508-memory brain). BFS walks these
  via the new `fingerprint_neighbors` store/Brain API.
- **Co-retrieval edges (`related_memories`) are structurally zero** here
  (fresh brain, single query — same caveat ORACLE_TIER0.md records for
  co-retrieval ranking). BFS also walks them; they contribute nothing in this
  benchmark but would in Permagent-live brains.
- The **entity/triple graph is empty** (minimal `version = 1` ontology, no
  LLM extraction at ingest). This run therefore measures the
  fingerprint-graph BFS specifically, not an entity-graph BFS.
- **ACT-R signals:** `created_at` (one encoding event) plus
  `last_reinforced_at`/`hits` (reinforcement events, collapsed at the last
  reinforcement time). In these brains `last_reinforced_at` is NULL for every
  row, so B degenerates to `-d·ln(age)` — a pure power-law recency prior. The
  frequency component is structurally flat in this benchmark (same class of
  limitation as co-retrieval).

### Baseline drift note

The frozen 2026-07-02 `oracle-baseline.jsonl` is stale (library defaults have
moved since: `fetch_mult=3`, FTS description column, etc.), so a fresh
baseline was run in-batch; all comparisons are paired per-question against it.
With both env vars unset the code path is structurally identical to main.

## Results (n=500, paired per-question)

| arm | sess-rec | key-rec | zero-evid | rank1 | tok mean | tok p95 | retr ms (mean/p95) |
|---|---|---|---|---|---|---|---|
| baseline | 97.9% | 54.9% | 3 | 2.3 | 14,440 | 23,564 | 7 / 16 |
| bfs hops=1 | 97.9% | 52.2% | **5** | 2.2 | 13,483 | 21,941 | 45 / 67 |
| bfs hops=2 | 97.9% | 52.1% | **5** | 2.2 | 13,470 | 21,886 | 182 / 346 |
| actr d=0.5 | 97.9% | 54.9% | 3 | 2.7 | 14,644 | 23,564 | 8 / 15 |

### BFS hops=1 — NEGATIVE

- Contexts changed 477/500. Session recall: 7 improved / 6 regressed — but
  3 of the regressions lose a question's **only** answer session
  (zero-evidence 3 → 5: 75832dbd, gpt4_e061b84f, gpt4_4929293b introduced;
  06f04340 fixed).
- **Net answer keys −316** (key recall 54.9% → 52.2%); every category loses
  1.8–3.8pp key recall. Mean tokens −957 (displaced tail turns were longer
  than the admitted neighbors).
- Mechanism: fingerprint edges encode temporal proximity (same-day
  constellations), not query relevance. Size-preserving displacement trades
  bm25-ranked tail evidence for query-agnostic same-day neighbors picked by
  edge multiplicity. The 7 recovered sessions show the graph channel *can*
  reach sessions FTS missed, but the exchange rate is net-negative.

### BFS hops=2 — NEGATIVE (strictly dominated)

- Improved/regressed/zero-key question sets **identical** to hops=1; net keys
  −323 vs −316. The dense fingerprint graph saturates the capped candidate
  ranking at hop 1 (hop-1 candidates outrank all hop-2 candidates by
  construction), so the second hop buys nothing and costs 4× the latency
  (25× baseline).

### ACT-R d=0.5 — NULL (negative rank skew)

- Recall unchanged on the gating metrics: net keys −1, sessions ±1 question
  (gpt4_1e4a8aec up, 2ebe6c92 down), zero-evidence unchanged at 3.
- 167/500 contexts changed (the other 333 are actor-identical — free pass).
- The one directional signal is negative: rank-of-first-answer-key worsens on
  156 questions vs improves on 42 (mean 2.3 → 2.7) — the recency prior
  demotes older first evidence more often than it promotes recent evidence.
  +204 mean tokens (widened-pool composition shift).
- Consistent with the degenerate substrate: with no reinforcement history the
  lever is a second recency weight on top of the pipeline's existing recency
  signal.

## Verdicts

| lever | verdict | Tier-1 replay |
|---|---|---|
| `SPECTRAL_BFS_HOPS=1` | **NEGATIVE** | no — do not spend |
| `SPECTRAL_BFS_HOPS=2` | **NEGATIVE** | no — dominated by hops=1 |
| `SPECTRAL_ACTR_DECAY=0.5` | **NULL** | no — recall unchanged, rank skew negative |

The pre-registered expectation (null; retrieval is not the bottleneck) held
for ACT-R and was exceeded on the downside by BFS. Both levers stay OFF by
default and in the published configuration.

**What would change the verdicts:** BFS over a *relevance-bearing* edge
substrate — entity/triple edges (requires extraction at ingest) or
co-retrieval edges (requires retrieval history, i.e. the shared-brain oracle
mode or real Permagent trace replay ORACLE_TIER0.md already calls for). ACT-R
would need real reinforcement history for its frequency term to exist at all.
Both are Permagent-live measurements, not fresh-brain bench measurements.

## Artifacts

- Rows: `~/spectral-local-bench/oracle-bfs-actr/oracle-{baseline,bfs1,bfs2,actr05}.jsonl`
- Comparison script: `~/spectral-local-bench/oracle-bfs-actr/compare.py`
- Code: `fingerprint_neighbors` (spectral-ingest store + Brain),
  `apply_bfs_expansion` / `merge_displacing_tail` /
  `base_level_activation` / `apply_actr_rerank`
  (spectral-bench-accuracy `retrieval.rs`), unit tests for activation
  monotonicity (age and access-count), BFS cap/size-preservation/dedup, and
  an end-to-end fingerprint-edge walk.
