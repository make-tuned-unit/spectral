# R31 — PREREG: the conversion question, on an instrument that can actually see

**Registered 2026-08-11, before any arm ran.** $0, fully on-device, ollama
`qwen25-16k`, actor + judge, temp 0, no cloud calls.

## Why there is a second run at all

R30 returned **NULL on its primary** (+1.79pp, p = 0.3833). But its own writeup
identified the binding problem as **the instrument, not the lever**: baseline
accuracy was **11.79%**, barely above the 10% line registered as UNINFORMATIVE,
and the prereg's detectable effect was ~8pp against an observed 1.79pp. **R30
could not have detected the effect it found.**

A null from an instrument that cannot see is not evidence of absence. This run
replaces the instrument and asks the same question.

## The one honest risk here, and the commitment that contains it

**Choosing a new slice after a null is how p-hacking starts.** Two things bound
it, both fixed now:

1. **The slice is chosen for a stated instrument reason, verified before
   running:** on `single-session-user` the baseline retrieval is 72.2% and the
   published cloud-actor accuracy is 70.15%, so a weak reader lands far off the
   floor. **And adjacency's retrieval gain there is verified equivalent to
   multi-session's** — +19.44pp vs +19.77pp, measured on the archived R28 arms
   *before* this prereg was written. The slice was not chosen for a bigger
   lever; it was chosen for a readable dial.
2. **This is the last slice.** If R31 is also null, the conclusion is recorded
   as *retrieval improvements do not demonstrably convert to answers with a
   local reader*, and **no third slice will be tried.** Written down now so the
   stopping rule cannot be revised after seeing the result.

## Design

| arm | config | retrieval on this slice (measured, R28/R29) |
|---|---|---|
| **B0** | cascade defaults | 72.2% evidence recall, 1,438 tok |
| **B_ADJ** | `SPECTRAL_ADJACENCY=1` | 91.6% evidence recall, 3,264 tok (2.27×) |

- **Slice:** `single-session-user`, **n = 300**, a random sample of the 841
  drawn with **seed 20260811**. The drawn IDs are committed at
  `docs/internal/r31-sample-ids.txt` and passed via `--question-id @file`, so
  the sample is auditable and reproducible rather than asserted.
- Sampled rather than complete because 841 questions is ~13.8 h *per arm*
  on-device; 300 is ~5 h.
- Everything else identical to R30: `--retrieval-path cascade`,
  `--no-expand-queries`, `SPECTRAL_ACTOR_MAX_TOKENS=384` on **both** arms,
  temp 0, same actor, same judge.

## Metrics — all three, fixed in advance this time

R30's amendment is now part of the registered design from the start:

1. **Primary: deterministic normalized containment** (exact rule and code
   unchanged, `scripts/score_containment.py`), exact McNemar.
2. **Secondary: local LLM judge**, exact McNemar.
3. **Secondary: item-level recall**, Wilcoxon signed-rank.

**Multiplicity is registered, not discovered afterwards:** three metrics are
tested, so a nominal p < 0.05 on a secondary will be reported **with its
Bonferroni-corrected value (×3)**, exactly as R30 reported its 0.0280 → ~0.084.

## Verdict rules — fixed now

| condition | verdict |
|---|---|
| B_ADJ > B0, p < 0.05 on the **primary** | **PASS** — retrieval converts to answers on this slice |
| B_ADJ < B0, p < 0.05 on the primary | **REGRESSION** — dilution is real; do not ship |
| p ≥ 0.05 on the primary | **NULL** — and, per the commitment above, the conversion question is closed at $0 |
| B0 accuracy < 20% or > 80% | **UNINFORMATIVE** — floor or ceiling, not a null |

Note the floor bar is **raised to 20%** and a ceiling bar added. R30 slipped
through at 11.79% by clearing a 10% line that was set too low.

## Power

At n = 300 with a base rate expected in the 30–45% band, the discordant rate
should be materially higher than R30's, giving roughly **6–7pp** detectable at
~80% power — against R30's ~8pp on a floored base rate. Still **not** sensitive
to a 2–3pp true effect, and the verdict will say so.

## Prediction, on the record

**I expect a small positive, +3 to +7pp, and I am genuinely unsure whether it
clears p < 0.05.** Reasoning: R30's three metrics all pointed positive and its
graded metric reached nominal significance, which is weak but real evidence of a
small true effect; this slice has ~4× the headroom above the floor to express
it. Retrieval on this slice goes 72.2% → 91.6%, so if evidence presence drives
answers at all, it should show here.

**If this is null too, that is the answer**, and the retrieval programme's
central assumption should be recorded as unsupported at $0 rather than
relitigated on a fourth slice.

**Refs:** `adjacency-accuracy-result-2026-08-11.md` (R30),
`cascade-token-match-result-2026-08-11.md` (R29).
