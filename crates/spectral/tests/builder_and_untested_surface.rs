//! The public facade's least-exercised surface: `BrainBuilder`, and the
//! delegating methods that no test called.
//!
//! Coverage of `spectral/src/lib.rs` sat at 47.9% — the lowest file in the
//! workspace — even though the engines behind it are in the 90s. The facade is
//! ~80 mostly one-line forwards, so the risk class here is not complex logic;
//! it is a **mis-wired forward**: a builder setter whose value never reaches
//! `BrainConfig`, or a method pointed at the wrong inner call. Both are
//! invisible to the type system and to any test that never calls them.
//!
//! Every test below therefore asserts an **observable behavioural
//! consequence** of the setting, not merely that the call compiles or that a
//! getter echoes back what was just set — an echo would pass even if the value
//! never reached the brain.

use spectral::{Brain, RecallTopKConfig, Visibility};
use spectral_core::device_id::DeviceId;
use spectral_graph::brain::EntityPolicy;
use std::path::PathBuf;
use tempfile::TempDir;

/// `auto_ontology` is private, so the public builder path requires an explicit
/// `ontology_path` — exactly as its own error message instructs. This writes
/// the minimal ontology and hands back the path.
fn ontology_in(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("ontology.toml");
    std::fs::write(&p, "version = 1\n").unwrap();
    p
}

/// An ontology declaring one predicate, so `assert()` has something valid to
/// validate against. `entity_policy` governs whether the *entities* may be
/// created; the *predicate* must exist regardless, which is why the minimal
/// `version = 1` ontology is not enough for the assert-based tests.
fn ontology_with_predicate(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("ontology.toml");
    std::fs::write(
        &p,
        "version = 1\n\n\
         [[entity]]\ntype = \"person\"\ncanonical = \"ada\"\naliases = [\"Ada\"]\n\
         visibility = \"private\"\n\n\
         [[entity]]\ntype = \"project\"\ncanonical = \"spectral\"\n\
         aliases = [\"spectral\"]\nvisibility = \"private\"\n\n\
         [[predicate]]\nname = \"works_on\"\ndomain = [\"person\"]\n\
         range = [\"project\"]\n",
    )
    .unwrap();
    p
}

// ── BrainBuilder: required inputs ──────────────────────────────────

#[test]
fn build_without_data_dir_is_an_error() {
    let err = Brain::builder().build().unwrap_err();
    assert!(
        err.to_string().contains("data_dir is required"),
        "got: {err}"
    );
}

#[test]
fn build_without_an_ontology_and_without_auto_is_an_error() {
    let tmp = TempDir::new().unwrap();
    let err = Brain::builder()
        .data_dir(tmp.path())
        .build()
        .expect_err("no ontology_path and no auto_ontology must fail");
    assert!(
        err.to_string().contains("ontology_path is required"),
        "got: {err}"
    );
}

/// `Brain::open` is documented as using `auto_ontology`, which writes a
/// minimal ontology when none exists. Asserted on disk, so a change that
/// stopped writing the file would fail here rather than at some later call.
#[test]
fn open_auto_creates_a_minimal_ontology_file() {
    let tmp = TempDir::new().unwrap();
    assert!(!tmp.path().join("ontology.toml").exists());
    let _brain = Brain::open(tmp.path()).unwrap();
    let written = std::fs::read_to_string(tmp.path().join("ontology.toml")).unwrap();
    assert!(written.contains("version = 1"), "got: {written:?}");
}

/// An explicit `ontology_path` must win over auto-creation, including when it
/// lives outside the data directory.
#[test]
fn explicit_ontology_path_is_used_instead_of_auto() {
    let tmp = TempDir::new().unwrap();
    let elsewhere = TempDir::new().unwrap();
    let onto = elsewhere.path().join("custom-ontology.toml");
    std::fs::write(&onto, "version = 1\n").unwrap();

    let _brain = Brain::builder()
        .data_dir(tmp.path())
        .ontology_path(&onto)
        .build()
        .unwrap();

    assert!(
        !tmp.path().join("ontology.toml").exists(),
        "auto-ontology ran despite an explicit ontology_path"
    );
}

// ── BrainBuilder: settings with observable consequences ────────────

/// `read_only(true)` must actually reach `BrainConfig`. A setter that dropped
/// the flag would hand back a writable brain — the failure mode this asserts
/// against is silent data mutation on a handle the caller believes is inert.
#[test]
fn read_only_setting_actually_makes_the_brain_reject_writes() {
    let tmp = TempDir::new().unwrap();
    // Create it writable first; a read-only open of a nonexistent brain fails
    // for the wrong reason.
    {
        let brain = Brain::open(tmp.path()).unwrap();
        brain
            .remember("seed", "a seeded memory", Visibility::Private)
            .unwrap();
    }

    let ro = Brain::builder()
        .data_dir(tmp.path())
        .ontology_path(tmp.path().join("ontology.toml"))
        .read_only(true)
        .build()
        .unwrap();

    let err = ro
        .remember("blocked", "should not be written", Visibility::Private)
        .expect_err("a read_only brain must reject remember()");
    assert!(
        matches!(err, spectral::Error::ReadOnly(_)),
        "got {err:?}, want Error::ReadOnly"
    );

    // And reads still work, so the flag did not simply break the handle.
    let hits = ro.recall_local("seeded memory").unwrap();
    assert!(
        hits.memory_hits.iter().any(|h| h.key == "seed"),
        "a read_only brain stopped serving reads"
    );
}

/// `memory_db_path` must relocate the memory database. Asserted by where the
/// file lands: a dropped setting would silently write inside `data_dir`, and
/// a caller who pointed at a separate volume would never know.
#[test]
fn memory_db_path_setting_relocates_the_database() {
    let tmp = TempDir::new().unwrap();
    let dbdir = TempDir::new().unwrap();
    let custom = dbdir.path().join("relocated.db");

    let brain = Brain::builder()
        .data_dir(tmp.path())
        .ontology_path(ontology_in(tmp.path()))
        .memory_db_path(&custom)
        .build()
        .unwrap();
    brain
        .remember("k", "content in a relocated db", Visibility::Private)
        .unwrap();
    drop(brain);

    assert!(custom.exists(), "memory_db_path was ignored");
    assert!(
        !tmp.path().join("memory.db").exists(),
        "a default memory.db was created despite an explicit memory_db_path"
    );
}

/// `device_id` must reach the brain rather than a fresh random id being
/// generated. Round-tripped through the accessor, which is itself untested.
#[test]
fn device_id_setting_reaches_the_brain() {
    let tmp = TempDir::new().unwrap();
    let id = DeviceId::from_bytes([7u8; 32]);
    let brain = Brain::builder()
        .data_dir(tmp.path())
        .ontology_path(ontology_in(tmp.path()))
        .device_id(id)
        .build()
        .unwrap();
    assert_eq!(brain.device_id().as_bytes(), &[7u8; 32]);
}

/// `wing_rules` must reach classification. Asserted through the wing a
/// remembered memory is actually filed under — an echo of the config would
/// prove nothing about whether the classifier ever saw it.
#[test]
fn wing_rules_setting_changes_how_a_memory_is_classified() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::builder()
        .data_dir(tmp.path())
        .ontology_path(ontology_in(tmp.path()))
        .wing_rules(vec![("zephyr".to_string(), "zephyr".to_string())])
        .build()
        .unwrap();

    let result = brain
        .remember("w", "the zephyr rollout is scheduled", Visibility::Private)
        .unwrap();
    assert_eq!(
        result.wing.as_deref(),
        Some("zephyr"),
        "custom wing_rules did not reach the classifier"
    );
}

/// A brain built with default rules must NOT file that memory under `zephyr`,
/// which is what makes the previous test meaningful rather than a tautology.
#[test]
fn without_the_custom_rule_the_same_memory_lands_elsewhere() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    let result = brain
        .remember("w", "the zephyr rollout is scheduled", Visibility::Private)
        .unwrap();
    assert_ne!(result.wing.as_deref(), Some("zephyr"));
}

/// `entity_policy` must reach the brain. `Strict` fails `assert()` on an
/// entity the ontology does not know; `AutoCreate` creates it. The two must
/// therefore behave differently on the *same* assertion — if the setting were
/// dropped, both would take the default (`Strict`) and agree.
#[test]
fn entity_policy_setting_reaches_the_brain() {
    let tmp = TempDir::new().unwrap();
    let strict = Brain::builder()
        .data_dir(tmp.path())
        .ontology_path(ontology_with_predicate(tmp.path()))
        .entity_policy(EntityPolicy::Strict)
        .build()
        .unwrap();
    let strict_result = strict.assert("Nobody", "works_on", "spectral", 1.0, Visibility::Private);
    drop(strict);

    let tmp2 = TempDir::new().unwrap();
    let auto = Brain::builder()
        .data_dir(tmp2.path())
        .ontology_path(ontology_with_predicate(tmp2.path()))
        .entity_policy(EntityPolicy::AutoCreate)
        .build()
        .unwrap();
    let auto_result = auto.assert("Nobody", "works_on", "spectral", 1.0, Visibility::Private);

    assert!(
        strict_result.is_err(),
        "Strict accepted an entity absent from an empty ontology: {strict_result:?}"
    );
    assert!(
        auto_result.is_ok(),
        "AutoCreate rejected an unknown entity, so entity_policy may not be \
         reaching BrainConfig: {auto_result:?}"
    );
}

/// `fts_tokenizer` must reach SQLite. An invalid tokenizer name is rejected by
/// FTS5 at table-creation time, so a build that *succeeds* with nonsense would
/// mean the setting never arrived.
#[test]
fn fts_tokenizer_setting_reaches_sqlite() {
    let tmp = TempDir::new().unwrap();
    let result = Brain::builder()
        .data_dir(tmp.path())
        .ontology_path(ontology_in(tmp.path()))
        .fts_tokenizer("definitely_not_a_tokenizer")
        .build();
    assert!(
        result.is_err(),
        "an invalid fts_tokenizer built successfully, so the setting never reached FTS5"
    );

    // And a valid one still builds.
    let tmp2 = TempDir::new().unwrap();
    assert!(Brain::builder()
        .data_dir(tmp2.path())
        .ontology_path(ontology_in(tmp2.path()))
        .fts_tokenizer("porter")
        .build()
        .is_ok());
}

// ── Untested delegating methods ────────────────────────────────────

/// `ontology()` must expose the ontology the brain actually loaded.
#[test]
fn ontology_accessor_returns_the_loaded_ontology() {
    let tmp = TempDir::new().unwrap();
    let onto = tmp.path().join("ontology.toml");
    std::fs::write(
        &onto,
        "version = 1\n\n[[entity]]\ntype = \"person\"\ncanonical = \"ada-lovelace\"\n\
         aliases = [\"Ada\"]\nvisibility = \"private\"\n",
    )
    .unwrap();
    let brain = Brain::builder()
        .data_dir(tmp.path())
        .ontology_path(&onto)
        .build()
        .unwrap();
    let onto = brain.ontology();
    assert_eq!(onto.version, 1);
    assert!(
        onto.entities.iter().any(|e| e.canonical == "ada-lovelace"),
        "the ontology accessor does not reflect the loaded file: {:?}",
        onto.entities
            .iter()
            .map(|e| &e.canonical)
            .collect::<Vec<_>>()
    );
}

/// `set_entity_field` / `get_entity_fields` round-trip, including the
/// documented provenance rule that an `Enriched` write never overwrites a
/// `Manual` one.
#[test]
fn entity_fields_round_trip_and_respect_provenance() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::builder()
        .data_dir(tmp.path())
        .ontology_path(ontology_with_predicate(tmp.path()))
        .entity_policy(EntityPolicy::AutoCreate)
        .build()
        .unwrap();
    let asserted = brain
        .assert("Ada", "works_on", "spectral", 1.0, Visibility::Private)
        .unwrap();
    // Use the entity id the assert actually resolved, rather than guessing at
    // the type prefix the canonicalizer chose.
    let eid = asserted.subject.entity_id;

    assert!(brain
        .set_entity_field(
            &eid,
            "title",
            "engineer",
            spectral_ingest::FieldSource::Manual,
            None
        )
        .unwrap());

    let fields = brain.get_entity_fields(&eid).unwrap();
    assert!(
        fields
            .iter()
            .any(|f| f.field_name == "title" && f.value == "engineer"),
        "set_entity_field did not round-trip through get_entity_fields: {fields:?}"
    );

    // Enriched must not clobber Manual.
    let applied = brain
        .set_entity_field(
            &eid,
            "title",
            "OVERWRITTEN",
            spectral_ingest::FieldSource::Enriched,
            None,
        )
        .unwrap();
    assert!(!applied, "an Enriched write overwrote a Manual field");
    let fields = brain.get_entity_fields(&eid).unwrap();
    assert!(fields.iter().any(|f| f.value == "engineer"));
}

/// The consolidation trio: `consolidate_as` writes the summary and links its
/// sources, `list_consolidated` reports the edges, `list_unconsolidated`
/// excludes anything already consolidated.
#[test]
fn consolidation_helpers_agree_with_each_other() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    for i in 0..3 {
        brain
            .remember(
                &format!("src-{i}"),
                &format!("standup note number {i} about the rollout"),
                Visibility::Private,
            )
            .unwrap();
    }
    brain
        .remember(
            "unrelated",
            "a memory that is never consolidated",
            Visibility::Private,
        )
        .unwrap();

    let sources: Vec<String> = (0..3).map(|i| format!("src-{i}")).collect();
    brain
        .consolidate_as(
            &sources,
            "summary",
            spectral_ingest::CompactionTier::DailyRollup,
            "three standup notes about the rollout",
        )
        .unwrap();

    let edges = brain.list_consolidated(Some("summary")).unwrap();
    assert_eq!(edges.len(), 3, "expected one edge per source: {edges:?}");
    for src in &sources {
        assert!(
            edges.iter().any(|e| &e.source_key == src),
            "missing edge for {src}"
        );
    }

    let unconsolidated = brain.list_unconsolidated(100).unwrap();
    assert!(
        unconsolidated.iter().any(|k| k == "unrelated"),
        "a never-consolidated memory is missing from list_unconsolidated"
    );
    for src in &sources {
        assert!(
            !unconsolidated.contains(src),
            "{src} was consolidated but still appears as unconsolidated"
        );
    }
}

/// `recall_local_at` is the anchored form of `recall_local`. Anchoring is the
/// determinism seam (R20), so the property asserted is that the same anchor
/// gives the same answer twice — not merely that the call returns.
#[test]
fn recall_local_at_is_stable_for_a_fixed_anchor() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    brain
        .remember(
            "a",
            "the deploy runbook lives in notion",
            Visibility::Private,
        )
        .unwrap();

    let anchor = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let first = brain.recall_local_at("deploy runbook", anchor).unwrap();
    let second = brain.recall_local_at("deploy runbook", anchor).unwrap();
    let keys = |r: &spectral_graph::brain::HybridRecallResult| {
        r.memory_hits
            .iter()
            .map(|h| h.key.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(&first), keys(&second));
    assert!(
        !first.memory_hits.is_empty(),
        "anchored recall found nothing"
    );
}

/// `data_dir()` on the builder is a setter, but the untested read path is the
/// pairing of `probe` / `probe_recent`: both must run against an empty brain
/// without erroring, since consumers call `probe_recent` on every chat turn
/// and an error there would break the ambient loop.
#[test]
fn probe_paths_are_safe_on_an_empty_brain() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();

    let probed = brain
        .probe("some ambient context text", Default::default())
        .expect("probe must not error on an empty brain");
    assert!(probed.is_empty());

    let recent = brain
        .probe_recent(Default::default(), Default::default())
        .expect("probe_recent must not error on an empty brain");
    assert!(recent.is_empty());
}

/// `recall_with_provenance` must pair a consolidated summary with the sources
/// behind it — the drill-down the layered-recall API exists for.
#[test]
fn recall_with_provenance_exposes_the_sources_behind_a_summary() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    for i in 0..2 {
        brain
            .remember(
                &format!("prov-src-{i}"),
                &format!("zephyr migration detail number {i}"),
                Visibility::Private,
            )
            .unwrap();
    }
    let sources: Vec<String> = (0..2).map(|i| format!("prov-src-{i}")).collect();
    brain
        .consolidate_as(
            &sources,
            "prov-summary",
            spectral_ingest::CompactionTier::DailyRollup,
            "zephyr migration summary",
        )
        .unwrap();

    let layered = brain
        .recall_with_provenance(
            "zephyr migration",
            &RecallTopKConfig::default(),
            Visibility::Private,
            10,
        )
        .unwrap();

    let summary = layered
        .iter()
        .find(|h| h.hit.key == "prov-summary")
        .expect("the summary should be recalled");
    assert_eq!(
        summary.sources.len(),
        2,
        "the summary did not carry its source memories: {:?}",
        summary.sources
    );
}
