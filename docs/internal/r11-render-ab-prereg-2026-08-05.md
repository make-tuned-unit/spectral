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
