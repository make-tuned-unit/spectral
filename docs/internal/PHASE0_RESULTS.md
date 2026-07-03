# Permagent benchmark — Phase 0 results (T1 ingest + T5 determinism)

**Date:** 2026-07-03
**Harness:** `crates/spectral-bench-real/src/bin/phase0.rs` (release build)
**Corpus:** the real brain — `~/.permagent/brain/memory.db`, 1738 memories,
~451k tokens, spanning ~48 days (~36 memories/day ingest rate).
**Spend:** $0. Embed-stack cost is computed analytically from the token count.
**Raw output:** `~/spectral-local-bench/phase0-results.json`

## Bottom line (losses first, per the spec's fairness rule)

**Phase 0 does not support Spectral's thesis. It weakens it.**

Spectral's one surviving claim was "cheapest-at-parity, offline, auditable."
Phase 0 measures the cost/offline/determinism half of that at Permagent's real
volume, and the edge is **not load-bearing**:

1. **The cost moat is worth ~$0.04/month.** Embedding the entire real corpus
   with a current top embedder (`text-embedding-3-large`, $0.13/1M tokens) costs
   **$0.059 once**; at Permagent's real ingest rate it projects to
   **$0.037/month**. Spectral's "$0 vs API" advantage is real as a ratio and
   trivial as a dollar figure. This trips the pre-registered kill criterion
   (cost difference must be ≥ $5/month to matter) by **two orders of magnitude**,
   and it is robust to embedder price: even a 100× more expensive embedder is
   ~$3.70/month — still under the $5 threshold. **At 36 events/day, no realistic
   embedding cost is load-bearing.**

2. **Against the free classical rival, Spectral loses every Phase-0 systems
   axis.** MinHash+BM25 is also $0/offline, and it is:
   - **~500× faster to ingest** — 21,800 ev/s vs Spectral's 43 ev/s.
   - **~40–70× lighter** — ~2 KB/event vs Spectral's ~87 KB/event steady-state
     (147 KB/event including un-checkpointed WAL; the real brain.db is 151 MB /
     1738 = 87 KB/event).
   - **tied on determinism** — both 1.0 (byte-identical rankings on repeat).

   This is the same MinHash+BM25 that already beat Spectral's recognition engine
   (see `RECOGNITION_BASELINE.md`). On Phase 0 it beats the rest of the stack on
   cost/throughput/storage too.

The only Phase-0 axis where Spectral can still lead is **auditability**, and even
there BM25 is fully auditable (it can name the matched terms and their score
contributions); MinHash is the weaker part. So auditability is a *soft* edge, not
a unique one.

## Numbers

| metric | Spectral (full stack) | MinHash+BM25 | embed stack (analytical) |
|---|---|---|---|
| ingest throughput | 43 ev/s | 21,818 ev/s | (API-bound) |
| storage / event | ~87 KB (steady) | ~2 KB (RAM) | vector ~6 KB (1536×f32) + Postgres |
| API $ / full corpus | $0 | $0 | $0.059 |
| projected $ / month @ 36 ev/day | $0 | $0 | $0.037 |
| on-device | yes | yes | no |
| determinism (repeat rank) | 1.0 | 1.0 | <1.0 (ANN drift, not measured) |

Absolute-terms caveat in Spectral's favor: 43 ev/s is *plenty* for a 36-event/day
workload — Spectral ingests a full day in under a second. It is not too slow to
use; it is just far heavier than the free alternative while, on Phase 0, doing
nothing the free alternative doesn't.

## What this does and does not settle

- **Settles:** the cost/offline argument. At Permagent's real volume it is worth
  pennies per month and cannot on its own justify building and maintaining
  Spectral over the boring stack. The pre-registered kill criterion for cost is
  met.
- **Does NOT settle:** recall *accuracy* (T2/T3/T4, Phase 1). Phase 0 was always
  the cheap gate — "if the cost/offline edge isn't real, stop before labeling."
  The result is that the edge, while real, is economically negligible here. So
  the entire case now rests on Phase 1 showing a **decisive recall win** that
  justifies the weight — but the thesis of record was that accuracy is a *tie*
  and the case rests on cost. Phase 0 removed the cost leg of that stool.

## Determinism (T5) — detail

103 probe queries (first-6-tokens of every stride-th memory), each run twice:
- Spectral FTS5: 103/103 byte-identical.
- MinHash+BM25: 103/103 byte-identical.
- Embedding-kNN: not measured; HNSW/IVF rankings drift on index rebuild and with
  `ef`/`nprobe` params — <1.0 by construction. This is a genuine Spectral/
  classical win over ANN, but it is shared with the free classical stack, not
  unique to Spectral.

## Auditability (T5) — 0–2 rubric

| system | score | why |
|---|---|---|
| Spectral | 2 | names matched FTS terms + fingerprint hashes + wing + graph edges (`AuditReport` in brain) |
| MinHash+BM25 | 1 | BM25 names matched terms and per-term score contributions; MinHash similarity is a bucketed estimate (partial "why") |
| embedding-kNN | 0 | opaque vector distance; no term-level explanation without a bolt-on |

Auditability is the strongest remaining Spectral differentiator — but it is a
*product property Permagent may or may not need*, not an accuracy or cost win, and
BM25 covers most of it for free.

## Recommendation

Per the pre-registered kill criteria, Phase 0 points toward **the boring stack**,
not Spectral, unless Phase 1 produces a decisive recall-accuracy win. Given the
prior evidence that accuracy is a tie (LongMemEval synthesis-bound; recognition
lost to MinHash), the honest expectation is that Phase 1 will not rescue the
thesis. Two options:

- **Stop here.** Phase 0 + the recognition post-mortem + the LongMemEval ceiling
  together already answer the section-0 question with "no": the boring stack
  (pgvector at $0.04/mo, or MinHash+BM25 at $0) is the rational choice at
  Permagent's volume. Cheapest way to a decision.
- **Run Phase 1 anyway** (~3–4 days + labeling) only if a *decisive* recall win
  would change the decision. It would have to be large enough to justify 40–70×
  the storage and 500× slower ingest versus MinHash+BM25 — a high bar.
