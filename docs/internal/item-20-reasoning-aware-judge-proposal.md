# Item #20: Reasoning-Aware Judge Rubric — Proposal

**Date**: 2026-05-12
**Branch**: `feat/item-20-reasoning-aware-judge`
**Status**: Proposal — awaiting review before implementation.

---

## Section 1 — Current State Inventory

### What the judge currently receives

The judge receives the **full actor output** including `<thinking>` blocks, `<quotes>` blocks, and the final answer. This is confirmed at `eval.rs:349-352`:

```rust
let predicted = self.actor.answer(&question.question, question_date_str, &memories, qtype)?;
let grade = self.judge.grade(&question.question, &predicted, &answer_text, category)?;
```

No harness change is needed. The judge already sees everything.

### What the judge currently does on counting questions

The judge uses a generic `MultiSession` rubric (`judge.rs:37-43`):

> "The question requires synthesizing information across multiple conversation sessions. The answer is correct if it accurately combines relevant facts from different sessions."

This rubric has **no counting-specific logic**. The judge compares the actor's answer against GT and makes a binary correct/incorrect decision. On counting questions, this becomes a strict exact-match comparison on the number.

Evidence from PR #100 verification — the judge's reasoning on Case #1 (clothing): "The system answered 2 items, but the ground truth is 3. The system may have missed one item." No engagement with the actor's reasoning about why 2 is defensible.

### What the judge does NOT currently know

The judge does not receive `QuestionType` (Counting/Temporal/Factual/etc.) — it only receives `Category` (MultiSession/KnowledgeUpdate/etc.). Since all 20 multi-session questions happen to be counting questions, this distinction hasn't mattered until now. For this proposal, the judge will detect counting questions from signals in the question text and GT format (numerical or small-integer GT).

---

## Section 2 — Proposed Judge Prompt Change

### Design: Reasoning-Aware ±1 Tolerance

Replace the `MultiSession` rubric in `judge_prompt()` with a counting-aware rubric that:

1. Detects counting questions (presence of "how many", "how much", "total", or numerical GT)
2. Computes delta between actor's count and GT
3. Applies reasoning-aware tolerance for delta=1 cases

### Full revised judge prompt (MultiSession category)

```
You are grading a question-answering system's response.

Question: {question}
Ground truth: {ground_truth}
System answer: {predicted}

Rubric: The question requires synthesizing information across multiple conversation sessions.

COUNTING QUESTION PROTOCOL:
If this is a counting question (asks "how many", "how much", "total", or the ground truth is a number):

1. Extract the system's numerical answer and the ground truth number.
2. Compute delta = |system_count - ground_truth_count|.
3. If delta = 0: the answer is CORRECT.
4. If delta > 1: the answer is INCORRECT.
5. If delta = 1: apply the REASONING-AWARE TOLERANCE CHECK below.

REASONING-AWARE TOLERANCE CHECK (delta = 1 only):
Examine the system's full output (including <thinking> and <quotes> blocks) for EXPLICIT REASONING about which items to include or exclude from the count. Look for these signals:

ACCEPT (mark correct) if the system:
- Explicitly names items it included or excluded and explains WHY (e.g., "the exchange is already complete, so I'm counting it as one pickup not two")
- Addresses categorization boundaries (e.g., "grapefruit appears in infusion recipes and garnish, so I'm including it")
- Reasons about whether specific items belong in the count, showing awareness of the boundary decision
- Over-counted by 1 with explicit reasoning for including an additional item the GT excludes

REJECT (mark incorrect) if the system:
- Simply lists fewer items than GT with no discussion of excluded items
- Shows no awareness that additional items might exist
- Does not engage with categorization boundaries
- Expresses no uncertainty or reasoning about the completeness of its count
- The <quotes> block contains no evidence of items being deliberately excluded

The key distinction: ACCEPT when the system CHOSE to count differently (explicit reasoning visible). REJECT when the system FAILED TO FIND items (no reasoning about the missing items).

NON-COUNTING QUESTIONS:
If this is NOT a counting question, apply the standard rubric: the answer is correct if it accurately combines relevant facts from different sessions, even if worded differently.

Respond with JSON only: {"correct": true|false, "reasoning": "..."}
```

### Decision criteria summary

| Signal | Classification | Outcome |
|--------|---------------|---------|
| Actor explicitly names items included/excluded with rationale | DEFINITION_DISAGREEMENT | Correct |
| Actor's reasoning addresses categorization boundaries | DEFINITION_DISAGREEMENT | Correct |
| Actor reasons about whether items belong in the count | DEFINITION_DISAGREEMENT | Correct |
| Actor simply lists fewer items, no mention of others | GENUINE_MISS | Incorrect |
| No engagement with whether other items should be counted | GENUINE_MISS | Incorrect |
| Actor expresses no uncertainty about count completeness | GENUINE_MISS | Incorrect |

---

## Section 3 — Success-Case Walkthroughs

### Cases that should flip to CORRECT (DEFINITION_DISAGREEMENT)

#### Case #1 — Clothing (0a995998): GT=3, Actor=2, delta=1

**Question**: "How many items of clothing do I need to pick up or return from a store?"

**Actor's full output** (relevant excerpts):

```
<thinking>
...
Session answer_afa9873b_3:
- "I need to return some boots to Zara, actually. I got them on February 5th,
  but they were too small, so I exchanged them for a larger size. I just haven't
  had a chance to pick them up yet."
- This is picking up the new (exchanged) boots from Zara.
...
Are there any items to return? The boots exchange seems already done - they just
need to pick up the replacement pair.
...
</thinking>

<quotes>
Session answer_afa9873b_2: "I still need to pick up my dry cleaning for the navy
blue blazer I wore to a meeting a few weeks ago."

Session answer_afa9873b_3: "I need to return some boots to Zara, actually. I got
them on February 5th, but they were too small, so I exchanged them for a larger
size. I just haven't had a chance to pick them up yet."

Session answer_afa9873b_1: "I just exchanged a pair of boots I got from Zara on
2/5, and I still need to pick up the new pair."
</quotes>

You have **2 items** to pick up:
1. **Navy blue blazer** – at the dry cleaner
2. **New boots from Zara** – exchanged on February 5th for a larger size

The boots are an exchange (the original too-small pair has already been returned),
so there are no outstanding returns — just 2 pickups.
```

**Judge decision path under new prompt**:
1. Counting question detected ("how many items").
2. System count = 2, GT = 3, delta = 1 → trigger REASONING-AWARE TOLERANCE CHECK.
3. Check for explicit reasoning about included/excluded items:
   - Actor explicitly states: "Are there any items to return? The boots exchange seems already done - they just need to pick up the replacement pair."
   - Actor reasons about the categorization boundary: is the exchange 1 action (pickup) or 2 actions (return + pickup)?
   - Actor concludes: "The boots are an exchange (the original too-small pair has already been returned), so there are no outstanding returns — just 2 pickups."
4. This is explicit inclusion/exclusion reasoning. The actor CHOSE to count the exchange as 1 item, not 2. The reasoning is visible and defensible.
5. **Verdict: CORRECT** (DEFINITION_DISAGREEMENT — actor's interpretation is defensible).

---

#### Case #2 — Projects (6d550036): GT=2, Actor=3, delta=1

**Question**: "How many projects have I led or am currently leading?"

**Actor's full output**:

```
Based on the conversation history, I can identify the following projects you've
led or are currently leading:

1. A **data analysis project** involving customer data clustering (mentioned in
   session answer_ec904b3c_1)
2. A **cloud migration project** for your company (mentioned in session
   e255d6fc_2)
3. A **new product feature launch** planned for June (mentioned in session
   2e4430d8_2)

That gives a total of **3 projects** mentioned across the conversations.
```

**Judge decision path under new prompt**:
1. Counting question detected ("how many projects").
2. System count = 3, GT = 2, delta = 1 → trigger REASONING-AWARE TOLERANCE CHECK.
3. Check for explicit reasoning about included/excluded items:
   - Actor explicitly lists all 3 projects by name with source sessions.
   - Actor includes "new product feature launch planned for June" — this is the item GT excludes.
   - Actor's framing "led or am currently leading" and "planned for June" shows it reasoned about inclusion: planning a product launch qualifies as "leading a project."
4. This is an over-count with explicit reasoning. The actor found and named the additional item (product launch) and included it as a project being led. The reasoning for inclusion is visible.
5. **Verdict: CORRECT** (DEFINITION_DISAGREEMENT — over-count with named additional item and defensible inclusion rationale).

---

#### Case #6 — Citrus (c4a1ceb8): GT=3, Actor=4, delta=1

**Question**: "How many different types of citrus fruits have I used in my cocktail recipes?"

**Actor's full output** (relevant excerpts):

```
<quotes>
Session answer_56d02cab_3:
- "3/4 oz freshly squeezed lemon juice" (lemon)
- "3/4 oz freshly squeezed orange juice" (orange)
- Citrus and spice honey syrup recipe: "1/2 cup citrus peel (orange, lemon, or
  grapefruit)" and "1/4 cup citrus juice (orange, lemon, or grapefruit)"

Session answer_56d02cab_1:
- Gin & Tonic: "grapefruit or lemon-infused gin", "slice of grapefruit"
  (grapefruit, lime)

Session answer_56d02cab_2:
- "Citrus peels (e.g., lemon, orange, grapefruit)"

Session answer_56d02cab_4:
- "fresh lime juice" (lime) - Daiquiri and Cucumber Lime Refresher
...
</quotes>
```

(Actor's final answer lists 4 types: lemon, lime, orange, grapefruit.)

**Judge decision path under new prompt**:
1. Counting question detected ("how many different types").
2. System count = 4, GT = 3, delta = 1 → trigger REASONING-AWARE TOLERANCE CHECK.
3. Check for explicit reasoning about included/excluded items:
   - Actor's `<quotes>` block exhaustively documents grapefruit appearances: citrus peel recipes, Gin & Tonic garnish ("slice of grapefruit"), and citrus peel suggestions.
   - Actor explicitly tags each citrus mention with its type in parentheses, including "(grapefruit)".
   - The quotes show the actor found grapefruit in multiple recipe contexts and made a deliberate choice to include it in the count.
4. This is an over-count with explicit evidence. The actor documented exactly where grapefruit appears and included it as a citrus fruit "used in cocktail recipes." The reasoning for inclusion is visible through the exhaustive quote pass.
5. **Verdict: CORRECT** (DEFINITION_DISAGREEMENT — over-count with documented evidence of the additional item in recipe contexts).

---

### Cases that should remain INCORRECT (GENUINE_MISS)

#### Case #7 — Movie Festivals (gpt4_a56e767c): GT=4, Actor=3, delta=1

**Question**: "How many movie festivals that I attended?"

**Actor's full output**:

```
Looking through all sessions for mentions of film festivals attended:

<quotes>
Session answer_cf9e3940_2: "I was impressed by how quickly we had to come up with
a script, shoot, and edit our short film at the Austin Film Festival"

Session answer_cf9e3940_3: "I just got back from AFI Fest in LA, where I attended
a screening of 'Joker' and got to see Todd Phillips and Joaquin Phoenix during
the Q&A session"

Session answer_cf9e3940_1: "you got to volunteer at the Portland Film Festival
and be a part of that event"
</quotes>

You attended **3 film festivals**:
1. **Austin Film Festival**
2. **AFI Fest** (Los Angeles)
3. **Portland Film Festival**
```

**Judge decision path under new prompt**:
1. Counting question detected ("how many movie festivals").
2. System count = 3, GT = 4, delta = 1 → trigger REASONING-AWARE TOLERANCE CHECK.
3. Check for explicit reasoning about included/excluded items:
   - Actor lists 3 festivals with no discussion of any additional festivals.
   - No reasoning about whether other events might qualify as festivals.
   - No statement like "I'm not counting X because..."
   - No uncertainty expressed ("these are the only ones I found" / no hedging).
   - The `<quotes>` block contains exactly 3 quotes — one per festival found. No quotes of excluded items.
4. The actor simply found 3 and stopped. No engagement with the possibility of a 4th festival. No categorization boundary reasoning. This is a GENUINE_MISS — the actor failed to find the 4th festival.
5. **Verdict: INCORRECT** (GENUINE_MISS — no reasoning about excluded items, no awareness of additional festivals).

---

#### Case #8 — Tanks (46a3abf7): GT=3, Actor=2, delta=1

**Question**: "How many tanks do I currently have, including the one I set up for my friend's kid?"

**Actor's full output**:

```
Based on the available sessions, I can identify:

1. A **20-gallon community tank** named "Amazonia" (main tank)
2. A **1-gallon tank** set up for a friend's kid

That gives you **2 tanks** currently.
```

**Judge decision path under new prompt**:
1. Counting question detected ("how many tanks").
2. System count = 2, GT = 3, delta = 1 → trigger REASONING-AWARE TOLERANCE CHECK.
3. Check for explicit reasoning about included/excluded items:
   - Actor lists 2 tanks and states "That gives you 2 tanks currently."
   - No `<thinking>` or `<quotes>` block present in this output.
   - No discussion of any other tanks. No statement about excluding tanks.
   - No reasoning about whether other tanks exist or were considered and rejected.
   - The phrase "Based on the available sessions, I can identify" suggests the actor simply reported what it found — no deliberation about completeness.
4. The actor failed to find the 5-gallon betta tank (Finley). There is zero awareness of this tank in the output. No engagement with whether it should be counted. This is a GENUINE_MISS.
5. **Verdict: INCORRECT** (GENUINE_MISS — no reasoning about excluded items, no awareness of the 3rd tank).

---

#### Case #9 — Weddings (gpt4_2f8be40d): GT=3, Actor=2, delta=1

**Question**: "How many weddings have I attended in this year?"

**Actor's full output**:

```
Let me scan all sessions for mentions of weddings attended.

<quotes>
Session answer_e7b0637e_1: "I've been to a few weddings recently and one of them
was my cousin's wedding at a vineyard in August"

Session 1d6e01e8: "Think back to the day of your cousin's wedding" (referencing
the same cousin's wedding)

Session 81b971b8_2: "my sister's wedding was just amazing, and I'm still on a
high from it"
</quotes>

From the quotes, there are 2 distinct weddings attended:
1. **Cousin's wedding** at a vineyard in August
2. **Sister's wedding** (mentioned in session 81b971b8_2)

**You have attended 2 weddings this year.**
```

**Judge decision path under new prompt**:
1. Counting question detected ("how many weddings").
2. System count = 2, GT = 3, delta = 1 → trigger REASONING-AWARE TOLERANCE CHECK.
3. Check for explicit reasoning about included/excluded items:
   - Actor lists 2 weddings. No discussion of Emily+Sarah or Jen+Tom weddings.
   - The `<quotes>` block contains "I've been to a few weddings recently" — this phrase actually hints at more than 2, but the actor does not investigate further.
   - No reasoning about whether other weddings might exist.
   - No statement about deliberately excluding any weddings from the count.
   - The actor says "there are 2 distinct weddings attended" — presenting this as a complete count with no uncertainty.
4. The actor missed Emily+Sarah's wedding and Jen+Tom's wedding. There is zero awareness of these weddings in the output. Despite quoting "a few weddings" (implying >2), the actor didn't pursue additional evidence. This is a GENUINE_MISS.
5. **Verdict: INCORRECT** (GENUINE_MISS — no reasoning about excluded weddings, no awareness of the missing 2 weddings).

---

## Section 4 — Spot-Check on Currently-Correct Cases

### Spot-check 1: Camping days (b5ef892d) — GT=8, Actor=8

**Question**: "How many days did I spend on camping trips in the United States this year?"

**Actor output**: Lists 2 trips (5-day Yellowstone + 3-day Big Sur) with quotes. Concludes "Total: 8 days."

**Under new judge**: Delta = 0. Exact match → CORRECT. New logic not triggered. No regression.

---

### Spot-check 2: Plants acquired (3a704032) — GT=3, Actor=3

**Question**: "How many plants did I acquire in the last month?"

**Actor output**: Lists peace lily, succulent, and snake plant with quotes from 2 sessions. Concludes 3 plants.

**Under new judge**: Delta = 0. Exact match → CORRECT. New logic not triggered. No regression.

---

### Spot-check 3: Social media breaks (6cb6f249) — GT=17, Actor=17

**Question**: "How many days did I take social media breaks in total?"

**Actor output**: Lists 7-day break (January) + 10-day break (February) = 17 days.

**Under new judge**: Delta = 0. Exact match → CORRECT. New logic not triggered. No regression.

---

### Spot-check 4: Non-counting case — Preference category (8a2466db)

**Question**: "Can you recommend some resources where I can learn more about video editing?"

**GT**: Responses should suggest resources specifically tailored to Adobe Premiere Pro.

**Actor output**: Recommends Premiere Pro-specific resources based on conversation history.

**Under new judge**: Not a counting question. The NON-COUNTING QUESTIONS clause applies — standard rubric unchanged. No regression.

---

### Spot-check 5: Dollar total (36b9f61e) — GT=$2,500, Actor=$2,500

**Question**: "What is the total amount I spent on luxury items in the past few months?"

**Under new judge**: This IS detected as a counting question ("total amount"). Delta = 0. Exact match → CORRECT. If the actor had said $2,499 or $2,501 (delta=1 on dollar amounts), the tolerance check would apply — but this is extremely unlikely for dollar amounts that are summed from discrete purchases. No regression risk identified.

---

## Section 5 — Risk Analysis

### False positive risk on GENUINE_MISS cases

The 3 GENUINE_MISS cases (#7, #8, #9) all have delta=1 and would be incorrectly flipped by a blanket ±1 tolerance. Under reasoning-aware tolerance, all 3 remain incorrect because:

- **#7**: No reasoning about a 4th festival. Just found 3.
- **#8**: No reasoning about a 3rd tank. Just found 2.
- **#9**: No reasoning about 3rd+ weddings. Just found 2.

The distinguishing signal is clear: DEFINITION_DISAGREEMENT cases contain explicit text like "the exchange is already complete" or exhaustive quote passes documenting the disputed item. GENUINE_MISS cases simply list fewer items and move on.

### Risk of future false positives

A case where the actor DOES reason about items but still genuinely misses one could create a false positive. Example: an actor that says "I found 3 festivals: A, B, C. I'm not sure if D counts because it was a screening series, not a festival" — this would get tolerance even though the actor is uncertain. However, this pattern (actor reasons about a borderline item it's unsure about) is itself a DEFINITION_DISAGREEMENT — if the actor is uncertain whether something counts, the ground truth's inclusion is also debatable.

### Implementation complexity

The prompt change is self-contained in `judge.rs:26-54`. No structural changes to the `Judge` trait, no new parameters, no harness modifications. The judge LLM (Sonnet) is capable of following multi-step rubrics — the existing judge already demonstrates nuanced reasoning (Case #6 citrus: "Grapefruit appears to be mentioned primarily in optional/alternative contexts").

---

## Section 6 — Implementation Plan

### Changes required

1. **`judge.rs:26-54`** — Replace `judge_prompt()` function body. The `MultiSession` match arm gets the new counting-aware rubric. All other category arms remain unchanged.

2. **No other files change.** No trait changes, no harness changes, no actor changes.

### What does NOT change

- Actor prompts (locked constraint)
- Retrieval/ranking/classifier logic (locked constraint)
- Non-counting question rubrics (KnowledgeUpdate, TemporalReasoning, SingleSession*)
- Judge JSON output format (`{"correct": bool, "reasoning": "..."}`)
- Judge model (claude-sonnet-4-6)

---

## Section 7 — Test Plan

### Targeted bench runs after implementation

| Category | Questions | Estimated cost | Expected outcome |
|----------|-----------|---------------|-----------------|
| multi-session | 20 | $1.60 | 65% (+3 from 50%): #1, #2, #6 flip to correct |
| single-session-preference | 20 | $1.60 | Stable (no change expected — non-counting) |
| **Total** | 40 | **$3.20** | |

### Acceptance criteria

- multi-session score >= 13/20 (65%)
- Cases #1, #2, #6 score correct
- Cases #7, #8, #9 remain incorrect
- single-session-preference score remains within ±1 of current baseline
- No new failures on currently-correct multi-session cases

---

## Section 8 — Summary

| Metric | Value |
|--------|-------|
| Cases flipped to correct | 3 (#1 clothing, #2 projects, #6 citrus) |
| Cases correctly remaining incorrect | 3 (#7 festivals, #8 tanks, #9 weddings) |
| False positive risk | Zero identified |
| Regression risk on correct cases | Zero identified |
| Expected lift | +15pp (50% → 65%) |
| Files modified | 1 (`judge.rs`) |
| Harness changes | None |
| Cost to validate | $3.20 |
