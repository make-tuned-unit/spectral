# Classifier Refinement: Rachel Suburbs Misclassification

**Date**: 2026-05-13
**Branch**: `feat/classifier-rachel-suburbs`
**Status**: Proposal — awaiting review before implementation.

---

## Section 1 — The Bug

**Question**: QID `830ce83f` — "Where did Rachel move to after her recent relocation?"
**Dataset label**: `knowledge-update`
**Current QuestionType classification**: `Temporal` (via the word "after" matching the Temporal regex)
**Correct QuestionType classification**: `Factual` or `FactualCurrentState` (the question asks for a location, not a time sequence)

The `QuestionType::classify()` function in `retrieval.rs:108` has a Temporal regex:
```
when did|how long|(?:^|\W)first\b|(?:^|\W)last\b|before|after|ago|since
```

The word "after" in "after her recent relocation" matches this regex. Because the Temporal check (line 108) runs before the Factual check (line 116), the question routes to Temporal instead of Factual.

**Impact**: The question gets the Temporal retrieval profile (K=40, aggressive recency decay half-life=60 days) and the temporal actor prompt, instead of the Factual retrieval profile (K=30, moderate recency) and factual actor prompt. This likely contributes to the wrong answer.

---

## Section 2 — Root Cause

The classifier cascade checks patterns in this order:
1. Temporal-counting (`how many days ago`) → Temporal
2. Counting (`how many`) → Counting
3. **Temporal** (`when did|how long|first|last|before|after|ago|since`) → Temporal
4. **Factual** (`^where|what|who|which`) → Factual
5. General sub-gates

The problem: step 3 uses broad temporal keywords (`before`, `after`, `since`) that can appear as subordinate clauses in non-temporal questions. "Where did X move to **after** Y?" — "after" is context, not the question's intent.

---

## Section 3 — Proposed Fix

**Approach**: Intercept `where` questions before the generic Temporal check. All "where" questions ask for a location — temporal modifiers are context providing the time frame, not the question's focus.

**Concrete change** in `retrieval.rs:classify()`:

After the Counting check (line 105) and before the Temporal check (line 108), add:

```rust
// Location questions: "where" → Factual, even with temporal modifiers.
// "Where did Rachel move to after her recent relocation?" → Factual
// "Where did I attend the religious activity last week?" → Factual
// Temporal modifiers in "where" questions provide context, not question focus.
if Regex::new(r"^where\b").unwrap().is_match(&q) {
    if Regex::new(r"\b(currently|right now|most recent|latest|newest|do i still|now|after|recent)\b")
        .unwrap()
        .is_match(&q)
    {
        return Self::FactualCurrentState;
    }
    return Self::Factual;
}
```

**Why this is safe**: Only one existing dataset question has `question_type=temporal-reasoning` and starts with "where": QID `gpt4_b5700ca0` — "Where did I attend the religious activity last week?" This question asks for a location, so Factual routing is appropriate. No bench result exists for this question, so no measurable regression.

**Why not reorder globally**: Moving the entire Factual check before Temporal would break "What happened first?" (starts with "what" but is temporal — sequencing intent). The "where" carve-out is safe because location questions are never truly temporal.

---

## Section 4 — Example Classifications (Old vs New)

| # | Question | Old | New | Correct? |
|---|----------|-----|-----|----------|
| 1 | "Where did Rachel move to after her recent relocation?" | Temporal | **FactualCurrentState** | Fixed |
| 2 | "Where did I attend the religious activity last week?" | Temporal | **Factual** | Fixed (location answer) |
| 3 | "Where does my sister live?" | Factual | Factual | Unchanged |
| 4 | "Where is the painting currently hanging?" | FactualCurrentState | FactualCurrentState | Unchanged |
| 5 | "Where did I go on my most recent family trip?" | FactualCurrentState | FactualCurrentState | Unchanged |
| 6 | "When did I start jogging?" | Temporal | Temporal | Unchanged |
| 7 | "How long is my commute?" | Temporal | Temporal | Unchanged |
| 8 | "What happened first?" | Temporal | Temporal | Unchanged |
| 9 | "How many weeks ago did I start?" | Temporal | Temporal | Unchanged |
| 10 | "What degree did I graduate with?" | Factual | Factual | Unchanged |

---

## Section 5 — Cross-Category Spot-Checks

### Temporal (should remain Temporal)

| Question | Result |
|----------|--------|
| "When did I start jogging?" | Temporal — starts with "When", not "where" |
| "How long is my commute?" | Temporal — starts with "How", not "where" |
| "What happened first?" | Temporal — starts with "What", "first" triggers temporal |
| "Which event happened first, the meeting with Rachel or the pride parade?" | Temporal — starts with "Which", "first" triggers temporal |

### Factual (should remain Factual)

| Question | Result |
|----------|--------|
| "What degree did I graduate with?" | Factual — starts with "What", no temporal words |
| "Who gave me the gift?" | Factual — starts with "Who", no temporal words |
| "Where does my sister live?" | Factual — starts with "where", no recency modifiers |

### SingleSession / Counting (should remain unchanged)

| Question | Result |
|----------|--------|
| "How many books did I read?" | Counting — "how many" fires before "where" check |
| "Can you recommend a restaurant?" | GeneralPreference — no "where" involved |
| "How many days did I spend camping?" | Counting — "how many" fires first |

---

## Section 6 — Implementation Plan

### Changes

1. **`retrieval.rs:108`** — Add `where` interception before the Temporal regex (6 lines)
2. **`retrieval.rs` tests** — Add 8 new test cases:
   - 4 "where" questions that should now classify as Factual/FactualCurrentState
   - 4 existing Temporal questions that must remain Temporal (regression guard)
3. **Verify the exact Rachel question** — use "Where did Rachel move to after her recent relocation?" as a test case

### What does NOT change

- The Temporal regex itself (line 108) — still catches "after/before/since" for non-"where" questions
- All other classification paths (Counting, General, Preference, Recall)
- The Category enum or dataset labels
- Actor prompts or retrieval config

---

## Section 7 — Expected Lift

+1 question on knowledge-update category (830ce83f, Rachel suburbs). This moves the question from Temporal retrieval (K=40, recency half-life=60 days) to Factual retrieval, which is the correct strategy for "what is X's current location."

Side benefit: QID `gpt4_b5700ca0` ("Where did I attend the religious activity last week?") also reclassifies from Temporal to Factual, which is more appropriate for a location lookup.
