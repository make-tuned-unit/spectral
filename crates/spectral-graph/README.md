# spectral-graph

Knowledge graph layer for Spectral. Stores entities, triples, and provenance
in an embedded [Kuzu](https://kuzudb.com/) graph database. Canonicalizes
free-text mentions through a TOML ontology.

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
    brain.assert("Sophie", "works_on", "Spectral", 0.95, Visibility::Private)?;
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
- **`kuzu_store.rs`** — Typed Kuzu wrapper with parameterized Cypher queries
- **`schema.rs`** — Graph schema (Entity, Document, Triple, Mentions tables)
- **`provenance.rs`** — Per-edge provenance metadata types
