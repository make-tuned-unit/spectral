//! Spectrogram audit binary: produces a falsifiability report
//! on whether memory data has the peak structure the architecture assumes.

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use clap::Parser;
use spectral_graph::brain::{AuditReport, Brain, BrainConfig, EntityPolicy};
use spectral_spectrogram::matching::{self, MatchTolerances};
use spectral_spectrogram::SpectralFingerprint;

#[derive(Parser)]
#[command(name = "spectral-audit", about = "Spectrogram falsifiability audit")]
struct Args {
    /// Path to brain database (memory.db)
    #[arg(long)]
    brain: PathBuf,

    /// Output path for the markdown report
    #[arg(long)]
    output: PathBuf,

    /// Number of memories to sample per wing
    #[arg(long, default_value = "30")]
    samples_per_wing: usize,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Open brain from the provided path
    // --brain can point to the .db file or the brain directory
    let (brain_dir, memory_db_path) = if args.brain.extension().is_some_and(|e| e == "db") {
        (
            args.brain.parent().unwrap_or(&args.brain).to_path_buf(),
            args.brain.clone(),
        )
    } else {
        (args.brain.clone(), args.brain.join("memory.db"))
    };

    let ontology_path = brain_dir.join("ontology.toml");
    if !ontology_path.exists() {
        anyhow::bail!(
            "ontology.toml not found in {}. Point --brain at the brain directory containing both memory.db and ontology.toml.",
            brain_dir.display()
        );
    }

    let config = BrainConfig {
        data_dir: brain_dir,
        ontology_path,
        memory_db_path: Some(memory_db_path),
        llm_client: None,
        wing_rules: None,
        hall_rules: None,
        device_id: None,
        enable_spectrogram: false,
        entity_policy: EntityPolicy::Strict,
        sqlite_mmap_size: None,
        activity_wing: "activity".into(),
        redaction_policy: None,
        tact_config: None,
    };

    let brain = Brain::open(config)?;

    // Discover wings and sample memories
    let all_memories = brain.list_all_memories(10000)?;
    eprintln!("Total memories in brain: {}", all_memories.len());

    // Target wings for high-signal sampling
    let target_wings = ["jesse", "permagent", "henry-infra", "getladle"];

    let mut sampled: Vec<AuditReport> = Vec::new();
    let mut wing_samples: HashMap<String, Vec<AuditReport>> = HashMap::new();

    for wing_name in &target_wings {
        let wing_mems: Vec<_> = all_memories
            .iter()
            .filter(|m| m.wing.as_deref() == Some(wing_name))
            .take(args.samples_per_wing)
            .collect();

        eprintln!(
            "  Wing '{}': {} memories available, sampling {}",
            wing_name,
            all_memories
                .iter()
                .filter(|m| m.wing.as_deref() == Some(wing_name))
                .count(),
            wing_mems.len()
        );

        let mut wing_reports = Vec::new();
        for mem in &wing_mems {
            match brain.audit_spectrogram(&mem.id) {
                Ok(report) => wing_reports.push(report),
                Err(e) => eprintln!("    WARN: failed to audit {}: {e}", mem.id),
            }
        }
        sampled.extend(wing_reports.clone());
        wing_samples.insert(wing_name.to_string(), wing_reports);
    }

    // Low-signal control sample: memories with signal_score < 0.6
    let low_signal: Vec<_> = all_memories
        .iter()
        .filter(|m| m.signal_score < 0.6)
        .filter(|m| m.compaction_tier.is_none()) // deliberate only
        .take(args.samples_per_wing)
        .collect();

    eprintln!(
        "  Low-signal control: {} available, sampling {}",
        all_memories
            .iter()
            .filter(|m| m.signal_score < 0.6 && m.compaction_tier.is_none())
            .count(),
        low_signal.len()
    );

    let mut low_signal_reports = Vec::new();
    for mem in &low_signal {
        match brain.audit_spectrogram(&mem.id) {
            Ok(report) => low_signal_reports.push(report),
            Err(e) => eprintln!("    WARN: failed to audit {}: {e}", mem.id),
        }
    }
    sampled.extend(low_signal_reports.clone());
    wing_samples.insert("_low_signal_control".into(), low_signal_reports);

    // Ambient events (compaction_tier is Some)
    let ambient: Vec<_> = all_memories
        .iter()
        .filter(|m| m.compaction_tier.is_some())
        .collect();

    let mut ambient_reports = Vec::new();
    for mem in &ambient {
        match brain.audit_spectrogram(&mem.id) {
            Ok(report) => ambient_reports.push(report),
            Err(e) => eprintln!("    WARN: failed to audit ambient {}: {e}", mem.id),
        }
    }

    eprintln!(
        "\nTotal sampled: {} (target: {})",
        sampled.len(),
        target_wings.len() * args.samples_per_wing + args.samples_per_wing
    );

    // Generate report
    let report = generate_report(
        &sampled,
        &wing_samples,
        &ambient_reports,
        &all_memories,
        &brain,
    );

    std::fs::write(&args.output, &report)?;
    eprintln!("\nReport written to: {}", args.output.display());

    Ok(())
}

fn generate_report(
    sampled: &[AuditReport],
    wing_samples: &HashMap<String, Vec<AuditReport>>,
    ambient_reports: &[AuditReport],
    all_memories: &[spectral_ingest::Memory],
    _brain: &Brain,
) -> String {
    let mut out = String::new();

    writeln!(out, "# Spectrogram Audit Report").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "**Date:** {}  ",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    )
    .unwrap();
    writeln!(out, "**Total memories in brain:** {}  ", all_memories.len()).unwrap();
    writeln!(out, "**Memories audited:** {}  ", sampled.len()).unwrap();
    writeln!(out, "**Ambient events:** {}  ", ambient_reports.len()).unwrap();
    writeln!(out).unwrap();

    // Section 1: Action type distribution per wing
    writeln!(out, "## 1. Action Type Distribution (per wing)").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "| Wing | Decision | Discovery | Task | Observation | Advice | Reflection | Total |"
    )
    .unwrap();
    writeln!(
        out,
        "|------|----------|-----------|------|-------------|--------|------------|-------|"
    )
    .unwrap();

    let mut sorted_wings: Vec<_> = wing_samples.keys().collect();
    sorted_wings.sort();
    for wing in &sorted_wings {
        let reports = &wing_samples[*wing];
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for r in reports {
            *counts
                .entry(r.fingerprint.action_type.as_str())
                .or_default() += 1;
        }
        writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            wing,
            counts.get("decision").unwrap_or(&0),
            counts.get("discovery").unwrap_or(&0),
            counts.get("task").unwrap_or(&0),
            counts.get("observation").unwrap_or(&0),
            counts.get("advice").unwrap_or(&0),
            counts.get("reflection").unwrap_or(&0),
            reports.len(),
        )
        .unwrap();
    }
    writeln!(out).unwrap();

    // Section 2: Signal score distribution
    writeln!(out, "## 2. Signal Score Distribution").unwrap();
    writeln!(out).unwrap();

    let mut buckets = [0usize; 10];
    for r in sampled {
        let idx = ((r.signal_score * 10.0).floor() as usize).min(9);
        buckets[idx] += 1;
    }
    writeln!(out, "| Range | Count | Bar |").unwrap();
    writeln!(out, "|-------|-------|-----|").unwrap();
    let max_count = *buckets.iter().max().unwrap_or(&1).max(&1);
    for (i, count) in buckets.iter().enumerate() {
        let bar_len = (*count * 30) / max_count;
        let bar: String = "█".repeat(bar_len);
        writeln!(
            out,
            "| {:.1}–{:.1} | {} | {} |",
            i as f64 / 10.0,
            (i + 1) as f64 / 10.0,
            count,
            bar
        )
        .unwrap();
    }
    writeln!(out).unwrap();

    // Section 3: Action type accuracy (20 sample memories)
    writeln!(out, "## 3. Action Type Accuracy (manual inspection)").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "| # | Action Type | Rationale | Content (first 120 chars) |"
    )
    .unwrap();
    writeln!(
        out,
        "|---|-------------|-----------|---------------------------|"
    )
    .unwrap();

    for (i, r) in sampled.iter().take(20).enumerate() {
        let excerpt: String = r.content_excerpt.chars().take(120).collect();
        let excerpt = excerpt.replace('|', "\\|").replace('\n', " ");
        writeln!(
            out,
            "| {} | {} | {} | {} |",
            i + 1,
            r.fingerprint.action_type,
            r.introspection.action_type_rationale.replace('|', "\\|"),
            excerpt,
        )
        .unwrap();
    }
    writeln!(out).unwrap();

    // Section 4: Peak dimension co-occurrence
    writeln!(out, "## 4. Peak Dimension Co-occurrence").unwrap();
    writeln!(out).unwrap();

    let dim_names = [
        "entity_density",
        "decision_polarity",
        "causal_depth",
        "emotional_valence",
        "temporal_specificity",
        "novelty",
    ];
    let mut cooccurrence = [[0usize; 6]; 6];
    for r in sampled {
        let peak_set: Vec<usize> = r
            .fingerprint
            .peak_dimensions
            .iter()
            .filter_map(|d| dim_names.iter().position(|n| n == d))
            .collect();
        for &i in &peak_set {
            for &j in &peak_set {
                cooccurrence[i][j] += 1;
            }
        }
    }

    write!(out, "| |").unwrap();
    for name in &dim_names {
        let short: String = name.chars().take(8).collect();
        write!(out, " {} |", short).unwrap();
    }
    writeln!(out).unwrap();
    write!(out, "|---|").unwrap();
    for _ in &dim_names {
        write!(out, "---|").unwrap();
    }
    writeln!(out).unwrap();

    for (i, name) in dim_names.iter().enumerate() {
        let short: String = name.chars().take(8).collect();
        write!(out, "| {} |", short).unwrap();
        for val in &cooccurrence[i] {
            write!(out, " {} |", val).unwrap();
        }
        writeln!(out).unwrap();
    }
    writeln!(out).unwrap();

    // Section 5: Cross-wing resonance test
    writeln!(out, "## 5. Cross-Wing Resonance Test").unwrap();
    writeln!(out).unwrap();

    // Collect fingerprints from all sampled memories
    let all_fps: Vec<SpectralFingerprint> = sampled.iter().map(|r| r.fingerprint.clone()).collect();

    // Pick 10 high-signal memories
    let mut high_signal: Vec<&AuditReport> =
        sampled.iter().filter(|r| r.signal_score >= 0.7).collect();
    high_signal.truncate(10);

    for r in &high_signal {
        let matches =
            matching::find_resonant(&r.fingerprint, &all_fps, 3, &MatchTolerances::default());

        writeln!(
            out,
            "### Query: `{}` (wing: {}, action: {})",
            r.memory_key,
            r.wing.as_deref().unwrap_or("?"),
            r.fingerprint.action_type
        )
        .unwrap();
        writeln!(
            out,
            "> {}",
            r.content_excerpt
                .chars()
                .take(200)
                .collect::<String>()
                .replace('\n', " ")
        )
        .unwrap();
        writeln!(out).unwrap();

        if matches.is_empty() {
            writeln!(out, "**No resonant matches found.**").unwrap();
        } else {
            for m in &matches {
                let matched_report = sampled.iter().find(|s| s.memory_id == m.memory_id);
                if let Some(mr) = matched_report {
                    writeln!(
                        out,
                        "- **{:.2}** resonance → `{}` (wing: {}) — {}",
                        m.resonance_score,
                        mr.memory_key,
                        mr.wing.as_deref().unwrap_or("?"),
                        mr.content_excerpt
                            .chars()
                            .take(100)
                            .collect::<String>()
                            .replace('\n', " ")
                    )
                    .unwrap();
                }
            }
        }
        writeln!(out).unwrap();
    }

    // Section 6: Peak-pair candidate count estimate
    writeln!(out, "## 6. Peak-Pair Candidate Count Estimate").unwrap();
    writeln!(out).unwrap();

    // Count memories with created_at timestamps for temporal window estimation
    let with_timestamps: Vec<_> = sampled
        .iter()
        .filter_map(|r| r.created_at.map(|ts| (r, ts)))
        .collect();

    let windows = [
        ("1 hour", 60i64),
        ("1 day", 1440),
        ("1 week", 10080),
        ("1 month", 43200),
    ];

    writeln!(
        out,
        "| Window | Avg pairs/memory | Total pairs (est) | Viable? |"
    )
    .unwrap();
    writeln!(
        out,
        "|--------|------------------|-------------------|---------|"
    )
    .unwrap();

    for (label, minutes) in &windows {
        let mut total_pairs = 0usize;
        let n = with_timestamps.len();
        for (i, (_, ts_i)) in with_timestamps.iter().enumerate() {
            let pairs_for_i = with_timestamps
                .iter()
                .skip(i + 1)
                .filter(|(_, ts_j)| {
                    let diff = (*ts_j - *ts_i).num_minutes().abs();
                    diff <= *minutes
                })
                .count();
            total_pairs += pairs_for_i;
        }
        let avg = if n > 0 {
            total_pairs as f64 / n as f64
        } else {
            0.0
        };
        let viable = if avg < 1.0 {
            "too few"
        } else if avg > 500.0 {
            "too many"
        } else {
            "viable"
        };
        writeln!(
            out,
            "| {} | {:.1} | {} | {} |",
            label, avg, total_pairs, viable
        )
        .unwrap();
    }
    writeln!(out).unwrap();

    // Section 7: Ambient events
    writeln!(out, "## 7. Ambient Events").unwrap();
    writeln!(out).unwrap();

    if ambient_reports.is_empty() {
        writeln!(
            out,
            "No ambient events found (compaction_tier is NULL for all memories)."
        )
        .unwrap();
    } else {
        for r in ambient_reports {
            writeln!(out, "### `{}`", r.memory_key).unwrap();
            writeln!(out, "- **Wing:** {}", r.wing.as_deref().unwrap_or("?")).unwrap();
            writeln!(
                out,
                "- **Action type:** {} ({})",
                r.fingerprint.action_type, r.introspection.action_type_rationale
            )
            .unwrap();
            writeln!(out, "- **Signal score:** {:.3}", r.signal_score).unwrap();
            writeln!(
                out,
                "- **Peak dimensions:** {}",
                r.fingerprint.peak_dimensions.join(", ")
            )
            .unwrap();
            writeln!(
                out,
                "- **Content:** {}",
                r.content_excerpt.replace('\n', " ")
            )
            .unwrap();
            writeln!(out).unwrap();
        }
    }
    writeln!(out).unwrap();

    // Section 8: placeholder for human assessment
    writeln!(out, "## 8. Honest Assessment").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "*[To be written by human review after reading sections 1-7.]*"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "Questions to answer:").unwrap();
    writeln!(
        out,
        "- Does Henry's brain show peak structure that the existing analyzer detects?"
    )
    .unwrap();
    writeln!(
        out,
        "- Where does the analyzer fail (false positives, false negatives)?"
    )
    .unwrap();
    writeln!(
        out,
        "- What would peak detection need to change to perform better?"
    )
    .unwrap();
    writeln!(
        out,
        "- Should we proceed to PR 2 as designed, with revisions, or halt?"
    )
    .unwrap();

    out
}
