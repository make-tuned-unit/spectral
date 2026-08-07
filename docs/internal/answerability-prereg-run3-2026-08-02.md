# Preregistration — answerability, run 3 (on the fixed foundation) — 2026-08-02

> **METRIC CAVEAT (R15, 2026-08-07):** "key-recall" in this document is
> evidence-**session** turn coverage — every turn of every `answer_` session, a
> ~12x-diluted denominator — not evidence-turn recall. See
> `turn-level-evidence-recall-2026-08-07.md`. This note does not assert what the
> correct metric would have shown here; the numbers below are left exactly as
> measured (Rule 5).

**Written before the measurement.** Binding.

## What changed since run 2

Runs 1 and 2 both measured a lever that was **structurally prevented from
acting** on most of the corpus. Two defects, both since fixed and both verified
byte-identical on the default path:

| defect | fix | verified |
|---|---|---|
| Cascade pool widening was a no-op — `run_cascade_pipeline_scoped` truncates to `config.k` (`cascade_layers.rs:441`), so `merged_hits.take(k * widen)` returned the same `k` | widen `pipeline_config.k` *before* the pipeline call, truncate to `output_k` after | oracle 0/120 context-hash diffs with all levers off |
| Session-grouped rendering discarded rank — grouped by episode, sorted by key and date, never consulting rank | `SessionOrder::ByRank` orders sessions by best contained rank; turn order within a session stays chronological | 13/13 render unit tests; `Chronological` remains the default |

Run 2 measured: cascade set changed on **0 of 84** questions. Run 3 is the
first run in which the lever can act on the whole corpus.

**This is a third distinct experiment, not a retry.** The scoring function and
its weights remain frozen at the run-1 values. Only the foundation changed.

## Hypothesis

**H3:** with membership-changing widening *and* rank reaching the actor,
query-conditioned answerability improves retrieval on the held-out set.

## Arms

| arm | env |
|---|---|
| A — baseline | none (reproduces published held-out figures) |
| B — answerability, chronological render | `SPECTRAL_ANSWERABILITY=1` |
| C — answerability + rank-preserving render | `SPECTRAL_ANSWERABILITY=1 SPECTRAL_RENDER_BY_RANK=1` |
| D — rank-preserving render alone | `SPECTRAL_RENDER_BY_RANK=1` |

Arm D is the control that separates the two changes. Without it, any movement
in C is unattributable — the failure mode that produced two uninterpretable
runs already.

## Decision rules (binding)

1. **Primary (B and C vs A).** Session-recall ≥ **+1.0pp** or key-recall ≥
   **+1.0pp**.
2. **Integrity.** Arm D must not change session-recall, key-recall or
   zero-recall **at all** — reordering sessions cannot change which memories
   were retrieved. If it does, the rendering change is corrupting the set and
   everything else is void.
3. **Cost.** Mean context tokens ≤ **+5%** in every arm.
4. **Attribution.** If C beats B, the difference must be attributable to
   rendering via arm D's context-hash diff count. An unexplained gap is not a
   result.
5. **Default stays OFF regardless of outcome.** Both `AnswerabilityConfig` and
   `SessionOrder::ByRank` remain non-default. Clearing these gates makes them
   candidates for a paid actor A/B — nothing more. The ACR precedent stands:
   +18–40pp answer-key recall converted to **−2 accuracy**.
6. **Final attempt.** If run 3 fails, the query-conditioned answerability
   hypothesis is closed permanently and recorded as refuted. No fourth run, no
   weight sweep. Three runs is already generous for a lever whose prior was
   low.

## Prior

Still low. The foundation fixes remove a *reason the measurement was invalid*;
they are not evidence the hypothesis is true. Run 2's TopkFts arm — where the
lever genuinely did act on all 36 questions — produced +1 answer key and a
category trade that netted to zero. That is the cleanest existing estimate of
the effect size, and it is approximately nothing.

The reason to run it anyway: the fixes are independently correct and now
measured, and this is the only way to close the hypothesis honestly rather than
leaving it open on a technicality.

## Method

`locomo_heldout.json`, 120 held-out questions, $0 oracle, zero LLM calls,
`--fresh-brains --no-keep-brains`. Weights frozen at run-1 values:
`answer_type_weight` 0.12, `coverage_weight` 0.10, `ack_penalty` 0.15,
`topic_only_penalty` 0.08, `rank_step` 0.03, `ANSW_POOL_WIDEN` 2.
