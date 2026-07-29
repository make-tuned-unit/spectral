# Hardened-judge re-grade of the frozen TIER1 head-to-head arms (2026-07-28)

Judge-only replay (`replay-actor --judge-only`, ~$0.80, zero actor calls) of the
two frozen head-to-head artifacts under the calibrated judge (PR #219: abstention
rule both directions, preference-compliance rubric, parse-failure exclusion).
Question: do the historical verdicts survive the rubric change?

## Result

| arm | frozen judge | hardened judge | net |
|---|---|---|---|
| porter-only (`tier1-h2h-porter.json`) | 46/59 | **48/59** | +2 |
| expansion-only (`tier1-h2h-exp.json`) | 55/70 | **57/70** | +2 |

**Every flip traces to a specific rubric change, by name:**

wrong→correct (+3 porter / +3 expansion):
- `bc8a6e93_abs` (both arms): "I don't know" vs a *not-mentioned* gold — the
  `_abs` direction of the new abstention rule. Previously scored wrong; abstention
  is the correct answer here.
- `06878be2`, `195a1a1b` (porter), `54026fce` (expansion): SSP questions whose
  answers complied with the stated preferences but failed the old fact-recall
  default rubric — recovered by the preference-compliance rubric.
- `09ba9854` (expansion): superset answer containing the exact gold ("$50")
  now credited.

correct→wrong (−1 both arms):
- `gpt4_fa19884d`: the actor quoted the gold description verbatim from the
  session, then concluded "no specific artist name is given". The old judge
  credited the quote; the calibrated judge enforces that a hedged abstention is
  not an answer. Defensible tightening, consistent in both arms.

## The load-bearing check: porter-vs-expansion verdict

On the 57-question clean common set under the hardened judge:
**porter 46 vs expansion 44** — porter still ahead (was 46/60 vs 43/60 frozen).
**"Porter replaces LLM expansion at $0" survives judge recalibration.** The
margin narrows from +3 to +2; the direction and the shipped decision hold.

## Notes

- The judge moved *up*, not down: the old rubric's net bias on these arms was
  −2 per arm (over-harsh on preference/abstention shapes, over-lenient on one
  hedged answer). Historical Tier-1 numbers are ~2–3pp understated per arm under
  the new rubric — reports carry `judge_rubric_fingerprint` from #219 on, so
  cross-rubric comparisons are mechanically blocked rather than silently made.
- **The n=500 headline artifact (#172 run, 2026-06-15) is NOT on this machine** —
  re-grading the published 81.5% requires locating that report file. Until then
  the honest statement is: the 81.5% was graded under the pre-calibration rubric;
  directionally the calibrated rubric grades *slightly higher* on the arms tested.
- Frozen: `~/spectral-local-bench/wa-ab/rejudge-h2h-{porter,exp}.json`.
