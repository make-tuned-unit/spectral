//! Consolidation candidate detection.
//!
//! Finds memory pairs within the same wing+hall that have moderate
//! content overlap (Jaccard between `overlap_min` and `overlap_max`),
//! indicating they cover related topics and could be merged.

use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};

/// A pair of memories eligible for LLM-mediated consolidation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConsolidationCandidate {
    pub wing: String,
    pub hall: String,
    pub key_a: String,
    pub key_b: String,
    pub content_a: String,
    pub content_b: String,
    pub overlap: f64,
}

/// Jaccard similarity on whitespace-split word sets (lowercase).
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

/// Find consolidation candidates: pairs with overlap in \[overlap_min, overlap_max\].
pub fn find_consolidation_candidates(
    conn: &Connection,
    overlap_min: f64,
    overlap_max: f64,
    skip_prefixes: &[String],
    skip_contains: &[String],
) -> anyhow::Result<Vec<ConsolidationCandidate>> {
    let mut candidates = Vec::new();

    // Wings with >= 5 memories
    let mut wing_stmt = conn.prepare(
        "SELECT wing, COUNT(*) as cnt FROM memories \
         WHERE wing IS NOT NULL \
         GROUP BY wing HAVING cnt >= 5 ORDER BY cnt DESC",
    )?;
    let wings: Vec<String> = wing_stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    for wing in &wings {
        let mut stmt = conn.prepare(
            "SELECT key, content, hall FROM memories \
             WHERE wing = ?1 ORDER BY hall, key",
        )?;
        let rows: Vec<(String, String, Option<String>)> = stmt
            .query_map(params![wing], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Group by hall
        let mut by_hall: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for (key, content, hall) in &rows {
            let h = hall.as_deref().unwrap_or("").to_string();
            by_hall
                .entry(h.clone())
                .or_default()
                .push((key.clone(), content.clone()));
        }

        for (hall, memories) in &by_hall {
            if memories.len() < 2 {
                continue;
            }

            let mut seen: HashSet<(String, String)> = HashSet::new();

            for i in 0..memories.len() {
                for j in (i + 1)..memories.len() {
                    let (key_a, content_a) = &memories[i];
                    let (key_b, content_b) = &memories[j];
                    let low_a = key_a.to_lowercase();
                    let low_b = key_b.to_lowercase();

                    // Skip system keys
                    if skip_prefixes.iter().any(|p| low_a.starts_with(p.as_str()))
                        || skip_prefixes.iter().any(|p| low_b.starts_with(p.as_str()))
                    {
                        continue;
                    }
                    if skip_contains.iter().any(|s| low_a.contains(s.as_str()))
                        || skip_contains.iter().any(|s| low_b.contains(s.as_str()))
                    {
                        continue;
                    }

                    let la = content_a.to_lowercase();
                    let lb = content_b.to_lowercase();
                    let overlap = jaccard(&la, &lb);
                    if overlap >= overlap_min && overlap <= overlap_max {
                        let pair = if key_a < key_b {
                            (key_a.clone(), key_b.clone())
                        } else {
                            (key_b.clone(), key_a.clone())
                        };
                        if seen.contains(&pair) {
                            continue;
                        }
                        seen.insert(pair);
                        candidates.push(ConsolidationCandidate {
                            wing: wing.clone(),
                            hall: hall.clone(),
                            key_a: key_a.clone(),
                            key_b: key_b.clone(),
                            content_a: content_a.clone(),
                            content_b: content_b.clone(),
                            overlap,
                        });
                    }
                }
            }
        }
    }

    candidates.sort_by(|a, b| {
        b.overlap
            .partial_cmp(&a.overlap)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(candidates)
}
