# Result — relative date offsets · **PASS** against a preregistered null

**Preregistered in `relative-offsets-prereg-2026-08-08.md`**, including the
expectation that this would fail. It did not.

This is the project's **second** preregistered accuracy win, after R11 — and it
is on the same axis, which is now the only axis that has ever paid here.

## Primary result — LongMemEval `temporal-reasoning`, n = 133

| | A: no offsets | B: `relative_offsets` |
|---|---:|---:|
| accuracy | 76.69% (102/133) | **83.46% (111/133)** |
| mean context tokens | 14,026 | 14,184 (**+1.1%**) |
| transport / auth / judge-parse failures | 0 / 0 / 0 | 0 / 0 / 0 |
| spend | $6.25 | $6.26 |

**Delta +6.77pp.**

| contingency | |
|---|---:|
| both correct | 102 |
| both wrong | 22 |
| **B fixed (A wrong, B right)** | **9** |
| **B broke (A right, B wrong)** | **0** |
| discordant pairs | 9 |

**McNemar exact two-sided p = 0.0039. PASS.**

Nine fixed, **zero broken** — a strictly one-directional effect on this slice.

## The prior was wrong, and wrong in an instructive way

The prereg committed to expecting null or negative, for three stated reasons.
Two of them were simply wrong, and the third was right about the mechanism and
backwards about the conclusion:

1. *"It costs tokens for redundancy."* **Wrong.** +1.1% (14,026 → 14,184). The
   offset annotation is a handful of characters per date tag.
2. *"The literature favours verbatim timestamps over derived annotations."*
   **Not applicable.** This does not replace the timestamp; it annotates it.
   The verbatim date is still there.
3. *"A relative offset is derivable from what the actor already has, so it adds
   no information — only a computation the model may or may not have been
   getting wrong."* **Correct premise, backwards conclusion. The model was
   getting it wrong.**

## Mechanism — measured, not inferred

Of the 9 questions B fixed, **7 (78%)** are phrased in relative time ("how many
days ago", "a week ago", "two months ago"), against **56%** of the slice as a
whole. The lever's effect concentrates exactly where its mechanism predicts.

The failure it removes is visible in the answers. On *"What was the life event
of one of my relatives that I participated in a week ago?"*:

- **A:** "Based on the session dated **2023-04-15**, you mentioned attending
  your…" — wrong session, because the actor computed the offset itself and
  missed.
- **B:** "Based on the session dated **2023-06-15 (7 days ago)**, you attended
  your…" — right session.

Same retrieval, same context, same model, same temperature. The only difference
is whether the offset arrived pre-computed. **This lever does not add
knowledge; it deletes an arithmetic failure mode.**

That also explains the zero regressions: a correct annotation cannot mislead an
actor that would otherwise have computed the same value correctly. It can only
help the cases where the actor would have been wrong.

## Replication — **INCOMPLETE, blocked on credit, NOT a result**

The replication (LoCoMo `temporal-reasoning`, n=317, preregistered above) got
one arm and part of the other before the account ran out of API credit:

| arm | progress | outcome |
|---|---|---|
| A (no offsets) | **317/317 complete** | 230 correct, 72.56%, $3.74 |
| B (relative offsets) | **59/317, then auth-failed** | 2 auth failures, run killed |

`Your credit balance is too low to access the Anthropic API.` The run was
stopped immediately so that further questions would not be recorded against a
dead key. Auth failures are excluded from the accuracy denominator by design,
so the partial data is incomplete rather than corrupted, and both arms are
resumable from their checkpoints when credit is restored.

**The partial has NOT been analysed, deliberately.** Fifty-seven paired
questions would produce a number, and that number would be read *after*
already knowing the primary result — a textbook garden-of-forking-paths. The
prereg specified n=317 and a single test. Looking at a partial and reporting
whichever way it fell is precisely the selective-reporting failure this
project exists to avoid, and the fact that the partial is *available* is not a
reason to spend it.

**Status: the primary result stands, unreplicated.**

## What this claims, and what it does not

**Claims:**
- On LongMemEval temporal-reasoning, with expansion disabled, `relative_offsets`
  improves accuracy by 6.77pp, p = 0.0039, at +1.1% context tokens.

**Does not claim:**
- Anything about **other categories.** The prereg excluded them to keep the run
  cheap, and R11 found them bit-flat on the same axis. Untested here.
- Anything about the **shipped expansion-on configuration.** Both arms ran
  `--no-expand-queries`, because expansion is a second variable whose
  nondeterminism voided the R11 stage-1 run. What ships has expansion on
  (shape-gated to counting questions), and this run does not speak to it.
- That **9 discordant pairs is a large sample.** The exact test is valid at that
  count — it is why an exact test was preregistered rather than a normal
  approximation — but the *effect size* carries wide uncertainty. The direction
  is well supported; +6.77pp as a point estimate is not.
- Comparability with the **65.02% LoCoMo baseline**, which used the pre-R21
  judge scorer. This run uses post-R21.

## Two disclosed process deviations

Both were found by launching arm A and reading the log, **before any arm
completed and before any A-vs-B comparison existed**:

1. **Query expansion was on** in the first launch — a second variable, and the
   specific one that voided R11 stage 1. Fixed to `--no-expand-queries` on both
   arms; 8 aborted questions discarded.
2. **The cost estimate was ~4× low** — $0.0127/question was carried over from
   LoCoMo, but LongMemEval contexts average 14,665 tokens against LoCoMo's
   2,841. Actual: $0.0478/question, $12.51 for both arms.

Both are recorded in the prereg as a dated correction rather than a silent
edit. Nothing comparative had been observed, and that is checkable: the aborted
arm has no counterpart.

## Recommendation

**Do NOT flip the default.** R11 was believed because it survived a disjoint
validation set. This has not been replicated, and one significant run against a
stated null is evidence, not proof.

What the primary supports today: `relative_offsets` is **promising enough to
finish testing**, and the finishing cost is ~$8 of API credit. Concretely:

1. Restore credit and **resume the replication from its checkpoints** — arm A is
   already complete, so only arm B remains (~$4).
2. If it replicates in the same direction, defaulting it on for temporal shapes
   is a strong case: 6.77pp at +1.1% tokens with zero measured regressions is a
   better trade than anything else in the record.
3. If it does not replicate, the primary is recorded as a single-corpus result
   and the lever stays off.

Until then the docstring's "off by default, untested for accuracy" note should
read **"measured once, significant, not yet replicated."**

**Refs:** `relative-offsets-prereg-2026-08-08.md`,
`r11-render-ab-stage2-result-2026-08-06.md` (the axis), `render.rs`,
`scripts/paired_mcnemar.py`.
