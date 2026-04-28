# Benchmark Methodology

## Hardware profile

Numbers in RESULTS.md were taken on:

- **CPU:** Apple M1 (8-core, 4 performance + 4 efficiency)
- **RAM:** 16 GB unified
- **Storage:** 256 GB SSD (APFS)
- **OS:** macOS Sequoia
- **Rust:** 1.95.0 (2026-04-14)
- **Spectral:** see commit SHA in RESULTS.md

## Framework

[Criterion.rs](https://github.com/bheisler/criterion.rs) v0.5. It handles
warm-up, multiple iterations, outlier detection, and statistical analysis
automatically. We don't implement our own timing.

## Test corpus generation

The synthetic corpus is generated deterministically from seed `0x5BEC_78A1`.
Anyone running the benchmark gets identical data.

### Structure

- **5 wings:** engineering, product, infrastructure, research, operations
- **4 halls:** fact, discovery, preference, advice
- Each memory contains wing-specific keywords (e.g., "server", "database"
  for infrastructure) and hall-specific keywords (e.g., "decided", "chose"
  for fact) to ensure the TACT classifier assigns the correct labels.
- Content is formulaic (`"Memory N: {hall_keyword} the {wing} team
  {wing_keyword} approach works well for {wing_keyword}"`) — not natural
  language. This means the benchmark tests the retrieval machinery, not
  the classifier's accuracy on real text.

### Why synthetic?

A synthetic corpus lets us control ground truth exactly. We know which
wing each memory belongs to, so we can compute precision/recall without
human annotation. The trade-off: synthetic text doesn't test classifier
robustness on ambiguous real-world input.

## Query set

| # | Query | Relevant wings | Type |
|---|---|---|---|
| 1 | infrastructure scaling server database cluster | infrastructure | single-hop |
| 2 | what did the engineering team decide about deploy | engineering | single-hop |
| 3 | research experiment hypothesis data model | research | single-hop |
| 4 | how does infrastructure scaling affect the deploy build | infrastructure, engineering | multi-hop |
| 5 | operations incident monitoring and infrastructure cluster | operations, infrastructure | multi-hop |

Multi-hop queries contain keywords from two wings. Spectral's fingerprint
system can match across wings when both have memories in the same time
bucket. TF-IDF treats this as a single bag-of-words query — it doesn't
distinguish topical structure.

## Baseline setup

### What we use: TF-IDF cosine similarity

A pure-Rust TF-IDF vectorizer with cosine-similarity scan over all
documents. This is the simplest meaningful "vector search" baseline:

- **Vectorization:** term frequency × inverse document frequency
- **Similarity:** cosine similarity (dot product / norms)
- **Index:** brute-force scan (no ANN index)
- **Top-K:** 5 results per query

### Why not neural embeddings (fastembed)?

The spec requested `fastembed` with `BAAI/bge-small-en-v1.5`. We chose
TF-IDF instead because:

1. `fastembed` requires downloading the ONNX Runtime (~200 MB) and model
   weights (~130 MB) at first run. This makes the benchmark non-hermetic
   and adds ~330 MB of untracked state.
2. Embedding computation time per query (~10 ms) would dominate the
   latency comparison, making it a benchmark of ONNX Runtime speed
   rather than retrieval architecture.
3. A TF-IDF baseline isolates the retrieval mechanism: both Spectral and
   TF-IDF work from keyword signals, so differences reflect the
   retrieval strategy (fingerprint vs. vector), not the encoding model.

A full neural-embedding comparison is valuable future work. When
implemented, it should pre-compute query embeddings separately from the
retrieval timing to avoid conflating encoding cost with search cost.

## What we measure

| Metric | What it tells you |
|---|---|
| **Ingest ops/sec** | How fast the brain can absorb new memories |
| **Recall latency (p50/p95/p99)** | Time from query to results, various percentiles |
| **Precision@5** | Fraction of top-5 results that are relevant (match expected wing) |

## What we don't measure

- **Cold-start time:** brain open/init is a separate concern
- **Memory footprint:** not instrumented
- **Disk usage:** not instrumented
- **Quality on real-world data:** synthetic corpus only
- **Recall@K (completeness):** we measure precision, not exhaustive recall
- **Neural embedding comparison:** TF-IDF only, see above

## Known limitations of this benchmark

1. **Synthetic corpus.** Memories are formulaic, not natural language.
   Classifier accuracy on real text may differ.
2. **Single machine.** Results may vary on different hardware.
3. **No ANN index in baseline.** TF-IDF uses brute-force scan. A real
   production vector setup would use HNSW or similar.
4. **Small scale.** Benchmarks go up to 1000 memories for timed tests.
   10k+ requires longer runs.
5. **TACT classifier is keyword-based.** The benchmark implicitly
   measures TACT's regex-based classification, not a hypothetical
   LLM-based classifier.

## How to reproduce

```bash
cargo bench --bench retrieval -p spectral
```

This produces:
- Criterion HTML reports in `target/criterion/`
- Console output with p50/p95/p99 latencies
- Quality comparison table printed to stdout
