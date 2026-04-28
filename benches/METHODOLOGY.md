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

- **Memory footprint (RSS):** not instrumented in-process
- **Quality on real-world data:** synthetic corpus only
- **Recall@K (completeness):** we measure precision, not exhaustive recall

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
# TF-IDF baseline (no external model)
cargo bench --bench retrieval -p spectral

# Neural vector comparison (downloads ~330 MB on first run)
cargo bench --bench vector_comparison -p spectral
```

This produces:
- Criterion HTML reports in `target/criterion/`
- Console output with p50/p95/p99 latencies
- Quality comparison table printed to stdout

---

## Neural vector comparison

### Why we added this

The TF-IDF baseline (PR #9) tests retrieval over keyword overlap. Modern
production systems use neural embeddings that capture semantic similarity.
This benchmark adds that comparison, testing three claims: speed,
multi-hop accuracy, and operational cost.

### Setup

- **Model:** BAAI/bge-small-en-v1.5 via fastembed v5.13 (ONNX Runtime)
- **Embedding dimensions:** 384 (float32)
- **Index:** linear scan over `Vec<Vec<f32>>` (brute-force cosine similarity)
- **Top-K:** 5
- **One-time setup cost:** ~130 MB model + ~200 MB ONNX Runtime, cached
  locally after first download

We chose brute-force scan over HNSW to match the TF-IDF baseline's
indexing strategy and to avoid introducing ANN recall as a confound.
At 1000 documents, brute-force is fast enough for criterion to sample.

### Corpus design

The TF-IDF benchmark used generic wing names (engineering, product,
infrastructure, research, operations) that **do not match TACT's
production wing rules**. This caused 0% precision for Spectral — a real
result, but one that tests the wrong thing (generic vocabulary vs.
domain-specific rules).

The vector comparison corpus uses vocabulary that matches the production
wing rules:

| Wing | Trigger examples |
|---|---|
| apollo | apollo, polymarket, weather, prediction, wager, trade |
| acme | acme, ladle, mel, recipe, cook, feast |
| infra | infrastructure, ollama, taskforge, litellm, gemma |
| alice | alice, coffee, anniversary, colour, noah, leo |
| polaris | polaris, wlr, plogging, summit, marathon |

Each memory combines a hall-specific trigger phrase (e.g., "Decided to"
for fact) with wing-specific vocabulary and topic content. Wing
vocabularies are isolated — no memory uses trigger words from a different
wing.

The corpus uses the same deterministic seed (`0x5BEC_78A1`) and LCG RNG
as the TF-IDF benchmark. 1000 memories, ~200 per wing, ~50 per
wing/hall combination.

### Query design

20 queries in 4 categories (5 each), generated by documented rules:

| Category | Rule | Expected winner |
|---|---|---|
| **Keyword overlap** | Uses exact trigger vocabulary from one wing. | Baseline for both. |
| **Paraphrase** | Rephrases the same concepts with zero trigger words. | Vector (semantic similarity bridges vocabulary gap). |
| **Multi-hop topical** | Contains wing trigger + hall trigger words. TACT should activate fingerprint search. | Spectral (cross-hall fingerprint traversal). |
| **Vocabulary bridge** | Completely different vocabulary, conceptual connection to wing(s). | Hard for both. |

The paraphrase queries were manually verified to contain **no** wing
trigger words (checked against all 8 wing regexes). Two of the five
paraphrase queries also have zero word overlap with the corpus, making
them pure vocabulary-gap tests.

### What we expected

- Vector should win on paraphrase queries (its forte).
- Spectral should win on multi-hop topical queries when fingerprints fire.
- Latency: Spectral faster with cold queries (no encoding), vector
  faster with pre-encoded queries.
- Operational: Spectral has clear cost advantages (no model in RAM,
  no encoding step).

### What we found

See RESULTS.md for the full numbers. Key findings:

1. **Vector wins on paraphrase queries** (1.00 vs 0.60 P@5). The two
   queries with truly zero word overlap scored 0.00 for Spectral. The
   other three had accidental FTS word overlap ("cluster", "family",
   "event") that let Spectral's FTS fallback succeed.

2. **Multi-hop topical queries tied** (both 1.00 P@5). Spectral's
   fingerprint search correctly activated and retrieved within-wing
   memories. But the queries contained enough keywords for vector search
   to find the same wing's memories via cosine similarity. The
   fingerprint advantage would be more visible with queries that trigger
   a wing+hall but share no vocabulary with the target memories.

3. **Spectral is 6.8x faster with cold queries** (no pre-computed
   embeddings). Vector warm (pre-encoded) is 1.6x faster than Spectral.

4. **Spectral cold-starts 4x faster** (34ms vs 143ms) and has no
   per-query encoding cost (~5ms saved per query).

5. **Spectral has larger per-memory disk footprint** (22 MB for 1k
   memories vs 1.5 MB for raw embeddings), but vector requires ~330 MB
   of model + runtime overhead regardless of corpus size.
