# Preregistration — retrieval policy V2Fixed (classifier defects) — 2026-08-02

**Written before the measurement.** Binding.

## The two defects

Both found by the classifier pinning corpus written during the regex-cache
work, both pinned by tests so they could not be closed accidentally, neither
fixed at the time because they change routing on published numbers.

**1. `FactualCurrentState` never matched bare `current`.** The variant's own doc
comment reads *"What is my current X" — most-recent-wins factual*, but the
recency sub-gate pattern lists `currently`, not `current`. So the exact phrasing
in the documentation routed to plain `Factual` and lost recency priority. Same
omission in the Counting and `where` sub-gates.

**2. `what should i` in the `GeneralPreference` gate was dead code.** It is
checked *after* the Factual branch `^(?:what|where|who|which)\b`, so any
question beginning with "what" — every natural phrasing of it — returned
`Factual` first. The alternative was unreachable.

## Why they are worth measuring

`single-session-preference` is the **weakest measured category at 56.0%**
(`docs/RESULTS.md`, n=25 clean) — 19pp below the next worst. Defect 2 sits
directly on it. Defect 1 sits on `knowledge-update` (87.2%, n=78), whose whole
premise is "the user's situation changed, use the updated information" — exactly
what a recency gate is for.

## Implementation

`RetrievalPolicyVersion::V2Fixed`, selected via
`QuestionShape::classify_with`. **`V1` is untouched and remains the default**,
so every published number keeps citing the routing that produced it. The
harness selects a version with `SPECTRAL_POLICY=v2` through a single helper
(`retrieval::classify_question`) so an ablation cannot mix versions across call
sites.

A test (`v2_changes_nothing_else_in_the_pinned_corpus`) asserts V2 routes every
other question in the pinned corpus identically to V1 — this must be a repair
of two defects, not a reclassification, or the comparison is uninterpretable.

## Measurement — and its honest limitation

**This is measured on LongMemEval-S, which is IN-SAMPLE.** The retrieval was
developed against it. The held-out LoCoMo set has no preference category
(its categories are multi-session, single-session-user, temporal-reasoning), so
it cannot test defect 2 at all.

Consequence, stated up front: **a positive result here is in-sample evidence
and will be labelled as such.** It cannot support a generalization claim. The
most it can establish is that the repair does what it says on the dataset the
routing was designed for, and does not break anything else.

Arms, $0 oracle, zero LLM calls:

| arm | env | categories |
|---|---|---|
| A | none (V1) | `single-session-preference`, `knowledge-update`, `temporal-reasoning` |
| B | `SPECTRAL_POLICY=v2` | same |

`temporal-reasoning` (n=133) is the control: neither defect touches it, so it
must not move.

## Decision rules (binding)

1. **Primary.** `single-session-preference` **or** `knowledge-update`
   session-recall must improve by **≥ +2.0pp**. (Higher bar than the LoCoMo
   runs because this is in-sample and n is smaller.)
2. **Control.** `temporal-reasoning` session-recall must not change by more
   than **±0.5pp**. If the control moves, V2 is doing something beyond the two
   defects and the result is void.
3. **No-harm.** No measured category may regress by more than **1.0pp**;
   zero-recall must not increase.
4. **Cost.** Mean context tokens ≤ **+5%** per category.
5. **Default stays V1 regardless of outcome.** Clearing these gates makes
   V2Fixed a candidate for a paid actor A/B and, separately, for a held-out
   confirmation on a dataset with a preference category. It does not flip the
   default, and it is not an accuracy claim — the retrieval-to-accuracy
   conversion failure is the most repeated finding in this repo.
6. **One shot.** No pattern tuning after seeing the result.

## A falsifiable prediction, derived from the profile table before running

Reading `cascade_profile()` rather than assuming:

| shape | k | max_per_episode |
|---|---:|---:|
| `Factual` **and** `FactualCurrentState` | 30 | 8 |
| `Counting` **and** `CountingCurrentState` | 60 | 3 |
| `GeneralPreference` (inherits default) | 40 | 5 |

**The `*CurrentState` sub-shapes carry no configuration difference whatsoever.**
They share a profile with their base shape and route identically (Cascade). They
were introduced to give recency priority, and no profile ever applied any.

Two consequences, both predicted here before the run:

1. **Defect 1's repair is inert by construction.** Widening the recency gate
   moves questions from `Factual` to `FactualCurrentState`, which changes the
   recorded shape label and nothing else. `knowledge-update` must **not** move.
   If it does, something other than the classifier changed and the run is void.
2. **Defect 2's repair is not inert.** Rerouting `Factual` →
   `GeneralPreference` changes k 30→40 and `max_per_episode` 8→5 — more
   results, more session diversity. `single-session-preference` can move.

So this experiment tests defect 2 and simultaneously **confirms or refutes** the
claim that the current-state sub-shapes are dead weight.

If prediction 1 holds, the finding is not "the classifier is fine" — it is that
**the per-shape profile table, not the classifier, is where current-state
handling is missing.** That would be the actionable result, and it would point
at a profile change (recency-priority for CurrentState shapes) as the next
candidate rather than more classifier work.

## Prior

Low-to-moderate for defect 2; **effectively zero for defect 1**, per the
prediction above. Note also that defect 2's effect, if any, arrives through a
*profile* change (k and diversity), not through better classification per se —
and admission widening is the family this repo has already rejected twice
(K=60→80 REJECTED; cascade `fetch_mult` an accuracy no-op).
