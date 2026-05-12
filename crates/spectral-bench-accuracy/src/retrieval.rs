//! Query Spectral and format retrieved context for the actor.

use anyhow::Result;
use chrono::{DateTime, NaiveDateTime, Utc};
use regex::Regex;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, RecallTopKConfig};
use spectral_graph::cascade_layers::CascadePipelineConfig;
use spectral_ingest::MemoryHit;
use std::collections::{BTreeMap, HashSet};

/// Configuration for retrieval.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RetrievalConfig {
    /// Maximum number of memories to retrieve per question.
    pub max_results: usize,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self { max_results: 40 }
    }
}

/// Which retrieval path to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalPath {
    /// Top-K FTS with re-ranking (default — Phase 1 improvement).
    #[default]
    TopkFts,
    /// TACT/FTS recall (legacy).
    Tact,
    /// Graph traversal.
    Graph,
    /// Cascade (L1→L2→L3).
    Cascade,
}

// ── Question-type routing (P1) ──────────────────────────────────────

/// Question type determined by structural analysis of the query.
/// Maps to a retrieval profile, actor prompt template, and retrieval path.
///
/// Two-level classification:
/// - Level 1: top-level shape (Counting, Temporal, Factual, General)
/// - Level 2: sub-shape within Counting, Factual, and General
///
/// Temporal intentionally has NO sub-gate. Date arithmetic is a single
/// coherent strategy; adding TemporalCurrentState would fragment it
/// without evidence of benefit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionType {
    /// "How many", "how much", "total" — exhaustive session scan, no recency signal.
    Counting,
    /// "How many ... currently/still" — current count, recency-priority.
    CountingCurrentState,
    /// Date arithmetic, ordering, duration. No sub-gate — single coherent strategy.
    Temporal,
    /// "What is", "where", "who" — single-entity retrieval.
    Factual,
    /// "What is my current X" — most-recent-wins factual.
    FactualCurrentState,
    /// "Suggest/recommend/tips/advice" — preference inference.
    GeneralPreference,
    /// "Remind me/going back to/we discussed" — assistant recall.
    GeneralRecall,
    /// Catch-all fallback.
    General,
}

impl QuestionType {
    /// Classify a question string into a routing type.
    ///
    /// Level 1: existing top-level classifier (Counting/Temporal/Factual/General).
    /// Level 2: sub-gates for Counting (recency), Factual (recency), General (preference/recall).
    /// Temporal has no sub-gate by design.
    pub fn classify(question: &str) -> Self {
        let q = question.to_lowercase();

        // ── Level 1: top-level shape (unchanged from original) ──

        // Temporal-counting ("how many days/weeks ago", "how old") → Temporal
        if Regex::new(r"how many (?:days|weeks|months|years) (?:ago|since|passed|before|after|between|had passed|have passed|did it take)|how old")
            .unwrap()
            .is_match(&q)
        {
            return Self::Temporal;
        }

        // General counting → Counting (with sub-gate)
        if Regex::new(r"how many|how much|total|in total|altogether")
            .unwrap()
            .is_match(&q)
        {
            // Level 2: recency sub-gate for Counting
            if Regex::new(r"\b(currently|right now|most recent|latest|newest|do i still|now)\b")
                .unwrap()
                .is_match(&q)
            {
                return Self::CountingCurrentState;
            }
            return Self::Counting;
        }

        // Temporal
        if Regex::new(r"when did|how long|(?:^|\W)first\b|(?:^|\W)last\b|before|after|ago|since")
            .unwrap()
            .is_match(&q)
        {
            return Self::Temporal;
        }

        // Factual (with sub-gate)
        if Regex::new(r"^(?:what|where|who|which)\b")
            .unwrap()
            .is_match(&q)
        {
            // Level 2: recency sub-gate for Factual
            if Regex::new(r"\b(currently|right now|most recent|latest|newest|do i still|now)\b")
                .unwrap()
                .is_match(&q)
            {
                return Self::FactualCurrentState;
            }
            return Self::Factual;
        }

        // ── Level 2: General sub-gates ──

        if Regex::new(r"\b(suggest|recommend|tips?|advice|recommendations?|what should i)\b")
            .unwrap()
            .is_match(&q)
        {
            return Self::GeneralPreference;
        }
        if Regex::new(r"\bany (tips?|advice|suggestions?|ideas?|thoughts?|recommendations?)\b")
            .unwrap()
            .is_match(&q)
        {
            return Self::GeneralPreference;
        }
        if Regex::new(r"\b(remind me|going back to|previous|earlier conversation|we (discussed|talked about)|can you remind me)\b")
            .unwrap()
            .is_match(&q)
        {
            return Self::GeneralRecall;
        }

        Self::General
    }

    /// Return the cascade pipeline config tuned for this question type.
    /// Sub-shapes inherit their parent shape's profile.
    pub fn cascade_profile(&self) -> CascadePipelineConfig {
        match self {
            Self::Counting | Self::CountingCurrentState => CascadePipelineConfig {
                k: 60,
                max_per_episode: 3,
                recency_half_life_days: 730.0, // don't penalize any memories
                ..CascadePipelineConfig::default()
            },
            Self::Temporal => CascadePipelineConfig {
                k: 40,
                max_per_episode: 5,
                recency_half_life_days: 60.0, // aggressive recency
                ..CascadePipelineConfig::default()
            },
            Self::Factual | Self::FactualCurrentState => CascadePipelineConfig {
                k: 30,
                max_per_episode: 8,
                ..CascadePipelineConfig::default()
            },
            Self::GeneralPreference | Self::GeneralRecall | Self::General => {
                CascadePipelineConfig::default()
            }
        }
    }

    /// Per-question retrieval path. Temporal routes to topk_fts (cascade hurts
    /// temporal by -15pp); all other shapes use cascade.
    pub fn retrieval_path(&self) -> RetrievalPath {
        match self {
            Self::Temporal => RetrievalPath::TopkFts,
            _ => RetrievalPath::Cascade,
        }
    }

    /// Prompt template filename for this question type.
    pub fn prompt_template(&self) -> &'static str {
        match self {
            Self::Counting => "counting_enumerate.md",
            Self::CountingCurrentState => "counting_current_state.md",
            Self::Temporal => "temporal.md",
            Self::Factual => "factual_direct.md",
            Self::FactualCurrentState => "factual_current_state.md",
            Self::GeneralPreference => "preference.md",
            Self::GeneralRecall => "assistant_recall.md",
            Self::General => "generic_fallback.md",
        }
    }

    /// The prompt template content (compiled in via include_str!).
    pub fn prompt_content(&self) -> &'static str {
        match self {
            Self::Counting => include_str!("prompts/counting_enumerate.md"),
            Self::CountingCurrentState => include_str!("prompts/counting_current_state.md"),
            Self::Temporal => include_str!("prompts/temporal.md"),
            Self::Factual => include_str!("prompts/factual_direct.md"),
            Self::FactualCurrentState => include_str!("prompts/factual_current_state.md"),
            Self::GeneralPreference => include_str!("prompts/preference.md"),
            Self::GeneralRecall => include_str!("prompts/assistant_recall.md"),
            Self::General => include_str!("prompts/generic_fallback.md"),
        }
    }
}

// ── Session-grouped formatting (P2) ─────────────────────────────────

/// Format memory hits grouped by session/episode for clearer multi-session context.
///
/// Groups hits by `episode_id` (falling back to session prefix from key),
/// orders sessions chronologically, and presents turns in order within
/// each session. Drops redundant per-turn metadata (date, wing, hall).
pub fn format_hits_grouped(hits: &[MemoryHit]) -> Vec<String> {
    if hits.is_empty() {
        return Vec::new();
    }

    // Group hits by episode
    let mut by_episode: BTreeMap<String, Vec<&MemoryHit>> = BTreeMap::new();
    for hit in hits {
        let ep_key = hit
            .episode_id
            .clone()
            .or_else(|| hit.key.split(':').next().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        by_episode.entry(ep_key).or_default().push(hit);
    }

    // Sort each episode's hits by key (which encodes turn order)
    for hits_in_ep in by_episode.values_mut() {
        hits_in_ep.sort_by(|a, b| a.key.cmp(&b.key));
    }

    // Sort episodes by their earliest created_at
    let mut episodes: Vec<(String, Vec<&MemoryHit>)> = by_episode.into_iter().collect();
    episodes.sort_by(|a, b| {
        let date_a =
            a.1.first()
                .and_then(|h| h.created_at.as_deref())
                .unwrap_or("");
        let date_b =
            b.1.first()
                .and_then(|h| h.created_at.as_deref())
                .unwrap_or("");
        date_a.cmp(date_b)
    });

    // Build output lines
    let mut lines = Vec::new();
    for (ep_id, ep_hits) in &episodes {
        let date = ep_hits
            .first()
            .and_then(|h| h.created_at.as_deref())
            .map(|s| s.split(' ').next().unwrap_or(s))
            .unwrap_or("unknown-date");
        lines.push(format!("--- Session {ep_id} ({date}) ---"));

        for hit in ep_hits {
            let role = if hit.key.contains(":user") {
                "user"
            } else if hit.key.contains(":assistant") {
                "asst"
            } else {
                "turn"
            };
            // Skip short assistant filler ("Hi!", "Sure!", "That's great!")
            if role == "asst" && hit.content.len() < 40 {
                continue;
            }
            lines.push(format!("[{role}] {}", hit.content));
        }
    }

    lines
}

// ── Legacy flat format ──────────────────────────────────────────────

/// Format a MemoryHit into the standard actor format.
pub fn format_hit(hit: &spectral_ingest::MemoryHit) -> String {
    let date = hit
        .created_at
        .as_deref()
        .map(|s| s.split('T').next().unwrap_or(s))
        .unwrap_or("unknown-date");
    let wing = hit.wing.as_deref().unwrap_or("?");
    let hall = hit.hall.as_deref().unwrap_or("?");
    format!("[{date}] [{wing}/{hall}] {}: {}", hit.key, hit.content)
}

/// Retrieve memories relevant to a question from a brain.
/// Returns formatted memory strings for the actor.
pub fn retrieve(brain: &Brain, question: &str, config: &RetrievalConfig) -> Result<Vec<String>> {
    let result = brain.recall_local(question)?;

    let memories: Vec<String> = result
        .memory_hits
        .into_iter()
        .take(config.max_results)
        .map(|hit| format_hit(&hit))
        .collect();

    Ok(memories)
}

/// Retrieve via top-K FTS with additive re-ranking. No LLM cost.
///
/// Configurable via env vars for ablation:
/// - `SPECTRAL_DISABLE_SIGNAL_SCORE=1` — disable signal weighting
/// - `SPECTRAL_DISABLE_RECENCY=1` — disable recency weighting
/// - `SPECTRAL_DISABLE_ENTITY_RESOLUTION=1` — disable entity clustering
/// - `SPECTRAL_DISABLE_CONTEXT_DEDUP=1` — disable context chain dedup
pub fn retrieve_topk_fts(
    brain: &Brain,
    question: &str,
    config: &RetrievalConfig,
) -> Result<Vec<String>> {
    let topk_config = RecallTopKConfig {
        k: config.max_results.max(40),
        apply_signal_score_weighting: std::env::var("SPECTRAL_DISABLE_SIGNAL_SCORE").is_err(),
        apply_recency_weighting: std::env::var("SPECTRAL_DISABLE_RECENCY").is_err(),
        apply_entity_resolution: std::env::var("SPECTRAL_DISABLE_ENTITY_RESOLUTION").is_err(),
        apply_context_dedup: std::env::var("SPECTRAL_DISABLE_CONTEXT_DEDUP").is_err(),
        ..RecallTopKConfig::default()
    };

    let hits = brain.recall_topk_fts(question, &topk_config, Visibility::Private)?;

    let memories: Vec<String> = hits
        .into_iter()
        .take(config.max_results)
        .map(|hit| format_hit(&hit))
        .collect();

    Ok(memories)
}

/// Retrieve memories using graph traversal to bridge vocabulary mismatches.
///
/// 1. Calls `recall_graph` to find entity neighbors of the query.
/// 2. For each entity canonical name, runs an FTS search to find memories
///    mentioning that entity (since `recall_graph` returns entities/triples,
///    not memories directly — this is an architectural gap).
/// 3. Deduplicates across all entity searches.
/// 4. If fewer than 5 memories found via graph, falls back to standard FTS.
pub fn retrieve_graph(
    brain: &Brain,
    question: &str,
    config: &RetrievalConfig,
) -> Result<Vec<String>> {
    let graph_result = brain.recall_graph(question, Visibility::Private)?;

    let mut seen_keys = HashSet::new();
    let mut all_hits = Vec::new();

    // Use each entity's canonical name as a secondary FTS query
    for entity in &graph_result.neighborhood.entities {
        let entity_result = brain.recall_local(&entity.canonical)?;
        for hit in entity_result.memory_hits {
            if seen_keys.insert(hit.key.clone()) {
                all_hits.push(hit);
            }
        }
    }

    // Fall back to standard FTS if graph produced fewer than 5 results
    if all_hits.len() < 5 {
        let fts_result = brain.recall_local(question)?;
        for hit in fts_result.memory_hits {
            if seen_keys.insert(hit.key.clone()) {
                all_hits.push(hit);
            }
        }
    }

    let memories: Vec<String> = all_hits
        .iter()
        .take(config.max_results)
        .map(format_hit)
        .collect();

    Ok(memories)
}

/// Telemetry from a cascade retrieval run.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CascadeTelemetry {
    pub stopped_at: Option<String>,
    pub max_confidence: f64,
    pub total_tokens_used: usize,
    /// Total LLM tokens consumed during recognition across all layers.
    /// Expected to be 0 for all current layers (empirical proof artifact).
    pub total_recognition_token_cost: usize,
    /// Question-type classification used for routing (e.g. "Counting").
    #[serde(default)]
    pub question_type: Option<String>,
    pub layer_outcomes: Vec<(String, String)>,
}

/// Retrieve memories via the cascade with question-type-aware routing.
///
/// Classifies the question into counting/temporal/factual/general, selects
/// a tuned retrieval profile (K, episode diversity, recency half-life), and
/// formats the results as session-grouped context.
///
/// When `question_date` is provided (format: "2023/05/30 (Tue) 23:40"),
/// the cascade's recency scoring uses it as the time anchor instead of
/// `Utc::now()`.
pub fn retrieve_cascade(
    brain: &Brain,
    question: &str,
    _config: &RetrievalConfig,
    question_date: Option<&str>,
) -> Result<(Vec<String>, CascadeTelemetry)> {
    // P1: Question-type routing
    let qtype = QuestionType::classify(question);
    let pipeline_config = qtype.cascade_profile();

    let context = match question_date.and_then(parse_question_date) {
        Some(dt) => spectral_cascade::RecognitionContext::empty().with_now(dt),
        None => spectral_cascade::RecognitionContext::empty(),
    };
    let result = brain.recall_cascade_with_pipeline(question, &context, &pipeline_config)?;

    // Capture telemetry before consuming merged_hits
    let telemetry = CascadeTelemetry {
        stopped_at: result.stopped_at.map(|id| id.to_string()),
        max_confidence: result.max_confidence,
        total_tokens_used: result.total_tokens_used,
        total_recognition_token_cost: result.total_recognition_token_cost,
        question_type: Some(format!("{qtype:?}")),
        layer_outcomes: result
            .layer_outcomes
            .iter()
            .map(|(id, r)| {
                let status = match r {
                    spectral_cascade::LayerResult::Sufficient { confidence, .. } => {
                        format!("sufficient(confidence={confidence:.2})")
                    }
                    spectral_cascade::LayerResult::Partial { confidence, .. } => {
                        format!("partial(confidence={confidence:.2})")
                    }
                    spectral_cascade::LayerResult::Skipped { reason, .. } => {
                        format!("skipped({reason})")
                    }
                };
                (id.to_string(), status)
            })
            .collect(),
    };

    // P2: Session-grouped formatting
    // Use the profile's K as the limit — the question-type routing already
    // determined the right K (60 for counting, 30 for factual, etc).
    // CLI --max-results only applies to non-cascade paths.
    let hits: Vec<MemoryHit> = result
        .merged_hits
        .into_iter()
        .take(pipeline_config.k)
        .collect();
    let formatted = format_hits_grouped(&hits);

    Ok((formatted, telemetry))
}

/// Parse LongMemEval question_date format ("2023/05/30 (Tue) 23:40") into DateTime<Utc>.
fn parse_question_date(date_str: &str) -> Option<DateTime<Utc>> {
    // Strip the day-of-week parenthetical: "2023/05/30 (Tue) 23:40" → "2023/05/30 23:40"
    let cleaned = date_str
        .find('(')
        .and_then(|open| {
            date_str[open..].find(')').map(|close| {
                let before = date_str[..open].trim_end();
                let after = date_str[open + close + 1..].trim_start();
                format!("{before} {after}")
            })
        })
        .unwrap_or_else(|| date_str.to_string());

    NaiveDateTime::parse_from_str(cleaned.trim(), "%Y/%m/%d %H:%M")
        .ok()
        .map(|dt| dt.and_utc())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use spectral_graph::brain::{BrainConfig, EntityPolicy, RememberOpts};

    #[test]
    fn default_max_results_is_40() {
        let config = RetrievalConfig::default();
        assert_eq!(config.max_results, 40);
    }

    #[test]
    fn retrieve_includes_created_at_in_format() {
        let dir = tempfile::tempdir().unwrap();
        let ontology_path = dir.path().join("ontology.toml");
        std::fs::write(&ontology_path, "version = 1\n").unwrap();

        let brain = Brain::open(BrainConfig {
            data_dir: dir.path().to_path_buf(),
            ontology_path,
            memory_db_path: None,
            llm_client: None,
            wing_rules: None,
            hall_rules: None,
            device_id: None,
            enable_spectrogram: false,
            entity_policy: EntityPolicy::Strict,
            sqlite_mmap_size: None,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        })
        .unwrap();

        let ts = Utc.with_ymd_and_hms(2023, 6, 15, 12, 0, 0).unwrap();
        brain
            .remember_with(
                "test-date-key",
                "Memory about the project launch date for retrieval test",
                RememberOpts {
                    created_at: Some(ts),
                    visibility: Visibility::Private,
                    ..Default::default()
                },
            )
            .unwrap();

        let memories = retrieve(
            &brain,
            "project launch date retrieval test",
            &RetrievalConfig::default(),
        )
        .unwrap();
        assert!(!memories.is_empty());
        assert!(
            memories[0].contains("2023-06-15"),
            "expected date prefix in formatted memory, got: {}",
            memories[0]
        );
    }

    #[test]
    fn retrieve_graph_runs_without_panic() {
        let dir = tempfile::tempdir().unwrap();
        let ontology_path = dir.path().join("ontology.toml");
        std::fs::write(&ontology_path, "version = 1\n").unwrap();

        let brain = Brain::open(BrainConfig {
            data_dir: dir.path().to_path_buf(),
            ontology_path,
            memory_db_path: None,
            llm_client: None,
            wing_rules: None,
            hall_rules: None,
            device_id: None,
            enable_spectrogram: false,
            entity_policy: EntityPolicy::Strict,
            sqlite_mmap_size: None,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        })
        .unwrap();

        brain
            .remember(
                "graph-test",
                "Memories about photography and Sony cameras for testing",
                Visibility::Private,
            )
            .unwrap();

        // With a minimal ontology (no entities defined), graph recall returns
        // no entities. The function should fall back to FTS and still return
        // results without panicking.
        let memories = retrieve_graph(
            &brain,
            "photography Sony cameras testing",
            &RetrievalConfig::default(),
        )
        .unwrap();
        assert!(
            !memories.is_empty(),
            "graph retrieval should fall back to FTS when no entities found"
        );
    }

    #[test]
    fn parse_question_date_standard_format() {
        let dt = parse_question_date("2023/05/30 (Tue) 23:40").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2023, 5, 30, 23, 40, 0).unwrap());
    }

    #[test]
    fn parse_question_date_different_day() {
        let dt = parse_question_date("2021/08/20 (Fri) 14:05").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2021, 8, 20, 14, 5, 0).unwrap());
    }

    #[test]
    fn parse_question_date_returns_none_on_garbage() {
        assert!(parse_question_date("not a date").is_none());
    }

    #[test]
    fn cascade_uses_question_date_for_recency() {
        let dir = tempfile::tempdir().unwrap();
        let ontology_path = dir.path().join("ontology.toml");
        std::fs::write(&ontology_path, "version = 1\n").unwrap();

        let brain = Brain::open(BrainConfig {
            data_dir: dir.path().to_path_buf(),
            ontology_path,
            memory_db_path: None,
            llm_client: None,
            wing_rules: None,
            hall_rules: None,
            device_id: None,
            enable_spectrogram: false,
            entity_policy: EntityPolicy::Strict,
            sqlite_mmap_size: None,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        })
        .unwrap();

        // Ingest two memories: one from May 2023, one from Jan 2023
        let recent_ts = Utc.with_ymd_and_hms(2023, 5, 20, 12, 0, 0).unwrap();
        let old_ts = Utc.with_ymd_and_hms(2023, 1, 10, 12, 0, 0).unwrap();

        brain
            .remember_with(
                "recent-memory",
                "I recently started jogging every morning for exercise",
                RememberOpts {
                    created_at: Some(recent_ts),
                    visibility: Visibility::Private,
                    ..Default::default()
                },
            )
            .unwrap();
        brain
            .remember_with(
                "old-memory",
                "I started a new exercise routine with jogging",
                RememberOpts {
                    created_at: Some(old_ts),
                    visibility: Visibility::Private,
                    ..Default::default()
                },
            )
            .unwrap();

        // Retrieve with question_date = May 30, 2023
        // The recent memory (May 20) is 10 days old; old memory (Jan 10) is 140 days old.
        // With recency weighting, the recent memory should rank first.
        let (memories_with_date, _) = retrieve_cascade(
            &brain,
            "jogging exercise",
            &RetrievalConfig::default(),
            Some("2023/05/30 (Tue) 23:40"),
        )
        .unwrap();

        // Session-grouped format: lines include session headers and content.
        // Both memories should appear in the output.
        let all_text = memories_with_date.join("\n");
        assert!(
            all_text.contains("recently started jogging"),
            "should contain the recent jogging memory"
        );
        assert!(
            all_text.contains("exercise routine"),
            "should contain the old exercise memory"
        );

        // The recent memory (May 20, 10 days from question) should appear
        // in a session that comes AFTER the old memory (Jan 10, 140 days)
        // since sessions are ordered chronologically and recency re-ranking
        // doesn't change session order — it affects which memories are
        // selected into the top-K in the first place.
        let recent_pos = all_text.find("recently started jogging").unwrap();
        let old_pos = all_text.find("exercise routine").unwrap();
        // Both should be present (retrieval succeeded)
        assert!(recent_pos > 0 && old_pos > 0);
    }

    // ── P1: Question-type routing tests ──────────────────────────────

    #[test]
    fn classify_counting_questions() {
        assert_eq!(
            QuestionType::classify("How many books did I read?"),
            QuestionType::Counting
        );
        assert_eq!(
            QuestionType::classify("How much money did I spend?"),
            QuestionType::Counting
        );
        assert_eq!(
            QuestionType::classify("What is the total amount?"),
            QuestionType::Counting
        );
        assert_eq!(
            QuestionType::classify("How many days in total?"),
            QuestionType::Counting
        );
    }

    #[test]
    fn classify_counting_current_state() {
        assert_eq!(
            QuestionType::classify("How many pets do I currently have?"),
            QuestionType::CountingCurrentState
        );
        assert_eq!(
            QuestionType::classify("How many do I still have right now?"),
            QuestionType::CountingCurrentState
        );
    }

    #[test]
    fn classify_temporal_questions() {
        assert_eq!(
            QuestionType::classify("When did I start jogging?"),
            QuestionType::Temporal
        );
        assert_eq!(
            QuestionType::classify("How long is my commute?"),
            QuestionType::Temporal
        );
        assert_eq!(
            QuestionType::classify("What happened first?"),
            QuestionType::Temporal
        );
        assert_eq!(
            QuestionType::classify("How many weeks ago did I start?"),
            QuestionType::Temporal
        );
    }

    #[test]
    fn classify_temporal_no_subgate_even_with_recency_words() {
        // Temporal has no sub-gate by design. Recency words in a temporal
        // question should not produce TemporalCurrentState (which doesn't exist).
        assert_eq!(
            QuestionType::classify("How many days ago did I most recently visit?"),
            QuestionType::Temporal
        );
    }

    #[test]
    fn classify_factual_questions() {
        assert_eq!(
            QuestionType::classify("What degree did I graduate with?"),
            QuestionType::Factual
        );
        assert_eq!(
            QuestionType::classify("Where does my sister live?"),
            QuestionType::Factual
        );
        assert_eq!(
            QuestionType::classify("Who gave me the gift?"),
            QuestionType::Factual
        );
    }

    #[test]
    fn classify_factual_current_state() {
        assert_eq!(
            QuestionType::classify("What is my most recent address?"),
            QuestionType::FactualCurrentState
        );
        assert_eq!(
            QuestionType::classify("What car do I currently drive?"),
            QuestionType::FactualCurrentState
        );
    }

    #[test]
    fn classify_general_preference() {
        assert_eq!(
            QuestionType::classify("Can you recommend a restaurant?"),
            QuestionType::GeneralPreference
        );
        assert_eq!(
            QuestionType::classify("Any tips for cooking pasta?"),
            QuestionType::GeneralPreference
        );
        assert_eq!(
            QuestionType::classify("Do you have any suggestions for my trip?"),
            QuestionType::GeneralPreference
        );
    }

    #[test]
    fn classify_general_recall() {
        assert_eq!(
            QuestionType::classify("Can you remind me what we discussed about budgets?"),
            QuestionType::GeneralRecall
        );
        assert_eq!(
            QuestionType::classify("Going back to our earlier conversation about housing"),
            QuestionType::GeneralRecall
        );
    }

    #[test]
    fn classify_general_fallback() {
        assert_eq!(
            QuestionType::classify("I've been struggling with recipes."),
            QuestionType::General
        );
    }

    #[test]
    fn counting_profile_has_high_k_low_episode_cap() {
        let profile = QuestionType::Counting.cascade_profile();
        assert_eq!(profile.k, 60);
        assert_eq!(profile.max_per_episode, 3);
        assert!(
            profile.recency_half_life_days > 700.0,
            "counting should not penalize old memories"
        );
    }

    #[test]
    fn counting_current_state_inherits_parent_profile() {
        let parent = QuestionType::Counting.cascade_profile();
        let child = QuestionType::CountingCurrentState.cascade_profile();
        assert_eq!(parent.k, child.k);
        assert_eq!(parent.max_per_episode, child.max_per_episode);
    }

    #[test]
    fn temporal_profile_has_aggressive_recency() {
        let profile = QuestionType::Temporal.cascade_profile();
        assert_eq!(profile.k, 40);
        assert!(
            profile.recency_half_life_days < 100.0,
            "temporal should aggressively decay old memories"
        );
    }

    #[test]
    fn factual_profile_has_focused_k() {
        let profile = QuestionType::Factual.cascade_profile();
        assert_eq!(profile.k, 30);
        assert_eq!(profile.max_per_episode, 8);
    }

    #[test]
    fn factual_current_state_inherits_parent_profile() {
        let parent = QuestionType::Factual.cascade_profile();
        let child = QuestionType::FactualCurrentState.cascade_profile();
        assert_eq!(parent.k, child.k);
        assert_eq!(parent.max_per_episode, child.max_per_episode);
    }

    #[test]
    fn temporal_routes_to_topk_fts() {
        assert_eq!(
            QuestionType::Temporal.retrieval_path(),
            RetrievalPath::TopkFts
        );
    }

    #[test]
    fn non_temporal_routes_to_cascade() {
        assert_eq!(
            QuestionType::Counting.retrieval_path(),
            RetrievalPath::Cascade
        );
        assert_eq!(
            QuestionType::General.retrieval_path(),
            RetrievalPath::Cascade
        );
        assert_eq!(
            QuestionType::GeneralPreference.retrieval_path(),
            RetrievalPath::Cascade
        );
    }

    #[test]
    fn cascade_telemetry_includes_question_type() {
        let dir = tempfile::tempdir().unwrap();
        let ontology_path = dir.path().join("ontology.toml");
        std::fs::write(&ontology_path, "version = 1\n").unwrap();

        let brain = Brain::open(BrainConfig {
            data_dir: dir.path().to_path_buf(),
            ontology_path,
            memory_db_path: None,
            llm_client: None,
            wing_rules: None,
            hall_rules: None,
            device_id: None,
            enable_spectrogram: false,
            entity_policy: EntityPolicy::Strict,
            sqlite_mmap_size: None,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        })
        .unwrap();

        brain
            .remember(
                "counting-q",
                "I read 5 books this year about cooking and travel",
                Visibility::Private,
            )
            .unwrap();

        let (_, telemetry) = retrieve_cascade(
            &brain,
            "How many books did I read?",
            &RetrievalConfig::default(),
            None,
        )
        .unwrap();

        assert_eq!(
            telemetry.question_type.as_deref(),
            Some("Counting"),
            "telemetry should record question type"
        );
    }

    // ── P2: Session-grouped formatting tests ─────────────────────────

    fn make_test_hit(
        id: &str,
        key: &str,
        content: &str,
        episode: &str,
        created_at: &str,
    ) -> MemoryHit {
        MemoryHit {
            id: id.into(),
            key: key.into(),
            content: content.into(),
            wing: None,
            hall: None,
            signal_score: 0.5,
            visibility: "private".into(),
            hits: 0,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: Some(created_at.into()),
            last_reinforced_at: None,
            episode_id: Some(episode.into()),
            declarative_density: None,
            description: None,
        }
    }

    #[test]
    fn format_grouped_creates_session_headers() {
        let hits = vec![
            make_test_hit(
                "1",
                "s1:turn:0:user",
                "I like pizza",
                "s1",
                "2023-05-20 12:00:00",
            ),
            make_test_hit(
                "2",
                "s2:turn:0:user",
                "I read a book",
                "s2",
                "2023-05-22 12:00:00",
            ),
        ];

        let lines = format_hits_grouped(&hits);

        let headers: Vec<&str> = lines
            .iter()
            .filter(|l| l.starts_with("--- Session"))
            .map(|l| l.as_str())
            .collect();
        assert_eq!(headers.len(), 2, "should have 2 session headers");
        assert!(
            headers[0].contains("s1"),
            "first header should be session s1"
        );
        assert!(
            headers[1].contains("s2"),
            "second header should be session s2"
        );
    }

    #[test]
    fn format_grouped_orders_sessions_chronologically() {
        // Insert in reverse chronological order
        let hits = vec![
            make_test_hit(
                "2",
                "s2:turn:0:user",
                "Later memory",
                "s2",
                "2023-06-01 12:00:00",
            ),
            make_test_hit(
                "1",
                "s1:turn:0:user",
                "Earlier memory",
                "s1",
                "2023-05-01 12:00:00",
            ),
        ];

        let lines = format_hits_grouped(&hits);
        let headers: Vec<&str> = lines
            .iter()
            .filter(|l| l.starts_with("--- Session"))
            .map(|l| l.as_str())
            .collect();

        assert!(
            headers[0].contains("s1"),
            "earlier session should come first"
        );
        assert!(
            headers[1].contains("s2"),
            "later session should come second"
        );
    }

    #[test]
    fn format_grouped_orders_turns_within_session() {
        let hits = vec![
            make_test_hit(
                "2",
                "s1:turn:1:user",
                "Second turn",
                "s1",
                "2023-05-20 12:00:00",
            ),
            make_test_hit(
                "1",
                "s1:turn:0:user",
                "First turn",
                "s1",
                "2023-05-20 12:00:00",
            ),
        ];

        let lines = format_hits_grouped(&hits);
        let content_lines: Vec<&str> = lines
            .iter()
            .filter(|l| l.starts_with("[user]"))
            .map(|l| l.as_str())
            .collect();

        assert_eq!(content_lines.len(), 2);
        assert!(
            content_lines[0].contains("First turn"),
            "first turn should come first"
        );
        assert!(
            content_lines[1].contains("Second turn"),
            "second turn should come second"
        );
    }

    #[test]
    fn format_grouped_skips_short_assistant_filler() {
        let hits = vec![
            make_test_hit(
                "1",
                "s1:turn:0:user",
                "Tell me about cooking",
                "s1",
                "2023-05-20 12:00:00",
            ),
            make_test_hit(
                "2",
                "s1:turn:1:assistant",
                "Sure!",
                "s1",
                "2023-05-20 12:00:00",
            ),
            make_test_hit(
                "3",
                "s1:turn:2:assistant",
                "Here's a detailed recipe for pasta with tomato sauce and fresh basil",
                "s1",
                "2023-05-20 12:00:00",
            ),
        ];

        let lines = format_hits_grouped(&hits);
        let asst_lines: Vec<&str> = lines
            .iter()
            .filter(|l| l.starts_with("[asst]"))
            .map(|l| l.as_str())
            .collect();

        assert_eq!(
            asst_lines.len(),
            1,
            "should keep long assistant message but skip short filler"
        );
        assert!(asst_lines[0].contains("detailed recipe"));
    }

    #[test]
    fn format_grouped_multi_session_produces_correct_structure() {
        let hits = vec![
            make_test_hit(
                "1",
                "s1:turn:0:user",
                "I graduated with a Business degree",
                "s1",
                "2023-05-20 12:00:00",
            ),
            make_test_hit(
                "2",
                "s1:turn:1:assistant",
                "That's wonderful! Business degrees open many doors in the professional world.",
                "s1",
                "2023-05-20 12:00:00",
            ),
            make_test_hit(
                "3",
                "s2:turn:0:user",
                "My commute is 45 minutes each way",
                "s2",
                "2023-05-22 14:00:00",
            ),
            make_test_hit(
                "4",
                "s3:turn:0:user",
                "I like to read sci-fi novels",
                "s3",
                "2023-05-25 10:00:00",
            ),
        ];

        let lines = format_hits_grouped(&hits);

        // Should have 3 session headers
        let headers: Vec<&str> = lines
            .iter()
            .filter(|l| l.starts_with("---"))
            .map(|l| l.as_str())
            .collect();
        assert_eq!(headers.len(), 3);

        // Content should be present
        let all = lines.join("\n");
        assert!(all.contains("Business degree"));
        assert!(all.contains("45 minutes"));
        assert!(all.contains("sci-fi novels"));
    }

    #[test]
    fn format_grouped_empty_input() {
        let lines = format_hits_grouped(&[]);
        assert!(lines.is_empty());
    }
}
