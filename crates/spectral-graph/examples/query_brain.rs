//! Query an existing migrated Spectral brain.
//! Usage: cargo run --release -p spectral-graph --example query_brain -- <brain-path> <query...>

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig};
use std::env;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: query_brain <brain-path> <query...>");
        std::process::exit(1);
    }
    let brain_path = PathBuf::from(&args[1]);
    let query = args[2..].join(" ");

    let ontology_path = brain_path.join("ontology.toml");
    if !ontology_path.exists() {
        eprintln!("Error: ontology.toml not found at {}", ontology_path.display());
        std::process::exit(1);
    }

    let brain = Brain::open(BrainConfig {
        data_dir: brain_path.clone(),
        ontology_path,
        memory_db_path: None,
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
    })?;

    let result = brain.recall(&query, Visibility::Private)?;

    println!("Query: {}\n", query);
    println!("Memory hits ({}):", result.memory_hits.len());
    for (i, hit) in result.memory_hits.iter().take(10).enumerate() {
        let preview: String = hit.content.chars().take(140).collect();
        println!("{}. [{} / {}] score={:.2}",
            i + 1,
            hit.wing.as_deref().unwrap_or("?"),
            hit.hall.as_deref().unwrap_or("?"),
            hit.signal_score
        );
        println!("   {}", preview.replace('\n', " "));
    }

    Ok(())
}
