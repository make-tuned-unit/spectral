//! Build a **fixture brain** from the public LoCoMo dataset and export its
//! graph as JSON, for honest product imagery.
//!
//! Run:
//! ```text
//! cargo run -p spectral --example fixture_brain -- \
//!     ~/spectral-local-bench/locomo10.json /tmp/fixture-root 2
//! ```
//! Arguments: `<locomo.json> <out-root> [samples]`. Writes the brain to
//! `<out-root>/brain/` and the export to `<out-root>/graph-export.json`.
//!
//! # What is honest about this fixture, and what is not
//!
//! Every memory is a real turn of a real public conversation, ingested with its
//! real session timestamp, so memory counts and episodic structure are genuine.
//!
//! The **graph edges are asserted by this program, not extracted.** That matters
//! and must be stated wherever the imagery appears. Spectral populates a graph
//! two ways: a caller asserts triples, or an **LLM** extracts them
//! (`spectral-graph::extract` is LLM-based; there is no deterministic
//! text-to-triple path). This program takes the first route and asserts only
//! facts that are *mechanically true of the source file* — who spoke in which
//! session, which session precedes which, and who talked to whom. No
//! interpretation of conversation content, and no model call.
//!
//! So a render of this file legitimately shows "a real Spectral graph over real
//! public data". It does **not** show automatic knowledge extraction, and must
//! not be captioned as if it did.
//!
//! Everything is written at `Visibility::Public`, and the export is taken at
//! `Public` scope, so the file cannot contain anything a public reader should
//! not see.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use spectral::{Brain, RememberOpts, Visibility};
use spectral_core::entity_id::EntityId;
use spectral_graph::graph_export::export_neighborhood;
use spectral_graph::graph_store::GraphStore;
use std::path::{Path, PathBuf};

/// LoCoMo stamps look like `1:56 pm on 8 May, 2023`. Tolerant of the
/// unpadded day; returns `None` rather than guessing if the shape changes.
fn parse_locomo_stamp(s: &str) -> Option<DateTime<Utc>> {
    let cleaned = s.trim().replace("  ", " ");
    for fmt in [
        "%l:%M %p on %e %B, %Y",
        "%I:%M %p on %d %B, %Y",
        "%l:%M %p on %e %B %Y",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(&cleaned, fmt) {
            return Some(Utc.from_utc_datetime(&naive));
        }
    }
    None
}

fn session_indices(conv: &serde_json::Map<String, serde_json::Value>) -> Vec<usize> {
    let mut idx: Vec<usize> = conv
        .keys()
        .filter_map(|k| {
            let rest = k.strip_prefix("session_")?;
            if rest.contains("date_time") {
                return None;
            }
            rest.parse::<usize>().ok()
        })
        .collect();
    idx.sort_unstable();
    idx
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        anyhow::bail!(
            "usage: {} <locomo.json> <out-root> [samples]",
            args.first().map(String::as_str).unwrap_or("fixture_brain")
        );
    }
    let locomo_path = PathBuf::from(&args[1]);
    let out_root = PathBuf::from(&args[2]);
    let max_samples: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);

    let brain_dir = out_root.join("brain");
    if brain_dir.exists() {
        anyhow::bail!(
            "{} already exists — refusing to write into an existing brain. \
             Remove it or pick a fresh root.",
            brain_dir.display()
        );
    }
    std::fs::create_dir_all(&brain_dir)?;

    let raw = std::fs::read_to_string(&locomo_path)?;
    let samples: Vec<serde_json::Value> = serde_json::from_str(&raw)?;
    println!(
        "loaded {} LoCoMo samples from {}",
        samples.len(),
        locomo_path.display()
    );

    // First open creates the default `ontology.toml` the runtime also expects
    // in this directory; `auto_ontology()` is private, so the builder below
    // needs that file to exist before it can be pointed at.
    drop(Brain::open(&brain_dir)?);

    // `AutoCreate` so asserting a person who is not yet in the graph creates
    // them, instead of failing with "unresolved mention". Default is Strict.
    let brain = Brain::builder()
        .data_dir(&brain_dir)
        .ontology_path(brain_dir.join("ontology.toml"))
        .entity_policy(spectral::EntityPolicy::AutoCreate)
        .build()?;

    let mut memories = 0usize;
    let mut edges_asserted = 0usize;
    let mut undated = 0usize;
    let mut start_entity: Option<EntityId> = None;

    for sample in samples.iter().take(max_samples) {
        let sample_id = sample
            .get("sample_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let Some(conv) = sample.get("conversation").and_then(|v| v.as_object()) else {
            continue;
        };
        let speaker_a = conv
            .get("speaker_a")
            .and_then(|v| v.as_str())
            .unwrap_or("A");
        let speaker_b = conv
            .get("speaker_b")
            .and_then(|v| v.as_str())
            .unwrap_or("B");

        // True of the file: these two people are each other's counterpart.
        let r = brain.assert_typed(
            ("person", speaker_a),
            "converses_with",
            ("person", speaker_b),
            1.0,
            Visibility::Public,
        )?;
        edges_asserted += 1;
        start_entity.get_or_insert(r.subject.entity_id);

        let indices = session_indices(conv);
        for (pos, &i) in indices.iter().enumerate() {
            let turns = conv
                .get(&format!("session_{i}"))
                .and_then(|v| v.as_array())
                .map(|a| a.as_slice())
                .unwrap_or(&[]);
            if turns.is_empty() {
                continue;
            }

            let stamp = conv
                .get(&format!("session_{i}_date_time"))
                .and_then(|v| v.as_str())
                .and_then(parse_locomo_stamp);
            if stamp.is_none() {
                undated += 1;
            }

            let session_label = format!("{sample_id} session {i}");

            for turn in turns {
                let Some(text) = turn.get("text").and_then(|v| v.as_str()) else {
                    continue;
                };
                let dia = turn.get("dia_id").and_then(|v| v.as_str()).unwrap_or("?");
                let speaker = turn.get("speaker").and_then(|v| v.as_str()).unwrap_or("?");
                let key = format!("locomo/{sample_id}/{dia}");

                brain.remember_with(
                    &key,
                    &format!("{speaker}: {text}"),
                    RememberOpts {
                        visibility: Visibility::Public,
                        created_at: stamp,
                        source: Some("locomo10".into()),
                        ..Default::default()
                    },
                )?;
                memories += 1;
            }

            // True of the file: each speaker took turns in this session.
            for who in [speaker_a, speaker_b] {
                brain.assert_typed(
                    ("person", who),
                    "participated_in",
                    ("session", &session_label),
                    1.0,
                    Visibility::Public,
                )?;
                edges_asserted += 1;
            }

            // True of the file: session ordering.
            if pos + 1 < indices.len() {
                let next_label = format!("{sample_id} session {}", indices[pos + 1]);
                brain.assert_typed(
                    ("session", &session_label),
                    "precedes",
                    ("session", &next_label),
                    1.0,
                    Visibility::Public,
                )?;
                edges_asserted += 1;
            }
        }
        println!("  ingested sample {sample_id}: {} sessions", indices.len());
    }

    println!("\nmemories written : {memories}");
    println!("edges asserted   : {edges_asserted}");
    if undated > 0 {
        println!("sessions whose timestamp did not parse (created_at left default): {undated}");
    }

    // Export a 2-hop neighbourhood at Public scope.
    let start = start_entity.ok_or_else(|| anyhow::anyhow!("no entity to start from"))?;
    let graph_path = brain_dir.join("graph.sqlite");
    let export = export_from(&graph_path, &start, 2)?;

    let json = export.to_json_pretty()?;
    let out_file = out_root.join("graph-export.json");
    std::fs::write(&out_file, &json)?;

    println!("\nexport          : {}", out_file.display());
    println!("nodes           : {}", export.meta.node_count);
    println!(
        "  entities      : {}   documents: {}",
        export.meta.entity_count, export.meta.document_count
    );
    println!("edges           : {}", export.meta.edge_count);
    println!("scope           : {}", export.meta.visibility_scope);
    println!("truncated       : {}", export.meta.truncated);
    println!("filtered out    : {}", export.meta.filtered_out);
    println!("bytes           : {}", json.len());
    Ok(())
}

fn export_from(
    graph_path: &Path,
    start: &EntityId,
    hops: u32,
) -> anyhow::Result<spectral_graph::graph_export::GraphExport> {
    // Read-only: the export must not mutate the brain whose counts it reports.
    let store = GraphStore::open_read_only(graph_path)?;
    let hood = store.neighborhood(start, hops)?;
    Ok(export_neighborhood(&hood, Visibility::Public))
}
