# Phase 0 re-run — the systems verdict has moved — 2026-08-03

**This supersedes the throughput and storage figures in
`PHASE0_RESULTS.md` (2026-07-03). Those numbers should no longer be cited.**

Same harness (`crates/spectral-bench-real/src/bin/phase0.rs`), same corpus (the
real Permagent brain, 1,738 memories, ~455k tokens), same machine, both arms
back to back, release.

## Numbers

| metric | PHASE0 record | fingerprints ON (today) | **fingerprints OFF** | MinHash+BM25 |
|---|---:|---:|---:|---:|
| ingest throughput | 43 ev/s | 428 ev/s | **3,148 ev/s** | 22,688 ev/s |
| storage / event | ~87 KB | 20.1 KB | **5.07 KB** | 2.03 KB (RAM) |
| API $ / corpus | $0 | $0 | $0 | $0 |
| on-device | yes | yes | yes | yes |
| determinism (repeat rank) | 1.0 | 1.0 | **1.0** | 1.0 |

## What changed, and what that says about the old verdict

The record's headline was:

> **Against the free classical rival, Spectral loses every Phase-0 systems
> axis.** ~500x faster to ingest — 21,800 ev/s vs Spectral's 43 ev/s.
> ~40–70x lighter — ~2 KB/event vs ~87 KB/event.

Two things have happened since:

1. **The 43 ev/s figure was already stale.** Re-running the *unmodified*
   configuration today gives **428 ev/s**, a 10x difference from the recorded
   number. The `max_fingerprint_peers` cap
   (`DEFAULT_MAX_FINGERPRINT_PEERS`) landed after the Phase 0 run and made
   ingest flat in corpus size rather than O(N) against a near-clique of
   `general`-wing peers. Nobody re-ran Phase 0 afterwards, so a superseded
   number stayed in the record as the live verdict for a month.

2. **Fingerprint retirement takes it to 3,148 ev/s and 5.07 KB/event**
   (`fingerprint-retirement-2026-08-03.md`), at **byte-identical retrieval**
   over 361 questions across two datasets.

### The gap now

| axis | recorded gap | current gap |
|---|---:|---:|
| ingest throughput | ~500x | **7.2x** |
| storage / event | ~40–70x | **2.4x** |
| determinism | tied | **tied (1.0 / 1.0)** |

## Losses first, per the spec's fairness rule

**Spectral is still slower to ingest than MinHash+BM25 — 7.2x.** That is a real
remaining gap and it is not closed. At the workload the spec describes (36
events/day) it is not operationally meaningful — Spectral ingests a full day in
~11 ms — but "not meaningful at this volume" is the same argument the record
correctly refused to accept for the cost moat, and it should not be accepted
here either.

**The cost-moat conclusion is unchanged and still negative.** The embed stack is
$0.0592 for the full corpus, ~$0.04/month at real volume — two orders of
magnitude below the pre-registered $5/month kill line. Retiring fingerprints
does nothing to that. Cost is not a differentiator and should not be sold as
one.

## Where the storage comparison is not apples to apples

The 2.03 KB/event figure for MinHash+BM25 is **RAM**, for an in-process index
that provides: no persistence, no crash recovery, no deletion guarantees, no
visibility scoping, no signatures or provenance, no graph, no episodes.

Spectral's 5.07 KB/event is **durable on-disk SQLite** including the FTS5 index,
WAL, episode and session tables, entity graph, and Ed25519 provenance columns.

Being within 2.4x of a RAM-only inverted index while providing a durable,
deletable, signed, visibility-scoped, graph-queryable store is not obviously a
loss. The record's framing — "far heavier than the free alternative while doing
nothing the free alternative doesn't" — was fair when the multiplier was 40–70x
and the fingerprint table was buying nothing. At 2.4x, with the dead weight
removed, the honest read flips: the remaining bytes are paying for capabilities
the rival does not have.

## What this does not change

- Recognition still loses to MinHash on lexical re-encounter (0.998 vs 0.941)
  and to embeddings on paraphrase. Unaffected.
- LongMemEval accuracy. Retrieval is byte-identical; this is purely a systems
  result.
- The cost moat. Still ~$0.04/month, still below the kill line.

## Reproduce

```bash
cargo run -p spectral-bench-real --release --bin phase0 -- \
  --brain ~/.permagent/brain/memory.db --limit 1738 --out on.json
cargo run -p spectral-bench-real --release --bin phase0 -- \
  --brain ~/.permagent/brain/memory.db --limit 1738 --no-fingerprints --out off.json
```

## Next, on this axis

The remaining 7.2x is in per-event SQLite transaction overhead, not in
Spectral-specific work: with fingerprints off, ingest is ~0.318 ms/event and the
store-layer floor measured separately is ~0.20 ms. Batching writes into one
transaction is the obvious next lever and has not been measured.
