# Retrieval foundation — F1–F3 — 2026-08-02

Three changes that make the retrieval path measurable and reproducible. All
three default to prior behaviour and are verified byte-identical with levers
off. None is an accuracy claim.

## F1 — candidate-pool widening now reaches the pipeline

**Defect.** The harness widened by slicing a bigger window off the results:
`result.merged_hits.into_iter().take(k * widen)`. But
`run_cascade_pipeline_scoped` ends with `results.truncate(config.k)`
(`cascade_layers.rs:441`), so the pipeline never returns more than `k` and the
slice was a **no-op**.

**Consequence.** Every admission lever routed through that code was inert on
the cascade route — the default for every non-Temporal shape. This includes
the pre-existing `ACTR_POOL_WIDEN`, so **ACT-R's pool widening was also inert
there**. ACT-R on a cascade-routed question could not change membership, and
(per F2) its reordering never reached the actor either.

**Fix.** Widen `pipeline_config.k` *before* the pipeline call; truncate to
`output_k` after the rerank stage. Widening `k` also widens what
`max_per_episode` diversity operates over, which is a genuine behaviour change
— so `widen = 1` whenever every rerank lever is off, keeping the default path
identical.

**Verified.** Oracle, held-out LoCoMo, all levers off: **0/120** context-hash
diffs, 0/120 retrieved-key diffs. With the answerability lever on, the set
changed on **114/120** questions versus **36/120** before the fix.

## F2 — rank can now reach the actor

**Defect.** `render::session_grouped` groups by `episode_id`, sorts turns by
`key` and orders groups by date. **Rank is never consulted.** Whatever order
retrieval produced was discarded before the actor saw anything.

**Consequence.** On the cascade route, a rerank changed the rendered context on
**0 of 84** held-out questions. No rerank-shaped lever could convert to
accuracy there, because its effect was erased.

**Fix.** `render::SessionOrder::ByRank` orders sessions by the best (lowest)
rank of any hit they contain. Turn order *within* a session stays chronological
— a session is a conversation, and reading it out of sequence would cost more
than the ranking gains. Ties break on session id (the sort discipline from
PR #238). `Chronological` remains the default and the published configuration.

**Verified.** Arm D of the run-3 A/B (rank rendering alone): rendered context
changed on **84/84** cascade questions with **0** change to the retrieved set on
any of the 120.

**Limitation, stated plainly.** The $0 oracle computes its metrics from
`retrieved_keys` — the set and its order. Rendered *session order* is invisible
to all of them by construction, so **F2's benefit is unmeasurable at $0**. It
removes a structural blocker; it is not itself evidence of benefit. Whether
reordering sessions helps an actor is a paid question.

## F3 — one pipeline from question to actor context

`Brain` exposes ~15 retrieval entry points, each composing its own reranking.
`recall_topk_fts` gets fetch-pool widening, entity clustering, context dedup and
a time anchor; `recall_at` — which `recall_local` calls, and the obvious entry
point for a new consumer — gets none of them. Improvements did not compound, and
reproducing the published configuration meant assembling the policy, choosing
the matching `Brain` method, and rendering the way the harness does.

`spectral::retrieve` is the single path:

```
plan → candidates → rerank → truncate → render
```

`RetrievePlan::v1(question, visibility)` is the published configuration in one
call. `Retrieved` carries hits, rendered lines, the shape, the route, and how
many candidates were considered.

Additive: nothing is deprecated, no existing method changes behaviour.

**Parity gate** (`spectral-bench-accuracy/tests/pipeline_parity.rs`): the
library pipeline and the harness produce **identical retrieved keys and
identical rendered lines** on the cascade route, over a multi-session fixture.
A second test pins that the published plan enables no unproven lever —
answerability off, chronological rendering, no cap, no descriptions, no
offsets — so a future default flip cannot quietly smuggle a measured null into
the configuration behind the published number.

Also added: `RetrievePlan::with_time_anchor`, which sets the cascade context's
`now` **and** the top-k config's `now` together. Previously each route anchored
recency separately and a caller could set one and forget the other — a quiet
bug class, because the result still looks plausible.

## What these did not do

They did not improve accuracy, and no arm of any run claims they did. What they
changed is that retrieval experiments are now *capable* of being measured on
the default route, and that a consumer can execute the published configuration
without reconstructing it.

The lever they unblocked — query-conditioned answerability — was measured on
the repaired foundation and **refuted** (`answerability-result-run3`). That is
the honest outcome: the foundation was genuinely broken, fixing it genuinely
changed what the lever could do (+0.1pp → +0.3pp), and the lever still does
approximately nothing.
