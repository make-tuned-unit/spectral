# Result — R11 stage 1 (clean rerun, 2026-08-05): GATE PASSED

Prereg: `r11-render-ab-prereg-2026-08-05.md` (+ amendments 1–5, all
committed before the measurements they govern). Void first attempt:
`r11-render-ab-stage1-void-2026-08-05.md`.

## Preconditions

- **Identity gate: PASS — 0/120 cross-arm retrieval mismatches** (with
  `--no-expand-queries` per amendment 5). The comparison is
  rendering-only by construction AND by verification.
- Reliability: 119/120 clean both arms (1 judge-parse retry each).

## Result

| arm | accuracy |
|---|---|
| A — `format_context_block` (shipped, uncapped) | 43/120 = **35.8%** |
| B — `render::session_grouped` (published) | 66/120 = **55.0%** |

Paired delta **+19.2pp**; discordant pairs **B-fixed 24, B-broke 1**;
McNemar exact two-sided **p = 1.55×10⁻⁶**. Stage-1 gate (≥ +5pp): **PASS**.

## The mechanism is localized and legible

| category | n | A | B | Δ |
|---|---|---|---|---|
| multi-session | 40 | 27.5% | 27.5% | +0.0pp |
| single-session-user | 40 | 72.5% | 72.5% | +0.0pp |
| **temporal-reasoning** | 40 | **7.5%** | **65.0%** | **+57.5pp** |

Two of three categories are *exactly* unchanged. The whole effect is
temporal reasoning, and the explanation is not subtle: the shipped
`format_context_block` renders memories **without dates**, and a temporal
question over undated context is guesswork (3/40). `session_grouped`
carries the session date header; the same retrieved memories become
evidence (26/40). This is not "grouping is nice" — it is "the shipped
default withholds the one field temporal questions require."

Consistency note: the void run's non-evidential toplines (37.5%/55.8%)
are closely reproduced here (35.8%/55.0%) — the contamination was real but
small, and the clean rerun stands on its own gate.

## What this is and is not

- It IS the first accuracy lever this project has confirmed under prereg —
  found at the actor boundary, exactly where the saturation analysis said
  any remaining lever must live. In-sample (dev) only until stage 2.
- It is NOT yet a shippable claim. Per prereg: stage 2 on a fresh
  **disjoint** LoCoMo sample (n=120, never used in any prior run —
  excludes both `locomo_heldout.json` and the k-lever's
  `locomo_validation.json`), gate ≥ +5pp AND McNemar p < 0.05. Requires a
  further ~$3.02 sign-off.
- If stage 2 passes: `recall_at`/tact context redirect to
  `session_grouped` (major-version note for consumers parsing the old
  format) — the R11 register row's original fix, now with an accuracy
  reason rather than a consistency preference.

Artifacts: `r11-rerun-{tact-block,session-grouped}.json` (scratch; copy to
`~/spectral-local-bench/r11-2026-08-05/` with the void-run files).
