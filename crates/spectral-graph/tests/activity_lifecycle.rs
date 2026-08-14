//! The activity wing's full lifecycle: ingest, redaction accounting, and the
//! two pruning paths.
//!
//! `ingest_activity`, `prune_activity_older_than`, and
//! `prune_activity_keep_recent` had **no test caller anywhere in the
//! workspace**. Two of the three are DELETE operations, and the store's own
//! comment on `delete_wing_memories_before` notes that a naive string compare
//! "would delete the wrong rows at a day boundary. This is a DELETE, so the
//! mis-compare is destructive."
//!
//! An untested deletion path is the worst thing to leave uncovered, so these
//! tests assert both halves of every prune: that the intended rows go, **and
//! that the rows outside the criterion survive**. A prune that deleted
//! everything would satisfy a naive "did it delete?" assertion.

use chrono::{DateTime, Duration, Utc};
use spectral_core::visibility::Visibility;
use spectral_graph::activity::ActivityEpisode;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
use tempfile::TempDir;

fn brain_config(tmp: &TempDir) -> BrainConfig {
    let ontology_path = tmp.path().join("ontology.toml");
    std::fs::write(&ontology_path, "version = 1\n").unwrap();
    BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path,
        entity_policy: EntityPolicy::Strict,
        activity_wing: "activity".into(),
        ..Default::default()
    }
}

fn open(tmp: &TempDir) -> Brain {
    Brain::open(brain_config(tmp)).unwrap()
}

fn episode(id: &str, bundle: &str, at: DateTime<Utc>) -> ActivityEpisode {
    ActivityEpisode {
        id: id.into(),
        started_at: at,
        ended_at: at + Duration::seconds(30),
        bundle_id: bundle.into(),
        app_name: format!("App {bundle}"),
        window_title: Some(format!("window for {id}")),
        url: None,
        excerpt: Some(format!("visible text of {id}")),
        source: bundle.into(),
        source_event_count: 3,
        metadata: serde_json::Value::Null,
        wing: None,
    }
}

/// Count memories currently filed in the activity wing.
fn activity_count(brain: &Brain) -> usize {
    brain.list_wing_memories("activity", 0.0).unwrap().len()
}

// ── ingest ─────────────────────────────────────────────────────────

#[test]
fn ingest_activity_reports_what_it_did_and_is_idempotent_by_id() {
    let tmp = TempDir::new().unwrap();
    let brain = open(&tmp);
    let now = Utc::now();

    let batch = vec![
        episode("e1", "com.example.editor", now),
        episode("e2", "com.example.browser", now),
    ];
    let stats = brain.ingest_activity(&batch).unwrap();
    assert_eq!(stats.episodes_received, 2);
    assert_eq!(stats.episodes_rejected, 0);
    assert_eq!(activity_count(&brain), 2, "both episodes should be stored");

    // Re-ingesting the SAME ids must not duplicate — the type documents
    // "Re-ingesting the same episode by id is a no-op (UPSERT)".
    let stats = brain.ingest_activity(&batch).unwrap();
    assert_eq!(stats.episodes_received, 2);
    assert_eq!(
        activity_count(&brain),
        2,
        "re-ingesting the same ids duplicated rows instead of upserting"
    );
}

#[test]
fn ingest_activity_on_an_empty_batch_is_a_no_op() {
    let tmp = TempDir::new().unwrap();
    let brain = open(&tmp);
    let stats = brain.ingest_activity(&[]).unwrap();
    assert_eq!(stats.episodes_received, 0);
    assert_eq!(stats.episodes_inserted, 0);
    assert_eq!(activity_count(&brain), 0);
}

#[test]
fn ingested_activity_lands_in_the_configured_activity_wing() {
    let tmp = TempDir::new().unwrap();
    // A non-default wing name proves the configured value is what is used,
    // rather than a hardcoded "activity" that happens to match the default.
    let mut config = brain_config(&tmp);
    config.activity_wing = "ambient".into();
    let brain = Brain::open(config).unwrap();

    brain
        .ingest_activity(&[episode("e1", "com.example.editor", Utc::now())])
        .unwrap();

    assert_eq!(
        brain.list_wing_memories("ambient", 0.0).unwrap().len(),
        1,
        "activity did not land in the configured wing"
    );
    assert!(
        brain
            .list_wing_memories("activity", 0.0)
            .unwrap()
            .is_empty(),
        "activity landed in the hardcoded default wing, ignoring the config"
    );
}

/// A read-only brain must refuse activity ingestion with `Error::ReadOnly`
/// rather than a driver error from the layer below.
#[test]
fn ingest_activity_is_refused_on_a_read_only_brain() {
    let tmp = TempDir::new().unwrap();
    drop(open(&tmp)); // create it first
    let ro = Brain::open(BrainConfig {
        read_only: true,
        ..brain_config(&tmp)
    })
    .unwrap();

    let err = ro
        .ingest_activity(&[episode("e1", "com.example.editor", Utc::now())])
        .expect_err("a read-only brain must refuse activity ingestion");
    assert!(
        matches!(err, spectral_graph::error::Error::ReadOnly(_)),
        "got {err:?}, want Error::ReadOnly"
    );
}

// ── prune_activity_older_than ──────────────────────────────────────

/// The destructive path the store warns about. Asserts BOTH halves: old rows
/// go, recent rows stay. An implementation that deleted the whole wing would
/// pass a one-sided "something was deleted" check.
#[test]
fn prune_older_than_deletes_only_what_precedes_the_cutoff() {
    let tmp = TempDir::new().unwrap();
    let brain = open(&tmp);
    let now = Utc::now();

    let old = now - Duration::days(30);
    let recent = now - Duration::hours(1);
    brain
        .ingest_activity(&[
            episode("old-1", "com.example.editor", old),
            episode("old-2", "com.example.browser", old),
            episode("recent-1", "com.example.editor", recent),
        ])
        .unwrap();
    assert_eq!(activity_count(&brain), 3);

    let cutoff = now - Duration::days(7);
    let pruned = brain.prune_activity_older_than(cutoff).unwrap();

    assert_eq!(pruned, 2, "expected exactly the two pre-cutoff episodes");
    let remaining = brain.list_wing_memories("activity", 0.0).unwrap();
    assert_eq!(
        remaining.len(),
        1,
        "a post-cutoff episode was deleted: {:?}",
        remaining.iter().map(|m| &m.key).collect::<Vec<_>>()
    );
    assert!(
        remaining[0].content.contains("recent-1"),
        "the surviving row is not the recent one: {}",
        remaining[0].content
    );
}

#[test]
fn prune_older_than_with_an_ancient_cutoff_deletes_nothing() {
    let tmp = TempDir::new().unwrap();
    let brain = open(&tmp);
    brain
        .ingest_activity(&[episode("e1", "com.example.editor", Utc::now())])
        .unwrap();

    let pruned = brain
        .prune_activity_older_than(Utc::now() - Duration::days(3650))
        .unwrap();
    assert_eq!(pruned, 0);
    assert_eq!(
        activity_count(&brain),
        1,
        "nothing should have been deleted"
    );
}

/// Pruning must not reach outside the activity wing. An ordinary memory is
/// older than the cutoff and must survive — this is the blast-radius check.
#[test]
fn prune_older_than_does_not_touch_memories_outside_the_activity_wing() {
    let tmp = TempDir::new().unwrap();
    let brain = open(&tmp);
    brain
        .remember(
            "ordinary",
            "an ordinary memory, not activity",
            Visibility::Private,
        )
        .unwrap();
    brain
        .ingest_activity(&[episode(
            "old-1",
            "com.example.editor",
            Utc::now() - Duration::days(30),
        )])
        .unwrap();

    let pruned = brain
        .prune_activity_older_than(Utc::now() + Duration::days(1))
        .unwrap();
    assert_eq!(pruned, 1, "only the activity row should be pruned");
    assert!(
        brain.get_memory_by_key("ordinary").unwrap().is_some(),
        "pruning the activity wing deleted a memory outside it"
    );
}

// ── prune_activity_keep_recent ─────────────────────────────────────

/// Keeps the N most recent per source. Asserts the retained count per source
/// AND that the survivors are the *recent* ones, not an arbitrary N.
#[test]
fn prune_keep_recent_retains_the_newest_n_per_source() {
    let tmp = TempDir::new().unwrap();
    let brain = open(&tmp);
    let now = Utc::now();

    let mut batch = Vec::new();
    for src in ["com.example.editor", "com.example.browser"] {
        for i in 0..5 {
            // i = 0 is the oldest, i = 4 the newest.
            batch.push(episode(
                &format!("{src}-{i}"),
                src,
                now - Duration::hours(10 - i as i64),
            ));
        }
    }
    brain.ingest_activity(&batch).unwrap();
    assert_eq!(activity_count(&brain), 10);

    let pruned = brain.prune_activity_keep_recent(2).unwrap();
    assert_eq!(pruned, 6, "5 per source, keeping 2, should prune 3 each");

    let remaining = brain.list_wing_memories("activity", 0.0).unwrap();
    assert_eq!(remaining.len(), 4, "expected 2 survivors per source");

    for src in ["com.example.editor", "com.example.browser"] {
        let kept: Vec<&String> = remaining
            .iter()
            .filter(|m| m.content.contains(src))
            .map(|m| &m.content)
            .collect();
        assert_eq!(kept.len(), 2, "source {src} kept {} rows", kept.len());
        // The newest two are index 3 and 4; none of 0..=2 may survive.
        for stale in 0..3 {
            assert!(
                !kept.iter().any(|c| c.contains(&format!("{src}-{stale}"))),
                "an older episode survived while a newer one was pruned: {src}-{stale}"
            );
        }
    }
}

#[test]
fn prune_keep_recent_is_a_no_op_when_the_budget_exceeds_what_exists() {
    let tmp = TempDir::new().unwrap();
    let brain = open(&tmp);
    let now = Utc::now();
    brain
        .ingest_activity(&[
            episode("e1", "com.example.editor", now),
            episode("e2", "com.example.editor", now - Duration::hours(1)),
        ])
        .unwrap();

    let pruned = brain.prune_activity_keep_recent(50).unwrap();
    assert_eq!(pruned, 0);
    assert_eq!(activity_count(&brain), 2);
}

/// Both prune paths must be refused on a read-only brain — they are deletes.
#[test]
fn both_prune_paths_are_refused_on_a_read_only_brain() {
    let tmp = TempDir::new().unwrap();
    {
        let brain = open(&tmp);
        brain
            .ingest_activity(&[episode("e1", "com.example.editor", Utc::now())])
            .unwrap();
    }
    let ro = Brain::open(BrainConfig {
        read_only: true,
        ..brain_config(&tmp)
    })
    .unwrap();

    assert!(
        matches!(
            ro.prune_activity_older_than(Utc::now()),
            Err(spectral_graph::error::Error::ReadOnly(_))
        ),
        "prune_activity_older_than must be refused read-only"
    );
    assert!(
        matches!(
            ro.prune_activity_keep_recent(1),
            Err(spectral_graph::error::Error::ReadOnly(_))
        ),
        "prune_activity_keep_recent must be refused read-only"
    );
    // And nothing was deleted by the refused calls.
    assert_eq!(activity_count(&ro), 1);
}

// ── backfill_declarative_density ───────────────────────────────────

/// `backfill_declarative_density` repairs rows written before the column
/// existed. Exercised through `repair_derivations`' sibling path: after a
/// backfill every memory must carry a density, and a second run must find
/// nothing left to do (idempotence).
#[test]
fn backfill_declarative_density_fills_every_row_and_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let brain = open(&tmp);
    for i in 0..3 {
        brain
            .remember(
                &format!("m{i}"),
                &format!("the deploy runbook number {i} lives in notion"),
                Visibility::Private,
            )
            .unwrap();
    }

    // Clear the column to simulate legacy rows.
    {
        let db = tmp.path().join("memory.db");
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute("UPDATE memories SET declarative_density = NULL", [])
            .unwrap();
    }

    let filled = brain.backfill_declarative_density().unwrap();
    assert_eq!(filled, 3, "every cleared row should have been backfilled");

    let again = brain.backfill_declarative_density().unwrap();
    assert_eq!(again, 0, "a second backfill should find nothing to do");
}

#[test]
fn backfill_declarative_density_is_refused_on_a_read_only_brain() {
    let tmp = TempDir::new().unwrap();
    drop(open(&tmp));
    let ro = Brain::open(BrainConfig {
        read_only: true,
        ..brain_config(&tmp)
    })
    .unwrap();
    assert!(matches!(
        ro.backfill_declarative_density(),
        Err(spectral_graph::error::Error::ReadOnly(_))
    ));
}
