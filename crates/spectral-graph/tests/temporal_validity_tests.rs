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

// ── Adjudicated supersession (the Librarian handoff) ─────────────────

use spectral_graph::supersession::{
    apply_adjudications, detect_candidates, Adjudication, Adjudicator, NoOpAdjudicator,
    SupersessionCandidate,
};

/// Stand-in for a consumer-side model. Deterministic so the *gate*, not a
/// model, is what these tests measure.
struct ScriptedAdjudicator {
    verdict: Box<dyn Fn(&SupersessionCandidate) -> Adjudication + Send + Sync>,
}

impl Adjudicator for ScriptedAdjudicator {
    fn adjudicate(&self, c: &SupersessionCandidate) -> anyhow::Result<Adjudication> {
        Ok((self.verdict)(c))
    }
}

fn seed_undeclared_conflict(brain: &Brain) {
    // `attended` is NOT declared single_valued, so both stay live and the slot
    // becomes a candidate for adjudication.
    for city in ["Berlin", "Lisbon"] {
        brain
            .assert_typed(
                ("person", "Alice"),
                "attended",
                ("city", city),
                0.9,
                Visibility::Private,
            )
            .unwrap();
    }
}

#[test]
fn detection_is_deterministic_and_skips_declared_predicates() {
    let dir = TempDir::new().unwrap();
    let brain = brain(&dir);
    seed_undeclared_conflict(&brain);
    // A declared-functional predicate supersedes at write time, so it must
    // never appear as a candidate.
    brain
        .assert_typed(
            ("person", "Alice"),
            "lives_in",
            ("city", "Berlin"),
            0.9,
            Visibility::Private,
        )
        .unwrap();
    brain
        .assert_typed(
            ("person", "Alice"),
            "lives_in",
            ("city", "Oslo"),
            0.9,
            Visibility::Private,
        )
        .unwrap();

    let candidates = detect_candidates(&brain, 100).unwrap();
    assert_eq!(
        candidates.len(),
        1,
        "only the undeclared slot is a candidate"
    );
    assert_eq!(candidates[0].predicate, "attended");
    assert_eq!(candidates[0].objects.len(), 2);
    assert_eq!(candidates[0].subject_canonical, "alice");
}

#[test]
fn default_adjudicator_never_retires_anything() {
    let dir = TempDir::new().unwrap();
    let brain = brain(&dir);
    seed_undeclared_conflict(&brain);

    let report = apply_adjudications(&brain, &NoOpAdjudicator, 100, 0.0, "test").unwrap();
    assert_eq!(report.considered, 1);
    assert_eq!(report.applied, 0);
    assert_eq!(report.retired, 0);
    assert_eq!(report.left_alone, 1);
    assert_eq!(
        brain
            .store()
            .find_triples(None, None, Some("attended"))
            .unwrap()
            .len(),
        2,
        "shipping default must not silently change data"
    );
}

#[test]
fn confidence_gate_blocks_low_confidence_verdicts() {
    let dir = TempDir::new().unwrap();
    let brain = brain(&dir);
    seed_undeclared_conflict(&brain);

    let timid = ScriptedAdjudicator {
        verdict: Box::new(|c| Adjudication::Supersedes {
            keep: c.objects.last().unwrap().object,
            confidence: 0.4,
        }),
    };
    let report = apply_adjudications(&brain, &timid, 100, 0.8, "librarian-7b").unwrap();
    assert_eq!(report.below_threshold, 1);
    assert_eq!(report.retired, 0, "below-threshold verdicts must not apply");
    assert_eq!(
        brain
            .store()
            .find_triples(None, None, Some("attended"))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn hallucinated_object_is_rejected_not_applied() {
    let dir = TempDir::new().unwrap();
    let brain = brain(&dir);
    seed_undeclared_conflict(&brain);

    // Names an entity that is not among the candidate's objects.
    let liar = ScriptedAdjudicator {
        verdict: Box::new(|_| Adjudication::Supersedes {
            keep: spectral_core::entity_id::entity_id("city", "atlantis"),
            confidence: 1.0,
        }),
    };
    let report = apply_adjudications(&brain, &liar, 100, 0.5, "librarian-7b").unwrap();
    assert_eq!(report.invalid_verdicts, 1);
    assert_eq!(report.retired, 0);
    assert_eq!(
        brain
            .store()
            .find_triples(None, None, Some("attended"))
            .unwrap()
            .len(),
        2,
        "an adjudicator must not be able to empty a slot with an invented object"
    );
}

#[test]
fn confident_verdict_applies_and_is_undoable() {
    let dir = TempDir::new().unwrap();
    let brain = brain(&dir);
    seed_undeclared_conflict(&brain);
    let candidates = detect_candidates(&brain, 100).unwrap();
    let keep = candidates[0].objects.last().unwrap().object;

    let confident = ScriptedAdjudicator {
        verdict: Box::new(move |_| Adjudication::Supersedes {
            keep,
            confidence: 0.95,
        }),
    };
    let report = apply_adjudications(&brain, &confident, 100, 0.8, "librarian-7b").unwrap();
    assert_eq!(report.applied, 1);
    assert_eq!(report.retired, 1);

    let live = brain
        .store()
        .find_triples(None, None, Some("attended"))
        .unwrap();
    assert_eq!(live.len(), 1, "the adjudicated slot now holds one value");
    assert_eq!(live[0].to, keep);

    // The retirement is attributed and reversible.
    let ledger = brain
        .store()
        .find_triples_including_superseded(None, None, Some("attended"))
        .unwrap();
    assert_eq!(ledger.len(), 2, "retired, not deleted");

    let survivor_rowid = brain.store().multi_valued_live_groups(100).unwrap().len();
    assert_eq!(survivor_rowid, 0, "no live conflicts remain");

    // Undo via the surviving assertion's rowid.
    let keep_rowid = candidates[0]
        .objects
        .iter()
        .find(|o| o.object == keep)
        .unwrap()
        .rowid;
    let reinstated = brain.store().undo_supersession(keep_rowid).unwrap();
    assert_eq!(reinstated, 1, "a wrong automated call must be reversible");
    let after_undo = brain
        .store()
        .find_triples(None, None, Some("attended"))
        .unwrap();
    assert_eq!(
        after_undo.len(),
        1,
        "undo swaps rather than leaving both live: {after_undo:?}"
    );
    assert_ne!(after_undo[0].to, keep, "the retired value is live again");
}

/// Cardinality is scoped by the ontology `domain`: the same predicate name can
/// be functional for one subject type and accumulating for another. Requested
/// by Permagent so `person.location` can retire without touching
/// `org.location`.
#[test]
fn cardinality_is_scoped_by_subject_type() {
    const SCOPED: &str = r#"
version = 1

[[entity]]
type = "person"
canonical = "alice"
aliases = ["Alice"]
visibility = "private"

[[entity]]
type = "org"
canonical = "acme"
aliases = ["Acme"]
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

[[predicate]]
name = "location"
domain = ["person", "org"]
range = ["city"]
single_valued_for = ["person"]
"#;
    let dir = TempDir::new().unwrap();
    let ontology_path = dir.path().join("ontology.toml");
    std::fs::write(&ontology_path, SCOPED).unwrap();
    let brain = Brain::open(BrainConfig {
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
    .unwrap();

    // person.location IS functional — moving retires the old value.
    for city in ["Berlin", "Lisbon"] {
        brain
            .assert_typed(
                ("person", "Alice"),
                "location",
                ("city", city),
                0.9,
                Visibility::Private,
            )
            .unwrap();
    }
    // org.location is NOT — an org may sit in several cities.
    for city in ["Berlin", "Lisbon"] {
        let r = brain
            .assert_typed(
                ("org", "Acme"),
                "location",
                ("city", city),
                0.9,
                Visibility::Private,
            )
            .unwrap();
        assert_eq!(
            r.superseded, 0,
            "org.location must accumulate even though person.location is functional"
        );
    }

    let live = brain
        .store()
        .find_triples(None, None, Some("location"))
        .unwrap();
    let alice = spectral_core::entity_id::entity_id("person", "alice");
    let acme = spectral_core::entity_id::entity_id("org", "acme");
    assert_eq!(
        live.iter().filter(|t| t.from == alice).count(),
        1,
        "person.location retired the stale value"
    );
    assert_eq!(
        live.iter().filter(|t| t.from == acme).count(),
        2,
        "org.location kept both"
    );
}
