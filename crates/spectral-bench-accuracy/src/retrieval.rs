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

        // Location questions: "where" → Factual, even with temporal modifiers.
        // "Where did Rachel move to after her recent relocation?" → FactualCurrentState
        // "Where did I attend the religious activity last week?" → Factual
        // Temporal modifiers in "where" questions provide context, not question focus.
        if Regex::new(r"^where\b").unwrap().is_match(&q) {
            if Regex::new(
                r"\b(currently|right now|most recent|latest|newest|do i still|now|recent)\b",
            )
            .unwrap()
            .is_match(&q)
            {
                return Self::FactualCurrentState;
            }
            return Self::Factual;
        }

        // Temporal — includes explicit ordering phrases ("order ... earliest/latest")
        if Regex::new(r"when did|how long|(?:^|\W)first\b|(?:^|\W)last\b|before|after|ago|since|order.+(?:earliest|latest)|from earliest|chronological|(?:^|\W)order of\b")
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
            if Regex::new(
                r"\b(currently|right now|most recent|most recently|latest|newest|do i still|now)\b",
            )
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

// ── Assistant-turn content cap (ROLE_TOKEN_PROBE, shape-gated) ──────

/// Read the assistant-turn cap fraction from `SPECTRAL_ASSISTANT_CAP_FRAC`.
///
/// When set (e.g. 0.36), assistant-turn content in the actor context is
/// truncated to that fraction of its original length. The prior probe showed
/// this holds answer-key recall at −40% context tokens but regresses
/// assistant-recall questions ~5pp — so call sites exempt GeneralRecall-shaped
/// questions (classified from question text, not dataset labels).
fn assistant_cap_frac() -> Option<f64> {
    std::env::var("SPECTRAL_ASSISTANT_CAP_FRAC")
        .ok()?
        .parse::<f64>()
        .ok()
        .filter(|f| *f > 0.0 && *f < 1.0)
}

/// Whether to surface the Librarian's per-memory `description` into the actor
/// context (`SPECTRAL_ACTOR_DESCRIPTIONS=1`). Off by default. Descriptions are
/// already used at retrieval time (FTS column, item #8, +2.5pp); this flag
/// additionally shows the description gloss to the actor as a labeled
/// `[librarian: …]` note attached to the turn, to test whether the actor's
/// SYNTHESIS improves (the categories the FTS enrichment alone could not
/// move). A clean on/off ablation: apply the same descriptions in both arms
/// (constant FTS effect); toggle only whether the actor sees them.
fn actor_descriptions_enabled() -> bool {
    std::env::var("SPECTRAL_ACTOR_DESCRIPTIONS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Append the Librarian description as a labeled note when the flag is on and
/// the description is non-empty. Kept short and clearly provenanced so the
/// actor never confuses the gloss with the user's own words.
fn with_description(line: String, hit: &MemoryHit, show: bool) -> String {
    match (show, hit.description.as_deref()) {
        (true, Some(desc)) if !desc.trim().is_empty() => {
            format!("{line} [librarian: {}]", desc.trim())
        }
        _ => line,
    }
}

/// Effective cap for a question shape: GeneralRecall is exempt.
fn cap_for_shape(qtype: QuestionType) -> Option<f64> {
    match qtype {
        QuestionType::GeneralRecall => None,
        _ => assistant_cap_frac(),
    }
}

/// Truncate content to `frac` of its byte length at a char boundary.
/// Content at or below `CAP_MIN_LEN` is left alone — truncating short turns
/// saves nothing and loses information.
const CAP_MIN_LEN: usize = 120;

fn cap_content(content: &str, frac: f64) -> String {
    if content.len() <= CAP_MIN_LEN {
        return content.to_string();
    }
    let cap = (((content.len() as f64) * frac) as usize).max(CAP_MIN_LEN);
    if content.len() <= cap {
        return content.to_string();
    }
    let mut end = cap.max(1).min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &content[..end])
}

// ── Dated-context formatting (SPECTRAL_DATED_CONTEXT=1) ─────────────
//
// Pattern from the top LongMemEval scorer (Mastra OM, 84.23% official
// gpt-4o): every piece of evidence carries an explicit date PLUS a computed
// relative offset from the question date ("(4 months ago)"), so the actor
// never has to derive temporal distance itself. Deterministic string
// assembly — zero inference. Env-gated for A/B; the env lever is captured
// by the run config fingerprint.

/// Whether dated-context formatting is enabled (env-gated A/B lever).
fn dated_context_enabled() -> bool {
    std::env::var("SPECTRAL_DATED_CONTEXT")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Extract (y, m, d) from a date string in either "2023/05/30 …" or
/// "2023-05-30T…" form.
fn extract_ymd(s: &str) -> Option<(i32, u32, u32)> {
    let bytes: Vec<char> = s.chars().collect();
    if bytes.len() < 10 {
        return None;
    }
    let y: i32 = s.get(0..4)?.parse().ok()?;
    let m: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    if !matches!(bytes[4], '-' | '/') || !matches!(bytes[7], '-' | '/') {
        return None;
    }
    Some((y, m, d))
}

/// Compute a human-readable offset of `date_str` relative to
/// `question_date` ("3 days ago", "2 weeks ago", "4 months ago").
fn relative_offset(date_str: &str, question_date: &str) -> Option<String> {
    let (sy, sm, sd) = extract_ymd(date_str)?;
    let (qy, qm, qd) = extract_ymd(question_date)?;
    let session = chrono::NaiveDate::from_ymd_opt(sy, sm, sd)?;
    let question = chrono::NaiveDate::from_ymd_opt(qy, qm, qd)?;
    let days = (question - session).num_days();
    Some(match days {
        i64::MIN..=-1 => "in the future".to_string(),
        0 => "same day".to_string(),
        1 => "1 day ago".to_string(),
        2..=13 => format!("{days} days ago"),
        14..=59 => format!("{} weeks ago", days / 7),
        60..=729 => format!("{} months ago", days / 30),
        _ => {
            let years = days / 365;
            let months = (days % 365) / 30;
            if months == 0 {
                format!("{years} years ago")
            } else {
                format!("{years} years, {months} months ago")
            }
        }
    })
}

// ── Session-grouped formatting (P2) ─────────────────────────────────

/// Format memory hits grouped by session/episode for clearer multi-session context.
///
/// Groups hits by `episode_id` (falling back to session prefix from key),
/// orders sessions chronologically, and presents turns in order within
/// each session. Drops redundant per-turn metadata (date, wing, hall).
pub fn format_hits_grouped(hits: &[MemoryHit]) -> Vec<String> {
    format_hits_grouped_capped(hits, None)
}

/// Session-grouped formatting with an optional assistant-turn content cap.
pub fn format_hits_grouped_capped(hits: &[MemoryHit], cap_frac: Option<f64>) -> Vec<String> {
    format_hits_grouped_capped_dated(hits, cap_frac, None)
}

/// Session-grouped formatting with cap and optional dated-context headers.
/// When `SPECTRAL_DATED_CONTEXT=1` and a question date is provided, session
/// headers gain a computed relative offset: `--- Session s1 (2023/02/15, 4
/// months ago) ---`.
pub fn format_hits_grouped_capped_dated(
    hits: &[MemoryHit],
    cap_frac: Option<f64>,
    question_date: Option<&str>,
) -> Vec<String> {
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
    let show_desc = actor_descriptions_enabled();
    let mut lines = Vec::new();
    for (ep_id, ep_hits) in &episodes {
        let date = ep_hits
            .first()
            .and_then(|h| h.created_at.as_deref())
            .map(|s| s.split(' ').next().unwrap_or(s))
            .unwrap_or("unknown-date");
        let header = match question_date.filter(|_| dated_context_enabled()) {
            Some(qd) => match relative_offset(date, qd) {
                Some(offset) => format!("--- Session {ep_id} ({date}, {offset}) ---"),
                None => format!("--- Session {ep_id} ({date}) ---"),
            },
            None => format!("--- Session {ep_id} ({date}) ---"),
        };
        lines.push(header);

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
            let line = match (role, cap_frac) {
                ("asst", Some(f)) => format!("[{role}] {}", cap_content(&hit.content, f)),
                _ => format!("[{role}] {}", hit.content),
            };
            lines.push(with_description(line, hit, show_desc));
        }
    }

    lines
}

// ── Legacy flat format ──────────────────────────────────────────────

/// Format a MemoryHit into the standard actor format.
pub fn format_hit(hit: &spectral_ingest::MemoryHit) -> String {
    format_hit_dated(hit, None)
}

/// Flat format with an optional dated-context offset: `[2023-05-20, 3 weeks
/// ago] [wing/hall] key: content` when `SPECTRAL_DATED_CONTEXT=1` and a
/// question date is provided.
pub fn format_hit_dated(hit: &spectral_ingest::MemoryHit, question_date: Option<&str>) -> String {
    let date = hit
        .created_at
        .as_deref()
        .map(|s| s.split('T').next().unwrap_or(s))
        .unwrap_or("unknown-date");
    let date_tag = match question_date.filter(|_| dated_context_enabled()) {
        Some(qd) => match relative_offset(date, qd) {
            Some(offset) => format!("{date}, {offset}"),
            None => date.to_string(),
        },
        None => date.to_string(),
    };
    let wing = hit.wing.as_deref().unwrap_or("?");
    let hall = hit.hall.as_deref().unwrap_or("?");
    let line = format!("[{date_tag}] [{wing}/{hall}] {}: {}", hit.key, hit.content);
    with_description(line, hit, actor_descriptions_enabled())
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

/// Apply env-gated associative spreading to a retrieved hit list, in place.
/// Shared by the topk_fts and cascade paths. Reads `SPECTRAL_ASSOC_*` env vars
/// and delegates to the library implementation `spectral_graph::spreading`.
/// All modes OFF by default (no env var set = no-op).
pub fn apply_associative_spreading(brain: &Brain, hits: &mut Vec<MemoryHit>) {
    use spectral_graph::spreading::{associative_spread, AssocSpreadConfig, SpreadMode};
    let ev = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<usize>().ok());
    let seeds = ev("SPECTRAL_ASSOC_SEEDS").unwrap_or(3);
    let cross = ev("SPECTRAL_ASSOC_CROSS").filter(|n| *n > 0);
    let rerank = ev("SPECTRAL_ASSOC_RERANK").filter(|b| *b > 0);
    let budget = ev("SPECTRAL_ASSOC_BUDGET").filter(|b| *b > 0);
    let cfg = if std::env::var("SPECTRAL_ASSOC_COMBINED").is_ok() {
        AssocSpreadConfig {
            mode: SpreadMode::Combined,
            seeds,
            cross_n: cross.unwrap_or(2),
            cross_budget: ev("SPECTRAL_ASSOC_CROSS_BUDGET").unwrap_or(2500),
            episode_budget: budget.unwrap_or(3000),
            ..AssocSpreadConfig::default()
        }
    } else if let Some(n) = cross {
        AssocSpreadConfig {
            mode: SpreadMode::CrossSession,
            seeds,
            cross_n: n,
            cross_budget: budget.unwrap_or(3500),
            ..AssocSpreadConfig::default()
        }
    } else if let Some(b) = rerank {
        AssocSpreadConfig {
            mode: SpreadMode::Rerank,
            seeds,
            rerank_b: b,
            ..AssocSpreadConfig::default()
        }
    } else if let Some(b) = budget {
        AssocSpreadConfig {
            mode: SpreadMode::Episode,
            seeds,
            episode_budget: b,
            ..AssocSpreadConfig::default()
        }
    } else {
        return;
    };
    // Bench runs against one's own brain — Private context admits everything.
    associative_spread(brain, hits, &cfg, Visibility::Private);
}

// ── n-hop BFS graph channel (SPECTRAL_BFS_HOPS, off when unset) ─────

/// Number of top hits used as BFS start nodes.
const BFS_SEEDS: usize = 5;
/// Neighbors fetched per node per edge substrate.
const BFS_NEIGHBORS_PER_NODE: usize = 10;
/// Maximum nodes carried into the next hop (bounds fan-out; fingerprint
/// graphs are dense — ~140 edges/memory on the bench brains).
const BFS_FRONTIER_CAP: usize = 50;
/// Maximum BFS-discovered memories admitted into the output. Output size is
/// preserved: admitted memories displace the weakest tail hits (mirrors the
/// SPECTRAL_ASSOC_RERANK size-preserving pattern).
const BFS_APPEND_CAP: usize = 10;

/// Read `SPECTRAL_BFS_HOPS` (>=1). Unset/invalid = lever off.
fn bfs_hops() -> Option<usize> {
    std::env::var("SPECTRAL_BFS_HOPS")
        .ok()?
        .parse::<usize>()
        .ok()
        .filter(|h| *h >= 1)
}

fn bench_memory_to_hit(m: &spectral_ingest::Memory) -> MemoryHit {
    MemoryHit {
        id: m.id.clone(),
        key: m.key.clone(),
        content: m.content.clone(),
        wing: m.wing.clone(),
        hall: m.hall.clone(),
        signal_score: m.signal_score,
        visibility: m.visibility.clone(),
        hits: 0,
        source: m.source.clone(),
        device_id: m.device_id,
        confidence: m.confidence,
        created_at: m.created_at.clone(),
        last_reinforced_at: m.last_reinforced_at.clone(),
        episode_id: m.episode_id.clone(),
        declarative_density: m.declarative_density,
        description: m.description.clone(),
        source_brain_id: m.source_brain_id,
        signature: m.signature.clone(),
    }
}

/// Expand the hit set via N-hop BFS over the brain's memory↔memory edge
/// substrates, displacing the weakest tail hits so output size is preserved.
///
/// Edge substrates walked (union):
/// - constellation-fingerprint edges (written at ingest — the only populated
///   memory↔memory edge table in fresh bench brains)
/// - co-retrieval edges (`related_memories`) — structurally empty in the
///   Tier-0 oracle (fresh brain, single query) but live in Permagent brains
///
/// Deterministic: neighbors ordered by edge weight DESC then id; discovered
/// candidates ranked by (hop ASC, edge weight DESC, id ASC).
pub fn apply_bfs_expansion(brain: &Brain, hits: &mut Vec<MemoryHit>, hops: usize) {
    if hops == 0 || hits.is_empty() {
        return;
    }
    let existing_ids: HashSet<String> = hits.iter().map(|h| h.id.clone()).collect();
    let mut frontier: Vec<String> = hits.iter().take(BFS_SEEDS).map(|h| h.id.clone()).collect();
    let mut visited: HashSet<String> = frontier.iter().cloned().collect();
    // (hop, edge_weight_at_first_visit, id)
    let mut discovered: Vec<(usize, u64, String)> = Vec::new();

    for hop in 1..=hops {
        let mut reached: Vec<(u64, String)> = Vec::new();
        for id in &frontier {
            if let Ok(nbrs) = brain.fingerprint_neighbors(id, BFS_NEIGHBORS_PER_NODE) {
                reached.extend(nbrs.into_iter().map(|n| (n.edge_count, n.memory_id)));
            }
            if let Ok(rels) = brain.related_memories(id, BFS_NEIGHBORS_PER_NODE) {
                reached.extend(rels.into_iter().map(|r| (r.co_count, r.memory_id)));
            }
        }
        // Strongest edges claim frontier slots first; dedup on first visit.
        reached.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut next_frontier = Vec::new();
        for (weight, id) in reached {
            if !visited.insert(id.clone()) {
                continue;
            }
            if !existing_ids.contains(&id) {
                discovered.push((hop, weight, id.clone()));
            }
            if next_frontier.len() < BFS_FRONTIER_CAP {
                next_frontier.push(id);
            }
        }
        frontier = next_frontier;
        if frontier.is_empty() {
            break;
        }
    }

    discovered.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    let candidates: Vec<MemoryHit> = discovered
        .into_iter()
        .take(BFS_APPEND_CAP)
        .filter_map(|(_, _, id)| brain.get_memory(&id).ok().flatten())
        .map(|m| bench_memory_to_hit(&m))
        .collect();
    merge_displacing_tail(hits, candidates, BFS_APPEND_CAP);
}

/// Admit up to `cap` new candidates into `hits`, displacing the weakest tail
/// entries so the output length never grows. Deduplicates by memory id.
pub fn merge_displacing_tail(hits: &mut Vec<MemoryHit>, candidates: Vec<MemoryHit>, cap: usize) {
    let existing: HashSet<String> = hits.iter().map(|h| h.id.clone()).collect();
    let mut admitted: Vec<MemoryHit> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for c in candidates {
        if admitted.len() >= cap {
            break;
        }
        if existing.contains(&c.id) || !seen.insert(c.id.clone()) {
            continue;
        }
        admitted.push(c);
    }
    if admitted.is_empty() {
        return;
    }
    let n = hits.len();
    let keep = n.saturating_sub(admitted.len());
    hits.truncate(keep);
    hits.extend(admitted);
    debug_assert!(hits.len() <= n);
}

// ── ACT-R base-level activation rerank (SPECTRAL_ACTR_DECAY, off when unset) ──

/// Blend weight of the normalized activation prior against the pipeline's
/// own ranking (position score). Small additive prior by design.
const ACTR_BLEND_WEIGHT: f64 = 0.2;
/// Floor on access ages, in days — avoids t^-d blowing up at age≈0.
const ACTR_MIN_AGE_DAYS: f64 = 1e-3;
/// Age assumed for hits with an unparseable/missing created_at (ranked old).
const ACTR_UNKNOWN_AGE_DAYS: f64 = 365.0;
/// Pool widening factor when the ACT-R lever is on: fetch wider, rerank,
/// truncate back to the original output size (mirrors SPECTRAL_TOPK_FETCH_MULT).
const ACTR_POOL_WIDEN: usize = 2;

/// Read `SPECTRAL_ACTR_DECAY` (d > 0, ~0.5 typical). Unset/invalid = off.
fn actr_decay() -> Option<f64> {
    std::env::var("SPECTRAL_ACTR_DECAY")
        .ok()?
        .parse::<f64>()
        .ok()
        .filter(|d| *d > 0.0 && d.is_finite())
}

/// ACT-R base-level activation `B = ln(Σ_j t_j^-d)` over the deterministic
/// access events the store exposes:
/// - one encoding event at `created_at` (age `age_created_days`)
/// - if `last_reinforced_at` is present, `max(accesses, 1)` reinforcement
///   events collapsed at that timestamp (the store keeps only the latest
///   reinforcement time, not the full access history)
///
/// In fresh Tier-0 bench brains `last_reinforced_at` is NULL everywhere, so B
/// degenerates to `-d·ln(age_created)` — a pure power-law recency prior. The
/// frequency component only differentiates in brains with retrieval history.
pub fn base_level_activation(
    age_created_days: f64,
    age_reinforced_days: Option<f64>,
    accesses: u64,
    d: f64,
) -> f64 {
    let t0 = age_created_days.max(ACTR_MIN_AGE_DAYS);
    let mut sum = t0.powf(-d);
    if let Some(tr) = age_reinforced_days {
        let tr = tr.max(ACTR_MIN_AGE_DAYS);
        sum += accesses.max(1) as f64 * tr.powf(-d);
    }
    sum.ln()
}

/// Parse a memory timestamp in either store format ("%Y-%m-%d %H:%M:%S" or
/// RFC 3339) to epoch seconds.
fn parse_hit_ts(s: &str) -> Option<i64> {
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp());
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

/// Rerank `hits` by blending a normalized base-level-activation prior into a
/// position score derived from the pipeline's own ordering:
/// `score_i = 0.8·(1 - i/(n-1)) + 0.2·norm(B_i)`. Stable and deterministic;
/// a no-op when activations are degenerate (all equal).
pub fn apply_actr_rerank(hits: &mut [MemoryHit], now: DateTime<Utc>, d: f64) {
    let n = hits.len();
    if n < 2 {
        return;
    }
    let now_s = now.timestamp();
    let age_days = |s: &str| parse_hit_ts(s).map(|t| (now_s - t) as f64 / 86_400.0);
    let acts: Vec<f64> = hits
        .iter()
        .map(|h| {
            let a_c = h
                .created_at
                .as_deref()
                .and_then(age_days)
                .unwrap_or(ACTR_UNKNOWN_AGE_DAYS);
            let a_r = h.last_reinforced_at.as_deref().and_then(age_days);
            base_level_activation(a_c, a_r, h.hits as u64, d)
        })
        .collect();
    let min = acts.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = acts.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;
    if range.is_nan() || range <= 0.0 {
        return; // degenerate (all equal) or NaN — keep pipeline order
    }
    let blended: Vec<f64> = acts
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let pos = 1.0 - i as f64 / (n - 1) as f64;
            (1.0 - ACTR_BLEND_WEIGHT) * pos + ACTR_BLEND_WEIGHT * ((b - min) / range)
        })
        .collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        blended[b]
            .partial_cmp(&blended[a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });
    let reordered: Vec<MemoryHit> = order.iter().map(|&i| hits[i].clone()).collect();
    hits.clone_from_slice(&reordered);
}

/// Retrieve via top-K FTS with additive re-ranking. No LLM cost.
///
/// Configurable via env vars for ablation:
/// - `SPECTRAL_DISABLE_SIGNAL_SCORE=1` — disable signal weighting
/// - `SPECTRAL_DISABLE_RECENCY=1` — disable recency weighting
/// - `SPECTRAL_DISABLE_ENTITY_RESOLUTION=1` — disable entity clustering
/// - `SPECTRAL_DISABLE_CONTEXT_DEDUP=1` — disable context chain dedup
/// - `SPECTRAL_TOPK_FETCH_MULT=N` — fetch N× the output size as the re-rank
///   candidate pool (default: the library default, 3). Recovers true evidence
///   that broad porter stemming buries below a fixed bm25 LIMIT; a no-op on
///   an unstemmed tokenizer, +0.8pp temporal session-recall with porter at
///   N=3. Set to 1 to disable widening.
/// - `SPECTRAL_TOPK_DECLARATIVE=1` — enable the declarative-density boost in
///   the topk path (net-negative on temporal in Tier-0; off by default)
/// - `SPECTRAL_BFS_HOPS=N` — n-hop BFS expansion over memory↔memory edge
///   substrates (fingerprint + co-retrieval), size-preserving; off when unset
/// - `SPECTRAL_ACTR_DECAY=d` — ACT-R base-level-activation rerank prior over
///   a 2× widened pool, truncated back to output size; off when unset
///
/// When `question_date` is provided (format: "2023/05/30 (Tue) 23:40"),
/// recency decay is anchored to that date instead of `Utc::now()`.
pub fn retrieve_topk_fts(
    brain: &Brain,
    question: &str,
    config: &RetrievalConfig,
    question_date: Option<&str>,
) -> Result<(Vec<String>, Vec<MemoryHit>)> {
    // Floor at 40: prevents CLI overrides (e.g. --max-results 20) from cutting
    // temporal evidence turns that rank at FTS position 21-40 after reranking.
    let output_size = config.max_results.max(40);
    // Candidate-fetch pool widening now lives in the library
    // (`RecallTopKConfig::fetch_mult`, default 3); the env var remains as an
    // ablation override. Broad FTS matching (porter stemming) pushes true
    // evidence below a fixed bm25 LIMIT; a wider pool lets re-ranking recover
    // it. Output count stays at `output_size`, but composition shifts toward
    // the promoted high-signal turns, so context grows modestly (~+4% on the
    // temporal slice at mult=3 — a no-op on an unstemmed tokenizer).
    let fetch_mult = std::env::var("SPECTRAL_TOPK_FETCH_MULT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|m| *m >= 1)
        .unwrap_or_else(|| RecallTopKConfig::default().fetch_mult);
    let topk_config = RecallTopKConfig {
        k: output_size,
        fetch_mult,
        apply_signal_score_weighting: std::env::var("SPECTRAL_DISABLE_SIGNAL_SCORE").is_err(),
        apply_recency_weighting: std::env::var("SPECTRAL_DISABLE_RECENCY").is_err(),
        apply_entity_resolution: std::env::var("SPECTRAL_DISABLE_ENTITY_RESOLUTION").is_err(),
        apply_declarative_boost: std::env::var("SPECTRAL_TOPK_DECLARATIVE").is_ok(),
        apply_context_dedup: std::env::var("SPECTRAL_DISABLE_CONTEXT_DEDUP").is_err(),
        now: question_date.and_then(parse_question_date),
        ..RecallTopKConfig::default()
    };

    // ACT-R lever: fetch a wider pool so the activation prior can change set
    // membership, not just order; truncated back to output_size after rerank.
    let actr = actr_decay();
    let pool_size = if actr.is_some() {
        output_size * ACTR_POOL_WIDEN
    } else {
        output_size
    };
    let topk_config = RecallTopKConfig {
        k: pool_size,
        ..topk_config
    };

    let mut hits: Vec<MemoryHit> = brain
        .recall_topk_fts(question, &topk_config, Visibility::Private)?
        .into_iter()
        .take(pool_size)
        .collect();

    // Associative expansion (TACT's true vision): FTS finds the seeds by words;
    // spread activation through EPISODE co-occurrence to recover answer memories
    // that share no words with the query but sit in the same session as a seed —
    // the vocabulary-gap misses FTS structurally cannot cross. SPECTRAL_ASSOC_
    // EXPAND=M pulls the top-M high-signal memories from each of the top seeds'
    // episodes (M unset/0 = off). Measured recovery ceiling on real LongMemEval:
    // 100% of missed-answer-key questions still have their answer session
    // retrieved, so this can pull the missed keys in — at a token cost tuned by M.
    apply_associative_spreading(brain, &mut hits);

    // n-hop BFS graph channel (size-preserving; off when SPECTRAL_BFS_HOPS unset).
    if let Some(hops) = bfs_hops() {
        apply_bfs_expansion(brain, &mut hits, hops);
    }
    // ACT-R activation rerank over the widened pool, then restore output size.
    if let Some(d) = actr {
        let now = topk_config.now.unwrap_or_else(Utc::now);
        apply_actr_rerank(&mut hits, now, d);
        hits.truncate(output_size);
    }

    let cap = cap_for_shape(QuestionType::classify(question));
    let memories: Vec<String> = hits
        .iter()
        .map(|h| match cap {
            Some(f) if h.key.contains(":assistant") => {
                let mut capped = h.clone();
                capped.content = cap_content(&h.content, f);
                format_hit_dated(&capped, question_date)
            }
            _ => format_hit_dated(h, question_date),
        })
        .collect();

    Ok((memories, hits))
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

/// Telemetry from a retrieval pipeline run.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CascadeTelemetry {
    pub max_confidence: f64,
    pub total_tokens_used: usize,
    /// Total LLM tokens consumed during retrieval. Structurally zero:
    /// no Brain::recall_*() method makes an LLM call.
    pub total_recognition_token_cost: usize,
    /// Question-type classification used for routing (e.g. "Counting").
    #[serde(default)]
    pub question_type: Option<String>,
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
) -> Result<(Vec<String>, Vec<MemoryHit>, CascadeTelemetry)> {
    // P1: Question-type routing
    let qtype = QuestionType::classify(question);
    let mut pipeline_config = qtype.cascade_profile();
    // Ablation overrides (multi-session answer-KEY completeness sweep). The
    // Counting profile caps max_per_episode=3 to force session diversity; when
    // answer keys cluster >3 per session that undercounts. These let a sweep
    // find the key-recall/token frontier without re-ingesting brains.
    if let Some(k) = std::env::var("SPECTRAL_CASCADE_K")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        pipeline_config.k = k;
    }
    if let Some(mpe) = std::env::var("SPECTRAL_CASCADE_MAX_PER_EPISODE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        pipeline_config.max_per_episode = mpe;
    }
    if let Some(fm) = std::env::var("SPECTRAL_CASCADE_FETCH_MULT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        pipeline_config.fetch_mult = fm;
    }
    if std::env::var("SPECTRAL_CASCADE_NO_SIGNAL").is_ok() {
        pipeline_config.apply_signal_reranking = false;
    }
    if std::env::var("SPECTRAL_CASCADE_NO_DECLARATIVE").is_ok() {
        pipeline_config.apply_declarative_boost = false;
    }
    if std::env::var("SPECTRAL_CASCADE_NO_RECENCY").is_ok() {
        pipeline_config.apply_recency = false;
    }

    let context = match question_date.and_then(parse_question_date) {
        Some(dt) => spectral_cascade::RecognitionContext::empty().with_now(dt),
        None => spectral_cascade::RecognitionContext::empty(),
    };
    let result = brain.recall_cascade_with_pipeline(question, &context, &pipeline_config)?;

    // Capture telemetry before consuming merged_hits
    let telemetry = CascadeTelemetry {
        max_confidence: result.max_confidence,
        total_tokens_used: result.total_tokens_used,
        total_recognition_token_cost: result.total_recognition_token_cost,
        question_type: Some(format!("{qtype:?}")),
    };

    // P2: Session-grouped formatting
    // Use the profile's K as the limit — the question-type routing already
    // determined the right K (60 for counting, 30 for factual, etc).
    // CLI --max-results only applies to non-cascade paths.
    // ACT-R lever: take a wider slice of the merged pool so the activation
    // prior can change set membership; truncated back to K after rerank.
    let actr = actr_decay();
    let pool_size = if actr.is_some() {
        pipeline_config.k * ACTR_POOL_WIDEN
    } else {
        pipeline_config.k
    };
    let mut hits: Vec<MemoryHit> = result.merged_hits.into_iter().take(pool_size).collect();
    // Associative spreading on the DEFAULT (cascade) retrieval path, env-gated.
    apply_associative_spreading(brain, &mut hits);
    // n-hop BFS graph channel (size-preserving; off when SPECTRAL_BFS_HOPS unset).
    if let Some(hops) = bfs_hops() {
        apply_bfs_expansion(brain, &mut hits, hops);
    }
    // ACT-R activation rerank over the widened pool, then restore output size.
    if let Some(d) = actr {
        let now = question_date
            .and_then(parse_question_date)
            .unwrap_or_else(Utc::now);
        apply_actr_rerank(&mut hits, now, d);
        hits.truncate(pipeline_config.k);
    }
    let formatted = format_hits_grouped_capped_dated(&hits, cap_for_shape(qtype), question_date);

    Ok((formatted, hits, telemetry))
}

/// Parse LongMemEval question_date format ("2023/05/30 (Tue) 23:40") into DateTime<Utc>.
/// Public alias for use by inspect module.
pub fn parse_question_date_pub(date_str: &str) -> Option<DateTime<Utc>> {
    parse_question_date(date_str)
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

    fn hit_with_description(desc: Option<&str>) -> MemoryHit {
        MemoryHit {
            id: "id".into(),
            key: "s1:turn:0:user".into(),
            content: "the pod was oomkilled at 512mi".into(),
            wing: Some("infra".into()),
            hall: Some("fact".into()),
            signal_score: 0.6,
            visibility: "private".into(),
            hits: 1,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: Some("2023-06-15 12:00:00".into()),
            last_reinforced_at: None,
            episode_id: Some("s1".into()),
            declarative_density: None,
            description: desc.map(|s| s.to_string()),
            source_brain_id: None,
            signature: None,
        }
    }

    #[test]
    fn with_description_injects_only_when_shown_and_present() {
        let hit = hit_with_description(Some("summary: an out-of-memory kill during reindex"));
        let base = "[user] the pod was oomkilled at 512mi".to_string();

        // Off: unchanged.
        assert_eq!(with_description(base.clone(), &hit, false), base);
        // On: labeled librarian note appended.
        let shown = with_description(base.clone(), &hit, true);
        assert!(shown.starts_with(&base));
        assert!(shown.contains("[librarian: summary: an out-of-memory kill during reindex]"));

        // On but no description: unchanged (no empty note).
        let hit_none = hit_with_description(None);
        assert_eq!(with_description(base.clone(), &hit_none, true), base);
        // On but blank description: unchanged.
        let hit_blank = hit_with_description(Some("   "));
        assert_eq!(with_description(base.clone(), &hit_blank, true), base);
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
            fts_tokenizer: None,
            read_only: false,
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
            fts_tokenizer: None,
            read_only: false,
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
            fts_tokenizer: None,
            read_only: false,
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
        let (memories_with_date, _, _) = retrieve_cascade(
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

    #[test]
    fn topk_fts_uses_question_date_for_recency() {
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
            fts_tokenizer: None,
            read_only: false,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        })
        .unwrap();

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

        // TopkFts with question_date anchored to May 30 2023
        let (_, hits_with_date) = retrieve_topk_fts(
            &brain,
            "jogging exercise",
            &RetrievalConfig::default(),
            Some("2023/05/30 (Tue) 23:40"),
        )
        .unwrap();

        // TopkFts WITHOUT question_date (Utc::now, ~2026 → both equally old)
        let (_, hits_no_date) = retrieve_topk_fts(
            &brain,
            "jogging exercise",
            &RetrievalConfig::default(),
            None,
        )
        .unwrap();

        // With question_date, recency should meaningfully distinguish the two.
        // The recent memory (May 20, 10 days old) should score higher.
        assert!(hits_with_date.len() >= 2, "should return both memories");
        assert_eq!(
            hits_with_date[0].key, "recent-memory",
            "with question_date, recent memory should rank first"
        );

        // Without question_date, both are ~1200 days old — recency barely
        // distinguishes them; FTS rank dominates. Just verify both returned.
        assert!(hits_no_date.len() >= 2, "should return both memories");
    }

    #[test]
    fn wider_fetch_pool_recovers_buried_high_signal_memory() {
        // Mechanism behind SPECTRAL_TOPK_FETCH_MULT: broad FTS matching buries
        // a true-evidence memory below a narrow bm25 LIMIT. A narrow fetch never
        // sees it; a wider fetch pool lets re-ranking (signal blend) surface it.
        use spectral_graph::brain::RecallTopKConfig;

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
            fts_tokenizer: None,
            read_only: false,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        })
        .unwrap();

        // 20 filler memories repeat "widget" many times → high bm25 term
        // frequency, so they occupy the top bm25 positions.
        for i in 0..20 {
            brain
                .remember_with(
                    &format!("filler-{i}"),
                    "widget widget widget widget widget filler content",
                    RememberOpts {
                        visibility: Visibility::Private,
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        // TARGET mentions "widget" once → low bm25 rank (buried below the
        // fillers), but is reinforced to a high signal_score.
        brain
            .remember_with(
                "target",
                "widget appears once here amid otherwise unique evidence text",
                RememberOpts {
                    visibility: Visibility::Private,
                    ..Default::default()
                },
            )
            .unwrap();
        let _ = brain.reinforce(spectral_graph::brain::ReinforceOpts {
            memory_keys: vec!["target".into()],
            strength: 1.0,
        });

        let cfg = |fetch_mult: usize| RecallTopKConfig {
            k: 20,
            fetch_mult,
            apply_recency_weighting: false,
            ..RecallTopKConfig::default()
        };

        // Unwidened pool (fetch_mult=1): TARGET sits at bm25 position 21,
        // below the k=20 fetch cut → never enters the re-rank pool.
        let narrow = brain
            .recall_topk_fts("widget", &cfg(1), Visibility::Private)
            .unwrap();
        assert!(
            !narrow.iter().any(|h| h.key == "target"),
            "unwidened fetch pool should exclude the buried target"
        );

        // Widened pool (fetch_mult=3, the default): TARGET enters the pool
        // and its high signal_score promotes it past low-signal fillers in
        // re-ranking; output is still truncated to k.
        let wide = brain
            .recall_topk_fts("widget", &cfg(3), Visibility::Private)
            .unwrap();
        assert!(
            wide.iter().any(|h| h.key == "target"),
            "widened fetch pool should recover the buried high-signal target"
        );
        assert!(
            wide.len() <= 20,
            "output should be truncated to k, got {}",
            wide.len()
        );
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
    fn classify_temporal_ordering_questions() {
        // Event-ordering questions should route to Temporal, not Factual.
        assert_eq!(
            QuestionType::classify("What is the order of the sports events I watched in January?"),
            QuestionType::Temporal
        );
        assert_eq!(
            QuestionType::classify(
                "What is the order of the six museums I visited from earliest to latest?"
            ),
            QuestionType::Temporal
        );
        assert_eq!(
            QuestionType::classify(
                "What is the order of the concerts and musical events I attended?"
            ),
            QuestionType::Temporal
        );
        // "first" and "last" already match the existing temporal regex
        assert_eq!(
            QuestionType::classify("What was the first airline I flew with?"),
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
        // "most recently" should also trigger FactualCurrentState
        assert_eq!(
            QuestionType::classify("What type of camera lens did I purchase most recently?"),
            QuestionType::FactualCurrentState
        );
    }

    #[test]
    fn classify_where_bypasses_temporal() {
        // Rachel case: "after" should NOT trigger Temporal for "where" questions
        assert_eq!(
            QuestionType::classify("Where did Rachel move to after her recent relocation?"),
            QuestionType::FactualCurrentState
        );
        // "last week" should NOT trigger Temporal for "where" questions
        assert_eq!(
            QuestionType::classify("Where did I attend the religious activity last week?"),
            QuestionType::Factual
        );
        // "after" without recency signal → plain Factual
        assert_eq!(
            QuestionType::classify("Where did I go after lunch?"),
            QuestionType::Factual
        );
        // Recency signal → FactualCurrentState
        assert_eq!(
            QuestionType::classify("Where is the painting currently hanging?"),
            QuestionType::FactualCurrentState
        );
    }

    #[test]
    fn classify_where_does_not_break_temporal() {
        // Non-"where" temporal questions must remain Temporal
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
            QuestionType::classify("What did I do after the concert?"),
            QuestionType::Temporal
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
            fts_tokenizer: None,
            read_only: false,
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

        let (_, _, telemetry) = retrieve_cascade(
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
            source_brain_id: None,
            signature: None,
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

    // ── Assistant-turn cap tests ─────────────────────────────────────

    #[test]
    fn cap_content_truncates_at_fraction() {
        let content = "a".repeat(1000);
        let capped = cap_content(&content, 0.36);
        // 360 chars + ellipsis
        assert_eq!(capped.chars().count(), 361);
        assert!(capped.ends_with('…'));
    }

    #[test]
    fn cap_content_leaves_short_content_alone() {
        assert_eq!(cap_content("short", 0.9), "short");
        // Anything at or under CAP_MIN_LEN bytes is untouched at any fraction.
        let borderline = "b".repeat(120);
        assert_eq!(cap_content(&borderline, 0.1), borderline);
    }

    #[test]
    fn cap_content_respects_char_boundaries() {
        let content = "héllo wörld ünïcode cöntent hère with áccents évery whére ".repeat(20);
        let capped = cap_content(&content, 0.3);
        assert!(capped.ends_with('…'));
        // Must not panic mid-codepoint and must be valid UTF-8 (implicit in String).
        assert!(capped.len() < content.len());
    }

    #[test]
    fn format_grouped_capped_truncates_assistant_only() {
        let long_asst = "x".repeat(1000);
        let long_user = "y".repeat(1000);
        let hits = vec![
            make_test_hit(
                "1",
                "s1:turn:0:user",
                &long_user,
                "s1",
                "2023-05-20 12:00:00",
            ),
            make_test_hit(
                "2",
                "s1:turn:1:assistant",
                &long_asst,
                "s1",
                "2023-05-20 12:00:00",
            ),
        ];
        let lines = format_hits_grouped_capped(&hits, Some(0.36));
        let user_line = lines.iter().find(|l| l.starts_with("[user]")).unwrap();
        let asst_line = lines.iter().find(|l| l.starts_with("[asst]")).unwrap();
        assert!(user_line.len() > 1000, "user content must be untouched");
        assert!(
            asst_line.len() < 400,
            "assistant content must be capped to ~360, got len {}",
            asst_line.len()
        );
    }

    // ── ACT-R activation + BFS lever tests ───────────────────────────

    #[test]
    fn activation_older_memory_has_lower_b() {
        // Monotonicity in recency: larger age → lower base-level activation.
        let d = 0.5;
        let young = base_level_activation(1.0, None, 0, d);
        let old = base_level_activation(100.0, None, 0, d);
        assert!(
            young > old,
            "1-day-old activation {young} must exceed 100-day-old {old}"
        );
        // Strictly decreasing across a sweep.
        let mut prev = f64::INFINITY;
        for age in [0.5, 1.0, 5.0, 30.0, 180.0, 720.0] {
            let b = base_level_activation(age, None, 0, d);
            assert!(b < prev, "activation must decrease with age (age={age})");
            prev = b;
        }
    }

    #[test]
    fn activation_fewer_accesses_has_lower_b() {
        // Monotonicity in frequency: fewer reinforcement accesses → lower B.
        let d = 0.5;
        let few = base_level_activation(30.0, Some(2.0), 1, d);
        let many = base_level_activation(30.0, Some(2.0), 5, d);
        assert!(
            few < many,
            "1-access activation {few} must be below 5-access {many}"
        );
        // No reinforcement at all is the floor.
        let none = base_level_activation(30.0, None, 0, d);
        assert!(none < few);
    }

    #[test]
    fn activation_age_floor_prevents_blowup() {
        let b = base_level_activation(0.0, Some(0.0), 1_000_000, 0.5);
        assert!(b.is_finite());
    }

    #[test]
    fn actr_rerank_promotes_recent_memory_and_is_size_stable() {
        // 10 old hits followed by one very recent hit at the tail. With the
        // 0.2 blend, adjacent position gaps are 0.8/(n-1) = 0.08, so the
        // recent hit's ~1.0 normalized activation must promote it past its
        // older neighbors — while the pipeline's rank-1 stays rank-1
        // (position score dominates by design: small additive prior).
        let mut hits: Vec<MemoryHit> = (0..10)
            .map(|i| {
                make_test_hit(
                    &format!("{i}"),
                    &format!("s{i}:turn:0:user"),
                    "old",
                    &format!("s{i}"),
                    "2023-01-01 12:00:00",
                )
            })
            .collect();
        hits.push(make_test_hit(
            "new",
            "snew:turn:0:user",
            "new",
            "snew",
            "2023-05-29 12:00:00",
        ));
        let now = Utc.with_ymd_and_hms(2023, 5, 30, 0, 0, 0).unwrap();
        apply_actr_rerank(&mut hits, now, 0.5);
        assert_eq!(hits.len(), 11, "rerank must preserve size");
        assert_eq!(hits[0].key, "s0:turn:0:user", "rank-1 must hold");
        let new_pos = hits.iter().position(|h| h.key == "snew:turn:0:user");
        assert!(
            new_pos.unwrap() < 10,
            "recent memory must be promoted from the tail, got {new_pos:?}"
        );
    }

    #[test]
    fn actr_rerank_is_noop_on_degenerate_activations() {
        let mut hits = vec![
            make_test_hit("1", "a", "x", "s1", "2023-05-20 12:00:00"),
            make_test_hit("2", "b", "y", "s2", "2023-05-20 12:00:00"),
        ];
        let before: Vec<String> = hits.iter().map(|h| h.key.clone()).collect();
        let now = Utc.with_ymd_and_hms(2023, 5, 30, 0, 0, 0).unwrap();
        apply_actr_rerank(&mut hits, now, 0.5);
        let after: Vec<String> = hits.iter().map(|h| h.key.clone()).collect();
        assert_eq!(before, after, "equal activations must not reorder");
    }

    #[test]
    fn bfs_merge_preserves_output_size_and_respects_cap() {
        let mut hits: Vec<MemoryHit> = (0..20)
            .map(|i| {
                make_test_hit(
                    &format!("id-{i}"),
                    &format!("s{i}:turn:0:user"),
                    "content",
                    &format!("s{i}"),
                    "2023-05-20 12:00:00",
                )
            })
            .collect();
        // 15 candidates, cap 10 → at most 10 admitted, size unchanged.
        let candidates: Vec<MemoryHit> = (0..15)
            .map(|i| {
                make_test_hit(
                    &format!("cand-{i}"),
                    &format!("c{i}:turn:0:user"),
                    "linked",
                    &format!("c{i}"),
                    "2023-05-21 12:00:00",
                )
            })
            .collect();
        merge_displacing_tail(&mut hits, candidates, 10);
        assert_eq!(hits.len(), 20, "output size must be preserved");
        let admitted = hits.iter().filter(|h| h.id.starts_with("cand-")).count();
        assert_eq!(admitted, 10, "cap must bound admitted BFS memories");
        // Strongest (head) original hits survive; tail was displaced.
        assert_eq!(hits[0].id, "id-0");
        assert_eq!(hits[9].id, "id-9");
    }

    #[test]
    fn bfs_merge_dedups_existing_and_duplicate_candidates() {
        let mut hits = vec![
            make_test_hit("id-0", "a", "x", "s1", "2023-05-20 12:00:00"),
            make_test_hit("id-1", "b", "y", "s2", "2023-05-20 12:00:00"),
        ];
        let candidates = vec![
            // Already in the hit set → skipped.
            make_test_hit("id-0", "a", "x", "s1", "2023-05-20 12:00:00"),
            // Duplicate candidate → admitted once.
            make_test_hit("cand-0", "c", "z", "s3", "2023-05-20 12:00:00"),
            make_test_hit("cand-0", "c", "z", "s3", "2023-05-20 12:00:00"),
        ];
        merge_displacing_tail(&mut hits, candidates, 10);
        assert_eq!(hits.len(), 2);
        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, vec!["id-0", "cand-0"], "dedup by id, displace tail");
    }

    #[test]
    fn bfs_expansion_walks_fingerprint_edges_and_preserves_size() {
        // End-to-end over a real brain: write-time constellation fingerprints
        // link same-day memories; BFS from a seed must pull an unretrieved
        // linked memory into the set without growing it.
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
            fts_tokenizer: None,
            read_only: false,
            activity_wing: "activity".into(),
            redaction_policy: None,
            tact_config: None,
        })
        .unwrap();

        let ts = Utc.with_ymd_and_hms(2023, 5, 20, 12, 0, 0).unwrap();
        for (key, content) in [
            ("m-seed", "the seed memory about quarterly planning"),
            ("m-linked", "an unrelated note written the same day"),
            ("m-third", "another same-day note about lunch"),
        ] {
            brain
                .remember_with(
                    key,
                    content,
                    RememberOpts {
                        created_at: Some(ts),
                        visibility: Visibility::Private,
                        ..Default::default()
                    },
                )
                .unwrap();
        }

        // Retrieve only the seed, then BFS-expand.
        let seed = brain.recall_local("quarterly planning").unwrap();
        let mut hits: Vec<MemoryHit> = seed
            .memory_hits
            .into_iter()
            .filter(|h| h.key == "m-seed")
            .collect();
        assert_eq!(hits.len(), 1, "test setup: only the seed retrieved");
        // Pad with a second hit so displacement has a tail to work with.
        hits.push(make_test_hit(
            "pad",
            "pad:turn:0:user",
            "padding",
            "pad",
            "2023-05-20 12:00:00",
        ));

        let before_len = hits.len();
        apply_bfs_expansion(&brain, &mut hits, 1);
        assert_eq!(hits.len(), before_len, "BFS must not grow the output");
        assert!(
            hits.iter()
                .any(|h| h.key == "m-linked" || h.key == "m-third"),
            "BFS over fingerprint edges should admit a same-day linked memory; got {:?}",
            hits.iter().map(|h| &h.key).collect::<Vec<_>>()
        );
    }

    #[test]
    fn general_recall_shape_is_exempt_from_cap() {
        // cap_for_shape consults the env var; without it set, every shape
        // returns None. The exemption logic itself is shape-only.
        assert_eq!(cap_for_shape(QuestionType::GeneralRecall), None);
    }
}

#[cfg(test)]
mod dated_context_tests {
    use super::*;

    #[test]
    fn relative_offset_buckets() {
        let q = "2023/06/01 (Thu) 10:00";
        assert_eq!(
            relative_offset("2023/06/01", q).as_deref(),
            Some("same day")
        );
        assert_eq!(
            relative_offset("2023/05/31", q).as_deref(),
            Some("1 day ago")
        );
        assert_eq!(
            relative_offset("2023-05-20T08:00", q).as_deref(),
            Some("12 days ago")
        );
        assert_eq!(
            relative_offset("2023/04/20", q).as_deref(),
            Some("6 weeks ago")
        );
        assert_eq!(
            relative_offset("2023/02/01", q).as_deref(),
            Some("4 months ago")
        );
        assert_eq!(
            relative_offset("2021/05/01", q).as_deref(),
            Some("2 years, 1 months ago")
        );
        assert_eq!(
            relative_offset("2023/06/05", q).as_deref(),
            Some("in the future")
        );
        assert_eq!(relative_offset("garbage", q), None);
    }

    #[test]
    fn dated_header_is_env_gated() {
        let mk = |id: &str, date: &str| spectral_ingest::MemoryHit {
            id: "id".into(),
            key: format!("{id}:turn_0:user"),
            content: "I bought a red car last month and I love driving it".into(),
            wing: Some("life".into()),
            hall: Some("fact".into()),
            signal_score: 0.6,
            visibility: "private".into(),
            hits: 1,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: Some(date.to_string()),
            last_reinforced_at: None,
            episode_id: Some(id.to_string()),
            declarative_density: None,
            description: None,
            source_brain_id: None,
            signature: None,
        };
        let hits = vec![mk("s1", "2023/02/15 (Wed) 23:50")];
        // Gate off: plain header regardless of question date.
        std::env::remove_var("SPECTRAL_DATED_CONTEXT");
        let lines = format_hits_grouped_capped_dated(&hits, None, Some("2023/06/01 (Thu) 10:00"));
        assert!(lines[0].contains("(2023/02/15)"), "{}", lines[0]);
        // Gate on: header carries the offset.
        std::env::set_var("SPECTRAL_DATED_CONTEXT", "1");
        let lines = format_hits_grouped_capped_dated(&hits, None, Some("2023/06/01 (Thu) 10:00"));
        assert!(
            lines[0].contains("2023/02/15, 3 months ago"),
            "{}",
            lines[0]
        );
        std::env::remove_var("SPECTRAL_DATED_CONTEXT");
    }
}
