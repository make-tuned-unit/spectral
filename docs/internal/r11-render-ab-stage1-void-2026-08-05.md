# R11 stage 1 — VOID (2026-08-05), diagnosis, and the fix

Prereg: `r11-render-ab-prereg-2026-08-05.md`, amendment 3: *"any
[cross-arm retrieval] mismatch voids the run before grading is read."*

## What happened

Both arms ran clean (118/120 clean each, ~$3.02 spent). The identity
precondition then failed: **3 of 120 questions retrieved different key SETS
across arms** (locomo_5_46, locomo_7_19, locomo_3_78 — membership swaps at
the k=60 admission boundary). The run is void as evidence, per prereg.

Honesty note: the completion line printed arm toplines before the identity
check ran (A 45/120, B 67/120 — nominally +18.3pp toward session-grouped).
Those numbers are **observed but non-evidential**: 3 contaminated retrievals
and a voided gate. They set no expectation the rerun must meet; recording
them here rather than pretending they were unseen.

## Diagnosis ($0, oracle replays)

| check | result |
|---|---|
| within-brain determinism (each work-dir, replay ×2, all 3 questions) | byte-identical |
| across-brain (arm A brain vs arm B brain, same question) | **byte-identical** — brains are NOT the source |
| replay vs paid-run recorded keys | matches NEITHER arm |

Brains identical and retrieval deterministic given the query ⇒ the query
text itself differed between arms ⇒ the only query-mutating stage in the
eval path is **pre-retrieval LLM query expansion** (`--no-expand-queries`
defaults OFF, i.e. expansion ON, sampled per run; the oracle replay ran
without it, which is also exactly why replay matched neither paid run).
117/120 agreement is the signature: expansion is usually stable across two
same-day runs, and occasionally is not.

Also caught during diagnosis: a first replay attempt passed a
comma-separated `--question-id` list the oracle doesn't parse, producing
empty files and a vacuously-true determinism check — the empty-metric trap
Permagent named in dispatch 04e §4. Re-run with per-question IDs and
non-empty outputs asserted.

## The fix (prereg amendment 5, committed before any rerun)

Both arms rerun with `--no-expand-queries`: expansion is an equal handicap
removed from both arms, restoring identical-retrieval-by-construction. The
A/B measures rendering, not expansion. Alternative (a shared frozen
expansion cache) is rejected for now: the eval path has no cache input, and
building one adds surface for exactly this class of leak.

## Budget

Stage 1 as-run consumed the approved ~$3.02 and is void. The clean rerun
needs a further **~$3.02** — awaiting sign-off before any call is made.

## What survives regardless

The harness (`--render`), the identity precondition (which WORKED — it
caught real contamination that raw deltas would have hidden), the
within/across-brain determinism evidence, and the finding that eval-path
query expansion makes "deterministic retrieval" false across paid runs —
worth its own register row.
