# R27 — the k-admission frontier at full N on LoCoMo · PREREGISTRATION

**$0. Retrieval-only oracle, LoCoMo, full N = 1,438, `--retrieval-path
topk_fts`, R19 turn labels. No model calls, model-free.** Written and committed
before any new arm ran.

## What is actually being revisited, stated precisely

**The prior rejection was not wrong, and this prereg does not claim it was.**

`k-admission-test-2026-07-20.md` rejected K=60→80, and it was re-tested on
2026-08-08 on the **corrected** evidence-turn metric over **500 LongMemEval**
brains: k=40 → k=80 moved **793/896 → 802/896 (+9 turns, +1.00pp)** for **+33%
context tokens**. Verdict: a real but tiny admission gain at a large token cost.
That measurement stands.

**The problem is where it was taken.**

| corpus | baseline evidence recall | headroom |
|---|---:|---:|
| LongMemEval (where k was rejected) | 793/896 = **88.5%** | **11.5pp** |
| LoCoMo (where the verdict is applied) | 1281/2140 = **59.9%** | **40.1pp** |

k was rejected on a corpus that was already near-saturated, and the verdict has
since been applied to one with **3.5× the headroom**. On LoCoMo at full N,
R25's control arm measured **k=40 → k=105 at +12.20pp (+261 turns), 215/215
questions improved, zero worsened** — against LongMemEval's +1.00pp.

**A verdict measured where there was nothing to gain should not govern a corpus
where there is.** That is the whole justification for R27.

## The thing that would make this dishonest, named up front

**Evidence-turn recall rises with k almost by construction.** Emit more turns,
capture more evidence. A sweep showing "recall goes up with k" measures
arithmetic, not a lever, and reporting it as a win would be misleading.

So R27 is **not a hypothesis test and has no PASS gate.** It is a **frontier
mapping**, and its output is a priced curve, not a recommendation. The
quantities that carry information are:

1. **Marginal evidence turns per 1,000 additional context tokens**, per step.
2. **Where that marginal return collapses** (the knee).
3. Whether any k is **Pareto-dominated** — another config with ≥ recall at
   ≤ tokens.

## Arms

k = **40** and **105** already exist at full N (R24's A0″ and R25's KMATCH) and
are **not re-run**. New arms:

| arm | k | note |
|---|---:|---|
| k60 | 60 | the original rejection's lower bound |
| k80 | 80 | **the exact point rejected on LongMemEval** |
| k150 | 150 | past R25's control |
| k200 | 200 | the failure analysis's "recovers 83%" figure |

All arms: `topk_fts`, `--fresh-brains --no-keep-brains`, single variable
(`--max-results`), scored against A0″.

## Reported quantities — fixed before running

Per arm, versus A0″ (k=40): evidence-turn micro-recall, Δpp, evidence turns
gained, zero-evidence count, mean context tokens, and **marginal turns per 1k
tokens against the next-smaller k**. Wilcoxon p and nonzero-pair counts are
reported for completeness but **no arm is declared PASS or REFUTED** — the
prereg deliberately withholds that verdict, because "more context retrieves more
evidence" is not a finding.

Secondary: the multi-session slice, and whether the k=80 point reproduces
LongMemEval's +1.00pp shape on LoCoMo (it should not, if the headroom
explanation is right — that is a falsifiable prediction of this document).

## What this cannot decide, and must not be read as deciding

**No k is adopted on retrieval evidence alone.** The token cost lands entirely
on the **reader**, and we have **no accuracy budget** to measure whether more
context helps or hurts answers. The failure analysis priced the reader's
residual at 11.69pp and there is a real possibility that 2.62× context lowers
end-to-end accuracy by dilution.

**This produces a priced menu for a decision that is Jesse's, not a
recommendation to raise k.** Any adoption requires an end-to-end arm that is not
budgeted.

Registered non-goals: no paid runs, no embeddings, no cascade measurement
(`recall_cascade` k is not controlled by `--max-results` — see the 2026-08-08
side observation about `single-session-preference` routing to cascade and coming
back bit-identical), no tuning of `fetch_mult`, and **no retroactive edit** of
the 2026-07-20 or 2026-08-08 records — corrections sit beside them.

## Correction issued by this document

`full-n-recheck-result-2026-08-09.md` (R26) listed the k-admission rejection
among verdicts "still measured at N=250 or on the pre-R19 diluted metric".
**That is wrong.** It was measured at N=500 on LongMemEval on the corrected
metric. The R26 doc is corrected accordingly; the error was mine and it
overstated the disrepair of the record.

**Register row:** R27. **Refs:** `k-admission-test-2026-07-20.md` (original),
`MEASURED_RECORD.md` (the 2026-08-08 re-test),
`turn-adjacency-prereg-2026-08-09.md` (R25, whose control produced the
+12.20pp figure), `failure-analysis-2026-08-08.md`.
