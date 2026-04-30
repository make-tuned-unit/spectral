//! Duplicate detection via Jaccard similarity on word sets.

use rusqlite::Connection;
use std::collections::HashSet;

/// A pair of memories with high content overlap within the same wing.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DuplicatePair {
    pub key_a: String,
    pub key_b: String,
    pub overlap: f64,
    pub wing: String,
}

/// Jaccard similarity on whitespace-split word sets.
fn jaccard(text_a: &str, text_b: &str) -> f64 {
    let words_a: HashSet<&str> = text_a.split_whitespace().collect();
    let words_b: HashSet<&str> = text_b.split_whitespace().collect();
    if words_a.is_empty() || words_b.is_empty() {
        return 0.0;
    }
    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();
    intersection as f64 / union as f64
}

/// Find memory pairs with content overlap above `threshold` within each wing.
pub fn find_duplicates(conn: &Connection, threshold: f64) -> anyhow::Result<Vec<DuplicatePair>> {
    let mut wings_stmt =
        conn.prepare("SELECT DISTINCT wing FROM memories WHERE wing IS NOT NULL")?;
    let wings: Vec<String> = wings_stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let mut duplicates = Vec::new();

    for wing in &wings {
        let mut stmt = conn.prepare("SELECT key, content FROM memories WHERE wing = ?1")?;
        let rows: Vec<(String, String)> = stmt
            .query_map([wing], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        let lowered: Vec<String> = rows.iter().map(|(_, c)| c.to_lowercase()).collect();

        for i in 0..rows.len() {
            for j in (i + 1)..rows.len() {
                let overlap = jaccard(&lowered[i], &lowered[j]);
                if overlap > threshold {
                    duplicates.push(DuplicatePair {
                        key_a: rows[i].0.clone(),
                        key_b: rows[j].0.clone(),
                        overlap,
                        wing: wing.clone(),
                    });
                }
            }
        }
    }

    Ok(duplicates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jaccard_identical() {
        assert!((jaccard("hello world", "hello world") - 1.0).abs() < 0.001);
    }

    #[test]
    fn jaccard_disjoint() {
        assert!((jaccard("hello world", "foo bar")).abs() < 0.001);
    }

    #[test]
    fn jaccard_partial() {
        // {hello, world} ∩ {hello, there} = {hello}, union = {hello, world, there}
        assert!((jaccard("hello world", "hello there") - 1.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn jaccard_empty() {
        assert!((jaccard("", "hello")).abs() < 0.001);
    }
}
