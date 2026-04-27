//! Smoke-test the Brain API against a small synthetic ontology.
//! Run with: cargo run --example try_brain -p spectral-graph

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig};

fn main() -> anyhow::Result<()> {
    let tmpdir = tempfile::tempdir()?;
    let data_dir = tmpdir.path().to_path_buf();
    let ontology_path = std::env::current_dir()?
        .join("crates/spectral-graph/examples/try_brain_ontology.toml");

    println!("Opening fresh brain at {:?}", data_dir);
    let brain = Brain::open(BrainConfig { data_dir, ontology_path })?;
    println!("Brain ID: {}", brain.brain_id());
    println!();

    println!("=== Asserting facts ===");
    let r = brain.assert("Alice", "knows", "Bob", 1.0, Visibility::Private)?;
    println!("  {} -> {} -> {}", r.subject.canonical, r.predicate, r.object.canonical);

    let r = brain.assert("Bob", "knows", "Carol", 1.0, Visibility::Private)?;
    println!("  {} -> {} -> {}", r.subject.canonical, r.predicate, r.object.canonical);

    let r = brain.assert("Carol", "studies", "Math", 1.0, Visibility::Private)?;
    println!("  {} -> {} -> {}", r.subject.canonical, r.predicate, r.object.canonical);

    println!();

    println!("=== Recall: 'Alice' (2-hop neighborhood) ===");
    let recall = brain.recall("Alice")?;
    println!("Seed entities: {}", recall.seed_entities.len());
    println!("Entities reached: {}", recall.neighborhood.entities.len());
    println!("Triples found: {}", recall.triples.len());
    for t in &recall.triples {
        let from_short = &t.from.to_string()[..8];
        let to_short = &t.to.to_string()[..8];
        println!("  {} --{}--> {} (conf={})", from_short, t.predicate, to_short, t.confidence);
    }

    println!();

    println!("=== Recall: 'Bob' (should reach Math via Carol) ===");
    let recall = brain.recall("Bob")?;
    println!("Triples found: {}", recall.triples.len());
    for t in &recall.triples {
        let from_short = &t.from.to_string()[..8];
        let to_short = &t.to.to_string()[..8];
        println!("  {} --{}--> {} (conf={})", from_short, t.predicate, to_short, t.confidence);
    }

    println!();

    println!("=== Error case: unknown mention ===");
    match brain.assert("Eve", "knows", "Bob", 1.0, Visibility::Private) {
        Ok(_) => println!("  unexpected success"),
        Err(e) => println!("  got error (good): {:?}", e),
    }

    println!();

    println!("=== Error case: predicate mismatch ===");
    match brain.assert("Alice", "studies", "Bob", 1.0, Visibility::Private) {
        Ok(_) => println!("  unexpected success"),
        Err(e) => println!("  got error (good): {:?}", e),
    }

    Ok(())
}
