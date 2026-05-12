# Actor Prompt Refinement Proposal (v2)

**Date**: 2026-05-12
**Branch**: `feat/refine-actor-prompts-v1`
**Status**: Implementing approved refinements.

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

## Structural Comparison (general knowledge, retained)

High-accuracy prompts (temporal 93%, assistant_recall 86%) share three properties:

1. **Single primary task.** Instruction 1 names one thing to do.
2. **Defined output shape.** The answer format is specified.
3. **Explicit null-result handling.** The actor knows what to do when evidence is absent.

These are valid general principles for prompt design. However, the v1 proposal incorrectly diagnosed counting_enumerate.md's failure mode as "cognitive overload from too many subtasks." The investigation in PR #93 refuted this.

## Corrected Diagnosis

### counting_enumerate.md (63.3%, 9 ACTOR_MISS out of 10 failures)

**Not cognitive overload.** The actor completes enumeration and states confident final counts. It doesn't abandon mid-task or run out of generation budget.

**Actual failure mode: embedded-reference-in-different-primary-context.** Validated on the weddings case (PR #93 walkthrough): all 3 answer sessions were retrieved. Session answer_e7b0637e_2 contains "My friend Emily finally got to tie the knot with her partner Sarah" and session answer_e7b0637e_3 contains "the bride, Jen, looked stunning." Both are explicit, direct references. The actor missed them because all three sessions are primarily about the user planning their own wedding. The attended-wedding references are subordinate clauses within wedding-planning conversations.

**Secondary failure: session-user confusion.** Failure #2 (camping): actor says "the camping references belong to other users" because different session IDs look like different user IDs.

### preference.md (66.7%, 5 ACTOR_MISS + 3 RETRIEVAL_MISS out of 8 failures)

**ACTOR_MISS (5/8):** Answer session retrieved but actor generates generic recommendations instead of grounding in the user's specific stated preferences. The "implicit signals from past activities" clause is too broad.

**RETRIEVAL_MISS (3/8):** Answer sessions absent from retrieval due to FTS vocabulary gap ("homegrown ingredients" vs "basil and mint"; "battery life" vs "power bank"; "coffee creamer recipe" vs "flavored creamer with almond milk"). Not addressable by prompt changes. Deferred to backlog item #8 (compiled-truth boost) once Permagent's Librarian populates descriptions.

## Changes to counting_enumerate.md

Two instructions added to the existing prompt. Current task structure preserved (scan-list-dedup-count); recognition criteria refined.

**Added instruction 2** (embedded-reference recognition):
> "Items may appear as passing mentions within conversations about other topics. A session about wedding planning might mention weddings you attended. Scan for the counted item even when the session's primary topic is different."

Targets the dominant failure mode directly. Tells the actor what to look for differently rather than asking it to look again.

**Added instruction 3** (session-user clarity):
> "All retrieved memories are about you across multiple sessions. Different session IDs do not mean different users."

Targets failure #2 (camping) and any similar cross-session attribution confusion.

**Not changed:** instruction 1 (scan-list-dedup-count), instruction 4 (synthesis), instruction 5 (concise). The v1 proposal's count-first reordering and dropped attribution are not applied -- those targeted cognitive overload, which is not the failure mode.

## Changes to preference.md

**Instruction 1** revised: "stated preferences" replaces "explicit statements OR implicit signals." The "implicit signals" escape hatch is removed. "Prefer these over inferred preferences" keeps inference possible but makes explicit statements primary.

**Instruction 2** revised: "If the user mentioned a product, ingredient, or experience by name, reference it directly." Concrete grounding targeting the failure mode where the actor references topic area but misses the named entity.

**Instruction 3** added: "Do not give generic advice. Every recommendation should trace back to something the user said." Direct negative instruction targeting the most common ACTOR_MISS pattern.

**Instruction 4** revised: "say so rather than guessing" replaces the synthesis directive. For preference questions, guessing IS the failure mode.

## What this PR does NOT address

- 1 multi-session RETRIEVAL_MISS (doctors, #4): zero answer sessions retrieved. Needs FTS coverage work.
- 3 preference RETRIEVAL_MISS cases: FTS vocabulary-gap failures. Deferred to item #8 (compiled-truth boost) once descriptions are populated.
- factual_direct.md (68.4%) and factual_current_state.md (50.0%): different failure modes, smaller samples. Separate investigation if prioritized.

## Test Plan

Targeted bench runs only ($3.20 total):
- **Multi-session** (n=20, ~$1.60): baseline 50% (10/20), target 60%+ (12+/20)
- **Single-session-preference** (n=20, ~$1.60): baseline 60% (12/20), target 70%+ (14+/20)

No full bench until targeted results confirm the intervention works.

## Safety Floor Verification

Both revised prompts preserve:
- Today's date: `{question_date}` present
- Memory format: "organized by session" / session header description present
- "Don't know" fallback: counting has synthesis instruction; preference has "say so rather than guessing"
- Concise answers: final instruction in both
