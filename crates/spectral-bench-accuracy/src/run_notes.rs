//! Auto-generation of RUN_NOTES.md stubs for bench eval-runs.

use crate::report::{EvalReport, RunStatus};
use anyhow::Result;
use std::path::Path;

/// CLI configuration captured at run time for RUN_NOTES.md generation.
pub struct RunNotesConfig {
    pub git_commit: Option<String>,
    pub git_message: Option<String>,
    pub cli_flags: Vec<(String, String)>,
}

impl RunNotesConfig {
    /// Build a minimal config from report metadata (for backfill).
    pub fn from_report(report: &EvalReport) -> Self {
        let (git_commit, git_message) = get_git_info();
        let cli_flags = vec![
            ("Actor Model".into(), report.actor_name.clone()),
            ("Judge Model".into(), report.judge_name.clone()),
            ("Retrieval Path".into(), report.retrieval_path.clone()),
            ("Spectral Version".into(), report.spectral_version.clone()),
            (
                "Note".into(),
                "Backfilled from report.json; some CLI flags unavailable".into(),
            ),
        ];
        Self {
            git_commit,
            git_message,
            cli_flags,
        }
    }
}

/// Attempt to get git commit hash and first line of commit message.
pub fn get_git_info() -> (Option<String>, Option<String>) {
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });

    let message = std::process::Command::new("git")
        .args(["log", "-1", "--format=%s"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });

    (hash, message)
}

fn format_run_status(status: &RunStatus) -> String {
    match status {
        RunStatus::Completed => "Completed".to_string(),
        RunStatus::HaltedOnErrors { consecutive_errors } => {
            format!("Halted ({consecutive_errors} consecutive errors)")
        }
    }
}

/// Generate RUN_NOTES.md content from a report and config.
pub fn generate_notes(report: &EvalReport, config: &RunNotesConfig) -> String {
    let mut lines = Vec::new();

    lines.push(format!(
        "# Bench Run: {}",
        report.started_at.format("%Y-%m-%dT%H:%M:%SZ")
    ));
    lines.push(String::new());

    // Configuration section
    lines.push("## Configuration".to_string());
    lines.push(String::new());
    lines.push("| Parameter | Value |".to_string());
    lines.push("|---|---|".to_string());

    if let Some(ref hash) = config.git_commit {
        lines.push(format!("| Git Commit | `{hash}` |"));
    }
    if let Some(ref msg) = config.git_message {
        lines.push(format!("| Git Message | {msg} |"));
    }
    for (key, value) in &config.cli_flags {
        lines.push(format!("| {key} | {value} |"));
    }

    lines.push(String::new());

    // Results section
    lines.push("## Results".to_string());
    lines.push(String::new());
    lines.push("| Category | Total | Correct | Accuracy |".to_string());
    lines.push("|---|---|---|---|".to_string());

    let mut cats: Vec<_> = report.per_category.iter().collect();
    cats.sort_by_key(|(k, _)| (*k).clone());
    for (cat_name, stats) in &cats {
        lines.push(format!(
            "| {} | {} | {} | {:.1}% |",
            cat_name,
            stats.total,
            stats.correct,
            stats.accuracy * 100.0
        ));
    }

    lines.push(format!(
        "| **Overall** | **{}** | **{}** | **{:.1}%** |",
        report.total_questions,
        report.correct,
        report.overall_accuracy * 100.0
    ));
    lines.push(String::new());

    lines.push(format!("- **Duration**: {}s", report.duration_seconds));
    lines.push(format!("- **Total Questions**: {}", report.total_questions));
    lines.push(format!("- **Correct**: {}", report.correct));
    lines.push(format!(
        "- **Failed**: {}",
        report.total_questions - report.correct
    ));
    lines.push(format!(
        "- **Run Status**: {}",
        format_run_status(&report.run_status)
    ));
    lines.push(String::new());

    // Manual sections (templated, empty)
    lines.push("## Findings".to_string());
    lines.push(String::new());
    lines.push("_(Add observations, surprising results, failure patterns, etc.)_".to_string());
    lines.push(String::new());

    lines.push("## Comparison".to_string());
    lines.push(String::new());
    lines.push("_(Compare against baseline runs, note improvements or regressions.)_".to_string());
    lines.push(String::new());

    lines.push("## Next Steps".to_string());
    lines.push(String::new());
    lines.push("_(Action items, follow-up experiments, issues to file.)_".to_string());
    lines.push(String::new());

    lines.join("\n")
}

/// Warn on stderr if work_dir has report.json but no RUN_NOTES.md.
pub fn warn_missing_notes(work_dir: &Path) {
    if !work_dir.exists() {
        return;
    }
    let report_path = work_dir.join("report.json");
    let notes_path = work_dir.join("RUN_NOTES.md");
    if report_path.exists() && !notes_path.exists() {
        eprintln!(
            "\u{26a0} Existing eval-run at {} has no RUN_NOTES.md. \
             Consider documenting the prior run before overwriting.",
            work_dir.display()
        );
    }
}

/// Write RUN_NOTES.md to work_dir if it doesn't already exist.
/// Returns Ok(true) if written, Ok(false) if already exists.
pub fn write_notes_if_missing(work_dir: &Path, content: &str) -> Result<bool> {
    let notes_path = work_dir.join("RUN_NOTES.md");
    if notes_path.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(work_dir)?;
    std::fs::write(&notes_path, content)?;
    Ok(true)
}

/// Write RUN_NOTES.md to work_dir, optionally forcing overwrite.
/// Returns Ok(true) if written, Ok(false) if exists and force is false.
pub fn write_notes(work_dir: &Path, content: &str, force: bool) -> Result<bool> {
    let notes_path = work_dir.join("RUN_NOTES.md");
    if notes_path.exists() && !force {
        return Ok(false);
    }
    std::fs::create_dir_all(work_dir)?;
    std::fs::write(&notes_path, content)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Category;
    use crate::report::EvalReport;

    fn mock_report() -> EvalReport {
        let mut report = EvalReport::new("test-actor", "test-judge");
        report.record(
            "q1",
            Category::MultiSession,
            true,
            "Q?",
            "A",
            "A",
            None,
            5,
            vec!["k1".into()],
            100,
            None,
            None,
        );
        report.record(
            "q2",
            Category::MultiSession,
            false,
            "Q2?",
            "B",
            "C",
            Some("wrong".into()),
            3,
            vec![],
            200,
            None,
            None,
        );
        report.finalize();
        report
    }

    #[test]
    fn notes_created_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let content = "# Test Notes\n";
        let result = write_notes_if_missing(dir.path(), content).unwrap();
        assert!(result);
        let written = std::fs::read_to_string(dir.path().join("RUN_NOTES.md")).unwrap();
        assert_eq!(written, content);
    }

    #[test]
    fn notes_not_overwritten_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let notes_path = dir.path().join("RUN_NOTES.md");
        std::fs::write(&notes_path, "existing content").unwrap();

        let result = write_notes_if_missing(dir.path(), "new content").unwrap();
        assert!(!result);
        let content = std::fs::read_to_string(&notes_path).unwrap();
        assert_eq!(content, "existing content");
    }

    #[test]
    fn warn_fires_for_dir_without_notes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("report.json"), "{}").unwrap();
        // Verify the condition logic is correct
        assert!(dir.path().join("report.json").exists());
        assert!(!dir.path().join("RUN_NOTES.md").exists());
        warn_missing_notes(dir.path()); // should not panic
    }

    #[test]
    fn warn_silent_when_notes_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("report.json"), "{}").unwrap();
        std::fs::write(dir.path().join("RUN_NOTES.md"), "notes").unwrap();
        warn_missing_notes(dir.path());
    }

    #[test]
    fn warn_silent_when_no_report() {
        let dir = tempfile::tempdir().unwrap();
        warn_missing_notes(dir.path());
    }

    #[test]
    fn warn_silent_when_dir_missing() {
        warn_missing_notes(Path::new("/nonexistent/path/that/does/not/exist"));
    }

    #[test]
    fn generate_notes_includes_all_sections() {
        let report = mock_report();
        let config = RunNotesConfig {
            git_commit: Some("abc123def".into()),
            git_message: Some("test commit message".into()),
            cli_flags: vec![("Dataset".into(), "test.json".into())],
        };
        let notes = generate_notes(&report, &config);

        assert!(notes.contains("# Bench Run:"));
        assert!(notes.contains("## Configuration"));
        assert!(notes.contains("`abc123def`"));
        assert!(notes.contains("test commit message"));
        assert!(notes.contains("| Dataset | test.json |"));
        assert!(notes.contains("## Results"));
        assert!(notes.contains("multi-session"));
        assert!(notes.contains("50.0%")); // 1/2 correct
        assert!(notes.contains("## Findings"));
        assert!(notes.contains("## Comparison"));
        assert!(notes.contains("## Next Steps"));
        assert!(notes.contains("Completed"));
    }

    #[test]
    fn generate_notes_handles_empty_report() {
        let mut report = EvalReport::new("actor", "judge");
        report.finalize();
        let config = RunNotesConfig {
            git_commit: None,
            git_message: None,
            cli_flags: vec![],
        };
        let notes = generate_notes(&report, &config);
        assert!(notes.contains("**Overall** | **0** | **0** | **0.0%**"));
        assert!(!notes.contains("Git Commit"));
    }

    #[test]
    fn force_overwrites_existing_notes() {
        let dir = tempfile::tempdir().unwrap();
        let notes_path = dir.path().join("RUN_NOTES.md");
        std::fs::write(&notes_path, "old content").unwrap();

        let result = write_notes(dir.path(), "new content", true).unwrap();
        assert!(result);
        let content = std::fs::read_to_string(&notes_path).unwrap();
        assert_eq!(content, "new content");
    }

    #[test]
    fn no_force_preserves_existing_notes() {
        let dir = tempfile::tempdir().unwrap();
        let notes_path = dir.path().join("RUN_NOTES.md");
        std::fs::write(&notes_path, "old content").unwrap();

        let result = write_notes(dir.path(), "new content", false).unwrap();
        assert!(!result);
        let content = std::fs::read_to_string(&notes_path).unwrap();
        assert_eq!(content, "old content");
    }

    #[test]
    fn backfill_from_report_json() {
        let dir = tempfile::tempdir().unwrap();
        let report = mock_report();

        // Save report to directory
        let report_path = dir.path().join("report.json");
        crate::report::save_report(&report, &report_path).unwrap();

        // Load and generate notes (simulating backfill)
        let loaded = crate::report::load_report(&report_path).unwrap();
        let config = RunNotesConfig::from_report(&loaded);
        let notes = generate_notes(&loaded, &config);

        let written = write_notes_if_missing(dir.path(), &notes).unwrap();
        assert!(written);

        let content = std::fs::read_to_string(dir.path().join("RUN_NOTES.md")).unwrap();
        assert!(content.contains("test-actor"));
        assert!(content.contains("test-judge"));
        assert!(content.contains("Backfilled"));
    }
}
