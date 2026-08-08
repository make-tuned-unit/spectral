# Prereg — relative date offsets in rendered context (2026-08-08)

**Status: committed before any measurement.** Rule 1.

> **CORRECTION 2026-08-08, after an 8-question aborted start and BEFORE any arm
> completed or any A-vs-B comparison existed.** Two defects in this document
> were found by launching arm A and reading its first lines, and both are fixed
> below rather than discovered afterwards:
>
> 1. **Query expansion was on.** The run log printed *"Query expansion enabled
>    (model: claude-haiku-4-5)"*. That is a **second variable**, and worse, one
>    with known nondeterminism — R14 expansion nondeterminism is precisely what
>    **voided the R11 stage-1 run**. Both arms now pass `--no-expand-queries`.
>    This makes both arms non-default in the same way, which preserves the
>    paired comparison of the one variable under test and removes a known
>    self-voiding hazard. It also means **this run does not measure the shipped
>    expansion-on configuration**, and no claim about it may be drawn.
> 2. **The cost estimate was wrong by ~4x.** $0.0127/question was taken from the
>    LoCoMo baseline, but LongMemEval contexts average **14,665 tokens** against
>    LoCoMo's 2,841. Measured: **$0.0478/question → ~$12.72** for both arms, not
>    $3.38.
>
> No result has been observed. The 8 aborted questions are discarded and both
> arms run fresh. Disclosed here rather than quietly amended, because "the
> prereg was adjusted once measurement started" is exactly the move this
> discipline exists to prevent — the mitigating fact is that nothing
> comparative was seen, and that fact is checkable: the aborted arm has no
> counterpart.

## Why this experiment exists

`RenderOptions::relative_offsets` is **built, tested at the contract level, and
off by default**. It annotates the date tag with a relative offset ("4 months
ago") computed from `question_date`.

It sits on the **only axis that has ever paid in this project**. R11 — the sole
preregistered, held-out accuracy win — was **+14.2pp on disjoint validation**,
and the entire effect was temporal-reasoning (20.0% → 62.5%), from **bare dates
alone**. The obvious follow-up question was never asked: if bare dates are worth
that much, is the *relative* framing worth more?

The capability has sat unmeasured since. This closes it in one direction or the
other.

## The lever — named and pinned

`SPECTRAL_DATED_CONTEXT=1`, which sets `RenderOptions::relative_offsets`
(`retrieval.rs:206`, `render.rs:123`). **Single variable.** It changes only the
date tag — pinned by `relative_offsets_are_opt_in_and_change_only_the_date_tag`
in `render_contract.rs`.

Nothing else moves: same retrieval, same k, same render mode, same models.

## The pre-registered expectation is NULL or NEGATIVE

Stated before running, because a stated hostile prior is what makes a positive
result mean anything:

1. **R11 established that the information is already present.** The context
   carries absolute dates and the prompt carries `question_date`. A relative
   offset is *derivable* from what the actor already has, so it adds no
   information — only a computation the model may or may not have been getting
   wrong.
2. **It costs tokens for redundancy.** Every date tag grows. On a context
   already averaging ~14k tokens that is not free.
3. **The literature points at verbatim timestamps, not derived annotations.**
   Timestamp-marked verbatim chunks score 50.2% on temporal questions against
   31.2% for extracted artifacts (arXiv 2601.00821). A relative offset is a
   small step toward the losing representation.

**A null here is the expected result and is publishable as one.** The reason to
run it anyway is that it is cheap, it is the last untested lever on the one axis
that has demonstrably paid, and "built but never measured" is exactly the state
this project exists to eliminate.

## Design

* **Population:** LongMemEval `temporal-reasoning`, **n = 133**. This is where
  R11's entire effect lived; if relative offsets do anything, it is here. Other
  categories were bit-flat under R11 and are excluded to keep the run cheap —
  which also means **this experiment cannot speak to them**, and no claim about
  them may be drawn from it.
* **Arms:** A = no relative offsets. B = `SPECTRAL_DATED_CONTEXT=1`.
  **Paired** — identical question set, identical retrieval. **Both arms run
  `--no-expand-queries`** (see the correction above): expansion is a second
  variable and a known source of cross-arm divergence.
* **Actor/judge:** `claude-sonnet-4-6`, **temperature pinned to 0** on both
  (`actor.rs:143`, `judge.rs:194`). The 2026-07-14 cascade A/B was rendered
  inconclusive by unpinned actor temperature; that defect is fixed and this
  run depends on it.
* **Scorer:** post-R21 (first-balanced-JSON-object). **This differs from the
  scorer the 65.02% LoCoMo baseline used** and the result doc must say so.
* **Commit:** SHA recorded in the result.

## Gate — significance REQUIRED

This authorizes an accuracy claim, so an effect size alone is not sufficient
(the distinction PR #239 got wrong).

| verdict | condition |
|---|---|
| **PASS** | McNemar exact test on the paired discordant pairs, **p < 0.05**, in favour of B |
| **NULL** | p ≥ 0.05 — recorded as a measured null, lever stays off |
| **REGRESSION** | p < 0.05 in favour of A — recorded, and the docstring's "untested" note is replaced with the measured harm |

n = 133 with a paired design detects roughly a 10pp shift at 80% power. **An
effect smaller than that will read as null here, and that is a limitation of
this run, not evidence of absence.** R11's effect on this slice was ~42pp, so
the design is well matched to the hypothesis actually being tested.

Context-token delta is **reported, not gated** — it is a cost, not the endpoint.

## Cost

**Measured: $0.0478/question → ~$12.72** for 133 questions × 2 arms.
LongMemEval contexts average 14,665 tokens, ~5× LoCoMo's, so the LoCoMo-derived
$0.0127 in the first draft of this document understated by ~4×.

Derived from the measured per-question cost of the LoCoMo baseline, **not** from
the binary's `--confirm-cost` pre-flight, which is a flat `$0.04/call` constant
(`eval.rs:97`) and overstates by ~6.5×. It will print ~$21 for this run; ignore
it.

## Publication commitment

The number is recorded whatever it is, in `MEASURED_RECORD.md` and a result
doc. No re-runs with different settings to find a better figure. If the run
reveals a config error, the corrected re-run is disclosed as such with both
numbers.

**Refs:** `r11-render-ab-stage2-result-2026-08-06.md` (the axis that paid),
`render.rs`, `render_contract.rs`, `landscape-research-2026-08-07.md` §G2.
