//! An enrolment whose memory row was deleted out from under it — and what
//! `recognize()` does with it.
//!
//! Permagent's Librarian pruning deletes noise memories with raw SQL
//! (`routes/librarian/pruning.rs`), relying on `PRAGMA foreign_keys = ON` to
//! cascade to the child tables. That cascade cannot reach the recognition
//! sidecar, because `recognition.db` is a **separate database file** and no
//! foreign key crosses files. Measured on the real brain 2026-08-19: two
//! pruned memories left 49 recognition rows behind.
//!
//! The question this pins is what the *consumer* sees, since a verdict naming
//! a memory that no longer exists is a broken contract.

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use spectral_recognition::Verdict;

fn test_brain(dir: &std::path::Path) -> Brain {
    let ontology_path = dir.join("ontology.toml");
    std::fs::write(&ontology_path, "version = 1\n").unwrap();
    Brain::open(BrainConfig {
        data_dir: dir.to_path_buf(),
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
        ..Default::default()
    })
    .unwrap()
}

const DOOMED: &str =
    "The nightly export of ledger 88213 finished in 4471ms writing 219 rows to archive-west";

/// Deleting a memory the way a consumer's pruning pass does — raw SQL on
/// `memories` — leaves its recognition trace enrolled. `recognize()` must not
/// hand the consumer an identity it cannot resolve: the evidence is real, so
/// the verdict degrades to `Familiar` and the dangling candidate disappears
/// from `traces`.
#[test]
fn a_raw_deleted_memory_never_yields_an_unresolvable_identity() {
    let dir = tempfile::tempdir().unwrap();
    let brain = test_brain(dir.path());
    let opts = || RememberOpts {
        visibility: Visibility::Private,
        ..Default::default()
    };
    let doomed = brain.remember_with("k_doomed", DOOMED, opts()).unwrap();
    brain
        .remember_with("k_other", "An unrelated note about the garden gate", opts())
        .unwrap();

    // Precondition: it is recognised while it exists.
    assert_eq!(
        brain.recognize(DOOMED).unwrap().verdict,
        Verdict::Recognized {
            memory_id: doomed.memory_id.clone()
        },
    );

    // Delete exactly as the consumer's pruning pass does: raw SQL with FKs on.
    // The cascade cleans same-database children and cannot touch the
    // recognition sidecar, which is a different file.
    {
        let conn = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        let n = conn
            .execute(
                "DELETE FROM memories WHERE id = ?1",
                [&doomed.memory_id as &dyn rusqlite::ToSql],
            )
            .unwrap();
        assert_eq!(n, 1, "precondition: the row was deleted");
    }
    assert!(
        brain.get_memory(&doomed.memory_id).unwrap().is_none(),
        "precondition: the memory is gone from the store"
    );

    // The contract: never name a memory the consumer cannot fetch.
    let r = brain.recognize(DOOMED).unwrap();
    assert!(
        !matches!(&r.verdict, Verdict::Recognized { memory_id } if memory_id == &doomed.memory_id),
        "recognize() named a deleted memory: {:?}",
        r.verdict
    );
    assert_eq!(
        r.verdict,
        Verdict::Familiar,
        "the evidence was real, so the honest verdict is Familiar — not Recognized, not Novel"
    );
    assert!(
        !r.traces.iter().any(|t| t.memory_id == doomed.memory_id),
        "a dangling candidate must not be returned; it can out-rank live memories"
    );
    assert!(
        !r.evidence.iter().any(|e| e.memory_id == doomed.memory_id),
        "and its evidence rows must not be cited"
    );

    // Every id a verdict or trace names must be fetchable — the property the
    // consumer actually depends on.
    for t in &r.traces {
        assert!(
            brain.get_memory(&t.memory_id).unwrap().is_some(),
            "trace names an unfetchable memory: {}",
            t.memory_id
        );
    }
}

/// A live memory must still be recognised normally when an orphan is present —
/// the guard withdraws the dangling identity, it does not suppress recognition.
#[test]
fn an_orphan_does_not_suppress_recognition_of_live_memories() {
    let dir = tempfile::tempdir().unwrap();
    let brain = test_brain(dir.path());
    let opts = || RememberOpts {
        visibility: Visibility::Private,
        ..Default::default()
    };
    let doomed = brain.remember_with("k_doomed", DOOMED, opts()).unwrap();
    let live_text = "Quarterly review moved to the Halifax office on 14 March, agenda unchanged";
    let live = brain.remember_with("k_live", live_text, opts()).unwrap();
    {
        let conn = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute(
            "DELETE FROM memories WHERE id = ?1",
            [&doomed.memory_id as &dyn rusqlite::ToSql],
        )
        .unwrap();
    }
    assert_eq!(
        brain.recognize(live_text).unwrap().verdict,
        Verdict::Recognized {
            memory_id: live.memory_id
        },
        "the orphan must not cost a live memory its identity"
    );
}

/// `forget()` is the path that does clean the sidecar — the fix available to
/// the consumer today, and the contrast that makes the bug above a choice of
/// deletion path rather than an inevitability.
#[test]
fn forget_removes_the_recognition_trace_that_raw_delete_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let brain = test_brain(dir.path());
    brain
        .remember_with(
            "k_doomed",
            DOOMED,
            RememberOpts {
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(matches!(
        brain.recognize(DOOMED).unwrap().verdict,
        Verdict::Recognized { .. }
    ));

    brain.forget("k_doomed").unwrap();

    let r = brain.recognize(DOOMED).unwrap();
    assert_eq!(
        r.verdict,
        Verdict::Novel,
        "after forget() the trace is gone, so the stimulus is genuinely novel"
    );
    assert!(r.traces.is_empty(), "no dangling candidates remain");
}

/// `recognition_residue()` is substrate truth, and it must answer the question
/// the guarded `recognize()` deliberately refuses to: *is the trace still
/// there?* — independent of whether it is strong enough to win a verdict.
///
/// This is the API a deletion audit needs. It is separate from the consumer
/// path on purpose: hiding a dangling identity from a consumer must not make
/// residue undetectable to an auditor, which is exactly the regression the
/// deletion-guarantee suite caught when the guard was first written.
#[test]
fn recognition_residue_reports_substrate_truth_not_the_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let brain = test_brain(dir.path());
    let m = brain
        .remember_with(
            "k_doomed",
            DOOMED,
            RememberOpts {
                visibility: Visibility::Private,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        brain.recognition_residue(&m.memory_id).unwrap(),
        "an enrolled memory is present in the index"
    );
    assert!(
        !brain.recognition_residue("never-enrolled").unwrap(),
        "an id that was never enrolled is not"
    );

    // Raw delete: the consumer view goes quiet, the auditor view does not.
    {
        let conn = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute(
            "DELETE FROM memories WHERE id = ?1",
            [&m.memory_id as &dyn rusqlite::ToSql],
        )
        .unwrap();
    }
    assert!(
        !matches!(
            brain.recognize(DOOMED).unwrap().verdict,
            Verdict::Recognized { .. }
        ),
        "consumer: no unresolvable identity"
    );
    assert!(
        brain.recognition_residue(&m.memory_id).unwrap(),
        "auditor: the residue is STILL detectable — the guard must not launder it"
    );

    // forget() purges it, and then both views agree.
    brain.forget("k_doomed").unwrap();
    assert!(
        !brain.recognition_residue(&m.memory_id).unwrap(),
        "forget() clears the sidecar"
    );
}
