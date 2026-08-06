# Prereg — R11: context rendering A/B on held-out LoCoMo (2026-08-05)

Status: **BUDGET-GATED — not run.** Requires Jesse's approval for ~$1.51
(stage 1) + ~$1.51 (stage 2, only if stage 1 passes). Committed before any
measurement.

## Question

`recall_at` ships `spectral_tact::format_context_block` — ungrouped,
undated, no role tags — while `render::session_grouped` (dated, grouped,
role-tagged) is the published format. This is the last family where an
accuracy lever plausibly lives: retrieval is saturated (97.9% session
recall) and the residual failures are actor-side, so presentation is the
only remaining input we control that the actor sees. Does the published
format improve end-to-end accuracy?

## Design

Two arms, identical retrieval by construction — the retrieved key set is
byte-identical between arms (pinned in the harness before any paid call);
ONLY the rendering of the same memories differs:

- **A (shipped):** `format_context_block`.
- **B (candidate):** `render::session_grouped`.

Same actor model, same params, held-out LoCoMo (nothing in Spectral tuned
on it). Paired per-question scoring.

## Two-stage gate — significance REQUIRED (closing the #239 hole)

The k-lever prereg set an effect-size threshold with no significance
requirement and landed on exactly the threshold; recorded lesson: never
again. Also recorded: single n=120 LoCoMo runs carry ~±10pp, so raw deltas
are noise — the gate is on paired flips.

- **Stage 1 (dev, n=120, ~$1.51):** proceed to stage 2 iff paired delta
  ≥ +5pp. Otherwise STOP, record NULL, R11 stays open as a formatting
  decision only (redirect may still happen for consistency, but with no
  accuracy claim).
- **Stage 2 (disjoint validation, n=120, ~$1.51):** PASS iff paired delta
  ≥ +5pp AND McNemar exact two-sided p < 0.05. Anything else is a NULL —
  including "≥5pp at p≥0.05", which is precisely the #239 case.
- No arm C, no prompt iteration, no re-rolls. One rendering candidate, one
  shot. A failed gate cannot be followed by "try a tweak" without a fresh
  prereg naming what changed and why.

## What a PASS ships

`recall_at` (and the tact context block) redirect to `session_grouped`,
with a major-version note for consumers parsing the old format. What a NULL
ships: nothing on the default path; the register row records the verdict.

## Cost ceiling

$3.02 total, sequential stages, abort-early. No other paid runs ride along.

## Amendments 2026-08-05, committed BEFORE any measurement

1. **Budget arithmetic corrected.** The original text implied $1.51/stage;
   a stage is TWO fresh arms (~$1.51 each), so stage 1 ≈ **$3.02** — the
   full approved ceiling. Stage 2 therefore requires a separate budget
   sign-off if stage 1 passes. Reusing the burned 2026-08-01 run as arm A
   was rejected: retrieval code has changed since (temporal rewrite, policy
   V2), which would break the identical-retrieval construction.
2. **Arm A runs UNCAPPED.** `format_context_block`'s shipped default caps at
   24,000 chars; a binding cap would confound format with budget. At LoCoMo
   k=30 (mean ~1.6k tokens) the cap essentially never binds, but it is
   disabled outright so rendering is provably the only variable. The cap is
   a separate lever with its own history (0.36 cap REJECTED, −15pp).
3. **Mechanics.** `spectral-bench-accuracy run --render {tact-block|
   session-grouped}` re-renders the identical raw cascade hits through the
   library surface; a non-Harness mode with no raw hits FAILS the question
   rather than silently falling back. Cross-arm identity is verified
   post-hoc: `retrieved_memory_keys` must be equal per question across
   arms; any mismatch voids the run before grading is read.
4. **Config:** dataset `locomo_heldout.json` (the burned dev 120 — stage 1
   is dev by design), cascade retrieval with shape routing, actor+judge
   `claude-sonnet-4-6`, no env levers, defaults otherwise. A ~$0.10 smoke
   run (2 questions/arm) validates plumbing first and counts inside the
   ceiling.
5. **(post-void, before any rerun) `--no-expand-queries` on BOTH arms.**
   The first stage-1 attempt was VOIDED by its own identity precondition:
   default-on LLM query expansion sampled differently across arms on 3/120
   questions, changing retrieved sets. Diagnosis and the honesty note on
   observed-but-void toplines: `r11-render-ab-stage1-void-2026-08-05.md`.
   Expansion is removed from both arms equally; the rerun (~$3.02, on top
   of the ~$3.02 consumed by the void run) requires fresh budget sign-off.
