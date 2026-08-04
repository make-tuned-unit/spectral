# Preregistration — answerability rerank, run 2 (pool-widened) — 2026-08-02

**Written before the measurement.** Binding.

This is a **different experiment**, not a retry of run 1
(`answerability-result-run1-2026-08-02.md`). Run 1 reordered a frozen set; on
the cascade route the renderer then discarded that order, so on 84 of 120
questions it could not change the output at all. Run 2 changes the *mechanism
of action*: widen the candidate pool, rerank, then truncate — so answerability
selects **which memories are admitted**, which is the only thing that survives
session-grouped rendering.

The scoring function and its weights are **unchanged from run 1**. Only the
pool widening is new. This is deliberate: changing weights and mechanism at
once would make the result uninterpretable.

## Hypothesis

**H2:** given a 2x-wider candidate pool, query-conditioned answerability
selects a better-answering subset than BM25-plus-the-existing-reranker does at
the same k — improving answer-key recall, not merely rank.

Because membership now changes, the metrics that were invariant in run 1 become
live:

| oracle metric | run 1 | run 2 |
|---|---|---|
| session-recall | invariant | **live — primary** |
| key-recall | invariant | **live — primary** |
| zero-recall | invariant | **live** |
| rank1 | primary | secondary |
| context tokens | ~flat | **live — must be watched** |

## Decision rules (binding)

1. **Primary.** Held-out session-recall must improve by **≥ +1.0pp**
   (92.9% → ≥ 93.9%) **or** key-recall by **≥ +1.0pp** (13.8% → ≥ 14.8%).
2. **No-harm, hard.** Session-recall must not regress at all, and zero-recall
   must not increase above 4. A wider pool that admits worse candidates is a
   regression even if rank1 improves.
3. **Cost.** Mean context tokens may not rise more than **+5%** (1603 →
   ≤ 1683). The pool widens 2x; if the admitted set gets more expensive without
   getting better, that is the K-widening result again ("+36.5% context tokens
   buys zero new answer sessions") and it is a reject.
4. **Category no-harm.** No category's session-recall may drop by more than
   1.0pp.
5. **Default stays OFF regardless.** Clearing these gates makes this a
   candidate for a paid actor A/B. It does not make it a default, and it is not
   an accuracy claim. The ACR precedent is explicit: +18–40pp answer-key recall
   converted to **−2 accuracy**. Retrieval lift is not accuracy lift.
6. **One shot, no tuning.** If this fails, it is recorded as a failure. Weights
   and widening factor are frozen now: weights as run 1,
   `ANSW_POOL_WIDEN = 2`.

## Why the gates are set here

+1.0pp on 120 questions is ~1.2 questions — deliberately close to the noise
floor, so this gate is weak evidence at best even if it passes. It is set this
low because the honest prior is that it fails; a lever that cannot clear even
this should be closed out permanently rather than left as a maybe. Anything
claimed from a pass would need the paid run to mean anything.

## Prior

Lower than run 1's. The pool-widening family has been measured and rejected
twice already — K=60→80 admission widening (**REJECTED**, +36.5% tokens for
zero new answer sessions) and cascade `fetch_mult` (**Pareto-safe but a proven
accuracy no-op**). The difference here is *what selects* from the wider pool: a
query-conditioned scorer rather than the same query-independent one. That is a
real difference, and it is still probably not enough.

## Method

Identical to run 1: `locomo_heldout.json`, 120 held-out questions, $0 oracle,
zero LLM calls, `--fresh-brains --no-keep-brains`, single env lever
`SPECTRAL_ANSWERABILITY=1`. Baseline is the same run-1 baseline arm, which
reproduces the published held-out figures exactly.
