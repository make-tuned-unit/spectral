<p align="center">
  <img src="assets/logo/mark_primary_128px.svg" alt="Spectral logo" width="80">
</p>

<h1 align="center">Spectral</h1>

<p align="center">
  A deterministic, embedding-free memory for AI agents — it recalls what you
  know, recognizes what it has seen, and adapts through use. Federation-ready.
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
`Brain` handle, unified on one embedded **SQLite** database: a typed knowledge
graph for entity relationships with ontology validation and 2-hop traversal, and
a deterministic full-text recall path (SQLite FTS5 + BM25) with signal-score and
recency ranking. No embedding model, no vector database, no LLM in the recall
path. Recall porter-stems by default, so plural and inflected queries match
singular content ("doctors" → "doctor") — deterministic, zero tokens.

## Recall *and* recognition

Most memory systems only *recall* — "what do I know about X?". Spectral adds a
second, independent mode: *recognition* — **"have I encountered this before, and
what happened last time?"**

- **Recall** retrieves what you know: deterministic FTS5 + BM25 over your
  memories, porter-stemmed, signal-score and recency ranked. No embeddings, no
  model, sub-millisecond on 1k memories, and ranked on transparent lexical
  signals (BM25, signal-score, recency) rather than an opaque vector distance.
- **Recognition** scores *familiarity vs novelty* for a new stimulus with an
  embedding-free engine built from three lineages: **landmark fingerprinting**
  (a stimulus's statistically salient features — rare stems, numbers,
  identifiers — scored by IDF; the text analog of spectral peaks above the noise
  floor, and where the name *Spectral* comes from), **Shazam-style pair hashes**
  of co-occurring landmarks (robust to reformatting and near-duplicate edits),
  and **winnowed k-grams** with the MOSS guarantee (any shared verbatim run of
  *w + k − 1* tokens is detected). Scoring borrows from cognitive psychology —
  REM's log-inverse-frequency weighting and MINERVA 2's echo aggregation — into a
  familiarity scalar; **novelty = 1 − familiarity**. No embeddings, no model,
  and every verdict carries the exact features that produced it. Recognition is
  strong at near-duplicate and verbatim re-encounter detection (its measured
  strength); it is **not** a paraphrase/semantic matcher — full paraphrases share
  few landmarks. It powers deduplication and recurrence feedback.

## A memory that adapts through use

Spectral is not a static index. Recall feeds a deterministic **ambient feedback
loop** — the closest thing to "what fires together, wires together" without a
neural net:

- **Use strengthens.** Every recall reinforces the memories it returns (a small
  signal-score nudge), so what the agent actually uses rises over time. This is a
  read with a write side effect: the write-back is batched into one transaction
  by default, and can be moved fully off the recall path with opt-in async
  write-back (`set_async_writeback`).
- **Disuse fades.** An opt-in maintenance pass (the Archivist) decays memories
  left unreinforced past an idle threshold — down-weighting the stale toward a
  floor, never silently deleting it. Decay is a maintenance operation you invoke,
  not an automatic background clock.
- **Context lifts what's relevant now.** A `RecognitionContext` (current focus,
  recent activity, freshness) re-ranks toward what you're doing right now —
  penalty-only, so it never hijacks a query that explicitly names something else.
- **Co-access becomes anticipation.** Retrieval events are mined into lift-based
  associations, so Spectral surfaces what your current context is *specifically*
  associated with — suppressing globally-popular memories the way a good
  recommender avoids pushing bestsellers at everyone.

All of it is deterministic, local, and free of any model or LLM. See
[docs/internal](docs/internal) for the recognition/spectrogram design and
measurements.

## Speed

The numbers: 6.8x faster than neural vector search (BGE-small-en-v1.5) on
cold queries where the query must be encoded. Sub-millisecond recall on 1,000
memories. ~2,500 ingests/sec. No GPU, no model weights, no per-query encoding
cost. See [benchmarks](benches/RESULTS.md) for the full breakdown.

## Results (accuracy)

On **LongMemEval-S** (500 long-term-memory QA questions), Spectral scores
**81.5% (401/492)** with a Claude Sonnet 4.6 actor — at a memory-layer
retrieval overhead of **~169 tokens/query** and **~17 ms median retrieval
latency**. The core recall path makes no LLM call, but this benchmarked config
front-runs one **optional** pre-retrieval query-expansion call (Claude Haiku)
— that call is the only memory-layer LLM cost, and it is what the **≈ $0.25/1k
queries** figure measures. Recall without expansion is fully LLM-free.

Honest framing: this is in-sample (the retrieval was developed against this
dataset), so it is not a state-of-the-art claim; held-out numbers are expected
to be lower. The denominator is 492 (8 transport failures quarantined), and
most remaining failures are actor-side synthesis, not retrieval misses.

Full results, per-category breakdown, and limitations: [docs/RESULTS.md](docs/RESULTS.md).

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
  │ (SQLite +  │   │ (classify + │   │ (retrieval) │
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

**spectral-graph** owns the SQLite-backed entity graph store (entities, triples,
2-hop neighborhood), the TOML ontology loader, canonicalization (fuzzy entity
resolution), and the `Brain` implementation. (The graph layer was formerly a
separate embedded Kuzu database; it now runs on the same SQLite engine as memory
and FTS — one embedded dependency, no second graph engine.)

**spectral-ingest** classifies incoming text into wings (topic areas) and halls
(memory types), computes signal scores, generates constellation fingerprints, and
writes to SQLite (memories, FTS index, episodes, fingerprints).

**spectral-tact** (Topic-Aware Context Triage) handles retrieval. The
production recall path is deterministic FTS5 + BM25 with signal-score, recency,
and episode-diversity re-ranking; TACT's fingerprint/wing tiers and associative
co-occurrence spreading are complementary layers under active measurement.

**spectral-core** provides content-addressed entity IDs, Ed25519 brain
identity, device IDs, and the four-level visibility system.

**spectral-recognition** is the embedding-free recognition engine (sidecar
`recognition.db`): landmark fingerprinting, Shazam-style pair hashes, and
winnowed k-grams scored into a familiarity/novelty verdict. Answers "have I seen
this before?" — powering deduplication, recurrence feedback, and near-duplicate
detection, deterministically and with fully explainable verdicts.

**spectral-spectrogram** classifies memories along seven cognitive dimensions
(entity density, action type, emotional valence, etc.). Opt-in and experimental
via `BrainConfig::enable_spectrogram`; the cross-wing resonance reader is not on
the default recall path.

Read-time **federation** (the `FederationCoordinator` fan-out) lives in
`spectral-graph`, built entirely above the `Brain` API — no core changes.

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

**Read-time fan-out (shipped).** A `FederationCoordinator` queries N held brains
on each recall, tags every hit with its origin brain, and merges them into one
provenance-ranked list — with the trust and privacy properties multi-contributor
memory actually needs:

- **Visibility boundary enforced coordinator-side** — a member's Private
  memories never cross into a Team/Org/Public context.
- **Poisoning-resistant merge (Reciprocal Rank Fusion).** A member controls its
  own memories' scores, so ranking on raw score lets one peer flood the top by
  self-asserting max-score, keyword-stuffed entries. RRF ranks on *position*, not
  self-asserted score, and *sums across* members — so a lone assertion can't
  dominate and independently-corroborated content rises. A per-child cap bounds
  flooding volume.
- **Read-only children** — a coordinator opens foreign brains read-only, so a
  query writes no reinforce nudges or `query_hash` side-channels into a peer's
  store.
- **Graceful degradation** — one unhealthy member (locked/corrupt DB) is skipped
  and reported, never aborts the whole query.

What is not built yet: write-merge into a shared brain and cross-machine
transport (sync, conflict resolution). Read-time federation over co-resident
brains works today.

## Performance

| Metric | Value |
|---|---|
| Ingest throughput (empty brain) | ~2,540 ops/sec (393 us each) |
| Recall latency (1,000 memories) | 564 us p50 |
| Batch ingest (100 memories, fresh brain) | 235 ms total (~425/sec) |
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
| **Spectral** | Not required | Yes (SQLite, 2-hop BFS) | Yes (identity, visibility, provenance) | Embedded, single binary |
| Vector DBs (Pinecone, Qdrant, Weaviate) | Required | No | No | Hosted service or self-hosted server |
| Cognee | Required | Yes (Cognify) | No | Python, external services |
| sqlite-vss / fastembed | Required | No | No | Embedded library |

Vector databases win at pure semantic similarity search, especially on
paraphrase queries where the query vocabulary differs entirely from the corpus.
Cognee has a mature graph extraction pipeline (Cognify). Spectral matches vector
search on multi-hop topical queries **without requiring a model** (a tie on our
small synthetic suite — see [benches/RESULTS.md](benches/RESULTS.md); a
keyword-free multi-hop test is future work), and is the only option here with
built-in federation primitives. Pick the tool that matches your query
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

## Examples

See [examples/](examples/) for integration patterns:
- Conversational memory — chat-based agents with persistent context
- Activity capture — agents that observe and learn from user activity

For runnable code, see `crates/spectral/examples/quickstart.rs`.

## Roadmap

- ✅ Hybrid memory (knowledge graph + fingerprint store + FTS fallback)
- ✅ Federation primitives (brain identity, device IDs, visibility, provenance)
- ✅ Natural-language ingest via LLM ([#12](https://github.com/make-tuned-unit/spectral/pull/12))
- ✅ Crash recovery and concurrency tested ([#8](https://github.com/make-tuned-unit/spectral/pull/8))
- ✅ Benchmarked vs TF-IDF and neural vectors ([#9](https://github.com/make-tuned-unit/spectral/pull/9), [#10](https://github.com/make-tuned-unit/spectral/pull/10))
- ✅ Performance optimizations from production audit ([#13](https://github.com/make-tuned-unit/spectral/pull/13))
- ✅ Cognitive Spectrogram (cross-wing matching, [#16](https://github.com/make-tuned-unit/spectral/pull/16))
- ✅ Recognition engine (landmark fingerprinting, familiarity/novelty scoring)
- ✅ Ambient feedback loop (use-driven reinforce, disuse decay, lift-based anticipation)
- ✅ Read-time federation (fan-out coordinator: RRF poisoning-resistance, visibility boundary, provenance, graceful degradation)
- 📋 Federation write-merge and cross-machine sync (conflict resolution)
- 📋 brain.db migration tooling

## Operational considerations

See [docs/operational-considerations.md](docs/operational-considerations.md)
for crash recovery, concurrency limits, and production deployment guidance.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup. We follow the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). For security issues, see [SECURITY.md](SECURITY.md).

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
