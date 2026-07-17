//! Associative recall spreading — TACT's realized vision as a library feature.
//!
//! Full-text search finds seeds by words; spreading activation then follows
//! co-occurrence links to recover associated memories that share NO words with
//! the query (the vocabulary gap FTS cannot cross). Two substrates:
//!
//! - **episode** (same-session co-occurrence): completes an already-found
//!   session — memories near the seed in the same conversation.
//! - **cross-session** (pseudo-relevance feedback): each seed's own content is
//!   used as a query, so BM25 IDF surfaces its distinctive tokens and reaches
//!   associated memories in OTHER sessions — finds contributing sessions the
//!   query alone missed.
//!
//! Deterministic, local, embedding-free. Proximity within a session is ranked by
//! `created_at` closeness (general to any ingest). OFF by default; opt-in via
//! [`AssocSpreadConfig`]. Measured on real LongMemEval (bench): recovers +16–30pp
//! answer-key recall depending on mode; the accuracy payoff is workload-dependent
//! (helps most where a weaker actor cannot compensate for a missing memory).

use std::collections::{HashMap, HashSet};

use spectral_core::visibility::Visibility;
use spectral_ingest::{Memory, MemoryHit};

use crate::brain::{Brain, RecallTopKConfig};

/// Which spreading strategy to apply. `Off` is a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpreadMode {
    /// No spreading.
    Off,
    /// Episode (same-session) proximity fill from the top seeds.
    Episode,
    /// Cross-session pseudo-relevance feedback only.
    CrossSession,
    /// Cross-session to find missed sessions, then episode to complete each.
    Combined,
    /// Precision-preserving: displace the weakest results with proximity mates,
    /// keeping context size constant. Never removes a session's sole memory.
    Rerank,
}

/// Configuration for [`associative_spread`]. Defaults to `Off`.
#[derive(Debug, Clone)]
pub struct AssocSpreadConfig {
    pub mode: SpreadMode,
    /// Number of top hits to spread from.
    pub seeds: usize,
    /// Token budget for episode (same-session) expansion.
    pub episode_budget: usize,
    /// Cross-session mates fetched per seed.
    pub cross_n: usize,
    /// Token budget for cross-session expansion.
    pub cross_budget: usize,
    /// Number of results to displace in `Rerank` mode.
    pub rerank_b: usize,
}

impl Default for AssocSpreadConfig {
    fn default() -> Self {
        Self {
            mode: SpreadMode::Off,
            seeds: 3,
            episode_budget: 3000,
            cross_n: 2,
            cross_budget: 2500,
            rerank_b: 15,
        }
    }
}

impl AssocSpreadConfig {
    /// Precision-preserving preset (`Rerank`, session-safe): recovers answer
    /// keys by displacing the weakest results with proximity mates, keeping
    /// context size ~constant. Measured +16–23pp answer-key recall at ~constant
    /// tokens with no distraction tax. Best where the actor is strong / recall is
    /// near-ceiling — the safest default to try first.
    pub fn precision() -> Self {
        Self {
            mode: SpreadMode::Rerank,
            rerank_b: 15,
            ..Self::default()
        }
    }

    /// Recall-completeness preset (`Combined`): cross-session spreading finds
    /// missed contributing sessions, then episode spreading completes each.
    /// Measured +21–30pp answer-key recall (grows context ~20–30%). Best where
    /// recall genuinely gates the answer — multi-session counting, weaker/cheaper
    /// actors that cannot compensate for a missing memory.
    pub fn completeness() -> Self {
        Self {
            mode: SpreadMode::Combined,
            cross_n: 3,
            cross_budget: 2500,
            episode_budget: 3000,
            ..Self::default()
        }
    }
}

/// Apply associative spreading to a retrieved hit list, in place.
///
/// `visibility` is the caller's boundary: spread only surfaces memories whose own
/// label admits it (`content >= context`), so spreading cannot re-introduce
/// content the recall's visibility filter already excluded. Pass
/// `Visibility::Private` for own-brain recall (admits everything).
pub fn associative_spread(
    brain: &Brain,
    hits: &mut Vec<MemoryHit>,
    cfg: &AssocSpreadConfig,
    visibility: Visibility,
) {
    match cfg.mode {
        SpreadMode::Off => {}
        SpreadMode::Episode => {
            let seed_refs = top_seed_refs(hits, cfg.seeds);
            episode_fill(brain, hits, &seed_refs, cfg.episode_budget, visibility);
        }
        SpreadMode::CrossSession => {
            cross_session(
                brain,
                hits,
                cfg.seeds,
                cfg.cross_n,
                cfg.cross_budget,
                visibility,
            );
        }
        SpreadMode::Combined => {
            let mut seed_refs = top_seed_refs(hits, cfg.seeds);
            let added = cross_session(
                brain,
                hits,
                cfg.seeds,
                cfg.cross_n,
                cfg.cross_budget,
                visibility,
            );
            seed_refs.extend(added); // also complete the newly-found sessions
            episode_fill(brain, hits, &seed_refs, cfg.episode_budget, visibility);
        }
        SpreadMode::Rerank => rerank_displace(brain, hits, cfg.seeds, cfg.rerank_b, visibility),
    }
}

// ── seed refs: (episode_id, seed created_at seconds) ──

type SeedRef = (Option<String>, Option<i64>);

fn top_seed_refs(hits: &[MemoryHit], seeds: usize) -> Vec<SeedRef> {
    hits.iter()
        .take(seeds)
        .map(|h| {
            (
                h.episode_id.clone(),
                h.created_at.as_deref().and_then(parse_ts),
            )
        })
        .collect()
}

// ── cross-session (PRF) ──

/// Returns the (episode_id, created_at) of each memory it added, so a caller can
/// then complete the newly-found sessions with episode spreading.
fn cross_session(
    brain: &Brain,
    hits: &mut Vec<MemoryHit>,
    seeds: usize,
    n: usize,
    budget: usize,
    visibility: Visibility,
) -> Vec<SeedRef> {
    let existing: HashSet<String> = hits.iter().map(|h| h.key.clone()).collect();
    let seed_contents: Vec<String> = hits.iter().take(seeds).map(|h| h.content.clone()).collect();
    let mut added_keys = HashSet::new();
    let mut added = Vec::new();
    let mut spent = 0usize;
    let prf_cfg = RecallTopKConfig {
        k: n + 4,
        ..RecallTopKConfig::default()
    };
    // recall_topk_fts enforces the boundary, so mates respect the caller's context.
    'seeds: for content in seed_contents {
        if let Ok(mates) = brain.recall_topk_fts(&content, &prf_cfg, visibility) {
            for mate in mates.into_iter().take(n) {
                if existing.contains(&mate.key) || !added_keys.insert(mate.key.clone()) {
                    continue;
                }
                let cost = mate.content.len() / 4;
                if spent + cost > budget {
                    break 'seeds;
                }
                spent += cost;
                added.push((
                    mate.episode_id.clone(),
                    mate.created_at.as_deref().and_then(parse_ts),
                ));
                hits.push(mate);
            }
        }
    }
    added
}

// ── episode (same-session) proximity fill ──

fn episode_fill(
    brain: &Brain,
    hits: &mut Vec<MemoryHit>,
    seed_refs: &[SeedRef],
    budget: usize,
    visibility: Visibility,
) {
    let existing: HashSet<String> = hits.iter().map(|h| h.key.clone()).collect();
    let mut candidates: Vec<(i64, MemoryHit)> = Vec::new();
    let mut seen_eps = HashSet::new();
    let mut cand_keys = HashSet::new();
    for (ep_opt, seed_ts) in seed_refs {
        let ep = match ep_opt {
            Some(e) => e.clone(),
            None => continue,
        };
        if !seen_eps.insert(ep.clone()) {
            continue;
        }
        if let Ok(mems) = brain.list_memories_by_episode(&ep) {
            for mem in mems {
                if existing.contains(&mem.key) || !cand_keys.insert(mem.key.clone()) {
                    continue;
                }
                // list_memories_by_episode has no visibility clause — filter here
                // so episode fill can't leak a private episode-mate of a seed.
                if !crate::brain::str_to_vis(&mem.visibility).allows(visibility) {
                    continue;
                }
                let prox = match (seed_ts, mem.created_at.as_deref().and_then(parse_ts)) {
                    (Some(s), Some(t)) => (s - t).abs(),
                    _ => i64::MAX, // unknown time → lowest priority
                };
                candidates.push((prox, memory_to_hit(&mem)));
            }
        }
    }
    candidates.sort_by_key(|(p, _)| *p);
    let mut spent = 0usize;
    for (_, hit) in candidates {
        let cost = hit.content.len() / 4;
        if spent + cost > budget {
            continue;
        }
        spent += cost;
        hits.push(hit);
    }
}

// ── rerank (session-preserving displacement) ──

fn rerank_displace(
    brain: &Brain,
    hits: &mut Vec<MemoryHit>,
    seeds: usize,
    replace_b: usize,
    visibility: Visibility,
) {
    let existing: HashSet<String> = hits.iter().map(|h| h.key.clone()).collect();
    let seed_refs = top_seed_refs(hits, seeds);
    let mut candidates: Vec<(i64, MemoryHit)> = Vec::new();
    let mut seen_eps = HashSet::new();
    let mut cand_keys = HashSet::new();
    for (ep_opt, seed_ts) in &seed_refs {
        let ep = match ep_opt {
            Some(e) => e.clone(),
            None => continue,
        };
        if !seen_eps.insert(ep.clone()) {
            continue;
        }
        if let Ok(mems) = brain.list_memories_by_episode(&ep) {
            for mem in mems {
                if existing.contains(&mem.key) || !cand_keys.insert(mem.key.clone()) {
                    continue;
                }
                if !crate::brain::str_to_vis(&mem.visibility).allows(visibility) {
                    continue;
                }
                let prox = match (seed_ts, mem.created_at.as_deref().and_then(parse_ts)) {
                    (Some(s), Some(t)) => (s - t).abs(),
                    _ => i64::MAX,
                };
                candidates.push((prox, memory_to_hit(&mem)));
            }
        }
    }
    candidates.sort_by_key(|(p, _)| *p);
    let to_add: Vec<MemoryHit> = candidates
        .into_iter()
        .take(replace_b)
        .map(|(_, h)| h)
        .collect();

    // Session-preserving removal: drop the weakest hits to make room, but never
    // remove a memory that is the SOLE representative of its session (episode) —
    // otherwise displacement can silently lose a contributing answer session.
    let session_of = |h: &MemoryHit| h.episode_id.clone().unwrap_or_default();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for h in hits.iter() {
        *counts.entry(session_of(h)).or_default() += 1;
    }
    let mut remove: HashSet<usize> = HashSet::new();
    for i in (0..hits.len()).rev() {
        if remove.len() >= to_add.len() {
            break;
        }
        let s = session_of(&hits[i]);
        if counts.get(&s).copied().unwrap_or(0) > 1 {
            remove.insert(i);
            *counts.get_mut(&s).unwrap() -= 1;
        }
    }
    let mut kept: Vec<MemoryHit> = std::mem::take(hits)
        .into_iter()
        .enumerate()
        .filter_map(|(i, h)| if remove.contains(&i) { None } else { Some(h) })
        .collect();
    kept.extend(to_add);
    *hits = kept;
}

// ── helpers ──

fn parse_ts(s: &str) -> Option<i64> {
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp());
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

fn memory_to_hit(m: &Memory) -> MemoryHit {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_off() {
        assert_eq!(AssocSpreadConfig::default().mode, SpreadMode::Off);
    }

    #[test]
    fn presets_pick_the_recommended_modes() {
        assert_eq!(AssocSpreadConfig::precision().mode, SpreadMode::Rerank);
        assert_eq!(AssocSpreadConfig::completeness().mode, SpreadMode::Combined);
    }

    #[test]
    fn parse_ts_handles_both_formats() {
        assert!(parse_ts("2023-06-15 12:00:00").is_some());
        assert!(parse_ts("2023-06-15T12:00:00Z").is_some());
        assert!(parse_ts("nonsense").is_none());
    }
}
