# R29 — adjacency beats equal-budget k-raising on the production path · **PASS**

**$0. Retrieval-only oracle, LoCoMo, full N = 1,438, `--retrieval-path cascade`,
R19 turn labels. No model calls, model-free.** Preregistered at `1eb7c39` before
any arm ran; grid amendment registered at `5c9ba77`, before any recall number
was read.

This is the comparison R28 could not make, and the last thing standing between
the adjacency work and an honest production claim.

## Result

| arm | config | evidence recall | tokens | vs c0 |
|---|---|---:|---:|---:|
| `c0` | cascade defaults | 58.60% | 1,500 | — |
| `c_kmult` | `K_MULT=2.5` | **69.25%** | **3,493** (2.33×) | +10.65pp |
| `c_adj` | `ADJACENCY=1` | **76.82%** | **3,401** (2.27×) | +18.22pp |

**Primary comparison — `c_adj` vs `c_kmult` at equal budget:**

> **+7.57pp** (76.82% vs 69.25%), **p < 0.0001**, discordant **169 for / 46
> against**, token ratio **−2.6%** (well inside the ±10% band).

**Verdict: PASS**, on the rule fixed in the prereg (`c_adj` > `c_kmult`,
p < 0.01, tokens within ±10%).

Multi-session, the hard slice: **58.86% vs 53.71%, +5.15pp.** Zero-evidence
questions: **143 vs 257.** Rank of first evidence turn: 483 promoted, 236
demoted.

## Both predictions in the prereg were right

I predicted `c_kmult` would land at **68–72%** and adjacency would survive at
**+5 to +9pp**. Measured: **69.25%** and **+7.57pp**. Recording this because the
prereg's predictions have been wrong before (Track C's H1, and the adjacency
mechanism the same day), and a register that only notes the hits is worthless.

## It replicates the topk number

R25's token-matched control on `topk_fts` gave **+6.73pp**. Cascade gives
**+7.57pp**. The lever's honest, cost-matched value is **~+7pp on both paths** —
and it is marginally *stronger* on the path Permagent actually calls.

## What must be said alongside it

**Plain k-raising is itself worth +10.65pp on cascade for 2.33× tokens**, and
that had never been measured before today. Adjacency's real claim is not
"+18.22pp"; it is **+7.57pp on top of a cheaper, dumber lever that already
delivers most of the way.** Anyone comparing against a k=40 baseline is being
shown the flattering number.

**At equal budget, adjacency is no longer free.** R28's headline included "331
nonzero pairs, all positive — not one question got worse". That property does
**not** survive token-matching: **46 questions retrieve full evidence under
k-raising and fail to under adjacency.** It wins 169 to 46, decisively, but it
is a trade now, not a Pareto improvement. That distinction should not be lost
in the summary.

## Preconditions verified, not assumed

- **Binary equivalence: 0/100 `context_hash` diffs** between the rebuilt binary
  on defaults and the archived `c0`. `target/` vanished four times last session
  and `retrieval.rs` was edited to add the lever, so reusing the R28 arms rather
  than re-running them required proof, not confidence.
- **Calibration was blind to the outcome.** `calibrate_token_match.py` reads
  `context_tokens_est` and never reads recall. `m=2.5` was picked at +2.7% off
  target, an interior point of the grid — so the prereg's endpoint rule did not
  fire and the registered refinement amendment was never needed.
- **The control preserves the tuned profile.** `SPECTRAL_CASCADE_K_MULT` scales
  each question shape's own k rather than flattening all shapes to one value, so
  a deficit cannot be blamed on destroying question-type routing. That confound
  would have pointed in favour of our own lever.

## What does NOT follow

- **No accuracy claim, and no default change on any path.** A PASS licenses a
  *proposal* to Permagent. Registered before the result was known, and it still
  holds.
- **Retrieval only.** At 2.27× context the reader-dilution question is
  unmeasured and unbudgeted, and it is the only question a consumer has. Nothing
  here says answers improve.
- **Mean-matched, not per-question-matched**, as the prereg declared.
- **Corpus-shaped, and now quantified.** The same-day mechanism diagnostic found
  adjacency is *indifferent to lexical overlap* — it works by being orthogonal
  to the lexical channel, not by attacking coreference — so its value depends on
  dialogue geometry. Offline pricing puts its LongMemEval ceiling at **+5.80pp**
  against LoCoMo's measured +18.22pp, because that corpus is already at 88.50%.
  **This +7.57pp is a LoCoMo number and should be quoted as one.**

**Refs:** `cascade-token-match-prereg-2026-08-11.md`,
`cascade-transfer-result-2026-08-10.md` (R28),
`turn-adjacency-result-2026-08-10.md` (R25),
`adjacency-mechanism-diagnostic-2026-08-11.md`,
`adjacency-second-corpus-pricing-2026-08-11.md`.
