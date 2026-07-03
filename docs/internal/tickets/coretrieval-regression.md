# Ticket: co-retrieval reranking is a live production regression — disable/gate it

**Filed:** 2026-07-03 · **Severity:** high (degrades live retrieval) · **Repo:** spectral (Permagent pins by git rev)
**Evidence:** `docs/internal/LAST_LOOK.md`, `~/spectral-local-bench/coret-judge-real*-results.json`

## What

Co-retrieval reranking runs at `co_retrieval_weight = 0.10` and, measured on
Permagent's **real** query workload with a blind, non-circular LLM relevance judge,
makes the top-5 **worse**, not better:

| run | ON wins | OFF wins | ties | p |
|---|---|---|---|---|
| real queries, isolation cfg | 13 | 59 | 44 | ≈0 |
| real queries, production cfg | 17 | 56 | 47 | ≈0 |

(A content-derived *proxy*-query run showed the opposite, 81–37 — that was a
mirage; the real workload inverts it. Do not trust the proxy number.)

Cause: 728 of 744 logged `retrieval_events` returned the *same* ~40 memories, so the
co-access graph is a dense generic blob → popularity bias pulls generic-but-
irrelevant memories into the top-5 on short conversational queries.

## Fix (spectral repo)

The weight is hardcoded at three non-test sites:
- `crates/spectral-graph/src/cascade_layers.rs:170` (production cascade path)
- `crates/spectral-graph/src/brain.rs:1275` (topk_fts path)
- `crates/spectral-graph/src/ranking.rs:266` (`RerankingConfig::default`)

Preferred: add `co_retrieval_weight: f64` to `CascadePipelineConfig` (default
**0.0**), plumb it into the `RerankingConfig` built at cascade_layers.rs:170, and
change the other hardcoded `0.10` literals to `0.0`. This disables the regression
by default while keeping the signal available for opt-in retuning. Keep
`rebuild_co_retrieval_index` and `compute_co_retrieval_boosts` — only the default
weight changes. Update/keep the ranking unit tests (they can set the weight
explicitly).

Then in permagent-runtime: bump the `spectral` git `rev` in `Cargo.toml:88` to the
fixed commit.

## Confirm (cheap — query text is already logged)

Real query text is already persisted in `recognition_events.query`
(`spawn_persist_recognition`). After the change, re-run the blind judge
(`~/spectral-local-bench/coret_judge.py`) on freshly logged real queries to confirm
OFF ≥ ON. Optionally add a per-recall A/B outcome signal for continuous monitoring.

## Linked bug (file/fix in same PR)

`crates/spectral-tact/src/lib.rs:166` panics — slices a string at a non-char
boundary (`end byte index 235 is not a char boundary; inside '—'`) on real memory
content. This crashes the production `cascade_retrieve` path. Fix: use
`char_indices`/`floor_char_boundary`-style truncation, never a raw byte slice.
