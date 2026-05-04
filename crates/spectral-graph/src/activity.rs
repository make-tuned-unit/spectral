//! Activity ingestion and continuous-awareness types.
//!
//! Spectral supports continuous-awareness consumers — operating-system-level
//! activity capture systems that feed live user activity into the brain for
//! recognition-mode operation.
//!
//! # Episode model
//!
//! An [`ActivityEpisode`] represents a contiguous period of user activity
//! in a single app/window context. Episodes are stored as regular memories
//! with `wing = "activity"` (configurable) and `hall = source`.
//!
//! # Probe vs recall
//!
//! - **`recall(query)`** is user-initiated: "what do I know about X?"
//! - **`probe(context)`** is system-initiated: given what the user is
//!   currently doing, what related knowledge exists?
//! - **`probe_recent(window)`** synthesizes recent activity into a
//!   context and probes the brain for related knowledge.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashSet;

// ActivityEpisode struct and basic impls live in spectral-ingest so that
// spectral-cascade can reference the type without a circular dependency.
pub use spectral_ingest::activity::ActivityEpisode;

/// Statistics from an activity ingestion batch.
#[derive(Debug, Clone, Default, Serialize)]
pub struct IngestActivityStats {
    pub episodes_received: usize,
    pub episodes_inserted: usize,
    pub episodes_updated: usize,
    pub episodes_redacted: usize,
    pub episodes_rejected: usize,
}

/// Options for probe-based recognition.
#[derive(Debug, Clone)]
pub struct ProbeOpts {
    pub max_results: usize,
    pub min_relevance: f64,
    pub wing_filter: Option<String>,
}

impl Default for ProbeOpts {
    fn default() -> Self {
        Self {
            max_results: 10,
            min_relevance: 0.0,
            wing_filter: None,
        }
    }
}

/// A memory recognized as relevant to the current context.
#[derive(Debug, Clone, Serialize)]
pub struct RecognizedMemory {
    pub id: String,
    pub key: String,
    pub content: String,
    pub wing: Option<String>,
    pub hall: Option<String>,
    pub signal_score: f64,
    pub relevance: f64,
    pub hits: usize,
}

/// Time window for probe_recent.
#[derive(Debug, Clone)]
pub enum ProbeWindow {
    /// Last N minutes of episodes.
    Duration(chrono::Duration),
    /// Last N episodes regardless of time.
    Count(usize),
    /// Episodes since a specific timestamp.
    Since(DateTime<Utc>),
}

impl Default for ProbeWindow {
    fn default() -> Self {
        Self::Duration(chrono::Duration::minutes(10))
    }
}

/// Statistics from a retention rollup operation.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RollupStats {
    pub pruned: usize,
    pub consolidated: usize,
}

// ── Redaction ────────────────────────────────────────────────────

/// Pluggable redaction policy applied to activity episodes before storage.
pub trait RedactionPolicy: Send + Sync {
    fn redact(&self, episode: ActivityEpisode) -> Option<ActivityEpisode>;
}

/// Default redaction policy. Strips common credential patterns from
/// window_title, url, and excerpt fields.
#[derive(Default)]
pub struct DefaultRedactionPolicy {
    /// Whether to redact email addresses. Default false.
    pub redact_emails: bool,
}

impl DefaultRedactionPolicy {
    fn redact_string(&self, s: &str) -> String {
        use std::sync::OnceLock;

        static SSH_RE: OnceLock<regex::Regex> = OnceLock::new();
        static URL_TOKEN_RE: OnceLock<regex::Regex> = OnceLock::new();
        static BEARER_RE: OnceLock<regex::Regex> = OnceLock::new();
        static API_KEY_RE: OnceLock<regex::Regex> = OnceLock::new();
        static EMAIL_RE: OnceLock<regex::Regex> = OnceLock::new();

        let ssh_re = SSH_RE.get_or_init(|| regex::Regex::new(r"(?i)ssh\s+\S+:\S+@\S+").unwrap());
        let url_token_re = URL_TOKEN_RE.get_or_init(|| {
            regex::Regex::new(
                r"(?i)([?&])(token|key|password|secret|auth|api_key|access_token|refresh_token)=[^&\s]*",
            )
            .unwrap()
        });
        let bearer_re =
            BEARER_RE.get_or_init(|| regex::Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._\-]+").unwrap());
        let api_key_re = API_KEY_RE.get_or_init(|| {
            regex::Regex::new(
                r"(?:sk-[A-Za-z0-9]{20,}|ghp_[A-Za-z0-9]{36,}|AIzaSy[A-Za-z0-9_\-]{33})",
            )
            .unwrap()
        });
        let email_re = EMAIL_RE.get_or_init(|| {
            regex::Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}").unwrap()
        });

        let mut result = s.to_string();
        result = ssh_re.replace_all(&result, "[SSH_REDACTED]").to_string();
        result = url_token_re
            .replace_all(&result, "${1}${2}=[REDACTED]")
            .to_string();
        result = bearer_re
            .replace_all(&result, "Bearer [REDACTED]")
            .to_string();
        result = api_key_re
            .replace_all(&result, "[API_KEY_REDACTED]")
            .to_string();
        if self.redact_emails {
            result = email_re
                .replace_all(&result, "[EMAIL_REDACTED]")
                .to_string();
        }
        result
    }
}

impl RedactionPolicy for DefaultRedactionPolicy {
    fn redact(&self, mut episode: ActivityEpisode) -> Option<ActivityEpisode> {
        if let Some(ref title) = episode.window_title {
            episode.window_title = Some(self.redact_string(title));
        }
        if let Some(ref url) = episode.url {
            episode.url = Some(self.redact_string(url));
        }
        if let Some(ref excerpt) = episode.excerpt {
            episode.excerpt = Some(self.redact_string(excerpt));
        }
        Some(episode)
    }
}

/// No-op policy that stores episodes verbatim.
pub struct NoOpRedactionPolicy;

impl RedactionPolicy for NoOpRedactionPolicy {
    fn redact(&self, episode: ActivityEpisode) -> Option<ActivityEpisode> {
        Some(episode)
    }
}

/// Drops episodes where bundle_id matches the excluded set.
pub struct ExcludeBundlesPolicy {
    pub excluded_bundles: HashSet<String>,
}

impl RedactionPolicy for ExcludeBundlesPolicy {
    fn redact(&self, episode: ActivityEpisode) -> Option<ActivityEpisode> {
        if self.excluded_bundles.contains(&episode.bundle_id) {
            None
        } else {
            Some(episode)
        }
    }
}

/// Composite policy that applies multiple policies in sequence.
pub struct ComposeRedaction(pub Vec<Box<dyn RedactionPolicy>>);

impl RedactionPolicy for ComposeRedaction {
    fn redact(&self, episode: ActivityEpisode) -> Option<ActivityEpisode> {
        let mut ep = episode;
        for policy in &self.0 {
            ep = policy.redact(ep)?;
        }
        Some(ep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_episode() -> ActivityEpisode {
        ActivityEpisode {
            id: "ep-1".into(),
            started_at: Utc::now(),
            ended_at: Utc::now() + chrono::Duration::minutes(5),
            bundle_id: "com.example.app".into(),
            app_name: "TestApp".into(),
            window_title: Some("My Window".into()),
            url: None,
            excerpt: None,
            source: "accessibility".into(),
            source_event_count: 10,
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn episode_duration_secs() {
        let ep = test_episode();
        assert!((ep.duration_secs() - 300.0).abs() < 1.0);
    }

    #[test]
    fn episode_signal_score_basic() {
        let ep = test_episode();
        let score = ep.compute_signal_score();
        assert!(score > 0.4 && score < 0.7, "got {score}");
    }

    #[test]
    fn episode_signal_score_with_engagement() {
        let mut ep = test_episode();
        ep.metadata = serde_json::json!({"engagement_score": 0.8});
        let score = ep.compute_signal_score();
        assert!(score > 0.6, "engagement should increase score, got {score}");
    }

    #[test]
    fn episode_to_content() {
        let mut ep = test_episode();
        ep.url = Some("https://example.com".into());
        let content = ep.to_content();
        assert!(content.contains("TestApp"));
        assert!(content.contains("My Window"));
        assert!(content.contains("example.com"));
    }

    #[test]
    fn default_policy_redacts_ssh_credentials() {
        let policy = DefaultRedactionPolicy::default();
        let mut ep = test_episode();
        ep.window_title = Some("ssh alice:s3cret@host.example.com".into());
        let result = policy.redact(ep).unwrap();
        assert!(!result.window_title.as_ref().unwrap().contains("s3cret"));
        assert!(result
            .window_title
            .as_ref()
            .unwrap()
            .contains("[SSH_REDACTED]"));
    }

    #[test]
    fn default_policy_redacts_url_token_query_params() {
        let policy = DefaultRedactionPolicy::default();
        let mut ep = test_episode();
        ep.url = Some("https://api.example.com?token=abc123&name=alice".into());
        let result = policy.redact(ep).unwrap();
        let url = result.url.as_ref().unwrap();
        assert!(!url.contains("abc123"));
        assert!(url.contains("token=[REDACTED]"));
        assert!(url.contains("name=alice"));
    }

    #[test]
    fn default_policy_redacts_bearer_tokens_in_excerpt() {
        let policy = DefaultRedactionPolicy::default();
        let mut ep = test_episode();
        ep.excerpt = Some("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload".into());
        let result = policy.redact(ep).unwrap();
        assert!(!result.excerpt.as_ref().unwrap().contains("eyJ"));
    }

    #[test]
    fn default_policy_redacts_known_api_key_patterns() {
        let policy = DefaultRedactionPolicy::default();
        let mut ep = test_episode();
        ep.window_title = Some("API key: sk-1234567890abcdefghijklmn".into());
        let result = policy.redact(ep).unwrap();
        assert!(result
            .window_title
            .as_ref()
            .unwrap()
            .contains("[API_KEY_REDACTED]"));
    }

    #[test]
    fn default_policy_preserves_emails_when_redact_emails_false() {
        let policy = DefaultRedactionPolicy {
            redact_emails: false,
        };
        let mut ep = test_episode();
        ep.excerpt = Some("Contact alice@example.com for details".into());
        let result = policy.redact(ep).unwrap();
        assert!(result
            .excerpt
            .as_ref()
            .unwrap()
            .contains("alice@example.com"));
    }

    #[test]
    fn noop_policy_passes_through() {
        let policy = NoOpRedactionPolicy;
        let ep = test_episode();
        let original_title = ep.window_title.clone();
        let result = policy.redact(ep).unwrap();
        assert_eq!(result.window_title, original_title);
    }

    #[test]
    fn exclude_bundles_drops_excluded() {
        let mut excluded = HashSet::new();
        excluded.insert("com.secret.banking".into());
        let policy = ExcludeBundlesPolicy {
            excluded_bundles: excluded,
        };

        let mut ep = test_episode();
        ep.bundle_id = "com.secret.banking".into();
        assert!(policy.redact(ep).is_none());

        let ep2 = test_episode();
        assert!(policy.redact(ep2).is_some());
    }

    #[test]
    fn compose_applies_in_sequence() {
        let mut excluded = HashSet::new();
        excluded.insert("com.secret.banking".into());
        let compose = ComposeRedaction(vec![
            Box::new(ExcludeBundlesPolicy {
                excluded_bundles: excluded,
            }),
            Box::new(DefaultRedactionPolicy::default()),
        ]);

        let mut ep = test_episode();
        ep.bundle_id = "com.secret.banking".into();
        assert!(compose.redact(ep).is_none());

        let mut ep2 = test_episode();
        ep2.window_title = Some("ssh bob:pass@host".into());
        let result = compose.redact(ep2).unwrap();
        assert!(result
            .window_title
            .as_ref()
            .unwrap()
            .contains("[SSH_REDACTED]"));
    }
}
