# R34 — G4 proximity at full N · PREREG (2026-08-14)

**$0, retrieval-only oracle, LoCoMo full N = 1438, `topk_fts`, R19 labels.
Committed before the arm runs.**

R26 re-tested the RRF family at full N because "every retrieval verdict in
this series was measured at N=250" and one had already flipped. Its closing
list of what was NOT repaired names **G4 proximity** first. This closes that
item.

## Design

One arm against the existing full-N baseline:

- **A0** — the R32 baseline (`a0.jsonl`, already on disk, reproduces the
  published corpus record to the digit). Not re-run.
- **P** — identical config plus the **most favourable** setting from G4's
  N=250 sweep: `SPECTRAL_TOPK_PROXIMITY=0.40` at default fetch-mult
  (the arm that scored +1 evidence turn, the sweep's best). Testing the
  best-case arm is deliberate: if the *best* setting fails at full N, the
  whole grid fails a fortiori; if it passes, the N=250 sweep under-resolved.

Binary `ee00624` (tiebreaks; verified retrieval-identical to `b3375e8`
0/400 on both paths). Same dataset, same host as R32.

## Gate — same two-clause form as the series

PASS requires two-sided Wilcoxon on per-question evidence-turn counts
p < 0.05 **and** ≥ +2.0pp micro evidence-turn recall. Anything less is
recorded as the N=250 verdict standing at 5.75× the data.

## Prediction, recorded

**The refutation stands: |Δ| < 1.0pp, not significant.** The N=250 sweep was
not a marginal null — it was flat across two orders of magnitude of weight
with monotone degradation past w=1, and the mechanism story (proximity is
redundant with BM25 on short turns, blind to the no-lexical-bridge family)
is consistent with everything measured since (R32's diagnostic: 61% of
misses share zero words — proximity needs ≥2 shared terms to say anything).
The asymmetric risk R26 found (nulls hiding small effects) applied to
levers with 3 discordant pairs, not to a flat grid.

## Environment note

Runs concurrently with the R31 accuracy arms (local LLM on Metal). The
oracle is hash/recall-based and load-insensitive; wall-time is not a
metric here.

**Refs:** `g4-proximity-result-2026-08-08.md`,
`full-n-recheck-result-2026-08-09.md` (R26, which queued this).
