# Constellation fingerprints — the cost is real, the conclusion was wrong

> **REFRAMED 2026-08-03.** This document originally recommended retiring
> constellation fingerprints. That conclusion was wrong, and the evidence for
> why is inside this same document.
>
> Tier 1 fires on 3.2% of questions **because the wing classifier that gates it
> ships as demo fixtures** (`alice|coffee|noah|carol-doe`,
> `acme|widget|bob|recipe`). The constellation tier was never given a real
> taxonomy to work with. Measuring a starved feature and deleting it because it
> produced nothing is amputation, not engineering.
>
> **What stands:** every cost measurement below (7x ingest, 14.7x storage,
> byte-identical retrieval over 361 questions). Those are facts about the
> *current* configuration and they are why this matters.
>
> **What is withdrawn:** the recommendation to retire. `fingerprints: false`
> stays as a measured option for consumers who genuinely do not use wings, and
> as the control arm for the real experiment — which is to build a real wing
> taxonomy and re-measure whether the constellation tier earns its cost when
> actually fed. See `wing-taxonomy-2026-08-03.md`.
>
> The default stays `true`. It was never flipped.


## Why this matters more than an accuracy lever

The Phase 0 verdict (`PHASE0_RESULTS.md`) is the sharpest negative result in the
repo:

> **LOSES the systems axes** — MinHash+BM25 is also $0/offline/deterministic and
> is ~500x faster to ingest and ~40–70x lighter.

If the goal is to be the best *deterministic* memory system, that is the finding
that has to be attacked. This is the largest single cause.

## What the fingerprint table costs

| cost | value |
|---|---|
| write time | **~39% of `Brain::remember`** |
| store-layer bytes | **~57%** (26.4 -> 11.6 KB/event) |
| rows | ~61 per memory (39,520 rows for 650 memories) |
| distinct hashes | **458 across 395k rows** on the real brain — `fingerprint_hash` is `(hall, hall, wing, bucket)` with **no memory identity**, so it is not a selective key by construction |

## What it buys — measured, not assumed

Its only production reader is TACT tier 1, which fires only when **both** a wing
and a hall are detected on the query:

```rust
if let (Some(w), Some(h)) = (wing, hall) { ... fingerprint_search ... }
```

`crates/spectral-bench-real/src/bin/tact_tier_reachability.rs` runs the shipped
classifier over every benchmark question:

| dataset | wing detected | hall detected | **tier 1 reachable** |
|---|---:|---:|---:|
| LongMemEval-S (500) | 11.4% | 11.4% | **3.2%** (16) |
| LoCoMo held-out (120) | 8.3% | 5.0% | **2.5%** (3) |

And where it *does* fire, the record already measured it at **0 wins, 2 losses,
9 ties** against plain FTS (`tact-unlock-synthesis-2026-07-15.md`).

### The wing taxonomy is demo data

Worth separating out, because it explains the 3.2%. `default_wing_rule_pairs()`
ships these as the library default:

```
alice|coffee|anniversary|colou?r|favourit|favorit|sons|noah|leo|carol-doe  -> alice
apollo|polymarket|strategy|weather|prediction|wager|trade                 -> apollo
acme|widget|bob|recipe|cook|feast                                         -> acme
```

These are fixture names from example data. Every tier-1-reachable question in
both datasets matched by coincidence — "favorite desserts" hitting `favorit`
(alice), "slow cooker recipes" hitting `recipe|cook` (acme). The wing
classifier, which gates two of TACT's three tiers, is not a taxonomy of anything
real. **This is a separate defect and is not fixed here.**

## Measurement

`crates/spectral-bench-real/src/bin/fingerprint_retirement.rs`, N=600, release,
warm, warm-up discarded, two runs:

| arm | ms/write | KB/event | fp rows |
|---|---:|---:|---:|
| fingerprints ON (default) | 1.771 / 1.632 | 16.7 | 39,520 |
| fingerprints OFF | 0.228 / 0.233 | 1.1 | 0 |
| **change** | **7.0–7.8x faster** | **14.7x smaller** | — |

(Store layer. `Brain::remember` adds graph-side work, so the end-to-end ratio is
smaller — the 2026-07-31 profile puts fingerprinting at ~39% of a full write.)

## Retrieval parity — the gate

Held-out LoCoMo, 120 questions, `--fresh-brains` (this is an ingest-time change,
so brains must be rebuilt):

| | fingerprints ON | OFF |
|---|---|---|
| session-recall | 92.9% | **92.9%** |
| key-recall | 13.8% | **13.8%** |
| zero-recall | 4 | **4** |
| rank1 | 3.7 | **3.7** |
| context tokens | 1603 | **1603** |
| **context_hash diffs** | — | **0 / 120** |
| **retrieved-set diffs** | — | **0 / 120** |

LongMemEval-S, 241 questions across `knowledge-update`,
`single-session-preference` and `temporal-reasoning` (the dataset with the
higher tier-1 reachability, 3.2%):

| | ON | OFF |
|---|---|---|
| session-recall | 96.7% | **96.7%** |
| key-recall | 50.6% | **50.6%** |
| answer sessions hit | 463 | **463** |
| answer keys retrieved | 2828 | **2828** |
| **context_hash diffs** | — | **0 / 241** |
| **retrieved-set diffs** | — | **0 / 241** |

**Combined: 0 changes across 361 questions on two datasets, one of them
held-out.** The tier-1-reachable questions in both are included and unaffected.

## Verdict

Retiring constellation fingerprints is a **7x ingest speedup and 14.7x storage
reduction at byte-identical retrieval** on the held-out set.

It is exposed as `IngestConfig::fingerprints` / `BrainConfig::fingerprints`,
**default `true`** — behaviour-preserving until the default is flipped
deliberately. The bench lever is `SPECTRAL_NO_FINGERPRINTS=1` (requires
`--fresh-brains`).

## Before flipping the default

1. ~~Confirm on LongMemEval-S~~ **done — 0/241 diffs.**
2. `forget`/deletion paths reference `constellation_fingerprints` row counts
   (`ForgetReport`); with the table empty those counts go to zero, which is
   correct but changes a reported number. The deletion proof suite must stay
   green.
3. `fingerprint_neighbors` (used by the bench BFS channel) returns nothing with
   fingerprints off. That channel is env-gated and off by default.
4. Federation sync and `backfill_fingerprint_time_buckets` touch the table;
   both should no-op cleanly on an empty table.

## What this does not claim

Nothing about accuracy. Retrieval is byte-identical, so end-to-end accuracy is
unchanged by construction. This is a **systems** result: it closes part of the
gap to MinHash+BM25 that Phase 0 measured, which is the axis Spectral has chosen
to compete on.


## Why this matters more than an accuracy lever

The Phase 0 verdict (`PHASE0_RESULTS.md`) is the sharpest negative result in the
repo:

> **LOSES the systems axes** — MinHash+BM25 is also $0/offline/deterministic and
> is ~500x faster to ingest and ~40–70x lighter.

If the goal is to be the best *deterministic* memory system, that is the finding
that has to be attacked. This is the largest single cause.

## What the fingerprint table costs

| cost | value |
|---|---|
| write time | **~39% of `Brain::remember`** |
| store-layer bytes | **~57%** (26.4 -> 11.6 KB/event) |
| rows | ~61 per memory (39,520 rows for 650 memories) |
| distinct hashes | **458 across 395k rows** on the real brain — `fingerprint_hash` is `(hall, hall, wing, bucket)` with **no memory identity**, so it is not a selective key by construction |

## What it buys — measured, not assumed

Its only production reader is TACT tier 1, which fires only when **both** a wing
and a hall are detected on the query:

```rust
if let (Some(w), Some(h)) = (wing, hall) { ... fingerprint_search ... }
```

`crates/spectral-bench-real/src/bin/tact_tier_reachability.rs` runs the shipped
classifier over every benchmark question:

| dataset | wing detected | hall detected | **tier 1 reachable** |
|---|---:|---:|---:|
| LongMemEval-S (500) | 11.4% | 11.4% | **3.2%** (16) |
| LoCoMo held-out (120) | 8.3% | 5.0% | **2.5%** (3) |

And where it *does* fire, the record already measured it at **0 wins, 2 losses,
9 ties** against plain FTS (`tact-unlock-synthesis-2026-07-15.md`).

### The wing taxonomy is demo data

Worth separating out, because it explains the 3.2%. `default_wing_rule_pairs()`
ships these as the library default:

```
alice|coffee|anniversary|colou?r|favourit|favorit|sons|noah|leo|carol-doe  -> alice
apollo|polymarket|strategy|weather|prediction|wager|trade                 -> apollo
acme|widget|bob|recipe|cook|feast                                         -> acme
```

These are fixture names from example data. Every tier-1-reachable question in
both datasets matched by coincidence — "favorite desserts" hitting `favorit`
(alice), "slow cooker recipes" hitting `recipe|cook` (acme). The wing
classifier, which gates two of TACT's three tiers, is not a taxonomy of anything
real. **This is a separate defect and is not fixed here.**

## Measurement

`crates/spectral-bench-real/src/bin/fingerprint_retirement.rs`, N=600, release,
warm, warm-up discarded, two runs:

| arm | ms/write | KB/event | fp rows |
|---|---:|---:|---:|
| fingerprints ON (default) | 1.771 / 1.632 | 16.7 | 39,520 |
| fingerprints OFF | 0.228 / 0.233 | 1.1 | 0 |
| **change** | **7.0–7.8x faster** | **14.7x smaller** | — |

(Store layer. `Brain::remember` adds graph-side work, so the end-to-end ratio is
smaller — the 2026-07-31 profile puts fingerprinting at ~39% of a full write.)

## Retrieval parity — the gate

Held-out LoCoMo, 120 questions, `--fresh-brains` (this is an ingest-time change,
so brains must be rebuilt):

| | fingerprints ON | OFF |
|---|---|---|
| session-recall | 92.9% | **92.9%** |
| key-recall | 13.8% | **13.8%** |
| zero-recall | 4 | **4** |
| rank1 | 3.7 | **3.7** |
| context tokens | 1603 | **1603** |
| **context_hash diffs** | — | **0 / 120** |
| **retrieved-set diffs** | — | **0 / 120** |

LongMemEval-S, 241 questions across `knowledge-update`,
`single-session-preference` and `temporal-reasoning` (the dataset with the
higher tier-1 reachability, 3.2%):

| | ON | OFF |
|---|---|---|
| session-recall | 96.7% | **96.7%** |
| key-recall | 50.6% | **50.6%** |
| answer sessions hit | 463 | **463** |
| answer keys retrieved | 2828 | **2828** |
| **context_hash diffs** | — | **0 / 241** |
| **retrieved-set diffs** | — | **0 / 241** |

**Combined: 0 changes across 361 questions on two datasets, one of them
held-out.** The tier-1-reachable questions in both are included and unaffected.

## Verdict

Retiring constellation fingerprints is a **7x ingest speedup and 14.7x storage
reduction at byte-identical retrieval** on the held-out set.

It is exposed as `IngestConfig::fingerprints` / `BrainConfig::fingerprints`,
**default `true`** — behaviour-preserving until the default is flipped
deliberately. The bench lever is `SPECTRAL_NO_FINGERPRINTS=1` (requires
`--fresh-brains`).

## Before flipping the default

1. ~~Confirm on LongMemEval-S~~ **done — 0/241 diffs.**
2. `forget`/deletion paths reference `constellation_fingerprints` row counts
   (`ForgetReport`); with the table empty those counts go to zero, which is
   correct but changes a reported number. The deletion proof suite must stay
   green.
3. `fingerprint_neighbors` (used by the bench BFS channel) returns nothing with
   fingerprints off. That channel is env-gated and off by default.
4. Federation sync and `backfill_fingerprint_time_buckets` touch the table;
   both should no-op cleanly on an empty table.

## What this does not claim

Nothing about accuracy. Retrieval is byte-identical, so end-to-end accuracy is
unchanged by construction. This is a **systems** result: it closes part of the
gap to MinHash+BM25 that Phase 0 measured, which is the axis Spectral has chosen
to compete on.
