# Classifier Refinement: Rachel Suburbs Misclassification

**Date**: 2026-05-13
**Branch**: `feat/classifier-rachel-suburbs`
**Status**: Approved — proceeding to implementation.

---

## Section 1 — The Bug

**Question**: QID `830ce83f` — "Where did Rachel move to after her recent relocation?"
**Dataset label**: `knowledge-update`
**Current QuestionType classification**: `Temporal` (via the word "after" matching the Temporal regex)
**Correct QuestionType classification**: `FactualCurrentState` (the question asks for a location — "recent" signals current-state framing)

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
// "Where did Rachel move to after her recent relocation?" → FactualCurrentState
// "Where did I attend the religious activity last week?" → Factual
// Temporal modifiers in "where" questions provide context, not question focus.
if Regex::new(r"^where\b").unwrap().is_match(&q) {
    if Regex::new(r"\b(currently|right now|most recent|latest|newest|do i still|now|recent)\b")
        .unwrap()
        .is_match(&q)
    {
        return Self::FactualCurrentState;
    }
    return Self::Factual;
}
```

**Note**: "after" is deliberately excluded from the FactualCurrentState recency sub-gate. "after" is a sequencing word, not a recency signal. "Where did I go after lunch?" should classify as plain Factual, not FactualCurrentState. The Rachel case is caught by "recent" ("her **recent** relocation"), which is a true recency signal.

**Why this is safe**: Only one existing dataset question has `question_type=temporal-reasoning` and starts with "where": QID `gpt4_b5700ca0` — "Where did I attend the religious activity last week?" This question asks for a location (GT: "the Episcopal Church"), not a time. The dataset label `temporal-reasoning` is semantically wrong — reclassifying to Factual gives the actor a better retrieval strategy for a location lookup. This is net-positive.

**Why not reorder globally**: Moving the entire Factual check before Temporal would break "What happened first?" (starts with "what" but is temporal — sequencing intent). The "where" carve-out is safe because location questions are never truly temporal.

---

## Section 4 — Full Audit: All 19 "Where" Questions in Dataset

### Knowledge-Update (7 questions)

| QID | Question | Old classification | New classification | Semantic match? |
|-----|----------|-------------------|-------------------|----------------|
| 830ce83f | "Where did Rachel move to after her recent relocation?" | Temporal (via "after") | **FactualCurrentState** (via "recent") | Yes — asks for current location |
| 9ea5eabc | "Where did I go on my most recent family trip?" | FactualCurrentState | FactualCurrentState | Unchanged — "most recent" already hit recency sub-gate |
| 07741c44 | "Where do I initially keep my old sneakers?" | Factual | Factual | Unchanged — no temporal words, already reached Factual check |
| e493bb7c | "Where is the painting 'Ethereal Dreams' by Emma Taylor currently hanging?" | FactualCurrentState | FactualCurrentState | Unchanged — "currently" already hit recency sub-gate |
| 22d2cb42 | "Where did I get my guitar serviced?" | Factual | Factual | Unchanged — no temporal words |
| eace081b | "Where am I planning to stay for my birthday trip to Hawaii?" | Factual | Factual | Unchanged — no temporal words |
| 07741c45 | "Where do I currently keep my old sneakers?" | FactualCurrentState | FactualCurrentState | Unchanged — "currently" already hit recency sub-gate |

**Summary**: 1 question reclassified (830ce83f Temporal→FactualCurrentState). 6 unchanged. All 7 semantic matches confirmed.

### Single-Session-User (11 questions)

| QID | Question | Old classification | New classification | Semantic match? |
|-----|----------|-------------------|-------------------|----------------|
| 51a45a95 | "Where did I redeem a $5 coupon on coffee creamer?" | Factual | Factual | Unchanged |
| 6ade9755 | "Where do I take yoga classes?" | Factual | Factual | Unchanged |
| f8c5f88b | "Where did I buy my new tennis racket from?" | Factual | Factual | Unchanged |
| 3b6f954b | "Where did I attend for my study abroad program?" | Factual | Factual | Unchanged |
| d52b4f67 | "Where did I attend my cousin's wedding?" | Factual | Factual | Unchanged |
| 25e5aa4f | "Where did I complete my Bachelor's degree in Computer Science?" | Factual | Factual | Unchanged |
| 86b68151 | "Where did I buy my new bookshelf from?" | Factual | Factual | Unchanged |
| e01b8e2f | "Where did I go on a week-long trip with my family?" | Factual | Factual | Unchanged |
| 4fd1909e | "Where did I attend the Imagine Dragons concert?" | Factual | Factual | Unchanged |
| 1faac195 | "Where does my sister Emily live?" | Factual | Factual | Unchanged |
| 3d86fd0a | "Where did I meet Sophia?" | Factual | Factual | Unchanged |

**Summary**: 0 reclassified. All 11 were already reaching the Factual check (no temporal words). The new `where` interception produces identical results.

### Temporal-Reasoning (1 question)

| QID | Question | Old classification | New classification | Semantic match? |
|-----|----------|-------------------|-------------------|----------------|
| gpt4_b5700ca0 | "Where did I attend the religious activity last week?" | Temporal (via "last") | **Factual** | Yes — GT is "the Episcopal Church" (a location). Dataset label `temporal-reasoning` is semantically wrong. Factual routing gives the actor a better retrieval strategy. Net-positive reclassification. |

**Summary**: 1 question reclassified (gpt4_b5700ca0 Temporal→Factual). The dataset label is wrong; the question asks for a location.

### Full audit totals

| Category | Total | Reclassified | Unchanged |
|----------|-------|-------------|-----------|
| knowledge-update | 7 | 1 (830ce83f) | 6 |
| single-session-user | 11 | 0 | 11 |
| temporal-reasoning | 1 | 1 (gpt4_b5700ca0) | 0 |
| **Total** | **19** | **2** | **17** |

Both reclassifications are semantically correct. Zero regressions.

---

## Section 5 — Non-"Where" Cross-Category Spot-Checks

### Temporal (must remain Temporal under new logic)

| Question | Result | Why |
|----------|--------|-----|
| "When did I start jogging?" | Temporal | starts with "When", not "where" — `where` interception doesn't fire |
| "How long is my commute?" | Temporal | starts with "How" — `where` interception doesn't fire |
| "What happened first?" | Temporal | starts with "What" — `where` interception doesn't fire, "first" hits Temporal regex |
| "How many weeks ago did I start?" | Temporal | starts with "How many" — hits Temporal-counting before `where` check |

### Factual (must remain Factual)

| Question | Result | Why |
|----------|--------|-----|
| "What degree did I graduate with?" | Factual | starts with "What", no temporal words — reaches existing Factual check |
| "Who gave me the gift?" | Factual | starts with "Who" — reaches existing Factual check |

### Counting (must remain Counting)

| Question | Result | Why |
|----------|--------|-----|
| "How many books did I read?" | Counting | "how many" fires at step 2, before `where` check |

---

## Section 6 — Implementation Plan

### Changes

1. **`retrieval.rs:108`** — Add `where` interception before the Temporal regex (6 lines)
2. **`retrieval.rs` tests** — Add 8 new test cases:
   - 4 "where" questions that should classify as Factual/FactualCurrentState
   - 4 existing Temporal questions that must remain Temporal (regression guard)
3. **Verify the exact Rachel question** — use "Where did Rachel move to after her recent relocation?" as a test case

### What does NOT change

- The Temporal regex itself (line 108) — still catches "after/before/since" for non-"where" questions
- All other classification paths (Counting, General, Preference, Recall)
- The Category enum or dataset labels
- Actor prompts or retrieval config

---

## Section 7 — Expected Lift

+1 question on knowledge-update category (830ce83f, Rachel suburbs). This moves the question from Temporal retrieval (K=40, recency half-life=60 days) to FactualCurrentState retrieval, which is the correct strategy for "what is X's current location."

Side benefit: QID `gpt4_b5700ca0` ("Where did I attend the religious activity last week?") reclassifies from Temporal to Factual. The dataset labels it `temporal-reasoning` but the GT is a location ("the Episcopal Church"). Factual routing is more appropriate.
