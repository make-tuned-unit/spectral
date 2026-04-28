# Spectral

A frequency-domain memory system for AI agents, designed for federation.

**Status:** experimental, pre-1.0. APIs will change.

## Quick start

```rust
use spectral::{Brain, Visibility};

let brain = Brain::open("./my-brain")?;

// Remember observations
brain.remember("auth-decision", "Decided to use Clerk for auth")?;

// Recall with hybrid search (memory fingerprints + graph traversal)
let result = brain.recall("what was the auth decision")?;
for hit in &result.memory_hits {
    println!("[{}] {}", hit.key, hit.content);
}

// Assert typed facts (requires an ontology)
brain.assert("Alice", "knows", "Bob", 1.0, Visibility::Private)?;
```

See [`examples/quickstart.rs`](crates/spectral/examples/quickstart.rs) for a
complete runnable example.

## What it is

Spectral gives your agent two complementary memory systems:

- **Knowledge graph** (Kuzu) — typed entity relationships with ontology
  validation, 2-hop neighborhood traversal, and federation-ready provenance
- **Fingerprint store** (SQLite + FTS5) — TACT-based topical retrieval using
  deterministic SHA-256 fingerprints inspired by Shazam's audio matching

Both are accessible through a single `Brain` handle.

## Architecture

| Crate | Role |
|---|---|
| `spectral` | Umbrella API: `Brain::open()`, `BrainBuilder`, `HttpLlmClient` |
| `spectral-core` | Content-addressed entity IDs, Ed25519 identity, visibility levels |
| `spectral-graph` | Kuzu graph store, TOML ontology, canonicalization, Brain internals |
| `spectral-ingest` | Memory ingestion: classify, score, fingerprint (Constellation) |
| `spectral-tact` | TACT retrieval: fingerprint → wing → FTS multi-tier search |
| `spectral-spectrogram` | *(reserved)* Phase 2 cognitive cross-wing matching |

> **Note:** Fingerprint generation (called "Constellation" in the original
> taskforge codebase) lives in `spectral-ingest::fingerprint`. There is no
> separate `spectral-constellation` crate.

## Performance

Sub-millisecond recall at 1000 memories. ~2,500 ingests/sec on empty brain.
See [benches/RESULTS.md](benches/RESULTS.md) for current numbers and
[benches/METHODOLOGY.md](benches/METHODOLOGY.md) for how they were measured.

```bash
cargo bench --bench retrieval -p spectral
```

### Compared to vector search

Spectral is **6.8x faster** than neural vector search (BGE-small-en-v1.5)
on cold queries (where the query must be encoded). Vector search is 1.6x
faster when queries are pre-encoded.

On retrieval quality, vector search wins on **paraphrase queries** where
the query uses completely different vocabulary than the corpus (1.00 vs
0.60 P@5). Both systems tie on keyword, multi-hop, and vocabulary-bridge
queries.

Spectral requires no embedding model (~0 MB baseline vs ~330 MB for
model + ONNX Runtime), no GPU, and no per-query encoding cost (~5 ms
saved per query).

```bash
cargo bench --bench vector_comparison -p spectral  # downloads ~330 MB on first run
```

See the [neural vector comparison](benches/RESULTS.md#spectral-vs-neural-vector-embeddings-bge-small-en-v15)
section for the full breakdown.

## Operational considerations

See [docs/operational-considerations.md](docs/operational-considerations.md)
for crash recovery, concurrency, and production deployment guidance.

## Design principles

- **Content-addressed entity IDs.** Same entity, same ID, every brain.
- **Embedded by default.** Single binary, no external services required.
- **Rust-native.** Predictable performance, deterministic behavior.
- **Federation-ready.** Every node and edge carries provenance and visibility from day one.

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
