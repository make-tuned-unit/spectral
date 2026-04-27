//! Smoke-test the Brain API: graph assertions, memory ingestion, hybrid recall.
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
    })?;
    println!("Brain ID: {}", brain.brain_id());
    println!();

    // ── Graph assertions ──
    println!("=== Asserting facts (graph) ===");
    let r = brain.assert("Alice", "knows", "Bob", 1.0, Visibility::Private)?;
    println!(
        "  {} -> {} -> {}",
        r.subject.canonical, r.predicate, r.object.canonical
    );

    let r = brain.assert("Bob", "knows", "Carol", 1.0, Visibility::Private)?;
    println!(
        "  {} -> {} -> {}",
        r.subject.canonical, r.predicate, r.object.canonical
    );

    let r = brain.assert("Carol", "studies", "Math", 1.0, Visibility::Private)?;
    println!(
        "  {} -> {} -> {}",
        r.subject.canonical, r.predicate, r.object.canonical
    );
    println!();

    // ── Memory ingestion ──
    println!("=== Remembering observations ===");
    let r = brain.remember("alice_pref", "Alice prefers morning meetings")?;
    println!(
        "  remembered: wing={:?} hall={:?} signal={:.2}",
        r.wing, r.hall, r.signal_score
    );

    let r = brain.remember("bob_decision", "Bob decided to use Rust for the rewrite")?;
    println!(
        "  remembered: wing={:?} hall={:?} signal={:.2}",
        r.wing, r.hall, r.signal_score
    );

    let r = brain.remember(
        "carol_insight",
        "Carol learned that async Rust needs careful testing",
    )?;
    println!(
        "  remembered: wing={:?} hall={:?} signal={:.2}",
        r.wing, r.hall, r.signal_score
    );
    println!();

    // ── Hybrid recall ──
    println!("=== Hybrid recall: 'Alice' ===");
    let recall = brain.recall("Alice")?;
    println!(
        "Graph: {} entities, {} triples",
        recall.graph.neighborhood.entities.len(),
        recall.graph.triples.len()
    );
    println!("Memory hits: {}", recall.memory_hits.len());

    for t in &recall.graph.triples {
        let from_short = &t.from.to_string()[..8];
        let to_short = &t.to.to_string()[..8];
        println!(
            "  {} --{}--> {} (conf={})",
            from_short, t.predicate, to_short, t.confidence
        );
    }
    for m in &recall.memory_hits {
        println!("  [memory] {}: {:.60}", m.key, m.content);
    }
    println!();

    // ── Error cases ──
    println!("=== Error: unknown mention ===");
    match brain.assert("Eve", "knows", "Bob", 1.0, Visibility::Private) {
        Ok(_) => println!("  unexpected success"),
        Err(e) => println!("  got error (good): {:?}", e),
    }

    println!("=== Error: predicate mismatch ===");
    match brain.assert("Alice", "studies", "Bob", 1.0, Visibility::Private) {
        Ok(_) => println!("  unexpected success"),
        Err(e) => println!("  got error (good): {:?}", e),
    }

    Ok(())
}
