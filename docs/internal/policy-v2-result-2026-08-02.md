# Policy V2Fixed — result — gates pass on paper, evidence is n=1

Prereg: `policy-v2-prereg-2026-08-02.md`. LongMemEval-S (**in-sample**), 241
questions across three categories, $0 oracle, zero LLM calls.

## Results

| category | n | V1 sess-rec | V2 sess-rec | Δ |
|---|---:|---:|---:|---:|
| knowledge-update | 78 | 99.4% (155/156) | 99.4% (155/156) | **0.0** |
| single-session-preference | 30 | 93.3% (28/30) | 96.7% (29/30) | +3.4pp |
| temporal-reasoning (control) | 133 | 96.0% (280/292) | 96.0% (280/292) | **0.0** |
| TOTAL | 241 | 96.7% | 97.2% | +0.5pp |

Zero-recall 3 → 2. Key-recall on preference 37.3% → 37.6%. Tokens +1.1% on
preference, unchanged elsewhere.

## Gates

| gate | rule | result |
|---|---|---|
| 1. Primary | preference or knowledge-update ≥ +2.0pp | PASS — preference +3.4pp |
| 2. Control | temporal-reasoning within ±0.5pp | **PASS — exactly 0.0** |
| 3. No-harm | no regression > 1.0pp, zero-recall must not rise | PASS — zero-recall fell 3 → 2 |
| 4. Cost | ≤ +5% tokens | PASS — +1.1% |

## …and why I am not calling this a pass

**The entire effect is one question.** Of 241 questions, 3 changed shape:

| change | n | retrieval effect |
|---|---:|---|
| `Factual` → `FactualCurrentState` (knowledge-update) | 2 | **none** |
| `Factual` → `GeneralPreference` (preference) | 1 | 0 → 1 answer sessions |

Question `06f04340` went from 0/1 to 1/1 answer sessions. That is the whole
result.

**The +2.0pp gate was miscalibrated for the sample size.** On n=30, a single
question is 3.3pp — so the gate was, in effect, "one question must improve".
That is my error in writing the prereg, and the correct response is to say so
rather than bank a pass the design could not have failed to produce from noise.
Recorded as **inconclusive, directionally correct**. Per prereg rule 5 the
default stays V1 regardless, so nothing ships on this.

## Both preregistered predictions confirmed

**Prediction 1 — defect 1's repair is inert by construction. CONFIRMED.** Two
knowledge-update questions reclassified `Factual` → `FactualCurrentState` and
retrieval did not change on either, because the two shapes share a cascade
profile (k=30, max_per_episode=8) and a route.

This is the durable finding of the run:

> **The `*CurrentState` sub-shapes are dead weight.** `FactualCurrentState` and
> `CountingCurrentState` were introduced to give recency priority, and no
> profile in `cascade_profile()` ever applied any. They change a label and
> nothing else.

So current-state handling is missing from the **per-shape profile table**, not
from the classifier. Widening the classifier gate — the "obvious" fix, and the
one the defect's own doc comment implies — could not have worked. A profile
that actually applies recency priority for CurrentState shapes is the real
candidate, and it has not been tried.

**Prediction 2 — defect 2's repair is not inert. CONFIRMED,** via the profile
change it causes (k 30→40, max_per_episode 8→5), on exactly one question.

## The finding that matters most here

`single-session-preference` is the weakest category at **56.0% end-to-end
accuracy** — and its **session-recall is already 93.3%**.

The answer session is retrieved for 28 of 30 questions, and the actor still
gets 44% of them wrong. **Retrieval is not the bottleneck on the weakest
category.** Total remaining retrieval headroom there is 6.7pp of session
recall; the gap to accuracy is ~37pp and is entirely actor-side.

This kills the theory that motivated the experiment — "fix preference routing
to fix the weakest category". It cannot be fixed from retrieval. It is a
synthesis problem, consistent with the record's standing conclusion and with
the two identity-keyed-prompt interventions that *did* convert (+8.0pp on
counting, actor-side).

## In-sample caveat

LongMemEval-S is the dataset the retrieval was developed against. Nothing here
is generalization evidence. The held-out LoCoMo set has no preference category,
so defect 2 cannot currently be tested out of sample at all.

## What is kept

`RetrievalPolicyVersion::V2Fixed` stays, default **V1**, selectable via
`QuestionShape::classify_with` and `SPECTRAL_POLICY=v2`. V1 remains verbatim so
every published number keeps citing the routing that produced it. Tests pin
that V2 repairs exactly the two defects and reclassifies nothing else.

## Next candidate, from this run rather than from a hunch

Give the `*CurrentState` shapes a profile that differs from their base shape —
recency-priority, i.e. a short `recency_half_life_days` — and measure whether
`knowledge-update` moves. That is the change the sub-shapes were introduced to
enable and never received. It needs its own prereg, and the prior should account
for knowledge-update already sitting at 99.4% session-recall, which leaves
almost no retrieval headroom either.
