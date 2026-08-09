# Turn adjacency — the best-priced lever found, and it is not a ranking lever

**$0. Computed from the archived R22 A0 arm. No new retrieval, no model calls,
model-free.** Diagnostic and a prediction. **No lever is claimed** — R25 must
prereg and run it.

## Where this came from

The coreference diagnostic measured an 8.5× inversion: a question names a
person, BM25 spends its top-40 on turns that *mention* them (36.4%), and the
evidence is that person's own turn, which almost never says their name (4.3%).

That mechanism makes a structural prediction nobody had checked. In a two-party
dialogue, the turn that *mentions* Andrew is typically the other speaker
addressing him — **and Andrew's answer is the very next turn.** If so, the
evidence we miss should sit immediately beside something we already retrieved.

## It does

All 125 missed evidence turns in the k=40 baseline, by distance to the nearest
retrieved turn in the same session:

| nearest retrieved turn | n | share |
|---|---:|---:|
| **1 away (immediately adjacent)** | **58** | **46.4%** |
| 2 away | 15 | 12.0% |
| 3 away | 3 | 2.4% |
| >3 away | 35 | 28.0% |
| no retrieved turn in that session | 14 | 11.2% |

**Nearly half of everything we miss is one turn away from something we found.**

## The trade, priced

Emitting neighbours is **deterministic and exact**: if a missed evidence turn is
at distance 1 from a retrieved turn and we emit all distance-1 neighbours, we
emit that evidence. This is arithmetic, not an estimate.

| lever | evidence turns | micro | Δ | context |
|---|---:|---:|---:|---:|
| baseline k=40 | 231/356 | 64.89% | — | 1.00× |
| **±1 adjacency** | **289/356** | **81.18%** | **+16.29pp** | **2.62×** |
| ±2 adjacency | 304/356 | 85.39% | +20.51pp | 4.03× |
| *k=500 (rejected 2026-08-08)* | — | *89.7% macro* | *+17pp* | *8.7×* |

**±1 adjacency buys roughly what k=500 buys, at 3.3× less token cost.** That is
the first lever to beat naive k-raising on the token/recall frontier, and
k-raising was rejected *on exactly that frontier*.

(2.62× rather than 2× because retrieved turns are scattered across sessions, so
most neighbours are genuinely new rather than already in the set.)

## Why this one is different from the six that failed

Every refuted lever — porter, widening, ACT-R, associative spreading,
proximity, RRF — tried to **rank better**. The failure analysis then proved
ranking cannot promote deep evidence, and R22 confirmed it empirically.

**Adjacency does not rank anything.** It exploits dialogue structure to change
what is *emitted*, sidestepping the composition entirely. It is in a different
family from everything that has failed here, which is the main reason to expect
the prior nulls not to carry over.

It is also **not** "associative spreading" (refuted 2026-07-27): that followed
co-retrieval/fingerprint links and was scored on the pre-R19 diluted metric.
This is positional adjacency within a session — a different signal, on the
corrected metric.

## What must be preregistered before this is run (R25)

- **Token cost is the live risk, not recall.** Recall is arithmetic; 2.62×
  context is a real regression on the axis the cap work optimised, and the
  end-to-end effect is unknown and unpaid-for. A retrieval PASS here does
  **not** imply an accuracy PASS.
- **Fair comparison must be token-matched.** ±1 adjacency against plain k=40 is
  not a fair fight. The honest control is **k≈105 without adjacency** (the same
  ~2.62× budget), answering the real question: *is adjacency better than simply
  retrieving more?* The archived k-sweep suggests it is, but that must be run,
  not assumed.
- Whether neighbours are emitted directly or admitted to the ranking pool.
  Emitting directly is the version this document prices.
- The multi-session slice, which every other lever has made worse.

## Honest limits

- Two-party dialogue is the ideal case for adjacency. A multi-party or
  document corpus would not have this structure, so this may be LoCoMo-shaped.
- The 28.0% at >3 away and 11.2% with nothing retrieved in-session are
  untouched by this and remain the lexical floor.
- Computed on one arm, one corpus, `topk_fts`, k=40.

**Refs:** `speaker-attribution-diagnostic-2026-08-09.md` (the mechanism that
predicted this), `rrf-composition-result-2026-08-09.md` (why ranking levers are
closed), `g4-proximity-result-2026-08-08.md` (the k-sweep this is priced
against).
