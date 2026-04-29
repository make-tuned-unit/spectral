//! Smoke-test the Brain API: graph + memory + hybrid recall + visibility filtering.
//! Run with: cargo run --example try_brain -p spectral-graph

use spectral_core::device_id::DeviceId;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, RememberOpts};

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
        device_id: None,
        enable_spectrogram: false,
        entity_policy: spectral_graph::brain::EntityPolicy::Strict,
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
        "polybot-decision",
        "Decided to use Polybot for the weather prediction strategy",
        Visibility::Private,
    )?;
    brain.remember(
        "polybot-public",
        "Polybot weather predictions are open source and publicly available",
        Visibility::Public,
    )?;

    // Recall with Private context — sees everything
    let all = brain.recall("polybot weather strategy", Visibility::Private)?;
    println!("\nrecall(Private): {} memory hits", all.memory_hits.len());
    assert!(!all.memory_hits.is_empty());

    // Recall with Public context — sees only Public memories
    let public_only = brain.recall("polybot weather strategy", Visibility::Public)?;
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

    // Provenance metadata demonstration
    let device = DeviceId::from_descriptor("smoke-test-host");
    brain.remember_with(
        "polybot-provenance",
        "Decided to use Polybot for weather prediction with provenance",
        RememberOpts {
            source: Some("native".into()),
            device_id: Some(device),
            confidence: Some(0.95),
            visibility: Visibility::Private,
        },
    )?;

    let recall = brain.recall("polybot weather prediction provenance", Visibility::Private)?;
    println!("\nProvenance metadata ({} hits):", recall.memory_hits.len());
    for hit in &recall.memory_hits {
        println!(
            "  source={:?} device={:?} confidence={:.2}: {}",
            hit.source.as_deref(),
            hit.device_id.map(DeviceId::from_bytes),
            hit.confidence,
            &hit.content[..hit.content.len().min(60)],
        );
    }

    // ── LLM-based text ingestion (optional) ─────────────────────────
    // Only runs if a real LLM is available via OPENAI_API_KEY env var.
    if std::env::var("OPENAI_API_KEY").is_ok() {
        println!("\n--- LLM text ingestion ---");
        // When running with a real LLM, uncomment and configure:
        // let llm = spectral::llm::HttpLlmClient::openai(api_key);
        // Then build a brain with .llm_client(Box::new(llm)) and call:
        // brain.ingest_text("Alice studies mathematics at Stanford.", opts)?;
        println!("(Set OPENAI_API_KEY to enable live LLM extraction demo)");
    }

    Ok(())
}
