//! The facade's consolidation family and its remaining recall variants.
//!
//! `consolidate_with`, `consolidate_extractive`, `consolidate_into` and
//! `consolidation_candidates` are four entry points into one subsystem, and
//! none of them was exercised through the public API. Consolidation *hides*
//! source memories from ordinary recall while keeping them reachable through
//! provenance, so a mistake here makes data appear to vanish — which is why
//! every test below checks both halves: the summary is recallable, and the
//! sources are still reachable.
//!
//! Also covers `recall_with` across all three `RecallProfile`s, `recall_graph`,
//! `assert_typed`, `recommend`, and the identity accessors that feed
//! `verify_hit`.

use spectral::{Brain, RecallOptions, RecallProfile, RecallTopKConfig, Visibility};
use tempfile::TempDir;

fn open(tmp: &TempDir) -> Brain {
    Brain::open(tmp.path()).unwrap()
}

/// Seed N related memories that consolidation can act on.
fn seed_sources(brain: &Brain, n: usize) -> Vec<String> {
    let mut keys = Vec::new();
    for i in 0..n {
        let key = format!("src-{i}");
        brain
            .remember(
                &key,
                &format!("standup note {i}: the zephyr rollout is on track"),
                Visibility::Private,
            )
            .unwrap();
        keys.push(key);
    }
    keys
}

fn recalled_keys(brain: &Brain, query: &str) -> Vec<String> {
    brain
        .recall_topk_fts(query, &RecallTopKConfig::default(), Visibility::Private)
        .unwrap()
        .into_iter()
        .map(|h| h.key)
        .collect()
}

// ── consolidate_with: the caller-supplied summarizer ───────────────

/// `consolidate_with` is the seam where an LLM *may* be used. The closure must
/// receive the source contents in order and its output must become the stored
/// summary — a wrapper that ignored the closure would still return `Ok`.
#[test]
fn consolidate_with_stores_exactly_what_the_summarizer_returned() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    let sources = seed_sources(&b, 3);

    let mut seen: Vec<String> = Vec::new();
    let result = b
        .consolidate_with(
            &sources,
            "summary",
            spectral_ingest::CompactionTier::DailyRollup,
            |contents| {
                seen = contents.to_vec();
                "THE SUMMARY TEXT".to_string()
            },
        )
        .unwrap();

    assert_eq!(
        seen.len(),
        3,
        "the summarizer should receive every source's content, got {seen:?}"
    );
    assert!(
        seen.iter().all(|c| c.contains("standup note")),
        "the summarizer received something other than the source contents"
    );

    let stored = b
        .get_memory(&result.memory_id)
        .unwrap()
        .expect("the summary should be stored");
    assert_eq!(
        stored.content, "THE SUMMARY TEXT",
        "the stored summary is not what the summarizer returned"
    );
}

/// Consolidation hides the sources from ordinary recall but must keep them
/// reachable through provenance. Both halves asserted — hiding without
/// preserving would be data loss.
#[test]
fn consolidation_hides_sources_from_recall_but_keeps_them_reachable() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    let sources = seed_sources(&b, 3);

    let before = recalled_keys(&b, "zephyr rollout standup");
    assert!(
        sources.iter().all(|k| before.contains(k)),
        "precondition: all sources recall before consolidation"
    );

    b.consolidate_as(
        &sources,
        "summary",
        spectral_ingest::CompactionTier::DailyRollup,
        "zephyr rollout summary for the week",
    )
    .unwrap();

    let after = recalled_keys(&b, "zephyr rollout standup");
    for k in &sources {
        assert!(
            !after.contains(k),
            "source {k} still appears in ordinary recall after consolidation"
        );
    }

    // ... but provenance still reaches them.
    let layered = b
        .recall_with_provenance(
            "zephyr rollout",
            &RecallTopKConfig::default(),
            Visibility::Private,
            10,
        )
        .unwrap();
    let summary = layered
        .iter()
        .find(|h| h.hit.key == "summary")
        .expect("the summary should be recallable");
    assert_eq!(
        summary.sources.len(),
        3,
        "the sources are hidden AND unreachable — that is data loss, not \
         consolidation"
    );
}

/// `consolidate_extractive` is the deterministic, $0 default: it picks the
/// longest source rather than calling a model.
#[test]
fn consolidate_extractive_picks_a_source_verbatim() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    b.remember("s1", "short note", Visibility::Private).unwrap();
    b.remember(
        "s2",
        "a considerably longer note about the zephyr rollout and its timeline",
        Visibility::Private,
    )
    .unwrap();

    let result = b
        .consolidate_extractive(
            &["s1".to_string(), "s2".to_string()],
            "summary",
            spectral_ingest::CompactionTier::DailyRollup,
        )
        .unwrap();

    let stored = b.get_memory(&result.memory_id).unwrap().unwrap();
    assert_eq!(
        stored.content, "a considerably longer note about the zephyr rollout and its timeline",
        "extractive consolidation should reuse the longest source verbatim, \
         with no model in the loop"
    );
}

/// `consolidate_into` links existing memories to an existing target, and is
/// documented as idempotent on the same source→target pair.
#[test]
fn consolidate_into_links_sources_and_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    let sources = seed_sources(&b, 2);
    b.remember(
        "target",
        "the rollup that absorbs them",
        Visibility::Private,
    )
    .unwrap();

    let opts = spectral_ingest::ConsolidateOpts::default();
    b.consolidate_into(&sources, "target", &opts).unwrap();
    let first = b.list_consolidated(Some("target")).unwrap();
    assert_eq!(first.len(), 2, "expected one edge per source");

    // Same pair again — must not duplicate.
    b.consolidate_into(&sources, "target", &opts).unwrap();
    let second = b.list_consolidated(Some("target")).unwrap();
    assert_eq!(
        second.len(),
        2,
        "re-consolidating the same pair created duplicate edges"
    );
}

/// `consolidation_candidates` surfaces recurring clusters. On a brain with no
/// co-retrieval history it must return nothing rather than erroring — the
/// common case for a fresh brain.
#[test]
fn consolidation_candidates_is_empty_without_co_retrieval_history() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    seed_sources(&b, 3);
    assert!(b.consolidation_candidates(2, 100).unwrap().is_empty());
}

// ── recall_with: the three profiles ────────────────────────────────

/// All three profiles must run and respect the visibility boundary that
/// `RecallOptions` requires. `Fast` disables the adaptive channels, so the
/// three are not expected to agree — only to be well-formed and scoped.
#[test]
fn every_recall_profile_runs_and_respects_visibility() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    b.remember("team", "the zephyr rollout runbook", Visibility::Team)
        .unwrap();
    b.remember("private", "my private zephyr thoughts", Visibility::Private)
        .unwrap();

    for profile in [
        RecallProfile::Fast,
        RecallProfile::Balanced,
        RecallProfile::Adaptive,
    ] {
        let team_view = b
            .recall_with(
                "zephyr",
                &RecallOptions::new(Visibility::Team).profile(profile),
            )
            .unwrap();
        assert!(
            team_view
                .merged_hits
                .iter()
                .all(|h| h.visibility != "private"),
            "{profile:?} leaked a private memory into a Team-scoped recall"
        );

        let private_view = b
            .recall_with(
                "zephyr",
                &RecallOptions::new(Visibility::Private).profile(profile),
            )
            .unwrap();
        assert!(
            private_view.merged_hits.len() >= team_view.merged_hits.len(),
            "{profile:?}: a Private view should see at least as much as a Team view"
        );
    }
}

/// The recall path must add no model cost — the structural `$0` claim, checked
/// through the facade a consumer actually calls.
#[test]
fn recall_with_adds_no_recognition_token_cost() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    b.remember("m", "the zephyr rollout runbook", Visibility::Private)
        .unwrap();

    let result = b
        .recall_with("zephyr", &RecallOptions::new(Visibility::Private))
        .unwrap();
    assert_eq!(result.total_recognition_token_cost, 0);
}

/// `recall_graph` is the relational entry point. On a brain with no asserted
/// triples it must return an empty result rather than erroring.
#[test]
fn recall_graph_is_empty_without_asserted_facts() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    b.remember("m", "some content about ada", Visibility::Private)
        .unwrap();
    let result = b.recall_graph("ada", Visibility::Private).unwrap();
    assert!(result.triples.is_empty());
}

// ── identity accessors and verify_hit ──────────────────────────────

/// `brain_id` and `verifying_key` must describe the SAME identity: a `BrainId`
/// is `blake3(public_key)`, and `verify_hit` re-derives it before checking a
/// signature. If the two accessors returned values from different identities,
/// every provenance check would fail for a reason no error message explains.
///
/// Asserted end to end: a memory this brain wrote carries a signature made by
/// its own identity, so verifying that hit against this brain's own key must
/// succeed.
#[test]
fn a_brains_own_signed_memory_verifies_against_its_own_key() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    b.remember("m", "the zephyr rollout runbook", Visibility::Private)
        .unwrap();

    let hits = b
        .recall_topk_fts("zephyr", &RecallTopKConfig::default(), Visibility::Private)
        .unwrap();
    let hit = hits.first().expect("one hit");
    assert!(
        hit.signature.is_some(),
        "precondition: remember() should have signed the memory"
    );

    assert!(
        Brain::verify_hit(hit, b.verifying_key()),
        "a brain's own signed memory did not verify against its own \
         verifying_key — brain_id and verifying_key may describe different \
         identities"
    );
}

/// `verify_hit` must reject an unsigned hit rather than treating absence of a
/// signature as success.
#[test]
fn verify_hit_rejects_an_unsigned_hit() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    b.remember("m", "the zephyr rollout runbook", Visibility::Private)
        .unwrap();

    let hits = b
        .recall_topk_fts("zephyr", &RecallTopKConfig::default(), Visibility::Private)
        .unwrap();
    let hit = hits.first().expect("one hit");

    // POSITIVE FLOOR, in this test rather than a neighbour's. Without it,
    // `verify_hit` hardcoded to `false` satisfies the rejection below and this
    // test stays green — confirmed by mutation. The honest-keeping assertions
    // previously lived in two OTHER crates, so deleting either would have
    // silently disarmed this one.
    assert!(
        Brain::verify_hit(hit, b.verifying_key()),
        "precondition: this brain's own signed hit must verify against its own \
         key, or the rejection below proves nothing"
    );

    let stranger = spectral_core::identity::BrainIdentity::generate();
    assert!(
        !Brain::verify_hit(hit, stranger.verifying_key()),
        "a hit verified against an unrelated key"
    );
}

// ── recommend ──────────────────────────────────────────────────────

/// `recommend` reads the co-retrieval index. With no history it must return
/// nothing rather than erroring, and an unknown memory id must be handled the
/// same way.
#[test]
fn recommend_is_empty_without_co_retrieval_history() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    let r = b
        .remember("m", "the zephyr rollout runbook", Visibility::Private)
        .unwrap();

    assert!(b.recommend(&r.memory_id, 10, 1).unwrap().is_empty());
    assert!(
        b.recommend("no-such-id", 10, 1).unwrap().is_empty(),
        "an unknown memory id should yield no recommendations, not an error"
    );
}

// ── entity description ─────────────────────────────────────────────

/// `set_entity_description` is documented as idempotent — setting the same
/// value twice is a no-op rather than an error.
#[test]
fn setting_an_entity_description_twice_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let b = open(&tmp);
    let eid = spectral_core::entity_id::entity_id("person", "ada");

    b.set_entity_description(&eid, "the first programmer")
        .unwrap();
    b.set_entity_description(&eid, "the first programmer")
        .unwrap();
}
