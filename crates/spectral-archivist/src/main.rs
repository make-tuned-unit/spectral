use clap::{Parser, Subcommand};
use spectral_archivist::Archivist;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "spectral-archivist",
    about = "Memory-quality maintenance for Spectral brains"
)]
struct Cli {
    /// Path to the brain directory (or memory.db file directly)
    #[arg(long)]
    brain: PathBuf,

    /// Output format: text or json
    #[arg(long, default_value = "text")]
    format: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Dry-run all passes and print a report
    Report,
    /// Apply signal score decay and boost
    Decay,
    /// Find duplicate memory pairs
    Duplicates,
    /// Detect coverage gaps across wings
    Gaps,
    /// Suggest reclassifications for general-wing memories
    Reclassify,
    /// Find consolidation candidates
    Candidates,
}

fn resolve_brain_path(input: &std::path::Path) -> anyhow::Result<PathBuf> {
    if input.is_dir() {
        let db_path = input.join("memory.db");
        if !db_path.exists() {
            anyhow::bail!(
                "Brain directory does not contain memory.db: {}",
                input.display()
            );
        }
        Ok(db_path)
    } else if input.is_file() {
        Ok(input.to_path_buf())
    } else {
        anyhow::bail!(
            "Brain path is neither a directory nor a file: {}",
            input.display()
        );
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let resolved = resolve_brain_path(&cli.brain)?;
    let archivist = Archivist::open(&resolved)?;
    let json = cli.format == "json";

    match cli.command {
        Command::Report => {
            let report = archivist.report()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_report(&report);
            }
        }
        Command::Decay => {
            let stats = archivist.apply_decay()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                println!("Signal decay applied:");
                println!("  Decayed: {} memories (-0.05)", stats.decayed);
                println!("  Boosted: {} memories (+0.02)", stats.boosted);
            }
        }
        Command::Duplicates => {
            let dupes = archivist.find_duplicates()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&dupes)?);
            } else {
                if dupes.is_empty() {
                    println!("No duplicates found (>60% overlap)");
                } else {
                    for d in &dupes {
                        println!(
                            "[{}] {} <> {} — {:.0}% overlap",
                            d.wing,
                            d.key_a,
                            d.key_b,
                            d.overlap * 100.0
                        );
                    }
                }
                println!("\nTotal: {} pairs", dupes.len());
            }
        }
        Command::Gaps => {
            let gaps = archivist.find_gaps()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&gaps)?);
            } else {
                print_gaps(&gaps);
            }
        }
        Command::Reclassify => {
            let suggestions = archivist.suggest_reclassifications()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&suggestions)?);
            } else {
                if suggestions.is_empty() {
                    println!("No reclassifications suggested");
                } else {
                    for s in &suggestions {
                        println!(
                            "{}: {:?}/{:?} -> {:?}/{:?}",
                            s.key,
                            s.current_wing,
                            s.current_hall,
                            s.suggested_wing,
                            s.suggested_hall
                        );
                        if !s.reason.is_empty() {
                            println!("  reason: {}", s.reason);
                        }
                    }
                }
                println!("\nTotal: {} suggestions", suggestions.len());
            }
        }
        Command::Candidates => {
            let cands = archivist.find_consolidation_candidates()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&cands)?);
            } else {
                if cands.is_empty() {
                    println!("No consolidation candidates found");
                } else {
                    for c in &cands {
                        println!(
                            "[{}/{}] {} <> {} — {:.0}% overlap",
                            c.wing,
                            c.hall,
                            c.key_a,
                            c.key_b,
                            c.overlap * 100.0
                        );
                    }
                }
                println!("\nTotal: {} candidates", cands.len());
            }
        }
    }

    Ok(())
}

fn print_report(report: &spectral_archivist::ArchivistReport) {
    println!("=== SPECTRAL ARCHIVIST REPORT ===");
    println!("Timestamp: {}", report.timestamp);
    println!("Total memories: {}", report.memory_count);

    println!("\n-- Duplicates --");
    if report.duplicates.is_empty() {
        println!("  No duplicates found");
    } else {
        for d in &report.duplicates {
            println!(
                "  [{}] {} <> {} — {:.0}%",
                d.wing,
                d.key_a,
                d.key_b,
                d.overlap * 100.0
            );
        }
    }

    println!("\n-- Gaps --");
    print_gaps(&report.gaps);

    println!("\n-- Reclassifications --");
    if report.reclassifications.is_empty() {
        println!("  No reclassifications suggested");
    } else {
        for s in &report.reclassifications {
            println!(
                "  {}: {:?}/{:?} -> {:?}/{:?}",
                s.key, s.current_wing, s.current_hall, s.suggested_wing, s.suggested_hall
            );
        }
    }

    println!("\n-- Consolidation Candidates --");
    if report.consolidation_candidates.is_empty() {
        println!("  No candidates found");
    } else {
        for c in &report.consolidation_candidates {
            println!(
                "  [{}/{}] {} <> {} — {:.0}%",
                c.wing,
                c.hall,
                c.key_a,
                c.key_b,
                c.overlap * 100.0
            );
        }
    }
}

fn print_gaps(gaps: &spectral_archivist::GapReport) {
    if !gaps.missing_summaries.is_empty() {
        println!("  Missing summaries:");
        for (wing, cnt) in &gaps.missing_summaries {
            println!("    {wing} ({cnt} memories)");
        }
    }
    if !gaps.no_facts.is_empty() {
        println!("  No fact memories:");
        for (wing, cnt) in &gaps.no_facts {
            println!("    {wing} ({cnt} memories)");
        }
    }
    if !gaps.no_people.is_empty() {
        println!("  No people mentioned:");
        for (wing, cnt) in &gaps.no_people {
            println!("    {wing} ({cnt} memories)");
        }
    }
    if !gaps.unmapped_projects.is_empty() {
        println!("  Unmapped projects:");
        for proj in &gaps.unmapped_projects {
            println!("    {proj}");
        }
    }
    if gaps.missing_summaries.is_empty()
        && gaps.no_facts.is_empty()
        && gaps.no_people.is_empty()
        && gaps.unmapped_projects.is_empty()
    {
        println!("  No gaps detected");
    }
}
