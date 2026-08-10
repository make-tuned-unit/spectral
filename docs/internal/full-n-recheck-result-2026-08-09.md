# R26 — the N=250 verdicts re-tested at full N (2026-08-09)

**$0. Retrieval-only oracle, LoCoMo, full N = 1,438, `topk_fts`, k=40, R19 turn
labels. No model calls, model-free.** Preregistered at `52c50df`/prereg doc
before any arm ran. Gates identical to R22's, so **N is the only variable**.

Baseline is R24's existing A0″ arm (full N; precondition already passed against
R19's *published* corpus figures).

## Verdicts

| arm | lever | N=250 | **N=1,438** | nonzero pairs | outcome |
|---|---|---|---|---:|---|
| **A1′** | RRF | REFUTED −5.90pp | **REFUTED −6.96pp**, p<0.0001 | 211 [+40/−171] | **survives, stronger** |
| **A2′** | RRF + declarative | NULL −3.65pp, p=0.0525 | **REFUTED −2.71pp**, p=0.0015 | 224 [+84/−140] | **NULL → REFUTED** |
| **A3′** | additive declarative | NULL +0.84pp, p=0.25 | **NULL +1.36pp**, p=0.0001 | 39 [+33/−6] | **NULL → real but sub-threshold** |

## The RRF refutation holds, and is strengthened

**A1′ is the headline.** Our most consequential negative claim — the refutation
of our own top-priority hypothesis — survives 5.75× the data with a **larger**
effect: −5.90pp at N=250 becomes **−6.96pp (−149 evidence turns)** at full N,
on 211 nonzero pairs, p<0.0001. Multi-session collapses from 40.91% to
**32.69%**.

**A2′ changed verdict, and in the direction that strengthens R22, not weakens
it.** R22's *primary* arm sat at p=0.0525 — just above α, the same profile that
made R23's null wrong. At full N it resolves to a **significant decrease**. So
R22's primary comparison was not "no effect"; it was a real harm the sample was
too small to confirm.

**R22's conclusion is unchanged and better supported than when published.**

## A3′ — the correction the record needs

My prereg predicted A3′ would flip to PASS. **It did not, and the prediction was
wrong.**

But it also did not stay a null in the sense the record implies. At full N the
additive declarative boost is **statistically unambiguous** (p=0.0001, 33
questions improved against 6 worsened) and **practically marginal**
(+1.36pp, +29 evidence turns — below the prespecified +2.0pp bar).

That distinction matters and the +2.0pp gate is what preserves it: without an
effect-size clause, this would be reported as a win on p<0.001. **It is a real
effect that is too small to act on**, not an absence.

So the "six lexical levers, six nulls" framing needs one amendment: declarative
is a **small real gain**, not a null. That does not rescue the lexical family —
+1.36pp against a +23.34pp opportunity is still a rounding error, and it is the
family's *best* member.

## What this repairs, and what it costs

**Repairs:** the RRF refutation and R22's conclusions are now measured at full
N. The published record's most load-bearing negative claim is verified, not
merely asserted.

**Costs:** two of three verdicts were imprecise. A2′ was published NULL and is
REFUTED; A3′ was published NULL and is a real sub-threshold effect. Neither
error changed a decision — both point the same way R22 already concluded — but
the record said "no effect" where it should have said "harmful" and "small."

**Still outstanding:** G4 proximity, the k-admission rejection, and the earlier
lexical levers (porter, widening, ACT-R, spreading) remain measured at N=250 or
on the pre-R19 diluted metric. They are **not** re-tested here and should not be
treated as settled.

## Honest limits

- One corpus, `topk_fts`, retrieval only. No accuracy claim; no cascade
  measurement, therefore no cascade change.
- A3′ has 39 nonzero pairs — above the 15 floor, so it is powered, but it is the
  smallest sample of the three and its +1.36pp should not be sharpened further.
- The arms re-ran ingest rather than sharing brains with A0″ (brains are
  streamed away to fit the disk). Ingest is deterministic — A0″ itself
  reproduced R19's published figures — so this is not a confound, but it is not
  a byte-identical control either.

**Refs:** `full-n-recheck-prereg-2026-08-09.md`,
`rrf-composition-result-2026-08-09.md` (R22, verified here),
`speaker-field-result-2026-08-09.md` (R24, which motivated this).
