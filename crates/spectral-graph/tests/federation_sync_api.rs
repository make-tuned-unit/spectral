//! The federation sync surface on `Brain`, exercised as a whole exchange.
//!
//! `share_memory`, `export_shared_wing`, `import_shared_wing`,
//! `shared_wing_hashes`, `shared_wing_want` and `tombstone_shared` are the six
//! public methods a peer implementation drives. Each is a thin forward into
//! `federation_sync`, which is the mis-wired-forward risk class: a method
//! pointed at the wrong inner call still compiles and still returns `Ok`.
//!
//! Rather than assert each wrapper in isolation, these tests run the exchange
//! two real brains would: share → advertise → want → export → import → recall
//! with provenance → retract. A wrapper wired to the wrong call breaks the
//! chain somewhere observable.

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
use spectral_graph::federation_recall::RealmScope;
use std::path::PathBuf;
use tempfile::TempDir;

fn brain(tmp: &TempDir) -> Brain {
    Brain::open(BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
        entity_policy: EntityPolicy::Strict,
        activity_wing: "activity".into(),
        ..Default::default()
    })
    .unwrap()
}

fn keys_of(
    hits: &[(
        spectral_ingest::MemoryHit,
        spectral_ingest::federation_sync::Origin,
    )],
) -> Vec<String> {
    hits.iter().map(|(h, _)| h.key.clone()).collect()
}

// ── the whole exchange ─────────────────────────────────────────────

/// The full round-trip between two brains. Every wrapper participates, and the
/// assertions are on what the RECEIVING brain can then see.
#[test]
fn a_shared_wing_replicates_from_one_brain_to_another() {
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();
    let a = brain(&tmp_a);
    let b = brain(&tmp_b);

    a.remember(
        "runbook",
        "the deploy runbook lives in notion",
        Visibility::Team,
    )
    .unwrap();
    a.remember("secret", "my private grocery list", Visibility::Private)
        .unwrap();

    // A shares ONE memory into the wing.
    let hash = a.share_memory("runbook", "team-ops").unwrap();
    assert_eq!(hash.len(), 64, "share should return the object hash");

    // A advertises what it holds; B computes what it wants.
    let advertised = a.shared_wing_hashes("team-ops").unwrap();
    assert_eq!(advertised, vec![hash.clone()]);

    let wanted = b.shared_wing_want("team-ops", &advertised).unwrap();
    assert_eq!(
        wanted,
        vec![hash.clone()],
        "B holds nothing, so it should want everything advertised"
    );

    // B imports A's pack.
    let pack = a.export_shared_wing("team-ops").unwrap();
    assert_eq!(pack.objects.len(), 1, "only the shared memory may travel");
    assert!(
        !pack.objects.iter().any(|o| o.content.contains("grocery")),
        "an unshared private memory leaked into the pack"
    );

    let merged = b.import_shared_wing(&pack).unwrap();
    assert_eq!(merged, 1);

    // B can now recall it, tagged as shared.
    let hits = b.recall_scoped("deploy runbook", RealmScope::All).unwrap();
    let shared_origin = hits
        .iter()
        .find(|(h, _)| h.content.contains("deploy runbook"))
        .map(|(_, o)| o.clone())
        .expect("B should recall the imported memory");
    assert!(
        matches!(
            shared_origin,
            spectral_ingest::federation_sync::Origin::Shared { .. }
        ),
        "the imported memory should carry Shared provenance, got {shared_origin:?}"
    );

    // And after the exchange B wants nothing more.
    let still_wanted = b.shared_wing_want("team-ops", &advertised).unwrap();
    assert!(
        still_wanted.is_empty(),
        "B still wants {still_wanted:?} after importing everything"
    );
}

/// Re-importing the same pack must be a no-op, since sync is an OR-Set union
/// over content-addressed objects. A peer that gossips the same pack twice
/// must not duplicate anything.
#[test]
fn re_importing_the_same_pack_merges_nothing_new() {
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();
    let a = brain(&tmp_a);
    let b = brain(&tmp_b);

    a.remember("note", "a shared team note", Visibility::Team)
        .unwrap();
    a.share_memory("note", "team-ops").unwrap();
    let pack = a.export_shared_wing("team-ops").unwrap();

    assert_eq!(b.import_shared_wing(&pack).unwrap(), 1);
    assert_eq!(
        b.import_shared_wing(&pack).unwrap(),
        0,
        "a second import of the same pack merged new objects"
    );

    let hits = b
        .recall_scoped("shared team note", RealmScope::All)
        .unwrap();
    let matching = hits
        .iter()
        .filter(|(h, _)| h.content.contains("a shared team note"))
        .count();
    assert_eq!(matching, 1, "the memory was duplicated by re-import");
}

/// A retraction must remove the object from the wing and survive a re-import
/// of a stale pack that still contains it — the tombstone dominates.
#[test]
fn a_tombstone_removes_the_object_and_resists_resurrection() {
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();
    let a = brain(&tmp_a);
    let b = brain(&tmp_b);

    a.remember("note", "a retractable team note", Visibility::Team)
        .unwrap();
    let hash = a.share_memory("note", "team-ops").unwrap();
    let stale_pack = a.export_shared_wing("team-ops").unwrap();
    b.import_shared_wing(&stale_pack).unwrap();

    // B retracts it locally.
    b.tombstone_shared("team-ops", &hash).unwrap();
    assert!(
        b.shared_wing_hashes("team-ops").unwrap().is_empty(),
        "the retracted object is still advertised"
    );

    // A peer re-sends the stale pack that still contains the object.
    b.import_shared_wing(&stale_pack).unwrap();
    assert!(
        b.shared_wing_hashes("team-ops").unwrap().is_empty(),
        "a stale pack resurrected a retracted object"
    );
    let hits = b
        .recall_scoped("retractable team note", RealmScope::All)
        .unwrap();
    assert!(
        !hits.iter().any(|(h, _)| h.content.contains("retractable")),
        "a retracted memory is still recallable"
    );
}

// ── scope filtering ────────────────────────────────────────────────

/// The three scopes must partition what a caller sees. `Shared` is the
/// sovereign case: it must never surface a private memory.
#[test]
fn the_three_realm_scopes_partition_what_is_visible() {
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();
    let a = brain(&tmp_a);
    let b = brain(&tmp_b);

    // B has a private memory of its own that matches the same query.
    b.remember(
        "mine",
        "rollout planning notes, private",
        Visibility::Private,
    )
    .unwrap();
    // A shares one.
    a.remember("theirs", "rollout planning notes, shared", Visibility::Team)
        .unwrap();
    a.share_memory("theirs", "team-ops").unwrap();
    b.import_shared_wing(&a.export_shared_wing("team-ops").unwrap())
        .unwrap();

    let all = keys_of(
        &b.recall_scoped("rollout planning", RealmScope::All)
            .unwrap(),
    );
    let private = keys_of(
        &b.recall_scoped("rollout planning", RealmScope::Private)
            .unwrap(),
    );
    let shared = b
        .recall_scoped(
            "rollout planning",
            RealmScope::Shared(vec!["team-ops".into()]),
        )
        .unwrap();

    assert!(
        all.contains(&"mine".to_string()),
        "All should include private"
    );
    assert!(all.len() >= 2, "All should span both realms, got {all:?}");

    assert!(private.contains(&"mine".to_string()));
    // Assert on CONTENT, not the key: an imported object is stored under a
    // synthetic local key (`author::key::hash`), so matching on "theirs" would
    // never fire and the assertion could not fail.
    let private_hits = b
        .recall_scoped("rollout planning", RealmScope::Private)
        .unwrap();
    assert!(
        !private_hits
            .iter()
            .any(|(h, _)| h.content.contains("shared")),
        "the Private scope surfaced a shared memory: {:?}",
        private_hits
            .iter()
            .map(|(h, _)| &h.content)
            .collect::<Vec<_>>()
    );

    assert!(
        !shared.iter().any(|(h, _)| h.key == "mine"),
        "the Shared scope surfaced a PRIVATE memory — this is the sovereign case"
    );
    assert!(
        shared.iter().any(|(h, _)| h.content.contains("shared")),
        "the Shared scope did not surface the shared memory"
    );
}

/// A `Shared` scope naming a wing the brain does not have must return nothing
/// rather than falling back to everything.
#[test]
fn a_shared_scope_for_an_unknown_wing_returns_nothing() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember("mine", "rollout planning notes", Visibility::Private)
        .unwrap();

    let hits = b
        .recall_scoped(
            "rollout planning",
            RealmScope::Shared(vec!["no-such-wing".into()]),
        )
        .unwrap();
    assert!(
        hits.is_empty(),
        "an unknown wing scope fell back to showing something: {:?}",
        keys_of(&hits)
    );
}

/// An empty `Shared` list names no wings, so it admits nothing.
#[test]
fn an_empty_shared_scope_admits_nothing() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    b.remember("mine", "rollout planning notes", Visibility::Private)
        .unwrap();
    assert!(b
        .recall_scoped("rollout planning", RealmScope::Shared(vec![]))
        .unwrap()
        .is_empty());
}

// ── the wrappers on an untouched brain ─────────────────────────────

/// Every read-side wrapper must behave on a brain that has never federated —
/// the sync tables do not exist yet, and none of these may fail with
/// "no such table".
#[test]
fn the_read_wrappers_are_safe_on_a_brain_that_never_federated() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);

    assert!(b.shared_wing_hashes("never-used").unwrap().is_empty());
    assert_eq!(
        b.shared_wing_want("never-used", &["deadbeef".repeat(8)])
            .unwrap()
            .len(),
        1,
        "a virgin brain should want everything a peer advertises"
    );
    assert!(b
        .export_shared_wing("never-used")
        .unwrap()
        .objects
        .is_empty());
}

/// Sharing a key that does not exist must be an error, not a silent no-op that
/// leaves the caller believing a memory was shared.
#[test]
fn sharing_an_unknown_key_is_an_error() {
    let tmp = TempDir::new().unwrap();
    let b = brain(&tmp);
    assert!(
        b.share_memory("no-such-key", "team-ops").is_err(),
        "sharing a nonexistent key silently succeeded"
    );
}
