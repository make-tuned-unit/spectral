//! G1 + G2 joined: the temporal resolver produces valid-time, the bi-temporal
//! columns store it, and `as_of` makes a past answer reproducible.
//!
//! Neither half is useful alone. `resolve_relative_dates` shipped exported and
//! wired to nothing because there was nowhere to put its output — `memories`
//! carried only `created_at`/`updated_at`, which are *system* time. Storing a
//! resolved world-date in `created_at` would have been a lie about when we
//! learned the fact, so the resolver had no honest sink.
//!
//! **These tests assert auditability, not accuracy.** No public benchmark
//! measures bi-temporal modelling — which is precisely why Zep's claim on it
//! is currently unfalsifiable. The property under test is that a past answer
//! stays reproducible against a store that has since changed.
//!
//! Scope: this exercises the **store** layer. Surfacing `as_of` through
//! `Brain` is deliberately not done here — `Brain::memory_store` is a private
//! `Arc<dyn MemoryStore>`, so it needs trait-level methods, which is a
//! default-path API change. See `g1-g2-bitemporal-2026-08-08.md`.

use chrono::NaiveDate;
use spectral::ingest::sqlite_store::SqliteStore;
use spectral::ingest::{Memory, MemoryStore};
use spectral::resolve_relative_dates;

fn mem(key: &str, content: &str) -> Memory {
    Memory {
        id: format!("id-{key}"),
        key: key.to_string(),
        content: content.to_string(),
        wing: Some("general".into()),
        hall: Some("fact".into()),
        signal_score: 0.5,
        visibility: "private".into(),
        source: None,
        device_id: None,
        confidence: 1.0,
        // Recorded in January, so "what was true in May" is a coherent
        // question about these rows.
        created_at: Some("2026-01-01T00:00:00Z".into()),
        last_reinforced_at: None,
        episode_id: None,
        compaction_tier: None,
        declarative_density: None,
        description: None,
        description_generated_at: None,
        content_hash: None,
        source_brain_id: None,
        signature: None,
    }
}

/// G2: the resolver turns a relative phrase plus an anchor into a date, with
/// no model call and no clock read.
#[test]
fn the_resolver_produces_a_world_date_deterministically() {
    let anchor = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
    let hits = resolve_relative_dates("I started the new job yesterday", anchor);
    assert!(!hits.is_empty(), "expected 'yesterday' to resolve");
    assert_eq!(
        hits[0].resolved,
        NaiveDate::from_ymd_opt(2026, 6, 14).unwrap()
    );

    // Same input, same output — and independent of when it runs.
    let again = resolve_relative_dates("I started the new job yesterday", anchor);
    assert_eq!(hits[0].resolved, again[0].resolved);
}

/// G1 + G2: a fact whose *content* says when it became true is stored with
/// that valid-time, and `as_of` respects it rather than the ingestion time.
#[tokio::test]
async fn resolved_valid_time_drives_as_of_visibility() {
    let store = SqliteStore::open_in_memory().unwrap();
    let content = "I started the new job yesterday";
    store.write(&mem("fact:job", content), &[]).await.unwrap();

    // G2 -> G1: resolve against the ingest anchor, then store as valid-time.
    let anchor = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
    let resolved = resolve_relative_dates(content, anchor);
    let valid_from = format!("{}T00:00:00Z", resolved[0].resolved);
    assert_eq!(valid_from, "2026-06-14T00:00:00Z");
    assert!(store.set_valid_from("fact:job", &valid_from).unwrap());

    let visible = |t: &str| {
        store
            .keys_as_of(t)
            .unwrap()
            .contains(&"fact:job".to_string())
    };

    assert!(!visible("2026-06-13T00:00:00Z"), "not yet true on the 13th");
    assert!(visible("2026-06-14T00:00:00Z"), "true from the 14th");
    assert!(visible("2026-06-20T00:00:00Z"), "still true later");
}

/// The audit claim end to end: superseding a fact does not destroy the answer
/// we would have given before it was superseded.
#[tokio::test]
async fn a_past_answer_stays_reproducible_after_the_fact_changes() {
    let store = SqliteStore::open_in_memory().unwrap();
    for (k, v) in [
        ("fact:employer", "I work at Acme"),
        ("fact:city", "I live in Lisbon"),
    ] {
        store.write(&mem(k, v), &[]).await.unwrap();
    }

    // Stops being true on 2026-06-01. INVALIDATED, never deleted.
    store
        .invalidate_at("fact:employer", "2026-06-01T00:00:00Z")
        .unwrap();

    let may = store.keys_as_of("2026-05-01T00:00:00Z").unwrap();
    let july = store.keys_as_of("2026-07-01T00:00:00Z").unwrap();

    assert!(
        may.contains(&"fact:employer".to_string()),
        "May must still see it"
    );
    assert!(
        !july.contains(&"fact:employer".to_string()),
        "July must not"
    );
    assert!(
        july.contains(&"fact:city".to_string()),
        "unrelated facts unaffected"
    );

    // The May answer is stable however often it is asked — the property that
    // makes an eval against a mutating store meaningful at all.
    for _ in 0..3 {
        assert_eq!(store.keys_as_of("2026-05-01T00:00:00Z").unwrap(), may);
    }
}

/// Rows written before the migration must not vanish. Their valid-time is
/// backfilled from system time — an approximation we state rather than a
/// claim about the world.
#[tokio::test]
async fn pre_migration_rows_remain_visible() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .write(&mem("fact:legacy", "recorded before G1 existed"), &[])
        .await
        .unwrap();

    assert!(store
        .keys_as_of("2999-01-01T00:00:00Z")
        .unwrap()
        .contains(&"fact:legacy".to_string()));
}
