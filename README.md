<p align="center">
  <img src="assets/logo/mark_primary_128px.svg" alt="Spectral logo" width="80">
</p>

<h1 align="center">Spectral</h1>

<p align="center">
  A frequency-domain memory system for AI agents, designed for federation.
</p>

<p align="center">
  <a href="https://github.com/make-tuned-unit/spectral/actions/workflows/ci.yml"><img src="https://github.com/make-tuned-unit/spectral/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg" alt="License: Apache 2.0"></a>
  <a href="https://crates.io/crates/spectral"><img src="https://img.shields.io/badge/crates.io-coming%20soon-orange.svg" alt="crates.io"></a>
</p>

**Status:** v0.0.1, experimental. APIs will change before 1.0.

## What Spectral does

AI agents need persistent memory that supports both "what do I know about X?"
and "how does X relate to Y?" Vector databases answer the first question well.
They struggle with the second. Retrieving a fact by embedding similarity does
not give you the two-hop chain of relationships that led to that fact.

Spectral gives your agent two complementary memory systems behind a single
`Brain` handle: a typed knowledge graph (Kuzu) for entity relationships with
ontology validation and multi-hop traversal, and a fingerprint store (SQLite +
FTS5) for fast topical retrieval using deterministic SHA-256 fingerprints.
The fingerprint approach is inspired by Shazam's audio matching. No embedding
model required.

The numbers: 6.8x faster than neural vector search (BGE-small-en-v1.5) on
cold queries where the query must be encoded. Sub-millisecond recall on 1,000
memories. ~2,500 ingests/sec. No GPU, no model weights, no per-query encoding
cost. See [benchmarks](benches/RESULTS.md) for the full breakdown.

## Quick start

Add Spectral via git dependency (crates.io publication coming soon):

```toml
[dependencies]
spectral = { git = "https://github.com/make-tuned-unit/spectral" }
```

```rust
use spectral::{Brain, Visibility};

// Open or create a brain at a directory path
let brain = Brain::open("./my-brain")?;

// Remember free-text observations
brain.remember("auth-decision", "Decided to use Clerk for auth", Visibility::Private)?;

// Recall with hybrid search (fingerprints + graph + FTS)
let result = brain.recall_local("what was the auth decision")?;
for hit in &result.memory_hits {
    println!("[{}] {}", hit.key, hit.content);
}

// Assert typed facts (requires an ontology)
brain.assert("Alice", "knows", "Bob", 1.0, Visibility::Private)?;
# Ok::<(), spectral::Error>(())
```

See [`examples/quickstart.rs`](crates/spectral/examples/quickstart.rs) for a
complete runnable example.

## Architecture

```
                    ┌─────────────┐
                    │  spectral   │  <- public API
                    │ (umbrella)  │
                    └──────┬──────┘
                           │
        ┌──────────────────┼──────────────────┐
        │                  │                  │
  ┌─────▼──────┐   ┌──────▼──────┐   ┌──────▼──────┐
  │ spectral-  │   │ spectral-   │   │ spectral-   │
  │   graph    │   │   ingest    │   │    tact     │
  │ (Kuzu +    │   │ (classify + │   │ (retrieval) │
  │  ontology) │   │ fingerprint)│   │             │
  └─────┬──────┘   └──────┬──────┘   └──────┬──────┘
        │                  │                  │
        └──────────────────┼──────────────────┘
                           │
                    ┌──────▼──────┐
                    │spectral-core│  <- identity, IDs, visibility
                    └─────────────┘
```

**spectral** is the umbrella crate. It re-exports `Brain`, `BrainBuilder`, and
all result types. Most users only need this crate.

**spectral-graph** owns the Kuzu graph database, TOML ontology loader,
canonicalization (fuzzy entity resolution), and the `Brain` implementation.

**spectral-ingest** classifies incoming text into wings (topic areas) and halls
(memory types), computes signal scores, generates SHA-256 constellation
fingerprints, and writes to SQLite.

**spectral-tact** (Topic-Aware Context Triage) handles retrieval. It routes
queries through fingerprint search, wing-scoped search, and FTS fallback,
then merges and deduplicates results.

**spectral-core** provides content-addressed entity IDs, Ed25519 brain
identity, device IDs, and the four-level visibility system.

**spectral-spectrogram** classifies memories along seven cognitive dimensions
(entity density, action type, emotional valence, etc.) and finds resonant
memories across wings. Opt-in via `BrainConfig::enable_spectrogram`.

## Federation primitives

"Designed for federation" is a specific claim. Here is what backs it up today.

**Content-addressed entity IDs.** Entity IDs are derived from
`blake3(entity_type || canonical_name)`. The same entity produces the same ID
on every machine. No central ID authority needed.

**Ed25519 brain identity.** Each brain generates a keypair on first open. The
brain ID is the public key hash. Every triple carries `source_brain_id` so you
can trace which brain asserted a fact.

**Content-addressed device IDs.** `DeviceId::from_descriptor("hostname")` is
deterministic. Memories carry `device_id` for multi-device provenance.

**Four-level visibility.** Every entity, triple, and memory has a visibility
level: Private, Team, Org, or Public. Recall filters by visibility context.
A Public query only sees Public data. A Private query sees everything. This is
enforced on read, not on write.

**Source attribution.** Every memory carries an optional `source` field
("native", "openbird_sidecar", "import") for tracking origin across systems.

**Versioned salt schemes.** Fingerprint hashes use a documented salt format
for forward compatibility when the algorithm evolves.

What is not built yet: the federation protocol itself (sync, conflict
resolution, merge semantics). The primitives above are in place so that
when federation ships, existing data is already annotated correctly.

## Performance

| Metric | Value |
|---|---|
| Ingest throughput (empty brain) | ~2,540 ops/sec (393 us each) |
| Recall latency (1,000 memories) | 564 us p50 |
| Batch ingest (100 memories) | 98 ms total (~1,020/sec) |
| vs neural vector cold query | 6.8x faster |
| vs neural vector warm query | 1.6x slower (no encoding step to skip) |

Full results: [benches/RESULTS.md](benches/RESULTS.md).
Methodology: [benches/METHODOLOGY.md](benches/METHODOLOGY.md).

```bash
cargo bench --bench retrieval -p spectral
cargo bench --bench vector_comparison -p spectral  # downloads ~330 MB model on first run
```

## Comparison with alternatives

| System | Embedding model | Multi-hop graph | Federation primitives | Deployment |
|---|---|---|---|---|
| **Spectral** | Not required | Yes (Kuzu, 2-hop BFS) | Yes (identity, visibility, provenance) | Embedded, single binary |
| Vector DBs (Pinecone, Qdrant, Weaviate) | Required | No | No | Hosted service or self-hosted server |
| Cognee | Required | Yes (Cognify) | No | Python, external services |
| sqlite-vss / fastembed | Required | No | No | Embedded library |

Vector databases win at pure semantic similarity search, especially on
paraphrase queries where the query vocabulary differs entirely from the corpus.
Cognee has a mature graph extraction pipeline (Cognify). Spectral wins at
multi-hop topical queries without requiring a model, and is the only option
with built-in federation primitives. Pick the tool that matches your query
pattern.

## When to use Spectral

**Good fit:**
- Agent memory at single-machine scale (under ~10k memories today)
- Multi-hop reasoning matters ("how does X relate to Y through Z?")
- No GPU or embedding model infrastructure available
- Multi-device or federation is on your roadmap
- You want a single embedded dependency, not a service

**Not the right fit:**
- Pure semantic similarity is your dominant query pattern (use a vector DB)
- You need approximate nearest neighbor search at 100k+ scale
- Single-shot Q&A without persistent state

## Roadmap

- ✅ Hybrid memory (knowledge graph + fingerprint store + FTS fallback)
- ✅ Federation primitives (brain identity, device IDs, visibility, provenance)
- ✅ Natural-language ingest via LLM ([#12](https://github.com/make-tuned-unit/spectral/pull/12))
- ✅ Crash recovery and concurrency tested ([#8](https://github.com/make-tuned-unit/spectral/pull/8))
- ✅ Benchmarked vs TF-IDF and neural vectors ([#9](https://github.com/make-tuned-unit/spectral/pull/9), [#10](https://github.com/make-tuned-unit/spectral/pull/10))
- ✅ Performance optimizations from production audit ([#13](https://github.com/make-tuned-unit/spectral/pull/13))
- ✅ Cognitive Spectrogram (cross-wing matching, [#16](https://github.com/make-tuned-unit/spectral/pull/16))
- 📋 Memify feedback loop (recall quality improves with use)
- 📋 Federation protocol (sync, conflict resolution, merge)
- 📋 brain.db migration tooling

## Operational considerations

See [docs/operational-considerations.md](docs/operational-considerations.md)
for crash recovery, concurrency limits, and production deployment guidance.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Short version: run `cargo test` before
opening a PR.

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
