# Multi-Session Failure Investigation

**Date**: 2026-05-12
**Branch**: `investigate/multi-session-failure-modes`
**Data**: `~/spectral-local-bench/eval-runs/20260512-1024-post-86-90/`

---

## Section 1 -- Methodology

Retrieved memories text is not preserved per-question in the bench work directory (only `report.json` + `checkpoint.json` + `run.log`). Methodology used:

1. For each failure, extracted `retrieved_memory_keys` from `report.json` and `answer_session_ids` from the LongMemEval source dataset (`longmemeval_s.json`).
2. Checked whether each answer session's keys appear as prefixes in the retrieved memory key set. If answer session keys are present, the session was RETRIEVED; otherwise MISSING.
3. For ACTOR_MISS cases, read the full session content from LongMemEval and searched for GT item names/terms to identify exactly what text the actor had available and missed.

Reproducible via the LongMemEval dataset at `~/spectral-local-bench/longmemeval/longmemeval_s.json` and the report at the path above.

## Section 2 -- Per-Failure Table

| # | Question | GT | Predicted | Answer sessions retrieved | Classification |
|---|----------|-----|-----------|--------------------------|----------------|
| 1 | Clothing to pick up/return | 3 | 2 | 3/3 | ACTOR_MISS |
| 2 | Days on camping trips | 8 | 0 | 3/3 | ACTOR_MISS + SESSION_USER_CONFUSION |
| 3 | Bike expenses total | $185 | $40 | 4/4 | ACTOR_MISS |
| 4 | Different doctors visited | 3 | 0 | 0/3 | RETRIEVAL_MISS |
| 5 | Citrus fruits in cocktails | 3 | 2 | 4/4 | ACTOR_MISS |
| 6 | Movie festivals attended | 4 | 3 | 3/3 | ACTOR_MISS |
| 7 | Tanks currently owned | 3 | 2 | 3/3 | ACTOR_MISS |
| 8 | Weddings attended | 3 | 2 | 3/3 | ACTOR_MISS |
| 9 | Furniture bought/assembled/sold/fixed | 4 | 2 | 4/4 | ACTOR_MISS |
| 10 | Baking events in 2 weeks | 4 | 3 | 4/4 | ACTOR_MISS |

## Section 3 -- Aggregate Breakdown

| Type | Count | Percentage |
|------|-------|------------|
| ACTOR_MISS | 9 | 90% |
| RETRIEVAL_MISS | 1 | 10% |
| SESSION_USER_CONFUSION (subset of ACTOR_MISS) | 1 | 10% |

**ACTOR_MISS dominates overwhelmingly.** In 9/10 failures, all answer sessions were retrieved. The actor had the evidence and failed to extract the items from it.

The 1 RETRIEVAL_MISS (#4, doctors) is complete -- zero answer sessions retrieved. The query "How many different doctors did I visit?" returned 88 memories but none from the answer sessions (answer_55a6940c_1/2/3), which discuss a primary care physician, ENT specialist, and dermatologist.

## Section 4 -- Validation Walkthrough

### Walkthrough 1: Weddings (Failure #8)

**GT**: 3 weddings -- Rachel and Mike, Emily and Sarah, Jen and Tom.
**Predicted**: 2 -- cousin's wedding at vineyard, sister's wedding.

**All 3 answer sessions were retrieved.** Here is exactly what appears in each:

**Session answer_e7b0637e_1** (RETRIEVED):
> [user] "I've been to a few weddings recently and one of them was my cousin's wedding at a vineyard in August"
> [user] "My cousin Rachel's wedding at the vineyard was just perfect, she looked stunning in her lace gown"
> [user] "My cousin Emily's wedding in the city was really lovely"

**Session answer_e7b0637e_2** (RETRIEVED):
> [user] "I just got back from my college roommate's wedding in the city, and it was beautiful -- they had a rooftop ceremony overlooking the skyline"
> [user] "My friend Emily finally got to tie the knot with her partner Sarah, and it was amazing to see them so happy"

**Session answer_e7b0637e_3** (RETRIEVED):
> [user] "I just got back from a friend's wedding last weekend, and it was amazing -- the bride, Jen, looked stunning in her bohemian-inspired dress, and her husband, Tom, was clearly smitten with her"
> [user] "the rustic barn where my friend Jen got married last weekend"

**What the actor found**: Rachel's wedding (session 1: "cousin's wedding at a vineyard"), and a "sister's wedding" from non-answer session 81b971b8_2.

**What the actor missed**: Emily and Sarah's wedding (session 2: "My friend Emily finally got to tie the knot with her partner Sarah") and Jen and Tom's wedding (session 3: "the bride, Jen...her husband, Tom").

**Critical finding**: These are NOT indirect references. "Emily finally got to tie the knot with Sarah" and "the bride, Jen, looked stunning" are direct, explicit statements about attending weddings with named individuals. The actor missed them.

**Why**: All three answer sessions are primarily about the user **planning their own wedding**. The attended-wedding references are embedded as context and inspiration within a wedding-planning conversation. The actor tracked the primary topic (planning) and did not register subordinate mentions of past weddings as counting toward "weddings I attended."

**This reframes the failure mode**: it's not "indirect reference recognition" but "embedded reference in a different primary context." The items are explicitly stated but are subordinate clauses in conversations about a different primary topic.

### Walkthrough 2: Re-scan effectiveness

The proposed "completion re-check" instruction ("after listing items, re-scan the sessions to check if you missed any") has a structural problem: if the actor missed explicit references on first pass because they were embedded in a different primary context, a second pass using the same recognition strategy will miss them again.

What would mechanistically work: telling the actor that counted items may appear as **passing mentions, context, or inspiration within conversations about other topics** -- not just as the primary topic of a session. This changes the recognition criteria, not the number of passes.

Concrete instruction: "Items may be mentioned in passing -- as context, examples, or inspiration within a conversation about something else. A session about wedding planning might mention weddings you attended. A session about cooking might mention ingredients you bought. Scan for the counted item even when the session's primary topic is different."

### Walkthrough 3: Preference RETRIEVAL_MISS cases

Three preference failures had answer sessions completely absent from retrieval:

| # | Query | Answer session topic | Vocabulary gap |
|---|-------|---------------------|----------------|
| 3 | "serve for dinner...homegrown ingredients" | "fresh basil and mint" + "companion plants for tomatoes" | "homegrown ingredients" vs "basil", "mint", "tomatoes" |
| 4 | "trouble with battery life on my phone" | "portable power bank" + "wireless charging pad" | "battery life" vs "power bank", "charging pad" |
| 7 | "new coffee creamer recipe" | "making my own flavored creamer with almond milk, vanilla extract, and honey" | "coffee creamer recipe" vs "flavored creamer", "almond milk" |

**All three are the same pattern**: semantic overlap but low lexical overlap. The query and the answer session describe the same topic using different words. FTS (BM25) requires term overlap; these queries share few terms with their answer sessions.

**Compiled-truth boost (backlog item #8) would address these directly.** If the Librarian had written descriptions like:
- "User grows cherry tomatoes and herbs including basil and mint in their garden"
- "User purchased a portable power bank for travel"
- "User makes homemade coffee creamer with almond milk, vanilla, and honey"

...and those descriptions were indexed in FTS, the vocabulary gap would be bridged. "Homegrown ingredients" would match "grows...garden"; "battery life" would appear in a description about the power bank; "coffee creamer recipe" would match "homemade coffee creamer."

This is useful for backlog prioritization: once Permagent's Librarian populates descriptions in production, item #8 (compiled-truth boost) should improve preference retrieval on these vocabulary-gap patterns.

## Section 5 -- Recommendation

### Primary intervention: actor prompt refinement (addresses 9/10 multi-session, 5/8 preference)

The ACTOR_MISS pattern is "embedded reference in different primary context" -- not cognitive overload, not indirect reference, not mid-task abandonment. The prompt refinement should:

1. **Add embedded-reference instruction** to counting_enumerate.md: "Items may appear as passing mentions within conversations about other topics. A session about wedding planning may mention weddings attended. Scan for the counted item even when the session's primary topic is different."

2. **Add session-user clarity** to counting_enumerate.md: "All sessions are about you. Different session IDs are different conversations, not different people." (Addresses failure #2 directly.)

3. **Drop the "re-scan" proposal.** A second pass with the same recognition criteria won't find what the first pass missed. Instead, the embedded-reference instruction changes what the actor looks for on its single pass.

4. **Refine preference.md** for the 5 ACTOR_MISS cases per the original PR #91 analysis (stated preferences over implicit signals, anti-generic instruction).

### Secondary: retrieval (addresses 1/10 multi-session, 3/8 preference)

1. **Doctors RETRIEVAL_MISS** (#4): query "How many different doctors did I visit?" failed to retrieve sessions about a primary care physician, ENT specialist, and dermatologist. The session content discusses these visits by doctor name (Dr. Smith, Dr. Patel, Dr. Lee) without using the word "doctor" prominently. FTS didn't match. This specific case would benefit from session summaries (backlog #13) or description text in FTS (backlog #10).

2. **Preference RETRIEVAL_MISS** (3 cases): all vocabulary-gap failures. Compiled-truth boost (backlog #8) is the natural intervention once descriptions are populated.

### Not recommended

- Increasing K: the 9 ACTOR_MISS cases already had all answer sessions retrieved. More candidates won't help and would increase noise.
- L2 episodes (backlog #12): not relevant -- the failures are at the actor recognition level, not the retrieval grouping level.
- Multi-step actor patterns: premature. Single-pass with better recognition criteria should be tried first.

## Diagnosis evolution

This investigation went through three diagnostic hypotheses:

1. **Cognitive overload** (PR #91 v1): Actor abandons enumeration mid-task because instruction 1 packs too many subtasks. **Refuted** -- the actor completes enumeration and states confident counts. It doesn't abandon.

2. **Indirect-reference recognition** (PR #92 triage): Actor misses items described with different words than the question uses. **Partially correct but imprecise** -- the weddings walkthrough showed the references are actually direct and explicit ("Emily finally got to tie the knot with Sarah"), not indirect.

3. **Embedded-reference-in-different-primary-context** (this doc): The correct framing. Items are explicitly stated but are subordinate clauses in conversations about a different primary topic. The actor tracks the primary topic and doesn't register subordinate mentions. **This is the diagnosis that drives the prompt refinement in PR #91 v2.**

The "indirect-reference recognition" framing from PR #92 is superseded by this document's "embedded-reference" framing. Both PRs remain useful -- PR #92 established the 9/10 ACTOR_MISS breakdown, this PR identified the specific mechanism.
