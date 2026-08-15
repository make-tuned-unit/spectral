//! `forget` must remove EVERY trace of a memory from the recognition index.
//!
//! The trait doc states the stakes plainly: this is "required for hard delete
//! / right-to-be-forgotten: without it, `recognize()` keeps surfacing content
//! whose source memory was deleted."
//!
//! The index spreads one memory across five tables — `recognition_enrolled`,
//! `recognition_pairs`, `recognition_grams`, `recognition_minhash_sig` and
//! `recognition_minhash_bands`. A forget that cleared four of them would still
//! make `recognize()` return Novel in the common case while leaving residue on
//! disk, so these tests assert **per-table** emptiness rather than trusting the
//! behavioural probe alone.

use spectral_recognition::{
    RecognitionConfig, RecognitionEngine, RecognitionStore, SqliteRecognitionStore, Verdict,
};
use tempfile::TempDir;

const CONTENT: &str = "the deploy runbook lives in notion and covers rollback \
                       procedures for the zephyr rollout, including the \
                       staging cutover and the incident escalation path";

fn engine(
    dir: &TempDir,
) -> (
    RecognitionEngine<SqliteRecognitionStore>,
    std::path::PathBuf,
) {
    let path = dir.path().join("recognition.db");
    let store = SqliteRecognitionStore::open(&path).unwrap();
    (
        RecognitionEngine::new(store, RecognitionConfig::default()),
        path,
    )
}

/// Row counts across every table the index writes to.
fn substrate_counts(path: &std::path::Path) -> Vec<(&'static str, i64)> {
    let conn = rusqlite::Connection::open(path).unwrap();
    [
        "recognition_enrolled",
        "recognition_pairs",
        "recognition_grams",
        "recognition_minhash_sig",
        "recognition_minhash_bands",
    ]
    .iter()
    .map(|t| {
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
            .unwrap_or(0);
        (*t, n)
    })
    .collect()
}

#[test]
fn forget_clears_every_substrate_table() {
    let dir = TempDir::new().unwrap();
    let (mut e, path) = engine(&dir);
    e.enroll("m1", CONTENT).unwrap();

    let before = substrate_counts(&path);
    let populated: Vec<&str> = before
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(t, _)| *t)
        .collect();
    assert!(
        populated.len() >= 3,
        "precondition: enrolment should populate several substrates, got {before:?}"
    );

    assert!(
        e.forget("m1").unwrap(),
        "forget should report the memory existed"
    );

    for (table, n) in substrate_counts(&path) {
        assert_eq!(
            n, 0,
            "{table} still holds {n} row(s) after forget — residue of a \
             deleted memory remains on disk"
        );
    }
}

/// The behavioural half: after forgetting, the exact content must no longer be
/// recognised. Asserted alongside the per-table check because either one alone
/// can pass while the other fails.
#[test]
fn forgotten_content_is_no_longer_recognised() {
    let dir = TempDir::new().unwrap();
    let (mut e, _path) = engine(&dir);
    e.enroll("m1", CONTENT).unwrap();

    assert!(
        matches!(
            e.recognize(CONTENT).unwrap().verdict,
            Verdict::Recognized { .. }
        ),
        "precondition: verbatim content should be recognised before forget"
    );

    e.forget("m1").unwrap();

    assert!(
        !matches!(
            e.recognize(CONTENT).unwrap().verdict,
            Verdict::Recognized { .. }
        ),
        "recognition still surfaces content whose memory was forgotten"
    );
}

/// Forgetting one memory must not disturb another — the blast-radius check.
#[test]
fn forgetting_one_memory_leaves_the_others_recognisable() {
    let dir = TempDir::new().unwrap();
    let (mut e, _path) = engine(&dir);
    let other = "an entirely separate note about the quarterly budget review \
                 and the finance team's reporting calendar for next year";

    e.enroll("m1", CONTENT).unwrap();
    e.enroll("m2", other).unwrap();

    e.forget("m1").unwrap();

    assert!(
        matches!(
            e.recognize(other).unwrap().verdict,
            Verdict::Recognized { .. }
        ),
        "forgetting one memory also destroyed another"
    );
    assert_eq!(
        e.store().enrolled_count().unwrap(),
        1,
        "exactly one memory should remain enrolled"
    );
}

/// Forgetting a memory that was never enrolled must report `false` rather than
/// erroring — a caller retrying a deletion should be able to.
#[test]
fn forgetting_an_unenrolled_memory_reports_false() {
    let dir = TempDir::new().unwrap();
    let (mut e, _path) = engine(&dir);
    assert!(
        !e.forget("never-enrolled").unwrap(),
        "forgetting an unknown memory should report false, not error"
    );
}

/// Forget is idempotent: the second call reports `false` and changes nothing.
#[test]
fn forgetting_twice_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let (mut e, path) = engine(&dir);
    e.enroll("m1", CONTENT).unwrap();

    assert!(e.forget("m1").unwrap());
    assert!(
        !e.forget("m1").unwrap(),
        "the second forget should report false"
    );
    for (table, n) in substrate_counts(&path) {
        assert_eq!(n, 0, "{table} gained rows from a second forget");
    }
}

/// Re-enrolling after a forget must work and be recognised again — forgetting
/// must not poison the id.
#[test]
fn a_forgotten_memory_can_be_re_enrolled() {
    let dir = TempDir::new().unwrap();
    let (mut e, _path) = engine(&dir);
    e.enroll("m1", CONTENT).unwrap();
    e.forget("m1").unwrap();
    e.enroll("m1", CONTENT).unwrap();

    assert!(
        matches!(
            e.recognize(CONTENT).unwrap().verdict,
            Verdict::Recognized { .. }
        ),
        "a re-enrolled memory is not recognised — forget poisoned its id"
    );
    assert_eq!(e.store().enrolled_count().unwrap(), 1);
}

/// Enrolment is idempotent by id: enrolling the same memory twice must not
/// double-count it, or `enrolled_count` misreports the corpus size that
/// rarity weighting divides by.
#[test]
fn enrolling_the_same_memory_twice_does_not_double_count() {
    let dir = TempDir::new().unwrap();
    let (mut e, _path) = engine(&dir);
    e.enroll("m1", CONTENT).unwrap();
    e.enroll("m1", CONTENT).unwrap();
    assert_eq!(
        e.store().enrolled_count().unwrap(),
        1,
        "re-enrolling the same id inflated the enrolled count"
    );
}

// ── read-only opens and vacuum ─────────────────────────────────────

/// A read-only store must serve recognition but refuse enrolment, rather than
/// silently accepting a write it cannot persist.
#[test]
fn a_read_only_store_serves_recognition_but_refuses_enrolment() {
    let dir = TempDir::new().unwrap();
    let path = {
        let (mut e, path) = engine(&dir);
        e.enroll("m1", CONTENT).unwrap();
        path
    };

    let ro_store = SqliteRecognitionStore::open_read_only(&path).unwrap();
    let mut ro = RecognitionEngine::new(ro_store, RecognitionConfig::default());

    assert!(
        matches!(
            ro.recognize(CONTENT).unwrap().verdict,
            Verdict::Recognized { .. }
        ),
        "a read-only store should still serve recognition"
    );
    assert!(
        ro.enroll("m2", "some new content to index").is_err(),
        "a read-only store accepted an enrolment it cannot persist"
    );
}

/// `vacuum` must leave the index functional — it is run after deletions, so a
/// vacuum that corrupted the store would turn a privacy operation into data
/// loss.
#[test]
fn vacuum_leaves_the_index_functional() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("recognition.db");
    let store = SqliteRecognitionStore::open(&path).unwrap();
    let mut e = RecognitionEngine::new(store, RecognitionConfig::default());

    e.enroll("keep", CONTENT).unwrap();
    e.enroll(
        "drop",
        "a second note that will shortly be forgotten entirely",
    )
    .unwrap();
    e.forget("drop").unwrap();

    // Vacuum through a fresh handle on the same file.
    SqliteRecognitionStore::open(&path)
        .unwrap()
        .vacuum()
        .unwrap();

    let store = SqliteRecognitionStore::open(&path).unwrap();
    let e = RecognitionEngine::new(store, RecognitionConfig::default());
    assert!(
        matches!(
            e.recognize(CONTENT).unwrap().verdict,
            Verdict::Recognized { .. }
        ),
        "the surviving memory is no longer recognised after a vacuum"
    );
    assert_eq!(e.store().enrolled_count().unwrap(), 1);
}
