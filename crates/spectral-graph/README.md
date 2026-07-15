# spectral-graph

Knowledge graph layer for Spectral. Stores entities, triples, and provenance
in an embedded SQLite graph store (the same engine as the memory/FTS store).
Canonicalizes free-text mentions through a TOML ontology.

## Quick start

```rust
use spectral_graph::brain::{Brain, BrainConfig};
use spectral_core::visibility::Visibility;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let brain = Brain::open(BrainConfig {
        data_dir: PathBuf::from("./my-brain"),
        ontology_path: PathBuf::from("ontology.toml"),
    })?;

    // Assert facts
    brain.assert("Carol", "works_on", "Spectral", 0.95, Visibility::Private)?;
    brain.assert("Mark", "studies", "Library", 0.9, Visibility::Private)?;
    brain.assert("Mark", "prepares_for", "Exam", 0.9, Visibility::Private)?;

    // Recall with 2-hop traversal
    let result = brain.recall("Mark")?;
    println!("Found {} entities, {} triples",
        result.neighborhood.entities.len(),
        result.neighborhood.triples.len());

    for triple in &result.triples {
        println!("  {} --{}-> (confidence: {:.0}%)",
            triple.predicate, triple.predicate, triple.confidence * 100.0);
    }

    Ok(())
}
```

## Architecture

- **`brain.rs`** — High-level API: `assert`, `recall`, `ingest_document`
- **`canonicalize.rs`** — Resolves text mentions to EntityIds (exact + fuzzy)
- **`ontology.rs`** — TOML ontology loader with domain/range validation
- **`graph_store.rs`** — SQLite-backed entity/triple/mention store + 2-hop
  neighborhood BFS (entity, triple, document, mention tables)
- **`provenance.rs`** — Per-edge provenance metadata types
