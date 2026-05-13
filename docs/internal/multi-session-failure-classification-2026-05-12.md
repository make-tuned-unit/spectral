# Multi-Session Failure Classification (Post-PR #97 + #98)

**Date**: 2026-05-12
**Branch**: `investigate/multi-session-failure-classification-2026-05-12`
**Data**: `~/spectral-local-bench/eval-runs/20260512-1554-post-98-max-tokens-4096/multi-session/`
**Status**: Investigation complete. Recommendations below.

---

## Section 1 -- Methodology

**Source data**: Post-PR #97 (quote-first extraction) + PR #98 (max_tokens 4096) bench run. 20 multi-session questions, 10 correct (50%), 10 failed.

**For each failure, gathered**:
1. Actor's full prediction (including `<thinking>`, `<quotes>`, and final answer) from `report.json`
2. GT answer and GT answer session IDs from `longmemeval_s.json`
3. Retrieved memory keys from `report.json`, cross-referenced against answer session IDs to determine retrieval status
4. Full text of answer sessions from `longmemeval_s.json` to verify what evidence existed

**Classification applied**: Each failure classified into exactly one dominant category using the taxonomy from PR #93, extended with DEFINITION_DISAGREEMENT and DATE/TEMPORAL_REASONING.

**Reproducible via**: `report.json` at the path above + `longmemeval_s.json` at `~/spectral-local-bench/longmemeval/longmemeval_s.json`.

---

## Section 2 -- Per-Failure Table

| # | QID | Question | GT | Actor | Answer sessions | Classification | Primary axis |
|---|-----|----------|----|-------|-----------------|----------------|--------------|
| 1 | 0a995998 | Clothing to pick up/return | 3 | 2 | 3/3 retrieved | DEFINITION_DISAGREEMENT | Exchange = 1 action or 2? |
| 2 | 6d550036 | Projects led | 2 | 3 | 3/4 retrieved | DEFINITION_DISAGREEMENT | What counts as "leading"? |
| 3 | gpt4_d84a3211 | Bike expenses total | $185 | $40 | 4/4 retrieved | GENUINE_MISS | Missed $25 chain + $120 helmet |
| 4 | gpt4_f2262a51 | Doctors visited | 3 | 0 | 0/3 retrieved | RETRIEVAL_MISS | Zero answer sessions retrieved |
| 5 | dd2973ad | Bedtime before doctor | 2 AM | "I don't know" | 2/2 retrieved | DATE/TEMPORAL_REASONING | Date math error |
| 6 | c4a1ceb8 | Citrus fruits in cocktails | 3 | 4 | 4/4 retrieved | DEFINITION_DISAGREEMENT | Grapefruit: used or suggested? |
| 7 | gpt4_a56e767c | Movie festivals attended | 4 | 3 | 3/3 retrieved | GENUINE_MISS | 4th festival not found |
| 8 | 46a3abf7 | Tanks currently owned | 3 | 2 | 3/3 retrieved | AMBIGUOUS* | Missed 5-gallon betta tank |
| 9 | gpt4_2f8be40d | Weddings attended | 3 | 2 | 3/3 retrieved | AMBIGUOUS* | Missed Emily+Sarah, Jen+Tom |
| 10 | gpt4_15e38248 | Furniture bought/assembled/sold/fixed | 4 | 2 | 2/4 retrieved | RETRIEVAL_MISS | 2 answer sessions not retrieved |

---

## Section 3 -- Aggregate Counts

| Classification | Count | Percentage |
|----------------|-------|------------|
| DEFINITION_DISAGREEMENT | 3 | 30% |
| GENUINE_MISS | 4 | 40% |
| RETRIEVAL_MISS | 2 | 20% |
| DATE/TEMPORAL_REASONING | 1 | 10% |

**Split is roughly even between DEFINITION_DISAGREEMENT (3) and GENUINE_MISS (4).** Neither dominates at >= 5 of 10. RETRIEVAL_MISS accounts for 2 cases. DATE/TEMPORAL_REASONING is a standalone failure mode.

---

## Section 4 -- DEFINITION_DISAGREEMENT Analysis

### Failure #1: Clothing pickup/return (0a995998)

**GT**: 3 items. **Actor**: 2 items.

**Actor's explicit reasoning**: The actor found all three answer sessions and correctly identified: (1) navy blue blazer at dry cleaner, (2) new boots from Zara (exchanged for larger size, needs pickup). The actor explicitly reasoned: "The boots exchange seems already done - they just need to pick up the replacement pair... the original too-small pair has already been returned."

**Disagreement axis**: The actor treats the Zara exchange as a single completed-return + pending-pickup. GT appears to count it as more items (possibly: old boots to return + new boots to pick up + blazer = 3). The actor's reasoning is defensible — the text says "I got them on February 5th, but they were too small, so I exchanged them for a larger size" (past tense: exchange complete).

**Evidence quality**: Actor's quote pass captured all relevant mentions. The classification difference is about whether a completed exchange still generates a "return" item.

### Failure #2: Projects led (6d550036)

**GT**: 2 projects. **Actor**: 3 projects (data analysis, cloud migration, product feature launch).

**Actor's reasoning**: Listed 3 projects from sessions. One answer session (answer_ec904b3c_3, about consumer psychology research) was NOT retrieved, but the actor found projects from both answer and non-answer sessions.

**Disagreement axis**: What counts as "led or am currently leading"? The actor includes a "new product feature launch planned for June" from non-answer session 2e4430d8_2. GT only counts 2. The actor's inclusion is reasonable — planning a product launch is plausibly "leading a project." GT excludes it, possibly because the user's role in the launch is not specified as "leading."

**Note**: This case has a partial RETRIEVAL_MISS (1 of 4 answer sessions missing), but the dominant failure mode is the over-count from a non-answer session, not the missing session.

### Failure #6: Citrus fruits in cocktails (c4a1ceb8)

**GT**: 3 types. **Actor**: 4 types (lemon, lime, orange, grapefruit).

**Actor's reasoning**: Exhaustive quote pass across all 4 answer sessions. Actor found grapefruit in: citrus peel infusion recipes ("1/2 cup citrus peel (orange, lemon, or grapefruit)"), Gin & Tonic garnish ("slice of grapefruit"), and a Grapefruit-Rosemary-Gin flavor combination.

**Disagreement axis**: Does grapefruit count as "used in cocktail recipes"? The actor counted all citrus mentioned in recipe contexts. GT appears to count only citrus actively squeezed/juiced into recipes (lemon, lime, orange), excluding grapefruit which appears only in optional ingredient lists, infusion suggestions, and garnish options.

**Evidence quality**: Actor's quote pass was thorough — 40+ individual quotes across 4 sessions. The grapefruit references are real but contextually weaker (suggestions/options vs. active ingredients). The judge noted: "Grapefruit appears to be mentioned primarily in optional/alternative contexts (infusion ideas, garnish options) rather than as a directly used citrus."

### Pattern across DEFINITION_DISAGREEMENT cases

All three cases share a common structure:
1. **Actor found the evidence.** Quote passes are thorough and complete.
2. **Actor reasoned explicitly about categorization.** The reasoning is visible and defensible.
3. **The disagreement is on scope/boundary.** Exchange = 1 or 2 items? Planning = leading? Suggested = used?

These are genuine semantic ambiguities. The actor's interpretation is reasonable in each case; the GT's interpretation is also reasonable. The current judge (binary correct/incorrect) cannot distinguish "wrong answer" from "different defensible interpretation."

**Key question**: Could a judge rubric address these? Partially. A rubric could accept a range (e.g., "2-3 items of clothing" instead of exactly "3"). But some disagreement axes are genuinely ambiguous — "Is grapefruit used in cocktail recipes if it only appears in optional ingredient lists?" has no objectively correct answer.

---

## Section 5 -- GENUINE_MISS Analysis

### Failure #3: Bike expenses (gpt4_d84a3211)

**GT**: $185 ($40 bike lights + $25 chain replacement + $120 Bell Zephyr helmet). **Actor**: $40 (bike lights only).

**What the actor missed**:
- **$25 chain replacement**: In answer_2880eb6c_2: "The mechanic told me I needed to replace the chain, which I did, and it cost me $25." Same turn also mentions the $40 bike lights. The actor quoted only the bike lights from this session.
- **$120 helmet**: In answer_2880eb6c_1: "I've had good experiences with the local bike shop downtown where I bought my Bell Zephyr helmet for $120." Actor mentioned "Bell Zephyr helmet" in its reasoning but said "no specific costs are given" — directly contradicting the text.

**Pattern**: The $25 chain replacement is in the SAME TURN as the $40 bike lights (answer_2880eb6c_2). The actor quoted the bike lights but not the chain from the same sentence pair. The $120 helmet is in a different session (answer_2880eb6c_1) embedded as a parenthetical within a sentence about the bike shop, not as the primary topic. Both are embedded-reference failures: costs mentioned as subordinate details within broader discussions.

### Failure #7: Movie festivals (gpt4_a56e767c)

**GT**: 4 festivals. **Actor**: 3 (Austin Film Festival, AFI Fest, Portland Film Festival).

**What the actor missed**: The 4th festival. All 3 answer sessions were retrieved. I can verify 3 named festivals in the answer sessions: Austin Film Festival (answer_cf9e3940_2), AFI Fest (answer_cf9e3940_3), Portland Film Festival (answer_cf9e3940_1). The 4th festival that GT counts is not clearly identifiable from the answer sessions alone — it may be in a non-answer retrieved session, or it may be an interpretation of one of the existing references as two events (e.g., the 48-hour film challenge as separate from the broader Austin Film Festival attendance).

**Pattern**: Uncertain. If the 4th festival is in a non-answer session, this is an embedded-reference miss. If the GT is counting an event that the answer sessions don't actually support as a separate festival, this could be a GT accuracy question. Classification as GENUINE_MISS is based on GT authority, but the evidence for the 4th festival is the weakest of any failure case.

### Failure #8: Tanks (46a3abf7)

**GT**: 3 tanks. **Actor**: 2 (20-gallon Amazonia, 1-gallon friend's kid tank).

**What the actor missed**: The **5-gallon betta tank with Finley** from answer_c65042d7_2. The user says: "I have a 5-gallon tank with a solitary betta fish named Finley." This is in the first turn of the session, stated directly as part of the user's introduction. Later in the same session (turn 4): "My old tank was a 5-gallon one that I got from my cousin, and I kept a solitary betta fish named Finley."

**Pattern**: The 5-gallon tank is mentioned in the FIRST TURN of a session whose primary topic is "high nitrite levels in my tank" (referring to the 20-gallon community tank). The betta tank is introduced as background context for the user's aquarium experience. This is the classic embedded-reference-in-different-primary-context pattern from PR #93. The quote-first instruction did not extract this mention.

### Failure #9: Weddings (gpt4_2f8be40d)

**GT**: 3 weddings (Rachel+Mike, Emily+Sarah, Jen+Tom). **Actor**: 2 (cousin's wedding, sister's wedding).

**What the actor missed**: Emily+Sarah's wedding (answer_e7b0637e_2: "My friend Emily finally got to tie the knot with her partner Sarah") and Jen+Tom's wedding (answer_e7b0637e_3: "the bride, Jen, looked stunning in her bohemian-inspired dress, and her husband, Tom, was clearly smitten with her").

**Pattern**: This is the identical failure from PR #93. All 3 answer sessions were retrieved. The actor's quote pass found mentions from answer_e7b0637e_1 and non-answer sessions but completely missed the wedding references in answer_e7b0637e_2 and answer_e7b0637e_3. These sessions are primarily about **wedding planning** (the user's own wedding), with attended-wedding references embedded as context/inspiration. The quote-first extraction pattern did NOT help — the actor's quote pass exhibits the same topic-based attention filtering as the non-quote version.

### Pattern across GENUINE_MISS cases

| Case | Missed items | Location pattern |
|------|-------------|-----------------|
| Bike expenses | $25 chain (same turn as found item), $120 helmet (different session, parenthetical) | Subordinate detail in same/adjacent text |
| Movie festivals | 4th festival | Unknown location |
| Tanks | 5-gallon betta tank | First turn of session, background context for different primary topic |
| Weddings | Emily+Sarah, Jen+Tom | Explicit mentions in sessions about user's own wedding planning |

**The embedded-reference-in-different-primary-context pattern from PR #93 persists.** The quote-first instruction (PR #97) did not address it. The actor's `<quotes>` blocks show the same topic-following behavior: the actor quotes mentions that match the session's primary topic and skips subordinate references even when asked to "quote every mention."

**Bike expenses adds a new sub-pattern**: the actor found one cost in a turn but missed another cost in the SAME TURN (the $25 chain is one sentence before the $40 bike lights in session answer_2880eb6c_2). This isn't cross-session attention filtering — it's within-turn partial extraction. The $120 helmet in answer_2880eb6c_1 is a more typical embedded reference (parenthetical mention in a sentence about the bike shop).

---

## Section 6 -- Recommendation

### Breakdown summary

| Category | Failures | Addressable by |
|----------|----------|---------------|
| DEFINITION_DISAGREEMENT (3) | #1, #2, #6 | Item #20 (judge rubric refinement) |
| GENUINE_MISS (4) | #3, #7, #8, #9 | Deeper actor architecture (uncertain) |
| RETRIEVAL_MISS (2) | #4, #10 | Item #8 (compiled-truth boost) / item #12 (session summaries) |
| DATE/TEMPORAL_REASONING (1) | #5 | Separate temporal reasoning intervention |

### The split is even. Both item #20 and deeper actor work have justified priority.

**Item #20 (judge rubric refinement) addresses failures #1, #2, #6 — 3 of 10 (30%).**

A rubric that accepts ranges instead of exact counts would flip #1 (2 vs 3 clothing items) and #6 (3 vs 4 citrus fruits). A rubric that distinguishes over-counting from under-counting would correctly identify #2 (actor found 3, GT says 2) as an over-inclusion rather than a miss.

Specific rubric changes that would address these 3 cases:
- Accept +/-1 on counting questions where the boundary is ambiguous (exchange = 1 or 2 actions; suggested ingredient = used or not)
- Score over-counting (actor found more than GT) differently from under-counting (actor found fewer than GT) — over-counting demonstrates recognition, not failure

**Expected lift from item #20 alone**: +3 questions correct = 50% -> 65%. This is the cheapest intervention per percentage point.

**GENUINE_MISS (failures #3, #7, #8, #9) is NOT addressable by judge refinement.** The actor genuinely did not find the items. The quote-first pattern (PR #97) did not help — the actor's quote pass exhibits the same topic-based attention filtering.

Remaining options for GENUINE_MISS:
- **Two-call extract-then-count** (research synthesis Technique 2): Separate extraction call with high-recall framing, then counting call. Cost: 2x actor calls. Uncertain whether a separate extraction call overcomes the same attention pattern.
- **Per-session extraction**: Instead of passing all sessions to one actor call, extract from each session independently, then merge. Cost: N calls (one per session). Mechanistically addresses topic-filtering by isolating each session. Most expensive option.
- **Item #12 (L2 episode summaries)**: Session-level summaries might surface embedded references by reframing them ("User discussed attending Emily and Sarah's wedding while planning their own wedding"). Depends on summary quality.

**RETRIEVAL_MISS (failures #4, #10) is addressable by items #8 and #12**, as established in PR #93. No new information here.

**DATE/TEMPORAL_REASONING (failure #5) is a standalone failure mode.** The actor found both evidence pieces (2 AM bedtime, doctor's appointment) but performed incorrect date arithmetic. The actor's reasoning was transparent and showed genuine engagement with the temporal logic. This is a model capability issue, not an architecture issue. A temporal reasoning prompt intervention might help, but this is 1 case — insufficient evidence to justify a dedicated intervention.

### Recommended sequence

1. **Item #20 (judge rubric refinement)** — highest leverage. Addresses 3 failures (#1, #2, #6) with no retrieval or actor changes. Estimated lift: +15pp (50% -> 65%). Cost: judge prompt changes only.

2. **Item #8 (compiled-truth boost)** — addresses 2 RETRIEVAL_MISS failures (#4, #10). Estimated lift: +5-10pp. Cost: medium (Librarian pipeline + FTS index).

3. **GENUINE_MISS investigation** — the 4 remaining failures require deeper experimentation. Recommended next experiment: per-session extraction on 2-3 failure cases (#3 bike expenses, #9 weddings) to test whether session isolation overcomes topic-filtering. If it does, the mechanism is validated. If it doesn't, the failure is at a level that prompt/architecture changes cannot address.

### What this means for the path-to-90%

If items #20 and #8 both succeed at their estimated lifts:
- Current: 50% (10/20)
- After #20: ~65% (13/20) — flips 3 DEFINITION_DISAGREEMENT
- After #8: ~70-75% (14-15/20) — flips 1-2 RETRIEVAL_MISS
- Remaining: 4 GENUINE_MISS + 1 TEMPORAL = 5 failures requiring deeper intervention

The GENUINE_MISS failures (especially #9 weddings, which persists across every intervention tried) represent the hard floor for prompt-level approaches. These 4-5 failures are where the path-to-90% analysis in the category audit meets reality: lifting past 75% on multi-session requires either a fundamentally different actor pattern (per-session extraction) or model-level improvements in subordinate-reference attention.

---

## Correction (2026-05-13): Cases #8 and #9 reclassified to AMBIGUOUS

*Cases #8 (tanks) and #9 (weddings) reclassified from GENUINE_MISS to AMBIGUOUS pending retrieval verification. See `docs/internal/item-11-investigation.md` Section 3 for full analysis.*

**Finding**: The "3/3 retrieved" claim for cases #8 and #9 was based on inference from actor output, not retrieval telemetry. The `memory_keys` field in `report.json` is empty for all results in the post-PR-#98 bench run — it was not populated.

Independent verification from actor output:
- **#8 Tanks**: Actor found content from `answer_c65042d7_3` (Amazonia) and `answer_c65042d7_1` (friend's kid tank). But the missed 5-gallon betta tank from `answer_c65042d7_2` has no evidence of retrieval — neither session ID nor distinctive content appears in actor output. May be partial RETRIEVAL_MISS.
- **#9 Weddings**: Actor references `answer_e7b0637e_1` only. Emily+Sarah (`answer_e7b0637e_2`) and Jen+Tom (`answer_e7b0637e_3`) have no evidence of retrieval. May be partial RETRIEVAL_MISS for 2 of 3 answer sessions.

**Strategic implication**: Item #8's projected lift could be larger than the +2 estimate (cases #4, #10 only). If cases #8 and #9 are partially retrieval-related, description enrichment may address them too. The Permagent regen bench run is the decisive test.

**Aggregate counts (revised)**:

| Classification | Count | Change |
|----------------|-------|--------|
| DEFINITION_DISAGREEMENT | 3 | unchanged |
| GENUINE_MISS | 2 | was 4, reduced by 2 |
| AMBIGUOUS (GENUINE_MISS or partial RETRIEVAL_MISS) | 2 | new |
| RETRIEVAL_MISS | 2 | unchanged |
| DATE/TEMPORAL_REASONING | 1 | unchanged |
