# Prereg — preference-question evidence retrieval (2026-08-07)

**Status: BLOCKED and BUDGET-GATED. Not run. No arm has been executed and no
result exists.** Committed before any measurement, per Rule 1.

Blockers, all of which must clear before stage 0 may run:

1. **R15 merged** — the endpoint of this experiment is R15's evidence-turn
   metric. Measuring against the 12.2×-diluted `answer_session_turns_*` would
   reproduce the exact defect this experiment exists because of.
2. **R16 merged**, which requires
   `recency_decay_is_order_invariant_in_the_topk_path` resolved (see R20).
   R16 shifts the retrieval baseline on 10/500 questions; a pre-arm measured
   before it is measuring a baseline we have already superseded.
3. **R20 adjudicated.** The candidate lever changes `fetch_mult`, which
   directly changes the size of the pool whose *rank position* becomes the base
   score (`ranking.rs:345-347`) — the same mechanism R20 names. An unresolved
   R20 means the lever and a known latent defect move together.

Budget sign-off is required separately, and only after stage 0 admits the
experiment.

---

## Why this experiment exists

R15 measured, at $0 from rows already on disk, that `single-session-preference`
retrieves **29 of 44 evidence turns = 65.9%**, with **9 of 30 questions
retrieving zero evidence at all**. Overall evidence-turn recall is 88.5%.
Preference is the single most localized retrieval gap the project has ever
measured, and it was invisible at session level — preference sessions are
short, so hitting the session was easy and looked like success (93.3% session
recall, published 2026-08-02 as "retrieval headroom is 6.7pp").

That published conclusion — *"~37pp of the preference gap is actor-side; total
retrieval headroom is 6.7pp"* — was computed on session recall. It is not
overturned by this document, and this document does not overturn it. What is
established is narrower and sufficient to justify a measurement: **for 9 of 30
preference questions the evidence never reaches the actor at all**, and no
actor-side or rendering work can fix those.

This is the first retrieval prereg in months with a specific, quantified,
pre-existing target rather than an intuition.

---

## The candidate lever — named and pinned before any measurement

**`CascadePipelineConfig::fetch_mult = 3`** (currently `1`;
`cascade_layers.rs:266`), applied on the cascade route for **all** shapes.

*Not* preference-only. A shape-conditional default would introduce two
variables (the widening, and the classifier's decision about which questions
get it) and would make a null uninterpretable. Preference is the subgroup we
predict will move; it is not the population under test.

**The prior on this lever is hostile, and saying so is part of the prereg.**
`cascade_layers.rs:254-265` records both halves:

* fm=3 is measured **retrieval-Pareto-safe and token-neutral**, and moved
  `single-session-preference` session recall 93.3% → 96.7% (2026-07-14).
* Its only end-to-end actor A/B was **directionally worse**: on
  `single-session-preference`, n=30, sonnet-4-6, fm=3 scored 14 fails vs
  fm=1's 11. That run was inconclusive — actor temperature was unpinned, so
  sampling noise swamps a ~5/30 retrieval delta — but it is evidence, and it
  points the wrong way.

The pre-registered expectation is therefore **null**. This experiment exists to
convert an inconclusive 2026-07-14 run into a decided one, in either direction.

**Any other lever requires a fresh prereg naming what changed and why.** A
failed gate here may not be followed by "try a tweak".

---

## Stage 0 — $0 oracle screen (a stop, never a pass)

Stage 0 cannot approve anything. It can only stop the experiment before money
is spent. Its thresholds are committed here so they cannot be adjusted after
the numbers are seen.

* **Design:** paired $0 oracle, 500 LongMemEval questions, reused brains at
  `~/spectral-local-bench/oracle-work`, single variable
  `SPECTRAL_CASCADE_FETCH_MULT=3`, run on the merge commit that lands R15+R16,
  SHA recorded in the result doc. Pre-arm must reproduce the post-R16 baseline
  at 0/500 context_hash diffs and reproduce *itself* at 0/500 before the post
  arm is read.
* **Endpoint:** R15 evidence-turn recall. Session recall and
  `answer_session_turn_coverage` are **reported but are not criteria** — they
  are the metrics that hid this defect.

**Proceed to stage 1 only if ALL of the following hold:**

| # | criterion | threshold |
|---|---|---|
| S0-1 | `single-session-preference` micro evidence-turn recall | **≥ +10.0pp** (29/44 = 65.9% → ≥ 75.9%, i.e. ≥ +5 evidence turns) |
| S0-2 | `single-session-preference` zero-evidence questions | **≤ 6** (from 9) |
| S0-3 | overall micro evidence-turn recall | **does not decrease** (≥ 793/896) |
| S0-4 | mean context tokens | **≤ +10%** (≤ 15,634 from 14,212.8) |

Anything else is a **STOP**: recorded as a null in the register and in
`MEASURED_RECORD.md`, no paid run, lever stays off by default. A stage-0 stop
is a publishable result, not a failure to produce one.

Stage 0 is deliberately a *screen*, not a gate. It carries no significance
requirement because it authorizes no claim — it only decides whether to spend.
No accuracy claim of any kind may be made from stage 0 output. This is the
distinction PR #239 got wrong in the other direction: an effect-size threshold
is fine for spending decisions and is **never** sufficient for a verdict.

---

## Stage 1 — paid, dev half (significance REQUIRED)

* **Population:** the 500 LongMemEval questions, split into two **disjoint**
  halves by a preregistered deterministic seed (`seed=42`, stratified by
  category so both halves carry ~15 preference questions). Half A = dev,
  half B = validation. The split is generated and its SHA256 recorded
  **before** stage 1 runs.
* **Arms:** A (`fetch_mult=1`, shipped) vs B (`fetch_mult=3`). Identical in
  every other respect. Actor and judge `claude-sonnet-4-6`, **temperature
  pinned to 0** — the unpinned temperature is the named reason the 2026-07-14
  run was inconclusive, and repeating that mistake would waste the budget.
  `--no-expand-queries` on both arms (R14: default-on expansion samples
  differently across arms and voided R11 stage 1).
* **Primary endpoint:** end-to-end accuracy over **all** questions in half A,
  paired per question.
* **Gate — both conditions, not either:**
  * paired delta **≥ +3.0pp**, AND
  * **McNemar exact two-sided p < 0.05**, with **b and c reported** (the
    discordant-pair counts), not only the p-value.
* Anything else — including "≥ +3.0pp at p ≥ 0.05" — is a **NULL**. That case
  *is* PR #239, and it stops here.

## Stage 2 — paid, disjoint validation half (significance REQUIRED)

Runs only if stage 1 passes. Half B, never previously scored for this lever.

* **Gate — both conditions:** paired delta **≥ +3.0pp** AND **McNemar exact
  two-sided p < 0.05**, b and c reported.
* **A PASS ships** `fetch_mult = 3` as the cascade default, recorded as a
  baseline shift with an oracle diff (Rule 6) and a consumer note.
* **A NULL ships nothing.** The register records the verdict, the lever stays
  an opt-in config field, and the 2026-07-14 inconclusive run is superseded by
  a decided one.

No arm C. No prompt iteration. No re-rolls. One candidate, one shot per stage.

---

## The preference subgroup is DESCRIPTIVE ONLY

`single-session-preference` is **n=30 total**, i.e. ~15 per half. **No
preference-only accuracy claim may be made from this experiment at any stage,
whatever the subgroup numbers look like.** The subgroup is reported because it
is the mechanism we predict, and reporting it is how the mechanism is falsified
— but it is powered for nothing. Any document quoting a preference-only
accuracy delta from this run is misusing it.

The preference *retrieval* numbers (stage 0, evidence-turn recall) are a
different matter: they are $0, deterministic, and have no sampling noise, so
they are quotable as retrieval measurements — and only as retrieval
measurements.

---

## Corpus honesty

The paid arms run on **LongMemEval, which is not held out.** It is the corpus
Spectral has been tuned against for months. It is used here because it is the
only corpus that carries labelled preference questions *and* per-turn
`has_answer` evidence labels — LoCoMo has neither (see R19).

Therefore: **a PASS here is a tuned-corpus result and must be published as
one.** It does not carry the standing of R11's held-out LoCoMo validation. If a
PASS occurs, the honest follow-up is a held-out confirmation, which needs a
corpus that does not yet exist (R10).

---

## Cost

| stage | questions × arms | rate | cost |
|---|---|---|---|
| 0 | 500 × 2, $0 oracle | — | **$0.00** |
| 1 | 250 × 2 = 500 | ~$0.08/q (`BENCH_METHODOLOGY.md`) | **~$40** |
| 2 | 250 × 2 = 500 | ~$0.08/q | **~$40** |

**Ceiling $80**, sequential, abort-early, no other paid run rides along. Stage
2 requires its own sign-off after stage 1 passes — a stage-1 pass does not
pre-authorize stage 2's spend.

Given the hostile prior and the four-way stage-0 conjunction, the most likely
total spend on this document is **$0.00**. That is the intended behaviour.

---

## What gets reported, unconditionally

1. Stage 0: all four criteria with their measured values, pass or fail.
2. Every arm's overall and per-category accuracy, and both McNemar counts.
3. The evidence-turn recall of both arms (the $0 companion).
4. Context-token cost of the widening.
5. Any transport, auth or judge-parse failure, with counts.
6. The verdict, including if it is a null, and including if it makes the lever
   look worse than shipped.

A null is recorded in `MEASURED_RECORD.md` with the same prominence a pass
would receive (Rule 5).

---

## Refs

* `turn-level-evidence-recall-2026-08-07.md` — the measured target
* `r15-evidence-metric-2026-08-07.md` — the endpoint's implementation
* `cascade-fetch-mult-lever-2026-07-14.md` — the hostile prior
* `policy-v2-result-2026-08-02.md` — the session-recall-based conclusion this
  does not overturn
* `r11-render-ab-prereg-2026-08-05.md` — the two-stage disjoint-split model
* `research-alignment-2026-08-07.md` §7 — why the originating spec was rejected
* `REPAIR_REGISTER.md` R15, R16, R20
