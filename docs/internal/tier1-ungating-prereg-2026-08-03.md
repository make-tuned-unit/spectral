# Preregistration — ungating TACT tier 1 from hall — 2026-08-03

**Written before the measurement.** Binding.

## Hypothesis

TACT tier 1 — the constellation/fingerprint path — is gated on detecting
**both** a wing and a hall on the query:

```rust
if let (Some(w), Some(h)) = (wing, hall) { ... fingerprint_search ... }
```

Measured on 217 real Permagent queries against the real taxonomy: wing fires
**46.5%**, hall fires **5.5%**, both **0.9%**.

A hall is a *memory type* (fact, preference, discovery, advice). The hall rules
match a speaker **asserting** one — `decided|chose|remember|prefers`. Real
queries are someone **asking**: *"Give me a tour of the app."* A question does
not announce what kind of memory answers it.

**H:** hall is a property of the memory, not of the question. Removing it from
the gate — firing tier 1 on wing alone, searching the wing's fingerprints across
all anchor halls — makes the constellation path reachable on **~46.5%** of real
queries instead of 0.9%, without degrading results.

## The limitation I cannot design around — stated up front

**This cannot be quality-measured on any labelled dataset available.**

The $0 oracle runs on LongMemEval-S and LoCoMo. Both are open-domain
conversation with **no within-brain topic structure**, and with the demo
fixtures now removed the library assigns no wings at all, so every memory lands
in `general`. Wing detection on those corpora is **0%**, which means tier 1
never fires there — ungated or not. Running the oracle would produce a
guaranteed null that says nothing about the hypothesis.

The corpus that *has* real wings — the Permagent brain, 12 genuine topic areas
over 1,979 memories — has **no ground-truth answer keys**. There is nothing to
score retrieval against.

So this experiment splits:

| phase | measures | runnable now |
|---|---|---|
| **A — behavioural** | does the tier fire; does it change results; at what latency | **yes**, real brain + 217 real queries |
| **B — quality** | does firing it produce *better* results | **no** — needs a labelled corpus with wings |

**Phase A cannot show the change is good. It can only show the mechanism works
and is safe.** Any claim beyond that requires Phase B, and Phase B requires a
dataset that does not exist yet. I will not report a Phase A pass as evidence
of improvement.

## Phase A decision rules (binding)

1. **Reachability.** Tier 1 must fire on **≥ 30%** of the 217 real queries
   (from 0.9%). Below that the gate is not the binding constraint and the
   hypothesis is wrong.
2. **Non-degradation.** For queries where tier 1 now fires, the returned set
   must not *lose* results relative to the gated path: `|results| ≥ |baseline|`
   on at least 95% of them. Tier 1 merges fingerprint hits with FTS hits, so a
   shrink means the merge is dropping evidence.
3. **Latency.** Median recall latency may rise by at most **20%**. Ungating
   widens the hash set from 40 to 100 per query (5 anchor halls x 5 target x 4
   buckets); if that costs more than a fifth of the read path it is not worth
   it regardless of quality.
4. **Determinism.** Repeated identical queries must return byte-identical
   results, as today (1.0).
5. **Default stays gated regardless of outcome.** Phase A passing makes this a
   candidate that unblocks Phase B. It does not flip a default, and it is not
   an accuracy claim.
6. **One shot.** No threshold tuning after seeing the result.

## What Phase B would require

A labelled corpus where (a) memories carry genuine topic areas and (b) questions
have known answer sessions. Neither LongMemEval nor LoCoMo qualifies. The
realistic source is Permagent itself: real queries with recorded outcomes via
the turn ledger (`turn_events` / `turn_members`). That data does not exist yet —
the ledger accumulates nothing until Permagent calls `turn`.

**This is the honest blocker on the whole constellation question**, and it has
been the blocker since the tier was first measured. Every verdict on tier 1 to
date — including the 0-wins/2-losses/9-ties that nearly got the fingerprints
deleted — was measured on corpora with no wing structure.

## Implementation

`TactConfig::tier1_requires_hall`, default `true` (behaviour-preserving).
When `false`, tier 1 fires on wing alone and `generate_query_hashes` enumerates
all anchor halls rather than fixing the anchor to the query's detected hall.

## Prior

**High** that Phase A passes reachability — it is close to arithmetic: wing
fires on 46.5% of real queries and the hall conjunction is what suppresses it.

**Unknown** whether it helps. That is the point, and it is what Phase B would
answer. The fingerprint index is non-selective by construction
(`fingerprint_hash` = `(hall, hall, wing, bucket)`, 458 distinct hashes over
395k rows — no memory identity), so a plausible outcome is that tier 1 fires
often and returns near-arbitrary members of the wing. Phase A's
non-degradation gate is designed to catch that: if tier-1 results displace
better FTS hits, the merged set will not obviously shrink, so **Phase A passing
is genuinely weak evidence** and must not be oversold.
