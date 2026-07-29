# PRE-REGISTRATION — verdict-threshold scale calibration (2026-07-29)

Committed before the fix is implemented or measured. Addenda only, no rewrites.

## The defect (measured, public benchmark PR #229)

Verdict-level false-familiar rates at public-benchmark enrollment scales:
R1 962/969 (99.3%), R2 455/559 (81.4%), R3 4464/4464 (100%). The `Familiar`
verdict fires on `best.score >= familiar_min_score (2.5)` where score is an
absolute rarity-weighted sum, `rarity = ln((enrolled+1)/df)`. Calibrated at
~1.6k enrolled memories; at ~9k enrolled a single shared df≈10 feature scores
ln(900) ≈ 6.8 — any one collision anywhere clears the constant. The
`best_similarity >= familiar_floor (0.10)` path is similarly loose between
same-domain short texts. Both thresholds are scale-blind.

## The fix (deterministic, config-gated)

1. **`familiar_min_features` (default 2):** the Familiar-by-score path
   requires at least 2 independent matched features (pair or gram hits) —
   a single rare-feature collision is no longer sufficient evidence of
   familiarity at any enrollment scale.
2. **`familiar_min_similarity` (default 0.20):** the similarity arm of the
   familiarity floor gets its own threshold, decoupled from the normalized
   coverage floor (which stays at 0.10 — coverage is already
   scale-independent).

Scalar/AUC is untouched by construction (verdict-only change). Old behavior
recoverable via config (features=1, similarity=0.10).

## Pre-registered targets (public bench, same splits as PR #229)

| metric | current | target | hard constraint |
|---|---|---|---|
| R1 missed re-encounters (pos_novel) | 0% | — | **≤ 1%** (never-miss property holds) |
| R1 false-familiar (neg) | 99.3% | **≤ 35%** | — |
| R2 paraphrase read Novel | 0% | — | **≤ 5%** (the 1.1% private property, with margin) |
| R2 false-familiar (neg) | 81.4% | **≤ 50%** | — |
| R3 false-familiar | 100% | report only | — (adversarial pairs genuinely share rare features; semantic error, not calibration error) |

Decision rules: if a hard constraint is violated, the defaults revert and the
knobs ship config-only with the finding. One calibration iteration on the two
constants is permitted and must be reported (values tried, values shipped).
Private-brain verdict rates (1.6k scale) re-checked if the local brain DB is
present — calibration must not regress the original scale.
