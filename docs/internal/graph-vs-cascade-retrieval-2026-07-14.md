# Does the Kuzu graph path earn its keep? — measured, no — 2026-07-14

**Context.** Kuzu (the embedded graph engine under `KuzuStore`) is archived
upstream, continued as LadybugDB (`lbug` Rust crate). Before deciding
migrate-to-lbug vs collapse-onto-SQLite, the prior question: does the
Kuzu-backed graph retrieval path add value over the default cascade path?

**Method.** $0 deterministic oracle (no LLM, network-immune), same cached
brains, `--retrieval-path graph` vs `--retrieval-path cascade`, per category.

## Result (single-session-user, n=70)

| path | session-recall | answer-key recall | context tokens |
|------|:---:|:---:|:---:|
| cascade | 100.0% | **50.3%** | 11465 |
| graph   | 100.0% | **43.6%** | 5977 |

The graph path is **retrieval-inferior**: −6.7pp answer-key recall while
retrieving about half the evidence (fewer memories surfaced), and session-recall
is tied at ceiling (100%) so it recovers nothing cascade misses. (The
multi-session arm was not completed — the oracle stalled loading 133 brains —
but the single-session-user result plus the architecture below is decisive.)

## Architecture context (from the coupling audit)

- The Brain's hot path (FTS, spectrograms, recall via cascade/topk) runs on
  **`SqliteStore`**, not Kuzu. Kuzu backs only the entity graph:
  `upsert_entity` / `insert_triple` / `insert_mention` writes plus one 2-hop
  `neighborhood()` read — **8 call sites total**, feeding the opt-in
  `RetrievalPath::Graph` (not the published default routing).
- Known Kuzu-specific debt we already carry: test-only-`--test-threads=1` mmap
  contention and the abort reproducers (`f1eb288`) — all now frozen upstream.

## Recommendation

**Collapse the entity graph onto the existing SQLite store; do not migrate to
lbug.** The graph path adds no retrieval value (it's worse), it is off the
default path, and its whole surface is 8 call sites with a 2-hop neighborhood
read that a recursive CTE over entity/triple tables serves directly. Collapsing
drops the second embedded engine, the mmap contention, and the abort bug in one
move — strictly better than swapping one embedded graph DB (frozen Kuzu) for
another (lbug). Migrating to lbug would spend effort to keep a component that
does not earn its place. If a future workload makes graph traversal load-bearing
(measured lift over cascade), revisit lbug then.
