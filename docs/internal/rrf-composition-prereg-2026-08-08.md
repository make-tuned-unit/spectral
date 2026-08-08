# R22 — RRF composition · PREREGISTRATION (written before any arm was run)

**$0. Retrieval-only oracle, LoCoMo, 250 answerable questions, R19 turn
labels. No model calls.** Written and committed before the first arm executed.

## Why this is the next lever

`failure-analysis-2026-08-08.md` priced perfect retrieval at **+23.34pp** and
then explained why six consecutive retrieval levers returned nulls. The
explanation was **not** that the signals are worthless. It was arithmetic about
the *composition*:

The base score is FTS **rank position**, `1 - i/n` (`ranking.rs:345-347`), and
every signal is an **additive** boost on top. So a boost's value measured *in
ranks* depends on the pool size. At the shipped `fetch_mult=3` (pool 120),
adjacent ranks differ by 0.0083, and the evidence turns we miss sit at median
BM25 rank 99 — promoting one into the top 40 needs **Δ = 0.492**. Recency
(0.10) + declarative (0.10) + entity (0.05) + proximity (0.15) = **0.40**, all
four firing maximally and in the same direction. **48 ranks moved, 59 needed.**

Widening does not rescue it. At pool 480 the budget reaches 192 ranks, but G4
measured recall *degrading* monotonically (72.7% → 61.5%) as weight rose,
because the boosts then override BM25 wholesale.

**Small pool: structurally too weak. Large pool: destroys the ordering.**

Reciprocal rank fusion has no scale term. Each channel contributes
`w / (K + rank)`, bounded and pool-independent.

## The reachability arithmetic, done in advance

The honest thing is to check *before* running whether RRF can even do the thing
additive boosts provably cannot. With `K = 60` and unit weights:

| candidate | BM25 rank | signal rank | fused score |
|---|---:|---:|---:|
| deep evidence turn | 99 | 1 | `1/159 + 1/61` = **0.02268** |
| shallow distractor | 1 | 200 | `1/61 + 1/260` = **0.02024** |

**The evidence turn wins.** That is the structural difference, and it is the
whole reason this measurement is worth taking: a signal that ranks evidence
first can promote it from rank 99 regardless of pool size. The additive scheme
cannot do this at any weight that does not also destroy the ordering.

## The symmetric risk, stated in advance

The same scale-freedom is the danger. Under RRF, a candidate BM25 ranks 1st is
only worth `1/61` — it can be displaced by a crowd of candidates the signal
ranks highly and BM25 ranks poorly. Declarative density separates evidence from
distractors by only **1.42×** (failure analysis §3), which is weak. A weak
signal given a full co-equal channel can plausibly do more damage than good.

**So a decrease here is a real and expected-frequency outcome, not a
measurement failure, and will be recorded as such.**

## Arms — fixed before running

All arms: LoCoMo `locomo_full_answerable_labelled.json`, 250 questions,
`per_turn`, shape routing (published configuration), `k = 40`, identical
brains, no re-ingest between arms. Single variable per arm.

| arm | environment | purpose |
|---|---|---|
| **A0** | *(none)* | baseline; **precondition**, must reproduce G4's k40 |
| **A1** | `SPECTRAL_RRF=1` | RRF over the default channels (BM25 + recency + signal) |
| **A2** | `SPECTRAL_RRF=1 SPECTRAL_TOPK_DECLARATIVE=1` | **PRIMARY** — RRF with the one signal measured to separate evidence |
| **A3** | `SPECTRAL_TOPK_DECLARATIVE=1` | additive control; isolates RRF from the signal |
| **A4** | `SPECTRAL_RRF=1 SPECTRAL_TOPK_DECLARATIVE=1 SPECTRAL_TOPK_PROXIMITY=0.15` | all deterministic signals fused |
| **A5** | A2 + `SPECTRAL_RRF_BM25_W=3` | BM25-dominant fusion; tests the displacement risk above |

A3 is the control that makes A2 interpretable. Under the additive scheme,
enabling declarative on `topk_fts` was already measured to buy **+3 evidence
turns**. If A2 ≈ A3, the composition is not the binding constraint and the
failure analysis's central claim is **wrong**.

## Primary metric and decision rule — fixed before running

**Primary:** evidence-turn **micro**-recall, `Σ evidence_turns_retrieved /
Σ evidence_turns_total`. Baseline is **231/356 = 64.89%** (macro 72.7%).

**Primary comparison:** A2 vs A0.

**Statistic:** exact two-sided McNemar over the paired per-question binary
`all evidence turns retrieved`, `scripts/paired_mcnemar.py`.

**PASS** requires *both*:
1. p < 0.05 two-sided, and
2. micro-recall increase ≥ **+2.0pp** (≥ +8 evidence turns).

Anything else is **NULL**. A significant *decrease* is **REFUTED** and is
published with the same prominence as a pass would be.

**Secondary, reported but not decisive:** macro recall, zero-evidence question
count (baseline 53/250), context tokens, and the multi-session slice — the
failure analysis identified multi-session as carrying +37.5pp with the worst
zero-evidence rate (31.2%), so it is the slice most likely to move.

## What would make this uninterpretable

- **A0 failing to reproduce G4's k40 arm** (231/356 micro, 53 zero-evidence).
  If the precondition fails, the run is void and nothing is claimed. G4's arm
  was measured on a different binary and brains are rebuilt here, so this is a
  real check, not a formality.
- Any arm differing from another in more than the one prespecified variable.

## Registered non-goals

- No end-to-end actor arm. **No paid runs.** If retrieval moves, the
  end-to-end claim stays unmade and is queued, not asserted.
- No tuning of `RRF_K` (fixed at 60, the Cormack et al. value already used by
  `sqlite_store::ranked_ids`). Sweeping it after seeing results would be
  exactly the garden-of-forking-paths this file exists to prevent.
- Per-channel weights: only the single prespecified A5 setting. No sweep.

**Register row:** R22. **Refs:** `failure-analysis-2026-08-08.md` (§4, §5),
`g4-proximity-result-2026-08-08.md`, `r19-locomo-turn-labels-2026-08-08.md`.
