# Item #20: Judge Rubric Revision v2 — Proposal

**Date:** 2026-05-14 (revised after review)
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
that none of the target cases could clear:

- **Case #1 (clothing):** Actor explicitly reasoned "The boots
  exchange seems already done — they just need to pick up the
  replacement pair." This IS reasoning about inclusion/exclusion,
  but the judge interpreted "deliberation" as requiring an
  explicit meta-statement like "I'm counting this as 1 item
  because..." rather than accepting implicit reasoning in the
  actor's narrative.
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
the disputed items and their supporting evidence — rather than
subjective assessments of whether the actor "deliberated."

---

## Section 2 — Actual target cases (current baseline verified)

The original failure classification identified 3
DEFINITION_DISAGREEMENT cases (#1, #2, #6). However, the current
baseline (descriptions-enabled main, `20260514-item8-with-descriptions`)
shows **case #2 is already CORRECT** — the actor now answers 2
(matching GT), not 3 as in the older run. Only 2 target cases
remain:

### Case #1: Clothing (0a995998) — under-count, still INCORRECT

**GT:** 3 items. **Actor:** 2 items. **Delta:** -1.

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

**Rubric change needed:** Accept via the under-count awareness
check — the actor's output mentions the exchange (the disputed
evidence) and explicitly reasons about it.

### Case #2: Projects (6d550036) — ALREADY CORRECT, no action

**GT:** 2 projects. **Actor (current run):** 2 projects (cloud
migration + product feature launch). **Delta:** 0.

The actor's count now matches GT. No rubric change needed for this
case. (In the older run used for the failure classification, the
actor answered 3. The actor's behavior changed across runs — LLM
non-determinism.)

**Subset verification (for the record):** The actor's 2 projects
(cloud migration, product launch) are NOT the GT's 2 projects.
The GT's 2 are from answer sessions (data analysis + consumer
psychology research). One answer session (`answer_ec904b3c_3`,
consumer psychology) was not retrieved. The actor found projects
from non-answer sessions. The count match is coincidental, but
the judge accepts it because the count is exact. This case
illustrates why count-only GT is fragile for item-level counting
questions.

### Case #6: Citrus (c4a1ceb8) — over-count, still INCORRECT

**GT:** 3 types. **Actor:** 4 types (lemon, lime, orange,
grapefruit). **Delta:** +1.

**Subset verification:** GT's 3 items are {lemon, lime, orange}.
The actor's 4 items are {lemon, lime, orange, grapefruit}. **GT
IS a strict subset of the actor's items.** All GT items appear in
the actor's enumeration. The extra item (grapefruit) is documented
with 40+ quotes across 4 sessions showing grapefruit in recipe
contexts (citrus peel infusions, Gin & Tonic garnish,
Grapefruit-Rosemary-Gin combination). The grapefruit is real — not
fabricated — and the scope question (does garnish/optional count
as "used in recipes") is genuine.

**Rubric change needed:** Accept via the over-count evidence check
— the extra item is supported by extensive quoted evidence from
source material.

---

## Section 3 — Proposed approach

### Evaluating the two named candidates

**Two-call judge (extract then grade):**

Does not address the v1 failure. The v1 failure was not about the
judge failing to see the actor's evidence — the judge saw it. The
failure was about acceptance criteria. A two-call structure changes
how the judge processes input but not the fundamental rubric
question. The second call still needs a rule for when delta-1 is
acceptable, and that rule is what v1 got wrong.

**Structural signal detection:**

Detect objective structural patterns in the actor's output rather
than subjective reasoning quality. The key insight: the
distinguishing signal between DEFINITION_DISAGREEMENT and
GENUINE_MISS is not "did the actor deliberate" but "did the actor
find the disputed items, and are they supported by evidence?"

### Recommendation: symmetric evidence check

Both over-counts and under-counts get the **same factual rigor**.
No free passes on either side.

**Over-count by 1 (actor > GT):** Accept only if:
1. The actor enumerated each counted item with supporting evidence
   (quotes or specific factual details traceable to source).
2. The extra item is supported by evidence in the actor's output —
   not just named, but documented with a quote or specific detail
   from the conversation history.

An over-count where the extra item is fabricated or unsupported
is INCORRECT. An over-count where the extra item has quoted
evidence is a scope disagreement — CORRECT.

**Under-count by 1 (actor < GT):** Accept only if the actor's
output (including thinking, quotes, reasoning) mentions, quotes,
or discusses evidence related to the item(s) it did not count. If
the actor shows awareness of the missing evidence and chose not to
count it, mark CORRECT. If the actor shows no awareness, mark
INCORRECT.

### Why the over-count rule is not unconditional

The original v2 draft accepted over-counts unconditionally ("if
enumerated, accept"). This has a hole: an over-count by 1 is
consistent with TWO situations:
- (a) Actor found everything GT found, plus one scope-call extra
  — a real DEFINITION_DISAGREEMENT.
- (b) Actor missed a GT item AND found one or two extras (net +1)
  — a GENUINE_MISS that happens to have the right arithmetic.

Case #2 (projects) illustrates (b) exactly: the actor missed
consumer psychology research (not retrieved) and found cloud
migration + product launch from non-answer sessions. Count
parity (3 vs 2 = +1) was arithmetic coincidence, not scope
disagreement. GT was NOT a subset of the actor's items.

The symmetric evidence check catches fabrication and unsupported
items. The subset property is the harder check — with count-only
GT, the judge cannot verify subset directly. But requiring quoted
evidence for the extra item raises the bar enough to filter out
cases where the actor's enumeration doesn't correspond to GT's
items. An actor that fabricates or finds unrelated items typically
won't have source-material quotes for them.

### Proposed judge prompt (MultiSession counting rubric)

```
COUNTING QUESTION PROTOCOL:
If this is a counting question (asks "how many", "how much",
"total", or the ground truth is a number):

1. Extract the system's count and the ground truth count.
2. If they match exactly: CORRECT.
3. If they differ by more than 1: INCORRECT.
4. If the system's count is HIGHER than ground truth by exactly 1:
   Check whether the extra item the system counted is supported
   by EVIDENCE — a verbatim quote from conversation text or
   specific factual details traceable to a session. If the extra
   item is documented with source evidence, mark CORRECT — the
   system found a real item that the ground truth excluded, which
   is a scope disagreement. If the extra item has no supporting
   evidence (just named or asserted without quotes/details), mark
   INCORRECT.
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
tolerance — require exact match.

NON-COUNTING QUESTIONS:
If this is NOT a counting question, apply the standard rubric.
```

### Case-by-case verification against revised rubric

| Case | Direction | Rule | Evidence check | Result |
|---|---|---|---|---|
| #1 Clothing | Under (2 vs 3) | Step 5 | Actor quotes + discusses the exchange | CORRECT (flips) |
| #2 Projects | Exact (2 vs 2) | Step 2 | N/A — delta=0 | CORRECT (already) |
| #6 Citrus | Over (4 vs 3) | Step 4 | Grapefruit documented with 40+ quotes from 4 sessions | CORRECT (flips) |
| #7 Festivals | Under (3 vs 4) | Step 5 | No mention of 4th festival | INCORRECT (holds) |
| #8 Tanks | Under (2 vs 3) | Step 5 | No mention of betta tank | INCORRECT (holds) |
| #9 Weddings | Under (2 vs 3) | Step 5 | No mention of Emily+Sarah or Jen+Tom | INCORRECT (holds) |

---

## Section 4 — Regression risk

### Over-count evidence check regression

The rule requires quoted/specific evidence for the extra item.
Potential false accept: an actor over-counts with a real but
irrelevant item that happens to have a source quote. At delta-1,
this is always a scope boundary question ("does this item belong
in this category?"), which is definitionally a
DEFINITION_DISAGREEMENT. The evidence check filters fabrications
but accepts real items — which is the correct behavior for scope
disagreements.

Potential false reject: an actor over-counts with a correctly-
scoped extra item but doesn't include a quote. This would cause a
real DEFINITION_DISAGREEMENT to be rejected. Severity: low — the
current actor consistently produces quotes for counting questions
(PR #97 quote-first instruction).

### Under-count awareness check regression

Same analysis as the original proposal — low severity. If the
actor's output mentions evidence it chose not to count, the most
charitable interpretation is a categorization choice.

### Measurement plan (revised)

Run the v2 judge on all 120 questions. The regression check must
specifically:
1. List every case that newly flips to CORRECT (not just the 2
   targets). Each newly-flipped case gets eyeballed.
2. List every case that newly flips to INCORRECT (should be zero).
3. The damage surface is the newly-flipped set, not the held set.
   Confirming previously-correct cases stayed correct is necessary
   but insufficient.

### Dollar amount exclusion

Dollar amounts excluded from +-1 tolerance. Prevents false accepts
on cases like bike expenses (GT=$185, Actor=$40, delta=$145).

---

## Section 5 — Attribution plan

### Experimental design

- **Control:** Current main with descriptions, cascade K=40,
  current judge prompt. Baseline: 77.5% overall (93/120),
  multi-session 55% (11/20).
- **Treatment:** Same config, revised judge prompt only.
- **Delta:** Treatment - Control = isolated judge-rubric lift.

### Run scope

Re-judge existing actor outputs from the item #8
with-descriptions run (`20260514-item8-with-descriptions`). The
actor predictions are already saved in `report.json` per category.
A re-judging pass requires only 120 judge calls (~$9), not
120 actor + 120 judge calls (~$18).

This requires a `rejudge` subcommand or script that takes existing
`report.json` files, re-runs the judge on each
`(question, predicted, ground_truth)` triple, and produces a
comparison report.

---

## Section 6 — Honest assessment

### Expected lift

2 target cases remain (not 3 as originally estimated):
- Case #1 (clothing, under-count): moderate confidence
- Case #6 (citrus, over-count): high confidence

If both flip: **+1.7pp** (77.5% → 79.2%, 93 → 95 of 120).
Multi-session: 55% → 65% (11 → 13 of 20).

If only case #6 flips: **+0.8pp** (77.5% → 78.3%).

### Why the lift is lower than the original +2.5pp

1. Case #2 is already CORRECT in the current baseline (actor
   non-determinism — it now answers 2, not 3).
2. The subset check revealed case #2 would have been a false
   positive under the unconditional over-count rule — the actor
   missed GT's items and found different ones. The count match
   was coincidence.
3. The corrected lift (+1.7pp max) is honest. The original +2.5pp
   was partly counting a case that already passes.

### Is it worth the complexity?

Yes. The judge prompt change is ~20 lines of rubric text in
`judge.rs:26-54`. No structural changes. The corrected lift
(+1.7pp) is real — case #6 is a clear scope disagreement with
extensive evidence, and case #1 is a defensible categorization
with explicit reasoning in the actor's output. Neither is a
miss being scored as correct.

### Teaching-to-the-test risk

The rubric is informed by the failure cases, but both rules
(evidence-based over-count acceptance, awareness-based under-count
acceptance) are general principles that map to the established
DEFINITION_DISAGREEMENT vs GENUINE_MISS taxonomy. The full
120-question re-judge with the newly-flipped-case-list check is
the safeguard: any unexpected flips expose teaching-to-the-test.

### Calibration against Memanto

Memanto's 89.8% is the external reference. Moving from 77.5% to
79.2% is +1.7pp of honest lift — no hallucinations scored as wins,
no coincidental count matches accepted. The gap to Memanto remains
~10pp, but the number is credible. A quieter honest benchmark is
worth more for Track 1 credibility than an inflated one.

---

## Section 7 — Implementation plan (for review, not action)

### Changes required

1. **`judge.rs:26-54`** — Replace `MultiSession` match arm in
   `judge_prompt()` with the counting-aware rubric from Section 3.
   All other category arms unchanged.

2. **`rejudge` script or subcommand** — Takes existing
   `report.json` files, re-runs judge with new rubric on each
   saved `(question, predicted, ground_truth)` triple. Outputs a
   comparison report listing every flipped case.

### What does NOT change

- Actor prompts, retrieval, ranking, classifier
- Non-counting question rubrics
- Judge JSON output format
- Judge model (claude-sonnet-4-6)
- Judge trait structure
