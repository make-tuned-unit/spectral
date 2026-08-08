# Preference evidence retrieval — stage 0 · **STOP** (2026-08-08)

**Preregistered in `preference-evidence-retrieval-prereg-2026-08-07.md`.** The
stage-0 screen fails on two of its four pinned criteria. **No paid run.** The
lever stays off by default.

The prereg's own stated expectation was null, and it is null. What this
converts is the inconclusive 2026-07-14 actor A/B into a decided retrieval
result: **`CascadePipelineConfig::fetch_mult = 3` does not move preference
evidence retrieval at all.**

## Preconditions — both required before the post arm could be read

| check | result |
|---|---|
| pre-arm reproduces the archived post-R16 arm | **0 / 500** contexts changed |
| pre-arm reproduces *itself* on a second run | **0 / 500** contexts changed |

Commit: R19/G4 working tree on `main`+`9978012` lineage; brains reused from
`~/spectral-local-bench/oracle-work`, no re-ingest. Pre-arm reproduces R15's
published figures exactly: overall **793/896 = 88.5%** micro evidence-turn
recall, `single-session-preference` **29/44** with **9** zero-evidence.

## Stage 0 result against the pinned thresholds

Single variable: `SPECTRAL_CASCADE_FETCH_MULT=3`.

| # | criterion | threshold | measured | |
|---|---|---|---|---|
| S0-1 | preference micro evidence-turn recall | ≥ +10.0pp | **+0.0pp** (29/44 → 29/44) | **FAIL** |
| S0-2 | preference zero-evidence questions | ≤ 6 | **9** (unchanged) | **FAIL** |
| S0-3 | overall micro evidence-turn recall | ≥ 793/896 | 796/896 (+3) | pass |
| S0-4 | mean context tokens | ≤ 15,634 | 14,124 (−0.6%) | pass |

Two of four fail, so this is a **STOP**. Per the prereg, a stage-0 stop is a
publishable result and may not be followed by "try a tweak" — any other lever
needs a fresh prereg naming what changed and why.

The preference row is **bit-identical** on every evidence metric. The lever is
not weak here; it is inert. It did change 223/500 contexts overall and moved
overall evidence turns by +3, so the arm ran and the variable was live — it
simply does nothing for the category it was predicted to help.

## Diagnostic: the gap is entirely ranking, not vocabulary

Since stage 0 authorizes no spend, the remaining budget went to a $0 question
worth more than the lever: *is the missing preference evidence reachable at
all?*

Forcing `topk_fts` and sweeping k on the same brains:

| k | evidence-turn recall | zero-evidence | context tokens |
|---|---:|---:|---:|
| **40 (shipped)** | 29/44 (65.9%) | **9** | 9,076 |
| 60 | 33/44 (75.0%) | 6 | 15,187 |
| 80 | 36/44 (81.8%) | 3 | 20,567 |
| 120 | 40/44 (90.9%) | 2 | 31,784 |
| **200** | **44/44 (100%)** | **0** | 56,714 |

**Every one of the 44 preference evidence turns is in the FTS match set.**
Nothing is lexically unreachable. The entire 34.1pp gap — including all 9
zero-evidence questions — is a **ranking** failure, and the evidence is spread
smoothly out to rank ~200 rather than clustered just past the cut.

This is a different failure from the LoCoMo deep misses measured the same day
(`g4-proximity-result-2026-08-08.md`), where 88.8% of below-cut evidence turns
contained at most one query term and were lexically thin. Preference evidence
is lexically *rich* and simply out-ranked. The two gaps do not share a fix.

**A perfect reranker over the existing match set would take preference from
65.9% to 100%** at unchanged token cost. That is the largest quantified
headroom the project has: it requires promoting items from rank ≤200 into the
top 40, a 5× compression.

Also worth recording: preference is the **cheapest** category (9,076 mean
context tokens vs 14,213 overall), so it has the most token headroom of any
category — which makes a shape-conditional k a natural candidate. It is **not**
tried here. It is a new lever and needs its own prereg, and the whole point of
the stage-0 discipline is that a failed gate does not license improvisation.

## What is recorded

- `fetch_mult=3` on the cascade route: **measured null on preference evidence
  retrieval**, decided rather than inconclusive. Default stays 1.
- The 2026-07-14 inconclusive actor A/B no longer needs re-running for this
  lever — there is no retrieval effect for an actor arm to detect.
- Preference retrieval headroom is **34.1pp and entirely rank-side**, with a
  measured ceiling of 100% over the current match set.

**Refs:** `preference-evidence-retrieval-prereg-2026-08-07.md` (the prereg),
`turn-level-evidence-recall-2026-08-07.md` (R15, the endpoint),
`g4-proximity-result-2026-08-08.md` (the contrasting failure mode),
`cascade_layers.rs:254-269` (the lever's prior).
