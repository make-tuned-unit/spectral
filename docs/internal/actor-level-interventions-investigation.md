# Actor-Level Interventions Investigation

**Date**: 2026-05-14
**Branch**: `investigate/actor-level-interventions`
**Status**: Investigation complete. Conclusion: **the GENUINE_MISS ceiling is real for single-context-window actor approaches.** Prompt interventions (PR #93, #97, #98) and structured-output variations within a single call all face the same input-attention limitation. The only structurally different lever -- per-session context isolation (Candidate C) -- is expensive and addresses at most the 2 AMBIGUOUS cases. One narrow experiment can determine whether context isolation helps, but the expected lift is modest (+0 to +2 questions, 0-10pp) and contingent on cases #8/#9 being actor failures rather than retrieval failures.

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

### Candidate A -- Per-session structured output (single call)

**Mechanism**: Restructure the single call's prompt to force explicit per-session extraction. The prompt would require the actor to process each session header sequentially and, for each, output a structured block:

```
<session id="answer_e7b0637e_2">
Items found: Emily+Sarah's wedding
</session>
```

After processing all sessions, the actor counts unique items from the per-session blocks.

**Why this is NOT meaningfully different from PR #97**: PR #97's instruction 2 already says: "quote every mention of the item being counted **from each session**. Place quotes in `<quotes>` tags, **one per session** that contains a mention." The model is already told to produce per-session quotes. The result documented in PR #99: the model's `<quotes>` blocks exhibit the same topic-following behavior -- it quotes from sessions it classifies as relevant and skips sessions where the primary topic doesn't match.

Candidate A changes the output format (structured `<session>` blocks instead of flat `<quotes>` tags) but does not change the input or the model's reading pass. In a single-call setup, the model processes all sessions in one context window. Output structure changes what the model *writes*, not what it *reads*. If the model's reading pass doesn't register "Emily finally got to tie the knot with Sarah" as relevant to "weddings attended" -- because the session's primary topic is wedding planning -- then forcing a `<session id="_2">` output block just produces `Items found: None` or skips the block entirely.

The chain-of-thought analogy does not apply. CoT helps with *reasoning* tasks (the model has the facts but needs to externalize logic steps). The GENUINE_MISS failure is an *extraction/attention* failure (the model doesn't register the fact during its reading pass). Structured output formatting doesn't change input attention allocation.

**Evidence**: PR #97 already tested per-session quoting. The model was instructed to produce "one [quote block] per session that contains a mention." It produced per-session quotes for the sessions it found relevant and skipped the sessions containing embedded references. Changing the XML tag format from `<quotes>` to `<session>` doesn't change which sessions the model identifies as containing a mention.

**Assessment**: **Downgraded. Not a viable intervention.** This is a cosmetic variation of PR #97, which already failed. The per-session output structure is not the untried variable -- the per-session structure was already present in PR #97's instruction. What's actually different in Candidate A is nothing.

### Candidate B -- Two-call extract-then-count (structured pre-pass)

**Mechanism**: Call 1 (extraction): receives all session-grouped memories and the question. Instructed: "For each session, list every [item being counted] mentioned, with a verbatim quote and session ID. Do not count or deduplicate. Your job is exhaustive extraction only." Output: JSON array of `{item, session_id, quote}`.

Call 2 (synthesis): receives the JSON array from Call 1 and the question. Instructed: "Given these extracted items, deduplicate and count. State the final answer." No access to raw memories -- only the extracted candidates.

**Why the separation-of-concerns argument is weaker than it appears**: The claimed advantage is that the extraction call's sole task is "find every mention" with no competing synthesis goal, so the model allocates more attention to extraction. But the model's attention allocation during its reading pass is not governed by the output task -- it's governed by the input content and the query. Whether the output task is "extract items as JSON" (Candidate B Call 1) or "quote every mention then count" (current prompt), the model processes the same input tokens with the same attention weights during reading.

PR #97 already separated extraction (in `<quotes>`) from counting (after quotes) within a single call. The two-phase structure exists. Candidate B moves the phases to separate API calls, but the extraction call still processes all sessions in one context window. The cross-session topic filtering that causes the model to skip subordinate references operates during the reading pass, not during the output generation pass. Changing whether extraction and counting are in the same call or separate calls doesn't change how the model reads the input.

The LangExtract/Multi-Step Reasoning survey evidence (cited in `docs/internal/external-research-synthesis-2026-05-12.md`) supports separation-of-concerns for tasks where extraction and reasoning compete for output-generation attention. The failure mode here is different: the model fails to *notice* the item during reading, not to *reason about* it during output. Separation of concerns addresses the wrong bottleneck.

**Sub-problem 1 (within-turn partial extraction)**: **Does not address.** The $25 chain and $40 lights are in the same sentence pair. The extraction call processes the same sentence pair in the same context window. If the model reads "it cost me $25...also got bike lights...which were $40" and extracts only the $40, having extraction as the sole task doesn't change the sentence-level attention drop.

**Sub-problem 2 (cross-session topic filtering)**: **Does not address.** The extraction call still processes all 20 sessions in one context window. The model's reading pass still classifies sessions by primary topic and allocates attention accordingly. A wedding-planning session's subordinate reference to an attended wedding is still subordinate during the extraction call's reading pass.

**Assessment**: **Downgraded.** Candidate B adds one LLM call but doesn't change the input-attention dynamic that causes the failure. The two-call structure addresses a bottleneck (extraction competing with counting) that is not the actual bottleneck (extraction failing during reading).

### Candidate C -- Per-session extraction (N calls)

**Mechanism**: One LLM call per retrieved session: "Given this session, what does it say relevant to [question]?" Then an aggregation call over all per-session extractions.

**This is the only candidate that changes the input-attention dynamic.** Each session is processed in a context window containing only that session's content (a few turns) plus the question. There is no cross-session attention competition. The model cannot skip a wedding-attendance reference in session _2 because it's "less relevant than the wedding-planning topic" -- there IS no wedding-planning topic in session _2's isolated context. Each session gets the model's full attention budget.

**Sub-problem 1 (within-turn partial extraction)**: **Does not address.** Isolating a single session doesn't change within-turn extraction. The $25 chain and $40 lights are in the same turn within one session; processing that session alone, the model faces the same sentence-level attention drop. This sub-problem is at a granularity below what context isolation can fix.

**Sub-problem 2 (cross-session topic filtering)**: **Directly addresses by eliminating the mechanism.** Cross-session topic filtering requires multiple sessions competing for attention. With one session per call, there is no competition. This is the mechanistically correct intervention for sub-problem 2.

**Sub-problem 3 (festivals)**: **May address** if the 4th festival is in a session that the actor skimmed due to topic filtering in the whole-context call.

**Cost**: **N additional LLM calls per question**, where N is the number of sessions (typically 10-20 for counting questions). At $0.04/call, that's $0.40-$0.80 per question or $8-$16 for a 20-question bench run. **Expensive for routine use.**

**Risk**: Decontextualization. Per-session calls lack cross-session framing that helps the actor distinguish "my wedding" from "a wedding I attended." A question like "How many weddings did I attend?" sent to a session about wedding planning might extract the user's own wedding plans rather than attended weddings. Mitigation: pass the original question explicitly ("What does this session say relevant to: How many weddings have I attended?"), so the question provides the framing. But this risk is real and untested.

### Candidate D -- Question-type-specific prompting

**Mechanism**: Counting questions get a different prompt emphasizing exhaustive enumeration and over-counting.

**Assessment**: **Not viable.** The current prompt already says "scan EVERY session" (instruction 1) and "items may appear as passing mentions" (instruction 3, added PR #93). PR #93 and PR #97 both added instructions targeting the same failure mode. Neither changed model behavior. A third iteration of emphatic prompting addresses the wrong layer. The failure is in input attention, not in instruction comprehension. The model understands the instruction; it doesn't execute it because its attention allocation during reading doesn't surface the embedded references.

**Risk**: "Err on over-counting" would regress currently-correct cases. The 3 DEFINITION_DISAGREEMENT cases (#1 clothing, #2 projects, #6 citrus) already show over-counting issues.

### Candidate E -- Re-read / verification pass

**Mechanism**: After the actor produces an answer, a second call receives the answer + the original context and checks for missed items.

**Assessment**: **Not viable.** PR #93 identified the structural problem: "a second pass using the same recognition strategy will miss them again." The verifier processes the same context window with the same attention patterns. The verification framing ("is there a 4th?") provides a slightly more directed scan, but the verifier doesn't know the GT count and so has no signal that the actor's count is wrong. The actor says "2 weddings" with confidence; the verifier has no reason to doubt it. Confirmation bias in LLMs is well-documented for this pattern.

### Candidate assessment summary (revised)

| Candidate | Differs from PR #97? | Sub-problem 1 (within-turn) | Sub-problem 2 (topic filtering) | Extra calls | Viable? |
|-----------|---------------------|----------------------------|---------------------------------|-------------|---------|
| A: Per-session structured output | **No** | Does not address | Does not address | 0 | **No** |
| B: Two-call extract-then-count | Superficially | Does not address | Does not address | 1 | **No** |
| C: Per-session N-calls | **Yes** -- actual context isolation | Does not address | **Directly addresses** | N (10-20) | Testable |
| D: Question-type prompting | No | Does not address | Does not address | 0 | **No** |
| E: Verification pass | No | Does not address | Weakly | 1 | **No** |

**The critical finding**: Candidates A, B, D, and E all operate within a single context window containing all sessions. They vary the output format, the task framing, or the number of calls, but the model's reading pass over the input is the same in all of them. The reading pass is where the failure occurs -- the model allocates attention to primary session topics and skips subordinate references. Only Candidate C changes the reading pass by eliminating cross-session attention competition entirely.

---

## Section 4 -- The Honest Cost/Benefit

### Why single-context-window approaches are exhausted

Three prompt-level interventions have been tried (PR #93, #97, #98). Each targeted the GENUINE_MISS failure mode. None worked. This investigation analyzed two additional single-context-window variations (Candidates A and B) and found they are not meaningfully different from what was already tried:

- **PR #97** already asked for per-session quotes. Candidate A changes the XML format but not the mechanism.
- **PR #97** already separated extraction (`<quotes>`) from counting (final answer). Candidate B moves them to separate API calls but doesn't change the reading pass.

The pattern is clear: **within a single context window, the model's attention allocation during reading is the bottleneck, and no output-side restructuring changes it.** The model reads 20 sessions, classifies each by primary topic, allocates attention accordingly, and skips subordinate references in off-topic sessions. Instructions to "scan every session" and "look for passing mentions" don't override this attention pattern -- the model complies with the instruction as it understands it (it does scan every session) but its reading doesn't surface the embedded references.

### What Candidate C actually costs

| Metric | Value |
|--------|-------|
| Calls per question | N+1 (11-21 typical) |
| Estimated $/question | $0.44-$0.84 |
| $/20-question bench run | $8.80-$16.80 |
| Latency multiplier | 5-10x (serial) / 2x (parallel) |
| Pre-validation cost (3 sessions, 1 case) | ~$0.12 |

### The AMBIGUOUS caveat

Cases #8 (tanks) and #9 (weddings) are the only cases where sub-problem 2 (cross-session topic filtering) is the likely mechanism. Both are classified AMBIGUOUS -- they may be partial RETRIEVAL_MISS, not actor failures. If the missed sessions weren't retrieved, context isolation can't help because the sessions aren't in the context to isolate.

The honest accounting:

- **Confirmed GENUINE_MISS**: #3 (bike expenses), #7 (festivals) -- 2 cases
- **AMBIGUOUS (may be retrieval)**: #8 (tanks), #9 (weddings) -- 2 cases

Case #3 is sub-problem 1 (within-turn) -- context isolation doesn't help. Case #7 is sub-problem 3 (unknown mechanism). Only cases #8 and #9 are sub-problem 2, and they're AMBIGUOUS.

**Candidate C addresses at most 2 of 10 multi-session failures, and only if those 2 are actually actor failures rather than retrieval failures.**

### Regression risk for Candidate C

Per-session calls lack cross-session context. This creates a specific risk: a question like "How many weddings did I attend?" sent to a session about wedding planning might extract the user's own wedding plans rather than attended weddings, because the per-session call lacks the broader conversational framing. The question provides some framing ("weddings I attended"), but within a wedding-planning session, the model may still extract the user's own wedding ("I'm planning a garden wedding for June") as a "wedding" mentioned in the session.

This risk could cause Candidate C to regress currently-correct cases where the whole-context actor correctly distinguishes primary-topic items from counted items.

---

## Section 5 -- Recommendation

### The GENUINE_MISS ceiling is real

Single-context-window prompt interventions for the GENUINE_MISS failure mode are exhausted:

1. **PR #93** (embedded-reference instruction): tried, failed. Actor still skips subordinate references.
2. **PR #97** (quote-first extraction with per-session quoting): tried, failed. Actor's `<quotes>` blocks exhibit same topic-following behavior.
3. **PR #98** (max_tokens increase): tried, no lift on GENUINE_MISS.
4. **Candidate A** (per-session structured output): analyzed, not meaningfully different from PR #97.
5. **Candidate B** (two-call extract-then-count): analyzed, separation of concerns doesn't change the reading pass.
6. **Candidate D** (emphatic prompting): third iteration of same approach, already failed twice.
7. **Candidate E** (verification pass): faces same attention patterns, confirmation bias.

The failure mechanism -- input-attention allocation that prioritizes primary session topics over subordinate references -- is not addressable by output-side restructuring within a single context window.

### One narrow experiment path remains

Candidate C (per-session context isolation) is the only structurally different lever that hasn't been tried. It eliminates cross-session attention competition by processing each session in isolation. This directly addresses sub-problem 2 (the only sub-problem it CAN address), at the cost of N additional LLM calls per question.

**Whether this experiment is worth running depends on a prior question that should be resolved first: are cases #8 and #9 actually actor failures?**

Item #21 (retrieval telemetry in bench reports) would populate `memory_keys` in `report.json`, resolving the AMBIGUOUS classification. If the missed sessions in cases #8 and #9 were NOT retrieved, they are retrieval failures, and Candidate C is moot for the current failure set. If they WERE retrieved, Candidate C targets exactly these cases.

**Recommended sequence**:

1. **Ship item #21** (retrieval telemetry). Effort: 2-3h. Resolves the AMBIGUOUS classification.
2. **Re-run multi-session bench** with telemetry populated. Determine whether cases #8 and #9 are retrieval or actor failures.
3. **If actor failures**: Run a manual pre-validation of Candidate C on case #9 (weddings). Take the 3 answer sessions from `longmemeval_s.json`. For each, send a single LLM call: "Given this conversation session, list every wedding the user mentions attending. Quote the relevant text." Cost: ~$0.12 for 3 calls. Check whether session `answer_e7b0637e_2` in isolation produces "Emily and Sarah's wedding" and session `answer_e7b0637e_3` produces "Jen and Tom's wedding." This isolates the variable: does context isolation surface the embedded references that whole-context processing misses?
4. **If the pre-validation succeeds**: Candidate C is validated for sub-problem 2. Decide whether the lift (at most +2 questions, 10pp) justifies the per-question cost ($0.40-$0.80) and latency (5-10x) for production use.
5. **If retrieval failures OR pre-validation fails**: The GENUINE_MISS floor is confirmed. Accept it in the path-to-90% math.

### What this means for the path-to-90%

The multi-session GENUINE_MISS failures represent a hard floor for prompt-level and call-structure interventions. The path to higher accuracy on multi-session runs through:

- **Retrieval improvements** (item #8: description-enriched FTS) -- addresses RETRIEVAL_MISS cases (#4, #10) and potentially the AMBIGUOUS cases (#8, #9) if they are retrieval failures. This is the highest-leverage remaining intervention for multi-session.
- **Judge refinement** -- the 3 DEFINITION_DISAGREEMENT cases (#1, #2, #6) are judge-side failures, not actor-side. Item #20 was reverted, but the cases remain addressable by a better judge approach.
- **Model capability improvements** -- sub-problem 1 (within-turn partial extraction) is a model-level limitation. A more capable model might extract both the $25 chain and $40 lights from the same sentence pair. This is out of scope for Spectral engineering.
- **Accepting the floor** -- if items #8 and #21 ship and the AMBIGUOUS cases resolve to retrieval, the actor-addressable GENUINE_MISS count drops to 2 (cases #3 and #7). Case #3 is within-turn (intractable). Case #7 is uncharacterized (may be GT accuracy). The practical GENUINE_MISS floor is 1-2 questions out of 20.

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

### Would Candidate C need Permagent-side changes?

**Yes, if validated and adopted.** Per-session extraction would require:

- Permagent splits retrieved memories by session before sending to the actor
- N per-session extraction calls + 1 aggregation call (replaces single actor call)
- Latency budget for N+1 calls per question (5-10x current)
- Possibly restricted to counting questions only (via `QuestionType` classifier, which would need to be promoted to Spectral core or reimplemented in Permagent)

### The `QuestionType` classifier

The classifier lives in `spectral-bench-accuracy/src/retrieval.rs`, not in Spectral core. If per-session extraction is shape-specific (only counting questions), Permagent would need the classifier. Backlog item notes that promoting `Brain::classify_question` to Spectral is deferred until a production consumer needs it.

### Immediate coordination required

**None.** This investigation concludes that the ceiling is real and the remaining experiment path (Candidate C pre-validation) is contingent on item #21 shipping first. No Permagent changes needed unless Candidate C is validated.

---

## Appendix A -- What was already tried and failed

| Intervention | PR | Target | Result |
|-------------|-----|--------|--------|
| Embedded-reference instruction | #93 | Sub-problem 2 | No lift. Actor still skips subordinate references. |
| Quote-first extraction (per-session quoting) | #97 | Sub-problems 1+2 | No lift. Actor's `<quotes>` blocks exhibit same topic-following behavior. |
| Max_tokens increase to 4096 | #98 | Output truncation | No lift on GENUINE_MISS. Fixed other issues. |
| Reasoning-aware judge rubric | #102 | DEFINITION_DISAGREEMENT | Reverted. Zero lift on target cases. |
| Session-user clarity instruction | #93 | Session confusion | Fixed case #2 (camping) but not generalizable. |

### Candidates analyzed and rejected in this investigation

| Candidate | Claim | Why rejected |
|-----------|-------|-------------|
| A: Per-session structured output | Per-session output blocks force deeper engagement | PR #97 already asked for per-session quotes. Output format changes what the model writes, not what it reads. Same attention pattern, same failures. |
| B: Two-call extract-then-count | Separation of concerns improves extraction recall | Extraction call still processes all sessions in one context window. Same reading pass, same attention allocation. PR #97 already separated extraction from counting within a single call. |
| D: Emphatic prompting | Stronger enumeration instructions improve coverage | Third iteration of same approach. PR #93 added "scan for passing mentions"; PR #97 added "quote every mention." Model understands the instruction; its attention doesn't execute it. |
| E: Verification pass | Second pass finds missed items | Same context window, same attention patterns. Confirmation bias. PR #93 already noted: "a second pass using the same recognition strategy will miss them again." |

### The underlying pattern

All failed and rejected interventions share the same limitation: **they operate within a single context window containing all sessions.** The model's reading pass over the input is where the failure occurs. The model allocates attention to primary session topics and de-prioritizes subordinate references. No output-side intervention (formatting, task framing, verification, multi-call with shared context) changes this reading-pass behavior.

The only intervention that changes the reading pass is context isolation -- processing each session in its own context window (Candidate C). This hasn't been tried and is testable.

## Appendix B -- Expected lift accounting

| Scenario | Cases flipped | Lift (of 20 multi-session) |
|----------|--------------|---------------------------|
| Context isolation works on sub-problem 2, #8/#9 are actor | +2 (#8, #9) | +10pp |
| Context isolation works, #8/#9 are retrieval | 0 | 0pp |
| Context isolation doesn't work (attention failure persists in isolation) | 0 | 0pp |
| Sub-problem 1 (#3) addressed by model upgrade (out of scope) | +1 | +5pp |
| Sub-problem 3 (#7) is GT accuracy issue | 0 | 0pp |

The maximum achievable lift from actor-level interventions on the current failure set is **+2 questions (+10pp)**, and this requires: (a) cases #8 and #9 are confirmed actor failures, (b) context isolation addresses sub-problem 2, and (c) the decontextualization risk doesn't regress other cases.

**Realistic expectation**: +0 to +1 questions (0-5pp). The highest-leverage next step is item #21 (retrieval telemetry) to resolve the AMBIGUOUS classification, followed by item #8 (description-enriched FTS) which addresses the cases whether they are retrieval or actor failures.
