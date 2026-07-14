# Lever incorporated: cascade candidate-pool widening (`fetch_mult`) — 2026-07-14

**A real, Pareto-safe retrieval lever.** The cascade path fetched only `k`
candidates before reranking, so any answer key ranked below FTS position `k` was
structurally unreachable — reranking cannot promote a candidate it never saw.
The topk_fts path already solved this (`RecallTopKConfig::fetch_mult = 3`: fetch
`3k`, rerank, take `k`). The cascade path lacked the parity. Now added:
`CascadePipelineConfig::fetch_mult` (default **3**).

## How it was found

Ran the $0 retrieval oracle on real LongMemEval-S and did failure analysis on
answer-KEY recall (the specific answer-bearing memories retrieved, vs
session-recall which was already ~98%).

1. **Baseline (multi-session, k=60):** session-recall 98.3%, answer-key recall
   50.4%. Looked like a large completeness gap.
2. **Raising output `k` recovered keys linearly** (k=60→200: key-rec 50→76%) but
   at +3× tokens (19.7k→61k) — a pure cost-for-recall line, no knee.
3. **Diagnostic that reframed the target:** the worst "gaps" were questions like
   *"How many different doctors did I visit?"* — **answer = 3**, but
   `answer_keys_total = 36` (the dataset marks every re-mention as evidence).
   Raw answer-key recall is therefore a *bloated* target: retrieving 9/36 turns
   that cover all 3 distinct doctors answers correctly. Session-recall (98%)
   already reflects true distinct-item coverage. **Chasing k→200 for "key
   completeness" would buy redundant evidence, not accuracy** — a token trap,
   avoided.
4. **The genuine lever** is not more output tokens but a wider *candidate pool*:
   let reranking (signal / recency / declarative) promote a buried-but-correct
   answer into the top-`k` at **constant output size**.

## Measured (real LongMemEval-S, $0 oracle)

`fetch_mult` sweep at fixed `k`, and the baked-in default (3) vs prior (1):

| category                   | session-recall 1→3 | tokens | note |
|----------------------------|:------------------:|:------:|------|
| single-session-preference  | **93.3% → 96.7%**  | 9598 → 9294 | recovered a fully-missed answer (rank1 None→29) |
| multi-session              | 98.3% → 98.3%      | ~neutral | saturated, no headroom |
| knowledge-update           | 98.7% → 98.7%      | ~neutral | saturated, no headroom |

**Pareto-safe by construction:** the widened pool is truncated back to `k` after
reranking, so context tokens track `k`, not the pool. It can only *add* a better
candidate the narrow fetch structurally excluded — never reorder the existing
top-`k` downward. Measured: never regresses; recovers buried answers wherever
session-recall has headroom; neutral where already at ceiling.

## What did NOT move it (ruled out by measurement)

- **`max_per_episode`** (episode-diversity cap): inert at `fetch_mult=1` because
  pool size == output size, so it only reorders, never changes membership.
- **Disabling signal / declarative / recency reranking:** zero effect on
  key-recall at `k=60, mult=1` — same reason (reorder ≠ re-select).
- **Pool widening on multi-session:** ~neutral (relevance-saturated; the buried
  keys are redundant re-mentions, per the diagnostic above).

## Incorporation

`CascadePipelineConfig::fetch_mult` default **3** (parity with topk_fts). Flows
to the bench's `cascade_profile()` routing (all shapes via `..default()`) and to
federation / `recall_cascade` default callers. Federation note: each child now
issues a `3k` FTS `LIMIT` instead of `k` — a bounded, cheap read-time change;
output and token cost are unchanged. Ablation override retained:
`SPECTRAL_CASCADE_FETCH_MULT`. Also added `apply_declarative_boost` to the config
(was hardcoded) so profiles can disable it; default unchanged (true).

Tests: the widening mechanism is unit-tested on the shared rerank pipeline
(`retrieval::tests::wider_fetch_pool_recovers_buried_high_signal_memory`, topk
path — same reranking code the cascade path now feeds); the cascade default is
locked by `cascade_layers::config_tests::default_fetch_mult_widens_pool_for_reranking`
and validated end-to-end on real LongMemEval via the oracle above. Regression:
graph lib 58/0, bench-accuracy retrieval 41/0.
