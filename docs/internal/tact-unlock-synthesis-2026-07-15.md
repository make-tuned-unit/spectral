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

## The key

**Stop asking TACT to do recall. Its power is recognition + ambient — which is
where the original vision actually pointed.** The failure was never the
fingerprint's resolution; it was the *job*. Applied to recall, TACT is dominated
by FTS (measured) and its wing-gating actively harms it. Applied to what it was
designed for, it is uniquely capable and embeddings-free:

1. **Recognition** (near-duplicate / "have I stored this?"): dedup, consolidation,
   recurrence-strength feedback. Already live at write-time — keep and lean in.
2. **Ambient feedback** (the founding vision): surface associatively-linked
   memories from what the user is *doing* in the app — recent activity, focus
   wing, session — as a boost, not a primary retriever. This is a
   production-quality signal the bench cannot see.
3. **Cheap pre-recognition**: a fast "is anything relevant here at all" gate.

## Actionable

- **Retrieval path: let FTS lead; retire the metadata constellation as a
  retrieval PRIMARY.** It contributes 0/500 and its wing-gating crowds out FTS.
  Concretely: route the shapes that lose session-recall (GeneralPreference /
  GeneralRecall / General) to `topk_fts` (already done for Temporal), or make
  cascade merge FTS-first with TACT as supplement. (Deterministic session-recall
  win; a default routing change still wants an end-to-end number when a
  stable-network env is available.)
- **Do NOT build T3 for retrieval.** The new probe shows it would not move
  bench recall. If T3 is built, build it for *recognition* quality (dedup /
  recurrence), where its combinatorial specificity is the point.
- **Invest the "unique vision" energy in ambient feedback**, not fingerprint
  resolution — that is the lane where TACT/spectrogram has power no vector system
  replicates and where the bench's saturated retrieval metrics are blind.

The reframe is the unlock: TACT is a recognition engine. Used as one, it is
powerful and cheap. Used as a retriever, it is a strictly worse FTS. The prior
audits diagnosed this; this pass verified it with a head-to-head measurement and
named where the power actually lives.
