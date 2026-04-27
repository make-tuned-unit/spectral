//! Smoke-test the Brain API: graph + memory + hybrid recall.
//! Run with: cargo run --example try_brain -p spectral-graph

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig};

fn main() -> anyhow::Result<()> {
    let tmpdir = tempfile::tempdir()?;
    let data_dir = tmpdir.path().to_path_buf();
    let ontology_path =
        std::env::current_dir()?.join("crates/spectral-graph/examples/try_brain_ontology.toml");

    println!("Opening fresh brain at {:?}", data_dir);
    let brain = Brain::open(BrainConfig {
        data_dir,
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
    })?;
    println!("Brain ID: {}", brain.brain_id());
    println!();

    println!("=== Asserting facts (graph) ===");
    for (s, p, o) in &[
        ("Alice", "knows", "Bob"),
        ("Bob", "knows", "Carol"),
        ("Carol", "studies", "Math"),
    ] {
        let r = brain.assert(s, p, o, 1.0, Visibility::Private)?;
        println!(
            "  {} -> {} -> {}",
            r.subject.canonical, r.predicate, r.object.canonical
        );
    }
    println!();

    println!("=== Remembering observations (memory) ===");
    for (key, content) in &[
        (
            "apollo-decision",
            "Decided to use Apollo for the weather prediction strategy",
        ),
        (
            "apollo-bug",
            "Apollo had a bug in the weather engine that caused real losses",
        ),
        (
            "apollo-fix",
            "Discovered the apollo weather strategy needs paper-trading first",
        ),
    ] {
        let r = brain.remember(key, content)?;
        println!(
            "  '{}' -> wing={:?} hall={:?} signal={:.2} fingerprints={}",
            key,
            r.wing.as_deref(),
            r.hall.as_deref(),
            r.signal_score,
            r.fingerprints_created
        );
    }
    println!();

    println!("=== Hybrid recall: 'apollo weather strategy' ===");
    let recall = brain.recall("apollo weather strategy")?;
    println!(
        "Graph: {} entities, {} triples",
        recall.graph.neighborhood.entities.len(),
        recall.graph.triples.len()
    );
    println!("Memory hits: {}", recall.memory_hits.len());
    for hit in &recall.memory_hits {
        println!(
            "  [{}/{}] {}: {}",
            hit.wing.as_deref().unwrap_or("?"),
            hit.hall.as_deref().unwrap_or("?"),
            hit.key,
            hit.content.chars().take(60).collect::<String>()
        );
    }
    assert!(
        !recall.memory_hits.is_empty(),
        "BUG: apollo observations were remembered but recall returned 0 memory hits"
    );
    println!();

    println!("=== Hybrid recall: 'Alice' ===");
    let recall = brain.recall("Alice")?;
    println!(
        "Graph: {} entities, {} triples",
        recall.graph.neighborhood.entities.len(),
        recall.graph.triples.len()
    );
    println!(
        "Memory hits: {} (expected 0 — no memory wing matches 'Alice')",
        recall.memory_hits.len()
    );
    for t in &recall.graph.triples {
        let from = &t.from.to_string()[..8];
        let to = &t.to.to_string()[..8];
        println!("  {} --{}--> {}", from, t.predicate, to);
    }

    Ok(())
}
