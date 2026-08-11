# R26 — do the N=250 verdicts survive at full N? · PREREGISTRATION

**$0. Retrieval-only oracle, LoCoMo, full N = 1,438, `--retrieval-path
topk_fts`, k=40, R19 turn labels. No model calls, model-free.** Written and
committed before any arm executed.

## Why this exists

R24 proved the sample was the problem, not the statistic: the **same lever** on
the **same corpus** returned `+1.69pp, p=0.25, NULL` at N=250 and
`+2.76pp, p<0.0001, PASS` at N=1,438.

Every retrieval verdict in this series was measured at N=250, a subset
inherited from G4, never justified, and **~5pp easier than the corpus it was
drawn from** (64.89% vs 59.86%). Those verdicts are published in
`MEASURED_RECORD.md` as measured results. **At least one of them is now known
to have been wrong.**

This run asks which of the rest survive. It is a repair of our own record, and
it is designed so that it can embarrass us.

## What is being re-tested, and why these

| arm | lever | N=250 verdict | why re-test |
|---|---|---|---|
| **A1′** | `SPECTRAL_RRF=1` | **REFUTED**, −5.90pp, p=0.0004 | our top-priority hypothesis; a *significant* effect, so most likely to survive — but it is the claim with the most riding on it |
| **A2′** | `SPECTRAL_RRF=1 SPECTRAL_TOPK_DECLARATIVE=1` | NULL, −3.65pp, p=0.0525 | R22's **primary** arm, and its p sat just above α — exactly the profile R23 had |
| **A3′** | `SPECTRAL_TOPK_DECLARATIVE=1` | NULL, +0.84pp, p=0.25 | +3 turns at N=250 is the same effect size R23 showed, and R23's null was wrong |

Baseline is the existing **A0″** arm (full N, already measured, precondition
already passed against R19's published corpus figures). No new baseline is run;
A0″ is reused, and the arms differ from it only in the named env lever.

**A3′ is the one I expect to flip.** +0.84pp/+3 turns at N=250 is
proportionally similar to R23's +1.69pp, and R23 flipped to a decisive PASS at
full N. If A3′ passes, the additive declarative boost is a real, shipped-but-off
capability we wrongly dismissed.

## Primary metric and decision rule — fixed before running

**Primary:** evidence-turn micro-recall, each arm vs A0″.
**Statistic:** Wilcoxon signed-rank on per-question evidence-turn counts,
two-sided, α = 0.05 (`scripts/score_r24.py`), nonzero-pair count always
reported.

Per arm:
- **PASS**: p < 0.05 **and** ≥ +2.0pp.
- **REFUTED**: p < 0.05 **and** ≤ −2.0pp.
- **NULL**: otherwise.
- **STILL UNDERPOWERED**: fewer than 15 nonzero pairs.

**These are the same gates R22 used**, so the comparison is like-for-like and
the only variable is N.

## What each outcome means, decided now

- **A1′ still REFUTED** → the RRF refutation stands and is strengthened. This is
  the expected case and it is *not* interesting.
- **A1′ or A2′ flips to PASS or NULL** → `MEASURED_RECORD.md` must be corrected
  and the failure analysis's §4/§5 conclusions revisited. **The published RRF
  refutation would be wrong**, and it would be corrected with the same
  prominence it was published with.
- **A3′ flips to PASS** → declarative-on-topk is a real gain we dismissed, and
  the "six lexical levers, six nulls" narrative — which motivated the entire
  composition programme — is partly an artefact of N.

## Registered non-goals

- **No paid runs, no embeddings, no model.**
- **No cascade measurement**, therefore no cascade change.
- No new levers, no parameter tuning, no arms beyond the three above.
- R22's published numbers are **not** retroactively edited. If a verdict
  changes, the original stands with a correction beside it — the same treatment
  R19 gave the BM25 baseline.

## Honest note

The cheap outcome is that everything survives and this costs a few hours of
machine time for a footnote. That is still worth it: **an unrepaired record
where one verdict is known wrong and the others are untested is worse than
either a repaired record or an honestly uncertain one.**

**Register row:** R26. **Refs:** `rrf-composition-result-2026-08-09.md` (R22),
`speaker-field-result-2026-08-09.md` (R24, which exposed the problem),
`speaker-attribution-result-2026-08-09.md` (R23, the null that was wrong).
