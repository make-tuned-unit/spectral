//! Gap detection: find wings missing summaries, facts, or people.

use regex::Regex;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::sync::OnceLock;

/// Report of detected gaps across all wings.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct GapReport {
    /// Wings with enough memories but no summary/index key.
    pub missing_summaries: Vec<(String, usize)>,
    /// Wings with no fact-hall memories.
    pub no_facts: Vec<(String, usize)>,
    /// Wings with no capitalized names in content.
    pub no_people: Vec<(String, usize)>,
    /// Known projects not represented as wings.
    pub unmapped_projects: Vec<String>,
}

static NAME_RE: OnceLock<Regex> = OnceLock::new();

fn name_regex() -> &'static Regex {
    NAME_RE.get_or_init(|| Regex::new(r"\b([A-Z][a-z]+(?:\s+[A-Z][a-z]+)*)\b").unwrap())
}

/// Common non-name capitalized words to filter out.
const STOPWORDS: &[&str] = &[
    "The",
    "This",
    "That",
    "These",
    "Those",
    "When",
    "Where",
    "What",
    "Which",
    "How",
    "Why",
    "Yes",
    "No",
    "True",
    "False",
    "None",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn extract_names(text: &str) -> HashSet<String> {
    let re = name_regex();
    let stopwords: HashSet<&str> = STOPWORDS.iter().copied().collect();
    re.find_iter(text)
        .map(|m| m.as_str().to_string())
        .filter(|s| s.len() > 2 && !stopwords.contains(s.as_str()))
        .collect()
}

/// Detect gaps in wings with at least `min_memories` memories.
/// If `known_projects` is provided, checks for unmapped projects.
pub fn find_gaps(
    conn: &Connection,
    min_memories: usize,
    known_projects: Option<&[String]>,
) -> anyhow::Result<GapReport> {
    let mut report = GapReport::default();

    // Wings with enough memories
    let mut wing_stmt = conn.prepare(
        "SELECT wing, COUNT(*) as cnt FROM memories \
         WHERE wing IS NOT NULL \
         GROUP BY wing HAVING cnt > ?1",
    )?;
    let wing_counts: Vec<(String, usize)> = wing_stmt
        .query_map(params![min_memories as i64], |row| {
            Ok((row.get(0)?, row.get::<_, i64>(1)? as usize))
        })?
        .filter_map(|r| r.ok())
        .collect();

    for (wing, cnt) in &wing_counts {
        // Check for summary keys
        let summary_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories \
             WHERE wing = ?1 AND (key LIKE '%_project' OR key LIKE '%_summary' OR key LIKE 'index_%')",
            params![wing],
            |row| row.get(0),
        )?;
        if summary_count == 0 {
            report.missing_summaries.push((wing.clone(), *cnt));
        }

        // Check for fact-hall memories
        let fact_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE wing = ?1 AND hall = 'fact'",
            params![wing],
            |row| row.get(0),
        )?;
        if fact_count == 0 {
            report.no_facts.push((wing.clone(), *cnt));
        }

        // Check for people mentioned
        let mut content_stmt = conn.prepare("SELECT content FROM memories WHERE wing = ?1")?;
        let mut all_names = HashSet::new();
        let mut rows = content_stmt.query(params![wing])?;
        while let Some(row) = rows.next()? {
            let content: String = row.get(0)?;
            all_names.extend(extract_names(&content));
        }
        if all_names.is_empty() {
            report.no_people.push((wing.clone(), *cnt));
        }
    }

    // Check unmapped projects
    if let Some(projects) = known_projects {
        let mut known_wings: HashSet<String> = HashSet::new();
        let mut stmt = conn.prepare("SELECT DISTINCT wing FROM memories WHERE wing IS NOT NULL")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let wing: String = row.get(0)?;
            known_wings.insert(wing);
        }

        for proj in projects {
            let normalized = proj.to_lowercase();
            let alt = normalized.replace('-', "_");
            if !known_wings.contains(&normalized) && !known_wings.contains(&alt) {
                report.unmapped_projects.push(normalized);
            }
        }
    }

    Ok(report)
}
