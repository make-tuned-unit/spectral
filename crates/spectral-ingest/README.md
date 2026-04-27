# spectral-ingest

Memory ingestion pipeline for [TACT](../spectral-tact/) (Topic-Aware Context
Triage). Takes raw text, classifies it into wing/hall categories, computes a
signal score, generates constellation fingerprints, and writes to a
`MemoryStore`.

Fingerprint hashes are byte-identical to the production Python implementation.

## Quick start

```rust
use spectral_ingest::ingest::{ingest, IngestConfig};
use spectral_ingest::sqlite_store::SqliteStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = SqliteStore::open_in_memory()?;
    let config = IngestConfig::default();

    let result = ingest(
        "mem-001", "auth_decision",
        "Jesse decided to use Clerk for auth",
        "core", 1700000000.0, &config, &store,
    ).await?;

    println!("wing={:?} hall={:?} signal={:.2} fps={}",
        result.memory.wing, result.memory.hall,
        result.memory.signal_score, result.fingerprints.len());

    Ok(())
}
```

## Features

- `sqlite` (default) — enables `SqliteStore` backed by rusqlite + FTS5
