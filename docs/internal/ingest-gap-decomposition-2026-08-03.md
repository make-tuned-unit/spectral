# Where the remaining ingest gap actually is — 2026-08-03

After fingerprint retirement, Spectral ingests at **3,148 ev/s** vs
MinHash+BM25's **22,688 ev/s** — a **7.2x** gap. This decomposes it, so the next
work targets the real term rather than the assumed one.

## Measurement

`crates/spectral-bench-real/src/bin/batch_write_headroom.rs` — identical rows,
identical FTS5 trigger work, against the real `memories` schema (WAL,
`synchronous=NORMAL`), one transaction per row vs one transaction for all.

| mode | ms/event | ev/s |
|---|---:|---:|
| one txn per row | 0.0846–0.0913 | 10,958–11,816 |
| single batched txn | 0.0165–0.0176 | **56,875–60,489** |
| speedup | — | **5.1–5.2x** |

## The decomposition

| layer | ms/event | ev/s | vs MinHash |
|---|---:|---:|---|
| MinHash+BM25 (RAM) | 0.0441 | 22,688 | 1.0x |
| **SQLite + FTS5, batched** | **0.0165** | **60,489** | **0.37x — 2.7x FASTER** |
| SQLite + FTS5, one txn per row | 0.0846 | 11,816 | 1.9x slower |
| Spectral today (fingerprints off) | 0.318 | 3,148 | 7.2x slower |

Reading the 7.2x gap as a budget of 0.318 ms/event:

| term | ms/event | share |
|---|---:|---:|
| batched insert + FTS5 floor | 0.017 | 5% |
| **per-event transaction commit** | **0.068** | **21%** |
| **Spectral's own per-event work** | **0.233** | **73%** |

## The two findings that matter

**1. Durability is not the problem.** A batched, durable SQLite+FTS5 store is
**2.7x faster than the in-RAM MinHash+BM25 index**. The "durable stores are
inherently heavier" intuition is wrong here — the classical rival's advantage
comes entirely from Spectral's per-event costs, not from SQLite.

**2. Batching is worth doing but does not close the gap.** Removing the
per-event transaction takes Spectral from 0.318 to ~0.25 ms/event — **7.2x →
~5.7x**. Real, worth doing, not transformative.

**73% of the cost is Spectral's own per-event work** — classification, signal
scoring, episode/session handling, content hashing — and that is where the gap
actually lives. It has never been profiled at that granularity. The 2026-07-31
ingest profile stopped at three bands (store / fingerprints / graph-side); this
says the interesting structure is *inside* the store band's non-SQLite portion.

## Next work, in priority order

1. **Profile the 0.233 ms.** Per-stage timing of classify / score / hash /
   episode-assign. This is 73% of the remaining gap and is currently a black
   box. Do this before optimising anything.
2. **Batched write API.** `MemoryStore::write_batch` committing N memories in
   one transaction. Worth ~21% of ingest cost. Note the durability semantics
   change — a crash loses the whole batch rather than one event — so it should
   be an explicit API, not a silent default.
3. Only then consider anything structural.

## What this does not claim

Nothing about accuracy or retrieval quality. Ingest throughput at the volumes
described in the Permagent spec (36 events/day) is not operationally binding
either way — this matters for the competitive claim, not for the workload.
