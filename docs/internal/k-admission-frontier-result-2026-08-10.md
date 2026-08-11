# R27 — the k-admission frontier · the whole axis is dominated (2026-08-10)

**$0. Retrieval-only oracle, LoCoMo, full N = 1,438, `topk_fts`, R19 turn
labels. No model calls, model-free.** Preregistered at `56fb365` before any new
arm ran.

**Per the prereg this carries NO PASS verdict and NO adoption recommendation.**
It is a priced curve. "Recall rises with k" is arithmetic, not a finding.

## The frontier

| config | evidence micro | turns | zero-ev | tokens | marginal turns / 1k extra tokens |
|---|---:|---:|---:|---:|---:|
| k=40 (baseline) | 59.86% | 1281 | 357 | 1.00× | — |
| k=60 | 65.00% | 1391 | 295 | 1.50× | 110.8 |
| k=80 | 68.69% | 1470 | 261 | 2.00× | 80.3 |
| k=105 | 72.06% | 1542 | 228 | 2.62× | 59.3 |
| k=150 | 76.21% | 1631 | 187 | 3.70× | 41.6 |
| k=200 | 79.25% | 1696 | 153 | 4.84× | 28.8 |
| **ADJ1** (k=40, ±1) | **78.79%** | 1686 | **119** | **2.50×** | **137.0** ¹ |
| **ADJ2** (k=40, ±2) | **84.53%** | 1809 | **63** | 3.63× | 101.6 ¹ |

¹ ADJ rows are *average* turns/1k extra tokens against baseline; k rows are
*marginal* against the previous k.

## Three findings

**1. The k curve has no knee.** Marginal return decays smoothly from 110.8 to
28.8 turns per 1k tokens — a 3.8× collapse with no inflection anywhere. There is
no measurement that says "stop here", which is why the 2026-07-20 decision was
always going to be a **cost judgment** rather than a discovery. That was true
then and remains true.

**2. The prereg's falsifiable prediction held, decisively.** It predicted k=80
would *not* reproduce LongMemEval's +1.00pp shape on LoCoMo, because LoCoMo has
40.1pp of headroom against LongMemEval's 11.5pp:

| corpus | k=40 → k=80 | tokens | efficiency |
|---|---:|---:|---:|
| LongMemEval (where k was rejected) | +1.00pp (+9 turns) | 1.33× | 27 turns/unit |
| **LoCoMo** (where the verdict was applied) | **+8.83pp (+189 turns)** | 2.00× | **189 turns/unit** |

**8.8× the effect, 7× the efficiency, same lever.** The 2026-08-08 re-test was
sound; it simply does not generalise off the corpus it was taken on.

**3. The entire k axis is dominated.** ADJ1 reaches k=200's recall (78.79% vs
79.25%) at **half the tokens** (2.50× vs 4.84×) — and is far ahead on
zero-evidence questions even so (**119 vs 153**). At matched budgets adjacency
wins at both levels tested, and **the margin widens with budget**:

- ADJ1 vs k=105 (~2.5×): **+6.73pp**, p<0.0001
- ADJ2 vs k=150 (~3.65×): **+8.32pp**, p<0.0001

That is the opposite of what you would see if adjacency were merely an efficient
way of buying the same evidence. It is buying different evidence — confirmed
independently in R25: **272 turns (31.7% of all misses) are reachable ONLY by
adjacency**, with just 15.5% overlap with k=105.

## What this does to the k-admission decision

**It dissolves it rather than reversing it.** The 2026-07-20 rejection reached
the right operational conclusion — *do not buy recall by raising k* — and this
run supports that conclusion at every point on the curve. What it got wrong is
only the *reason*: the rejection was justified by a measurement on a
near-saturated corpus, and the real justification is that **the k axis is
dominated by a different axis entirely.**

No correction to the original verdict is required. The correction already issued
was mine: R26 wrongly listed the k rejection as measured at N=250 when it was
N=500 on LongMemEval. That stands corrected in `full-n-recheck-result-2026-08-09.md`.

## What this cannot decide, restated

**No k is adopted, and neither is adjacency, on this evidence.** The token cost
lands entirely on the **reader**, and there is no accuracy budget. At 2.5–4.8×
context it is genuinely possible that higher evidence recall produces *worse*
answers through dilution — the failure analysis already priced an 11.69pp
reader-side residual. **A retrieval frontier is a menu, not a decision.**

Also unmeasured: `recall_cascade`, the only path Permagent calls. Cascade `k` is
not controlled by `--max-results` (it comes from the question-type profile), so
this entire curve may not have a cascade analogue. R28 tests transfer.

## Honest limits

- One corpus, one path, retrieval only.
- The ADJ rows are average-efficiency and the k rows marginal-efficiency; they
  are not the same statistic and the table says so rather than blending them.
- k=200 is the largest point run. The curve is monotone to that point; nothing
  here rules out different behaviour beyond it, and the failure analysis's
  k=500 figure (89.7% macro at 8.7×) sits off this table's right edge.

**Refs:** `k-admission-frontier-prereg-2026-08-10.md`,
`k-admission-test-2026-07-20.md` (the original), `MEASURED_RECORD.md`
(2026-08-08 re-test), `turn-adjacency-result-2026-08-10.md` (R25),
`cascade-transfer-prereg-2026-08-10.md` (R28).
