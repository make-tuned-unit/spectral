//! Smoke-test the Brain API: graph + memory + hybrid recall + visibility filtering.
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

    // Memory ingestion at different visibility levels
    brain.remember(
        "apollo-decision",
        "Decided to use Apollo for the weather prediction strategy",
        Visibility::Private,
    )?;
    brain.remember(
        "apollo-public",
        "Apollo weather predictions are open source and publicly available",
        Visibility::Public,
    )?;

    // Recall with Private context — sees everything
    let all = brain.recall("apollo weather strategy", Visibility::Private)?;
    println!("\nrecall(Private): {} memory hits", all.memory_hits.len());
    assert!(!all.memory_hits.is_empty());

    // Recall with Public context — sees only Public memories
    let public_only = brain.recall("apollo weather strategy", Visibility::Public)?;
    println!(
        "recall(Public):  {} memory hits",
        public_only.memory_hits.len()
    );
    assert!(
        public_only.memory_hits.len() < all.memory_hits.len(),
        "Public context should see fewer memories than Private"
    );
    for hit in &public_only.memory_hits {
        assert_eq!(hit.visibility, "public");
    }

    println!("\nVisibility enforcement verified.");
    Ok(())
}
