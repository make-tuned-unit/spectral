# Dispatch → Permagent: Kuzu is out — collapsed onto SQLite (not migrated to lbug)

**Date:** 2026-07-14 · **Spectral commit:** `4290c31` (impl), `bce5df0` (evidence)
**TL;DR:** You flagged Kuzu as archived (→ LadybugDB/`lbug`). We measured, decided
**collapse onto SQLite rather than migrate**, and **it's done**. Your integration
needs no code change; there is one on-disk cleanup you can do at leisure.

## Why collapse, not migrate to lbug

We measured whether the Kuzu graph path earns its keep before spending effort
keeping it (any form). $0 deterministic oracle on real LongMemEval-S,
single-session-user n=70:

| path | session-recall | answer-key recall | context tokens |
|------|:---:|:---:|:---:|
| cascade | 100.0% | **50.3%** | 11465 |
| graph (Kuzu) | 100.0% | **43.6%** | 5977 |

The graph path is retrieval-**inferior** — worse key-recall, half the evidence,
and it recovers nothing cascade misses (session-recall tied at ceiling). Its
whole surface is 8 call sites (entity/triple/mention writes + one 2-hop
`neighborhood()` read) feeding the **opt-in** `RetrievalPath::Graph`, not the
default routing. Migrating that to `lbug` would spend effort to keep a component
that doesn't earn its place. Collapsing onto the SQLite store the Brain already
runs drops the second embedded engine, the mmap contention, and the abort bug in
one move. Full reasoning: `docs/internal/graph-vs-cascade-retrieval-2026-07-14.md`.

## What changed (Spectral side)

- New `spectral_graph::graph_store::GraphStore` (SQLite-backed) **reimplements the
  exact former `KuzuStore` public API** — same types (Entity/Triple/DocumentNode/
  Neighborhood), same methods, same semantics (idempotent entity upsert that
  preserves description; edges only when endpoints exist; mentions unique per
  (doc, entity)). `Brain::store()` now returns `&GraphStore`.
- `kuzu` dependency removed from the crate and the workspace (gone from
  Cargo.lock). `kuzu_store.rs` + Kuzu `schema.rs` deleted. Error variant
  `Error::Kuzu` → `Error::Sqlite`.
- Full spectral-graph suite green; graph tests now run in ms (no Kuzu mmap
  contention → the `--test-threads=1` requirement is gone).

## What you need to do

1. **Code:** nothing required — the graph API is preserved. If you referenced the
   type path `spectral_graph::kuzu_store::KuzuStore`, it is now
   `spectral_graph::graph_store::GraphStore` (same methods). If you only go
   through `Brain`, you're unaffected.
2. **On disk:** the Brain graph file moved `graph.kz` → `graph.sqlite`. **Existing
   `graph.kz` Kuzu directories are orphaned and safe to delete.** The entity graph
   rebuilds on ingest, and it's off the default recall path, so there is no
   accuracy impact and no migration/backfill step.
3. **If you were counting on the `Graph` retrieval path** for anything in prod:
   flag it — but note it was measured inferior to cascade, so you likely want
   cascade regardless.

## Open item (unrelated, for awareness)

Actor-side accuracy validation (the synthesis bottleneck) is currently blocked on
*our* sandbox's flaky API connectivity, not on Spectral. The deterministic
harness is ready (temp=0 pinned on actor+judge; an "effective-mover" $0 pre-check
that bounds a retrieval lever's accuracy ceiling before spending). If you have a
stable-network environment, that validation kit is ready to hand off.
