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
projects (apollo, acme, alice, etc. — from the production taskforge
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

## How to reproduce

```bash
cargo bench --bench retrieval -p spectral
```

Criterion HTML reports are generated in `target/criterion/`.
