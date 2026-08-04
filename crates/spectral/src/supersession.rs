//! Read-time suppression of superseded facts.
//!
//! # The problem
//!
//! `knowledge-update` questions ("the user's situation changed — use the
//! updated information") have **99.4% session-recall** and **87.2% end-to-end
//! accuracy**. Retrieval is not the failure. The failure is that retrieval
//! hands the actor *every* version of a changed fact at once — "I use Notion",
//! then later "I switched from Notion to Obsidian" — and leaves it to work out
//! which one is current.
//!
//! DMF (arXiv 2606.03463) suppresses records "invalidated by lineage, user
//! corrections, or newer topic winners" before rendering, and reports 1.000 on
//! knowledge-update (n=10). Spectral already does this for **triples** —
//! `insert_triple_superseding`, ontology `single_valued` — but the FTS recall
//! path that answers these questions has no equivalent.
//!
//! # Design constraints, taken from this repo's own record
//!
//! 1. **Read-time only, never deletion.** The write path must not erase
//!    evidence. The turn-contract debate settled that "repeated delivery is
//!    partly a retriever property, so penalising it lets the write path erase
//!    evidence of a read-path defect". Suppression here filters a *result
//!    set*; the memories are untouched.
//! 2. **Conservative extraction.** A topic key is only assigned when a narrow,
//!    deterministic pattern matches. Anything unrecognised is never suppressed.
//!    Over-suppression destroys answer evidence, and the oracle can measure
//!    that harm even though it cannot measure the benefit.
//! 3. **Superseded items are marked, not silently dropped**, so a caller can
//!    render them as history or audit the decision.
//!
//! # Status
//!
//! **Default OFF and unproven.** The $0 oracle can only measure whether this
//! *harms* retrieval; the benefit is actor-side and needs a paid run. See
//! `docs/internal/supersession-prereg-2026-08-03.md`.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use spectral_ingest::MemoryHit;

macro_rules! cached {
    ($name:ident, $pattern:expr) => {
        fn $name() -> &'static Regex {
            static CELL: OnceLock<Regex> = OnceLock::new();
            CELL.get_or_init(|| Regex::new($pattern).unwrap())
        }
    };
}

// Narrow, first-person, present-tense state assertions. Each captures the
// ATTRIBUTE being asserted about; the value is deliberately not captured,
// because two different values for the same attribute is exactly the
// supersession case we want to detect.
cached!(
    re_my_x_is,
    r"(?i)\bmy (?:current |new |latest )?([a-z][a-z '\-]{2,30}?) (?:is|are|has become|'s)\b"
);
cached!(
    re_switched_to,
    r"(?i)\bi (?:switched|moved|upgraded|downgraded|changed)(?: over)? (?:to|from)\b.{0,60}?\bfor (?:my )?([a-z][a-z '\-]{2,30})"
);
cached!(
    re_i_now_use,
    r"(?i)\bi (?:now |currently )(?:use|have|own|work at|live in)\b"
);

/// How a hit was classified for supersession.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// No topic key could be extracted conservatively — never suppressed.
    Unclassified,
    /// Carries a topic key and is the newest assertion for it.
    Current(String),
    /// Carries a topic key and an older timestamp than another retrieved hit
    /// asserting the same topic.
    Superseded {
        /// Normalised topic key.
        topic: String,
        /// Key of the memory that supersedes it.
        superseded_by: String,
    },
}

/// Result of a supersession pass. Nothing is discarded — the caller decides.
#[derive(Debug, Clone)]
pub struct SupersessionReport {
    /// Hits considered current or unclassifiable, in the input order.
    pub kept: Vec<MemoryHit>,
    /// Hits suppressed, each with the key that superseded it.
    pub suppressed: Vec<(MemoryHit, String)>,
}

/// Config for the supersession pass.
#[derive(Debug, Clone)]
pub struct SupersessionConfig {
    /// Master switch. **Default `false`** — unproven.
    pub enabled: bool,
    /// Require both hits to sit in *different* sessions before suppressing.
    ///
    /// Within one session a later turn is usually elaboration, not
    /// replacement ("my laptop is old" → "my laptop is a Framework 13"), and
    /// suppressing the earlier one loses context the actor may need. Across
    /// sessions, a restated attribute is far more likely a genuine update.
    /// Default `true` — the conservative choice.
    pub cross_session_only: bool,
}

impl Default for SupersessionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cross_session_only: true,
        }
    }
}

impl SupersessionConfig {
    /// Enabled with conservative defaults.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }
}

/// Normalise an extracted attribute into a topic key.
fn normalise(attr: &str) -> String {
    attr.trim()
        .to_lowercase()
        .trim_end_matches(|c: char| !c.is_alphanumeric())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Conservatively extract a topic key for a memory, or `None`.
///
/// Only first-person state assertions with an explicit attribute qualify.
/// Everything else is left unclassified and therefore unsuppressable.
pub fn topic_key(content: &str) -> Option<String> {
    if let Some(c) = re_my_x_is().captures(content) {
        let attr = normalise(c.get(1)?.as_str());
        if !attr.is_empty() {
            return Some(attr);
        }
    }
    if let Some(c) = re_switched_to().captures(content) {
        let attr = normalise(c.get(1)?.as_str());
        if !attr.is_empty() {
            return Some(attr);
        }
    }
    None
}

/// Whether a memory reads as a present-tense state assertion at all.
///
/// Used only to keep `i now use ...` phrasing from being treated as
/// unclassified when it carries no explicit attribute; it never assigns a
/// topic on its own.
pub fn asserts_current_state(content: &str) -> bool {
    re_i_now_use().is_match(content)
}

fn session_of(hit: &MemoryHit) -> String {
    hit.episode_id
        .clone()
        .or_else(|| hit.key.split(':').next().map(|s| s.to_string()))
        .unwrap_or_default()
}

/// Timestamp used for ordering. Missing timestamps sort oldest, so an
/// undated memory can never suppress a dated one.
fn ts(hit: &MemoryHit) -> String {
    hit.created_at.clone().unwrap_or_default()
}

/// Partition `hits` into current and superseded.
///
/// For each conservatively-extracted topic key, the hit with the latest
/// `created_at` wins; the rest are marked superseded by it. Ties break on
/// memory key so the outcome never depends on input order.
///
/// A no-op returning everything as `kept` when `config.enabled` is false.
pub fn partition(hits: &[MemoryHit], config: &SupersessionConfig) -> SupersessionReport {
    if !config.enabled || hits.len() < 2 {
        return SupersessionReport {
            kept: hits.to_vec(),
            suppressed: Vec::new(),
        };
    }

    // topic -> index of the winning hit
    let mut winner: HashMap<String, usize> = HashMap::new();
    let keys: Vec<Option<String>> = hits.iter().map(|h| topic_key(&h.content)).collect();

    for (i, key) in keys.iter().enumerate() {
        let Some(topic) = key else { continue };
        match winner.get(topic) {
            None => {
                winner.insert(topic.clone(), i);
            }
            Some(&j) => {
                let better = (ts(&hits[i]), &hits[i].key) > (ts(&hits[j]), &hits[j].key);
                if better {
                    winner.insert(topic.clone(), i);
                }
            }
        }
    }

    let mut kept = Vec::new();
    let mut suppressed = Vec::new();
    for (i, hit) in hits.iter().enumerate() {
        let Some(topic) = &keys[i] else {
            kept.push(hit.clone());
            continue;
        };
        let w = winner[topic];
        if w == i {
            kept.push(hit.clone());
            continue;
        }
        if config.cross_session_only && session_of(hit) == session_of(&hits[w]) {
            kept.push(hit.clone());
            continue;
        }
        suppressed.push((hit.clone(), hits[w].key.clone()));
    }

    SupersessionReport { kept, suppressed }
}

/// Classify one hit's standing within a result set. For audit and explanation.
pub fn standing(hits: &[MemoryHit], index: usize, config: &SupersessionConfig) -> Standing {
    let report = partition(hits, config);
    let hit = &hits[index];
    if let Some((_, by)) = report.suppressed.iter().find(|(h, _)| h.key == hit.key) {
        return Standing::Superseded {
            topic: topic_key(&hit.content).unwrap_or_default(),
            superseded_by: by.clone(),
        };
    }
    match topic_key(&hit.content) {
        Some(t) => Standing::Current(t),
        None => Standing::Unclassified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(key: &str, content: &str, created: &str, episode: &str) -> MemoryHit {
        MemoryHit {
            id: key.to_string(),
            key: key.to_string(),
            content: content.to_string(),
            wing: None,
            hall: None,
            signal_score: 1.0,
            visibility: "private".to_string(),
            hits: 1,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: Some(created.to_string()),
            last_reinforced_at: None,
            episode_id: Some(episode.to_string()),
            declarative_density: None,
            description: None,
            source_brain_id: None,
            signature: None,
        }
    }

    #[test]
    fn extracts_only_narrow_first_person_assertions() {
        assert_eq!(
            topic_key("my note-taking app is Notion").as_deref(),
            Some("note-taking app")
        );
        assert_eq!(
            topic_key("My current employer is Stripe").as_deref(),
            Some("employer")
        );
        // Not a first-person state assertion — must stay unclassified.
        assert_eq!(topic_key("the weather is nice today"), None);
        assert_eq!(topic_key("her laptop is a Framework"), None);
        assert_eq!(topic_key("I went running yesterday"), None);
    }

    #[test]
    fn disabled_is_a_no_op() {
        let hits = vec![
            hit("s1:0", "my note-taking app is Notion", "2023-01-01", "s1"),
            hit("s2:0", "my note-taking app is Obsidian", "2023-06-01", "s2"),
        ];
        let r = partition(&hits, &SupersessionConfig::default());
        assert_eq!(r.kept.len(), 2);
        assert!(r.suppressed.is_empty());
    }

    #[test]
    fn newest_assertion_wins_across_sessions() {
        let hits = vec![
            hit("s1:0", "my note-taking app is Notion", "2023-01-01", "s1"),
            hit("s2:0", "my note-taking app is Obsidian", "2023-06-01", "s2"),
        ];
        let r = partition(&hits, &SupersessionConfig::enabled());
        assert_eq!(r.kept.len(), 1);
        assert_eq!(r.kept[0].key, "s2:0");
        assert_eq!(r.suppressed.len(), 1);
        assert_eq!(r.suppressed[0].0.key, "s1:0");
        assert_eq!(r.suppressed[0].1, "s2:0");
    }

    #[test]
    fn within_session_restatement_is_not_suppressed_by_default() {
        // Elaboration inside one session, not replacement.
        let hits = vec![
            hit("s1:0", "my laptop is old", "2023-01-01", "s1"),
            hit("s1:1", "my laptop is a Framework 13", "2023-01-01", "s1"),
        ];
        let r = partition(&hits, &SupersessionConfig::enabled());
        assert_eq!(r.kept.len(), 2, "same-session elaboration must survive");
    }

    #[test]
    fn unclassified_hits_are_never_suppressed() {
        let hits = vec![
            hit(
                "s1:0",
                "we discussed the roadmap at length",
                "2023-01-01",
                "s1",
            ),
            hit(
                "s2:0",
                "we discussed the roadmap again later",
                "2023-06-01",
                "s2",
            ),
        ];
        let r = partition(&hits, &SupersessionConfig::enabled());
        assert_eq!(r.kept.len(), 2);
        assert!(r.suppressed.is_empty());
    }

    #[test]
    fn different_topics_never_collide() {
        let hits = vec![
            hit("s1:0", "my note-taking app is Notion", "2023-01-01", "s1"),
            hit("s2:0", "my employer is Stripe", "2023-06-01", "s2"),
        ];
        let r = partition(&hits, &SupersessionConfig::enabled());
        assert_eq!(r.kept.len(), 2);
    }

    #[test]
    fn undated_memory_never_suppresses_a_dated_one() {
        let mut undated = hit("s2:0", "my employer is Acme", "", "s2");
        undated.created_at = None;
        let hits = vec![
            hit("s1:0", "my employer is Stripe", "2023-06-01", "s1"),
            undated,
        ];
        let r = partition(&hits, &SupersessionConfig::enabled());
        assert_eq!(r.kept[0].key, "s1:0", "dated memory must win");
    }

    #[test]
    fn partition_is_order_independent() {
        let a = hit("s1:0", "my employer is Stripe", "2023-01-01", "s1");
        let b = hit("s2:0", "my employer is Acme", "2023-06-01", "s2");
        let cfg = SupersessionConfig::enabled();
        let fwd = partition(&[a.clone(), b.clone()], &cfg);
        let rev = partition(&[b, a], &cfg);
        assert_eq!(fwd.kept[0].key, rev.kept[0].key);
        assert_eq!(fwd.suppressed[0].0.key, rev.suppressed[0].0.key);
    }

    #[test]
    fn nothing_is_lost_overall() {
        let hits = vec![
            hit("s1:0", "my employer is Stripe", "2023-01-01", "s1"),
            hit("s2:0", "my employer is Acme", "2023-06-01", "s2"),
            hit("s3:0", "unrelated content here", "2023-07-01", "s3"),
        ];
        let r = partition(&hits, &SupersessionConfig::enabled());
        assert_eq!(
            r.kept.len() + r.suppressed.len(),
            hits.len(),
            "supersession must partition, never drop"
        );
    }

    #[test]
    fn standing_explains_the_decision() {
        let hits = vec![
            hit("s1:0", "my employer is Stripe", "2023-01-01", "s1"),
            hit("s2:0", "my employer is Acme", "2023-06-01", "s2"),
        ];
        let cfg = SupersessionConfig::enabled();
        assert!(matches!(
            standing(&hits, 0, &cfg),
            Standing::Superseded { .. }
        ));
        assert!(matches!(standing(&hits, 1, &cfg), Standing::Current(_)));
    }
}
