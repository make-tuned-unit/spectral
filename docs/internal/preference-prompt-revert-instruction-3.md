# Preference Prompt Fix: Revert Instruction 3, Add Session-User Clarity

**Date**: 2026-05-12
**Branch**: `fix/preference-prompt-revert-instruction-3`
**Triggered by**: Post-#91 bench regression: single-session-preference 60% -> 35% (-25pp)

---

## Root Cause

Two clear patterns in the 5 regressed cases:

### Pattern 1: Instruction 3 is over-restrictive (4 of 5 regressions)

Instruction 3 ("Do not give generic advice. Every recommendation should trace back to something the user said.") converts useful inference into refusal. The actor reads "trace back to something the user said" as "refuse unless there's explicit per-question grounding."

**Hotel in Miami (0edc2aef):** Pre-#91 actor noted user discussed Seattle hotel preferences (great views, hot tubs, luxury), applied those as transferable preferences to Miami. Correct. Post-#91 actor: "the user has never mentioned Miami... I don't have enough information." Refuses.

**Cultural events (35a27287):** Pre-#91 actor lists user's known cultural interests (Spanish, French language practice), recommends events. Correct. Post-#91 actor: "I don't have access to your location, can't recommend specific events." Refuses.

**Evening activities (195a1a1b):** Pre-#91 actor mentions "before winding down" aligning with 9:30pm constraint and avoids phone suggestions. Post-#91 actor provides activities but loses the relaxation/no-phone constraints — the "trace back" framing made it overshoot on some constraints while missing others.

**Furniture rearranging (57f827a0):** Pre-#91 actor mentions mid-century modern and dresser, provides actionable tips. Post-#91 actor says "No relevant preferences about your bedroom layout" before belatedly mentioning mid-century modern, provides weaker response.

### Pattern 2: Session-user confusion (1 of 5 regressions)

**Colleagues (54026fce):** Post-#91 actor literally said "wait, that was a different user in Session answer_f7b22c66... Looking specifically at your sessions, I don't see explicit statements from you about working remotely." This is the camping-case failure mode from counting_enumerate — actor treats different session IDs as different users.

counting_enumerate.md already has the fix: "All retrieved memories are about you across multiple sessions. Different session IDs do not mean different users." preference.md does not.

## Proposed Changes

### preference.md diff

**Before (current, post-#91):**
```
1. Find the user's stated preferences relevant to this question -- look for
   explicit statements about what they like, dislike, own, or have tried.
   Prefer these over inferred preferences from general activities.
2. Base your recommendation on specific details from the sessions. If the user
   mentioned a product, ingredient, or experience by name, reference it directly.
3. Do not give generic advice. Every recommendation should trace back to
   something the user said.
4. When no relevant preferences are found, say so rather than guessing.
   Answer concisely.
```

**After (proposed):**
```
1. Find the user's stated preferences relevant to this question -- look for
   explicit statements about what they like, dislike, own, or have tried.
   Prefer these over inferred preferences from general activities.
2. Base your recommendation on specific details from the sessions. If the user
   mentioned a product, ingredient, or experience by name, reference it directly.
3. All retrieved memories are about you across multiple sessions. Different
   session IDs do not mean different users.
4. When no relevant preferences are found, say so rather than guessing.
   Answer concisely.
```

Changes:
- **DROP** instruction 3 ("Do not give generic advice..."). This is the destructive instruction.
- **KEEP** instructions 1, 2, 4 (stated > inferred, reference by name, say so rather than guessing). These appear fine.
- **ADD** session-user clarity instruction (verbatim from counting_enumerate.md) as new instruction 3.

## Success-Case Check

For 3 of the 12 pre-#91-correct preference cases, I verify the proposed prompt would not regress them.

### Case 1: Hotel in Miami (0edc2aef) — strong inference

**Pre-#91 (correct):** "Based on the conversation history, the user is planning a trip to Seattle (not Miami), and has shown preferences for: Hotels with great views (specifically Space Needle views), Hot tubs on balconies, Complimentary room service breakfast, Romantic/luxury amenities. They ultimately chose The E..."

**Proposed prompt effect:** Instruction 1 ("stated preferences... Prefer these over inferred") still guides toward explicit preferences. Without instruction 3's "trace back" straitjacket, the actor can transfer Seattle hotel preferences to Miami — which is the correct behavior. The pre-#91 prompt did exactly this. **No regression expected; regression fix.**

### Case 2: Photography accessories (06878be2) — explicit preferences

**Pre-#91 (correct):** "Based on the conversations, I can see that the user is a Sony camera user (Session answer_555dfb94) who uses a Godox V1 flash. Here are some tailored accessory suggestions: Godox X2T-S Trigger, Diffusers/Softboxes..."

**Proposed prompt effect:** This case has strong explicit preferences (Sony camera, Godox V1 flash). Instruction 1 and 2 handle it perfectly — they find named products and reference them directly. Instruction 3 (session-user clarity) is neutral here (single session). Instruction 4 is irrelevant (preferences exist). **No regression expected; stable.**

### Case 3: Evening activities (195a1a1b) — generic-starter "Any tips"

**Pre-#91 (correct):** Actor mentions yoga, walks, and "before winding down" (aligning with 9:30pm constraint), avoids phone/screen suggestions.

**Proposed prompt effect:** Without instruction 3's "trace back" constraint, the actor returns to synthesizing across the user's stated habits and constraints (9:30pm bedtime, preference for relaxation) as it did pre-#91. Instructions 1-2 still ensure grounding in specific details. **No regression expected; regression fix.**

## Bench Plan

- **Run**: single-session-preference only (n=20, ~$1.60)
- **Target**: restore to 60% minimum, ideally 65%+ if session-user clarity helps case 54026fce
- **Do NOT re-bench**: multi-session (its +5pp lift is already attributed and preference.md changes don't affect it)
