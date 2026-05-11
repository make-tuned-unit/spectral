# Synthesis Prompt Revision Proposal

**Date:** 2026-05-11
**Branch:** `feat/bench-synthesis-prompt-revisions`
**Scope:** Actor prompt, judge prompt, memory-formatting code
**Baseline reference:** 65.8% overall on LongMemEval-S (top-K=20, pre-audit)

---

## 1. Current state (verbatim)

### Actor prompt (`actor.rs:46-74`)

Single user-turn prompt (no system message). Template:

```
You are answering a question based on a long conversation history.
Today's date is {question_date}.
Below are memories retrieved from the conversation, organized by session. Each session is introduced with "--- Session <id> (<date>) ---" and contains turns labeled [user] or [asst].

Instructions:
1. For counting, listing, or ordering questions: the answer may be distributed across multiple sessions. Scan EVERY session header below, extract relevant items from each, then count or list all of them. Do not stop after the first or second session.
2. For questions about your current or most recent X: identify the most recent session mentioning X and treat that value as definitive, even if older sessions mention different values.
3. When information appears partial across sessions, attempt synthesis from the available evidence rather than saying "I don't know." Only respond with "I don't know" when no session contains relevant content for the question.
4. When the question asks whether something happened (e.g., "did I mention X?"), and X is not present in any session, state that clearly and note what IS present (e.g., "You mentioned Y but not X").
5. When multiple distinct entities or locations match the question (e.g., multiple stores, multiple vehicles), do not pick the first one mentioned. Identify which entity the question is specifically asking about and verify against the most relevant sessions before answering.
6. For questions requiring arithmetic across sessions (computing differences, sums, ages, totals): identify the relevant numerical values from each session and perform the calculation explicitly. Show the values used and the result.

Memories:
{memories_text}

Question: {question}

Answer:
```

### Judge prompt (`judge.rs:26-53`)

Single user-turn prompt. Category-specific rubrics:

```
You are grading a question-answering system's response.

Question: {question}
Ground truth: {ground_truth}
System answer: {predicted}

Rubric: {rubric}

Respond with JSON only: {"correct": true|false, "reasoning": "..."}
```

Where `rubric` is one of:
- **KnowledgeUpdate:** "The question tests whether the system recognizes updated information. The answer is correct if it reflects the MOST RECENT information, not older versions."
- **TemporalReasoning:** "The question requires reasoning about when events happened. The answer is correct if the temporal aspect is accurately captured."
- **MultiSession:** "The question requires synthesizing information across multiple conversation sessions. The answer is correct if it accurately combines relevant facts from different sessions."
- **All others** (SingleSessionUser, SingleSessionAssistant, SingleSessionPreference): "An answer is correct if it conveys the same factual information as the ground truth, even if worded differently. Synonyms and paraphrasing are acceptable."

### Memory formatting (`retrieval.rs`)

Two formatting paths:

**Flat format** (`format_hit`, `retrieval.rs:191-200`) — used by `topk_fts`, `tact`, `graph`:
```
[{date}] [{wing}/{hall}] {key}: {content}
```
Example: `[2023-06-15] [?/?] s1:turn:0:user: I like pizza`

**Session-grouped format** (`format_hits_grouped`, `retrieval.rs:124-186`) — used by `cascade`:
```
--- Session {ep_id} ({date}) ---
[user] {content}
[asst] {content}  (skipped if <40 chars)
```

---

## 2. Proposed changes

### Change A: Add preference-priority instruction to actor prompt

**Target category:** `single-session-preference` (30 questions), `single-session-user` (70 questions)

**Failure mode:** When the user explicitly states a preference ("I prefer X"), the actor sometimes infers a different preference from context or picks a more recent but implicit mention. The current instructions have no guidance on stated vs. inferred preferences.

**Proposed edit** — add instruction 7 to the actor prompt:

```
7. For questions about preferences, favorites, or personal choices: prioritize the user's
   explicit statements ("I prefer X", "my favorite is Y", "I chose Z") over inferred or
   contextual information. If the user stated a preference directly, that is the answer.
```

**Expected impact:** +1-3pp on single-session-preference, +0.5-1pp on single-session-user. These categories contain many "what is my favorite X?" questions where the answer is an explicit user statement.

### Change B: Add answer-format instruction to actor prompt

**Target category:** All categories, especially `multi-session` (133 questions)

**Failure mode:** The actor sometimes produces verbose, hedged answers ("Based on what I can see, it appears that...") that contain the right information but are hard for the judge to evaluate. The judge sometimes marks correct-but-verbose answers as wrong because the factual content is buried in qualifications.

**Proposed edit** — add instruction 8 to the actor prompt:

```
8. Be direct and concise. State the answer clearly without hedging or qualifying
   (e.g., "Paris" not "Based on the conversation history, it appears that the answer
   might be Paris"). If the answer is a number, state just the number. If the answer
   is a name, state just the name.
```

**Expected impact:** +0.5-1pp overall. Most impactful on factual single-session categories where the answer is a single entity, but helps across all categories by reducing judge parse errors.

### Change C: Add temporal calculation instruction to actor prompt

**Target category:** `temporal-reasoning` (133 questions)

**Failure mode:** The actor already has instruction 6 for arithmetic, but temporal questions often require date-difference calculations ("how many months between X and Y?", "how old was I when..."). The instruction doesn't explicitly mention using session dates for temporal reasoning.

**Proposed edit** — replace instruction 6 with a more specific version:

```
6. For questions requiring dates, durations, or temporal calculations: use the session
   dates shown in "--- Session <id> (<date>) ---" headers as the time reference for
   events in that session. When computing time differences (days, weeks, months between
   events), identify the relevant session dates and calculate explicitly. Today's date
   is provided above — use it for "how long ago" or "how old" calculations.
```

**Expected impact:** +0.5-1pp on temporal-reasoning. The current instruction 6 covers arithmetic but doesn't tell the actor to use session header dates for temporal calculations. This is especially relevant because `created_at` is now present in session headers (fixed in PR #38 audit fixes).

### Change D: Add single-session-specific rubrics to judge

**Target category:** `single-session-user` (70 questions), `single-session-assistant` (56 questions), `single-session-preference` (30 questions)

**Failure mode:** These three categories currently get the generic "factual equivalence" rubric. But they have different evaluation semantics:
- `single-session-preference`: answer should match the user's stated preference exactly
- `single-session-user`/`single-session-assistant`: answer should come from within a single session's context

The generic rubric is too permissive — it accepts paraphrases that change the actual preference value, and too strict — it rejects correct answers that use different but equivalent phrasing for non-preference facts.

**Proposed edit** — add rubrics in `judge_prompt`:

```rust
Category::SingleSessionPreference => {
    "The question asks about the user's stated preference, favorite, or personal choice. \
     The answer is correct if it matches what the user explicitly stated as their preference, \
     even if worded differently. Synonyms are acceptable but the core preference must match."
}
Category::SingleSessionUser | Category::SingleSessionAssistant => {
    "The question asks about information from a specific conversation. \
     The answer is correct if it conveys the same factual information as the ground truth, \
     even if worded differently. Synonyms and paraphrasing are acceptable."
}
```

**Expected impact:** +0.5pp on single-session-preference (sharper rubric reduces false negatives on exact-match preferences). Neutral on single-session-user/assistant (functionally equivalent to the current generic rubric, but explicitly documented for clarity and future tuning).

---

## 3. What NOT to change

### Not modifying memory formatting code

The session-grouped format (`format_hits_grouped`) and flat format (`format_hit`) are already in good shape post-audit. `created_at` is included in both paths. Changing the format would conflate prompt changes with retrieval presentation changes, making it impossible to attribute any accuracy movement to prompts alone.

### Not adding a system message

The current approach uses a single user message. Splitting into system+user would change the LLM's behavior in ways that are hard to predict and control. The single-message approach is simpler and sufficient.

### Not modifying both actor and judge simultaneously on the same axis

Each actor change is paired with a non-overlapping judge change. For example, the actor gets preference-priority instructions (Change A) and the judge gets a preference-specific rubric (Change D). These target the same category but through independent mechanisms — the actor is told to prioritize explicit preferences, the judge is told to evaluate preference accuracy. If only one of them moves the number, we can bisect by reverting one.

### Not touching retrieval (K, ranking, cascade, dedup)

Explicitly out of scope. These changes are prompt-only.

### Not adding new categories, metrics, or CLI flags

The bench harness structure is frozen for this PR.

---

## 4. Expected impact summary

| Change | Target categories | Expected lift | Confidence |
|--------|-------------------|---------------|------------|
| A (preference-priority) | single-session-preference, single-session-user | +1-3pp on target, +0.2-0.5pp overall | Medium |
| B (concise answers) | All | +0.5-1pp overall | Medium-high |
| C (temporal calculation) | temporal-reasoning | +0.5-1pp on target, +0.1-0.3pp overall | Medium |
| D (judge rubrics) | single-session-* | +0.5pp on target, +0.1-0.2pp overall | Low-medium |

**Combined expected lift:** +1-2pp overall on LongMemEval-S.

The highest-confidence change is B (concise answers) — verbose actor outputs are a known source of false negatives in LLM-as-judge evaluations. The lowest-confidence change is D (judge rubrics) — the current generic rubric is already reasonably calibrated for single-session questions.

---

## 5. Review outcome

**Approved:** A (preference-priority), B (concise answers), C-modified (as new instruction 9, instruction 6 unchanged).

**Deferred to follow-up PR:** D (judge rubrics). Reasoning: lowest-confidence/lowest-impact change; shipping with A+B+C muddies attribution since we won't re-run $40 benches to bisect. Cleaner to evaluate D as a standalone experiment after the next bench checkpoint.

**Modification to C:** Original proposal replaced instruction 6. Reviewer correctly noted this drops coverage on non-temporal cross-session arithmetic (counting, summing, totals). Fix: keep instruction 6 intact, add temporal guidance as new instruction 9.
