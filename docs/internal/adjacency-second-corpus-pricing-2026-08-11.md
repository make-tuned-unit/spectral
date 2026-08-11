# Adjacency on a second corpus, priced for $0 — the +18.22pp does not travel at that size

**2026-08-11. $0, offline.** Archived `r12-baseline.jsonl` (LongMemEval, shipped
config, 500 questions) + the corpus's own `has_answer` turn labels. No brains,
no retrieval, no model calls, no run scheduled.
`scripts/price_adjacency_longmemeval.py`.

## Why this was worth doing before any run

The mechanism diagnostic found adjacency is **indifferent to lexical overlap** —
it does not attack the coreference inversion, it is merely orthogonal to the
lexical channel. Its value therefore rests on **dialogue geometry**: the answer
landing within ±1 of something the lexical channel already found. LoCoMo is
two-party and strictly alternating, which is that geometry's best case.

**R24 is the precedent that makes this urgent.** It PASSED on LoCoMo and
provably does not transfer to LongMemEval, because the structure it needs is
absent there. Discovering that after the run would have been expensive; here it
is free.

## Instrument check

The script derives evidence keys from the dataset labels independently of the
oracle, and reproduces the archived figure **exactly**: **793/896 = 88.50%**
micro evidence recall, the same number in `MEASURED_RECORD.md` for R15/R16. The
key arithmetic is therefore right, and so is the corpus join.

## Result

| | LongMemEval | LoCoMo (cascade) |
|---|---:|---:|
| labelled evidence turns | 896 | 2,140 |
| retrieved by baseline | **88.50%** | **58.60%** |
| missed | 103 | 886 |
| **adjacency ±1 ceiling** | **+5.80pp** | +18.22pp *(measured, not a ceiling)* |

Distance from the nearest retrieved turn, for the 103 misses:

| window | new | cumulative | share of misses | ceiling on micro recall |
|---|---:|---:|---:|---:|
| ±1 | +52 | 52 | **50.5%** | **+5.80pp** |
| ±2 | +10 | 62 | 60.2% | +6.92pp |
| ±3 | +3 | 65 | 63.1% | +7.25pp |
| ±4 | +3 | 68 | 66.0% | +7.59pp |
| ±5 | +10 | 78 | 75.7% | +8.71pp |
| ±6 | +3 | 81 | 78.6% | +9.04pp |
| unreachable ≤6 | — | 22 | 21.4% | — |

## What it says

**The geometry is present — this is not R24.** Half the misses (50.5%) sit
directly beside a retrieved turn, so the mechanism adjacency relies on exists on
LongMemEval. Expect it to work here *directionally*. That is a real and useful
difference from speaker attribution, which had nothing to grab at all.

**But the magnitude does not travel.** The absolute ceiling is **+5.80pp against
a measured +18.22pp on LoCoMo — under a third, and it is a ceiling, not a
forecast.** The reason is not that adjacency is weaker here; it is that
**LongMemEval's baseline is already at 88.50%** and there are only 103 turns
left to win. LoCoMo had 886.

**The cost side is worse here, too.** Adjacency ran 2.27× tokens on LoCoMo, off
a 1,500-token base. LongMemEval contexts average ~14,200 tokens, so the same
multiplier is an enormous absolute spend for at most 52 turns. On this corpus
the token-matched control would very likely win outright — which is exactly the
comparison R29 is running on LoCoMo.

## What this changes

- **The R28 headline should not be stated corpus-neutrally.** "+18.22pp" is a
  LoCoMo-cascade number on a corpus with 41pp of retrieval headroom. Where the
  headroom is small, so is the prize — by construction, not by mechanism.
- **It does not refute adjacency**, and it is not a run. It is a ceiling that
  says a LongMemEval adjacency experiment is **low-value at high token cost**,
  and should not be the next thing scheduled.
- **The pattern holds across both corpora**: the fraction of misses reachable by
  *any* ±N window plateaus early (78.6% here at ±6, 51.2% on LoCoMo), so the
  window axis is not where the remaining evidence lives on either corpus.

## Limits

- A ceiling, not a measurement. Real adjacency also displaces turns under a
  fixed budget and admits distractors; the yield is strictly below this.
- `r12-baseline` is the shipped mixed-routing config, not a cascade-only arm, so
  it is not directly comparable to the LoCoMo cascade rows line for line.
- Retrieval only, as everything in this programme is.

**Refs:** `adjacency-mechanism-diagnostic-2026-08-11.md`,
`cascade-transfer-result-2026-08-10.md` (R28),
`r24-longmemeval-nonreplication-2026-08-09.md`.
