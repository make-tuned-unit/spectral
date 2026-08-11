# R29 — PREREG: the token-matched control on the production cascade path

**Registered 2026-08-11, before any arm ran.** $0, retrieval-only oracle,
LoCoMo, full N = 1,438, `--retrieval-path cascade`, R19 turn labels. No model
calls.

## The question

R28 measured turn adjacency on `recall_cascade` — the only path Permagent calls
— at **+18.22pp evidence recall for 2.27× tokens** (58.60% → 76.82%, 1,500 →
3,401 tokens). That number is **cost-unmatched**, and R25 already showed what
cost-matching does to this lever on `topk_fts`: **+18.93pp collapses to
+6.73pp** once the control is allowed to spend the same context.

So the honest cascade question is not "does adjacency help" — R28 settled that.
It is:

> **At equal token budget, does adjacency beat simply retrieving more?**

Everything downstream of a production claim depends on this and nothing else
answers it.

## Why this needs new code

R25's control was constructible because `--max-results` sets k on `topk_fts`.
On cascade, k comes from the question-type profile (`QuestionShape::
cascade_profile`): Counting 60, Temporal 40, Factual 30, General 40.

`SPECTRAL_CASCADE_K` already exists but **flattens every shape to one k**, which
would confound the control: any deficit could be blamed on destroying the tuned
per-shape profile rather than on k-raising being the weaker way to spend tokens.
That is a confound in *favour of our own lever*, which is the direction we are
least entitled to.

So the primary control uses a new **`SPECTRAL_CASCADE_K_MULT`** — a float that
scales each shape's own k, preserving the profile's shape. This is the fairest
possible version of "spend 2.27× the plain way".

## Arms

| arm | config | status |
|---|---|---|
| `c0` | cascade defaults | **already measured** (R28) — 58.60%, 1,500 tok |
| `c_adj` | `SPECTRAL_ADJACENCY=1` | **already measured** (R28) — 76.82%, 3,401 tok |
| `c_kmult` | `SPECTRAL_CASCADE_K_MULT=m` | **NEW — the control** |
| `c_kflat` | `SPECTRAL_CASCADE_K=k` | secondary, optional |

`c0` and `c_adj` are reused as-is. They were produced by `run_cascade_transfer.
sh` at full N under the same binary and labels; re-running them would spend
hours to reproduce identical rows.

## Calibrating `m` — declared before it is chosen

`m` is picked by a **calibration sweep, not by the outcome**, and the rule is
fixed here so the pick cannot be made post-hoc:

1. Sweep `m ∈ {1.5, 2.0, 2.5, 3.0}` at **N = 100** (the first 100 questions).
2. For each, compute mean `context_tokens_est`.
3. Compute `c_adj`'s mean tokens **over the same 100 `question_id`s** (not its
   full-N mean — subsets differ).
4. **Pick the `m` whose mean is closest to that target.** Ties → lower `m`.
5. If the best `m` is at an endpoint of the sweep, extend the sweep in that
   direction rather than accepting an edge fit.

Calibration is **not a result** and will not be reported as one. If the chosen
`m` fails the ±10% token match at full N, the run is **INCONCLUSIVE** and gets
re-calibrated — it is not reported as a PASS.

### AMENDMENT, 2026-08-11, registered before any recall number was read

**The grid above is too coarse and I should have seen it when I wrote it.** The
target is 2.28×. If tokens scale near-linearly in `m` — which R27 found on topk,
where k=40→105 (2.62× k) cost 2.62× tokens — then `m=2.0` lands ≈−12% and
`m=2.5` lands ≈+10%, i.e. **both candidates straddle the ±10% band and the grid
may contain no admissible point at all**. A prereg that can only return
INCONCLUSIVE is a badly designed prereg.

**Amendment:** after the grid runs, a **refinement point** may be interpolated
on the token axis — fit `m` against measured mean tokens from the completed
arms, solve for the target, run that single `m` as one further calibration arm,
and apply the same nearest-token rule over the enlarged set.

This stays outcome-blind by construction: `calibrate_token_match.py` reads
`context_tokens_est` and **never reads recall**, so no refinement can be steered
by the answer. What is amended is the *grid*, not the selection rule, the
metric, the statistic, or any verdict threshold. Recorded here rather than
applied silently.

## Primary comparison

**`c_adj` vs `c_kmult`.** Both at ~3,400 mean tokens.

- **Metric:** micro evidence-turn recall (R19 labels). Not answer-session
  recall — that metric is 35pp diluted and is what made six levers read as
  noise.
- **Statistic:** exact two-sided McNemar on the paired per-question
  "all evidence turns retrieved" indicator, with discordant counts always shown.
- **N:** full 1,438. **Never N=250** — that subset is ~5pp easier than the
  corpus and produced a wrong null in R23.

## Verdict rules — fixed now

| condition | verdict |
|---|---|
| `c_adj` > `c_kmult`, p < 0.01, token ratio within ±10% | **PASS** — adjacency earns its cost on production |
| `c_kmult` ≥ `c_adj` | **REFUTED** — adjacency is just k-raising with extra steps on this path |
| p ≥ 0.01 | **NULL** — no separation at equal budget |
| token ratio outside ±10% | **INCONCLUSIVE** — recalibrate, do not report |

A PASS licenses a **proposal** to Permagent. It does **not** flip a default on
any path, and it is **not** an accuracy claim.

## What this cannot show, registered in advance

- **Retrieval only.** At 2.3× context, whether the reader answers *better* is
  unmeasured and unbudgeted. This is the question a consumer actually has and
  nothing in this programme answers it.
- **Mean-matched, not per-question-matched.** Two arms with equal mean tokens
  can distribute that budget differently across questions. Declared, not fixed.
- **Corpus-shaped.** Two-party strictly-alternating dialogue is adjacency's
  ideal case. R24 passed on LoCoMo and provably does not transfer to
  LongMemEval; adjacency is exposed to the same risk and this run does not
  address it.
- **Bench-scoped implementation.** `apply_turn_adjacency` parses the harness key
  format; production needs real sequence metadata.

## Predictions, on the record

Based on R25's topk collapse (+18.93 → +6.73pp) and R27 (the k axis is
dominated by adjacency on topk), I expect **c_kmult ≈ 68–72%** and adjacency to
survive with **roughly +5 to +9pp**. If adjacency lands below +3pp the
production case is materially weaker than R28 reads, and that must be said
plainly.

**Refs:** `cascade-transfer-result-2026-08-10.md` (R28),
`turn-adjacency-result-2026-08-10.md` (R25),
`k-admission-frontier-result-2026-08-10.md` (R27).
