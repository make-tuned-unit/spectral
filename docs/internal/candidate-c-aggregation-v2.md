# Candidate C Aggregation v2 — Revised Prompt + Re-Validation

**Date:** 2026-05-14
**Status:** Re-validation complete.
**Source:** `docs/internal/candidate-c-prevalidation.md` (v1 findings).
**Cost:** 8 API calls, ~$0.08.

---

## Executive summary

The revised aggregation prompt partially fixed the temporal
filtering failure (case #8: betta tank now listed) but did not
fix the dedup failure (case #9: still over-counted). More
importantly, re-running the extraction step revealed
**non-determinism in extraction itself** — session
`answer_e7b0637e_2` (Emily+Sarah's wedding) was extracted in v1
but returned "nothing relevant" in v2, same prompt, same model.

**Neither case flips to correct with the revised prompt.**

**Verdict: STOP.** The extraction mechanism is validated (v1
confirmed it), but the full pipeline is too fragile for reliable
bench lift. Extraction non-determinism and aggregation complexity
compound to make net lift uncertain. Implementing
`--actor-mode isolated` at this point would add harness complexity
for a result that may not be reproducibly better than the
single-call actor.

---

## Section 1 — Revised aggregation prompt

```
You are answering a question based on evidence extracted from
multiple conversation sessions.

Question: {question}

Evidence from each session:
{per_session_extractions}

Instructions:
1. List every item mentioned across all extractions, with its
   source session.
2. Deduplicate: for each pair of items, state whether they refer
   to the same thing or different things, and why. Items described
   differently across sessions (e.g. "cousin Emily" in one session
   and "friend Emily" in another) are likely the same person
   unless there is specific evidence they are different.
3. Do NOT filter items by temporal status (current vs past, old
   vs new). Your job is to list and count items mentioned, not to
   judge whether they still exist. Include every item regardless
   of tense.
4. State the final deduplicated list and count.
```

**Changes from v1:**
- Added instruction 2: explicit show-your-work dedup with guidance
  that different relationship descriptors for the same name are
  likely the same person.
- Added instruction 3: no temporal filtering — the aggregation
  step should not judge whether items are "current."
- Added instruction 4: structured output (deduplicated list then
  count).

---

## Section 2 — Case #8: Tanks

**Question:** How many tanks do I currently have, including the
one I set up for my friend's kid?
**Ground truth:** 3 tanks.

### Extraction results (v2 run)

Extraction prompt unchanged from v1.

| Session | Model | Result |
|---|---|---|
| answer_c65042d7_1 | haiku (sonnet 529) | "Nothing relevant" — saw the 20-gallon Amazonia but said no mention of friend's kid tank. Same behavior as v1. |
| answer_c65042d7_2 | sonnet | Found 5-gallon betta tank, 20-gallon community tank, and planned quarantine tank. Same as v1. |
| answer_c65042d7_3 | sonnet | Found community tank and 1-gallon friend's kid tank. Same as v1. |

Extraction behavior is consistent with v1. The 5-gallon betta
tank surfaced again.

### Aggregation v2 result

The model listed 4 tanks: 5-gallon betta, 20-gallon community,
quarantine (planned), and 1-gallon friend's kid. It correctly
deduplicated the community tank across sessions 2 and 3.

**Temporal filtering partially fixed:** The 5-gallon betta tank
was included in the full list (instruction 3 worked). However,
the model then added a hedge: "if the question is specifically
about tanks the user currently has, the answer is 2 tanks" —
re-introducing the temporal filter despite the instruction
against it. The question itself says "currently have," which
overrides the prompt instruction in the model's reasoning.

**New problem:** The quarantine tank (planned, not set up) was
included in the count of 4. The instruction to "not filter by
temporal status" also prevented filtering out planned-but-
not-existing items.

**Final count: ambiguous (model says "4 mentioned" but "2
current").** GT is 3. Neither number is correct.

### Assessment

The v2 prompt moved in the right direction — the betta tank is no
longer discarded — but the "don't filter temporally" instruction
is too blunt. It prevents the correct temporal filter (old vs
current) AND prevents filtering things that don't exist yet
(quarantine tank). The question itself asks about "currently have,"
so the aggregation inevitably reasons about currency regardless
of the prompt instruction.

---

## Section 3 — Case #9: Weddings

**Question:** How many weddings have I attended in this year?
**Ground truth:** 3 weddings (Rachel+Mike, Emily+Sarah, Jen+Tom).

### Extraction results (v2 run) — REGRESSION

| Session | Model | v1 result | v2 result |
|---|---|---|---|
| answer_e7b0637e_1 | sonnet | Rachel, cousin Emily (2 weddings) | Rachel, cousin Emily (2 weddings) — same |
| answer_e7b0637e_2 | sonnet | Emily+Sarah, college roommate (2 items) | **"Nothing relevant"** — declined because "does not specify that either occurred in the current year" |
| answer_e7b0637e_3 | haiku (sonnet 529) | Jen+Tom (1 wedding) | Jen+Tom (1 wedding) — same |

**Critical finding: extraction non-determinism.** Session 2 used
sonnet in both v1 and v2 runs. Same extraction prompt. In v1, the
model extracted Emily+Sarah and the college roommate's wedding. In
v2, the model said "nothing relevant" because the question asks
about weddings "in this year" and the session doesn't specify the
year. Both responses are defensible — but they produce different
downstream results. This is LLM non-determinism, not a prompt
problem.

### Aggregation v2 result

With the degraded extraction (session 2 returning "nothing
relevant"), the aggregation received only 3 items: Rachel's
wedding, cousin Emily's wedding, and Jen+Tom's wedding. The
aggregation then added items from session 2's "nothing relevant"
text (the model still mentioned weddings even while saying they
weren't relevant), producing a list of 5 items.

The dedup instruction partially worked — the model explicitly
analyzed each pair — but concluded that "cousin Emily" and "friend
Emily" are "POTENTIALLY DIFFERENT" people because of different
relationship labels. The explicit instruction that "items described
differently across sessions are likely the same person" was
overridden by the model's own judgment.

**Final count: 5.** GT is 3. Worse than v1 (which produced 4).

### Assessment

Two compounding problems:
1. **Extraction non-determinism** — same prompt, same model,
   different extraction on the key session.
2. **Dedup instruction insufficient** — the model acknowledged
   the "likely same person" instruction but decided the evidence
   was ambiguous enough to treat them as different.

---

## Section 4 — What the v2 iteration shows

### The aggregation problem is harder than expected

The v1 pre-validation identified two specific failures (temporal
filtering, under-dedup) and the v2 prompt targeted them directly.
Results:

| Failure | Fix attempted | Outcome |
|---|---|---|
| Temporal over-filtering (#8) | "Do NOT filter by temporal status" | Partially worked — betta tank listed, but quarantine also included, and model still hedged to "2 current" |
| Under-deduplication (#9) | "Items described differently are likely the same person" | Did not work — model overrode instruction with its own judgment |

The aggregation step is not a simple "merge and count" operation.
It requires nuanced reasoning about identity, temporality, and
reference resolution — exactly the kind of reasoning that is
difficult to control via prompt instructions.

### Extraction non-determinism is a real concern

The Emily+Sarah extraction (session 2) succeeded in v1 but failed
in v2, same model. This means even if the aggregation were perfect,
the pipeline would produce different results across runs. For a
bench measurement, this non-determinism means:
- A single 20-question bench run might show +1-2 questions from
  isolation
- A second run on the same questions might show 0 or even -1
- The "lift" would be within LLM noise, not a reliable signal

### The net lift calculation has changed

The proposal estimated +1 to +2 questions (cases #8 and #9).
The pre-validation data now shows:

- **Case #8:** Extraction works reliably, but aggregation cannot
  consistently produce the correct count (3) from the extracted
  items — it oscillates between 2 (over-filtered) and 4
  (under-filtered).
- **Case #9:** Extraction works sometimes (v1) but not always
  (v2), and aggregation cannot reliably deduplicate across sessions
  even when extraction succeeds.

Expected bench lift: **+0 to +1 questions, not reproducible
across runs.** This is below the threshold where the result is
distinguishable from noise at N=20.

---

## Section 5 — Go/No-Go

### Decision: STOP

The extraction mechanism is validated — context isolation surfaces
items that cross-session processing misses. This finding stands
from the v1 pre-validation.

But the full pipeline (extraction + aggregation) has two
compounding sources of unreliability:
1. **Extraction non-determinism** — the same session may or may
   not yield the target items across runs.
2. **Aggregation fragility** — prompt-based dedup and temporal
   reasoning produce inconsistent results that the prompt
   instructions cannot reliably control.

Implementing `--actor-mode isolated` would add non-trivial
harness complexity (new actor wrapper, session splitting, prompt
templates, CLI flag) for a mechanism that produces unreliable
results on the 2 target cases. The expected bench lift (+0 to +1
questions) is within LLM noise at N=20 and would not constitute
a credible measurement.

### What this means

**The GENUINE_MISS floor for multi-session counting is
structural.** It is not addressable by prompt-level or
call-structure interventions in the current bench architecture.
The sub-problem 2 failures (cross-session topic filtering) can be
mechanistically isolated but cannot be reliably aggregated back
into correct answers.

The remaining paths for multi-session improvement are:
- **Judge refinement (item #20):** Addresses 3
  DEFINITION_DISAGREEMENT cases. No actor change needed.
- **Model capability:** A more capable model might extract
  embedded references without context isolation. Out of scope
  for Spectral engineering.
- **Accepting the floor:** Multi-session counting at 55% (11/20)
  with 4 GENUINE_MISS + 3 DEFINITION_DISAGREEMENT + 2
  RETRIEVAL_MISS + 1 TEMPORAL is the current structural reality.

### Actor-level investigation complete

This is the end of the Candidate C line of investigation. The
sequence was:
1. Actor-level interventions investigation — 5 candidates
   analyzed, only Candidate C structurally different.
2. Candidate C proposal — mechanism designed, pre-validation
   planned.
3. Pre-validation v1 — extraction validated, aggregation failures
   identified.
4. Aggregation v2 (this doc) — prompt refinement attempted,
   fragility confirmed.

No further actor-level investigation is recommended. The
highest-leverage next step is item #20 (judge refinement) which
targets a different 3 cases with no actor changes.

---

## Appendix: Model availability

Sonnet returned 529 (overloaded) on 3 of 8 calls. Those calls
fell back to `claude-haiku-4-5-20251001`. This affected calls 1
(tank extraction, session 1), 7 (wedding extraction, session 3),
and 8 (wedding aggregation). The haiku responses were consistent
with the sonnet responses on the same sessions in v1, so model
mixing is not the primary cause of the v2 differences. The
critical extraction regression (session 2, weddings) occurred on
sonnet, not haiku.
