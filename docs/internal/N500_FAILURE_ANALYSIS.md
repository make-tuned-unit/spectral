# N500_FAILURE_ANALYSIS — failure autopsy of the shipped-config primary run

Date: 2026-06-15 | Branch: bench/stable-fail-2x2 | READ-ONLY, no API.
Source: `~/spectral-local-bench/eval-report-n500.json` — the n=500 primary run
(expansion-ON, `--use-cascade`, sonnet-4-6 actor+judge, cdd793e). **80.9% overall**
(398/492 evaluated; 8 transport failures excluded). This is the FIRST failure
classification under the shipped expansion config; prior autopsies were
expansion-OFF.

Taxonomy (established): retrieval-starved / synthesis-bound / judge-strict /
GT-defect / dilution / other. Per-failure evidence signal: `ans_keys` =
`answer_`-prefixed (evidence-session) keys in the retrieved set.

## The 94 failures

| category | failures | n (evaluated) | failure rate |
|---|---|---|---|
| multi-session | 34 | 131 | 26% |
| temporal-reasoning | 23 | 133 | 17% |
| single-session-preference | 12 | 25 | 48% |
| single-session-user | 10 | 70 | 14% |
| knowledge-update | 10 | 78 | 13% |
| single-session-assistant | 5 | 55 | 9% |

**Only 5/94 retrieved ZERO answer-keys; median answer-keys retrieved = 11.**
Under expansion-ON the evidence is almost always *present* in context — so the
failure mass is synthesis, not retrieval.

## Mechanism distribution (primary bucket per failure)

| mechanism | count | share | where |
|---|---|---|---|
| **synthesis-bound** | ~62 | 66% | all TR date-math/ordering (~20), MS counting/value-selection (~26), KU update-application (~9), SSP no-tailoring (~6), abstention premise-miss (~1) |
| **retrieval-starved** | ~16 | 17% | single-session factual/recall where the one answer turn is missed (SSU 4, SSA 3, SSP ~3), plus MS multi-operand `c18a7dc8` and vocab-wall `ba358f49`, TR `gpt4_e061b84f`/`71017277`/`gpt4_8279ba03` |
| **judge-strict / harness** | ~9 | 10% | ~3 evaluation-bug (correct answer scored vs wrong question), ~6 strict-rubric (abstention not citing contrastive fact; correct answer + appended noise; range-includes-answer) |
| **GT-defect** | ~2 | 2% | `75f70248` (Luna/deep-clean not in any answer session), `0ddfec37` (GT=15 but judge applies "most-recent=20") |
| **dilution** | ~2 | 2% | `73d42213` (wrong session's time picked from noisy context) |

(~8–10 cases sit on a synthesis/retrieval boundary — "I don't know" with low
ans-keys; counts are ±, but the conclusion below is robust to the boundary.)

### How this differs from the expansion-OFF laggard autopsy

The expansion-OFF LAGGARD_AUTOPSY put ~31% of laggard failures as
retrieval-fixable. Under shipped expansion that slice **collapsed to ~17%**:
expansion surfaced the operands (median 11 answer-keys), converting former
retrieval-starved cases into synthesis exposures. This is exactly the
cross-check prediction — the shipped expansion already closed most of the
retrieval headroom, and what remains is dominated by synthesis. The two
laggards confirmed: MS 74% (counting synthesis) and SSP 52% (no-tailoring +
volatile n=25).

## Concentration: which fix flips the most cases (weighted by n)

- **temporal-reasoning synthesis (~20 cases, n=133):** date arithmetic
  ("N weeks ago" misread), multi-event ordering (drops/misorders events), and
  wrong-event identification. The largest single synthesis cluster. Pure actor
  reasoning over already-retrieved evidence.
- **multi-session counting/value synthesis (~26, n=131):** off-by-one miscounts
  under the reasoning-aware tolerance, wrong-value selection (Walmart vs Thrive),
  overcounts/undercounts. Volume lives here.
- **single-session-preference (12, n=25):** half the category — but low absolute
  count and volatile; ~half no-tailoring synthesis, ~half retrieval-starved
  (preference turn missed). Not where volume is.

The single mechanism whose fix flips the most cases is **synthesis** (~62) — but
see the cross-check: it is already-rejected territory.

## Cross-check against already-shelved / exhausted levers

| failure cluster | already-closed by | new lever? |
|---|---|---|
| TR date-math/ordering synthesis (~20) | model-capability-bound (BENCH_METHODOLOGY limitation 5); no architecture lever | **NO** |
| MS counting synthesis (~26) | extract→operate SHELVED at 7/26 ("no cheap architectural lever") | **NO** |
| KU update-application (~9) | same synthesis class (pick most-recent / detect false premise) | **NO** |
| retrieval-starved (~16) | K-residual Phase 0 STOP + RESIDUAL_FLOOR (free retrieval levers exhausted); `ba358f49` vocab-wall is architectural | **NO** |
| **judge-strict / harness (~9)** | **not closed by any prior analysis** — rubric/eval, not architecture | **YES** |

## Judge-strict audit (the one free-upside cluster)

Splitting the ~9:

**(a) Evaluation bug — correct answer scored against the wrong question (~3):**
- `55241a1f` (MS): actor computed 12+21 = **33**, which **equals GT 33** — but the
  judge reasoning evaluates a *Sony lens purchase* question. Scored wrong.
- `8b9d4367` (SSA): actor answered **"Jaipur Rugs" = GT**, but judge says "no
  ground truth provided" and discusses "Bajaj Auto." Scored wrong.
- `b6025781` (SSP): judge evaluates a *gift-for-mom* question against a meal-prep
  answer. Mismatched pairing.
These are a **harness/judge question↔answer pairing bug**, not model failures —
the answers are right. ~3 free recoveries (+0.6pp).

**(b) Strict-rubric on genuinely-near-correct answers (~6):**
- `0862e8bf_abs`, `bc8a6e93_abs`, `f4f1d8a4_abs`, `29f2956b_abs` (SSU/abstention):
  actor correctly **abstained** ("I don't know" / "no match") but was marked
  wrong for not naming the *contrastive* fact ("you mentioned X but not Y").
- `4baee567` (SSA): answered **"12 games" = GT** but penalized for appended
  off-topic text.
- `09ba9854` (MS): gave a range **including the $50 GT** but penalized for not
  being exact.
These are debatable: LongMemEval's abstention rubric legitimately wants the false
premise recognized, so "I don't know" is arguably incomplete — but the answers
are not *wrong*. Rubric tuning could recover ~3–6 (+0.6–1.2pp).

**Judge-cluster ceiling: ~9 cases ≈ +1.8pp → ~82.7%.** Of which ~3 are an
unambiguous eval bug worth fixing regardless.

## VERDICT — next levers ranked by (cases recoverable × cheapness)

1. **Fix the judge/harness question↔answer pairing bug** (`55241a1f`, `8b9d4367`,
   `b6025781`). ~3 cases, **free** (a harness/eval defect, not architecture), and
   a genuinely NEW lever no prior analysis closed. **Do this** — it's also a
   correctness bug in the eval itself, independent of the score.
2. **Abstention-rubric review** (~4–6 cases). Cheap (rubric/prompt), partially
   legitimate; expect ~3 recoverable. Worth a look, not load-bearing.
3. **Everything else (~78 cases): no new cheap lever.** Synthesis (~62) is the
   already-shelved extract→operate class (7/26) plus model-capability-bound
   temporal/counting reasoning; retrieval-starved (~16) is the architectural
   floor established by K-residual STOP and RESIDUAL_FLOOR (incl. the `ba358f49`
   vocabulary wall).

**Honest bottom line:** beyond the ~9-case judge/harness cluster (ceiling
~82.7%, ~3 of them a real eval bug), there is **no new cheap lever**. The
residual is the known architectural floor — synthesis-bound reasoning over
correctly-retrieved evidence, which the shelved extract→operate work and the
methodology's stated model-capability limitation already cover. That is the
limitations section, not a failure: 80.9% is at the top of the pre-registered
~79–82% band, and the gap to higher is dominated by actor synthesis the project
has already determined has no cheap architectural fix.
