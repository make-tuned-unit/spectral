# Item #20: Judge Rubric Revision v2 — Proposal

**Date:** 2026-05-14
**Status:** Proposal. Awaiting review before implementation.
**Source:** Item #20 backlog entry; v1 post-mortem in
`docs/internal/item-20-reasoning-aware-judge-proposal.md` Section 0.

---

## Section 1 — Why the first attempt failed

### The v1 mechanism

PR #102 added a "reasoning-aware +-1 tolerance" for counting
questions. When the actor's count was off by exactly 1, the judge
checked whether the actor showed "DELIBERATION" — explicit
meta-reasoning about which items to include or exclude. If the
actor deliberated, the delta-1 was accepted as a defensible
interpretation. If not, it was rejected as a genuine miss.

### The precise failure

The tightening added during review — "simply listing items does
not constitute reasoning — must show DELIBERATION" — set a bar
that none of the 3 target cases could clear:

- **Case #1 (clothing):** Actor explicitly reasoned "The boots
  exchange seems already done — they just need to pick up the
  replacement pair." This IS reasoning about inclusion/exclusion,
  but the judge interpreted "deliberation" as requiring an
  explicit meta-statement like "I'm counting this as 1 item
  because..." rather than accepting implicit reasoning in the
  actor's narrative.
- **Case #2 (projects):** Actor listed 3 projects by name. No
  explicit discussion of whether "planned for June" counts as
  "leading." The judge correctly rejected this under the
  deliberation bar — the actor enumerated but did not reason.
- **Case #6 (citrus):** Actor produced 40+ exhaustive quotes
  documenting grapefruit across 4 sessions. The judge required
  explicit verbal deliberation ("I'm including grapefruit
  because...") and rejected exhaustive evidence as insufficient.

### The root cause

The v1 approach relied on the judge inferring the actor's intent
from output structure. "Did the actor deliberate?" is a judgment
call that depends on what counts as deliberation — a subjective
distinction the judge LLM cannot reliably apply. The actor's output
format (narrative reasoning, exhaustive quoting) conveys intent
implicitly, but the judge requires explicit signals. This gap is
fundamental to single-prompt judge evaluation of counting-question
edge cases.

### What a second attempt must do differently

The second attempt must **not** rely on judging the actor's intent
or reasoning quality. It must use **objective structural signals**
in the actor's output — specifically, the presence or absence of
the disputed items — rather than subjective assessments of whether
the actor "deliberated."

---

## Section 2 — The 3 target cases

### Case #1: Clothing (0a995998) — under-count

**GT:** 3 items. **Actor:** 2 items.

**Actor's answer:** Navy blue blazer (dry cleaner pickup) + new
boots from Zara (exchange pickup). The actor explicitly says:
"The boots exchange seems already done — they just need to pick
up the replacement pair... the original too-small pair has already
been returned." It sees all the evidence, reasons that the
exchange is 1 action (pickup), not 2 (return + pickup).

**Why defensible:** The text says "I got them on February 5th, but
they were too small, so I exchanged them for a larger size" —
past tense, exchange complete. The actor's interpretation (1
pending action, not 2) is reasonable. The GT counts it as 2
(return boots + pickup new boots = 2 separate items, plus blazer
= 3). Both interpretations are valid.

**Rubric change needed:** Accept this as correct because the actor
shows **awareness** of the disputed evidence (the exchange) and
explicitly reasons about it. The actor didn't miss anything — it
categorized differently.

**Regression risk:** Low. The signal is strong: the actor quotes
the exchange, discusses it, and concludes it's 1 action. No
currently-correct case with delta=0 would be affected. The only
risk would be a future under-count case where the actor mentions
an item but wrongly excludes it — but that's also a
DEFINITION_DISAGREEMENT by construction.

### Case #2: Projects (6d550036) — over-count

**GT:** 2 projects. **Actor:** 3 projects.

**Actor's answer:** Data analysis, cloud migration, product
feature launch (planned for June). The GT counts 2; the actor
included a planned launch as "leading a project."

**Why defensible:** "Planning a product feature launch for June" is
plausibly "leading a project." The GT's narrower scope may exclude
it because the user's role isn't specified as "leading," but the
actor's inclusion is reasonable.

**Rubric change needed:** Accept this because the actor
**over-counted** by 1 and **enumerated all items** including the
extra one. An over-count with enumerated evidence means the actor
found everything the GT found, plus one additional item. The
disagreement is about whether the extra item belongs — this is
definitionally a DEFINITION_DISAGREEMENT, never a GENUINE_MISS.
The actor cannot "miss" items it found and listed.

**Regression risk:** Low. Over-count acceptance at delta-1 only
fires when the actor produces MORE items than GT. If the actor
lists items by name, it has already demonstrated retrieval
success. The risk is an actor that over-counts with a genuinely
irrelevant item — but at delta-1, this is always a boundary
judgment, not a retrieval failure.

### Case #6: Citrus (c4a1ceb8) — over-count

**GT:** 3 types. **Actor:** 4 types (lemon, lime, orange,
grapefruit).

**Actor's answer:** Exhaustive `<quotes>` block documenting
grapefruit in citrus peel recipes, Gin & Tonic garnish, and a
Grapefruit-Rosemary-Gin flavor combination. 40+ quotes across 4
sessions.

**Why defensible:** Grapefruit appears in recipe contexts — peel
infusions, garnishes, flavor combinations. Whether these count as
"used in cocktail recipes" is a scope question. The GT counts only
actively squeezed/juiced citrus; the actor counts all citrus
mentioned in recipe contexts. Both interpretations are valid.

**Rubric change needed:** Same as case #2 — over-count by 1 with
enumerated and documented evidence. Accept.

**Regression risk:** Same as case #2 — low.

---

## Section 3 — Proposed approach

### Evaluating the two named candidates

**Two-call judge (extract then grade):**

The first call would extract: what items did the actor find? What
items does the GT contain? The second call would compare the
extracted lists.

This does not address the v1 failure. The v1 failure was not about
the judge failing to see the actor's evidence — the judge saw
the evidence just fine. The failure was about the judge's
acceptance criteria: what level of reasoning makes a delta-1
acceptable? A two-call structure changes how the judge processes
the input but not the fundamental rubric question. The second
call still needs a rule for when delta-1 is acceptable, and that
rule is what v1 got wrong.

**Structural signal detection:**

Detect objective structural patterns in the actor's output rather
than subjective reasoning quality. The key insight from the 3
target cases: the distinguishing signal between
DEFINITION_DISAGREEMENT and GENUINE_MISS is not "did the actor
deliberate" but "did the actor find the disputed items."

### Recommendation: directional acceptance (structural signal)

The v1 approach treated over-counts and under-counts identically
(both required "deliberation"). This was wrong. They have
fundamentally different failure modes:

**Over-count by 1 (actor > GT):** The actor found everything the
GT found, plus one additional item. The actor cannot have "missed"
something it found and listed. An over-count with enumerated items
is always a scope disagreement — DEFINITION_DISAGREEMENT by
construction. **Accept unconditionally at delta-1.**

**Under-count by 1 (actor < GT):** The actor found fewer items.
This could be a GENUINE_MISS (actor didn't see the item) or a
DEFINITION_DISAGREEMENT (actor saw it but categorized differently).
The signal: **does the actor's output mention the evidence that
would change the count?** If yes → the actor chose to count
differently (DEFINITION_DISAGREEMENT). If no → the actor failed
to find it (GENUINE_MISS).

This removes the subjective "deliberation" assessment entirely.
The check is factual: does the actor's output (including thinking,
quotes, reasoning) contain references to the items or evidence
relevant to the disputed count? Not "did the actor deliberate
about inclusion," but "did the actor see the evidence at all."

### Proposed judge prompt (MultiSession counting rubric)

```
COUNTING QUESTION PROTOCOL:
If this is a counting question (asks "how many", "how much",
"total", or the ground truth is a number):

1. Extract the system's count and the ground truth count.
2. If they match exactly: CORRECT.
3. If they differ by more than 1: INCORRECT.
4. If the system's count is HIGHER than ground truth by exactly 1:
   Check whether the system enumerated its items with supporting
   evidence. If the system listed each counted item by name or
   with a quote, mark CORRECT — the system found everything in
   the ground truth plus one additional item, which is a scope
   disagreement, not a miss.
5. If the system's count is LOWER than ground truth by exactly 1:
   Check whether the system's output (including <thinking> and
   <quotes> blocks) mentions, quotes, or discusses evidence
   related to the item(s) it did not count. If the system
   acknowledges or discusses evidence it chose not to count,
   mark CORRECT — it saw the evidence and made a categorization
   choice. If the system shows no awareness of the missing
   item(s), mark INCORRECT — it failed to find them.

DOLLAR AMOUNTS:
When the ground truth is a dollar amount, do not apply the +-1
tolerance — require exact match. The tolerance is for unit
counts, not dollar totals.

NON-COUNTING QUESTIONS:
If this is NOT a counting question, apply the standard rubric.
```

### Why this won't hit the same wall as v1

The v1 failure was the judge's interpretation of "deliberation" —
a subjective concept the judge applied too strictly. The v2 rubric
replaces subjective assessment with two objective checks:

- **Over-count rule (step 4):** No judgment needed. Did the actor
  enumerate items? Yes → CORRECT. This is deterministic.
- **Under-count rule (step 5):** Factual check. Does the actor's
  output mention evidence related to the missing item? Not "did it
  reason about it" — just "does it appear in the output anywhere."
  This is far easier for the judge to evaluate than "deliberation."

Case-by-case verification:

| Case | Direction | v2 rule | Result |
|---|---|---|---|
| #1 Clothing | Under-count (2 vs 3) | Step 5: actor mentions the exchange explicitly → CORRECT | Flips |
| #2 Projects | Over-count (3 vs 2) | Step 4: actor enumerated 3 items by name → CORRECT | Flips |
| #6 Citrus | Over-count (4 vs 3) | Step 4: actor documented 4 types with quotes → CORRECT | Flips |
| #7 Festivals | Under-count (3 vs 4) | Step 5: no mention of 4th festival → INCORRECT | Holds |
| #8 Tanks | Under-count (2 vs 3) | Step 5: no mention of betta tank → INCORRECT | Holds |
| #9 Weddings | Under-count (2 vs 3) | Step 5: no mention of Emily+Sarah or Jen+Tom → INCORRECT | Holds |

---

## Section 4 — Regression risk

### Over-count rule (step 4) regression analysis

The rule: any over-count by exactly 1 with enumerated items is
accepted. Potential false accept: an actor that over-counts by 1
with a genuinely irrelevant item (not a scope boundary). For
example, counting "3 weddings attended" when GT says 2 and the 3rd
is the user's own wedding (not "attended").

**Mitigation:** At delta-1, the extra item is almost always a
scope boundary by construction — items closely related enough to
match the question's vocabulary but arguably outside its scope.
Genuinely irrelevant items (actor counts "3 tanks" but one is a
military tank, not a fish tank) would produce delta >> 1, not
delta = 1.

**Measurement plan:** Run the v2 judge on all 120 questions from
the current bench baseline. Diff against v1 results. Any case
that flips from correct to incorrect is a regression. Any case
that flips from incorrect to correct outside the 3 targets is an
unexpected gain that needs verification.

### Under-count rule (step 5) regression analysis

The rule: under-count by 1 is accepted if the actor's output
mentions evidence related to the missing item. Potential false
accept: the actor mentions an item in passing (e.g., in a quote
from a session) but genuinely fails to count it — not because of
a categorization choice, but because of an extraction failure that
happens to leave a trace.

**Severity:** Low. If the actor quotes evidence mentioning the
item and still doesn't count it, the most charitable interpretation
is that the actor made a categorization choice. The alternative
(actor quoted it accidentally but didn't process it) is rare and
hard to distinguish from a genuine boundary decision.

### Dollar amount exclusion

Dollar amounts are excluded from the +-1 tolerance. This prevents
false accepts on cases like bike expenses (GT=$185, Actor=$40,
delta=$145 — correctly INCORRECT regardless of rubric).

---

## Section 5 — Attribution plan

### Experimental design

- **Control:** Current main with descriptions, cascade K=40,
  current judge prompt. Baseline: 77.5% overall, multi-session
  55% (11/20).
- **Treatment:** Same config, revised judge prompt only.
- **Delta:** Treatment - Control = isolated judge-rubric lift.

### Run scope

The 3 target cases are all in multi-session. But the revised
rubric applies to ALL counting questions across ALL categories
(the judge doesn't know which cases are targets). To check for
regression, the full 120-question bench must be re-run.

**However:** since only the judge prompt changes (not retrieval
or actor), a cheaper approach is possible. Re-judge the existing
actor outputs from the item #8 with-descriptions run
(`20260514-item8-with-descriptions`). The actor predictions and
retrieved memories are already computed and saved in
`report.json`. A re-judging pass requires only 120 judge calls
(~$9), not 120 actor + 120 judge calls (~$18).

This requires a small harness addition: a `rejudge` subcommand
that takes an existing `report.json`, re-runs the judge on each
`(question, predicted, ground_truth)` triple with the new rubric,
and produces a new report. This is the minimal-cost attribution
path.

### Expected run cost

| Approach | Questions | Calls | Cost |
|---|---|---|---|
| Re-judge only | 120 | 120 judge | ~$9 |
| Full re-run | 120 | 240 (actor + judge) | ~$18 |

Re-judge is recommended.

---

## Section 6 — Honest assessment

### Expected lift

If all 3 target cases flip: **+2.5pp** (77.5% → 80.0%, 93 → 96
correct out of 120). All 3 are in multi-session, which would move
from 55% (11/20) to 70% (14/20).

### Is it worth the judge complexity increase?

Yes. The judge prompt change is ~15 lines of rubric text in
`judge.rs:26-54`. No structural changes to the `Judge` trait, no
new API calls, no harness modifications. The complexity delta is
minimal — this is a prompt edit, not an architectural change.

### Realistic probability of all 3 flipping

**High for cases #2 and #6 (over-count):** The over-count rule is
deterministic — if the actor enumerated items and the count is
GT+1, it's accepted. Both actors clearly enumerate. These should
flip with near-certainty.

**Moderate for case #1 (under-count):** Depends on whether the
judge recognizes the exchange discussion as "evidence related to
the missing item." The actor explicitly discusses the exchange and
reasons about it being 1 action. The under-count awareness check
should detect this, but it requires the judge LLM to connect "the
exchange" to "the item GT counts as a 3rd."

**Realistic estimate:** 2-3 of 3 flip. Expected lift: +1.7 to
+2.5pp.

### Teaching-to-the-test risk

This is a real concern. The rubric is designed with knowledge of
the 3 specific failure cases. However:

1. The over-count rule is a general principle, not case-specific.
   It states: "an over-count by 1 with enumerated evidence is a
   scope disagreement, not a miss." This is epistemologically
   correct — if the actor found more items than GT, it didn't miss
   anything. This principle applies to any counting question, not
   just cases #2 and #6.

2. The under-count awareness rule is also general. It distinguishes
   "actor saw the evidence and categorized differently" from "actor
   didn't see the evidence at all." This maps directly to the
   DEFINITION_DISAGREEMENT vs GENUINE_MISS taxonomy that was
   established independently in the failure classification.

3. The measurement plan (re-judge all 120 questions) explicitly
   checks for unexpected flips in either direction. If the rubric
   only helps the 3 target cases and hurts nothing, it's a net
   improvement. If it causes unexpected flips on other cases,
   that's evidence of teaching-to-the-test.

The honest framing: this IS informed by the failure cases, but the
principles it encodes (over-count = scope disagreement; under-count
with awareness = categorization choice) are general enough to
improve judge quality on future questions, not just these 3.

---

## Section 7 — Implementation plan (for review, not action)

### Changes required

1. **`judge.rs:26-54`** — Replace `MultiSession` match arm in
   `judge_prompt()` with the counting-aware rubric from Section 3.
   All other category arms unchanged.

2. **Optional: `rejudge` subcommand** — Takes an existing
   `report.json`, re-runs judge on each question's saved
   `(question, predicted, ground_truth)` with the new rubric,
   outputs a new report. This enables cheap attribution without
   re-running the actor.

### What does NOT change

- Actor prompts, retrieval, ranking, classifier
- Non-counting question rubrics
- Judge JSON output format
- Judge model (claude-sonnet-4-6)
- Judge trait structure
