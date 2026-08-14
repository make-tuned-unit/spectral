# R30 — query-alias vocabulary bridging · **FAIL on the gate, small real transfer** (2026-08-13)

**$0. Retrieval-only oracle, LoCoMo full N = 1438, `topk_fts`, R19 labels,
binary `fa5763d`. Preregistered at `71cb543` before any arm ran** —
committed first, closing the process gap recorded the same morning.

## Result

| half | base | arm | Δ | pairs | p |
|---|---:|---:|---:|---:|---:|
| derivation (fitted, no verdict) | 59.13% | 62.37% | **+3.24pp** (+35) | 32 [+29/−3] | 2.1e-05 |
| **test (PRIMARY)** | 60.60% | 62.30% | **+1.70pp** (+18) | 23 [+19/−4] | **0.0045** |

**Gate (test half): p<0.05 AND ≥+2.0pp → FAIL.** The significance clause
passes; the effect-size clause does not.

Zero-evidence questions on the test half: **178 → 166**. Context churn:
386/1438 questions changed context for a net +53 evidence turns corpus-wide —
a wide perturbation for a small net gain.

## Preconditions

- A0 reproduces the tiebreak-verification `base_topk` rows **0/400 hash
  diffs**, and reproduces the published corpus record **to the digit**
  (59.86% micro / 68.63% macro / 357 zero-evidence) — the strongest
  validation yet that the regenerated dataset, new host, and `fa5763d`
  binary are the measured configuration.
- Lever proven-on before the arm: 4/10 contexts changed in a 10-question
  probe.
- Split hygiene enforced in code: `extract_r30_derivation_misses.py` refuses
  to emit test-half rows, so authoring physically could not consult them.

## How the prediction did

Recorded before running: *"test half fails the gate, under +1.0pp."* **Half
right.** The gate verdict was called correctly, but the effect is +1.70pp and
clearly significant — nearly double the predicted ceiling. The reasoning
error is instructive: I argued bridging only converts 0-shared turns into
1-shared turns, and 1-shared turns mostly stay missed. But several alias
words are *rare* terms ("tourney", "sprained", "pottery"-adjacent
vocabulary), and one rare shared term ranks a turn high — the BM25-IDF case
the argument ignored.

## Reading the number honestly

- **This is the best-transferring lexical lever measured on this corpus** —
  +1.70pp held-out against declarative's +1.36pp (R26 A3′, fitted-corpus) —
  and it is still a rounding error against the 39.40pp missing-evidence
  opportunity, exactly the A3′ situation: *statistically unambiguous,
  practically marginal.* The effect-size clause is again what keeps that
  distinction visible.
- **The dilution cost is real and specific:** all four test-half regressions
  are alias-admitted competitors displacing evidence ("movies"→film,
  "games"→gaming, "visit"→trip, "tournaments"→competitions); one question
  fell to zero evidence. 19-vs-4 is a good trade, but it is a trade.
- **The fitted ceiling is low.** Even authoring the table while staring at
  the derivation misses buys only +3.24pp there. The zero-overlap failure
  family is inference-shaped, as the same-day diagnostic suggested (61.3% of
  missed turns share zero content words, and the examples are "problems
  before adopting Toby" ↔ "contacting landlords"). A word↔word table cannot
  say that.

## Verdict

**FAIL as preregistered; default stays off on every path**
(`query_aliases_path = None`; the env lever remains bench-scoped opt-in).
The residue of R22 now reads: vocabulary bridging is measured and small;
answer-shape matching is the one remaining untested $0 idea; beyond that the
frontier is a second modality, as the adjacency mechanism diagnostic argued.

## What does NOT follow

- No accuracy claim (retrieval only). No cascade claim (`recall_cascade`
  unmeasured; a FAIL licenses nothing there anyway).
- No claim about consumer corpora: project jargon and product-name aliases
  on a real brain are a different, plausibly stronger use of the same
  channel — this measures generic-English bridging on LoCoMo only.
- The alias table (committed beside this doc as
  `r30-aliases-table-2026-08-13.json`) is an experimental artifact fitted to
  LoCoMo's derivation half. **It must not ship as a default table.**

## The table, with its motivations

51 keys, generic English only, no proper nouns, no numbers. Families and the
derivation examples that motivated them:

| family | keys | derivation trigger (question ↔ missed turn) |
|---|---|---|
| morphology | buy/bought, win/won, meet/met, write/wrote, promoted/promotion | "equipments did John buy" ↔ "I bought a mouse" |
| synonym | tournament↔tourney/competition, organize↔host/held, movie↔film, class↔course, exercise↔workout, accident↔crash, degree↔graduated, relax↔unwind, depart↔leave, collect↔collection, ailment↔illness, injury↔hurt/twisted/sprained | "charity tournaments organized" ↔ "held a gaming tourney" |
| travel cluster | visit/visiting/vacationed↔trip/travel/vacation | "places was Evan visiting" ↔ "road trip to the Rockies" |
| food cluster | food↔meal/dish/eating, cook↔recipe | "What food…" ↔ "salads, sandwiches, desserts" |
| closed hypernym | pet(s)↔dog/cat/pup/kitten, kids↔children | "How many pets" ↔ "my dogs" |

**Refs:** `query-aliases-prereg-2026-08-13.md`,
`diagnose_lexical_misses.py` output in the prereg,
`rrf-composition-result-2026-08-09.md` (R22, which queued this),
`full-n-recheck-result-2026-08-09.md` (A3′, the twin profile).
