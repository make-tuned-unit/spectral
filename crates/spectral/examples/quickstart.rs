//! Spectral quickstart: open a brain, remember facts, recall them.
//!
//! Run with: cargo run --example quickstart -p spectral

use spectral::{Brain, Visibility};

fn main() -> Result<(), spectral::Error> {
    let dir = tempfile::tempdir().unwrap();
    let brain = Brain::open(dir.path())?;
    println!("Brain ID: {}", brain.brain_id());

    // Remember some observations
    brain.remember(
        "auth-decision",
        "Decided to use Clerk for authentication",
        Visibility::Private,
    )?;
    brain.remember(
        "deploy-insight",
        "Learned that blue-green deploys reduce downtime",
        Visibility::Public,
    )?;
    brain.remember(
        "apollo-strategy",
        "Apollo weather prediction strategy looks promising",
        Visibility::Private,
    )?;

    // Recall — hybrid search across memory store
    let result = brain.recall_local("what did we decide about auth")?;
    println!("\nRecall 'auth': {} memory hits", result.memory_hits.len());
    for hit in &result.memory_hits {
        println!("  [{}] {}", hit.key, hit.content);
    }

    let result = brain.recall("apollo weather prediction strategy", Visibility::Private)?;
    println!(
        "\nRecall 'apollo': {} memory hits",
        result.memory_hits.len()
    );
    for hit in &result.memory_hits {
        println!("  [{}] {}", hit.key, hit.content);
    }

    Ok(())
}
