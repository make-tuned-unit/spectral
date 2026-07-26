//! Bi-temporal validity: a changing fact must stop returning its old value.
//!
//! Separating valid time from transaction time (Snodgrass & Ahn, 1985) is what
//! lets a superseded assertion be retired without being deleted. The failure
//! mode this guards against is a stale fact that still scores highly and is
//! therefore returned confidently after it stopped being true.

use chrono::Utc;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
use tempfile::TempDir;

/// Ontology with one functional predicate (`lives_in`) and one accumulating
/// predicate (`attended`), so both branches are covered by the same brain.
const ONTOLOGY: &str = r#"
version = 1

[[entity]]
type = "person"
canonical = "alice"
aliases = ["Alice"]
visibility = "private"

[[entity]]
type = "city"
canonical = "berlin"
aliases = ["Berlin"]
visibility = "private"

[[entity]]
type = "city"
canonical = "lisbon"
aliases = ["Lisbon"]
visibility = "private"

[[entity]]
type = "city"
canonical = "oslo"
aliases = ["Oslo"]
visibility = "private"

[[predicate]]
name = "lives_in"
domain = ["person"]
range = ["city"]
single_valued = true

[[predicate]]
name = "attended"
domain = ["person"]
range = ["city"]
"#;

fn brain(dir: &TempDir) -> Brain {
    let ontology_path = dir.path().join("ontology.toml");
    std::fs::write(&ontology_path, ONTOLOGY).unwrap();
    Brain::open(BrainConfig {
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
    .unwrap()
}

#[test]
fn single_valued_predicate_supersedes_the_stale_fact() {
    let dir = TempDir::new().unwrap();
    let brain = brain(&dir);

    let first = brain
        .assert_typed(
            ("person", "Alice"),
            "lives_in",
            ("city", "Berlin"),
            0.9,
            Visibility::Private,
        )
        .unwrap();
    assert_eq!(
        first.superseded, 0,
        "nothing to supersede on the first write"
    );

    let second = brain
        .assert_typed(
            ("person", "Alice"),
            "lives_in",
            ("city", "Lisbon"),
            0.9,
            Visibility::Private,
        )
        .unwrap();
    assert_eq!(second.superseded, 1, "moving city must retire the old fact");

    // The live view answers "where does Alice live" with exactly one city.
    let live = brain
        .store()
        .find_triples(None, None, Some("lives_in"))
        .unwrap();
    assert_eq!(
        live.len(),
        1,
        "a functional predicate must not leave two live values: {live:?}"
    );

    // Nothing was destroyed — the ledger still holds both.
    let all = brain
        .store()
        .find_triples_including_superseded(None, None, Some("lives_in"))
        .unwrap();
    assert_eq!(all.len(), 2, "superseded facts are retired, not deleted");
    assert_eq!(
        all.iter()
            .filter(|(_, valid_to)| valid_to.is_none())
            .count(),
        1
    );
}

#[test]
fn re_asserting_the_same_value_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let brain = brain(&dir);
    for _ in 0..3 {
        brain
            .assert_typed(
                ("person", "Alice"),
                "lives_in",
                ("city", "Oslo"),
                0.9,
                Visibility::Private,
            )
            .unwrap();
    }
    let all = brain
        .store()
        .find_triples_including_superseded(None, None, Some("lives_in"))
        .unwrap();
    assert_eq!(
        all.len(),
        1,
        "re-stating an unchanged fact must not churn the ledger: {all:?}"
    );
}

#[test]
fn accumulating_predicates_are_unaffected() {
    let dir = TempDir::new().unwrap();
    let brain = brain(&dir);
    for city in ["Berlin", "Lisbon", "Oslo"] {
        let r = brain
            .assert_typed(
                ("person", "Alice"),
                "attended",
                ("city", city),
                0.9,
                Visibility::Private,
            )
            .unwrap();
        assert_eq!(r.superseded, 0, "accumulating predicate must never retire");
    }
    let live = brain
        .store()
        .find_triples(None, None, Some("attended"))
        .unwrap();
    assert_eq!(
        live.len(),
        3,
        "attendance accumulates; it does not supersede"
    );
}

#[test]
fn as_of_query_recovers_the_historical_value() {
    let dir = TempDir::new().unwrap();
    let brain = brain(&dir);
    brain
        .assert_typed(
            ("person", "Alice"),
            "lives_in",
            ("city", "Berlin"),
            0.9,
            Visibility::Private,
        )
        .unwrap();
    let before_move = Utc::now();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    brain
        .assert_typed(
            ("person", "Alice"),
            "lives_in",
            ("city", "Lisbon"),
            0.9,
            Visibility::Private,
        )
        .unwrap();

    let then = brain
        .store()
        .find_triples_as_of(None, None, Some("lives_in"), before_move)
        .unwrap();
    assert_eq!(then.len(), 1, "exactly one city was true at that instant");

    let now = brain
        .store()
        .find_triples(None, None, Some("lives_in"))
        .unwrap();
    assert_eq!(now.len(), 1);
    assert_ne!(
        then[0].to, now[0].to,
        "the as-of answer must differ from the current answer"
    );
}
