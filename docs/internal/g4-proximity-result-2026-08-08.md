# G4 — term proximity · REFUTED with a mechanism (2026-08-08)

**$0. Retrieval-only oracle, 250 LoCoMo questions, R19 turn labels.** No model
calls. Capability shipped, **default off**.

Term proximity was the last classic IR signal never tested here, and the
argument for it was good: on 10–50-token memories BM25 degenerates — `tf` is
almost always 1, so saturation does nothing and ranking collapses toward
`Σ IDF`. Position is exactly what BM25 discards.

**The signal is real. It is also redundant and blind to our actual failures.**

## The target, established first

R19 made evidence-turn recall computable on LoCoMo. A paired k-sweep on the
same brains located the headroom:

| | ev-turn recall (macro) | zero-evidence | context tokens |
|---|---:|---:|---:|
| k=40 | 72.7% | 53/250 (21.2%) | 1,989 |
| k=500 | **89.7%** | 18/250 (7.2%) | 17,300 |

So the evidence is **already in the FTS match set** for most misses — it is
ranked too low, not absent. That is an admission/ranking problem, which is
precisely what proximity claims to fix. Only 7.2% is true vocabulary miss, the
irreducible lexical floor. Naive k-raising costs 8.7× the tokens, so the goal
was k=500 recall at k=40 cost.

## Result: null across the entire parameter space

`proximity = coverage × density` in `[0,1]`, added to the composite score.
Coverage is the fraction of distinct query terms present; density is 1.0 when
matched terms are adjacent, decaying as the smallest window containing them
widens. Stem-folded so it sees what porter-stemmed FTS admitted.

| arm | ev-macro | ev-micro | zero-evidence |
|---|---:|---:|---:|
| **baseline** (mult=3, no proximity) | **72.7%** | **231/356** | **53** |
| mult=12, no proximity | 71.8% | 228/356 | 55 |
| mult=3, w = 0.005 → 0.15 | 72.7% | 231/356 | 53 |
| mult=3, w = 0.40 | 73.1% | 232/356 | 52 |
| mult=12, w = 0.05 → 1.0 | 71.6–72.7% | 225–227/356 | 52–55 |
| mult=12, w = 2 | 68.0% | 213/356 | 62 |
| mult=12, w = 4 | 65.2% | 202/356 | 69 |
| mult=12, w = 8 | 62.8% | 192/356 | 76 |
| mult=12, w = 16 | 61.5% | 187/356 | 80 |

Best case is +1 evidence turn (noise). Large weights degrade **monotonically**.
Pool widening alone is also a wash — slightly negative — consistent with the
prior fetch-mult finding and with R20's mechanism (widening changes `n`, which
changes every `1 - i/n` base score).

## Why it fails — the part worth keeping

Proximity separates evidence from distractors **strongly** in isolation:

| | n | mean proximity | scoring exactly 0 |
|---|---:|---:|---:|
| evidence turns | 308 | **0.1197** | 39.9% |
| non-evidence turns | 87,799 | 0.0053 | 91.8% |

A **22.6× separation**. On that number alone this looks like a winning signal.
Two measurements explain why it buys nothing:

**1. It is redundant with BM25.**

| | mean BM25 rank |
|---|---:|
| candidates with proximity > 0 | **54.6** |
| all candidates | 192.5 |

**51.6% of proximity-positive candidates are already inside the top 40.**
BM25's `Σ IDF` already prefers documents where the query terms co-occur;
proximity mostly re-states that preference, and re-stating a preference cannot
change a ranking.

**2. It is blind to the failures that matter.** Of the 80 evidence turns
ranked *below* the k=40 cut — exactly the population that must be promoted:

- **71 (88.8%) score proximity 0.**
- 9 (11.2%) are visible to it.

Those turns contain **at most one query term**. There is no second term for a
window to be tight around. Proximity cannot promote what it cannot see, and
forcing it to try (large weights) displaces the 39.9% of evidence turns that
have no proximity either — which is exactly the monotonic degradation above.

## What this says about the next lever

The deep misses are **lexically thin, not badly positioned**. They match the
query on one common term and are ranked low for that reason. No positional
signal can fix that; the missing information is not *where* the terms are, it
is *which* terms count as the same concept.

That points away from the positional family entirely and toward coverage /
vocabulary bridging — and specifically not toward the rerank family, which is
where the query-conditioned levers were previously closed. Rocchio PRF over
the outcome ledger (G3) operates on **expansion**, which has never been tested.

## What shipped

- `ranking::proximity_score` — deterministic, allocation-light, unit-tested (8
  tests: adjacency ordering, coverage scaling, stem folding, short-stem guard,
  bounds, single-term inertness).
- `ranking::apply_reranking_pipeline_with_query` — an **additive** entry point.
  The pipeline was deliberately query-free (`answerability.rs`); every existing
  caller keeps query-free behaviour and `&[]` is exactly equivalent to the old
  function. This plumbing is the reusable part: query-conditioned signals now
  have a place to live, on both the topk and cascade paths.
- `RecallTopKConfig::apply_proximity` / `CascadePipelineConfig::apply_proximity`,
  **both default false**. Bench lever `SPECTRAL_TOPK_PROXIMITY=<weight>`.

Wired on the **cascade** path as well as topk, deliberately: `recall_cascade`
is the only path our real consumer calls (Permagent 08b), so a topk-only
measurement would not have spoken to production.

## Honest limits

- One corpus (LoCoMo), 250 questions, retrieval-only. No end-to-end actor arm
  was run — there is nothing to run one on, since retrieval did not move.
- `fold_token` approximates porter rather than implementing it. It errs toward
  folding too little, so a missed fold costs a boost and never invents one.
- The 22.6× separation is a real property of the corpus and is not refuted
  here. What is refuted is that it carries **incremental** information over the
  ranking BM25 already produces.

**Refs:** `r19-locomo-turn-labels-2026-08-08.md` (the metric that made this
measurable), `landscape-research-2026-08-07.md` §G4 (the hypothesis),
`REPAIR_REGISTER.md`.
