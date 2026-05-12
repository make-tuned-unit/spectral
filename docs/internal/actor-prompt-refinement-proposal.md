# Actor Prompt Refinement Proposal

**Date**: 2026-05-12
**Branch**: `feat/refine-actor-prompts-v1`
**Status**: Proposal. Implementation after approval.

---

## Evidence: Accuracy by Prompt Template (post-PR #86 + #90 bench)

| Template | Accuracy | n |
|----------|----------|---|
| temporal.md | 92.9% | 28 |
| assistant_recall.md | 85.7% | 14 |
| counting_current_state.md | 80.0% | 5 |
| generic_fallback.md | 80.0% | 5 |
| factual_direct.md | 68.4% | 19 |
| **preference.md** | **66.7%** | **18** |
| **counting_enumerate.md** | **63.3%** | **30** |
| factual_current_state.md | 50.0% | 2 |

Two templates have both low accuracy AND high sample count: `counting_enumerate.md` (63.3%, n=30) and `preference.md` (66.7%, n=18). These are the refinement targets.

## Structural Comparison

### What high-accuracy prompts have in common

**temporal.md (93%)** and **assistant_recall.md (86%)** share three properties:

1. **Single primary task.** Instruction 1 names one thing to do:
   - temporal: "identify session dates, then perform the calculation"
   - assistant_recall: "find the relevant session and quote/paraphrase"

2. **Defined output shape.** Instruction 4 says exactly what the answer looks like:
   - temporal: "State the date(s) or duration"
   - assistant_recall: "Answer concisely" (after instruction 1 already said "quote or paraphrase")

3. **Explicit null-result handling.** Both tell the actor what to do when evidence is absent:
   - temporal: "attempt synthesis... Only respond 'I don't know' when no session contains relevant content"
   - assistant_recall: "state clearly what IS present" / "You mentioned Y but not X"

### What counting_enumerate.md gets wrong

**Instruction 1 packs four subtasks into one sentence:**
> "Scan EVERY session header below. For each match, list the item explicitly with its source session. Deduplicate before counting. State the final count last."

This is scan + list-with-attribution + deduplicate + count = 4 operations. The actor starts enumerating, gets bogged down in attribution, and either stops mid-task or produces an undercount.

**Evidence from 11 failures (all counting):**
- 8/11 find most items but miss 1-2 (undercount by 1 consistently)
- 2/11 produce false negatives ("no information about X" when X is present)
- 1/11 attributes user's own memories to "other users"

The common thread: the actor allocates generation budget to per-item attribution (session IDs, dates, context) and runs out of attention before completing the scan. Listing with source is a nice-to-have that actively hurts the primary task (getting the count right).

**Instruction 2 ("Do not stop after the first or second session")** is a symptom-patch for instruction 1's overload. If the primary task were lighter, the actor wouldn't need to be told not to stop.

### What preference.md gets wrong

**Instruction 1 is two tasks with a permissive scope:**
> "Identify the user's relevant preferences from the conversation (explicit statements OR implicit signals from past activities). Tailor your suggestion to those preferences."

The "implicit signals from past activities" clause is too broad. The actor latches onto any activity mentioned in any session and generates a recommendation from general knowledge rather than the specific preference stated in the conversation.

**Evidence from 8 failures:**
- 4/8 produce generic recommendations ignoring specific user statements (guitar advice, battery tips, cookie ingredients, dinner ingredients)
- 2/8 pull wrong context entirely (cocktails when user wants AI papers, Ozark when user wants comedy)
- 1/8 answers "I don't know" despite relevant context being present
- 1/8 references adjacent context but misses the specific preference

The pattern: the actor generates a plausible recommendation for the topic rather than grounding in what the user specifically said. Instruction 2 says "reference what the user has said" but this is too weak against instruction 1's "implicit signals" escape hatch.

## Revised counting_enumerate.md

```
You are answering a question based on a long conversation history.
Today's date is {question_date}.
Below are memories retrieved from the conversation, organized by session. Each session is introduced with "--- Session <id> (<date>) ---" and contains turns labeled [user] or [asst].

Instructions:
1. State the count first, then briefly list the items. Scan every session below before answering — the items are spread across multiple sessions.
2. If two sessions mention the same item, count it once.
3. If no items match, answer "0" — do not abandon the task or say "I don't know."
4. Answer concisely. State the count and the items.

Memories:
{memories_text}

Question: {question}

Answer:
```

**Changes:**
- Instruction 1: single directive "state the count first, list briefly second." Scan-all is a subordinate clause, not a multi-step process. Dropped "list each item explicitly with its source session" — the per-item attribution that consumed generation budget.
- Instruction 2: dedup is now its own short sentence instead of buried in instruction 1.
- Instruction 3: explicit zero-result fallback. Addresses the "no information about X" false-negative failures. The phrasing "do not abandon the task" directly targets the mid-enumeration dropout pattern.
- Instruction 4: unchanged safety floor.
- Length: 591 chars (vs 621 original). Fewer words doing more work.

## Revised preference.md

```
You are answering a question based on a long conversation history.
Today's date is {question_date}.
Below are memories retrieved from the conversation, organized by session. Each session is introduced with "--- Session <id> (<date>) ---" and contains turns labeled [user] or [asst].

Instructions:
1. Find the user's stated preferences relevant to this question — look for explicit statements about what they like, dislike, own, or have tried. Prefer these over inferred preferences from general activities.
2. Base your recommendation on specific details from the sessions. If the user mentioned a product, ingredient, or experience by name, reference it directly.
3. Do not give generic advice. Every recommendation should trace back to something the user said.
4. When no relevant preferences are found, say so rather than guessing. Answer concisely.

Memories:
{memories_text}

Question: {question}

Answer:
```

**Changes:**
- Instruction 1: "stated preferences" replaces "explicit statements OR implicit signals." The "implicit signals" escape hatch that caused generic recommendations is removed. "Prefer these over inferred preferences" keeps the door open for inference but makes explicit statements primary.
- Instruction 2: "mentioned a product, ingredient, or experience by name" — concrete grounding instruction targeting the specific failure mode (actor references topic but misses the named entity the user discussed).
- Instruction 3: new. "Do not give generic advice" — direct negative instruction targeting the most common failure. "Trace back to something the user said" reinforces grounding.
- Instruction 4: "say so rather than guessing" replaces the synthesis directive. For preference questions, guessing IS the failure mode. Better to admit absence than fabricate.
- Length: 719 chars (vs 622 original). Slightly longer due to the anti-generic instruction, but each sentence addresses a specific failure pattern.

## Test Plan

No new code tests. The bench is the test.

- **Multi-session category** (n=20, ~$1.60): primary verification for counting_enumerate.md. Baseline: 50% (10/20). Target: 60%+ (12+/20). The 10 failures were all counting — even converting 2-3 is a meaningful lift.
- **Single-session-preference category** (n=20, ~$1.60): primary verification for preference.md. Baseline: 60% (12/20). Target: 70%+ (14+/20).
- Both categories run as part of the standard bench sweep; no special configuration needed.

## Safety Floor Verification

Both revised prompts preserve:
- Today's date: `{question_date}` present
- Memory format: "organized by session" / "--- Session <id> (<date>) ---" header description present
- "Don't know" fallback: counting has "answer '0'"; preference has "say so rather than guessing"
- Concise answers: instruction 4 in both
