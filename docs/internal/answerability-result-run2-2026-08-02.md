# Answerability rerank — run 2 (pool-widened) — REJECTED

> **METRIC CAVEAT (R15, 2026-08-07):** "key-recall" in this document is
> evidence-**session** turn coverage — every turn of every `answer_` session, a
> ~12x-diluted denominator — not evidence-turn recall. See
> `turn-level-evidence-recall-2026-08-07.md`. This note does not assert what the
> correct metric would have shown here; the numbers below are left exactly as
> measured (Rule 5).

Prereg: `answerability-prereg-run2-2026-08-02.md`. Held-out LoCoMo, 120
questions, $0 oracle, zero LLM calls. Baseline is the run-1 baseline arm, which
reproduces the published held-out figures exactly.

## Verdict against the preregistered gates

| gate | rule | result |
|---|---|---|
| 1. Primary | session-recall ≥ +1.0pp **or** key-recall ≥ +1.0pp | **FAIL** — +0.1pp and +0.0pp |
| 2. No-harm, hard | overall session-recall must not regress; zero-recall ≤ 4 | PASS — 92.9% → 93.0%, zero-recall 4 → 4 |
| 3. Cost | mean context tokens ≤ +5% | PASS — 1603 → 1612 (+0.6%) |
| 4. Category no-harm | no category session-recall drop > 1.0pp | PASS (marginal) — temporal 100.0% → 99.2% (−0.8pp) |

**REJECTED on the primary gate.** Per prereg rule 6 this is one shot: no weight
or widening-factor tuning, recorded as a failure.

| | baseline | run 2 | Δ |
|---|---:|---:|---:|
| session-recall | 92.9% | 93.0% | +0.1pp |
| key-recall | 13.8% | 13.8% | 0.0pp |
| zero-recall | 4 | 4 | 0 |
| rank1 | 3.7 | 3.7 | 0.0 |
| tokens (mean) | 1603 | 1612 | +0.6% |

Pooled across questions: answer sessions hit **167/189 → 167/189** (net zero),
answer keys retrieved **515 → 516** (+1). The category movement is a pure
trade — multi-session +1 session, temporal −1 session.

(The oracle's summary macro-averages per question; pooling sessions directly
gives temporal 100.0% → 97.9%. The direction is robust, the magnitude depends
on the averaging. Gate 4 is evaluated on the canonical oracle summary, which is
the metric the published figures use.)

## The lever only engaged on half the corpus — second structural finding

| route | n | **set changed** | context changed |
|---|---:|---:|---:|
| Cascade | 84 | **0** | 0 |
| TopkFts | 36 | 36 | 36 |

The pool widening did not take effect on the cascade route, for a reason worth
recording:

`run_cascade_pipeline_scoped` ends with `results.truncate(config.k)`
(`cascade_layers.rs:441`). So `merged_hits` never contains more than `k` items,
and the harness's widening — `result.merged_hits.into_iter().take(k * widen)`
— is a **no-op**. Widening the cascade pool requires raising
`pipeline_config.k` before the call and truncating after, which also changes
what `max_per_episode` diversity operates over. Not a one-line change.

### This is a latent defect in an existing lever, not just in mine

`ACTR_POOL_WIDEN` uses the identical construction
(`retrieval.rs:781-786`). Therefore **ACT-R's pool widening is also inert on
the cascade route**. Combined with run 1's finding that session-grouped
rendering discards rank order, ACT-R on a cascade-routed question is doubly
inert: it cannot change membership, and its reordering never reaches the actor.

Any past ACT-R measurement that pooled cascade-routed and TopkFts-routed
questions was therefore diluted by whatever fraction was cascade-routed —
70% on this held-out set. This does not invalidate a specific published claim
(ACT-R is an off-by-default env lever and is not part of any published number),
but it means ACT-R's recorded behaviour should not be trusted without a re-run
that fixes the widening.

## Closing the family

Across two runs the query-conditioned answerability hypothesis is a **measured
null** under both available mechanisms of action:

- **run 1** — reorder a fixed set: −0.1 rank1 overall, failed the effect and
  no-harm gates; 36 improved / 23 regressed, sign-test p = 0.059.
- **run 2** — change membership from a widened pool: +0.1pp session-recall,
  +1 answer key across 120 questions, failed the primary gate.

The hypothesis was worth testing because it is genuinely different in kind from
everything in the rejected column — those are all *query-independent* levers
(K widening, fetch-mult, spreading, fingerprint tier, ACR), whereas this one
scores each candidate against the question. That difference did not produce
lift on the $0 gate.

**The retrieval-lever family is now closed on query-conditioned levers too.**
The record's conclusion stands and is strengthened: the ceiling is the
actor/synthesis stage, not retrieval ranking.

## What is kept and why

`spectral::answerability` stays in the tree, `enabled: false`, documented as a
measured null — the same treatment as other measured-and-not-shipped levers
(`fetch_mult`, `apply_declarative_boost`). Reasons to keep it:

- the scoring features are individually reusable (the acknowledgement penalty
  is a strictly better form of the renderer's `< 40` char hard drop, which
  destroys evidence rather than demoting it);
- re-deriving it to re-test under a different mechanism would cost more than
  keeping 300 lines behind a default-off flag;
- the unit tests pin the two non-obvious design decisions (uniform rank prior,
  membership preservation) that a future attempt would otherwise get wrong.

It must not be enabled by default without a paid, powered actor A/B — and the
ACR precedent (+18–40pp answer-key recall → **−2 accuracy**) says even that
could go either way.
