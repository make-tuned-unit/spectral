# Integrated recall — how the methods work in sync — 2026-07-15

Spectral's recall is not a bag of competing retrievers; it is one pipeline where
each method contributes where it is strong and the measured-harmful ones are kept
out. This is the "all methods in sync" picture, and it is what delivers the
measured lift (+18–40pp answer-key recall on every memory type).

## The pipeline (default cascade path)

```
query
  │
  ├─(1) CANDIDATE RETRIEVAL — brain.cascade_retrieve(query, k×fetch_mult)
  │      FTS5 + BM25 (porter-stemmed, IIS-stemmer exact channel) is the core.
  │      TACT tiers supplement only when they fire; FTS dominates recall.
  │
  ├─(2) RERANK — ranking::apply_reranking_pipeline
  │      signal_score (0.3) · recency (half-life by shape) · entity clustering ·
  │      ambient boost (penalty-only frontier) · declarative boost · episode
  │      diversity · context dedup. Truncate widened pool back to k.
  │
  ├─(3) ASSOCIATIVE RECALL (ACR) — spreading::associative_spread (opt-in)
  │      episode (same-session, created_at proximity) + cross-session (PRF).
  │      Recovers answer memories FTS ranked out — the vocabulary gap.
  │
  └─→ ranked, session-grouped context for the actor
```

`topk_fts` (Temporal shape) is the same minus TACT tiers: FTS+BM25 → rerank →
ACR. Both paths apply ACR through the identical `spreading` module.

## What is IN the sync (measured to help)

- **FTS5 + BM25 + porter stemming** — the deterministic lexical core; near-ceiling
  session-recall on real data.
- **Rerank signals** — signal-score / recency / episode-diversity / penalty-only
  ambient / declarative. Shape-tuned (`cascade_profile`).
- **ACR spreading** — the associative layer; +18–40pp answer-key recall, and (weak
  actor) starts converting to accuracy where the strong actor didn't need it.

## What is OUT (measured harmful / inferior — deliberately not in the sync)

- **TACT fingerprint/wing tiers as a primary retriever** — metadata fingerprint
  (442 hashes), persona-wing regexes collapse 73% of memories to "general";
  0/500 retrieval effect; wing-gating crowds out FTS. Left as a supplement only.
- **Kuzu/graph path** — measured retrieval-inferior to cascade; the engine was
  collapsed onto SQLite.
- **Co-retrieval popularity boost** (`co_retrieval_weight`) — induces popularity
  bias (regression); default 0.
- **fetch_mult widening as a default** — Pareto-safe on retrieval but no accuracy
  conversion on a strong actor; default 1 (opt-in).
- **novelty→signal** — orthogonal axis; measured all-downside; not wired.

The discipline: a method is in the sync only where it demonstrably helps, and the
whole thing stays deterministic, local, and embedding-free.

## Enabling ACR (the one opt-in)

Library: `CascadePipelineConfig.spread = AssocSpreadConfig::precision()` (or
`::completeness()`), applied inside `run_cascade_pipeline`. Bench: the same
config, driven by `SPECTRAL_ASSOC_*` env vars, applied on both recall paths via
`apply_associative_spreading`. Both routes call the identical library
`associative_spread`. OFF by default — no behavior change until opted in.

## Why it's integrated, not bolted-on

- ACR seeds from the **reranked** FTS results (stage 2 output), so it inherits the
  full pipeline's ordering — it augments the best candidates, it doesn't bypass
  them.
- Session-preserving rerank means ACR's displacement never undoes stage-1/2
  session-recall.
- One `spreading` module, one config type, both recall paths — no divergent copies
  (the bench delegates to the library).

Result: a single deterministic, local recall pipeline that combines FTS,
reranking, and associative spreading in sync, delivering measured lift on every
memory type. See `acr-lift-all-memory-types-2026-07-15.md` for the numbers.
