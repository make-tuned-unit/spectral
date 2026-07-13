# Layered recall + ambient consolidation — TencentDB lessons, applied

## What we studied
`TencentCloud/TencentDB-Agent-Memory`: a hybrid agent memory. BM25 + vector
(`sqlite-vec`) fused by RRF; an LLM write-time pyramid **L0** raw → **L1** atoms →
**L2** scenarios → **L3** persona; a symbolic "task canvas" with `node_id`
drill-down claiming ~61% token reduction and *"lossless recovery: a deterministic
path from high-level abstractions back to ground-truth evidence."*

## What transfers (and what deliberately doesn't)
- **Confirms our bets:** RRF as the fusion primitive; provenance/traceability as
  the differentiator (they *market* the auditable drill-down Spectral is built
  for; we already had `consolidation_edges` + signed provenance).
- **Does NOT transfer:** the LLM L1/L2/L3 extraction and vector embeddings — the
  exact things Spectral avoids to stay the cheapest, deterministic, zero-embedding
  option. Real-corpus data this session showed recall@40 already near-ceiling, so
  embeddings would not help retrieval anyway.
- **The separable, high-value idea:** their real lift is not *better retrieval* —
  it is handing the actor a **compact, layered, provenance-linked context**. That
  is the actor-synthesis layer we independently measured as the bottleneck
  (multi-session counting). The *structure* (layered abstraction + drill-down) is
  separable from the LLM/vector machinery, and Spectral can provide it
  deterministically.

## What we built (this branch)
A deterministic, LLM-**optional** layered-memory loop:
1. **Ambient signal picks what to abstract.** `consolidation_candidates` clusters
   memories the co-retrieval signal (usage) repeatedly pulls together; recognition
   recurrence (spectrogram/MinHash) is the complementary content gate. So we only
   ever abstract *already-recurring* groups.
2. **Consolidation is a pluggable seam.** `consolidate_with(sources, target, tier,
   summarize)` folds a cluster into one higher-tier memory; `summarize` is a
   closure. Default `consolidate_extractive` is `$0` (no LLM). A consumer
   (Permagent's Librarian) may pass a **sparse** LLM closure — gated by (1), so
   the LLM touches only high-value clusters, keeping cost near-zero.
3. **Layered recall drills down.** `recall_with_provenance` surfaces the compact
   abstract memory and attaches its ground-truth sources via `consolidation_edges`
   — the actor gets summary + evidence, no re-derivation.

## Why this fits Spectral's identity
The recognition/spectrogram + ambient-feedback engine is the deterministic driver
(what the user does in the app → what recurs → what gets abstracted). The LLM, if
used at all, is sparse, gated, and replaceable by a `$0` extractive fallback — so
the system runs incredibly cheaply (or free) while offering the layered,
auditable, provenance-linked context that helps the actor where we measured the
real headroom.

## Next
- Port a sparse-LLM summarizer into Permagent's Librarian (the `summarize`
  closure) and A/B multi-session counting with `recall_with_provenance` context
  vs flat context — the hypothesis is it reduces the cross-session-dedup errors
  the counting-prompt intervention only partly fixed.
- Consider a recognition-recurrence candidate path (write-time pairs) alongside
  the co-retrieval one, for cold brains with no usage history yet.
