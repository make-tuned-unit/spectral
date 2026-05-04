//! Activity episode types shared across Spectral crates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A coalesced unit of user activity.
///
/// Consumers using capture systems (Accessibility APIs, workspace monitors)
/// should coalesce raw events into episodes before passing to Spectral.
/// A typical compression ratio is ~3:1 (raw events to episodes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEpisode {
    /// Stable identifier. Re-ingesting the same episode by id is a no-op (UPSERT).
    pub id: String,
    /// Episode start time.
    pub started_at: DateTime<Utc>,
    /// Episode end time. May equal started_at for instantaneous events.
    pub ended_at: DateTime<Utc>,
    /// Application identifier (e.g., macOS bundle ID).
    pub bundle_id: String,
    /// Human-readable app name.
    pub app_name: String,
    /// Window title or session label at episode start.
    pub window_title: Option<String>,
    /// URL if applicable (browsers, web apps).
    pub url: Option<String>,
    /// Excerpt of visible text. Consumers SHOULD pre-redact before passing.
    pub excerpt: Option<String>,
    /// Source label: "accessibility", "workspace", "manual", etc.
    pub source: String,
    /// Number of raw events that coalesced into this episode.
    pub source_event_count: u32,
    /// Optional consumer metadata (e.g., engagement score).
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Wing this episode belongs to. Set by consumers who classify
    /// activity into Spectral wings (e.g., "permagent", "spectral").
    /// None means unclassified.
    #[serde(default)]
    pub wing: Option<String>,
}

impl ActivityEpisode {
    /// Duration in seconds.
    pub fn duration_secs(&self) -> f64 {
        (self.ended_at - self.started_at).num_milliseconds() as f64 / 1000.0
    }

    /// Synthesize a content string for memory storage.
    pub fn to_content(&self) -> String {
        let mut parts = Vec::new();
        parts.push(format!("[{}] {}", self.app_name, self.bundle_id));
        if let Some(ref title) = self.window_title {
            if !title.is_empty() {
                parts.push(title.clone());
            }
        }
        if let Some(ref url) = self.url {
            if !url.is_empty() {
                parts.push(url.clone());
            }
        }
        if let Some(ref excerpt) = self.excerpt {
            if !excerpt.is_empty() {
                parts.push(excerpt.clone());
            }
        }
        parts.join(" | ")
    }

    /// Compute a signal score from episode metadata.
    pub fn compute_signal_score(&self) -> f64 {
        let base = 0.3;
        let duration_bonus = (self.duration_secs() / 600.0).min(1.0) * 0.3;
        let event_bonus = (self.source_event_count as f64 / 20.0).min(1.0) * 0.2;
        let engagement_bonus = self
            .metadata
            .get("engagement_score")
            .and_then(|v| v.as_f64())
            .map(|s| s.clamp(0.0, 1.0) * 0.2)
            .unwrap_or(0.0);
        (base + duration_bonus + event_bonus + engagement_bonus).min(1.0)
    }
}
