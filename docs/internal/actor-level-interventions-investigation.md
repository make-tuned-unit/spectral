# Actor-Level Interventions Investigation

**Date**: 2026-05-14
**Branch**: `investigate/actor-level-interventions`
**Status**: Investigation complete. Recommendation: **sequenced approach** -- Candidate A (quote-anchored per-session extraction) as cheapest viable, with Candidate B (two-call extract-then-count) as escalation.

---

## Section 1 -- Inventory of the Current Actor Path

### Where the actor prompt is constructed

The bench actor lives in `crates/spectral-bench-accuracy/src/actor.rs`. The `AnthropicActor` implementation:

1. Receives `question`, `question_date`, `memories: &[String]`, and `shape: QuestionType` (added in PR #86 via the shape-routed actors work).
2. Selects a prompt template via `shape.prompt_content()` which calls `include_str!` on one of 8 markdown files in `crates/spectral-bench-accuracy/src/prompts/`.
3. Performs string replacement: `{question_date}`, `{memories_text}`, `{question}`.
4. Makes a single Anthropic Messages API call (model: `claude-sonnet-4-6`, max_tokens: 4096).
5. Extracts the answer from `content[0].text`.

**Single call.** No multi-call, no chaining, no verification pass. One prompt in, one answer out.

### What the actor receives

For cascade-routed questions (all non-Temporal), the actor receives **session-grouped formatted memories** from `format_hits_grouped()` at `retrieval.rs:242`. The format:

```
--- Session answer_e7b0637e_1 (2023-05-20) ---
[user] I've been to a few weddings recently...
[asst] That sounds wonderful! Wedding planning can be exciting...
[user] My cousin Rachel's wedding at the vineyard was just perfect...
--- Session answer_e7b0637e_2 (2023-05-22) ---
[user] I just got back from my college roommate's wedding...
[user] My friend Emily finally got to tie the knot with her partner Sarah...
```

Sessions are ordered chronologically. Turns within each session are ordered by key (which encodes turn order). Short assistant filler (<40 chars) is stripped. Session headers include episode ID and date.

For counting questions, K=60 memories from the cascade pipeline, capped at 3 per episode. For a typical 20-session question, this means 20 session headers with up to 3 turns each -- roughly 60 text blocks.

### The actor prompt template (counting_enumerate.md)

```
You are answering a question based on a long conversation history.
Today's date is {question_date}.
Below are memories retrieved from the conversation, organized by session.
Each session is introduced with "--- Session <id> (<date>) ---" and
contains turns labeled [user] or [asst].

Instructions:
1. Scan EVERY session header below. For each match, list the item
   explicitly with its source session. Deduplicate before counting.
   State the final count last.
2. Before counting, quote every mention of the item being counted from
   each session. Place quotes in <quotes> tags, one per session that
   contains a mention. Then count the unique items from your quotes.
3. Items may appear as passing mentions within conversations about other
   topics. A session about wedding planning might mention weddings you
   attended. Scan for the counted item even when the session's primary
   topic is different.
4. All retrieved memories are about you across multiple sessions.
   Different session IDs do not mean different users.
5. When information appears partial across sessions, attempt synthesis
   from the available evidence rather than saying "I don't know."
6. Answer concisely. State the count and the items.
```

This prompt already includes the quote-first instruction (instruction 2, added in PR #97) and the embedded-reference instruction (instruction 3, added in PR #93). Both were explicitly designed to address the GENUINE_MISS failure mode. **Neither worked.**

### How the answer is extracted

The full text output from `content[0].text` becomes `predicted`. This includes any `<thinking>`, `<quotes>`, and final answer text the model generates. The judge receives the entire output.

### Permagent coordination: bench vs production actor

The bench actor is **entirely separate** from any production actor path. Permagent has its own actor/synthesis logic. The bench actor's `AnthropicActor` is a standalone implementation in `spectral-bench-accuracy` that calls the Anthropic API directly. Any intervention designed here would need to be replicated in Permagent's actor if it proves effective. The shared surface is the Spectral retrieval API (`Brain::recall_cascade_with_pipeline`) and the memory format (`format_hits_grouped`), not the actor prompt or call structure.

---

## Section 2 -- Failure Characterization

### Source data

The GENUINE_MISS and AMBIGUOUS cases from the post-PR-#98 failure classification (`docs/internal/multi-session-failure-classification-2026-05-12.md`), with the 2026-05-13 correction reclassifying cases #8 and #9 to AMBIGUOUS.

### Sub-problem taxonomy

The failures decompose into **three distinct sub-problems**, not one:

#### Sub-problem 1: Within-turn partial extraction

**Case #3 (bike expenses)**: The actor quoted "$40 bike lights" from session `answer_2880eb6c_2` but missed "$25 chain replacement" from the SAME user turn:

> "The mechanic told me I needed to replace the chain, which I did, and it cost me $25. While I was there, I also got a new set of bike lights installed, which were $40."

The actor extracted one price from a sentence pair and skipped the other. This is not a cross-session problem. It is not a topic-filtering problem. It is **within-turn attention drop**: the model reads a sentence containing two costs, registers one, moves on.

Additionally, the actor missed "$120 Bell Zephyr helmet" from `answer_2880eb6c_1`, where it mentioned "Bell Zephyr helmet" in reasoning but stated "no specific costs are given" -- directly contradicting the source text ("I bought my Bell Zephyr helmet for $120"). This is a **factual hallucination about the source**: the model acknowledges the entity but fabricates the absence of its associated data point.

**Distinctive characteristics**: The evidence is in the same turn or sentence pair as a found item. The actor reads the text, extracts a subset of facts, and skips the rest. The failure is at sentence-level granularity, not session-level.

#### Sub-problem 2: Embedded reference in different primary context (cross-session topic filtering)

**Case #9 (weddings, AMBIGUOUS)**: The actor found Rachel's wedding from `answer_e7b0637e_1` and a "sister's wedding" from a non-answer session. It missed Emily+Sarah's wedding (`answer_e7b0637e_2`: "My friend Emily finally got to tie the knot with her partner Sarah") and Jen+Tom's wedding (`answer_e7b0637e_3`: "the bride, Jen, looked stunning in her bohemian-inspired dress").

All three answer sessions are primarily about the user **planning their own wedding**. The attended-wedding references are subordinate clauses providing context/inspiration. The actor tracked the primary topic (wedding planning) and didn't register subordinate mentions as counting toward "weddings I attended."

**Case #8 (tanks, AMBIGUOUS)**: The actor found the 20-gallon Amazonia tank and the 1-gallon friend's kid tank. It missed the 5-gallon betta tank with Finley from `answer_c65042d7_2`, where the session's primary topic is high nitrite levels in the community tank. The betta tank is introduced as background context: "I have a 5-gallon tank with a solitary betta fish named Finley."

**Important caveat**: Cases #8 and #9 were reclassified to AMBIGUOUS in the 2026-05-13 correction. The "3/3 retrieved" claim was based on actor output inference, not retrieval telemetry (`memory_keys` was empty). For both cases, the missed sessions have **no evidence of retrieval** in actor output. These may be partial RETRIEVAL_MISS failures, not actor failures at all. If the sessions weren't retrieved, no actor intervention can help.

**Distinctive characteristics**: The evidence is in a different session whose primary topic doesn't match the question. The actor's attention follows session topics, skipping subordinate mentions. This is fundamentally about how the model allocates attention across a large context with competing topics.

#### Sub-problem 3: Incomplete enumeration with unknown cause

**Case #7 (movie festivals)**: The actor found 3 of 4 festivals (Austin Film Festival, AFI Fest, Portland Film Festival). All 3 answer sessions were confirmed retrieved. The 4th festival's identity is unclear from the analysis -- it may be in a non-answer session, or the GT may be counting an event that the answer sessions don't clearly support as a separate festival.

**Distinctive characteristics**: This case doesn't fit cleanly into sub-problems 1 or 2. The actor found 3 festivals across 3 sessions; the 4th is either a session-level miss (sub-problem 2) or a GT accuracy question. Without knowing which festival is the 4th, the failure mechanism can't be precisely characterized.

### Summary of sub-problems

| Sub-problem | Cases | Mechanism | Granularity |
|-------------|-------|-----------|-------------|
| Within-turn partial extraction | #3 (bike expenses) | Extracts subset of facts from a sentence | Sentence-level |
| Cross-session topic filtering | #8 (tanks), #9 (weddings) | Skips subordinate references in off-topic sessions | Session-level |
| Incomplete enumeration (unclear) | #7 (festivals) | Missing 4th item, mechanism unknown | Unknown |

**Critical observation**: Sub-problems 1 and 2 require different interventions. A per-session extraction approach addresses sub-problem 2 (isolating sessions removes cross-session topic interference) but does NOT address sub-problem 1 (within-turn extraction failure persists even in a single-session context). The existing quote-first instruction (PR #97) was designed to address both but failed on both.

---

## Section 3 -- Candidate Interventions

### Candidate A -- Quote-anchored per-session extraction

**Mechanism**: Instead of one actor call over all sessions, restructure the single call to force explicit per-session extraction. The prompt would require the actor to process each session header sequentially and, for each, output a structured block:

```
<session id="answer_e7b0637e_2">
Relevant quotes: "My friend Emily finally got to tie the knot with her partner Sarah"
Items found: Emily+Sarah's wedding
</session>
```

After processing all sessions, the actor counts unique items from the per-session blocks.

This differs from the current quote-first instruction (PR #97) in structure: the current prompt says "quote every mention" but lets the model quote globally. This candidate forces the model to iterate session-by-session with an explicit output structure per session.

**Sub-problem 1 (within-turn partial extraction)**: **Partially addresses.** By forcing per-session structured output, the model must engage with each session's content individually rather than skimming. However, the within-turn miss in case #3 involves two facts in the same sentence pair -- structured per-session output won't force per-sentence scanning. The $25 chain and $40 lights are in the same session, same turn. A per-session structure ensures the actor looks at the session, but doesn't guarantee it extracts both prices from one turn.

**Sub-problem 2 (cross-session topic filtering)**: **Directly addresses.** This is the primary target. When processing `answer_e7b0637e_2` in isolation (a session about wedding planning), the instruction "extract items relevant to [weddings attended]" forces the model to scan the session for wedding-attendance references regardless of the session's primary topic. The cross-session attention competition that causes the model to skip subordinate mentions is eliminated -- each session gets its own extraction pass.

**Sub-problem 3 (festivals)**: **May address** if the 4th festival is in a session that the actor skimmed due to topic filtering.

**Cost**: **Zero additional LLM calls.** This is a prompt restructuring within the single existing call. Output tokens increase (structured per-session blocks are longer than a flat answer), but no additional API calls.

**Risk**: Increased output length may cause the model to truncate or abbreviate later sessions (positional attention decay -- "lost in the middle" effect). For 20+ sessions, the structured output could be 2-3x longer than the current format, potentially exceeding the 4096 max_tokens or causing quality degradation in later session blocks.

### Candidate B -- Two-call extract-then-count (structured pre-pass)

**Mechanism**: Call 1 (extraction): receives all session-grouped memories and the question. Instructed: "For each session, list every [item being counted] mentioned, with a verbatim quote and session ID. Do not count or deduplicate. Your job is exhaustive extraction only." Output: JSON array of `{item, session_id, quote}`.

Call 2 (synthesis): receives the JSON array from Call 1 and the question. Instructed: "Given these extracted items, deduplicate and count. State the final answer." No access to raw memories -- only the extracted candidates.

**Sub-problem 1 (within-turn partial extraction)**: **Better than A.** The extraction call's sole task is "find every mention" with no competing synthesis goal. The model's full attention is on extraction. This is the separation-of-concerns argument from the external research synthesis (Google LangExtract pattern, Multi-Step Reasoning survey). However, it's not guaranteed -- the same model processing the same text may still miss the $25 chain even when extraction is the only task. The within-turn attention drop is a model capability issue, not a task-framing issue.

**Sub-problem 2 (cross-session topic filtering)**: **Directly addresses.** Same mechanism as Candidate A -- the extraction call scans each session for relevant items. Separation from counting removes the possibility that the model prematurely concludes "I found enough" and stops scanning.

**Sub-problem 3 (festivals)**: **May address** if the 4th festival is findable with more careful extraction.

**Cost**: **One additional LLM call per question.** At Sonnet pricing (~$0.04/call), this doubles the actor cost from ~$0.04 to ~$0.08 per question. For a 20-question multi-session bench run, that's $0.80 additional. For a 120-question full run, ~$4.80 additional. Not prohibitive.

**Risk**: The extraction call could produce false positives (items that aren't actually relevant), which would inflate the count. The synthesis call has no access to raw context to verify -- it trusts the extraction. This could REGRESS currently-correct cases where the actor currently gets the count right by being appropriately selective. Mitigation: the synthesis call could include a "verify each extracted item is actually [the counted thing]" instruction.

### Candidate C -- Per-session extraction (N calls)

**Mechanism**: One LLM call per retrieved session: "Given this session about [topic], what does it say relevant to [question]?" Then an aggregation call over all per-session summaries.

**Sub-problem 1 (within-turn partial extraction)**: **Similar to A.** Isolating a single session doesn't guarantee within-turn extraction. The $25 chain and $40 lights are in the same turn; processing that session alone, the model may still extract one and miss the other.

**Sub-problem 2 (cross-session topic filtering)**: **Strongly addresses.** Each session is processed in complete isolation. No cross-session topic competition at all. This is mechanistically the strongest intervention for this sub-problem.

**Sub-problem 3 (festivals)**: **Addresses** if the 4th festival exists in a retrieved session.

**Cost**: **N additional LLM calls per question**, where N is the number of sessions (typically 10-20 for counting questions). At $0.04/call, that's $0.40-$0.80 per question or $8-$16 for a 20-question bench run. **This is expensive.** Full 120-question run: $48-$96 additional.

**Risk**: High latency (serial: N * call_latency; parallel: still N API calls). The aggregation call must handle deduplication across sessions, which reintroduces a synthesis task that could miss cross-session connections. Also, individual session extraction calls have very little context -- a question like "How many weddings did I attend?" sent to a session about wedding planning might extract the user's own wedding rather than attended weddings, because the per-session call lacks the broader context that distinguishes "own wedding" from "attended wedding."

### Candidate D -- Question-type-specific prompting

**Mechanism**: Counting questions get a different prompt than the current `counting_enumerate.md`. Specifically: "Enumerate exhaustively, then count. For each session, list EVERY [item type] mentioned, even if it seems tangential. Err on the side of over-counting -- include items you're uncertain about and flag the uncertainty."

**Sub-problem 1 (within-turn partial extraction)**: **Weakly addresses.** "Enumerate exhaustively" is already the intent of the current prompt (instruction 1: "Scan EVERY session header"). Adding "err on over-counting" might shift the model's threshold for inclusion, but the within-turn miss in case #3 isn't a threshold problem -- the model simply didn't see the $25 chain. You can't over-count what you didn't notice.

**Sub-problem 2 (cross-session topic filtering)**: **Weakly addresses.** The current prompt already has instruction 3 ("Items may appear as passing mentions within conversations about other topics"). Restating this more emphatically is unlikely to change behavior -- PR #93 already added this instruction and it didn't help.

**Sub-problem 3 (festivals)**: **Minimal impact.**

**Cost**: **Zero.** Prompt change only.

**Risk**: "Err on over-counting" directly risks regressing currently-correct cases. If the model includes uncertain items, cases that currently get exact counts may start over-counting. The 3 DEFINITION_DISAGREEMENT cases (#1 clothing, #2 projects, #6 citrus) already show over-counting issues -- this instruction would make them worse.

### Candidate E -- Re-read / verification pass

**Mechanism**: After the actor produces an answer, a second call receives the answer + the original context and checks: "The answer says 3 festivals. Verify against the source: is there a 4th? Quote any mentions the answer may have missed."

**Sub-problem 1 (within-turn partial extraction)**: **Uncertain.** If the verifier is given the original context and told "look for items the answer missed," it faces the same extraction task as the original call. It may find the $25 chain because it's specifically looking for missed items (directed search vs. open scan). Or it may exhibit the same attention patterns.

**Sub-problem 2 (cross-session topic filtering)**: **Partially addresses.** The verifier knows the count to verify against (GT-implied or answer-stated), giving it a target. "The answer says 2 weddings, but are there more?" is a more focused scan than "count all weddings." However, the verifier doesn't know the GT -- it only knows the actor's answer. If the actor says "2 weddings" with confidence, the verifier has no signal that 2 might be wrong.

**Sub-problem 3 (festivals)**: **May help** if the verifier re-scans and finds the 4th.

**Cost**: **One additional LLM call per question.** Same as Candidate B (~$0.04/question).

**Risk**: The verifier operates in a biased context -- it receives the actor's answer as a hypothesis and is asked to confirm or refute it. Confirmation bias in LLMs is well-documented: given an answer, the verifier may rationalize it rather than genuinely re-scan. The PR #93 investigation already noted this: "a second pass using the same recognition strategy will miss them again." The verification framing ("is there a 4th?") is better than a re-scan, but still faces the same attention patterns on the same context.

### Candidate assessment summary

| Candidate | Sub-problem 1 (within-turn) | Sub-problem 2 (topic filtering) | Sub-problem 3 (festivals) | Extra calls | Regression risk |
|-----------|----------------------------|---------------------------------|---------------------------|-------------|-----------------|
| A: Quote-anchored per-session | Partial | Direct | Maybe | 0 | Truncation at high session count |
| B: Two-call extract-then-count | Better | Direct | Maybe | 1 | False-positive inflation |
| C: Per-session N-calls | Partial | Strong | Maybe | N (10-20) | Decontextualization; expensive |
| D: Question-type prompting | Weak | Weak | Minimal | 0 | Over-counting regression |
| E: Verification pass | Uncertain | Partial | Maybe | 1 | Confirmation bias |

---

## Section 4 -- The Honest Cost/Benefit

### Cost structure

| Candidate | Calls per question | Estimated $/question | $/20-question run | Latency multiplier |
|-----------|-------------------|---------------------|-------------------|-------------------|
| A | 1 (same as current) | $0.04 | $0.80 | 1.0-1.5x (longer output) |
| B | 2 | $0.08 | $1.60 | 2x |
| C | N+1 (11-21) | $0.44-$0.84 | $8.80-$16.80 | 5-10x (serial) / 2x (parallel) |
| D | 1 (same as current) | $0.04 | $0.80 | 1.0x |
| E | 2 | $0.08 | $1.60 | 2x |

### Regression risk analysis

**Candidate A** (prompt restructuring): Low regression risk. The actor still receives the same context and produces an answer. The structured per-session output changes the reasoning path but not the information available. Risk: if the structured output exceeds max_tokens for high-session-count questions, the actor may truncate analysis of later sessions. Testable: check output length on a 20-session question.

**Candidate B** (two-call): Moderate regression risk. The extraction call may produce false positives that inflate counts. Currently-correct counting questions (10/20 pass) could regress if the extraction over-identifies items. Mitigation: the synthesis call can include deduplication and relevance verification instructions.

**Candidate C** (N-calls): Moderate regression risk from decontextualization. Per-session calls lack cross-session context that helps the actor distinguish "my wedding" from "a wedding I attended." The aggregation call must resolve these ambiguities without access to the original text. Also high cost makes iterative experimentation expensive.

**Candidate D** (prompting): High regression risk. "Over-count" instruction directly conflicts with precision on currently-correct questions. The 3 DEFINITION_DISAGREEMENT cases already show that over-counting is the dominant failure mode for the judge.

**Candidate E** (verification): Low regression risk but low expected benefit. The verifier is unlikely to harm correct answers (it's asked to check, not change). But it's also unlikely to find items the original call missed, due to the same attention patterns.

### Which candidates are cheap AND address real sub-problems?

**Candidate A is the cheapest viable intervention.** Zero extra calls, directly addresses sub-problem 2 (the largest sub-problem by case count), partially addresses sub-problem 1. The risk (truncation) is testable before committing to a full bench run.

**Candidate B is the cheapest STRONG intervention.** One extra call, better coverage of sub-problem 1 (separation of extraction from counting), direct coverage of sub-problem 2. Moderate regression risk from false-positive inflation, but mitigable.

**Candidate D is cheap but doesn't address the real sub-problems.** The current prompt already says "scan every session" and "items may appear as passing mentions." More emphatic phrasing won't change model attention patterns.

**Candidate C addresses sub-problem 2 most strongly but is expensive.** The cost makes it unsuitable for iterative experimentation. Worth considering only if A and B fail.

### The AMBIGUOUS caveat

Cases #8 (tanks) and #9 (weddings) are classified AMBIGUOUS -- they may be partial RETRIEVAL_MISS, not actor failures. If the missed sessions weren't retrieved, no actor intervention helps. The honest accounting:

- **Confirmed GENUINE_MISS**: #3 (bike expenses), #7 (festivals) -- 2 cases
- **AMBIGUOUS (may be retrieval)**: #8 (tanks), #9 (weddings) -- 2 cases

If cases #8 and #9 are retrieval failures, actor interventions address only 2 of 10 multi-session failures. If they are actor failures, interventions address 4 of 10. The expected lift from actor interventions alone is therefore **+1 to +2 questions out of 20** (5-10pp), not the +4 (20pp) that would be the case if all GENUINE_MISS were actor-addressable.

This is a meaningful finding. The ceiling for actor-level interventions on the current failure set is modest even if the interventions work perfectly.

---

## Section 5 -- Recommendation

### Sequenced approach: Candidate A first, Candidate B as escalation

**Step 1: Pre-validation of Candidate A on 2 cases.**

Pick cases #3 (bike expenses) and #7 (festivals). These are the 2 confirmed GENUINE_MISS cases where all answer sessions are verified retrieved.

**Manual experiment for case #3 (bike expenses)**:

Construct the prompt manually with the quote-anchored per-session structure:

```
For each session below, extract EVERY expense or cost mentioned,
with a verbatim quote. Process one session at a time.

<session id="answer_2880eb6c_1">
[session content]
</session>
Items found: [model fills in]

<session id="answer_2880eb6c_2">
[session content]
</session>
Items found: [model fills in]

[...remaining sessions...]

Now count the total unique expenses from all sessions above.
```

Run this manually against Claude Sonnet 4.6 (or the bench model). Check:
- Does the per-session structure force the model to find "$25 chain replacement" in `answer_2880eb6c_2`?
- Does it find "$120 Bell Zephyr helmet" in `answer_2880eb6c_1`?
- Does it still find the "$40 bike lights" it currently finds?

**Manual experiment for case #7 (festivals)**:

Same structure, extracting "film festivals attended" per session. Check whether the per-session scan surfaces the 4th festival.

**Success criteria**: If the per-session structure finds at least one additional item in either case that the current prompt misses, Candidate A is validated for implementation. If it finds zero additional items in both cases, the failure is at a granularity that prompt restructuring can't address, and the conclusion is that sub-problem 1 (within-turn) is a model capability limitation.

**Step 2 (contingent): If Candidate A succeeds on pre-validation, implement in counting_enumerate.md.**

Replace the current quote-first instruction with the per-session structured extraction format. Run targeted multi-session bench (20 questions, ~$1.60).

**Step 3 (contingent): If Candidate A fails on pre-validation OR succeeds but bench lift is < +1, escalate to Candidate B.**

Implement the two-call extract-then-count pattern. Pre-validate on the same 2 cases with the extraction-only call. If extraction-only finds more items, implement the two-call pipeline in the bench harness.

**Step 4 (if both fail): Declare the GENUINE_MISS floor.**

If neither Candidate A nor B finds additional items on the pre-validation cases, the within-turn partial extraction failure is a model capability limitation. The honest conclusion: actor-level prompt/call-structure interventions cannot address sub-problem 1. Sub-problem 2 may be addressable by A or B but is confounded by the AMBIGUOUS retrieval status of cases #8 and #9. The practical ceiling improvement from actor interventions is +0 to +1 questions (0-5pp).

### Why this recommendation, not another

**Why not Candidate D (prompting)?** Already tried twice. PR #93 added the embedded-reference instruction. PR #97 added quote-first. Both are in the current `counting_enumerate.md` and neither worked. A third round of emphatic restating won't change model attention patterns.

**Why not Candidate C (per-session N-calls)?** Too expensive for pre-validation. At $0.40-$0.84/question, testing even 2 cases costs $0.80-$1.68, comparable to a full Candidate B bench run. And per-session extraction faces the decontextualization risk -- a single-session call about "wedding planning" might extract the user's own wedding plans rather than attended weddings, because it lacks the cross-session framing of the original question.

**Why not Candidate E (verification)?** PR #93's analysis already identified the structural problem: "a second pass using the same recognition strategy will miss them again." The verification framing is slightly better (directed search for missing items vs. open re-scan), but the verifier doesn't know the GT count, so it can't know whether to keep looking.

**Why Candidate A before B?** A is cheaper (zero extra calls) and tests the same hypothesis (per-session structured extraction improves recall). If A works, there's no need for B's extra call. If A fails, B's separation-of-concerns adds one more lever (extraction as sole task). The pre-validation experiment is the same for both -- apply per-session structure manually, check if more items are found.

### Backing by failure characterization

- **Candidate A addresses sub-problem 2** (cross-session topic filtering: cases #8, #9) by forcing per-session extraction, eliminating cross-session attention competition. This is the mechanism that PR #93 identified as the dominant failure mode.
- **Candidate A partially addresses sub-problem 1** (within-turn extraction: case #3) by increasing the model's engagement with each session's content. Whether this is sufficient is what the pre-validation tests.
- **Candidate A does NOT address the AMBIGUOUS cases if they are retrieval failures.** The pre-validation uses confirmed GENUINE_MISS cases (#3, #7) specifically to avoid this confound.
- **Candidate B adds separation-of-concerns** for sub-problem 1: the extraction call's sole task is finding items, not counting or synthesizing. This is the best shot at the within-turn miss, because the model's full attention budget is allocated to extraction.

---

## Section 6 -- Permagent Coordination Implications

### Does the production actor share code with the bench actor?

**No.** The bench actor (`AnthropicActor` in `spectral-bench-accuracy/src/actor.rs`) is entirely independent of any Permagent actor. They share:

- The Spectral retrieval API (`Brain::recall_cascade_with_pipeline`)
- The memory formatting function (`format_hits_grouped`)
- The `QuestionType` classifier (lives in `spectral-bench-accuracy`, not in Spectral core)

They do NOT share:
- Prompt templates (bench has `src/prompts/*.md`; Permagent has its own)
- LLM call mechanics (bench uses `reqwest::blocking::Client`; Permagent has its own async pipeline)
- Multi-call patterns (bench is single-call; Permagent's call structure is independent)

### Would an intervention need Permagent-side changes?

**Yes, if the intervention proves effective.** Any successful actor intervention would need to be replicated in Permagent's actor:

- **Candidate A (prompt restructuring)**: Permagent would need to adopt the per-session structured extraction format in its synthesis prompt. Effort: prompt change only, no code change.
- **Candidate B (two-call)**: Permagent would need a two-call actor pipeline: extraction call then synthesis call. Effort: moderate code change (add extraction step, parse structured output, pass to synthesis).
- **Candidate C (N-calls)**: Permagent would need per-session extraction + aggregation. Effort: significant code change and latency impact on production.

### The `QuestionType` classifier

The classifier lives in `spectral-bench-accuracy/src/retrieval.rs`, not in Spectral core. If actor interventions are shape-specific (e.g., only counting questions get the two-call pattern), Permagent would either need to import the classifier or implement its own. Backlog item notes that promoting `Brain::classify_question` to Spectral is deferred until a production consumer needs it.

### Immediate coordination required

**None.** This is a proposal-only investigation. No implementation, no Permagent changes needed. If the recommendation is approved and pre-validation succeeds, Permagent coordination should happen before the bench intervention is promoted to production.

---

## Appendix A -- What was already tried and failed

| Intervention | PR | Target | Result |
|-------------|-----|--------|--------|
| Embedded-reference instruction | #93 | Sub-problem 2 | No lift. Actor still skips subordinate references. |
| Quote-first extraction | #97 | Sub-problems 1+2 | No lift. Actor's `<quotes>` blocks exhibit same topic-following behavior. |
| Max_tokens increase to 4096 | #98 | Output truncation | No lift on GENUINE_MISS. Fixed other issues. |
| Reasoning-aware judge rubric | #102 | DEFINITION_DISAGREEMENT | Reverted. Zero lift on target cases. |
| Session-user clarity instruction | #93 | Session confusion | Fixed case #2 (camping) but not generalizable. |

Each of these was designed to address the GENUINE_MISS failure mode. None worked. This is the context for the recommendation: incremental prompt changes have been tried three times and failed three times. A structural change (per-session extraction format, or separation of extraction from counting) is the next-cheapest lever that hasn't been tried.

## Appendix B -- Expected lift accounting

| Scenario | Cases flipped | Lift (of 20 multi-session) |
|----------|--------------|---------------------------|
| Candidate A works on sub-problem 2 only, #8/#9 are retrieval | 0 | 0pp |
| Candidate A works on sub-problem 2, #8/#9 are actor | +2 (#8, #9) | +10pp |
| Candidate A works on sub-problems 1+2, #8/#9 are actor | +3 (#3, #8, #9) | +15pp |
| Candidate A works on all, #7 addressable | +4 (#3, #7, #8, #9) | +20pp |
| Neither A nor B works | 0 | 0pp |

The expected value depends critically on whether cases #8 and #9 are retrieval or actor failures. Item #21 (retrieval telemetry in bench reports) would resolve this ambiguity. If #21 ships before the actor intervention, the pre-validation can be better targeted.

**Realistic expectation**: +0 to +2 questions (0-10pp). The pre-validation experiment is designed to determine which end of this range is achievable before investing in a full bench run.
