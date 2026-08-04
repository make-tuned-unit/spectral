//! Dry-run (or apply) wing-taxonomy repair on a real brain.
//!
//! The library used to ship example-scenario wing rules as the default, which
//! filed real content into fictional topic areas. This reports exactly what
//! the current rules would change. **Defaults to a dry run**; `--apply` writes.
//!
//! `cargo run -p spectral-bench-real --release --bin wing_repair -- --brain <dir> [--apply]`
//!
//! **Even the dry run opens the brain read-write** (`Brain::open` has no
//! read-only path here, and a missing `ontology.toml` is created). To inspect
//! a brain a live daemon is serving, run against a copy — and take the copy
//! with `sqlite3 <brain>/memory.db ".backup /tmp/brain-inspect/memory.db"`
//! while a daemon is up (a raw `cp` misses outstanding WAL; a live brain was
//! measured 758KB behind its file). `cp -R` is fine only when nothing has
//! the database open. Only `--apply` against the real path should touch a
//! live brain: backup first, daemon stopped.
//!
//! Counts include ephemeral `activity:*` memories, which churn with retention
//! — dry-run totals can move a few rows between runs without any durable
//! change. Split durable vs activity before reading a delta as drift.

use spectral_graph::brain::{Brain, BrainConfig};

/// The wing names the library used to ship as defaults.
const FIXTURE_WINGS: &[&str] = &[
    "alice", "apollo", "acme", "charity", "vega", "travel", "polaris", "infra",
];

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut dir = String::new();
    let mut apply = false;
    // Default: only the retired demo fixtures. Repairing every wing would
    // reclassify the consumer's real taxonomy too — measured at 1,053/1,979 on
    // a live brain. Pass --wings all to override deliberately.
    let mut wings: Vec<String> = FIXTURE_WINGS.iter().map(|s| s.to_string()).collect();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--brain" => dir = args.next().unwrap_or_default(),
            "--apply" => apply = true,
            "--wings" => {
                let v = args.next().unwrap_or_default();
                wings = if v == "all" {
                    Vec::new()
                } else {
                    v.split(',').map(|s| s.trim().to_string()).collect()
                };
            }
            _ => {}
        }
    }
    anyhow::ensure!(
        !dir.is_empty(),
        "usage: wing_repair --brain <dir> [--apply]"
    );
    let dir = std::path::PathBuf::from(shellexpand(&dir));
    let ontology = dir.join("ontology.toml");
    if !ontology.exists() {
        std::fs::write(&ontology, "version = 1\n")?;
    }

    let brain = Brain::open(BrainConfig {
        data_dir: dir.clone(),
        ontology_path: ontology,
        ..Default::default()
    })?;

    let refs: Vec<&str> = wings.iter().map(|s| s.as_str()).collect();
    if refs.is_empty() {
        eprintln!("WARNING: --wings all reclassifies EVERY wing, including your own taxonomy.\n");
    } else {
        println!("restricting to wings: {}\n", refs.join(", "));
    }
    let report = brain.reclassify_wings_in(&refs, apply)?;
    println!(
        "brain: {}\nscanned: {}   would change: {}   applied: {}\n",
        dir.display(),
        report.scanned,
        report.changed(),
        report.applied
    );
    println!("memories leaving each wing:");
    for (wing, n) in report.departures_by_wing() {
        println!("  {wing:<32} {n}");
    }
    if !apply {
        println!("\nDRY RUN — nothing written. Re-run with --apply to repair.");
    }
    Ok(())
}

fn shellexpand(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return format!("{}/{}", home.to_string_lossy(), rest);
        }
    }
    p.to_string()
}
