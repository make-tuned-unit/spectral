# Answerability rerank — run 3 (fixed foundation) — REFUTED, hypothesis closed

Prereg: `answerability-prereg-run3-2026-08-02.md`. Held-out LoCoMo, 120
questions, $0 oracle, zero LLM calls, 4 arms.

## Results

| arm | config | sess-rec | key-rec | zero | tokens |
|---|---|---:|---:|---:|---:|
| **A** | baseline | 92.9% | 13.8% | 4 | 1603 |
| **B** | answerability | 93.2% | 14.1% | 4 | 1625 |
| **C** | answerability + rank render | 93.2% | 14.1% | 4 | 1625 |
| **D** | rank render only | 92.9% | 13.8% | 4 | 1603 |

## Verdict against the preregistered gates

| gate | rule | result |
|---|---|---|
| 1. Primary | session-recall or key-recall ≥ +1.0pp | **FAIL** — +0.3pp and +0.3pp |
| 2. Integrity (arm D) | rank rendering must not change set metrics | **PASS** — D identical to A; 0/120 set diffs |
| 3. Cost | ≤ +5% tokens | PASS — +1.4% |
| 4. Attribution | C-vs-B difference explained by arm D | **PASS** — see below |

**REFUTED on the primary gate.** Per prereg rule 6 this was the final attempt:
the query-conditioned answerability hypothesis is **closed permanently**. No
fourth run, no weight sweep.

## The foundation fixes worked — the hypothesis still failed

This run separates "the lever was broken" from "the lever doesn't work". Both
foundation fixes are proven effective:

| fix | before | after |
|---|---|---|
| **F1** cascade pool widening (`pipeline_config.k`, not a post-hoc `take`) | lever changed the set on **36/120** questions (TopkFts only) | **114/120** |
| **F2** `SessionOrder::ByRank` | rank reached the actor on **0/84** cascade questions | **84/84**, with **0** set changes |

Effect size tracked the fixes exactly — +0.1pp (run 2, lever mostly inert) →
+0.3pp (run 3, lever fully active). It tripled, and is still more than 3x below
the gate. The lever is not broken. It does approximately nothing.

## Attribution: C vs B

C and B are identical on every oracle metric, and that is expected, not a null
result. Context-hash diffs, C vs B:

| route | changed |
|---|---|
| Cascade | **84/84** |
| TopkFts | 0/36 (uses flat rendering, where rank order already survived) |

So `ByRank` did change the actor-visible text on exactly the route where rank
was previously discarded — while changing the retrieved set on zero questions.

### The $0 oracle cannot measure the rendering fix

Oracle metrics (session-recall, key-recall, rank1) are computed from
`retrieved_keys` — the set and its order. Session *ordering in the rendered
output* is invisible to all of them by construction. **F2's value is therefore
unmeasurable at $0** and is not claimed here as a win. What it does is remove a
structural reason a whole class of lever could not work; whether reordering
sessions helps an actor is a paid question.

That is also the honest limit of arm D: it is an integrity control, not
evidence of benefit.

## Closing the family

Three runs, three mechanisms of action, all null:

| run | mechanism | result |
|---|---|---|
| 1 | reorder a fixed set | −0.1 rank1; 36 better / 23 worse, sign test p=0.059 |
| 2 | change membership from a widened pool | +0.1pp session-recall (widening was inert on cascade) |
| 3 | both, on the fixed foundation | +0.3pp session-recall, +0.3pp key-recall |

The hypothesis was worth testing: every lever in the rejected column is
*query-independent* (K widening, fetch-mult, spreading, fingerprint tier, ACR),
while this one scores each candidate against the question — the one untested
family, and the deterministic analogue of the cross-encoder that measured best
of anything tried. It does not produce lift on the $0 gate.

**The retrieval-lever family is now closed on query-conditioned levers too.**
The record's conclusion stands and is strengthened by having survived a test
designed to break it: the ceiling is the actor/synthesis stage, not retrieval
ranking.

## What is kept

- `spectral::answerability` — `enabled: false`, documented as refuted. Kept for
  the same reasons as other measured-and-not-shipped levers: the features are
  individually reusable, and its tests pin two design decisions a future
  attempt would otherwise get wrong (uniform rank prior; membership
  preservation).
- **F1 and F2 are kept unconditionally.** They are correctness fixes
  independent of this hypothesis. F1 makes every future admission lever
  measurable on the cascade route; F2 makes every future rerank lever able to
  reach the actor there. Both default to the previous behaviour and are proven
  byte-identical with levers off.

## Reproduce

```bash
DS=locomo_heldout.json
./target/release/spectral-bench-accuracy oracle --dataset $DS --label A --fresh-brains --no-keep-brains
SPECTRAL_ANSWERABILITY=1                             ... --label B
SPECTRAL_ANSWERABILITY=1 SPECTRAL_RENDER_BY_RANK=1   ... --label C
SPECTRAL_RENDER_BY_RANK=1                            ... --label D
```
