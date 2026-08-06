# Result — R11 stage 2, disjoint validation (2026-08-06): GATE PASSED — SHIPPED

Prereg: `r11-render-ab-prereg-2026-08-05.md` (amendments 1–5). Stage 1:
`r11-render-ab-stage1-result-2026-08-05.md` (+19.2pp dev, p=1.6e-6).

## Sample construction (disjoint by construction and verification)

`scripts/locomo_to_oracle.py --seed 44 --exclude locomo_heldout.json
--exclude locomo_validation.json` over raw `locomo10.json`: 1,198 answerable
after excluding all 240 previously-used questions; sampled 40/40/40
(multi-session / single-session-user / temporal-reasoning). Verified: zero
ID overlap with both burned sets. Never touched by any prior run of
anything. Sample: `~/spectral-local-bench/r11-2026-08-05/locomo_stage2.json`.

## Preconditions

- Identity gate: **PASS — 0/120 cross-arm retrieval mismatches**
  (`--no-expand-queries`, amendment 5).
- Reliability: A 117/120 clean, B 120/120 clean.

## Result

| arm | accuracy |
|---|---|
| A — `format_context_block` (shipped, uncapped) | 46/120 = **38.3%** |
| B — `render::session_grouped` (published) | 63/120 = **52.5%** |

Paired **+14.2pp**; discordant **B-fixed 20, B-broke 3**; McNemar exact
two-sided **p = 4.88×10⁻⁴**. Gate (≥ +5pp AND p < 0.05): **PASS on both
prongs** — this is NOT the #239 edge case (≥5pp at p≥0.05), which the
prereg pre-declared a NULL.

| category | n | A | B | Δ |
|---|---|---|---|---|
| multi-session | 40 | 32.5% | 32.5% | +0.0pp |
| single-session-user | 40 | 62.5% | 62.5% | +0.0pp |
| **temporal-reasoning** | 40 | **20.0%** | **62.5%** | **+42.5pp** |

The dev signature replicates exactly: two categories bit-flat, the entire
effect temporal. Dev→validation shrinkage (+19.2 → +14.2pp) is present and
modest; the mechanism (dates in context) survives out of sample.

## Shipped (per the prereg's PASS clause)

Facade `recall` / `recall_at` / `recall_local` / `recall_local_at` now
publish `render::session_grouped` as `tact.context_block` (`recall_at`
threads its time anchor as the render question-date). **BREAKING for
consumers parsing the old undated TACT bundle** — the hits are untouched,
only the rendering changed. Pinned by
`render_contract::recall_context_block_is_session_grouped` (byte-equality
with the direct rendering + old-format leak guard). Lower layers
(`spectral_tact::format_context_block`, spectral-graph internals) are
unchanged.

## Ledger

Total R11 spend: ~$9.15 (void $3.02 + stage 1 $3.05 + stage 2 $3.08 est).
Bought: the first prereg-validated accuracy lever in the project's history,
plus R14 (expansion nondeterminism) found by the identity gate.

Artifacts: `~/spectral-local-bench/r11-2026-08-05/` (all six run JSONs +
stage-2 sample).
