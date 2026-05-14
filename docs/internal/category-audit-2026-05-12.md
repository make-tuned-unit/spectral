# Spectral Bench Category Audit

**Date**: 2026-05-12
**Bench state**: 75.8% pre-#91, 76% projected post-revert-#94-full-revert
**Purpose**: Honest assessment of each LongMemEval-S category — what works, what doesn't, what's been tried, what remains untried. Drives the path-to-90% strategy.

---

## Strategic Frame

Prompt iteration has hit a ceiling. Today's pattern (PR #91 caused 25pp regression in preference, recovered only partially by PR #94, then required full revert) demonstrates: cheap prompt-level interventions are largely spent. **The remaining lift comes from architectural work, external research, or accepting category ceilings.**

This audit groups categories by their intervention status:

- **Solved**: above 90%, no further work warranted
- **Strong**: 85-90%, marginal interventions only
- **Working at ceiling**: 60-80%, prompt iteration tried multiple times, remaining failures are structural
- **Underexplored**: weak categories where we haven't yet exhausted simple options

---

## Category-by-Category Audit

### 1. temporal-reasoning — 85% (Strong)

**Status**: Strong. PR #86 added topk_fts routing (recovered the cascade regression) and a date-table-first actor prompt. Both interventions empirically validated.

**Strategy used**: `temporal.md` template + topk_fts retrieval path.

**Strategy accuracy**: 93% (26/28 questions where Temporal shape fired).

**What works**: Date-table-first reasoning. Topk_fts retrieval (cascade hurts this category by -15pp).

**Remaining failures** (3 of 20):
- One regression case: "How many months passed since two consecutive charity events" — multi-event temporal calculation
- Two still-failing cases involving order-of-events questions that classified to Factual or FactualCurrentState instead of Temporal

**Intervention status**: Classifier accuracy issue, not prompt or retrieval issue. Some temporal questions misclassify and get the wrong strategy. Fixing this is small-scope: refine the temporal sub-gate patterns to catch ordering-language ("What is the order of...", "Which event happened first...").

**Recommended next move**: 1-2h tightening of temporal classifier patterns. Expected lift: +5pp (90% target). Low risk.

---

### 2. knowledge-update — 85% (Strong, with retrieval blind spot)

**Status**: Strong overall, but failures point to retrieval limitations the prompt cannot fix.

**Strategies used**: Counting (45% of category), Factual (30%), General (15%), CountingCurrentState (15%), FactualCurrentState (5%), Temporal (10%).

**Strategy accuracies in this category**: Counting=100%, Factual=60%, General=100%, CountingCurrentState=100%, FactualCurrentState=100%, Temporal=50%.

**What works**: Most-recent-wins semantics handled correctly when classifier routes appropriately.

**Remaining failures** (3 of 20):
- "What was my mortgage pre-approval amount?" → Factual_direct, gave wrong specific number ($350K vs $400K). Retrieval-rank issue.
- "What type of camera lens did I purchase most recently?" → Factual_direct, gave wrong lens type. Retrieval-rank issue.
- "Where did Rachel move to?" → misclassified as Temporal because "recent relocation" matched temporal pattern. Classifier issue.

**Intervention status**: 2 of 3 failures are retrieval-rank issues — the right memory exists but isn't surfacing in top-K with the highest signal score. Item #2 (co-retrieval boost, shipped in PR #90) helps marginally but isn't enough. Item #11 (session signal in ranking) would help if user described the recent purchase in the latest session. Item #8 (compiled-truth boost) helps once Librarian populates descriptions.

**Recommended next move**: Classifier fix for "Rachel" case (~30 min). Then defer — remaining failures wait on item #11 (session signal) or item #8 (compiled truth).

---

### 3. single-session-user — 85% (Strong)

**Status**: Strong, hovering at ceiling. Failures look like retrieval misses or judge variance, not actor failures.

**Strategy used**: Mostly Factual (60% of category), some Temporal, some Counting.

**Strategy accuracies**: Factual=75%, Temporal=100%, Counting=100%.

**What works**: Single-entity retrieval is mostly clean.

**Remaining failures** (3 of 20):
- "What was my previous occupation?" → actor said "I don't know" despite info existing. Possible retrieval miss.
- "What speed is my new internet plan?" → "I don't know" response. Retrieval miss.
- "What did I buy for my sister's birthday gift?" → minor judge variance ("yellow dress and earrings" vs GT "a yellow dress").

**Intervention status**: 2 retrieval misses, 1 judge edge case. Neither is prompt-fixable.

**Recommended next move**: None category-specific. Item #20 (judge rubric refinement) could address the sister's-birthday case marginally. Don't iterate further on Factual prompt.

---

### 4. single-session-assistant — 90% (Strong)

**Status**: Strong. The GeneralRecall strategy with assistant_recall.md template is the second-highest-performing strategy in the bench (86% strategy accuracy, only behind Temporal's 93%).

**Strategy used**: GeneralRecall (75%), Temporal (15%), Counting (10%), General fallback (5%).

**Strategy accuracies**: GeneralRecall=86%, Temporal=100%, Counting=100%, General=100%.

**What works**: "Find the relevant prior session and quote what was said" — direct, clear, single-task prompt.

**Remaining failures** (2 of 20):
- "Children's book on dinosaurs" → asked about prior conversation, actor said no session contains this info. Retrieval miss.
- "Work-from-home jobs for seniors" → asked for second item from prior list, actor only saw first prompt. Retrieval truncation issue.

**Intervention status**: Both retrieval-shape issues. Prompt is fine.

**Recommended next move**: None. Accept the 90% ceiling unless retrieval architecture changes (item #12 L2 episodes might help).

---

### 5. multi-session — 55% (Working at ceiling)

**Status**: At ceiling for prompt iteration. The embedded-reference framing (PR #91's instruction 2) yielded +5pp. Underlying failure mode is structural: actor misses items embedded as passing mentions in different-topic sessions.

**Strategy used**: Counting (90% of category), CountingCurrentState (10%).

**Strategy accuracy in this category**: Counting=44%-50% depending on bench run.

**What we've tried**:
- PR #84: synthesis prompt revisions including "scan EVERY session" — small lift
- PR #86: counting_enumerate.md with enumerate-then-count discipline — 0pp net
- PR #91 v1: cognitive overload framing (count-first, dropped attribution) — wrong diagnosis, didn't ship
- PR #91 v2: embedded-reference framing — +5pp lift

**What we know about remaining failures** (per `multi-session-failure-investigation.md`):
- 9 of 10 failures: ACTOR_MISS. Items present in retrieved memories but actor doesn't count them as matches.
- 1 of 10: pure RETRIEVAL_MISS (doctors question).
- Specific cognitive operation failing: extracting passing references from sessions about different primary topics.

**What's structurally hard about this**:
- Single-shot LLM generation can scan a long context but exhibits "lost in the middle" attention drop.
- Subordinate clauses in narrative sessions don't trigger the same "this is a wedding I attended" recognition as direct statements would.
- This pattern likely persists even with better prompts.

**Untried interventions**:
- **Multi-step actor**: two-call pattern where call 1 lists candidates and call 2 counts them. Splits cognition across generations. Doubles cost.
- **Item #12 (L2 episodes)**: would provide episode-grouped retrieval. The actor sees discrete session blocks. Doesn't fix the embedded-reference recognition issue but may help by reducing context length per scan.
- **Actor model upgrade**: Claude Sonnet 4.5 → newer model. Out of scope for now (model is the bench standard).

**Recommended next move**: Defer further prompt iteration. Two paths forward:
1. Architectural: Item #12 L2 episodes. ~1 week. Might lift multi-session 5-10pp.
2. Research: External survey on long-context entity extraction. Cheap. May surface non-obvious technique.

**Realistic ceiling without architectural change**: 60-65%.

---

### 6. single-session-preference — 60% (Working at ceiling, possibly with judge issues)

**Status**: At ceiling for prompt iteration. Today's iteration moved 60% → 35% → 50% → projected 60-65% with full revert. **Net behavioral progress on preference today: zero or marginal.**

**Strategy used**: GeneralPreference (90% of category), Factual (5%), General fallback (5%).

**Strategy accuracy**: GeneralPreference=67% historically.

**What works** (in pre-#91 preference.md):
- "Explicit OR implicit signals" framing — permits inference from related contexts.
- "Attempt synthesis from partial evidence" — permits cross-domain transfer (Seattle hotels → Miami hotels).
- "Reference what the user has said" — keeps responses grounded.

**What broke things** (in PR #91):
- Restricting source priority to "stated > inferred" eliminated implicit-signal handling.
- "Do not give generic advice; trace back to what user said" converted inference into refusal.
- "Say so rather than guessing" reinforced refusal over synthesis.

**Specific failure patterns identified**:
- **Cross-domain transfer**: Hotel Miami / Bedroom furniture cases. Actor needs to recognize that preferences expressed in one domain are evidence about preferences generally, and apply them.
- **Vocabulary-gap retrieval misses (3 cases)**: Query says "homegrown ingredients" / "battery life" / "coffee creamer recipe", session content uses different terms ("fresh basil and mint" / "portable power bank" / "almond milk + vanilla"). Compiled-truth boost (item #8) would bridge once descriptions exist.
- **Judge ambiguity**: The "would prefer" rubric is genuinely vague. Many actor responses are arguably correct under a different rubric (item #20 deferred Change D from PR #84).

**Untried interventions**:
- **Judge rubric refinement (item #20)**: shipped Change D from PR #84. ~2-3h. Marginal lift expected.
- **Cross-domain transfer prompt instruction**: Could explicitly tell the actor "when the user discusses related domains, apply those preferences to the question's domain." Risk: might cause overreach on unrelated questions.
- **Item #8 (compiled-truth boost)**: addresses the 3 vocabulary-gap cases. Dormant until Permagent populates descriptions.

**Recommended next move**: Accept 60-65% as the prompt-only ceiling. Item #20 (judge rubric) and item #8 (compiled-truth, when Librarian ships) are the realistic levers. Multi-step actor not promising given the cross-domain transfer issue isn't about cognition.

**Realistic ceiling without architectural change**: 65-70%.

---

## Path to 90%: Honest Reassessment

After today's iteration, the original path-to-90% projection (+15pp from #86, +5pp from #90, +5pp from prompt refinements, +5pp from item #12, +3pp from item #8, +2pp from #19/judge) overestimated prompt-level lift and underestimated category ceilings.

**Realistic per-category ceilings**:

| Category | Current | Prompt-only ceiling | With architectural work |
|---|---|---|---|
| temporal-reasoning | 85% | 90% (classifier fix) | 90% |
| knowledge-update | 85% | 87% | 92% (item #8 + #11) |
| single-session-user | 85% | 87% | 90% |
| single-session-assistant | 90% | 92% | 92% |
| multi-session | 55% | 60-65% | 70-75% (item #12) |
| single-session-preference | 60% | 65-70% (item #20) | 75-80% (item #8) |

**Composite ceiling**:
- **Prompt-only**: ~80-82%
- **With items #8, #11, #12, #20, classifier refinements**: 85-88%
- **90% would require**: actor architecture change (multi-step, model upgrade) or new architectural primitives we haven't designed

**80% is reachable. 85% is reachable with architectural work over 3-4 weeks. 90% requires changes we haven't planned for.**

This is a meaningful recalibration from yesterday's projection.

---

## Recommended Strategic Sequence

Based on this audit, the right next moves in order:

1. **External research dispatch** (parallel, dispatching now). Survey field practices for our specific failure modes. Output informs item #12 design and may surface non-obvious techniques.
2. **Targeted classifier fixes** (1-2h). Fix the misclassifications affecting knowledge-update and temporal-reasoning. Expected lift: +2-3pp overall.
3. **Item #20 judge rubric** (2-3h). Marginal preference lift. May also clarify what failures are genuine vs judge artifacts.
4. **Item #12 L2 episodes** (1 week). Biggest single unshipped lever. Likely lifts multi-session and possibly knowledge-update. Should be planned with external research findings.
5. **Wait for Permagent description coverage** (timeline TBD). Then item #8 (compiled-truth boost) activates.

This sequence trades the 90% ambition for honest 85% with margin. The 90% target may need to wait for actor-side changes (multi-step actor, model upgrade) we haven't planned.

---

## What's Out of Scope

Things we considered today and consciously rejected:
- More single-prompt iteration on counting_enumerate.md or preference.md — diminishing returns.
- Adding new shape variants or sub-gates without evidence they capture distinct failure modes.
- Rebuilding the orchestrator — the architecture decision is settled.
- LLM-in-loop reranking — violates the deterministic-recognition commitment.
- Cross-domain transfer prompt instruction without external research first — high risk of overreach.

---

*Audit complete. Companion document: research dispatch for external best practices.*
