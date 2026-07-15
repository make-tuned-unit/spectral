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

### Accuracy verification (the decisive test)

Key-recall is a proxy; the real question is whether recovery converts to correct
answers. Temp=0 actor A/B on knowledge-update (n=29 transport-clean, sonnet-4-6,
FTS vs FTS+spreading S3/budget=3500):

| arm | accuracy |
|---|:-:|
| FTS baseline | 27/29 (93%) |
| FTS + spreading | 27/29 (93%) — **net +0** (fixed 1, broke 1) |

**Ceiling effect + a distraction cost.** The actor was already at 93%, so there
was almost no headroom. The two flips are the mechanism working *both* ways:
- **FIXED** (Starbucks gold, gold=120): spreading added 4 episode-mates including
  the answer memory FTS missed → correct. Recovery converted.
- **BROKE** (5K PB time, gold=25:50): spreading added **10** mates; the answer
  FTS had cleanly got *buried by distraction* → actor said "I don't have that."

Lesson: recovering answer keys helps only when it adds the *needed* memory
without burying others. **The lean config did not rescue it** — a second A/B
(SEEDS=1/budget=1500, ~2–3 mates) scored **27/30 vs FTS 28/30, net −1** (fixed 0,
broke 1). Fewer mates avoided some distraction but also missed the fixes.

**Decisive verdict: on knowledge-update, associative spreading is accuracy-
neutral-to-negative** (aggressive net +0, lean net −1). The +20–27pp key-recall
recovery does not convert, for two compounding reasons: the actor is near-ceiling
(93%) so there is almost no retrieval-failure-driven error to fix, and adding
context to a strong actor has a distraction cost that offsets the occasional real
fix. This is the recurring finding of the whole arc — on LongMemEval with a
strong actor, retrieval is near-ceiling and is not the accuracy bottleneck
(fetch_mult null, novelty null, TACT tiers harmful, and now spreading null-to-
negative). key-recall is a partly-bloated proxy; session-recall is the metric
that gates answerability and it is already ~98–100%.

**Where spreading's accuracy value could still be real** (untested here): a
retrieval-*failure*-bound workload — multi-session counting (needs every
instance), a weaker/cheaper actor that cannot compensate for a missing memory, or
production recall where paraphrase gaps are common. These need a different test
bed (and a stable-network env for the actor A/B).

**Iteration — rerank, don't expand (precision-preserving).** The distraction cost
comes from *growing* the context. `SPECTRAL_ASSOC_RERANK=B` instead promotes the
top-B proximity-ranked episode-mates INTO the window by *displacing the weakest B
FTS results* — context size stays constant, no distraction tax.

Retrieval measurement (constant context):

| category | config | key-recall | tokens |
|---|---|:-:|:-:|
| knowledge-update | FTS baseline | 56.0% | 13007 |
| knowledge-update | rerank B=8 | **69.8%** | **12537** (−4%) |
| knowledge-update | expand S3/b3500 | 79.5% | 15678 (+20%) |
| single-session-preference | FTS baseline | 37.6% | 9935 |
| single-session-preference | rerank B=8 | 41.3% | 10616 |

Rerank recovers **+14pp key-recall at *constant* (slightly lower) tokens** on
knowledge-update — recovery without the token or distraction tax. It recovers
less raw key-recall than expand (69.8% vs 79.5%) but never dilutes.

**Accuracy A/B result (rerank B=8 vs FTS, knowledge-update): net +0** (fixed
Starbucks by promoting the answer memory in; broke the to-watch count by
*displacing* items it needed). So all three variants land net ≤ 0:

| variant | accuracy | mechanism cost |
|---|:-:|---|
| expand S3/b3500 | net +0 | added context distracts (buried the 5K answer) |
| lean-expand S1/b1500 | net −1 | too few mates: missed fixes, still one break |
| rerank B=8 (constant ctx) | net +0 | displacing FTS results removes needed items |

**Closing verdict.** The tension is fundamental at a fixed actor budget: expand
*adds* context (distracts), rerank *swaps* context (displaces). Tellingly, on
knowledge-update every flip — fix and break — is a **counting** question, exactly
where completeness matters and swapping cannot win. Associative spreading is a
real, efficient *retrieval* mechanism (+14–27pp key-recall, cost-smart and
precision-preserving variants both built and measured) but it does **not** convert
to accuracy on LongMemEval near-ceiling categories, because retrieval is not the
bottleneck there. Its accuracy payoff, if any, needs a genuinely
retrieval-*failure*-bound test bed — multi-session counting paired with a
larger budget AND a counting-aware actor prompt (the +8pp intervention), or a
weaker/cheaper actor that cannot compensate for a missing memory — plus a
stable-network env for the A/B. That is the honest limit of what LongMemEval +
a strong actor can show, reached by measurement, not assumption.

### Cross-session substrate — the association episode can't reach

Episode spreading cannot leave a session, so it recovers *keys within already-
found sessions* but never a new session. `SPECTRAL_ASSOC_CROSS=N` adds the
cross-session substrate via pseudo-relevance feedback: each top seed's own
content is used as a query, so BM25's IDF weighting lets the seed's distinctive
tokens (entities, specifics) reach ASSOCIATED memories in OTHER sessions.
Deterministic, local, embedding-free — no entity graph needed.

Multi-session (n=40) — the category where cross-session association matters:

| config | session-recall | key-recall | tokens |
|---|:-:|:-:|:-:|
| FTS baseline | 95.5% | 40.9% | 14041 |
| episode (within-session) | 95.5% | **59.9%** | 17255 |
| cross-session N=5 | **97.3%** | 44.9% | 16057 |

**They are complementary.** Episode lifts *key*-recall +19pp but 0 *session*-
recall (it only completes found sessions). Cross-session lifts *session*-recall
95.5→97.3% — it **finds answer sessions FTS missed**, which episode/FTS
structurally cannot. Session-recall is the gating metric for multi-session
completeness (you need every contributing session), so recovering a missed one is
the more valuable — and harder — recovery.

**Combined mode (`SPECTRAL_ASSOC_COMBINED=1`) — the full associative layer.**
Cross-session spreading finds the missed answer sessions (PRF), then episode
spreading completes *every* session — the FTS seeds' AND the newly-found
cross-session ones — by turn-proximity. It dominates both single substrates:

| config | session-recall | key-recall | tokens |
|---|:-:|:-:|:-:|
| FTS baseline | 95.5% | 40.9% | 14041 |
| episode only | 95.5% | 59.9% | 17255 |
| cross-session only | 97.3% | 44.9% | 16057 |
| **COMBINED** | **96.8%** | **61.6%** | 17826 |

COMBINED gets both gains at once — **the highest key-recall of any config
(61.6% > episode-alone's 59.9%, because it now completes the cross-found sessions
too) plus a session-recall lift** — at comparable token cost. The associative
layer is realized: reach missed contributing sessions, then complete each. (The
`assoc_cross_session` / `assoc_episode_budget` helpers compose cleanly; the
individual modes are refactored onto them, numbers verified unchanged.)

### Wired into the default (cascade) retrieval path

The spreading cascade is extracted into `apply_associative_spreading(brain,
&mut hits)` (all modes, env-gated OFF), called by BOTH `retrieve_topk_fts` and
`retrieve_cascade` — so it now works on the **published default** path (cascade,
used for every shape except Temporal), not just topk. Refactor verified no
regression (topk COMBINED reproduces sess=96.8%/key=61.6% exactly). On the
cascade path spreading recovers *more*, because the stronger cascade baseline
compounds:

| cascade path (n=40) | session-recall | key-recall |
|---|:-:|:-:|
| multi-session baseline | 98.8% | 48.8% |
| multi-session + COMBINED | 98.8% | **70.0%** (+21pp) |
| knowledge-update baseline | 100% | 57.1% |
| knowledge-update + COMBINED | 100% | **86.7%** (+30pp) |

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
