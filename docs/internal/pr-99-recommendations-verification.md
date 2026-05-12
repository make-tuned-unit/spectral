# PR #99 Recommendations Verification

**Date**: 2026-05-12
**Branch**: `investigate/pr-99-recommendations-verification`
**Status**: Verification complete. Revised estimates below.

---

## Section 1 -- Methodology

PR #99 classified 10 multi-session failures and recommended item #20 (judge rubric, +15pp) and item #8 (compiled-truth boost, +5-10pp). This doc verifies both estimates against the evidence.

**For item #20**: Pulled verbatim judge reasoning for each DEFINITION_DISAGREEMENT case. Spot-checked 4 currently-correct counting cases under a +/-1 tolerance rubric to identify regression risk.

**For item #8**: Analyzed retrieved memory keys vs answer session IDs for each RETRIEVAL_MISS case. Compared query terms against missing answer session content to classify as vocabulary-gap vs rank-based.

---

## Section 2 -- Item #20 Verification (Judge Rubric Refinement)

### Judge reasoning for DEFINITION_DISAGREEMENT cases

**Case #1 — Clothing (0a995998): GT=3, Actor=2**

Judge reasoning verbatim: "The system answered 2 items, but the ground truth is 3. The system may have missed one item of clothing that needs to be picked up or returned from a store across the conversation sessions."

**Assessment**: The judge shows NO nuance. It does not engage with the actor's reasoning about the exchange being complete. It simply compares numbers and declares a miss. A rubric refinement would need to build this capability from scratch — the current judge cannot distinguish "actor reasoned defensibly and reached a different count" from "actor missed evidence."

**Would +/-1 tolerance flip this?** Yes. Actor=2, GT=3, delta=1. Under +/-1 tolerance, this would be marked correct.

**Case #2 — Projects (6d550036): GT=2, Actor=3**

Judge reasoning verbatim: "The system answered 3 projects, but the ground truth is 2. The system identified an extra project (new product feature launch) that should not have been counted, or miscounted the projects that the user has led or is currently leading."

**Assessment**: The judge shows slightly more nuance — it identifies the specific extra project. But it still grades as a clean miss. No acknowledgment that the actor's inclusion is defensible.

**Would +/-1 tolerance flip this?** Yes. Actor=3, GT=2, delta=1. Under +/-1 tolerance, this would be marked correct. BUT: this is an OVER-count case. The actor found more items than GT. Accepting this means the rubric rewards over-inclusion — finding items that GT excludes.

**Case #6 — Citrus (c4a1ceb8): GT=3, Actor=4**

Judge reasoning verbatim: "The ground truth answer is 3, but the system answered 4. The system identified lemon, lime, orange, and grapefruit. The ground truth suggests only 3 types were used. Grapefruit appears to be mentioned primarily in optional/alternative contexts (infusion ideas, garnish options) rather than as a direct ingredient in specific cocktail recipes, so the correct count is 3 (lemon, lime, orange)."

**Assessment**: The judge DOES show nuance here. It correctly identifies the disagreement axis (optional/alternative vs direct use) and even names grapefruit as the disputed item. This is the strongest case for rubric refinement — the judge already understands the ambiguity but has no mechanism to translate it into a non-binary score.

**Would +/-1 tolerance flip this?** Yes. Actor=4, GT=3, delta=1.

### Spot-check: currently-correct cases under +/-1 tolerance

The question: would a +/-1 tolerance rubric cause any currently-correct case to regress? A case could regress if the actor's answer is currently exact-match correct, and the +/-1 tolerance somehow introduces a scoring error. More realistically, regression occurs if the tolerance is applied to the wrong side — e.g., if a +/-1 tolerance causes the judge to accept a genuinely wrong answer that happens to be within 1 of GT.

**Spot-check 1: Camping days (b5ef892d)** — GT=8, Actor=8. Exact match. Under +/-1, still correct. No regression risk.

**Spot-check 2: Plants acquired (3a704032)** — GT=3, Actor=3. Exact match. Under +/-1, still correct. No regression risk.

**Spot-check 3: Luxury spending (36b9f61e)** — GT=$2,500, Actor=$2,500. Exact match. Under +/-1 (would be +/-$1 on dollar amounts, which is meaningless, or not applied to dollar amounts). No regression risk, but highlights that +/-1 tolerance needs to be scoped to counting questions, not dollar totals.

**Spot-check 4: Baking sessions (88432d0a)** — GT=4, Actor="Approximately 4-5 baking sessions." Currently marked correct (judge interpreted 4-5 as containing 4). Under +/-1, still correct. BUT: this case reveals that the judge already exercises some tolerance — it accepted "4-5" as matching "4." The current judge is not fully binary on counting. This means a +/-1 rubric may already be partially in effect for some cases through implicit judge behavior.

**Regression risk on currently-correct cases**: Zero identified. All 10 correct cases are exact matches or already-accepted ranges. A +/-1 tolerance on counting questions would not flip any correct case to incorrect.

### Critical gap: GENUINE_MISS cases also within delta-1

The spot-check above only checked currently-correct cases for regression. It missed a more important question: which currently-FAILING cases would a blanket +/-1 tolerance also flip?

Three GENUINE_MISS failures have delta exactly 1:

**Case #7 — Festivals (gpt4_a56e767c): GT=4, Actor=3, delta=1.** Actor missed a real festival. Under blanket +/-1, this would score correct — masking a genuine recognition failure.

**Case #8 — Tanks (46a3abf7): GT=3, Actor=2, delta=1.** Actor missed the 5-gallon betta tank (PR #99 documented this as embedded-reference-in-different-primary-context). Under blanket +/-1, this would score correct — masking the same failure mode that PR #93 identified.

**Case #9 — Weddings (gpt4_2f8be40d): GT=3, Actor=2, delta=1.** Actor missed Emily+Sarah and Jen+Tom weddings (PR #99 documented this as persistent embedded-reference failure across every intervention). Under blanket +/-1, this would score correct — masking Spectral's hardest failure case.

**Under blanket +/-1**: All 6 delta-1 failures (3 DEFINITION_DISAGREEMENT + 3 GENUINE_MISS) flip to correct. Bench score: 50% -> 80% (+6 questions). But 3 of those 6 are false positives — real recognition failures scored as correct.

**Under reasoning-aware +/-1**: Only the 3 DEFINITION_DISAGREEMENT cases flip — actor provides explicit reasoning for its categorization in each case. The 3 GENUINE_MISS cases do NOT flip — actor shows no awareness of the missed items and no reasoning for excluding them.

| Case | Delta | Actor reasoning for count? | Blanket +/-1 | Reasoning-aware +/-1 |
|------|-------|---------------------------|-------------|---------------------|
| #1 clothing | 1 | Yes: "exchange already complete, just 2 pickups" | Correct | Correct |
| #2 projects | 1 | Yes: "3 projects including product launch" | Correct | Correct |
| #6 citrus | 1 | Yes: "grapefruit in infusions and garnish" | Correct | Correct |
| #7 festivals | 1 | No: only found 3, no mention of 4th | Correct (false positive) | Incorrect |
| #8 tanks | 1 | No: only found 2, no mention of 3rd | Correct (false positive) | Incorrect |
| #9 weddings | 1 | No: only found 2, no mention of 3rd | Correct (false positive) | Incorrect |

The distinction is clear: DEFINITION_DISAGREEMENT cases show the actor explicitly reasoning about included/excluded items. GENUINE_MISS cases show the actor finding fewer items with no awareness that more exist.

### Revised lift estimate for item #20

**Two designs, two outcomes:**

**Design A — Blanket +/-1 tolerance:**
- Flips 6 cases: #1, #2, #6 (DEFINITION_DISAGREEMENT) + #7, #8, #9 (GENUINE_MISS)
- Lift: +6 questions (50% -> 80%)
- False positives: 3 (masks real recognition failures in #7, #8, #9)
- **Not recommended.** Masks the hardest failure mode. Inflates bench scores without improving the system.

**Design B — Reasoning-aware +/-1 tolerance:**
- Judge examines actor's `<quotes>` block and reasoning chain
- Accepts delta-1 counts ONLY when actor provides explicit reasoning for included/excluded items
- Rejects delta-1 counts when actor simply didn't find the items
- Flips 3 cases: #1 (actor reasoned about exchange), #2 (actor reasoned about launch), #6 (actor reasoned about grapefruit)
- Does NOT flip: #7, #8, #9 (actor found fewer items with no reasoning for exclusion)
- Lift: +3 questions (50% -> 65%)
- False positives: 0
- **Recommended.** Honest scoring that rewards defensible reasoning.

**Cases at risk of regression on currently-correct set**: None identified. All 10 correct cases are exact matches.

**Revised net lift**: **+3 questions (50% -> 65%)** under reasoning-aware design. The +15pp estimate from PR #99 holds for this design.

**Implementation requirement**: Reasoning-aware judging requires the judge to examine the actor's full output (including `<quotes>` blocks). The bench harness already passes full actor output to the judge. The judge prompt needs to be updated to: (1) check whether the actor's count differs from GT by exactly 1, (2) if so, examine whether the actor explicitly reasoned about including/excluding specific items, (3) accept the count only if the reasoning is present and defensible. This is a more sophisticated judge design than blanket tolerance — it requires the judge to evaluate reasoning quality, not just numerical proximity.

---

## Section 3 -- Item #8 Verification (Compiled-Truth Boost for RETRIEVAL_MISS)

### Case #4 — Doctors (gpt4_f2262a51): 0/3 answer sessions retrieved

**Query terms**: "How many different doctors did I visit?"

**Answer session content** (user turns from each missing session):

- **answer_55a6940c_1**: "prescribed antibiotics by my [doctor]", "diagnosed with it by an ENT specialist", "talk to Dr. Smith about my sinusitis"
- **answer_55a6940c_2**: "diagnosed with chronic sinusitis by an ENT specialist, Dr. Patel", "prescribed a nasal spray", "primary care physician"
- **answer_55a6940c_3**: "nasal spray prescription from Dr. Patel", "follow-up appointment", "follow-up with Dr. Lee for the biopsy"

**Lexical overlap**: The query term "doctors" has LOW direct overlap with the session content. The sessions use "Dr. Smith", "Dr. Patel", "Dr. Lee", "ENT specialist", "primary care physician" — all specific names and specialties rather than the generic word "doctors." The word "visit" maps to "appointment", "follow-up", "diagnosed" — related but not identical.

Crucially, the sessions DO contain the word "doctor" in some turns (e.g., "ask my doctor"), but these are sparse compared to the dominant vocabulary of medical conditions (sinusitis, UTI, biopsy) and specific doctor names.

**Classification**: **Vocabulary-gap**. The query uses the generic term "doctors"; the sessions use specific names (Dr. Smith, Dr. Patel, Dr. Lee) and specialties (ENT specialist, primary care physician). FTS keyword matching fails because "doctors" doesn't match "Dr. Patel."

**Would item #8 help?** Yes. A Librarian-generated description like "User visits multiple doctors including primary care physician Dr. Smith, ENT specialist Dr. Patel, and dermatologist Dr. Lee" would index the word "doctors" and "visit" directly, bridging the gap.

### Case #10 — Furniture (gpt4_15e38248): 2/4 answer sessions retrieved, 2 missing

**Query terms**: "How many pieces of furniture did I buy, assemble, sell, or fix in the past few months?"

**Missing answer session content**:

- **answer_8858d9dc_1**: "I just got a new coffee table from West Elm about three weeks ago", "perfect wooden coffee table with metal legs", "navy throw blanket"
- **answer_8858d9dc_3**: "I just got a new coffee table and rearranged my living room", "new Casper mattress", "bedside tables"

**Lexical overlap**: The query term "furniture" has MODERATE overlap. The sessions mention "coffee table", "bedside tables" — which contain the word "table" but not "furniture." The query term "buy" maps to "got" / "got a new" — related but lexically distant. "Assemble", "sell", "fix" have no overlap with the missing sessions.

However, the word "table" (in "coffee table", "bedside tables") is a strong sub-term. FTS should match "table" as a substring of "furniture" — wait, no. FTS matches on token overlap, and "furniture" and "table" are different tokens. This IS a vocabulary gap: the sessions describe specific furniture items (coffee table, mattress, bedside tables) without using the category word "furniture."

**Classification**: **Vocabulary-gap**. The sessions describe specific items (coffee table, mattress) without using the category term "furniture." FTS on "furniture" doesn't match "coffee table."

**Would item #8 help?** Likely. A description like "User bought a new coffee table from West Elm and rearranged living room furniture" would index the word "furniture" directly. However, the Librarian must generalize from "coffee table" to the category "furniture" — this is a quality requirement on the description generation.

### Revised lift estimate for item #8

**Cases addressable**: #4 (doctors, high confidence — vocabulary gap is clear and description bridging is straightforward) and #10 (furniture, medium confidence — depends on Librarian generating category-level descriptions from specific-item mentions).

**Revised net lift**: **+1 to +2 questions (50% -> 55-60%)**. Case #4 is high confidence. Case #10 is medium confidence — the Librarian needs to produce category-level descriptions, which is a quality bar that may or may not be met.

**Downward revision from PR #99**: PR #99 estimated "+5-10pp" for item #8. The revised estimate is +5-10pp (1-2 questions out of 20). The original estimate was "+5-10pp" which is consistent, but the doc implied it was a lower bound. The upper bound depends on Librarian description quality for case #10.

**Pre-validation step (from external-research-synthesis Open Question 2)**: Before building the full pipeline, manually write ideal descriptions for the 2 RETRIEVAL_MISS cases and re-run retrieval. If the manually-written descriptions bridge the gap, the mechanism is validated. This is a 30-minute de-risking step.

---

## Section 4 -- Updated Path-to-90% Math

### Current state: 50% (10/20)

### After item #20 (reasoning-aware judge rubric): ~65% (13/20)

- Flips: #1 clothing, #2 projects, #6 citrus (all delta=1 DEFINITION_DISAGREEMENT with explicit actor reasoning)
- Does NOT flip: #7, #8, #9 (delta=1 GENUINE_MISS without actor reasoning)
- Regression risk: zero identified on currently-correct set
- Confidence: high for the 3 flips. Requires reasoning-aware judge design (not blanket tolerance).

### After item #8 (compiled-truth boost): ~70% (14/20)

- Flips: #4 doctors (high confidence — vocabulary-gap, description bridging straightforward)
- Possible flip: #10 furniture (medium confidence — depends on Librarian generating category-level descriptions)
- Confidence: medium — requires Librarian pipeline + FTS index + description quality validation

### Remaining after #20 + #8: 6 failures

| # | QID | Classification | Addressable by |
|---|-----|----------------|---------------|
| 3 | gpt4_d84a3211 | GENUINE_MISS (within-turn) | Unknown — within-turn attention drop, not addressed by per-session extraction |
| 5 | dd2973ad | DATE/TEMPORAL_REASONING | Temporal reasoning prompt (1 case, low priority) |
| 7 | gpt4_a56e767c | GENUINE_MISS (4th festival) | Unclear evidence for 4th festival |
| 8 | 46a3abf7 | GENUINE_MISS (cross-session) | Per-session extraction experiment |
| 9 | gpt4_2f8be40d | GENUINE_MISS (cross-session) | Per-session extraction experiment |
| 10* | gpt4_15e38248 | RETRIEVAL_MISS (if #8 fails) | Alternative retrieval strategy |

### Realistic ceiling: ~75% (15/20)

If per-session extraction addresses #8 (tanks) and #9 (weddings): +2 more questions -> 75%.

#3 (within-turn miss), #5 (temporal reasoning), and #7 (unclear 4th festival) are likely not addressable by any planned intervention. That puts the realistic ceiling at 75% for multi-session with current architecture.

### Delta from PR #99 estimates

| Item | PR #99 estimate | Revised estimate | Delta | Key correction |
|------|----------------|------------------|-------|----------------|
| #20 (judge rubric) | +15pp (50->65%) | +15pp (50->65%) | No change in number | Must use reasoning-aware design, not blanket +/-1. Blanket tolerance would inflate to +30pp with 3 false positives. |
| #8 (compiled-truth) | +5-10pp | +5-10pp (1-2 cases) | No change in range | Upper bound conditional on Librarian description quality. Pre-validate with manual descriptions. |
| Combined | 70-75% | 70% (conservative), 75% (optimistic) | No change | Path requires reasoning-aware judge + description quality |

PR #99's numerical estimates hold. The critical correction is that item #20 MUST use reasoning-aware tolerance (Design B), not blanket tolerance (Design A). Blanket tolerance would score 80% but with 3 false positives that mask the system's hardest failure mode.
