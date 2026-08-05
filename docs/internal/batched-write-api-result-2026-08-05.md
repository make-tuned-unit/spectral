# Result — R7: batched write API (2026-08-05)

Register row R7 (READY, no prereg needed — additive API, no default-path
behaviour change; the sequential path is pinned by parity test).

## What shipped

- `MemoryStore::write_batch(&[(Memory, Vec<Fingerprint>)]) -> Vec<WriteOutcome>`
  — trait method with a default sequential implementation (per-event
  durability), so non-SQLite backends are unchanged.
- `SqliteStore` override: ONE transaction for the whole batch. The
  per-memory transactional body was extracted verbatim into
  `write_memory_in_tx` and is shared by both paths — they cannot drift.
  Probes inside the batch transaction see earlier same-batch writes, so
  intra-batch duplicate keys behave exactly as sequential writes (pinned).
- `ingest::ingest_batch_with(Vec<IngestItem>, …)` — per-item preparation
  identical to `ingest_with` (shared `prepare_ingest`), then one
  `write_batch`. Episode bookkeeping deliberately stays per-event.

## The durability contract (why this is explicit, never a default)

A crash mid-batch loses the whole batch, not one event. Per the register:
callers choose it knowingly for bulk paths (imports, replays, brain builds);
`write`/`ingest_with` remain the default everywhere. Nothing existing changed
behaviour: 165 pre-existing spectral-ingest tests pass untouched.

## The one semantic divergence, documented and pinned

Batch members do NOT fingerprint-pair with each other — peers are read at
prepare time, before any member is written. Sequential ingest pairs item N
against 1..N-1. The parity test pins this (batch: 0 intra-batch fingerprints
on an empty store; sequential: >0). Bulk imports where intra-batch pairing
matters should batch per session or accept the difference.

## Measurement (the shipped API, on disk, release, two passes)

`cargo run -p spectral-bench-real --release --bin store_write_batch_bench`
(N=3,000; disk-backed, not in-memory — commits must hit storage):

| pass | sequential `write` | `write_batch` | speedup |
|---|---|---|---|
| 1 | 7,847 ev/s | 37,429 ev/s | **4.77×** |
| 2 | 7,811 ev/s | 39,443 ev/s | **5.05×** |

Context against the R7 register numbers: raw batched SQLite measured
60,489 ev/s; the shipped API reaches ~38k ev/s because it keeps the
per-memory probe (`SELECT content_hash…` for upsert semantics) and outcome
tracking. Sequential-API 7.8k ev/s vs raw per-row-txn ~— the commit was the
dominant cost at this layer, which is why the batch wins ~5× here while the
end-to-end ingest gap decomposition attributed only 21% to commits
(fingerprints and graph-side work dominate full ingest; those are untouched).

## Tests

3 new (parity row-for-row incl. outcomes, intra-batch duplicate semantics,
end-to-end ingest parity + the pinned fingerprint divergence); 166+2
spectral-ingest tests green.

## Not done, deliberately

No `Brain::remember_batch`. The full remember pipeline has per-item
auxiliary commits (density, signing, session assoc, recognition enroll) that
would dilute the win and each carry their own semantics; exposing a facade
batch API is a separate decision for when a consumer actually has the bulk
use case. The bench/replay bulk writers can use `ingest_batch_with` directly.
