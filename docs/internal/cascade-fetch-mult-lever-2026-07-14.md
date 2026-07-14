# Cascade candidate-pool widening (`fetch_mult`) — capability added, default OFF — 2026-07-14

**Status: capability shipped, default = 1 (off). Retrieval-Pareto-safe but NOT
end-to-end validated — do not re-default without a powered actor A/B.**

The cascade path fetched only `k` candidates before reranking, so any answer key
ranked below FTS position `k` was structurally unreachable — reranking cannot
promote a candidate it never saw. The topk_fts path already solved this
(`RecallTopKConfig::fetch_mult = 3`: fetch `3k`, rerank, take `k`). The cascade
path lacked the parity. Added: `CascadePipelineConfig::fetch_mult`.

## Why default OFF — proven accuracy no-op ($0 effective-mover analysis)

The lever is Pareto-safe on the *retrieval* metric, but that is a proxy. A
retrieval change can only alter an ANSWER on a question where fetch_mult=3 flips
the gold answer-bearing memory from **absent→present** in context (an "effective
mover"); everything else is a provable no-op. Counting effective movers is a $0
oracle metric — no actor spend needed to bound the lever's ceiling.

Across **156 single-session questions** (the categories with retrieval headroom;
multi-session/knowledge-update are ~98% saturated → ~0 movers by construction):

| category | n | session-recall | effective movers (can-help / can-hurt) |
|----------|:-:|:---:|:---:|
| single-session-user       | 70 | 100.0% | 0 / 0 |
| single-session-assistant  | 56 | 100.0% | 0 / 0 |
| single-session-preference | 30 |  ~95%  | **1** / 0 |
| **total** | **156** | | **1 / 0** |

fetch_mult can change the answer on **1 of 156** questions. That one
(`06f04340`) was tested deterministically (temp=0, both arms): fm=1→44 keys,
fm=3→47 keys, **actor answered "I don't know" in both → both wrong.** The one
possible flip does not convert.

Conclusion: **fetch_mult is an accuracy no-op** — not because the widening fails
(it improves session-recall) but because retrieval is not the binding constraint
where it has headroom. The single-session categories are already at/near 100%
session-recall (the answer is in the one relevant session; retrieval finds it),
and the residual failures are **synthesis** (actor gives generic / "I don't
know" answers with the content present) or **lexical retrieval misses** (query
"homegrown ingredients" vs doc "growing cherry tomatoes" — query *expansion*'s
job, not pool width). Per project discipline (consolidation −9.2pp, fusion null),
the capability stays (field + `SPECTRAL_CASCADE_FETCH_MULT` + ablation knobs) but
the default is 1.

**Cost of this verdict:** ~$0.16 (one 2-arm smoke test on the sole mover). The
effective-mover method turned a would-be ~$5–40 actor sweep into a $0 count plus
a spot check.

**Earlier inconclusive run (superseded):** a first n=30 A/B on
single-session-preference showed fm=3 at 14 fails vs fm=1's 11, but was invalid —
unpinned actor temperature (=1.0 sampling noise), 4+2 transport failures counted
as wrong, and 29/30 questions were provable no-ops for the lever anyway. It is
what motivated the temp=0 harness pin and the effective-mover method.

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

## Capability (default off)

`CascadePipelineConfig::fetch_mult` default **1** (off). When set >1 it flows to
the bench's `cascade_profile()` routing (all shapes via `..default()`) and to
federation / `recall_cascade` callers; each child then issues a `mult×k` FTS
`LIMIT` (bounded, cheap; output/token cost unchanged as the pool truncates back
to `k`). Opt-in override: `SPECTRAL_CASCADE_FETCH_MULT`. Also added
`apply_declarative_boost` to the config (was hardcoded) so profiles can disable
it; default unchanged (true). Harness fix landed alongside: the actor and judge
now pin `temperature: 0` (an unpinned actor is what made this A/B inconclusive).

Tests: the widening mechanism is unit-tested on the shared rerank pipeline
(`retrieval::tests::wider_fetch_pool_recovers_buried_high_signal_memory`, topk
path — same reranking code the cascade path now feeds); the cascade default is
locked by `cascade_layers::config_tests::default_fetch_mult_widens_pool_for_reranking`
and validated end-to-end on real LongMemEval via the oracle above. Regression:
graph lib 58/0, bench-accuracy retrieval 41/0.
