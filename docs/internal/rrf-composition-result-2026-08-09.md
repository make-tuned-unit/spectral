# R22 — RRF composition · REFUTED (2026-08-09)

**$0. Retrieval-only oracle, LoCoMo, 250 questions, `--retrieval-path
topk_fts`, k=40, R19 turn labels. No model calls, no paid runs.**
Preregistered at `a3b241d` before the first arm executed:
`rrf-composition-prereg-2026-08-08.md`.

**Capability shipped, default OFF. It stays off.**

## Verdict

The failure analysis named RRF "the fix we already own" and the highest-value
untested lever, on the argument that **the composition, not the signals**, was
the binding constraint. That argument is **wrong**, and this is the measurement
that shows it.

| arm | config | ev-micro | Δ | turns | zero-ev | McNemar p | verdict |
|---|---|---:|---:|---:|---:|---:|---|
| **A0** | baseline (additive) | **64.89%** | — | 231/356 | 53 | — | precondition ✔ |
| A1 | RRF, default channels | 58.99% | **−5.90pp** | 210 | 65 | **0.0004** | **REFUTED** |
| **A2** | **RRF + declarative (PRIMARY)** | 61.24% | **−3.65pp** | 218 | 65 | 0.0525 | **NULL, negative** |
| A3 | additive + declarative (control) | **65.73%** | +0.84pp | 234 | 52 | 0.2500 | null (as predicted) |
| A4 | RRF + declarative + proximity | 61.24% | −3.65pp | 218 | 61 | 0.0931 | null, negative |
| A5 | RRF + declarative, BM25 w=3 | 65.45% | +0.56pp | 233 | 54 | 0.3750 | null |

**The primary arm moved the wrong way.** A2 fails the prespecified PASS gate
(p < 0.05 **and** ≥ +2.0pp) on both clauses, and A1 clears the REFUTED bar
outright: a decrease significant at p = 0.0004.

**A0 reproduced G4's archived k40 arm exactly** — 231/356, macro 72.75%, 53
zero-evidence, 1,989 tokens, and an identical multi-session slice, with
**0 discordant pairs**. The precondition holds, so the run is valid.

## The mechanism worked. The hypothesis still failed.

This is the part worth keeping, and it is not "RRF is bad at promoting."

RRF **did** exactly what the prereg's arithmetic said it would. Additive boosts
could move a candidate 48 ranks when 59 were needed; RRF moved ranks freely and
in bulk:

| arm | first-evidence turn promoted | demoted | unchanged |
|---|---:|---:|---:|
| A1 | 71 | 76 | 36 |
| A2 | 82 | 64 | 34 |
| A5 | **109** | 41 | 45 |

And it reached genuinely deep evidence — **A2 rescued 5 questions that had zero
evidence under the baseline**, which is precisely the class the additive
composition was proven unable to touch.

It also destroyed more than it rescued:

| arm | zero-ev **fixed** | zero-ev **newly broken** | turns gained | turns lost | net |
|---|---:|---:|---:|---:|---:|
| A1 | 2 | 14 | 4 | 25 | **−21** |
| **A2** | **5** | **17** | **8** | **21** | **−13** |
| A5 | 1 | 2 | 5 | 3 | +2 |

**A2 breaks 3.4 questions for every one it fixes.** The promotion mechanism is
real; what it promotes is not evidence.

## Why — and it confirms §3 of the failure analysis while killing §4/§5

The failure analysis measured that, against the distractors outranking them,
missed evidence turns have query-term overlap **0.46×** and IDF-weighted
overlap **0.47×** — both *inverted*. Its own conclusion was that **BM25 is
ranking correctly by its own criterion**. That finding survives here intact.

What it then inferred — that a scale-free composition would let the other
signals rescue those turns — does not. Under RRF a candidate BM25 ranks first
is worth only `1/61`, so a crowd the signal ranks highly and BM25 ranks poorly
displaces it. Declarative separates evidence from distractors by just **1.42×**.
Handing a 1.42× signal a channel co-equal with the one signal that is *right*
costs more than it buys.

**A5 is the confirmation.** Raise the BM25 weight to 3 and recall climbs back to
65.45% — statistically indistinguishable from the 64.89% baseline, with the
most rank movement of any arm (109 promotions) and almost no damage (net +2).
The best thing RRF does is **stop being RRF**. There is no interior optimum
here; the frontier runs monotonically back toward BM25-only.

## The control is the decisive comparison

The prereg fixed this in advance: *"If A2 ≈ A3, the composition is not the
binding constraint and the failure analysis's central claim is wrong."*

A3 — the *additive* declarative arm — scored **+3 evidence turns**, reproducing
the previously measured additive result exactly. A2, the same signal under the
composition that was supposed to unlock it, scored **−13**.

A2 is not merely ≈ A3. **A2 is 16 evidence turns worse than A3.** The
composition was never the constraint. Given a composition that *can* promote
deep evidence, promoting deep evidence **makes retrieval worse**, because the
signals available to do the promoting rank the wrong things highly.

RRF also costs more: 2,241 tokens vs 1,989 (**+12.7%**) for −13 evidence turns.

## What this closes, and what it opens

**Closes:** the composition hypothesis, and with it the last item the failure
analysis ranked above signal quality. Six lexical levers, and now the
composition itself, have been measured. The residue is not "we composed the
signals badly." It is that **we do not have a signal that identifies
answer-bearing turns**, and no arrangement of the signals we do have will
manufacture one.

**Opens:** the two items that were queued *behind* RRF are now the front of the
queue, and they are both about acquiring a signal rather than rearranging
signals:

1. **Vocabulary bridging** for the ~34% of zero-evidence failures that are true
   lexical misses. `query_aliases` is a shipped deterministic channel and has
   never been tested. This is the only remaining $0 lever.
2. **Answer-shape matching** — "how many" preferring turns containing
   quantities. Unlike declarative density this is *query-conditioned*, so it is
   not another static prior on the corpus.

Multi-session stays the slice to beat: **44.70%** micro at baseline against
64.89% overall, and every RRF arm made it worse.

## Honest limits

- **One corpus, one path.** LoCoMo, 250 questions, `topk_fts` only. Retrieval
  only — no end-to-end actor arm, and none is queued: retrieval did not move
  in the good direction, so there is nothing to price.
- **`recall_cascade` is untested here** and it is the only path Permagent
  calls. This result does **not** license any cascade change. RRF stays
  default-off on both paths regardless, so nothing ships either way.
- **A2's p = 0.0525 is not significant.** It is reported as a null that trends
  negative, not as a refutation. A1 is the arm that is refuted. The direction
  is consistent across all three unweighted RRF arms, which is why the
  conclusion does not rest on A2 alone.
- **A0 is metric-identical to G4 but not byte-identical**: 64/181 context
  hashes differ on the shared subset, of which **63 are pure reorderings** of
  an identical key set and 1 is a genuine set change. That is the R16 tiebreak
  signature, and R16 landed between the two runs. Worth noting on its own —
  R16 was validated 0/500 on LongMemEval, so LoCoMo is the first corpus where
  it visibly moves ordering. It does not affect this comparison: all six arms
  share one binary.

## Reproducing

```bash
bash scripts/run_rrf_arms.sh          # six arms, one shared brain set, $0
python3 scripts/analyze_rrf_arms.py \
  --arms a0=a0.jsonl a1=a1.jsonl a2=a2.jsonl \
         a3=a3.jsonl a4=a4.jsonl a5=a5.jsonl --baseline a0
```

Two `rrf_fuse` defects were found by audit and fixed **before any arm produced
a row** (`b0ed077`), both recorded as Amendment 1 in the prereg: inert
candidates were being paid channel mass in memory-id order, and RRF silently
dropped the entity signal the additive path applies. Without the first, A4
would have been measuring id order. `rrf_fuse` had shipped with **no tests**;
six were added at `59fbdb4`.

**Refs:** `rrf-composition-prereg-2026-08-08.md`,
`failure-analysis-2026-08-08.md` (§3 survives, §4–§5 refuted),
`g4-proximity-result-2026-08-08.md`, `r19-locomo-turn-labels-2026-08-08.md`.
