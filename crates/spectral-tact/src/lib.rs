//! TACT (Topic-Aware Context Triage) — fingerprint-based memory retrieval.
//!
//! Finds relevant memories from a structured store and formats them for
//! system-prompt injection. No embedding inference required.

pub mod classifier;
pub mod extractor;
pub mod prompts;

// Re-export canonical types from spectral-ingest.
pub use spectral_ingest::{Memory, MemoryHit, MemoryStore};

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// Trait for injecting an LLM implementation (optional — TACT's core
/// pipeline is regex-only and does not call an LLM).
pub trait LlmClient: Send + Sync {
    fn complete(
        &self,
        prompt: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + '_>>;
}

/// TACT pipeline configuration.
#[derive(Debug, Clone)]
pub struct TactConfig {
    /// Minimum word count for TACT classification to engage. Queries with
    /// fewer words return `RetrievalMethod::Skipped` with zero results.
    /// Default 1 means single-word queries are classified normally. Set
    /// higher (e.g., 3) if consuming code wants to bypass TACT for
    /// greeting-style short messages.
    pub min_words: usize,
    /// Maximum results to return.
    pub max_results: usize,
    /// Maximum characters in the context bundle (~tokens * 4).
    pub max_context_chars: usize,
    /// Wing detection rules: (regex_pattern, wing_name).
    pub wing_rules: Vec<(String, String)>,
    /// Hall detection rules: (regex_pattern, hall_name).
    pub hall_rules: Vec<(String, String)>,
    /// Require a hall on the QUERY before tier 1 (fingerprint search) may run.
    ///
    /// Default `true` — the historical behaviour.
    ///
    /// A hall is a *memory type*, and the hall rules match a speaker asserting
    /// one (`decided|chose|remember|prefers`). A question rarely announces what
    /// kind of memory would answer it, so this conjunction suppresses tier 1
    /// almost entirely: measured on 217 real queries against a real taxonomy,
    /// wing fires 46.5%, hall 5.5%, **both 0.9%**.
    ///
    /// Setting this `false` fires tier 1 on wing alone, searching the wing's
    /// fingerprints across all anchor halls. See
    /// `docs/internal/tier1-ungating-prereg-2026-08-03.md`.
    pub tier1_requires_hall: bool,
}

impl Default for TactConfig {
    fn default() -> Self {
        Self {
            min_words: 1,
            max_results: 5,
            max_context_chars: 24000,
            wing_rules: Vec::new(),
            tier1_requires_hall: true,
            hall_rules: vec![
                (
                    r"decided|chose|switching to|using|will use|agreed|locked in|decision|auth"
                        .into(),
                    "fact".into(),
                ),
                (
                    r"remember|preference|favourit|favorit|likes|prefers".into(),
                    "preference".into(),
                ),
                (
                    r"learned|discovered|found that|realized|breakthrough|roadmap|setup".into(),
                    "discovery".into(),
                ),
                (
                    r"recommend|should|advice|suggest|try using".into(),
                    "advice".into(),
                ),
            ],
        }
    }
}

/// The retrieval method that produced results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RetrievalMethod {
    Fingerprint,
    FingerprintPlusFts,
    WingOnly,
    Fts,
    Skipped,
    Empty,
}

impl std::fmt::Display for RetrievalMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fingerprint => write!(f, "fingerprint"),
            Self::FingerprintPlusFts => write!(f, "fingerprint+fts"),
            Self::WingOnly => write!(f, "wing_only"),
            Self::Fts => write!(f, "fts_fallback"),
            Self::Skipped => write!(f, "skipped"),
            Self::Empty => write!(f, "empty"),
        }
    }
}

/// Full retrieval result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TactResult {
    pub method: RetrievalMethod,
    pub wing: Option<String>,
    pub hall: Option<String>,
    pub memories: Vec<MemoryHit>,
    /// Formatted context block for system-prompt injection.
    pub context_block: String,
}

/// Run the full TACT retrieval pipeline.
/// [`retrieve`] without building `context_block`.
///
/// The cascade path consumes only `TactResult::memories` and discards the
/// formatted block, so paying for the formatting there is pure waste on every
/// recall. Same classification, same tiers, same `memories` and `method` —
/// only `context_block` differs (always empty). Callers that need the prompt
/// block should keep using [`retrieve`].
pub async fn retrieve_memories(
    user_msg: &str,
    config: &TactConfig,
    store: &dyn MemoryStore,
) -> anyhow::Result<TactResult> {
    retrieve_inner(user_msg, None, config, store, false).await
}

/// [`retrieve_memories`] with an **ambient wing hint**.
///
/// A wing is detected from the query text today, which requires the user to
/// *name* the project — measured at **12.4%** of real agent queries ("Give me a
/// tour of the app" names nothing). The remaining 87.6% is scope the agent
/// already has and the library never receives.
///
/// `wing_hint` supplies it. The query's own wing still wins when present, so an
/// explicit mention overrides ambient context rather than the reverse.
///
/// `None` reproduces [`retrieve_memories`] exactly.
/// See `docs/internal/tier1-ungating-result-2026-08-03.md`.
pub async fn retrieve_memories_scoped(
    user_msg: &str,
    wing_hint: Option<&str>,
    config: &TactConfig,
    store: &dyn MemoryStore,
) -> anyhow::Result<TactResult> {
    retrieve_inner(user_msg, wing_hint, config, store, false).await
}

pub async fn retrieve(
    user_msg: &str,
    config: &TactConfig,
    store: &dyn MemoryStore,
) -> anyhow::Result<TactResult> {
    retrieve_inner(user_msg, None, config, store, true).await
}

async fn retrieve_inner(
    user_msg: &str,
    wing_hint: Option<&str>,
    config: &TactConfig,
    store: &dyn MemoryStore,
    build_context_block: bool,
) -> anyhow::Result<TactResult> {
    if user_msg.split_whitespace().count() < config.min_words {
        return Ok(TactResult {
            method: RetrievalMethod::Skipped,
            wing: None,
            hall: None,
            memories: Vec::new(),
            context_block: String::new(),
        });
    }

    // Query-named wing wins; ambient scope fills the gap when the user did not
    // name a project.
    let wing = classifier::detect_wing(user_msg, &config.wing_rules)
        .or_else(|| wing_hint.map(|w| w.to_string()));
    let hall = classifier::detect_hall(user_msg, &config.hall_rules);

    let (memories, method) = extractor::search(user_msg, &wing, &hall, config, store).await?;

    if memories.is_empty() {
        return Ok(TactResult {
            method: RetrievalMethod::Empty,
            wing,
            hall,
            memories: Vec::new(),
            context_block: String::new(),
        });
    }

    let context_block = if build_context_block {
        format_context_block(&memories, config.max_context_chars)
    } else {
        String::new()
    };

    Ok(TactResult {
        method,
        wing,
        hall,
        memories,
        context_block,
    })
}

/// Format the system-prompt injection block for a set of memories. The single
/// source of truth for the block's shape, so a caller that filters `memories`
/// (e.g. by visibility) can rebuild `context_block` and be sure the formatted
/// text can never contain a dropped memory. Returns `""` for an empty set.
pub fn format_context_block(memories: &[MemoryHit], max_context_chars: usize) -> String {
    if memories.is_empty() {
        return String::new();
    }
    let bundle = build_context_bundle(memories, max_context_chars);
    format!("\n--- MEMORY CONTEXT (via TACT) ---\n{bundle}\n--- END MEMORY CONTEXT ---\n")
}

fn build_context_bundle(memories: &[MemoryHit], max_chars: usize) -> String {
    let mut parts = Vec::new();
    let mut char_count = 0;

    for m in memories {
        let wing = m.wing.as_deref().unwrap_or("unknown");
        let hall = m.hall.as_deref().unwrap_or("unknown");
        let entry = format!("[{}/{}] {}: {}", wing, hall, m.key, m.content);

        if char_count + entry.len() > max_chars {
            let remaining = max_chars.saturating_sub(char_count);
            if remaining > 50 {
                // Back off to a UTF-8 char boundary — a raw byte slice panics
                // when `remaining` lands inside a multi-byte char (e.g. an em-dash).
                let mut cut = remaining.min(entry.len());
                while cut > 0 && !entry.is_char_boundary(cut) {
                    cut -= 1;
                }
                parts.push(format!("{}...", &entry[..cut]));
            }
            break;
        }

        char_count += entry.len() + 1;
        parts.push(entry);
    }

    parts.join("\n")
}

#[cfg(test)]
mod context_block_contract {
    //! The formatted block is what gets injected into a system prompt, and
    //! `format_context_block`'s own doc makes a safety claim about it: it is
    //! "the single source of truth for the block's shape, so a caller that
    //! filters `memories` (e.g. by visibility) can rebuild `context_block` and
    //! be sure the formatted text can never contain a dropped memory."
    //!
    //! `cargo mutants` found that claim was unenforced. Both this function and
    //! `build_context_bundle` could be replaced with `String::new()` or
    //! `"xyzzy"`, and every comparison in the truncation logic could be
    //! flipped, with the suite still green — the one existing test asserted
    //! only `result.len() <= 110`, which an empty string satisfies too.
    //!
    //! The truncation path also carries a fixed panic (a raw byte slice that
    //! split a multi-byte char), so it is exactly the code that should not be
    //! left unpinned.

    use super::*;

    fn hit(key: &str, content: &str) -> MemoryHit {
        MemoryHit {
            id: key.into(),
            key: key.into(),
            content: content.into(),
            wing: Some("proj".into()),
            hall: Some("fact".into()),
            signal_score: 0.9,
            visibility: "private".into(),
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: None,
            last_reinforced_at: None,
            episode_id: None,
            declarative_density: None,
            description: None,
            hits: 1,
            source_brain_id: None,
            signature: None,
        }
    }

    /// The safety claim, stated directly: a memory that is not in the input
    /// cannot appear in the output.
    #[test]
    fn the_block_never_contains_a_memory_that_was_filtered_out() {
        let all = [
            hit("public-1", "shareable note"),
            hit("secret-1", "classified payload"),
        ];
        let filtered: Vec<MemoryHit> = all
            .iter()
            .filter(|m| m.key != "secret-1")
            .cloned()
            .collect();

        let block = format_context_block(&filtered, 10_000);
        assert!(
            block.contains("shareable note"),
            "the surviving memory is missing from the block: {block}"
        );
        assert!(
            !block.contains("classified payload") && !block.contains("secret-1"),
            "a filtered-out memory appeared in the formatted block: {block}"
        );
    }

    /// The block must actually carry the content it was given — the check the
    /// old length-bound assertion could not make.
    #[test]
    fn the_block_contains_the_content_and_its_delimiters() {
        let block = format_context_block(&[hit("k1", "the deploy runbook")], 10_000);
        assert!(
            block.contains("the deploy runbook"),
            "content missing: {block}"
        );
        assert!(block.contains("k1"), "key missing: {block}");
        assert!(block.contains("[proj/fact]"), "wing/hall missing: {block}");
        assert!(
            block.contains("--- MEMORY CONTEXT (via TACT) ---"),
            "opening delimiter missing"
        );
        assert!(
            block.contains("--- END MEMORY CONTEXT ---"),
            "closing delimiter missing"
        );
    }

    /// An empty set yields an empty block — not a bare pair of delimiters,
    /// which would inject a misleading "memory context" header with nothing in
    /// it.
    #[test]
    fn an_empty_set_produces_an_empty_block() {
        assert_eq!(format_context_block(&[], 10_000), "");
    }

    /// Every memory that fits must be present. A truncation comparison that is
    /// off by one direction silently drops the last one.
    #[test]
    fn every_memory_that_fits_within_the_budget_is_included() {
        let memories = vec![hit("k1", "alpha"), hit("k2", "bravo"), hit("k3", "charlie")];
        let block = format_context_block(&memories, 10_000);
        for needle in ["alpha", "bravo", "charlie"] {
            assert!(
                block.contains(needle),
                "{needle} was dropped despite fitting: {block}"
            );
        }
    }

    /// The budget is a real bound, and it bites: a set far larger than the
    /// budget must be cut down, and must not simply be emitted whole.
    #[test]
    fn the_budget_actually_truncates_an_oversized_set() {
        let memories = vec![hit("k1", &"a".repeat(400)), hit("k2", &"b".repeat(400))];
        let bundle = build_context_bundle(&memories, 200);

        assert!(
            bundle.len() < 800,
            "the budget did not bind at all: {} chars",
            bundle.len()
        );
        assert!(
            !bundle.contains(&"b".repeat(400)),
            "the second oversized memory was included whole"
        );
        assert!(!bundle.is_empty(), "truncation produced nothing at all");
    }

    /// A budget too small even for a partial entry yields nothing, rather than
    /// a stub that claims to be memory context.
    #[test]
    fn a_budget_below_the_partial_entry_floor_yields_nothing() {
        let bundle = build_context_bundle(&[hit("k1", &"a".repeat(400))], 10);
        assert!(
            bundle.is_empty(),
            "expected no entry under a tiny budget, got {bundle:?}"
        );
    }

    /// The regression that the char-boundary loop exists for: truncating
    /// inside a multi-byte character used to panic on a raw byte slice.
    #[test]
    fn truncating_multibyte_content_does_not_panic_and_stays_valid_utf8() {
        for filler in ["—", "日本語", "🙂", "é"] {
            let content = filler.repeat(300);
            for budget in [60, 61, 62, 63, 64, 100, 137, 200] {
                let bundle = build_context_bundle(&[hit("k1", &content)], budget);
                // Reaching here at all means no panic; String is UTF-8 by
                // construction, so re-validating the bytes proves the cut
                // landed on a character boundary.
                assert!(
                    std::str::from_utf8(bundle.as_bytes()).is_ok(),
                    "invalid UTF-8 for {filler:?} at budget {budget}"
                );
            }
        }
    }

    /// The clipped entry must still respect the budget it was clipped to.
    ///
    /// Sharper than "is it valid UTF-8": walking the cut the wrong way to find
    /// a character boundary also produces valid UTF-8, just longer than
    /// allowed, so only a length bound catches it.
    #[test]
    fn a_clipped_entry_does_not_exceed_its_budget() {
        for filler in ["—", "日本語", "🙂", "a"] {
            let content = filler.repeat(300);
            for budget in [60usize, 61, 62, 63, 64, 100, 137, 200] {
                let bundle = build_context_bundle(&[hit("k1", &content)], budget);
                assert!(
                    bundle.len() <= budget + 3,
                    "{filler:?} at budget {budget}: clipped entry is {} bytes,                      over budget even allowing for the \"...\" marker",
                    bundle.len()
                );
            }
        }
    }

    /// A clipped entry must still carry real content. Collapsing the boundary
    /// search to a zero-length cut yields a bare "..." — technically truncated,
    /// entirely useless, and indistinguishable from working code by any test
    /// that only checks for the marker.
    #[test]
    fn a_clipped_entry_still_carries_its_prefix() {
        let bundle = build_context_bundle(&[hit("k1", &"—".repeat(300))], 120);
        assert!(
            bundle.starts_with("[proj/fact] k1:"),
            "the clipped entry lost its own prefix: {bundle:?}"
        );
        assert!(
            bundle.trim_end_matches('.').len() > 20,
            "the clip kept almost nothing: {bundle:?}"
        );
    }

    /// Budget accounting must accumulate across entries. If the running total
    /// stops growing, every memory is emitted regardless of budget — the
    /// truncation is silently disabled rather than visibly broken.
    #[test]
    fn the_budget_accumulates_across_entries() {
        let memories = vec![
            hit("k1", &"a".repeat(120)),
            hit("k2", &"b".repeat(120)),
            hit("k3", &"c".repeat(120)),
        ];
        // Room for the first entry and nothing like all three.
        let bundle = build_context_bundle(&memories, 150);
        assert!(
            bundle.contains(&"a".repeat(50)),
            "the first memory should fit: {bundle:?}"
        );
        assert!(
            !bundle.contains(&"c".repeat(120)),
            "the third memory was emitted whole despite the budget being spent"
        );
        assert!(
            bundle.len() <= 150 + 3,
            "cumulative output is {} bytes against a 150 budget",
            bundle.len()
        );
    }

    /// Truncated output is marked as truncated, so a reader can tell a clipped
    /// memory from a short one.
    #[test]
    fn a_partial_entry_is_marked_with_an_ellipsis() {
        let bundle = build_context_bundle(&[hit("k1", &"a".repeat(400))], 200);
        assert!(
            bundle.ends_with("..."),
            "a clipped entry is not marked: {bundle}"
        );
    }

    /// Multiple entries are newline-separated, which is the shape a caller
    /// rebuilding the block depends on.
    #[test]
    fn entries_are_newline_separated() {
        let bundle = build_context_bundle(&[hit("k1", "alpha"), hit("k2", "bravo")], 10_000);
        assert_eq!(
            bundle.lines().count(),
            2,
            "expected one line per memory: {bundle}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_context_bundle_truncation() {
        let memories = vec![
            MemoryHit {
                id: "1".into(),
                key: "test key".into(),
                content: "a".repeat(200),
                wing: Some("proj".into()),
                hall: Some("fact".into()),
                signal_score: 0.9,
                visibility: "private".into(),
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
                hits: 3,
                source_brain_id: None,
                signature: None,
            },
            MemoryHit {
                id: "2".into(),
                key: "another".into(),
                content: "b".repeat(200),
                wing: Some("proj".into()),
                hall: Some("discovery".into()),
                signal_score: 0.7,
                visibility: "private".into(),
                source: None,
                device_id: None,
                confidence: 1.0,
                created_at: None,
                last_reinforced_at: None,
                episode_id: None,
                declarative_density: None,
                description: None,
                hits: 1,
                source_brain_id: None,
                signature: None,
            },
        ];

        let result = build_context_bundle(&memories, 100);
        assert!(result.len() <= 110);
    }

    #[test]
    fn test_retrieval_method_display() {
        assert_eq!(RetrievalMethod::Fingerprint.to_string(), "fingerprint");
        assert_eq!(RetrievalMethod::Fts.to_string(), "fts_fallback");
    }
}
