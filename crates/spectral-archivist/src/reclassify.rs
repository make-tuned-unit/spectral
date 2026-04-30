//! Reclassification suggestions for general-wing memories.

use rusqlite::Connection;
use std::collections::HashSet;

/// A suggestion to reclassify a memory to a different wing or hall.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReclassificationSuggestion {
    pub key: String,
    pub current_wing: Option<String>,
    pub current_hall: Option<String>,
    pub suggested_wing: Option<String>,
    pub suggested_hall: Option<String>,
    pub reason: String,
}

/// Find memories in wing='general' or wing IS NULL and suggest better placement.
pub fn suggest_reclassifications(
    conn: &Connection,
    weak_wings: &[String],
) -> anyhow::Result<Vec<ReclassificationSuggestion>> {
    let mut suggestions = Vec::new();

    // Get known wings (excluding 'general' and weak wings)
    let weak_set: HashSet<&str> = weak_wings.iter().map(|s| s.as_str()).collect();
    let mut wing_stmt = conn.prepare(
        "SELECT DISTINCT wing FROM memories \
         WHERE wing IS NOT NULL AND wing != 'general'",
    )?;
    let known_wings: Vec<String> = wing_stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .filter(|w: &String| !weak_set.contains(w.as_str()))
        .collect();

    // Sort by length descending so longer (more specific) wings match first
    let mut sorted_wings = known_wings;
    sorted_wings.sort_by_key(|w| std::cmp::Reverse(w.len()));

    // Get unclassified memories with optional spectrogram data
    let mut rows_stmt = conn.prepare(
        "SELECT m.key, m.content, m.wing, m.hall, \
                s.decision_polarity, s.entity_density, s.temporal_specificity \
         FROM memories m \
         LEFT JOIN memory_spectrogram s ON m.id = s.memory_id \
         WHERE m.wing = 'general' OR m.wing IS NULL",
    )?;
    let mut rows = rows_stmt.query([])?;

    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        let content: String = row.get(1)?;
        let current_wing: Option<String> = row.get(2)?;
        let current_hall: Option<String> = row.get(3)?;
        let decision_polarity: Option<f64> = row.get(4)?;
        let entity_density: Option<f64> = row.get(5)?;
        let temporal_specificity: Option<f64> = row.get(6)?;

        let content_lower = content.to_lowercase();
        let key_lower = key.to_lowercase();
        let mut suggested_wing = None;
        let mut suggested_hall = current_hall.clone();
        let mut reason = String::new();

        // Try to match against known wings
        for wing in &sorted_wings {
            let exact_key_hit = key_lower.contains(wing.as_str());
            let exact_content_hit = content_lower.contains(wing.as_str());
            let spaced_hit = content_lower.contains(&wing.replace('-', " "));
            let repeated_hit = wing.len() >= 5 && content_lower.matches(wing.as_str()).count() >= 2;

            if exact_key_hit || spaced_hit || repeated_hit || (exact_content_hit && wing.len() >= 6)
            {
                suggested_wing = Some(wing.clone());
                reason = format!("content strongly mentions '{wing}'");
                break;
            }
        }

        // Use spectrogram dimensions for hall suggestion
        if let Some(dp) = decision_polarity {
            if dp.abs() > 0.5 {
                let hall = if dp > 0.0 { "discovery" } else { "advice" };
                suggested_hall = Some(hall.to_string());
                reason = format!("{reason}; high decision_polarity ({dp:.2})")
                    .trim_start_matches("; ")
                    .to_string();
            } else if let (Some(ed), Some(ts)) = (entity_density, temporal_specificity) {
                if ed > 0.5 && ts > 0.5 {
                    suggested_hall = Some("event".to_string());
                    reason = format!("{reason}; high entity_density+temporal_specificity")
                        .trim_start_matches("; ")
                        .to_string();
                } else if ed > 0.5 && ts < 0.3 {
                    suggested_hall = Some("fact".to_string());
                    reason = format!("{reason}; high entity_density, low temporal_specificity")
                        .trim_start_matches("; ")
                        .to_string();
                }
            }
        }

        let hall_changed = suggested_hall.as_deref() != current_hall.as_deref();
        if suggested_wing.is_some() || hall_changed {
            suggestions.push(ReclassificationSuggestion {
                key,
                current_wing: current_wing.clone(),
                current_hall,
                suggested_wing: suggested_wing.or(current_wing),
                suggested_hall,
                reason,
            });
        }
    }

    Ok(suggestions)
}
