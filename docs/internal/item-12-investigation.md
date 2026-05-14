# Item #12 Investigation: L2 Episode Summaries

**Date**: 2026-05-14
**Branch**: `investigate/item-12-l2-episodes`
**Status**: Investigation complete. Recommendation: **defer**.

---

## Section 1 -- Current Cascade Architecture Inventory

### What the pipeline is today

The cascade is a **single integrated pipeline** at `cascade_layers.rs:143`. It replaced the original three-layer orchestrator (L1 AaakLayer, L2 EpisodeLayer, L3 ConstellationLayer) in the pivot commit `7e0448d` on May 7. The old Layer trait implementations were deleted; the `spectral-cascade` crate retains the `Cascade`, `Layer`, and `LayerResult` types as dead code, with live types being `RecognitionContext`, `CascadeResult`, and `CascadeConfig`.

**Pipeline stages (in order)**:

| # | Stage | File:Line | Description |
|---|-------|-----------|-------------|
| 1 | TACT + FTS retrieval | `brain.rs:1043-1083` | TACT tiered search (fingerprint -> wing -> FTS), supplemented with raw FTS if TACT < K. Single retrieval pass, K=40 default. |
| 2 | Ambient boost | `cascade_layers.rs:17-58` | Wing alignment (1.5x) + recency (1.3x/1.1x) + mismatch penalty (0.7x). Multiplicative, clamped [0.5, 2.0]. Identity when context empty. |
| 3 | Unified re-ranking | `ranking.rs:282+` | Signal score blending (w=0.3), declarative density boost (w=0.10), co-retrieval boost (w=0.10), recency decay (half-life 365d), entity boost (+0.05 for wing cluster leaders). All composable, controlled by `RerankingConfig`. |
| 4 | Episode diversity | `cascade_layers.rs:66-98` | Cap per-episode memories at `max_per_episode` (default 5), push overflow to tail. Interleave rather than discard. |
| 5 | Context chain dedup | `ranking.rs:152-187` | Collapse near-duplicate `[Memory context]` references. |
| 6 | Auto-reinforce | `cascade_layers.rs:191-194` | +0.01 signal_score nudge on retrieved memories. |
| 7 | Retrieval event logging | `cascade_layers.rs:197-207` | Log query hash, memory IDs, method, wing, session for co-access mining. |

### How episodes exist today

Episodes are a **first-class data concept** but a **lightweight retrieval concept**:

- **Schema**: `episodes` table in SQLite with `id`, `started_at`, `ended_at`, `memory_count`, `wing`, `summary_preview` (first ~200 chars of highest-signal memory). No `summary` field.
- **Memory association**: `memories.episode_id` (nullable). Set via `RememberOpts::episode_id` (consumer-provided) or auto-detected via time-gap heuristic at ingest.
- **Brain API**: `list_episodes(wing, limit)`, `list_memories_by_episode(episode_id)` -- both exist and work.
- **Retrieval use**: `MemoryHit.episode_id` is populated on recall. Episode diversity (`cascade_layers.rs:66`) uses it to cap per-episode representation in results.
- **What episodes DON'T have**: No summary text. No description. No indexed representation in FTS. Episodes are grouping metadata, not searchable content.

### What "L2 episode summaries" would add

The gap between the current architecture and a hypothetical L2 is:

1. **Summary generation**: Computing a summary for each episode (what happened, key facts, entities mentioned).
2. **Summary storage**: A `summary` column on the `episodes` table (or a separate table).
3. **Summary retrieval**: Indexing summaries in FTS so they're searchable alongside individual memories.
4. **Summary-vs-memory ranking**: Deciding when to surface a summary vs. constituent memories, or both.

---

## Section 2 -- Candidate L2 Episode Summary Design

### What is an "episode"?

In the current system, an episode is a **session-level grouping**. For LongMemEval bench data, each session (a conversation between user and assistant) is one episode. For Permagent production data, episodes are activity windows grouped by time-gap heuristic or consumer-provided IDs.

For the L2 design, an episode = a session. The summary describes what happened in that session.

### What would a summary contain?

A summary would be a concise paragraph (50-150 words) capturing:
- **Key facts stated** by the user (preferences, decisions, experiences)
- **Entities mentioned** (people, places, projects, pets)
- **Activities described** (what the user did, visited, bought, attended)

Example: For a session about wedding planning that mentions attending Emily and Sarah's wedding, an ideal summary would be: "User discussed wedding planning (choosing between garden and ballroom venues). User mentioned attending Emily and Sarah's rooftop ceremony in the city and Jen and Tom's rustic barn wedding."

### When would summaries be generated?

Two options:

**Option A -- At ingest (Librarian-generated)**: Permagent's Librarian (the LLM-powered background processor) generates summaries as part of its description-writing pass. After the Librarian writes descriptions for individual memories, it rolls up per-episode summaries. This requires Permagent coordination.

**Option B -- On demand (at retrieval time)**: When the cascade runs, compute episode summaries from constituent memories. This would require LLM-in-loop retrieval, violating the zero-LLM recognition commitment.

**Only Option A is viable** given the architectural commitment. Summaries must be pre-computed and stored before retrieval.

### How would summaries be used in retrieval?

The most natural integration is **dual-index FTS**: summary text is indexed alongside memory content. A query like "How many doctors did I visit?" would match a summary containing "User discussed visits to Dr. Smith, Dr. Patel, and Dr. Lee" even if individual memory turns don't contain the word "doctors."

This is essentially the same mechanism as item #8 (compiled-truth boost / description-enriched FTS), but operating at the episode level instead of the memory level.

### Who generates the summaries?

The Librarian. This is a Permagent coordination concern:

1. Librarian already writes per-memory descriptions (item #8 dependency).
2. Librarian would additionally write per-episode summaries after processing constituent memories.
3. Spectral would expose `Brain::set_episode_summary(episode_id, summary)` and index the summary in FTS.
4. The cascade pipeline would include episode summaries in the candidate pool (or boost memories from episodes whose summary matches the query).

---

## Section 3 -- Mapping to Documented Failure Cases

### DEFINITION_DISAGREEMENT (cases #1, #2, #6) -- 3 failures

**Would L2 summaries help?** No.

These are judge-side disagreements about counting boundaries (exchange = 1 or 2? grapefruit = used or suggested?). The actor found the evidence and reasoned about it. The failures are in the counting judgment, not in retrieval or recognition. An episode summary wouldn't change the actor's interpretation of what counts.

### GENUINE_MISS (cases #3, #7) -- 2 confirmed failures

**Case #3 -- Bike expenses ($25 chain, $120 helmet):**

The actor had all 4 answer sessions retrieved. The $25 chain is **in the same turn** as the $40 bike lights the actor DID find. This is within-turn partial extraction -- the actor read the sentence pair and extracted one price but not the other.

**Would an episode summary help?** An ideal summary might say: "User spent $25 on chain replacement, $40 on bike lights, and $120 on a Bell Zephyr helmet." If this summary were in the actor's context, would the actor use it? Possibly -- but the actor ALREADY has the original turns in context and missed the items there. A summary is a compressed version of what the actor already has. If the actor can't extract "$25 chain" from "The mechanic told me I needed to replace the chain, which I did, and it cost me $25", it's unclear why it would extract it from a summary that says the same thing in fewer words.

**Key tension**: Summaries compress. The original text is maximally explicit about the $25 chain. A summary would be less explicit. This is the compression-vs-embedded-reference problem: summaries remove detail, and the failure mode is about failing to notice detail.

**Counter-argument**: A summary that explicitly lists itemized expenses as a bulleted fact might be MORE salient than a narrative sentence. This depends entirely on summary format -- a structured summary (key: expenses, items: [$25 chain, $40 lights, $120 helmet]) would be more salient than a narrative one. But structured summaries are a format design choice, not a retrieval architecture choice.

**Verdict**: Unlikely to help. The failure is within-turn attention, not retrieval.

**Case #7 -- Movie festivals (4th festival):**

The 4th festival's location is unclear from the analysis. 3 festivals were found in 3 answer sessions. If the 4th is in a non-answer session, it's an embedded-reference retrieval issue. If the GT is wrong about the count, it's a GT accuracy issue.

**Would an episode summary help?** If the 4th festival is in a non-answer session (i.e., a session whose primary topic isn't festivals), a summary saying "User mentioned attending [festival name]" would bridge the topic gap. This is the same mechanism as item #8's description-enriched FTS. But it's also the same mechanism -- no added value over per-memory descriptions that already would surface the reference.

**Verdict**: No incremental value over item #8.

### AMBIGUOUS -- likely partial RETRIEVAL_MISS (cases #8, #9) -- 2 failures

**Case #8 -- Tanks (5-gallon betta tank):**

Reclassified from GENUINE_MISS to AMBIGUOUS in the 2026-05-13 correction. The 5-gallon betta tank from `answer_c65042d7_2` has no evidence of retrieval. The session's primary topic is high nitrite levels in a community tank; the betta tank is background context.

**Would an episode summary help?** An ideal summary would say: "User discussed high nitrite levels in their 20-gallon community tank. User also mentions a 5-gallon tank with a betta fish named Finley." This summary would index "tank" and "betta" and "5-gallon", making the session retrievable for "How many tanks do you own?"

**But**: A per-memory description (item #8) on the individual memory turn would do the same thing. "User mentions owning a 5-gallon betta tank with a fish named Finley, alongside a 20-gallon community tank." The description and the episode summary serve the same FTS-bridging function.

**Incremental value of summary over description**: Minimal. Both bridge the vocabulary gap. The summary might be slightly better at capturing cross-turn relationships within a session (the 5-gallon tank is mentioned in turn 1, nitrite levels in turn 3, and the connection is that both are about the user's aquarium setup). But FTS matching doesn't need cross-turn relationships -- it needs the word "tank" near the word "own" in an indexed document, which a per-memory description provides.

**Verdict**: No incremental value over item #8.

**Case #9 -- Weddings (Emily+Sarah, Jen+Tom):**

Reclassified to AMBIGUOUS. The wedding sessions `answer_e7b0637e_2` and `answer_e7b0637e_3` have no evidence of retrieval. Their primary topic is the user's own wedding planning; the attended weddings are embedded as context/inspiration.

**Would an episode summary help?** An ideal summary: "User discussed wedding planning (venue selection, decor). User mentioned attending friend Emily and Sarah's rooftop wedding and friend Jen and Tom's rustic barn wedding."

This would index "wedding", "attended", "Emily", "Sarah", "Jen", "Tom" in a searchable document. For the query "How many weddings have you attended?", this summary would match on "attended" + "wedding".

**But again**: A per-memory description on the turn mentioning Emily and Sarah's wedding ("User mentioned attending Emily and Sarah's rooftop wedding ceremony") would do the same thing. The description enriches the individual memory's FTS footprint the same way the summary enriches the episode's.

**Incremental value**: Near zero. Both mechanisms bridge the same vocabulary gap.

**Verdict**: No incremental value over item #8.

### RETRIEVAL_MISS (cases #4, #10) -- 2 failures

**Case #4 -- Doctors (0/3 sessions retrieved):**

Vocabulary gap: "doctors" vs "Dr. Smith", "Dr. Patel", "Dr. Lee", "ENT specialist". Zero answer sessions retrieved.

**Would an episode summary help?** Yes -- a summary containing "User visited three doctors: Dr. Smith (primary care), Dr. Patel (ENT), Dr. Lee (dermatologist)" would bridge the gap. But a per-memory description would also bridge it: "User discussed visit to ENT specialist Dr. Patel for sinusitis diagnosis."

**Incremental value**: The summary has a slight edge here because it aggregates across turns within a session. A single session has 3 turns, each mentioning different doctors. Individual descriptions would mention one doctor each. The summary would mention all three in one document, potentially getting a stronger FTS match on "doctors" (plural). But this advantage is marginal -- FTS matches on any document containing the term, so three documents each containing one doctor name are as retrievable as one document containing all three.

**Verdict**: Marginal improvement over item #8, not sufficient to justify the complexity.

**Case #10 -- Furniture (2/4 sessions missing):**

Vocabulary gap: "furniture" vs "coffee table", "mattress", "bedside tables". Two answer sessions not retrieved.

**Same analysis as #4.** Episode summaries and per-memory descriptions serve the same FTS-bridging function.

**Verdict**: No incremental value over item #8.

### DATE/TEMPORAL_REASONING (case #5) -- 1 failure

**Would L2 summaries help?** No. The actor found the evidence, performed incorrect date arithmetic. This is a model capability issue.

### Summary: mapping to failures

| Failure category | Count | L2 summaries help? | Incremental over item #8? |
|-----------------|-------|--------------------|-----------------------------|
| DEFINITION_DISAGREEMENT | 3 | No | N/A |
| GENUINE_MISS | 2 | Unlikely | No |
| AMBIGUOUS (partial RETRIEVAL_MISS) | 2 | Yes, but same mechanism as #8 | No |
| RETRIEVAL_MISS | 2 | Yes, but same mechanism as #8 | Marginal at best |
| DATE/TEMPORAL_REASONING | 1 | No | N/A |

**L2 episode summaries do not address any documented failure that item #8 (per-memory descriptions) does not already address.** The mechanisms are identical: generate text that bridges vocabulary gaps, index it in FTS, improve retrieval. The granularity differs (episode vs. memory), but the FTS-bridging function is the same.

---

## Section 4 -- The Compression-vs-Embedded-Reference Tension

This is the central analytical finding of the investigation.

### The hardest failures are about detail

The multi-session failures that resist every intervention are embedded-reference failures:

- **Weddings**: "Emily finally got to tie the knot with Sarah" -- a direct, explicit statement embedded in a wedding-planning session. The actor sees it and doesn't register it.
- **Tanks**: "I have a 5-gallon tank with a solitary betta fish named Finley" -- a direct statement in the first turn, used as background context for a nitrite-levels discussion.
- **Bike expenses**: "$25 chain replacement" in the same sentence pair as "$40 bike lights" -- the actor extracts one and misses the other.

### Summaries are lossy compression

A summary is, by definition, a shorter version of the source material. Summaries decide what's important enough to include and what to omit. This is the fundamental tension:

**If a summary includes the embedded reference**, it serves the same function as a per-memory description -- it bridges the vocabulary gap for FTS. No incremental value over item #8.

**If a summary omits the embedded reference** (because the summarizer decides the 5-gallon betta tank is background context, not the session's main topic), the summary actively hurts: it creates a searchable representation of the episode that doesn't mention the item the user is asking about. The retrieval system would find the summary, show it to the actor, and the actor would see a summary that says "User discussed high nitrite levels in aquarium" with no mention of the betta tank.

**The summarizer faces exactly the same attention problem as the actor.** If we ask an LLM to summarize a wedding-planning session, will it mention that the user attended Emily and Sarah's wedding? Or will it track the primary topic (the user's own wedding planning) and omit the attended-wedding references?

This is not hypothetical. The same LLM attention pattern that causes the actor to miss embedded references during synthesis would cause the summarizer to miss them during summary generation. Summaries generated by an LLM with standard instructions will emphasize primary topics and de-emphasize subordinate mentions -- precisely the references we most need to preserve.

### Could structured summaries help?

A structured summary format (entity list, event list, key facts) might mitigate the compression risk by forcing exhaustive extraction. But:

1. **This is a quality requirement on the Librarian's summarization prompt**, not an architecture choice. If the Librarian can be prompted to extract every entity and event from a session, it can also be prompted to write per-memory descriptions that capture those entities and events.

2. **Structured summaries are per-memory descriptions in disguise.** A "summary" that exhaustively lists every entity and event mentioned in every turn of a session is equivalent to the union of per-memory descriptions for each turn. The episode grouping adds no information that per-memory descriptions don't provide.

3. **The real value would be in cross-turn relationship extraction** -- "The user discussed their own wedding planning AND mentioned attending 3 other weddings" -- but this cross-turn synthesis is exactly what the actor is failing to do at retrieval time. Moving the synthesis to ingest time helps only if the ingest-time synthesizer is better at it than the retrieval-time actor. Both use the same LLM. There's no reason to expect the Librarian's summarization to succeed where the actor's extraction fails, given the same underlying attention patterns.

---

## Section 5 -- Recommendation: Defer

**L2 episode summaries should not be pursued for the current failure set.** The investigation finds:

1. **No documented failure case exists that L2 summaries address and item #8 (per-memory descriptions) does not.** Both mechanisms bridge vocabulary gaps via FTS. The granularity differs, but the retrieval function is identical.

2. **The compression-vs-embedded-reference tension is real and structural.** Summaries compress. The hardest failures are about detail that compression loses. A summary that omits embedded references is worse than no summary -- it creates a false sense of coverage. A summary that includes embedded references duplicates what per-memory descriptions already provide.

3. **The summarizer faces the same attention problem as the actor.** There's no reason to expect LLM-generated summaries to capture embedded references that the LLM actor misses during synthesis. Both use the same model, the same attention patterns, the same bias toward primary topics.

4. **Effort is disproportionate.** The backlog estimates item #12 at "1 week (single largest item in the backlog)." For zero incremental lift over item #8 (estimated at 2-3h), this is unjustifiable.

### What the backlog entry got wrong

The backlog entry for item #12 states: "Bench analysis confirmed this is the highest-leverage individual item: multi-session counting (9 failures, biggest absolute failure count) needs L2 episode-grouped retrieval to let the actor scan over discrete session blocks rather than 80+ interleaved hits."

This was written on May 11, before three important investigations:

1. **PR #93 / multi-session failure investigation** showed that 9/10 multi-session failures are ACTOR_MISS (actor has the evidence, fails to extract). L2 doesn't address actor attention.

2. **PR #99 / failure classification** refined the breakdown to DEFINITION_DISAGREEMENT (3) + GENUINE_MISS (2) + AMBIGUOUS (2) + RETRIEVAL_MISS (2) + TEMPORAL (1). The failures are more varied than "counting needs session blocks."

3. **PR #70 / session-grouped formatting** already solved the "discrete session blocks" problem as a formatting concern without any summary generation. Memories are presented grouped by session in the actor's context. The actor still misses embedded references even when sessions are cleanly delineated.

The backlog's "+3-4pp" estimate was based on L2 addressing the multi-session counting bottleneck broadly. The investigation shows L2 doesn't address DEFINITION_DISAGREEMENT (judge-side), doesn't address GENUINE_MISS (actor attention), and duplicates item #8 for RETRIEVAL_MISS.

### Conditions that would change the assessment

Item #12 becomes worthwhile if:

1. **Item #8 ships and demonstrates a ceiling.** If per-memory descriptions bridge some vocabulary gaps but episode-level vocabulary gaps remain unaddressed (e.g., cross-turn entity aggregation that individual descriptions can't capture), episode summaries provide incremental value. This is testable: after item #8 ships, re-run the bench and check whether any RETRIEVAL_MISS failures persist where the missing sessions have descriptions but the descriptions don't contain the query vocabulary. If so, episode summaries that aggregate across turns might help.

2. **A structured summary format is validated** that reliably captures embedded references. If the Librarian's summarization can be shown to extract subordinate mentions with high recall (>90% of entities/events, including embedded ones), the compression risk is mitigated. This would require a dedicated summarization quality eval, not speculation.

3. **Production usage reveals session-level retrieval patterns** that LongMemEval doesn't capture. Real conversations span multiple days and topics. Session-level summaries might matter more for production retrieval than for the benchmark's single-session format.

4. **The actor attention problem is solved** (by per-session extraction, two-call patterns, or model improvements), shifting the bottleneck from actor synthesis to retrieval. If the actor can reliably extract embedded references, then improving retrieval quality (including via session summaries) becomes the next priority.

---

## Section 6 -- Permagent Coordination Implications

If L2 summaries were pursued (they shouldn't be yet), the coordination requirements would be:

### Spectral-side

1. Add `summary: Option<String>` to `Episode` struct.
2. Add `Brain::set_episode_summary(episode_id, summary)` API.
3. Index `episode.summary` in the FTS index (either as a virtual memory or as a separate FTS table).
4. Modify the cascade pipeline to include episode summaries in the candidate pool, or boost memories from episodes whose summary matches the query.

### Permagent-side

1. Librarian adds a summarization pass after writing per-memory descriptions.
2. For each episode with all constituent memories described, Librarian generates an episode summary.
3. Librarian calls `Brain::set_episode_summary()` to persist.
4. Scheduling: summary generation runs after description generation, batched per episode.

### Quality requirements

The Librarian's summarization prompt would need:
- **Exhaustive entity extraction**: Every person, place, project, pet, event mentioned in any turn must appear in the summary.
- **Embedded reference preservation**: The prompt must explicitly instruct: "Include entities and events mentioned in passing, as context, or as examples -- not just the session's primary topic."
- **This is the same quality requirement as per-memory descriptions**, applied at session granularity.

### Coordination cost

The Librarian already has a description-writing pipeline (item #8 dependency). Adding episode summaries would be an incremental extension -- not a new system. Estimated additional Permagent effort: 4-6h for the summarization prompt, scheduling, and API integration. Total Spectral + Permagent effort: ~1.5 weeks.

---

## Section 7 -- Summary Table

| Aspect | Finding |
|--------|---------|
| **Documented failures addressed by L2 summaries** | 0 uniquely (all also addressed by item #8) |
| **Documented failures addressed by item #8** | 2-4 (RETRIEVAL_MISS + possibly AMBIGUOUS) |
| **Incremental value of L2 over item #8** | None demonstrated |
| **Compression risk** | Real: summaries may omit embedded references, the hardest failure mode |
| **Summarizer attention problem** | Same LLM attention patterns as actor; no reason to expect better embedded-reference capture |
| **Effort** | ~1.5 weeks (Spectral + Permagent) vs 2-3h for item #8 |
| **Recommendation** | **Defer** until item #8 ships and demonstrates a ceiling |
| **Conditions to revisit** | (1) Item #8 ceiling identified, (2) structured summary format validated, (3) production usage data, (4) actor attention solved |

**Investigation conclusion**: L2 episode summaries are architecturally coherent but operationally redundant with item #8 for the documented failure set. The compression-vs-embedded-reference tension is a structural concern, not a solvable implementation detail. Defer until the conditions above are met.
