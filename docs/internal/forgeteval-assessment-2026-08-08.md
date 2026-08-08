# ForgetEval — assessment, and its prediction tested against us (2026-08-08)

**$0.** No harness run, no dataset downloaded, no model calls. What was tested
is the benchmark's **hypothesis about systems like ours**, which is free.

## What ForgetEval is

*Control-Plane Placement Shapes Forgetting: An Architectural Study of Agent
Memory Across Thirteen System Configurations* (arXiv 2606.15903, MIT licence,
publicly released).

- **1,000-case templated suite + a 385-case adversarial layer** (132
  hand-crafted, 253 LLM-drafted and oracle-validated), with external validation
  from four blind contributors.
- **Deterministic substring scoring** — no LLM judge. Given that our own
  baseline had to carry a caveat that the standard judge accepts 62.8% of
  wrong-but-adjacent answers, a benchmark with no judge is worth more to us
  than one with a better judge.
- **Six-method adapter protocol**, claimed to admit a heterogeneous store in
  ~130 lines.
- Its research question **is** our thesis: deterministic primitives versus LLM
  control planes.
- Its framing claim is also directly relevant: *production failures are
  predominantly forgetting failures, not recall failures, yet existing
  benchmarks measure only recall.*

## Its prediction about us

ForgetEval reports that deterministic stores hold the lexical and temporal
categories and then fail one specific category — **canonicalization** — at
**5% on identifier-obfuscation and 0% on cross-lingual**, recoverable only by
an **inscribe-time** LLM. It also finds a mutation-time hook reaching 78–85% on
intent-aware deletion and 91.7–93.2% overall.

That is a falsifiable prediction about a system built exactly like ours. So we
tested it on ours instead of assuming it transfers.

## Result — `crates/spectral/tests/canonicalization_gap.rs`

| category | ForgetEval (deterministic stores) | **Spectral, measured** |
|---|---|---|
| lexical (control) | holds | holds |
| identifier obfuscation — case (`SK-…`) | ~5% | **retrieves** |
| identifier obfuscation — separator (`sk_…`) | ~5% | **retrieves** |
| identifier obfuscation — concatenation (`skABC…`) | ~5% | **misses** |
| cross-lingual (fr/es/de/pt) | **0%** | **0% — reproduced exactly** |

**We are better than predicted on identifier obfuscation, and exactly as
predicted on cross-lingual.**

The identifier result was **not** what we assumed going in — the first version
of the test asserted that separator substitution fails, and it was wrong.
FTS5's `unicode61` treats both `-` and `_` as separators and case-folds, so
`sk-ABC123XYZ`, `sk_ABC123XYZ`, `SK-ABC123XYZ` and `sk-abc123xyz` all tokenize
to the same terms. The gap appears only when the token **boundary itself** is
removed (`skABC123XYZ`), which produces a term that shares nothing with the
index. The test now records the measured behaviour, including the correction.

**Cross-lingual reproduces the 0% exactly**, and one further test establishes
*why it is not fixable from our side of the pipeline*:
`the_canonicalization_gap_is_admission_not_ranking` runs the French query at
`k=500` with a 12× pool and still cannot admit the English memory. No query
term matches, so there is no candidate to rank. This is the same
admission-versus-ranking distinction that decided G4 — and it is why
ForgetEval finds the effective fix to be **inscribe-time**, not read-time.

## What this means for the thesis

It draws the boundary of the zero-inference claim precisely, which is more
useful than defending it:

- **Inside the boundary:** lexical, temporal, identifier variants up to token
  boundaries. Deterministic primitives hold, at $0 and ~1 ms.
- **Outside it:** cross-lingual, and any canonicalization that requires knowing
  two surface forms mean the same thing. **No ranking signal can reach these** —
  proven, not asserted. A term index cannot match a word it has never seen.

Closing that gap requires inference at **write** time, which is exactly the
property the BM25 baseline exists to measure the absence of. So it is a genuine
trade, not an oversight, and it should be stated as one wherever the
zero-inference claim appears.

Cheaper partial mitigations exist and are **untested**: the alias file
(`query_aliases`, already a shipped capability) is a deterministic, consumer-
supplied canonicalization channel that could cover known identifier and
terminology variants without any model. It cannot cover open-ended
cross-lingual.

## Should we run the actual benchmark?

**Yes, and it is the best remaining external move** — but it is a real piece of
work, not a free win, and the honest reasons are:

**For.** Deterministic scoring removes the judge, which is the largest single
source of noise in our LoCoMo number (25.6pt cross-harness spread, 62.8% judge
leniency). It measures *forgetting*, where we have real assets nobody else
does — verified deletion with a byte-scan test, `Brain::vacuum`, and the
deletion proof suite that found and fixed two real leaks. And its research
question is ours, so a result is interpretable either way.

**Against, stated plainly.** The published numbers say a deterministic store
lands around **63.4%** on ForgetEval-Adv (Lethe 63.4%, LangGraph 62.9%) while
the mutation-time-hook configurations reach **91.7–93.2%**. We should expect to
land in the former group and should say so *before* running, not after.

**Cost:** the six-method adapter (~130 lines claimed) plus a scoring harness.
No model spend for the deterministic-substring path.

**Recommendation:** preregister it — arm, expected range including the ~63%
prior, and the commitment to publish — then implement the adapter. Do **not**
run it first and write the prereg afterwards; that is the failure mode this
project exists to avoid, and today already produced one example of reasoning
past what a metric could carry.

## Limits of this assessment

The benchmark harness was **not** run and its dataset was **not** downloaded.
Categories it names that we did not probe — prefix-collision, compound-fact,
intent-aware deletion — are untested here. The scores quoted for other systems
are from the paper's abstract, not reproduced. What is established is narrow:
**ForgetEval's canonicalization prediction holds against Spectral for
cross-lingual and is too pessimistic for identifier obfuscation.**

**Refs:** arXiv 2606.15903, `landscape-research-2026-08-07.md` §5,
`g4-proximity-result-2026-08-08.md` (the same admission-vs-ranking split),
`crates/spectral/tests/canonicalization_gap.rs`.
