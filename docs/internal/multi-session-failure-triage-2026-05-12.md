# Multi-Session & Preference Failure Triage

**Date**: 2026-05-12
**Branch**: `investigate/multi-session-failure-triage`
**Data**: `~/spectral-local-bench/eval-runs/20260512-1024-post-86-90/`

---

## Method

For each failure, cross-referenced `retrieved_memory_keys` from the bench report against `answer_session_ids` from the LongMemEval source data. If all answer sessions appear in retrieved keys, the GT items were available to the actor and the failure is an ACTOR_MISS. If answer sessions are absent, the failure is a RETRIEVAL_MISS.

## Multi-Session Failures: 10 total

### Triage Summary

| # | Question | GT | Predicted | Verdict |
|---|----------|-----|-----------|---------|
| 1 | Clothing items to pick up/return | 3 | 2 | ACTOR_MISS |
| 2 | Days on camping trips | 8 | 0 | ACTOR_MISS + SESSION_USER_CONFUSION |
| 3 | Bike expenses total | $185 | $40 | ACTOR_MISS |
| 4 | Different doctors visited | 3 | 0 | RETRIEVAL_MISS (complete) |
| 5 | Citrus fruits in cocktails | 3 | 2 | ACTOR_MISS |
| 6 | Movie festivals attended | 4 | 3 | ACTOR_MISS |
| 7 | Tanks currently owned | 3 | 2 | ACTOR_MISS |
| 8 | Weddings attended this year | 3 | 2 | ACTOR_MISS |
| 9 | Furniture bought/assembled/sold/fixed | 4 | 2 | ACTOR_MISS |
| 10 | Baking events in past 2 weeks | 4 | 3 | ACTOR_MISS |

**Distribution: 9 ACTOR_MISS, 1 RETRIEVAL_MISS, 1 SESSION_USER_CONFUSION (overlaps with ACTOR_MISS)**

### ACTOR_MISS pattern (9/10)

All answer sessions were retrieved. The actor had the evidence and missed items. Sub-patterns:

**Type A — Undercount by 1 (6 cases: #1, #5, #6, #7, #8, #10):** Actor enumerates most items, states a confident final count, misses exactly one. The missed item is present in a retrieved session but either:
- Uses indirect language (e.g., festival mentioned as an activity without the word "festival")
- Is embedded in a longer turn among other topics
- Requires inference (e.g., an exchanged item counts as both a return and a pickup)

**Type B — Undercount by 2+ (2 cases: #3, #9):** Actor finds some items, acknowledges likely incompleteness ("this is likely incomplete"), but doesn't revisit the sessions to find the rest. The bike case (#3) is notable: actor finds 3 other items but says "costs not listed" — the costs are in the sessions, actor didn't extract them.

**Type C — False negative (1 case: #2):** Actor sees the evidence, attributes it to "other users" because different session IDs appear. This is the SESSION_USER_CONFUSION pattern — the camping trips are in sessions answer_a8b4290f_1 and answer_a8b4290f_2, but the actor treats different session IDs as different people.

### RETRIEVAL_MISS (1/10)

**#4 (doctors):** All three answer sessions (answer_55a6940c_1/2/3) are completely absent from retrieved memories. The query "How many different doctors did I visit?" retrieved 88 memories but none from the answer sessions. This is a pure retrieval failure — no prompt change can fix it.

## Preference Failures: 8 total

### Triage Summary

| # | Question | Verdict |
|---|----------|---------|
| 1 | Publications/conferences | ACTOR_MISS |
| 2 | Show/movie to watch | ACTOR_MISS |
| 3 | Dinner with homegrown ingredients | RETRIEVAL_MISS |
| 4 | Phone battery tips | RETRIEVAL_MISS |
| 5 | Chocolate chip cookies | ACTOR_MISS |
| 6 | Guitar shopping tips | ACTOR_MISS |
| 7 | Coffee creamer recipe | RETRIEVAL_MISS |
| 8 | Sneezing / living room | ACTOR_MISS |

**Distribution: 5 ACTOR_MISS, 3 RETRIEVAL_MISS**

### ACTOR_MISS pattern (5/8)

The answer session was retrieved. The actor had the user's specific preference but generated a generic or adjacent recommendation instead. The actor references session content but fails to identify the specific preference as the answer target.

### RETRIEVAL_MISS (3/8)

The answer session was completely absent from retrieved memories. Queries like "dinner with homegrown ingredients" and "coffee creamer recipe" failed to retrieve the sessions where the user discussed their garden/recipe. These are FTS vocabulary-gap failures — the query terms don't overlap with the session content terms.

## Conclusions

### Multi-session: prompt refinement IS the right lever

9/10 failures are ACTOR_MISS. The retrieval system delivered the evidence; the actor failed to use it. The PR #91 structural analysis (single primary task, defined output shape) remains relevant, but the specific refinement should target:

1. **Recognition of indirect references** — The actor undercounts because items are described with different words than the question uses. Instruction should encourage the actor to consider indirect matches.
2. **SESSION_USER_CONFUSION** — One unambiguous fix: "All sessions are about you. Different session IDs are different conversations, not different people." This addresses failure #2 directly and may help other cases where the actor hesitates about session attribution.
3. **Completion check** — Instead of "state the final count," instruct "after listing items, re-scan the sessions to check if you missed any." This targets the Type A undercount-by-1 pattern.

The 1 RETRIEVAL_MISS (#4, doctors) is not addressable by prompt changes.

### Preference: mixed — both prompt and retrieval needed

5/8 are ACTOR_MISS (prompt-addressable), 3/8 are RETRIEVAL_MISS (not prompt-addressable). The PR #91 preference prompt refinement should proceed for the ACTOR_MISS cases. The RETRIEVAL_MISS cases need FTS vocabulary coverage improvements (description text in FTS index, or synonym expansion).

### Actionable next steps

1. **Update PR #91** with revised counting_enumerate.md targeting ACTOR_MISS patterns (indirect references, session-user clarity, completion re-check). The original "cognitive overload" diagnosis was wrong; the new diagnosis is "recognition of indirect references + premature conclusion."
2. **Update PR #91** with revised preference.md targeting ACTOR_MISS pattern (specific preference extraction vs generic recommendation). The 3 RETRIEVAL_MISS cases are out of scope.
3. **File retrieval issue** for the 1 multi-session RETRIEVAL_MISS (#4) and 3 preference RETRIEVAL_MISS cases (#3, #4, #7). These need FTS coverage work (backlog item #10: MemoryHit carries description, or item #13: per-session summaries).
