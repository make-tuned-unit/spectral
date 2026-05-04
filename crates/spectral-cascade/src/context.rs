//! Recognition context: ambient state carried into cascade recognition.

use chrono::{DateTime, Utc};
use spectral_ingest::activity::ActivityEpisode;

/// Ambient state carried into cascade recognition.
///
/// Empty context ([`RecognitionContext::empty()`]) means no ambient signal
/// is available — used by ad-hoc queries, the bench harness, and any
/// caller without continuous-awareness data.
///
/// Populated context carries recent activity, current time, optional
/// wing focus, and optional persona. Layers use this to condition
/// recognition decisions: AAAK filters by current activity wing,
/// EpisodeLayer scores by recency-to-now, etc. (Behavior change ships
/// in subsequent PRs; this PR adds the primitive only.)
#[derive(Debug, Clone)]
pub struct RecognitionContext {
    /// Recent activity episodes (typically last N minutes from the
    /// activity wing). Provides the "what is the user doing now"
    /// signal. Empty Vec means no recent activity available.
    pub recent_activity: Vec<ActivityEpisode>,

    /// Current time. Used downstream for temporal decay and recency
    /// weighting. Defaults to Utc::now() when empty() is called.
    pub now: DateTime<Utc>,

    /// Optional explicit wing focus. Set by consumers who know what
    /// the user is currently working on (e.g., "permagent" when the
    /// user is in the Permagent UI). None means no explicit focus.
    pub focus_wing: Option<String>,

    /// Optional persona block — who is asking, in what role.
    /// Defaults to None. Reserved for future multi-persona scenarios.
    pub persona: Option<String>,
}

impl RecognitionContext {
    /// Empty context — no ambient signal. Use for bench, ad-hoc queries,
    /// or any caller without continuous-awareness data.
    pub fn empty() -> Self {
        Self {
            recent_activity: Vec::new(),
            now: Utc::now(),
            focus_wing: None,
            persona: None,
        }
    }

    /// Builder: set the time anchor (useful for tests and replay).
    pub fn with_now(mut self, now: DateTime<Utc>) -> Self {
        self.now = now;
        self
    }

    /// Builder: set recent activity episodes.
    pub fn with_recent_activity(mut self, episodes: Vec<ActivityEpisode>) -> Self {
        self.recent_activity = episodes;
        self
    }

    /// Builder: set focus wing.
    pub fn with_focus_wing(mut self, wing: impl Into<String>) -> Self {
        self.focus_wing = Some(wing.into());
        self
    }

    /// Builder: set persona.
    pub fn with_persona(mut self, persona: impl Into<String>) -> Self {
        self.persona = Some(persona.into());
        self
    }

    /// True when no ambient signal is available. Layers can use this
    /// as a fast-path check to skip context-conditional logic.
    pub fn is_empty(&self) -> bool {
        self.recent_activity.is_empty() && self.focus_wing.is_none() && self.persona.is_none()
    }
}

impl Default for RecognitionContext {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognition_context_empty_is_empty() {
        let ctx = RecognitionContext::empty();
        assert!(ctx.is_empty());
        assert!(ctx.recent_activity.is_empty());
        assert!(ctx.focus_wing.is_none());
        assert!(ctx.persona.is_none());
    }

    #[test]
    fn recognition_context_builder_pattern() {
        let now = Utc::now();
        let episode = ActivityEpisode {
            id: "ep-1".into(),
            started_at: now,
            ended_at: now,
            bundle_id: "com.test".into(),
            app_name: "Test".into(),
            window_title: None,
            url: None,
            excerpt: None,
            source: "test".into(),
            source_event_count: 1,
            metadata: serde_json::Value::Null,
        };

        let ctx = RecognitionContext::empty()
            .with_now(now)
            .with_recent_activity(vec![episode])
            .with_focus_wing("permagent")
            .with_persona("developer");

        assert!(!ctx.is_empty());
        assert_eq!(ctx.now, now);
        assert_eq!(ctx.recent_activity.len(), 1);
        assert_eq!(ctx.focus_wing.as_deref(), Some("permagent"));
        assert_eq!(ctx.persona.as_deref(), Some("developer"));
    }
}
