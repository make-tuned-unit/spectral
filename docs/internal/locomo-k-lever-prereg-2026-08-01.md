# Prereg — cascade `k` as a turn-recall lever on LoCoMo — 2026-08-01

> **METRIC CAVEAT (R15, 2026-08-07):** "key-recall" in this document is
> evidence-**session** turn coverage — every turn of every `answer_` session, a
> ~12x-diluted denominator — not evidence-turn recall. See
> `turn-level-evidence-recall-2026-08-07.md`. This note does not assert what the
> correct metric would have shown here; the numbers below are left exactly as
> measured (Rule 5).

**Written BEFORE the paid A/B.** Claims, expectations and decision rules stated
in advance so the result cannot be retrofitted.

## Origin, including a correction

The first LoCoMo held-out end-to-end run scored **64/118 = 54.2%**. My initial
autopsy concluded the failures were **synthesis-bound** — "42/56 had evidence
present", "81% of abstentions had evidence at rank 1".

**That conclusion was wrong.** It used the `answer_`-prefix proxy, which marks
turns belonging to the answer *session*. LoCoMo sessions run 15–30 turns, so
"3 answer keys retrieved" routinely means three irrelevant turns from the right
session. `BENCHMARKING.md` §4 warns about exactly this; I explained the warning
away.

Measuring whether the ground-truth answer text is actually **in the actor's
context** inverts it:

| | failures | passes |
|---|---|---|
| answer absent (<25% of GT terms in context) | 24 | 19 |
| partial (25–75%) | 22 | 12 |
| answer present (≥75%) | **9** | **32** |
| median GT-term coverage | **0.40** | **0.80** |

And **0 of 11 abstentions** had the answer present — the actor was abstaining
correctly. Retrieval *is* the discriminator, not synthesis.

## Mechanism

Spectral retrieves the right **session** but not the right **turn** inside it.
Session-recall 92.9% looked healthy; key-recall **13.8%** was the real signal.

The cap is `k` (30 for `Factual` shapes). `max_per_episode: 8` is inert because
`apply_episode_diversity` defaults false. With 15–30-turn sessions and k=30
across *all* sessions, the answer turn frequently does not make the cut.

The prior **"K=60→80 REJECTED"** result does not close this: it was measured on
**session**-recall, already saturated ~98% on LongMemEval. Turn-level recall was
never the metric there.

## $0 sweep (already run, LoCoMo dev set n=120)

| k | session-recall | key-recall | zero-retrieval | tokens (mean) |
|---|---|---|---|---|
| 30 (current) | 92.9% | 13.8% | 4 | 1603 |
| 60 | 97.5% | 17.9% | 0 | 2380 |
| 100 | 99.2% | 23.0% | 0 | 3448 |
| 150 | 99.6% | **28.9%** | 0 | 4728 |
| 250 | 99.6% | 39.1% | 0 | 7020 |

`k` does **not** saturate on LoCoMo, unlike LongMemEval.

## Claim under test

**K1** — raising cascade `k` from 30 to 150 converts turn-recall into end-to-end
accuracy on LoCoMo.

Candidate arm: `SPECTRAL_CASCADE_K=150`. Control: the already-measured 54.2%
baseline, same dataset, same actor/judge, same code.

## Expectation, stated in advance

The standing objection is **ACR**: +18–40pp answer-key recall produced **−2 net
accuracy**, because extra evidence distracts. Also the assistant-cap result:
context content changes accuracy by 15pp.

So this is **not** expected to convert automatically. My prior is roughly
even — the mechanism is better-targeted than ACR (it adds turns from the
*already-correct* session rather than new documents, and 24 failures had the
answer literally absent), but token volume triples and dilution is real.

## Decision rules

1. **PASS** — accuracy improves ≥5pp (54.2% → ≥59.2%) AND no category regresses
   by more than 5pp. Then, and only then, validate on a fresh **disjoint**
   sample before any claim.
2. **NULL** — accuracy within ±2pp. Record as another instance of
   "retrieval lift does not convert", alongside ACR. Do not ship. Do not retune.
3. **FAIL** — accuracy drops >2pp, or any category regresses >5pp. Record and
   close the `k` lever on LoCoMo.
4. No intermediate `k` values get tried after seeing this result. Sweeping `k`
   until one converts is exactly the failure this prereg exists to prevent.

## Held-out discipline

The 120-question set (seed 42) is now a **development set** — its failures have
been inspected, so it is burned. The published **54.2% held-out number stands as
measured** and is not invalidated, but this same sample cannot serve as held-out
evidence for any tuned configuration.

Only 120 of 1,438 answerable LoCoMo questions are used, so a genuinely disjoint
validation sample is available (converter supports `--seed`; ids from the dev set
must be excluded explicitly, since a different seed alone does not guarantee
disjointness).

## Not claimed

Nothing here claims Spectral improved. `k` is a configuration knob that was
always available. A positive result would say the shipped default is wrong for
long-session corpora — which is a finding about defaults, not capability.

---

# OUTCOME — recorded 2026-08-01, after the runs

## Verdict: **NULL. Not shipped.**

| | dev sample (burned) | disjoint validation (clean) |
|---|---|---|
| k=30 | 64/120 = **53.3%** | 77/120 = **64.2%** |
| k=150 | 79/120 = **65.8%** | 83/120 = **69.2%** |
| paired delta | **+12.7pp** | **+5.0pp** |
| discordant (recovered / regressed) | 16 / **1** | 14 / **8** |
| McNemar exact 2-sided | **p = 0.0003** | **p = 0.2863 — not significant** |

The dev-set effect did not replicate. Regressions rose 8×. This is the same
shape as the RERANK spreading lever (+10pp at n=30, refuted at n=78, p=0.81),
and the risk was named in this prereg before the runs.

**A hole in this prereg, stated plainly:** decision rule 1 set an *effect-size*
threshold for validation (≥5pp) but no *significance* requirement, and the
validation delta was exactly +5.0pp. Applying the effect-size rule literally
would score this a PASS. It is being called a NULL because 14-vs-8 discordant
pairs at p=0.29 is noise, and because this project refuted RERANK on precisely
that basis. Rewriting the bar after seeing the result would be the failure this
document exists to prevent — so the bar is being applied *more* strictly than
written, never less. Future preregs must state a significance requirement at the
validation stage.

## Second finding — a single n=120 LoCoMo run carries ~±10pp

Same config (k=30), same benchmark, two **disjoint** samples: **53.3% vs 64.2%**
(difference +10.8pp, SE 6.3pp, z=1.72, p≈0.086).

Consequence: the "**54.2% first held-out accuracy number**" reported earlier from
a single sample is **over-precise**. The honest statement is *~53–64% depending
on sample draw*. Any future single-sample LoCoMo headline carries the same
caveat. Two samples minimum before quoting a figure.

## What survives

Retrieval genuinely improves with `k`, and that part is solid and $0-measured:
zero-retrieval 4→0, key-recall 13.8%→28.9%, session-recall 92.9%→99.6% (k=150).

**It does not reliably convert to accuracy.** This is now the *third* independent
non-conversion, alongside ACR (+18–40pp answer-key recall → −2 net accuracy) and
the K=60→80 rejection. The hypothesis that same-session turns would distract less
than new documents is **not supported**.

Informational only (pooled across a burned dev set, therefore not a claim):
k=30 141/240 = 58.8%, k=150 162/240 = 67.5%, +8.8pp.

## Cost

$6.03 total for four end-to-end runs. Settling +8.8pp at 80% power would need
roughly n≈500 per arm (~$20). Not recommended: three independent nulls on
retrieval→accuracy conversion is a strong prior that the money buys a fourth.

## Corrected diagnosis, for the record

The first autopsy of the 54.2% run concluded the failures were **synthesis-bound**
("42/56 had evidence present", "81% of abstentions had evidence at rank 1") and
named abstention calibration as the top lever. **That was wrong** — it trusted the
`answer_` prefix, which is a *session*-level proxy on a corpus with 15–30-turn
sessions. Measuring GT-term presence in the actual actor context inverted it:
median coverage 0.40 on failures vs 0.80 on passes, and 0 of 11 abstentions had
the answer present. The actor was abstaining correctly.

**Method rule adopted:** on any corpus with long sessions, `answer_`-prefix
counts are not evidence that the answer reached the actor. Measure presence in
the assembled context.
