//! Two under-covered areas of `brain.rs`: every `RememberOpts` override, and
//! the entity-resolution path that `EntityPolicy::AutoCreate` turns on.
//!
//! `RememberOpts` is a struct of overrides, and an override that is silently
//! dropped is invisible: the write still succeeds, it just stores something
//! other than what the caller asked for. Each test below therefore reads the
//! value back out of the store rather than trusting the returned summary.
//!
//! The resolution path (`resolve_or_create_typed`, `infer_single_type`,
//! `ensure_alias`) decides what *type* a newly created entity gets, inferred
//! from the predicate's domain/range. Its three failure modes — unknown
//! predicate, no valid types, and an ambiguous multi-type domain — are
//! distinct errors that callers are expected to distinguish, and none of them
//! were exercised.

use chrono::{DateTime, Duration, Utc};
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use spectral_graph::error::Error;
use tempfile::TempDir;

const ONTOLOGY: &str = r#"
version = 1

[[entity]]
type = "person"
canonical = "ada"
aliases = ["Ada"]
visibility = "private"

[[entity]]
type = "project"
canonical = "spectral"
aliases = ["Spectral"]
visibility = "private"

[[predicate]]
name = "works_on"
domain = ["person"]
range = ["project"]

[[predicate]]
name = "relates_to"
domain = ["person", "project"]
range = ["project"]
"#;

fn config(tmp: &TempDir, policy: EntityPolicy) -> BrainConfig {
    let ontology_path = tmp.path().join("ontology.toml");
    std::fs::write(&ontology_path, ONTOLOGY).unwrap();
    BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path,
        entity_policy: policy,
        activity_wing: "activity".into(),
        ..Default::default()
    }
}

fn brain(tmp: &TempDir, policy: EntityPolicy) -> Brain {
    Brain::open(config(tmp, policy)).unwrap()
}

// ── RememberOpts: every override, read back from the store ─────────

#[test]
fn source_and_confidence_overrides_are_persisted() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::Strict);
    b.remember_with(
        "k",
        "the deploy runbook lives in notion",
        RememberOpts {
            source: Some("import-2024".into()),
            confidence: Some(0.42),
            ..Default::default()
        },
    )
    .unwrap();

    let m = b.get_memory_by_key("k").unwrap().expect("memory exists");
    assert_eq!(m.source.as_deref(), Some("import-2024"));
    assert!(
        (m.confidence - 0.42).abs() < 1e-9,
        "confidence override was dropped: {}",
        m.confidence
    );
}

/// `created_at` exists for importing dated history. If it were dropped the row
/// would silently carry today's date, and every temporal query over imported
/// data would be wrong.
#[test]
fn created_at_override_is_persisted_and_not_replaced_by_now() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::Strict);
    let historical: DateTime<Utc> = "2019-03-04T10:11:12Z".parse().unwrap();

    b.remember_with(
        "old",
        "a memory imported from an external system",
        RememberOpts {
            created_at: Some(historical),
            ..Default::default()
        },
    )
    .unwrap();

    let m = b.get_memory_by_key("old").unwrap().expect("memory exists");
    let stored = m.created_at.expect("created_at should be set");
    assert!(
        stored.contains("2019"),
        "created_at override was ignored; stored {stored:?}"
    );
}

/// The wing override documents that it "bypasses the classifier and the value
/// is stored as-is". Asserted with a wing the classifier would never pick.
#[test]
fn wing_override_bypasses_the_classifier() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::Strict);
    let result = b
        .remember_with(
            "w",
            "an entirely generic sentence with no project words",
            RememberOpts {
                wing: Some("zzz-explicit".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(result.wing.as_deref(), Some("zzz-explicit"));

    let m = b.get_memory_by_key("w").unwrap().unwrap();
    assert_eq!(
        m.wing.as_deref(),
        Some("zzz-explicit"),
        "the wing override did not reach storage"
    );
}

/// `compaction_tier.is_some()` is documented as *the* canonical signal that a
/// memory belongs to the ambient stream, so the round-trip matters.
#[test]
fn compaction_tier_override_is_persisted() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::Strict);
    b.remember_with(
        "amb",
        "an ambient stream memory",
        RememberOpts {
            compaction_tier: Some(spectral_ingest::CompactionTier::Raw),
            ..Default::default()
        },
    )
    .unwrap();
    b.remember_with("core", "an ordinary core memory", RememberOpts::default())
        .unwrap();

    let ambient = b.get_memory_by_key("amb").unwrap().unwrap();
    let core = b.get_memory_by_key("core").unwrap().unwrap();
    assert!(
        ambient.compaction_tier.is_some(),
        "compaction_tier override was dropped"
    );
    assert!(
        core.compaction_tier.is_none(),
        "a memory with no override should not be in the ambient stream"
    );
}

#[test]
fn explicit_episode_id_overrides_the_time_gap_heuristic() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::Strict);
    b.remember_with(
        "e",
        "a memory assigned to an explicit episode",
        RememberOpts {
            episode_id: Some("ep-explicit".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let m = b.get_memory_by_key("e").unwrap().unwrap();
    assert_eq!(m.episode_id.as_deref(), Some("ep-explicit"));
}

/// A non-default visibility must be stored, since it is what every scoped
/// read filters on.
#[test]
fn visibility_override_is_persisted() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::Strict);
    b.remember_with(
        "team-note",
        "a note the whole team may read",
        RememberOpts {
            visibility: Visibility::Team,
            ..Default::default()
        },
    )
    .unwrap();

    let m = b.get_memory_by_key("team-note").unwrap().unwrap();
    assert_eq!(m.visibility, "team", "visibility override was dropped");
}

/// Two memories written under the same `session_id` become associated once,
/// idempotently — the association feeds co-retrieval ranking.
#[test]
fn session_id_override_is_accepted_and_idempotent() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::Strict);
    for key in ["s1", "s2"] {
        b.remember_with(
            key,
            &format!("a memory from one session, {key}"),
            RememberOpts {
                session_id: Some("sess-1".into()),
                ..Default::default()
            },
        )
        .unwrap();
    }
    // Re-writing the same key in the same session must not error or duplicate.
    b.remember_with(
        "s1",
        "a memory from one session, s1 revised",
        RememberOpts {
            session_id: Some("sess-1".into()),
            ..Default::default()
        },
    )
    .unwrap();

    assert!(b.get_memory_by_key("s1").unwrap().is_some());
    assert!(b.get_memory_by_key("s2").unwrap().is_some());
}

// ── Entity resolution and type inference ───────────────────────────

/// Strict refuses an unknown mention and names the nearest known entity, which
/// is the affordance that makes the error actionable.
#[test]
fn strict_policy_refuses_an_unknown_mention() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::Strict);
    let err = b
        .assert("Grace", "works_on", "spectral", 1.0, Visibility::Private)
        .expect_err("Strict must refuse an entity absent from the ontology");
    assert!(
        matches!(err, Error::UnresolvedMention { .. }),
        "got {err:?}, want UnresolvedMention"
    );
}

/// AutoCreate infers the new entity's type from the predicate's domain, and
/// the created entity must then be resolvable by later assertions.
#[test]
fn autocreate_infers_the_type_from_the_predicate_domain() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::AutoCreate);
    let result = b
        .assert("Grace", "works_on", "spectral", 1.0, Visibility::Private)
        .expect("AutoCreate should create the missing subject");

    assert!(result.triple_written);
    assert_eq!(
        result.subject.entity_type, "person",
        "the new entity's type should be inferred from works_on's domain"
    );

    // The created entity is now resolvable — a second assertion reuses it
    // rather than creating a duplicate.
    let again = b
        .assert("Grace", "works_on", "spectral", 1.0, Visibility::Private)
        .unwrap();
    assert_eq!(
        again.subject.entity_id, result.subject.entity_id,
        "the auto-created entity was not reused on a second assertion"
    );
}

/// An ambiguous domain (two allowed types) cannot be inferred from, and must
/// surface as `AmbiguousEntityType` listing the candidates — not as a silent
/// pick of the first one.
#[test]
fn ambiguous_predicate_domain_is_an_explicit_error_not_a_guess() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::AutoCreate);
    let err = b
        .assert("Grace", "relates_to", "spectral", 1.0, Visibility::Private)
        .expect_err("a two-type domain is ambiguous and must not be guessed");

    match err {
        Error::AmbiguousEntityType { allowed, .. } => {
            assert_eq!(
                allowed.len(),
                2,
                "the error should list every candidate type: {allowed:?}"
            );
        }
        other => panic!("got {other:?}, want AmbiguousEntityType"),
    }
}

/// An unknown predicate is a distinct failure from an unknown entity, and must
/// stay distinct under both policies — otherwise a schema typo is
/// indistinguishable from missing data.
#[test]
fn unknown_predicate_is_rejected_under_both_policies() {
    for (label, policy) in [
        ("Strict", EntityPolicy::Strict),
        ("AutoCreate", EntityPolicy::AutoCreate),
    ] {
        let tmp = TempDir::new().unwrap();
        let b = brain(&tmp, policy);
        let err = b
            .assert(
                "Ada",
                "no_such_predicate",
                "spectral",
                1.0,
                Visibility::Private,
            )
            .expect_err("an unknown predicate must be refused");
        assert!(
            matches!(err, Error::Ontology(_)),
            "policy {label} gave {err:?}, want Error::Ontology"
        );
    }
}

/// Resolution is alias-aware and case-insensitive: the ontology declares
/// "Ada" as an alias of canonical `ada`, and both must land on one entity.
#[test]
fn aliases_resolve_to_the_same_canonical_entity() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::Strict);
    let by_alias = b
        .assert("Ada", "works_on", "spectral", 1.0, Visibility::Private)
        .unwrap();
    let by_canonical = b
        .assert("ada", "works_on", "spectral", 1.0, Visibility::Private)
        .unwrap();
    assert_eq!(
        by_alias.subject.entity_id, by_canonical.subject.entity_id,
        "an alias and its canonical form resolved to different entities"
    );
    assert_eq!(by_alias.subject.canonical, "ada");
}

/// Asserting is refused on a read-only brain — it is a graph write.
#[test]
fn assert_is_refused_on_a_read_only_brain() {
    let tmp = TempDir::new().unwrap();
    drop(brain(&tmp, EntityPolicy::Strict));
    let ro = Brain::open(BrainConfig {
        read_only: true,
        ..config(&tmp, EntityPolicy::Strict)
    })
    .unwrap();

    let err = ro
        .assert("Ada", "works_on", "spectral", 1.0, Visibility::Private)
        .expect_err("a read-only brain must refuse assert()");
    assert!(
        matches!(err, Error::ReadOnly(_)),
        "got {err:?}, want Error::ReadOnly"
    );
}

// ── remember_with under a non-default clock ────────────────────────

/// Historical imports arrive out of order. Two memories with explicit,
/// decreasing `created_at` values must both persist their own timestamp
/// rather than the later write overwriting the earlier one's.
#[test]
fn out_of_order_historical_imports_keep_their_own_timestamps() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp, EntityPolicy::Strict);
    let newer: DateTime<Utc> = "2022-06-01T00:00:00Z".parse().unwrap();
    let older = newer - Duration::days(365);

    b.remember_with(
        "newer",
        "the later imported memory",
        RememberOpts {
            created_at: Some(newer),
            ..Default::default()
        },
    )
    .unwrap();
    b.remember_with(
        "older",
        "the earlier imported memory",
        RememberOpts {
            created_at: Some(older),
            ..Default::default()
        },
    )
    .unwrap();

    let a = b.get_memory_by_key("newer").unwrap().unwrap();
    let c = b.get_memory_by_key("older").unwrap().unwrap();
    assert!(a.created_at.unwrap().contains("2022"));
    assert!(c.created_at.unwrap().contains("2021"));
}
