# Benchmark Results

Date: 2026-04-28
Hardware: Apple M1 Mac mini, 16 GB RAM, macOS Sequoia, APFS SSD
Rust: 1.95.0 (2026-04-14)
Spectral: commit bd3644c (main)

## Ingest throughput

| Scenario | Mean latency | ~ops/sec |
|---|---|---|
| Single `remember()` (empty brain) | 393 µs | ~2,540 |
| Batch 100 (fresh brain) | 235 ms total | ~425 /sec |
| Into populated brain (1000 existing) | ~24 ms | ~42 /sec |

**Observation:** Ingest into a populated brain is ~6x slower than into an
empty one. The cost comes from `list_wing_memories()` during fingerprint
generation — each new memory must pair against all existing peers in its
wing. This is O(peers-in-wing) per ingest.

## Recall latency

| Scenario | Mean | Notes |
|---|---|---|
| Small brain (100 memories) | 542 µs | Wing match via TACT |
| Medium brain (1000 memories) | 564 µs | Scales well |
| No-match query (1000 memories) | 795 µs | FTS fallback is ~40% slower |

**Observation:** Recall latency is sub-millisecond at 1000 memories and
scales well. The no-match case (FTS fallback) is slower because it must
scan the full-text index rather than doing a targeted fingerprint lookup.

## Spectral vs TF-IDF comparison

### Latency (5 queries × 1000 memories)

| System | Mean (all 5 queries) |
|---|---|
| Spectral (TACT retrieval) | 4.39 ms |
| TF-IDF cosine similarity | 8.24 ms |

**Spectral is 1.9x faster** than brute-force TF-IDF cosine similarity
over the same corpus. Spectral's fingerprint hash lookups in SQLite are
faster than computing cosine similarity against 1000 TF-IDF vectors.

### Retrieval quality (Precision@5)

| Metric | Spectral | TF-IDF |
|---|---|---|
| Precision@5 (single-hop queries) | 0.00 | 1.00 |
| Precision@5 (multi-hop queries) | 0.00 | 1.00 |
| Mean Precision@5 | 0.00 | 1.00 |

**Spectral shows 0% precision on the synthetic corpus.** This is a real
result that needs explanation:

The TACT wing classifier uses regex rules designed for specific real-world
projects (polybot, getladle, jesse, etc. — from the production zeroclaw
deployment). The synthetic corpus uses generic topic words ("infrastructure",
"engineering") that don't match any TACT wing rule. Without wing detection,
TACT falls back to FTS search, which uses different query terms than our
precision measurement expects.

TF-IDF achieves 100% because it operates purely on keyword overlap — the
synthetic queries contain the exact keywords used to generate the corpus.

### What this actually tells us

1. **TACT's value is domain-specific.** It excels when wing rules match
   the caller's vocabulary. On out-of-domain queries, it degrades to FTS.
2. **TF-IDF has a structural advantage on synthetic data** because the
   corpus and queries share exact vocabulary by construction.
3. **The latency comparison is fair** — both systems process the same
   queries against the same corpus. Spectral is faster despite the wing
   miss because its code path is lighter than N-dimensional cosine scans.
4. **A fair quality comparison requires either:**
   - Custom wing rules matching the corpus vocabulary, or
   - A real-world corpus where TACT's production rules apply.

### Caveat on the vector baseline

This benchmark uses TF-IDF (bag-of-words), not neural embeddings. A
`fastembed` + `bge-small-en-v1.5` baseline would test semantic similarity
rather than keyword overlap. The TF-IDF baseline was chosen to avoid
ONNX Runtime and model download dependencies. See METHODOLOGY.md for
details.

---

## Spectral vs neural vector embeddings (BGE-small-en-v1.5)

Date: 2026-04-28
Spectral: commit on feat/vector-comparison branch
Model: BAAI/bge-small-en-v1.5 (384 dims, via fastembed v5.13)

### Claim 1: Speed

20 queries against 1000 memories:

| System | Mean (20 queries) | Per-query | Notes |
|---|---|---|---|
| Spectral (TACT retrieval) | 25.3 ms | ~1.3 ms | Regex classification + hash lookup |
| Vector cold (encode + search) | 171.4 ms | ~8.6 ms | Includes ~5 ms/query encoding |
| Vector warm (search only) | 15.5 ms | ~0.8 ms | Pre-encoded queries, scan only |

**Spectral is 6.8x faster than cold vector search.** When query
embeddings are pre-computed (warm), vector search is 1.6x faster than
Spectral because brute-force cosine over 384-dim float32 vectors is
cheaper than Spectral's SQLite + fingerprint lookup.

Cold vs warm matters: in a real deployment, every new user query must be
encoded. The cold path is the common case unless you batch and cache
queries.

### Claim 2: Multi-hop accuracy (Precision@5)

| Category | Spectral P@5 | Vector P@5 | Winner |
|---|---|---|---|
| Keyword overlap (5 queries) | 1.00 | 1.00 | Tie |
| Paraphrase (5 queries) | 0.60 | 1.00 | **Vector** |
| Multi-hop topical (5 queries) | 1.00 | 1.00 | Tie |
| Vocabulary bridge (5 queries) | 1.00 | 1.00 | Tie |

**Vector wins on paraphrase queries.** Two of the five paraphrase
queries had truly zero word overlap with the corpus. Spectral scored
0.00 on these (TACT fell to FTS, FTS found nothing). Vector found
relevant results via semantic similarity.

The other three paraphrase queries had accidental word overlap ("cluster",
"family", "event") that let Spectral's FTS fallback succeed. This shows
that TACT's weakness is specifically **vocabulary-gap queries where no
word in the query appears in any relevant memory**.

**Multi-hop queries tied.** Both systems achieved 1.00 P@5. Spectral's
fingerprint search activated correctly (wing + hall detected), but the
queries also contained enough keywords for vector to find the same
results. A more challenging test would use queries with wing+hall
triggers but zero vocabulary overlap with target memories.

**Vocabulary bridge queries tied.** Surprisingly, both systems found
relevant results even with completely different vocabulary. FTS matched
common words ("pipeline", "data", "event", "system") that happened to
appear in relevant memories. Vector matched via semantic similarity.

### Claim 3: Operational cost

| Metric | Spectral | Vector |
|---|---|---|
| Cold-start time | 34 ms | 143 ms |
| Per-query encoding | 0 ms | ~5 ms |
| Disk (1k memories) | 22.1 MB | 1.5 MB embeddings + ~330 MB model/runtime |
| Disk (10k projected) | ~220 MB | 14.6 MB embeddings + ~330 MB model/runtime |
| External dependencies | None | ONNX Runtime + model weights |

**Spectral has no model dependency.** No GPU, no embedding model in
memory, no per-query encoding cost. The trade-off is a larger per-memory
disk footprint (SQLite + Kuzu graph + fingerprints) — but at small scale
(<10k memories), Spectral's total disk usage is smaller than vector's
model overhead.

At ~15k memories, the crossover occurs: Spectral's data exceeds vector's
fixed model cost + embeddings.

### What surprised us

1. **Multi-hop tie.** We expected Spectral's fingerprint traversal to
   outperform vector on cross-hall queries. It didn't — because the
   queries contained enough direct keywords for both systems.

2. **FTS fallback effectiveness.** Three "paraphrase" queries succeeded
   for Spectral because common words like "cluster" and "event" provided
   just enough FTS signal. Pure vocabulary-gap queries (0/5 word overlap)
   were the only total failures.

3. **Vector warm is fast.** Brute-force cosine over 384×1000 float32
   vectors (0.8 ms for 20 queries) is cheaper than we expected. At 10k+
   memories, an ANN index (HNSW) would be needed.

### When to use each

| Use Spectral when... | Use vector when... |
|---|---|
| Queries use domain vocabulary | Queries may use paraphrased vocabulary |
| Sub-ms cold-query latency matters | Encoding latency is acceptable (~5 ms) |
| No GPU/model infrastructure available | Embedding infrastructure exists |
| Memory count is <15k (disk advantage) | Large corpus (model cost amortized) |
| Domain-specific wing rules are well-tuned | General-purpose semantic search needed |

### Caveats

- **No ANN index.** Vector uses brute-force scan. Production vector
  search with HNSW would be faster on warm queries but has additional
  memory and index-build costs.
- **1000 memories.** Results may differ at 100k+ scale.
- **Synthetic corpus.** Real-world text is noisier; both systems may
  perform differently.
- **Model download.** First run downloads ~330 MB. This is cached
  locally but is a real user cost.

---

## Optimization round 1: wing cache + compound indexes + CTE + transaction batching

Date: 2026-04-28
Spectral: perf/wing-cache-and-indexes branch
Changes: LRU wing cache (32 entries), compound hall indexes, unified CTE
fingerprint search, single-transaction writes.

### Ingest throughput (before → after)

| Scenario | Before | After | Change |
|---|---|---|---|
| Single `remember()` (empty brain) | 393 µs | 605 µs | +54% slower |
| Batch 100 (fresh brain) | 235 ms | 98 ms | **2.4x faster** |
| Into populated brain (1000 existing) | ~24 ms | ~12 ms | **2.0x faster** |

**Transaction batching was the big win.** Wrapping memory + fingerprint
inserts in a single `BEGIN..COMMIT` eliminated per-statement autocommit
overhead. The batch-100 benchmark improved 2.4x. The populated-brain
benchmark (which generates many fingerprints) improved 2.0x.

Single-empty-brain regressed. This is expected: the transaction overhead
is higher than autocommit for a single insert with zero fingerprints.
At scale (any batch or populated brain), the transaction wins decisively.

### Recall latency (before → after)

| Scenario | Before | After | Change |
|---|---|---|---|
| Small brain (100 memories) | 542 µs | 758 µs | First run variance |
| Medium brain (1000 memories) | 564 µs | 590 µs | No change (p=0.90) |
| No-match query (1000 memories) | 795 µs | 821 µs | No change (within noise) |

**Recall was not meaningfully affected.** The wing cache doesn't help
the benchmark's cold-query pattern (each iteration queries fresh). In a
real app with repeated wing queries, the cache would avoid SQLite
round-trips entirely. The compound indexes help fingerprint search but
the benchmark's synthetic corpus doesn't exercise that path (wing
misses → FTS fallback).

### Summary

| Optimization | Measured impact |
|---|---|
| Wing LRU cache | Not measurable in cold benchmarks; serves warm queries from memory |
| Compound hall indexes | Not measurable (benchmark doesn't hit hall-match path) |
| Unified CTE fingerprint search | Merged with hash-match path; reduces round-trips |
| Transaction batching | **2.0-2.4x faster ingest** at scale |

### Explicit non-goals (not ported)

- MISS-2: Materialized `wing_to_memory_ids` table — premature at current scale
- MISS-5: Stop-word stripping — quality improvement, not perf
- MISS-6: Negative pattern filtering — domain-specific, not library concern
- MISS-7: Bulk regeneration batching — migration tooling only

## How to reproduce

```bash
# TF-IDF baseline
cargo bench --bench retrieval -p spectral

# Neural vector comparison
cargo bench --bench vector_comparison -p spectral
```

Criterion HTML reports are generated in `target/criterion/`.
