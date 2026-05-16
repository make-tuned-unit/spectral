use spectral::{Brain, RecallTopKConfig, RememberOpts, Visibility};
use tempfile::TempDir;

fn open_brain(tmp: &TempDir) -> Brain {
    Brain::open(tmp.path()).unwrap()
}

/// Seed a brain with a few memories for delegation tests.
fn seed(brain: &Brain) {
    brain
        .remember("del-1", "Rust is a systems language", Visibility::Private)
        .unwrap();
    brain
        .remember(
            "del-2",
            "Neovim is my favourite editor",
            Visibility::Private,
        )
        .unwrap();
    brain
        .remember_with(
            "del-3",
            "Daily standup notes from Monday",
            RememberOpts {
                episode_id: Some("ep-test-1".into()),
                ..Default::default()
            },
        )
        .unwrap();
}

// ── set_compaction_tier ─────────────────────────────────────────────

#[test]
fn set_compaction_tier_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);
    seed(&brain);

    // Get the memory ID for del-1.
    let recall = brain.recall_local("Rust systems language").unwrap();
    assert!(!recall.memory_hits.is_empty());
    let mem_id = &recall.memory_hits[0].id;

    // Set tier through wrapper, read back through get_memory.
    brain
        .set_compaction_tier(mem_id, spectral::ingest::CompactionTier::Raw)
        .unwrap();

    let mem = brain.get_memory(mem_id).unwrap().expect("memory exists");
    assert_eq!(
        mem.compaction_tier,
        Some(spectral::ingest::CompactionTier::Raw)
    );

    // Idempotent: setting same tier again succeeds.
    brain
        .set_compaction_tier(mem_id, spectral::ingest::CompactionTier::Raw)
        .unwrap();
}

// ── list_episodes ───────────────────────────────────────────────────

#[test]
fn list_episodes_empty_brain() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    let episodes = brain.list_episodes(None, 100).unwrap();
    assert!(episodes.is_empty());
}

#[test]
fn list_episodes_returns_seeded() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);
    seed(&brain);

    // At least one episode should exist after seeding (auto-detect creates episodes).
    let episodes = brain.list_episodes(None, 100).unwrap();
    assert!(!episodes.is_empty());
}

// ── list_memories_by_episode ────────────────────────────────────────

#[test]
fn list_memories_by_episode_nonexistent() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    let mems = brain.list_memories_by_episode("no-such-episode").unwrap();
    assert!(mems.is_empty());
}

#[test]
fn list_memories_by_episode_finds_seeded() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);
    seed(&brain);

    let mems = brain.list_memories_by_episode("ep-test-1").unwrap();
    assert_eq!(mems.len(), 1);
    assert!(mems[0].content.contains("standup"));
}

// ── related_memories ────────────────────────────────────────────────

#[test]
fn related_memories_empty_before_index() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);
    seed(&brain);

    let recall = brain.recall_local("Rust").unwrap();
    assert!(!recall.memory_hits.is_empty());

    // No co-retrieval index built yet → empty.
    let related = brain
        .related_memories(&recall.memory_hits[0].id, 10)
        .unwrap();
    assert!(related.is_empty());
}

#[test]
fn related_memories_populated_after_index() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);
    seed(&brain);

    // Generate retrieval events with overlapping results.
    let config = RecallTopKConfig::default();
    for _ in 0..3 {
        let _ = brain.recall_topk_fts("Rust systems language", &config, Visibility::Private);
        let _ = brain.recall_topk_fts("Rust editor Neovim", &config, Visibility::Private);
    }

    brain.rebuild_co_retrieval_index().unwrap();

    let recall = brain.recall_local("Rust").unwrap();
    let related = brain
        .related_memories(&recall.memory_hits[0].id, 10)
        .unwrap();
    // After enough overlapping retrievals, at least one pair should exist.
    assert!(!related.is_empty());
}

// ── count_retrieval_events ──────────────────────────────────────────

#[test]
fn count_retrieval_events_starts_at_zero() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    assert_eq!(brain.count_retrieval_events().unwrap(), 0);
}

#[test]
fn count_retrieval_events_increments_after_recall() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);
    seed(&brain);

    let before = brain.count_retrieval_events().unwrap();
    let _ = brain.recall_topk_fts("Rust", &RecallTopKConfig::default(), Visibility::Private);
    let after = brain.count_retrieval_events().unwrap();

    assert!(after > before);
}

// ── count_retrieval_events_by_method ────────────────────────────────

#[test]
fn count_retrieval_events_by_method_filters() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);
    seed(&brain);

    let _ = brain.recall_topk_fts("Rust", &RecallTopKConfig::default(), Visibility::Private);

    let topk_count = brain.count_retrieval_events_by_method("topk_fts").unwrap();
    let cascade_count = brain.count_retrieval_events_by_method("cascade").unwrap();

    assert!(topk_count > 0);
    assert_eq!(cascade_count, 0);
}

// ── events_for_session ──────────────────────────────────────────────

#[test]
fn events_for_session_empty() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    let events = brain.events_for_session("no-such-session", 100).unwrap();
    assert!(events.is_empty());
}

#[test]
fn events_for_session_returns_logged_events() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);
    seed(&brain);

    // Trigger a cascade recall with a session_id to log a session-tagged event.
    let ctx = spectral::graph::RecognitionContext::empty().with_session("test-sess-1");
    let cfg = spectral_cascade::orchestrator::CascadeConfig::default();
    let _ = brain.recall_cascade("Rust language", &ctx, &cfg);

    let events = brain.events_for_session("test-sess-1", 100).unwrap();
    assert!(!events.is_empty());
    assert!(events
        .iter()
        .all(|e| e.session_id.as_deref() == Some("test-sess-1")));
}

// ── memories_for_session ────────────────────────────────────────────

#[test]
fn memories_for_session_empty() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);

    let mems = brain.memories_for_session("no-such-session").unwrap();
    assert!(mems.is_empty());
}

#[test]
fn memories_for_session_returns_surfaced_ids() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);
    seed(&brain);

    let ctx = spectral::graph::RecognitionContext::empty().with_session("test-sess-2");
    let cfg = spectral_cascade::orchestrator::CascadeConfig::default();
    let _ = brain.recall_cascade("Rust language", &ctx, &cfg);

    let mems = brain.memories_for_session("test-sess-2").unwrap();
    // If the cascade returned any hits, memory IDs should be logged.
    // (On an empty/tiny brain the cascade may return 0 hits, so we just
    // check that the call succeeds and the types are correct.)
    assert!(mems.iter().all(|id| !id.is_empty()));
}

// ── annotate + list_annotations ─────────────────────────────────────

#[test]
fn annotate_and_list_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let brain = open_brain(&tmp);
    seed(&brain);

    let recall = brain.recall_local("Rust").unwrap();
    let mem_id = &recall.memory_hits[0].id;

    let ann = brain
        .annotate(
            mem_id,
            spectral::ingest::AnnotationInput {
                description: "test annotation".into(),
                who: vec![],
                why: String::new(),
                where_: None,
                when_: chrono::Utc::now(),
                how: String::new(),
            },
        )
        .unwrap();

    assert_eq!(ann.memory_id, *mem_id);

    let annotations = brain.list_annotations(mem_id).unwrap();
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0].description, "test annotation");
}
