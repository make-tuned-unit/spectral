//! Smoke-test the Brain API: graph + memory + hybrid recall.
//! Run with: cargo run --example try_brain -p spectral-graph

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig};

fn main() -> anyhow::Result<()> {
    let tmpdir = tempfile::tempdir()?;
    let data_dir = tmpdir.path().to_path_buf();
    let ontology_path =
        std::env::current_dir()?.join("crates/spectral-graph/examples/try_brain_ontology.toml");

    let brain = Brain::open(BrainConfig {
        data_dir,
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
    })?;
    println!("Brain ID: {}", brain.brain_id());

    // Graph assertions
    for (s, p, o) in &[
        ("Alice", "knows", "Bob"),
        ("Bob", "knows", "Carol"),
        ("Carol", "studies", "Math"),
    ] {
        let r = brain.assert(s, p, o, 1.0, Visibility::Private)?;
        println!(
            "assert: {} -> {} -> {}",
            r.subject.canonical, r.predicate, r.object.canonical
        );
    }

    // Memory ingestion
    for (key, content) in &[
        (
            "polybot-decision",
            "Decided to use Polybot for the weather prediction strategy",
        ),
        (
            "polybot-bug",
            "Polybot had a bug in the weather engine that caused real losses",
        ),
        (
            "polybot-fix",
            "Discovered the polybot weather strategy needs paper-trading first",
        ),
    ] {
        let r = brain.remember(key, content)?;
        println!(
            "remember: '{}' wing={:?} hall={:?} signal={:.2}",
            key,
            r.wing.as_deref(),
            r.hall.as_deref(),
            r.signal_score
        );
    }

    // Hybrid recall — memory
    let recall = brain.recall("polybot weather strategy")?;
    println!(
        "\nrecall 'polybot weather strategy': {} memory hits, {} graph triples",
        recall.memory_hits.len(),
        recall.graph.triples.len()
    );
    assert!(
        !recall.memory_hits.is_empty(),
        "BUG: polybot recall returned 0 memory hits"
    );

    // Hybrid recall — graph
    let recall = brain.recall("Alice")?;
    println!(
        "recall 'Alice': {} memory hits, {} graph triples",
        recall.memory_hits.len(),
        recall.graph.triples.len()
    );

    Ok(())
}
