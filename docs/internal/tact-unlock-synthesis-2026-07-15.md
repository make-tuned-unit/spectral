# Unlocking TACT: the key is a reframe, not a better fingerprint — 2026-07-15

Research + review + verify pass on TACT ("Topic-Aware Context Triage"), the
fingerprint/constellation retrieval subsystem. Consolidates the scattered prior
audits, adds the missing empirical measurement, and states the actionable key.

## The vision (as designed)

`topology-lineage.md`: TACT + spectrogram = **"Shazam for memory"** — recognize a
memory by a compact *signal signature*, not by reading its contents.
"Recognition, not retrieval." Zero embeddings, zero LLM in the hot path,
deterministic, cheap. This is a genuinely distinctive idea.

## The diagnosis (prior audits, confirmed)

`SPECTROGRAM_AUDIT.md`: the fingerprint is "applied wrong, in a specific and
fixable way — **a recognition engine being asked to do recall.**"
- It hashes *metadata categories*: `SHA256(anchor_hall | target_hall | wing |
  time_bucket)` → **442 distinct hashes across 390k rows** in production. "442
  near-constant hashes cannot identify anything." Shazam's power is *millions* of
  **content-derived** hashes.
- The one topic coordinate, `wing`, is **8 hardcoded persona regexes**
  (`alice|coffee|apollo|recipe|...`). On real data ~76% of memories collapse to
  `general`; on LongMemEval the generic words fire *spuriously*, and the wing
  scope crowds out the answer-session memory FTS would surface (measured −6.7pp
  session-recall on preference — `tact-tiers-cost-session-recall-2026-07-14.md`).
- At query time the pipeline brute-enumerates all halls × all buckets, so those
  dimensions provide **zero** discrimination. `fingerprint_search` degenerates to
  wing-scoped co-occurrence **popularity**.

Measured contribution: **0/500 retrieval effect** (Tier-0 oracle); FTS already
beats it. The specced fix (backlog **T3, content peak-pair fingerprinting**) is
implemented in the `spectral-recognition` crate but wired only to *write-time*
(recurrence/dedup), never to retrieval.

## The missing measurement (new, this pass)

The audits left one question open: would the *proposed* fix — content recognition
(peak-pairs/MinHash), pointed at retrieval — actually beat FTS? Nobody had tested
it. `recognition_recall_probe` (deterministic, $0) enrolls one corpus in both the
recognition engine and a real FTS brain and compares the rank of the gold answer
across a spectrum of query↔memory relationships:

| | result |
|---|---|
| Recognition wins | **0** |
| FTS wins | 2 (found 2 recognition missed) |
| Ties | 9 |

Content recognition **never beats FTS at recall**. It ties where FTS already
wins, and — tuned for near-duplicate *recognition* — is *more conservative*, so it
misses thin-overlap matches FTS catches (a lone "shellfish" keyword). Because
Spectral deliberately rejects embeddings, recognition and FTS are **both
lexical**, and BM25 is already strong. There is no retrieval headroom to unlock,
even with the fingerprint done right. (And the associative/co-occurrence angle was
already measured harmful — the co-retrieval popularity-bias regression.)

## The key — CORRECTED after digging deeper

An earlier draft of this doc concluded "stop asking TACT to do recall; its power
is only recognition/ambient." That was premature — it tested TACT as a *primary
retriever* and as a *content fingerprint*, but never tested the one thing the
"constellation" is actually about: **association**. Digging deeper found the
vision working at recall after all.

**The real failure: TACT has a co-occurrence graph but uses it as a POPULARITY
degree-count, not as a SPREADING-ACTIVATION substrate.** Human associative recall
is: find seeds by direct match, then let activation spread through associative
links to co-occurring memories that *share no words with the query*. The shipped
TACT throws this away three times over — popularity scoring, broken metadata edge
keys (442 hashes), and broken persona-wing scoping.

**Demonstrated (mechanism, `associative_expansion_probe`):** with FTS honestly
excluding a no-word-overlap answer, spreading activation through EPISODE
co-occurrence recovers it — e.g. query "dinner using my homegrown ingredients"
→ answer "growing cherry tomatoes, basil, mint" (zero shared words): FTS misses,
episode-expansion recovers. The exact vocabulary-gap FTS cannot cross.

**Verified at scale (real LongMemEval, `SPECTRAL_ASSOC_EXPAND=M`):**

| category | expand M | answer-key recall | tokens |
|---|:-:|:-:|:-:|
| single-session-preference | 0 → 4 | 37.6% → **43.8%** | 9935 → 16853 |
| knowledge-update | 0 → 4 | 56.0% → **65.7%** | 13007 → 16689 |

**+3–10pp answer-key recovery**, deterministic, tunable — and the recovery
ceiling is high: 100% of missed-answer-key questions still have their answer
session retrieved, so the missed keys are reachable by episode-spreading. The
shipped popularity/metadata TACT delivered 0/500; the vision, implemented as
spreading activation over meaningful co-occurrence, delivers real recovery.

So the key is: **implement the constellation as spreading activation (FTS seeds →
spread over episode / entity co-occurrence), not as popularity over metadata
edges.** Recognition-as-primary-retriever and content-fingerprint-vs-FTS are dead
ends (measured); associative expansion is the live one.

### Cost-smart spreading — proximity is the efficiency unlock

The naive expansion ranked episode-mates by *signal* and recovered only +3–10pp,
because answer memories are often *low-signal events*. Cost-smart mode
(`SPECTRAL_ASSOC_BUDGET=E`, `SPECTRAL_ASSOC_SEEDS=S`) instead ranks episode-mates
by **turn-proximity to the seed** (the answer sits near the matched turn — the
key `session:turn:N:role` carries N) and fills only up to a token budget from the
top-S high-confidence seeds. Same mechanism, ranking swapped — the difference is
the ablation:

| category | config | answer-key recall | tokens |
|---|---|:-:|:-:|
| single-session-preference | FTS baseline | 37.6% | 9935 |
| single-session-preference | naive signal (M=4) | 43.8% | 16853 |
| single-session-preference | **cost-smart (budget=7000)** | **57.7%** | 15972 |
| knowledge-update | FTS baseline | 56.0% | 13007 |
| knowledge-update | naive signal (M=4) | 65.7% | 16689 |
| knowledge-update | **cost-smart (budget=7000)** | **83.4%** | 16665 |

Proximity beats signal by **+14–18pp at matched-or-lower token cost**; total
recovery over FTS is **+20pp (preference)** and **+27pp (knowledge-update)**, the
latter approaching its 98.7% session-recall ceiling. Answer memories are near the
matched turn; signal-ranking was structurally blind to them.

**Efficiency frontier (knowledge-update, seeds × budget):**

| config | answer-key recall | tokens (Δ vs FTS) |
|---|:-:|:-:|
| FTS baseline | 56.0% | 13007 |
| SEEDS=1, budget=1500 | **70.3%** | 13999 (**+7.6%**) |
| SEEDS=1, budget≥3500 (saturates) | 72.5% | 14400 |
| SEEDS=3, budget=3500 | 79.5% | 15678 (+20%) |
| SEEDS=3, budget=7000 | 83.4% | 16665 (+28%) |

**SEEDS=1/budget=1500 buys +14pp for +7.6% tokens — nearly free**, which
answers the "incredibly cheap" concern: one high-confidence seed's episode,
proximity-ranked, tightly budgeted, recovers most of the gain. It saturates at
~72.5% (a single episode has finite answer keys); unlocking the rest to 83.4%
needs more seeds at higher budget. The dial spans cheap-and-large to
expensive-and-maximal.

### Honest caveats (do not oversell)
- **Token cost is the weakness**: full/partial episode expansion adds 30–70%
  context tokens. The "incredibly cheap" goal needs a cost-smart expansion (cap
  total expansion tokens; expand only high-confidence seeds' episodes).
- **Expansion ranked by signal is suboptimal** for recovering answer memories,
  which are often *low*-signal events — hence recovery is +3–10pp, not the full
  ceiling. Ranking episode-mates by something answer-appropriate (recency, or
  proximity to the seed turn) should recover more per token.
- **Accuracy conversion unverified**: key-recall is a partly-bloated proxy and
  actor validation is blocked by this env's connectivity. But the mechanism
  recovers the *specific* answer-bearing memory (the archetype), so a real share
  is accuracy-relevant — worth an end-to-end A/B in a stable-network env.
- Recognition/ambient are still valid TACT jobs (below); this just adds the
  retrieval job back, done right.

## Actionable

- **Build associative expansion as the constellation's real retrieval role:**
  FTS seeds → spread activation over episode (and later entity) co-occurrence →
  merge. Prototype lives behind `SPECTRAL_ASSOC_EXPAND=M` in `retrieve_topk_fts`;
  it recovers +3–10pp answer keys FTS misses. Next: make it cost-smart (cap
  expansion tokens; rank episode-mates by recency/seed-proximity not signal;
  expand only high-confidence seed episodes) and A/B end-to-end in a
  stable-network env.
- **Retire the metadata constellation** (`hall|hall|wing|bucket`, 442 hashes,
  0/500) and the persona-wing scoping as the retrieval mechanism — replace, don't
  augment. The pairwise `constellation_fingerprints` EDGES could be reused as the
  co-occurrence substrate, but they are wing-scoped and quadratic (a clique within
  `general`); episode_id is the cleaner, sparser association graph already
  present.
- **Do NOT build T3 (content peak-pairs) for retrieval.** The recognition-vs-FTS
  probe shows it would not beat FTS at recall. Build T3 only for *recognition*
  quality (dedup / recurrence), where combinatorial specificity is the point.
- **Recognition + ambient remain valid TACT jobs** (near-dup / recurrence at
  write-time; ambient associative boost from app activity) — the associative
  retrieval role is additive to these, not a replacement.

The corrected unlock: TACT is a *constellation* — a co-occurrence graph. Its power
at retrieval is **spreading activation** (bridging FTS's vocabulary gap by
association), which the shipped popularity/metadata implementation discarded.
Verified: mechanism recovers the archetypal lexical-gap miss, and +3–10pp answer
keys on real data. The earlier "recognition can't do recall" finding is true but
was the wrong lens — the constellation was never about matching a memory's
content; it was about matching its *neighbors*.
