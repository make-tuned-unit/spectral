# RESIDUAL_FLOOR — retrieval floor of the cases still failing after the admission fix

Date: 2026-06-15 | Branch: bench/stable-fail-2x2 | FREE analysis, no API calls.
Scope: the RETRIEVAL_STARVED cases that did NOT flip in PHASE3 (the fused-admission
Sonnet replay, commit c8521b1). Taxonomy + verdict only — NO fixes, NO recommendations.

## What this measures that prior reports did not

PHASE3 confirmed flips under **expansion-OFF + fused (RRF) admission** (contexts came
from `recall-snapshot --dump-contexts`, which runs expansion OFF). EXPANSION_DELTA
measured **expansion-ON + TACT-first admission**. Neither measured the configuration
the dispatch names as the evaluation target — **expansion ON *and* fused RRF admission**,
i.e. current main's shipped expansion plus the prospective `exp/cascade-admission` merge.
This report replays exactly that combined configuration and classifies, per still-failing
case, whether the load-bearing operand key(s) are reachable under it.

This is a **retrieval-floor** analysis: it asks whether the fact(s) the question needs
are *retrievable*, not whether the actor would then synthesize the answer. The combined
config has never been run through the actor, so no PASS/FAIL is claimed here.

## Method

Driver: `crates/spectral-bench-accuracy/src/bin/fused-expansion-replay.rs` (committed on
`exp/cascade-admission`, where the RRF binary lives). Per case it ingests a fresh
per-question brain (PerTurn), applies the v3 MS+SSP cleaned descriptions, classifies
`QuestionType` from the original question, and runs the real fused-admission
`retrieve_cascade`/`retrieve_topk_fts` over **the persisted Haiku expanded query** from
EXPANSION_DELTA Appendix A. Using the persisted expansion output (rather than a fresh
Haiku call) isolates the admission change from expansion's stochasticity — the expansion
term set is held fixed at the exact sample EXPANSION_DELTA recorded.

Per blocked operand it also probes `cascade_retrieve(expanded_query, 500)` to read the
key's position in the fused candidate ordering, separating "just past K" from "out of reach".

### Integrity

- **Fused-off path byte-reproduces the PHASE3 fused contexts: 26/26 cases, identical key
  sets.** The replay binary is the genuine RRF admission path, not a re-implementation.
- Critical operand keys are taken verbatim from MISSING_KEY_AUTOPSY's per-case
  answerability lines; each was re-checked for load-bearingness (below).

## The case set

20 RETRIEVAL_STARVED − 2 PHASE3 flips (0100672e, f35224e0) = **18 non-flipped cases.**
(The dispatch's "~17" is this set; the count is 18 including the GT-defect case 75f70248.
gpt4_5501fe77 is PENDING_OPUS / provisional and is excluded — it is not a confirmed
RETRIEVAL_STARVED case.)

## Per-case result (combined config: expansion ON + fused RRF)

| case | load-bearing operand(s) | status under exp+fused | recovered by |
|---|---|---|---|
| 0a995998 | — (all GT-critical already retrieved) | retrieval-complete | — (synthesis-bound) |
| 37f165cf | — (both operands already retrieved) | retrieval-complete | — (synthesis-bound) |
| 9ee3ecd6 | — (100-pt options already retrieved) | retrieval-complete | — (synthesis-bound) |
| gpt4_2ba83207 | — (Thrive $150 retrieved & only quantified spend → answer derivable) | retrieval-complete | — (synthesis-bound)¹ |
| 75f70248 | — (Luna / deep-clean in NO answer-session turn) | unanswerable | — (GT defect) |
| 157a136e | user age 32 (_1:t2, _1:t3) | **IN-K** | expansion |
| 2ce6a0f2 | lecture Mar 3 (_4:t0), Rachel Lee exhibit (_2:t2) | **IN-K** | expansion **and** fusion (d-displaced) |
| 51c32626 | Feb 1 date (_2:t8) | **IN-K** | expansion (TopkFts) |
| 8e91e7d9 | 3 sisters (_1:t2) | **IN-K** | expansion |
| 92a0aa75 | 2y4m tenure (_1:t0) | **IN-K** | expansion (TopkFts) |
| a1cc6108 | user age 32 (_1:t0, _1:t1) | **IN-K** | expansion (TopkFts) |
| c18a7dc8 | grad@25 (_1:t0) + 32yo (_2:t8), both | **IN-K** | expansion |
| gpt4_731e37d7 | $500 workshop (_2:t0) | **IN-K** | expansion |
| gpt4_d84a3211 | helmet $120 (_1:t6) + chain $25 (_2:t2) | **IN-K** | expansion |
| 6d550036 | solo Data Mining project (_2:t2) | **MISS** — pos 68 | — |
| gpt4_7fce9456 | condo Feb10 (_3:t6, IN-K) + pool condo (_3:t8) | **MISS** — _3:t8 pos 61 | partial |
| gpt4_15e38248 | wobbly-leg fix (_4:t3) + mattress (_3:t1) | **MISS** — pos 63 / pos 158 | — |
| ba358f49 | user age "I'm 32" (_2:t2) | **MISS** — pos 187 | — |

¹ gpt4_2ba83207: the question is "which store did I spend the most at." Thrive Market
($150) is retrieved and is the only *quantified* store spend, so the comparison resolves
from the retrieved set alone — the missing Instacart estimate (_4:t8, pos 147) is not
load-bearing. Reclassified from blocked to synthesis-bound on this re-check.

## Aggregate (18 non-flipped cases)

| bucket | count | cases |
|---|---|---|
| retrieval-complete / synthesis-bound (no load-bearing operand missing) | 4 | 0a995998, 37f165cf, 9ee3ecd6, gpt4_2ba83207 |
| GT defect (unanswerable from any retrieval) | 1 | 75f70248 |
| operand(s) RECOVERED under combined free config | 9 | 157a136e, 2ce6a0f2, 51c32626, 8e91e7d9, 92a0aa75, a1cc6108, c18a7dc8, gpt4_731e37d7, gpt4_d84a3211 |
| still retrieval-BLOCKED under combined free config | 4 | 6d550036, gpt4_7fce9456, gpt4_15e38248, ba358f49 |

**Only 4 of 18 cases remain retrieval-blocked once both already-built free levers
(shipped expansion + candidate fusion) are applied together.** The remaining 14 are not
retrieval-floored: 4 already have every load-bearing fact in context (their failure is
synthesis-side), 1 is a ground-truth defect, and 9 have their load-bearing operand
recovered.

### Where the recoveries come from

All 9 recovered cases are recovered by **expansion**, which is *already shipped on main*
(8 by expansion only; 2ce6a0f2 by expansion and fusion independently). None requires the
candidate fusion uniquely, and none requires the combination. The implication: the
RETRIEVAL_STARVED label was assigned against the **frozen expansion-OFF 2×2 contexts**
(answer-key recall computed on the 2026-06-04 pre-expansion run). Re-evaluated against
main's *actual* retrieval (expansion ON), those 9 cases are no longer evidence-starved —
the label overstates the floor that exists under shipped retrieval. The candidate fusion
adds the d-displaced cases (the 2 PHASE3 flips plus 2ce6a0f2); for the rest of the
still-failing set it contributes nothing new at the retrieval level.

## Classification of the 4 blocked cases (dispatch taxonomy)

| case | blocking operand(s) | pos@500 (fused, expanded) | autopsy class | classification |
|---|---|---|---|---|
| gpt4_7fce9456 | _3:t8 (pool condo) | 61 | (c) | **deep-rank** — 1 past K=60 |
| gpt4_15e38248 | _4:t3 (wobbly leg) | 63 | (b) | deep-rank — 3 past K=60 |
| gpt4_15e38248 | _3:t1 (mattress) | 158 | (c) | not K-reachable (expansion-recoverable in principle, sample missed it) |
| 6d550036 | _2:t2 (solo project) | 68 | (c) | **deep-rank** — 8 past K=60 |
| ba358f49 | _2:t2 ("I'm 32") | 187 | (a) | **vocabulary-mismatch** — no content-word overlap, deep |

Case-level (a case recovers only if *all* its load-bearing operands become reachable):

- **Deep-rank-recoverable** (the blocking operand sits just past K=60; a bounded K/admission
  reach would admit it): **6d550036** (pos 68), **gpt4_7fce9456** (pos 61) — 2 cases.
- **Mixed**: **gpt4_15e38248** — one operand deep-rank (pos 63), but the second (pos 158,
  class c) is not bounded-K-reachable; the case as a whole is not K-recoverable.
- **Hard vocabulary-mismatch** (no cheap fix): **ba358f49** (pos 187, class a) — 1 case.

The blocking deep-rank operands cluster tightly just past the K=60 cutoff — pos **61, 63,
68** in the fused candidate ordering *under expansion*. This is the signature of a key the
admission ordering ranks just below threshold, not one it cannot see.

## Caveats

- Retrieval-floor only. "RECOVERED" / "IN-K" means the operand is in the retrieved context;
  it does not mean the actor would pass. Several recovered cases (6d550036, gpt4_7fce9456,
  gpt4_15e38248 per the autopsy) also require interpretive enumeration the actor may still
  fumble — a synthesis question outside this report's scope.
- Expansion is stochastic. The recoveries and the pos-158 miss reflect the single persisted
  Haiku sample; a different sample would shift class-(c) keys.
- pos@500 is the position in `cascade_retrieve(expanded, 500)` (candidate ordering before
  episode-diversity/dedup); a key at pos 61–68 is just past the K cutoff but its final-set
  admission after re-ranking is not separately re-confirmed here.
- "current main + RRF admission": RRF admission is NOT yet on main; it is the
  `exp/cascade-admission` candidate. The combined config evaluated here is the prospective
  post-merge state, per the dispatch's "still failing AFTER the admission fix".

## VERDICT

**Free retrieval levers exhausted: NO.**

The residual is not dominated by vocabulary-mismatch. Of the 18 non-flipped cases, only 4
remain retrieval-blocked under the combined free config, and of those only **1**
(ba358f49) is a hard vocabulary-mismatch architectural floor. A meaningful share is
deep-rank-recoverable: **2 cases** (6d550036, gpt4_7fce9456) are blocked solely by a
load-bearing operand sitting at pos 68 and pos 61 — barely past K=60 — and a third
(gpt4_15e38248) is partly so. The deep-rank operands cluster at pos 61/63/68 in the fused
expanded ordering, which licenses one more bounded admission/K experiment to test whether
they can be lifted into the retrieved set.
