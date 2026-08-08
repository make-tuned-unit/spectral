# Deep failure analysis — where the lift actually is (2026-08-08)

**$0.** Computed entirely from the published baseline's own output, the R19
turn labels, and the archived k-sweep arms. No model calls, no new runs.

## 1. The headline: retrieval is worth +23pp, and we can price it

Decomposing all 1,436 labelled LoCoMo questions by how much of their evidence
actually reached the actor:

| evidence retrieved | n | share | accuracy |
|---|---:|---:|---:|
| **none (0%)** | 357 | 24.9% | **21.29%** |
| partial | 181 | 12.6% | 35.36% |
| **full (100%)** | 898 | 62.5% | **88.31%** |
| overall | 1,436 | | 64.97% |

**`P(correct | full evidence) = 88.31%`.** That is the reader ceiling on this
benchmark, and it sits just under the ~93.6% practical maximum imposed by
LoCoMo's ~6.4%-wrong answer key.

So **perfect retrieval is worth +23.34pp** (64.97% → 88.31%), and the residual
11.69pp belongs to the reader, the answer key and the judge — not to us.

Per category, the available lift is wildly uneven:

| category | accuracy | ceiling | zero-evidence | lift available |
|---|---:|---:|---:|---:|
| multi-session | 39.4% | 76.9% | 87 (31.2%) | **+37.5pp** |
| single-session-user | 70.2% | 88.9% | 200 (23.8%) | +18.7pp |
| temporal-reasoning | 73.7% | 89.4% | 70 (22.2%) | +15.7pp |

## 2. How much of it is reachable

Comparing k=40 against k=500 on the same brains splits the zero-evidence
failures cleanly:

- **66% are RANKING failures** — the evidence is in the match set, below the cut.
- **34% are true vocabulary misses** — absent at any depth. This is the lexical
  floor, and no ranker touches it.

Projected to the full set: ~201 questions are ranking-fixable. Moving them from
21.3% to 88.3% is **~+9.4pp overall (65.0% → ~74.4%)**, deterministically and
with no read-time model calls. The remaining ~103 need vocabulary bridging.

**But you cannot get there with k.** For the recoverable questions the first
evidence turn sits at a **median rank of 99** (mean 134, max 356):

| k | recovers |
|---:|---:|
| 60 | 14.3% |
| 80 | 37.1% |
| 100 | 51.4% |
| 200 | 82.9% |
| 300 | 97.1% |

k=200 would recover 83% at ~5× the token cost — which is why the k-admission
lever was rejected in 2026-07-20 and why that rejection **survived** re-testing
on the corrected metric. A ranker has to compress rank ~99 into the top 40.

## 3. Why every lexical lever failed — one explanation for all of them

Comparing the 50 missed evidence turns against the 1,400 distractors that
outranked them:

| feature | missed evidence | distractors | ratio |
|---|---:|---:|---:|
| query-term overlap | 0.460 | 0.998 | **0.46 — inverted** |
| IDF-weighted overlap | 1.350 | 2.885 | **0.47 — inverted** |
| token length | 18.58 | 16.28 | 1.14 |
| has a date | 0.060 | 0.071 | 0.85 |
| has a number | 0.000 | 0.014 | 0.00 |
| **first-person** | **0.860** | **0.606** | **1.42 — evidence higher** |

**The missed evidence has half the lexical overlap of the documents that beat
it.** BM25 is not malfunctioning; it is ranking correctly by its own criterion.
On these questions, lexical overlap is simply a poor proxy for
answer-bearing-ness.

That single fact explains the entire null streak at once. **Porter stemming,
pool widening, associative spreading, ACT-R decay, cascade-k, term proximity** —
every one of them is a lexical or lexical-adjacent refinement. Refining a
signal that is *pointing the wrong way* on these cases cannot recover them, no
matter how it is weighted. G4 made this concrete: proximity separates evidence
from distractors 22.6× in isolation and still bought nothing, because it is
redundant with BM25 and blind to the 88.8% of deep misses that carry at most
one query term.

**The correct conclusion is not "the retrieval lever family is exhausted." It
is "the LEXICAL lever family is exhausted."** Non-lexical deterministic signals
are almost entirely untested, and the one measured here separates.

## 4. The one signal that inverts

**First-person declarative content**: 86% of missed evidence turns vs 61% of
the distractors that outranked them, a 1.42× separation, and — critically —
**orthogonal to lexical overlap**, which is the property every failed lever
lacked.

This is already implemented as `ranking::declarative_density` and exposed as
`apply_declarative_boost`. Its own docstring says it exists for exactly this:
*"the topk_fts path where broad FTS matching surfaces generic distractor turns
that this signal down-weights."*

**It defaults to `false` on `RecallTopKConfig` — the path the published
baseline ran.** It is on for the cascade profile, off for top-k.

Result of enabling it: see §5.

## 5. Measured: declarative boost on `topk_fts` — +3 turns, and that is the point

Enabling `SPECTRAL_TOPK_DECLARATIVE=1` on the same 250 questions:

| | evidence-turn micro | macro | zero-evidence |
|---|---:|---:|---:|
| declarative OFF (baseline) | 231/356 | 72.7% | 53 |
| declarative ON | **234/356** | 73.5% | 52 |

**+3 evidence turns, −1 zero-evidence question.** Directionally correct — the
signal is real — but nowhere near the ~+9.4pp the decomposition says is
available, and far less than a 1.42× separation ought to buy.

**That gap is the most useful thing in this document.**

## 5b. The composition is the ceiling, not the signal

The composite score starts as **FTS rank position**: `scores[i] = 1.0 - i/n`
(`ranking.rs:345-347`). Every signal is then an **additive** boost on top. So
the value of a boost in *ranks* depends entirely on the pool size:

At the shipped `fetch_mult=3` (pool = 120), adjacent ranks differ by 1/120 =
0.0083, and promoting the median recoverable evidence turn from rank 99 into
the top 40 requires **Δ = 0.492**:

| boost | max value | max movement |
|---|---:|---:|
| recency | 0.10 | 12 ranks |
| declarative | 0.10 | 12 ranks |
| entity | 0.05 | 6 ranks |
| proximity (G4) | 0.15 | 18 ranks |
| **all four combined, all maximal, all aligned** | **0.40** | **48 ranks** |

**48 < 59. The architecture cannot make the promotion, even in the impossible
best case where every signal fires at maximum in the same direction.**

Widening the pool does not rescue it. At `fetch_mult=12` (pool = 480) the same
0.40 budget moves 192 ranks — enough — but G4's weight sweep measured recall
getting *worse* there (72.7% → 61.5% as weight rose), because the base score is
now so finely divided that the boosts override BM25's ordering wholesale and
scramble the cases it was getting right.

So the system is caught between two failure modes:
- **small pool:** boosts are too weak to promote anything meaningful;
- **large pool:** boosts are strong enough to destroy the ordering they sit on.

The root cause is that **rank position is a bad base score.** It has no stable
scale — the same 0.10 boost means 12 ranks or 48 ranks depending on a config
value that was chosen for entirely unrelated reasons.

## 5c. The fix we already own: rank fusion

The principled composition for "combine a lexical ranking with a signal
ranking" is **reciprocal rank fusion**, `Σ 1/(K + rank_i)` — and this codebase
**already implements RRF**, in `sqlite_store.rs::ranked_ids`, to fuse the two
FTS channels on the fusion path.

RRF has exactly the property the additive scheme lacks: it is **scale-free**.
A document ranked 3rd by declarative density and 150th by BM25 gets a
meaningful fused score regardless of pool size, and no signal can wholesale
override another because each contributes a bounded `1/(K+rank)`.

**This is the highest-value untested change in the project**, and it is
deterministic, read-time-free, and measurable on the $0 oracle:

> Rank the widened pool independently by BM25 and by each deterministic signal
> (declarative density first — it is the one measured to separate), fuse with
> RRF instead of adding boosts, and measure evidence-turn recall.

It reuses machinery already in the tree, it is the standard answer to this
exact problem, and every lever refuted to date was refuted *inside* the
composition that this replaces — which means those nulls do not carry over.

## 6. What this says to do next

Ranked by expected value, all deterministic and all read-time-free:

1. **Replace additive boosts with RRF fusion (§5c).** This is first because it
   is the *precondition* for every other signal being able to act at all. The
   measured target is ~+9.4pp, the evidence sits at median rank 99, and the
   current composition can move it 48 ranks in its best case. Deterministic,
   read-time-free, $0 to measure, and it reuses RRF machinery already in the
   tree.
2. **Then non-lexical signals**, evaluated *inside* the new composition:
   first-person declarative (measured to separate 1.42×), speaker-role priors,
   answer-shape matching (question asks "how many" → prefer turns containing
   quantities), turn-position priors. Testing these under the additive scheme
   would repeat the mistake this document diagnoses.
3. **Vocabulary bridging** for the 34% that no ranker can reach — the
   `query_aliases` file is a shipped, deterministic, consumer-supplied channel
   and is untested. Rocchio PRF over the outcome ledger is the other untested
   expansion-side lever (blocked on ledger volume — see G3).
4. **Multi-session first, within all of the above.** It carries +37.5pp of available lift, the worst
   zero-evidence rate (31.2%), and the lowest ceiling — meaning it is both the
   biggest retrieval opportunity and the place where reader work would also pay.

**What NOT to do:** more lexical refinement. Six levers, six nulls, and now a
measured explanation for why.

**Refs:** `r19-locomo-turn-labels-2026-08-08.md`,
`g4-proximity-result-2026-08-08.md`, `preference-stage0-result-2026-08-08.md`,
`bm25-locomo-baseline-result-2026-08-07.md`, `k-admission-test-2026-07-20.md`.
