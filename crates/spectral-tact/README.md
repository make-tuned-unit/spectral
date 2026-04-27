# spectral-tact

TACT (Topic-Aware Context Triage) — fingerprint-based memory retrieval for
Spectral. Finds relevant memories from a `MemoryStore` and formats them as a
context block for LLM system-prompt injection. No embedding inference required.

## Quick start

```rust
use spectral_tact::{retrieve, TactConfig, MemoryStore};
use spectral_ingest::sqlite_store::SqliteStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = SqliteStore::open_in_memory()?;
    let config = TactConfig::default();

    // (after ingesting memories via spectral-ingest...)
    let result = retrieve(
        "what was the auth decision for the project?",
        &config, &store,
    ).await?;

    println!("method={} hits={}", result.method, result.memories.len());
    if !result.context_block.is_empty() {
        println!("{}", result.context_block);
    }

    Ok(())
}
```

## Retrieval tiers

1. **Fingerprint** — SHA-256 hash lookup (wing + hall detected)
2. **Wing-only** — high-signal memories in detected wing
3. **FTS fallback** — SQLite FTS5 keyword search
4. **Hybrid merge** — fingerprint + FTS results deduplicated
