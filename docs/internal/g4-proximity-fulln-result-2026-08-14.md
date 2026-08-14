# R34 — G4 proximity at full N · NULL, the refutation stands (2026-08-14)

**$0, retrieval-only oracle, LoCoMo full N = 1438, `topk_fts`, R19 labels,
binary `ee00624`. Preregistered at `9a165cf`-adjacent (see
`g4-proximity-fulln-prereg-2026-08-14.md`, committed before the arm ran).**

## Result

| | evidence micro | zero-evidence |
|---|---:|---:|
| A0 (baseline, reused) | 59.86% (1281/2140) | 357 |
| P (`SPECTRAL_TOPK_PROXIMITY=0.40`, the sweep's best arm) | **60.19% (1288/2140)** | 352 |

**Δ +0.33pp (+7 turns), 17 nonzero pairs [+12/−5], Wilcoxon p = 0.1435 —
NULL on both gate clauses** (needed p < 0.05 AND ≥ +2.0pp).

## Reading

- **The prediction held in full**: "refutation stands, |Δ| < 1.0pp, not
  significant" — measured +0.33pp, p = 0.14. The prediction register is now
  3 hits / 1 half / 2 misses across the series.
- The **best-case** arm was tested deliberately: the whole N=250 grid fails
  a fortiori. G4's mechanism story survives — proximity needs ≥2 shared
  query terms to say anything, and 61.3% of missed evidence shares zero.
- This was the first item on R26's "not repaired" list. It is now settled at
  5.75× the data: **G4 proximity is REFUTED at full N.** Porter, widening,
  ACT-R, and spreading remain measured only at N=250 or on the diluted
  metric — unsettled, but all are static rerank levers of the family this
  series has now closed from four directions (R22, R26, R32, R33, R34).

## Does NOT follow

No cascade claim (unmeasured). No default change (proximity was already
default-off; it stays off).

**Refs:** `g4-proximity-fulln-prereg-2026-08-14.md`,
`g4-proximity-result-2026-08-08.md`, `full-n-recheck-result-2026-08-09.md`.
