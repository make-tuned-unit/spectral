# PR #173 — Blast-Radius Verification (artifact-carrying "correct" cases)

**Status**: Integrity check complete. PR #173 publishable.
**Method**: Formal re-judge — apply the shipped `strip_actor_continuation`
sanitizer to the on-disk actor answers, re-judge the SANITIZED answer against
the correct question/GT with the real `AnthropicJudge` (`claude-sonnet-4-6`).
**No actor re-run.** Judge calls only (~$0.08, 26 calls).
**Source**: `~/spectral-local-bench/eval-report-n500.json` (the published run).
**Tool**: `crates/spectral-bench-accuracy/src/bin/rejudge_artifacts.rs`
(reproducible).

## Result

> **Final integrity-checked accuracy: 401 / 492 = 81.5%** (clean denominator,
> 8 transport failures excluded). The published 81.5% holds — pinned by judge
> calls, not assessment.

- **0 false positives** among the graded-correct artifact cases.
- **+3** false negatives corrected (the autopsy trio), confirmed exactly +3
  (not +4) after checking every fix-affected wrong case.

---

## What "artifact" means here

The actor sometimes runs past its real answer and fabricates trailing content.
Two distinct shapes:

1. **Scaffold continuation** — `Question:` / `## Question` / `Now answer the
   following question…` + an invented Q&A. This is what
   `strip_actor_continuation` removes (3 markers).
2. **Dialogue-tag continuation** — fabricated `[user]`/`[asst]` turns.

**Important**: `strip_actor_continuation` deliberately does **not** strip
`[user]`/`[asst]`. Those tags also appear *legitimately* — the actor's
two-pass "session scan" answer format quotes retrieved `[user]` turns as
evidence (e.g. 21436231 quotes `[user]: "I caught 12 largemouth bass"` then
concludes "12"). Stripping at the first `[user]` would destroy correct
answers. The fix targets only the unambiguous scaffold markers; trailing
`[asst]`/`[user]` fabrications occur *after* the real answer, so the real
answer (which the judge reads first) is intact.

## Detection reconciliation (18 vs. our superset)

The PR body cites a prior audit's "26 carry an artifact (8 wrong + 18
correct)". The audit docs lived in worktrees and are gone, so we
**re-derived** the set directly from the report using the union of scaffold
markers and dialogue-tag markers — a **broader** detector:

| | total | graded correct | graded wrong |
|---|---|---|---|
| Re-derived superset (scaffold ∪ tags) | 31 | **22** | 9 |

We verified the **22** graded-correct cases — a strict superset of the
audit's 18 — so whatever exact 18 the audit meant, they are all covered.

---

## Direction 1 — false negatives (graded wrong → correct)

Only cases where the fix actually changes the text can flip. Among the 9
graded-wrong artifact cases, the scaffold strip changes the text for **4**:

| qid | category | fix changes text? | re-judge (sanitized) | flip |
|-----|----------|-------------------|----------------------|------|
| 55241a1f | multi-session | yes | CORRECT (33=33) | ✅ wrong→correct |
| 8b9d4367 | single-session-assistant | yes | CORRECT (Jaipur Rugs) | ✅ wrong→correct |
| b6025781 | single-session-preference | yes | CORRECT (healthy meal-prep) | ✅ wrong→correct |
| **09d032c9** | single-session-preference | yes | **WRONG (unchanged)** | ❌ stays wrong |
| 0a995998, a9f6b44c, 195a1a1b, ef9cf60a, c4f10528 | — | **no** ([user] quotes) | n/a — fix doesn't touch | ❌ stays wrong (true negative) |

**09d032c9 is the integrity catch.** It carries a scaffold marker (so the fix
*does* change its text) but the PR's autopsy only flipped 3. We re-judged it:
its real answer is a generic battery-tips list that does not build on the
user's previously-mentioned portable power bank as the preference GT requires
→ genuine **true negative**, stays wrong. This confirms the fix yields
**exactly +3**, not +4.

The other 5 wrong cases are `[user]`-quote answers the fix does not touch;
they remain genuinely wrong (true negatives).

## Direction 2 — false positives (graded correct → wrong)

All **22** graded-correct artifact cases re-judge to **CORRECT** after
sanitization. **0 false positives.**

The PR's risk hypothesis (real answer wrong AND a fabricated trailing answer
coincidentally matching GT) did not occur in any case: every real answer
explicitly states the correct value. Knowledge-update supersession cases
(e.g. 618f13b2: "four… → six times", GT "six") correctly land on the most
recent value; the truncated 300-char preview that initially looked like a
false positive was just hiding the supersession sentence.

Notable: **7a8d0b71** — the *original* judge graded a fabricated trailing
sub-question ("how many recreational leagues") yet returned correct; after
sanitization the judge grades the *real* answer ("$2,000" influencer
marketing) against the *real* question — correct for the right reason. Same
verdict, sound basis.

---

## Accounting

| | correct | denom | accuracy |
|---|---|---|---|
| Published run (as-judged) | 398 | 492 | 80.9% |
| + Direction 1 (false negatives corrected) | +3 | | |
| + Direction 2 (false positives) | +0 | | |
| **Integrity-checked** | **401** | **492** | **81.5%** |

## Caveat closed

b6025781 (the preference case previously rubric-assessed) is now pinned by a
formal `AnthropicJudge` call (CORRECT). 81.5% rests on judge calls end to end.
