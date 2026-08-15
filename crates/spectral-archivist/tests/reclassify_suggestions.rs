//! `suggest_reclassifications` — the heuristic that proposes better homes for
//! memories stuck in the `general` wing.
//!
//! It is advisory (it suggests; nothing here writes), but its output drives
//! wing changes downstream, so a wrong suggestion moves real data. The
//! matching rules encode deliberate precision guards that were untested:
//!
//! - only `general`/NULL-wing memories are candidates;
//! - `weak_wings` are excluded from the target set;
//! - longer wing names are tried first, so the most specific one wins;
//! - a wing matches on a key hit, a spaced form, a repeat, or a long name.
//!
//! Writing these surfaced that the last two of those conditions are **inert**
//! for hyphen-free wings, and that content matching is an unanchored substring
//! test — see the two tests that record it. Both are pinned as behaviour
//! rather than "fixed", since retuning a heuristic belongs in this repo's
//! measured-change process, not a test pass.

use rusqlite::{params, Connection};
use spectral_archivist::reclassify::suggest_reclassifications;

fn test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE memories (
            id            TEXT PRIMARY KEY,
            key           TEXT NOT NULL UNIQUE,
            content       TEXT NOT NULL,
            wing          TEXT DEFAULT NULL,
            hall          TEXT DEFAULT NULL,
            signal_score  REAL DEFAULT 0.5
        );
        CREATE TABLE memory_spectrogram (
            memory_id            TEXT PRIMARY KEY,
            entity_density       REAL,
            decision_polarity    REAL,
            temporal_specificity REAL
        );",
    )
    .unwrap();
    conn
}

/// Insert a memory. `wing` of `None` stores SQL NULL.
fn mem(
    conn: &Connection,
    id: &str,
    key: &str,
    content: &str,
    wing: Option<&str>,
    hall: Option<&str>,
) {
    conn.execute(
        "INSERT INTO memories (id, key, content, wing, hall) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, key, content, wing, hall],
    )
    .unwrap();
}

fn spectro(conn: &Connection, id: &str, dp: Option<f64>, ed: Option<f64>, ts: Option<f64>) {
    conn.execute(
        "INSERT INTO memory_spectrogram (memory_id, decision_polarity, entity_density, \
         temporal_specificity) VALUES (?1, ?2, ?3, ?4)",
        params![id, dp, ed, ts],
    )
    .unwrap();
}

/// Establish a real wing so it becomes a candidate target.
fn anchor(conn: &Connection, id: &str, wing: &str) {
    mem(
        conn,
        id,
        &format!("anchor-{wing}"),
        "anchor memory",
        Some(wing),
        Some("fact"),
    );
}

// ── scope ──────────────────────────────────────────────────────────

/// Only memories in `general` or with no wing at all are candidates. A memory
/// already filed somewhere specific must never be proposed for a move.
#[test]
fn only_general_and_unwinged_memories_are_considered() {
    let conn = test_db();
    anchor(&conn, "a1", "permagent");
    mem(
        &conn,
        "m1",
        "k1",
        "permagent rollout notes",
        Some("general"),
        None,
    );
    mem(&conn, "m2", "k2", "permagent rollout notes", None, None);
    // Already filed elsewhere — must be left alone even though it matches.
    mem(
        &conn,
        "m3",
        "k3",
        "permagent rollout notes",
        Some("otherwing"),
        None,
    );

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    let keys: Vec<&str> = out.iter().map(|s| s.key.as_str()).collect();

    assert!(
        keys.contains(&"k1"),
        "a general-wing memory should be a candidate"
    );
    assert!(
        keys.contains(&"k2"),
        "a NULL-wing memory should be a candidate"
    );
    assert!(
        !keys.contains(&"k3"),
        "a memory already filed in a specific wing was proposed for a move"
    );
}

/// `weak_wings` are excluded from the candidate targets — the caller's way of
/// saying "this wing is not a good home".
#[test]
fn weak_wings_are_never_suggested_as_targets() {
    let conn = test_db();
    anchor(&conn, "a1", "scratchpad");
    mem(
        &conn,
        "m1",
        "scratchpad-note",
        "some scratchpad content",
        Some("general"),
        None,
    );

    let unfiltered = suggest_reclassifications(&conn, &[]).unwrap();
    assert_eq!(
        unfiltered
            .iter()
            .find(|s| s.key == "scratchpad-note")
            .and_then(|s| s.suggested_wing.as_deref()),
        Some("scratchpad"),
        "precondition: it is suggested when not marked weak"
    );

    let filtered = suggest_reclassifications(&conn, &["scratchpad".to_string()]).unwrap();
    assert!(
        filtered
            .iter()
            .all(|s| s.suggested_wing.as_deref() != Some("scratchpad")),
        "a weak wing was still suggested as a target"
    );
}

/// `general` itself is excluded from the candidate wings, so nothing is ever
/// suggested to move from general to general.
#[test]
fn general_is_not_a_suggestion_target() {
    let conn = test_db();
    mem(
        &conn,
        "m1",
        "k1",
        "a memory mentioning general repeatedly general",
        Some("general"),
        None,
    );
    let out = suggest_reclassifications(&conn, &[]).unwrap();
    assert!(out
        .iter()
        .all(|s| s.suggested_wing.as_deref() != Some("general")));
}

// ── the four match routes ──────────────────────────────────────────

/// Route 1: the wing name appears in the memory KEY. Enough on its own, even
/// for a short wing that never appears in the content.
#[test]
fn a_wing_name_in_the_key_is_enough() {
    let conn = test_db();
    anchor(&conn, "a1", "ops");
    mem(
        &conn,
        "m1",
        "ops-runbook",
        "entirely unrelated prose",
        Some("general"),
        None,
    );

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    assert_eq!(
        out.iter()
            .find(|s| s.key == "ops-runbook")
            .unwrap()
            .suggested_wing
            .as_deref(),
        Some("ops")
    );
}

/// Route 2: a hyphenated wing matched by its spaced form in prose.
#[test]
fn a_hyphenated_wing_matches_its_spaced_form_in_content() {
    let conn = test_db();
    anchor(&conn, "a1", "data-platform");
    mem(
        &conn,
        "m1",
        "k1",
        "notes about the data platform migration",
        Some("general"),
        None,
    );

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    assert_eq!(
        out.iter()
            .find(|s| s.key == "k1")
            .unwrap()
            .suggested_wing
            .as_deref(),
        Some("data-platform"),
        "the spaced form of a hyphenated wing did not match"
    );
}

/// Route 3 as WRITTEN is unreachable for a hyphen-free wing, and this test
/// records why rather than asserting an intent the code does not implement.
///
/// The match conditions are:
/// ```text
/// exact_key_hit || spaced_hit || repeated_hit || (exact_content_hit && wing.len() >= 6)
/// ```
/// `spaced_hit` is `content.contains(&wing.replace('-', " "))`. For a wing with
/// no hyphen that `replace` is a **no-op**, so `spaced_hit` reduces to plain
/// `content.contains(wing)` — which already subsumes both `repeated_hit` (>=5
/// chars, >=2 occurrences) and the `len() >= 6` guard. Those two conditions
/// therefore change nothing for the common, hyphen-free case: a single mention
/// always matches, at any length.
#[test]
fn a_single_content_mention_matches_at_any_length() {
    let conn = test_db();
    anchor(&conn, "a1", "vega5");
    mem(
        &conn,
        "once",
        "k-once",
        "we touched vega5 briefly",
        Some("general"),
        None,
    );
    mem(
        &conn,
        "twice",
        "k-twice",
        "vega5 again: vega5 is the theme",
        Some("general"),
        None,
    );

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    let wing_for = |k: &str| {
        out.iter()
            .find(|s| s.key == k)
            .and_then(|s| s.suggested_wing.clone())
    };

    assert_eq!(wing_for("k-twice").as_deref(), Some("vega5"));
    assert_eq!(
        wing_for("k-once").as_deref(),
        Some("vega5"),
        "a single mention already matches via spaced_hit; the repeat and \
         length guards are inert for hyphen-free wings"
    );
}

/// **FINDING — unanchored substring matching produces false suggestions.**
///
/// Because the content test is a bare `contains`, a wing name matches inside a
/// LARGER word. With a wing called `ops`, all three of these are proposed for
/// it: "dev**ops**", "o**ops**", "sh**ops**".
///
/// Severity is bounded — suggestions are advisory. Nothing applies them:
/// `archivist.rs:210` returns them and `main.rs:114` prints them for a human.
/// So this is suggestion noise, not silent data movement.
///
/// NOT fixed here: word-boundary matching is a retune of a heuristic whose
/// current shape presumably came from real use, and this repo gates behavioural
/// changes behind measurement rather than a drive-by edit. Pinned so the
/// behaviour is executable and a future retune has to confront it.
#[test]
fn known_limitation_a_wing_name_matches_inside_a_larger_word() {
    let conn = test_db();
    anchor(&conn, "a1", "ops");
    for (id, k, c) in [
        ("m1", "k-devops", "we discussed devops pipelines today"),
        ("m2", "k-oops", "oops, that deploy broke"),
        ("m3", "k-shops", "the shops are closed"),
    ] {
        mem(&conn, id, k, c, Some("general"), None);
    }

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    for k in ["k-devops", "k-oops", "k-shops"] {
        assert_eq!(
            out.iter()
                .find(|s| s.key == k)
                .and_then(|s| s.suggested_wing.clone())
                .as_deref(),
            Some("ops"),
            "{k} no longer matches 'ops' as a substring — if content matching \
             was anchored to word boundaries, update this test"
        );
    }
}

/// Route 4: a long wing (>= 6 chars) mentioned once in the content is enough,
/// because a long name is unlikely to be incidental.
#[test]
fn a_long_wing_name_qualifies_on_a_single_content_mention() {
    let conn = test_db();
    anchor(&conn, "a1", "permagent");
    mem(
        &conn,
        "m1",
        "k1",
        "one passing note about permagent",
        Some("general"),
        None,
    );

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    assert_eq!(
        out.iter()
            .find(|s| s.key == "k1")
            .unwrap()
            .suggested_wing
            .as_deref(),
        Some("permagent")
    );
}

/// Specificity: with two candidate wings both matching, the LONGER (more
/// specific) one wins, because the list is sorted by length descending.
#[test]
fn the_longer_more_specific_wing_wins() {
    let conn = test_db();
    anchor(&conn, "a1", "data");
    anchor(&conn, "a2", "dataplatform");
    mem(
        &conn,
        "m1",
        "k1",
        "notes about the dataplatform rollout",
        Some("general"),
        None,
    );

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    assert_eq!(
        out.iter()
            .find(|s| s.key == "k1")
            .unwrap()
            .suggested_wing
            .as_deref(),
        Some("dataplatform"),
        "the shorter, less specific wing won"
    );
}

#[test]
fn a_memory_matching_nothing_produces_no_suggestion() {
    let conn = test_db();
    anchor(&conn, "a1", "permagent");
    mem(
        &conn,
        "m1",
        "k1",
        "wholly unrelated prose about gardening",
        Some("general"),
        None,
    );

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    assert!(
        out.iter().all(|s| s.key != "k1"),
        "a memory matching no wing and no hall rule should produce nothing"
    );
}

// ── hall suggestions from spectrogram dimensions ───────────────────

#[test]
fn strong_positive_decision_polarity_suggests_discovery() {
    let conn = test_db();
    mem(
        &conn,
        "m1",
        "k1",
        "some content",
        Some("general"),
        Some("fact"),
    );
    spectro(&conn, "m1", Some(0.9), None, None);

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    let s = out
        .iter()
        .find(|s| s.key == "k1")
        .expect("a hall change is a suggestion");
    assert_eq!(s.suggested_hall.as_deref(), Some("discovery"));
    assert!(
        s.reason.contains("decision_polarity"),
        "reason: {}",
        s.reason
    );
}

#[test]
fn strong_negative_decision_polarity_suggests_advice() {
    let conn = test_db();
    mem(
        &conn,
        "m1",
        "k1",
        "some content",
        Some("general"),
        Some("fact"),
    );
    spectro(&conn, "m1", Some(-0.9), None, None);

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    assert_eq!(
        out.iter()
            .find(|s| s.key == "k1")
            .unwrap()
            .suggested_hall
            .as_deref(),
        Some("advice"),
        "negative polarity should suggest advice, not discovery"
    );
}

/// Below the polarity threshold the density/specificity pair decides instead.
#[test]
fn density_and_specificity_choose_between_event_and_fact() {
    let conn = test_db();
    mem(&conn, "m1", "k-event", "c", Some("general"), Some("fact"));
    spectro(&conn, "m1", Some(0.1), Some(0.9), Some(0.9));
    mem(&conn, "m2", "k-fact", "c", Some("general"), Some("event"));
    spectro(&conn, "m2", Some(0.1), Some(0.9), Some(0.1));

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    let hall_for = |k: &str| {
        out.iter()
            .find(|s| s.key == k)
            .and_then(|s| s.suggested_hall.clone())
    };
    assert_eq!(
        hall_for("k-event").as_deref(),
        Some("event"),
        "high entity_density + high temporal_specificity should read as an event"
    );
    assert_eq!(
        hall_for("k-fact").as_deref(),
        Some("fact"),
        "high entity_density + low temporal_specificity should read as a fact"
    );
}

/// The middle band — specificity between 0.3 and 0.5 — is deliberately
/// undecided, and must leave the hall alone rather than guessing.
#[test]
fn the_undecided_specificity_band_leaves_the_hall_unchanged() {
    let conn = test_db();
    mem(&conn, "m1", "k1", "c", Some("general"), Some("fact"));
    spectro(&conn, "m1", Some(0.1), Some(0.9), Some(0.4));

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    assert!(
        out.iter().all(|s| s.key != "k1"),
        "a memory in the undecided band was given a hall suggestion anyway"
    );
}

/// A memory with no spectrogram row at all must not error — the query
/// LEFT JOINs, so the dimensions are simply absent.
#[test]
fn a_missing_spectrogram_row_is_not_an_error() {
    let conn = test_db();
    anchor(&conn, "a1", "permagent");
    mem(
        &conn,
        "m1",
        "k1",
        "a note about permagent",
        Some("general"),
        Some("fact"),
    );

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    let s = out.iter().find(|s| s.key == "k1").unwrap();
    assert_eq!(s.suggested_wing.as_deref(), Some("permagent"));
    assert_eq!(
        s.suggested_hall.as_deref(),
        Some("fact"),
        "with no spectrogram the hall should be carried through unchanged"
    );
}

/// A suggestion must carry the memory's CURRENT placement too, or a reviewer
/// cannot tell what is being proposed.
#[test]
fn a_suggestion_reports_the_current_placement_alongside_the_proposed_one() {
    let conn = test_db();
    anchor(&conn, "a1", "permagent");
    mem(
        &conn,
        "m1",
        "k1",
        "a note about permagent",
        Some("general"),
        Some("fact"),
    );

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    let s = out.iter().find(|s| s.key == "k1").unwrap();
    assert_eq!(s.current_wing.as_deref(), Some("general"));
    assert_eq!(s.current_hall.as_deref(), Some("fact"));
    assert!(!s.reason.is_empty(), "a suggestion should say why");
}

/// When only the hall changes, the wing must be carried through rather than
/// reported as `None` — otherwise applying the suggestion would blank it.
#[test]
fn a_hall_only_suggestion_preserves_the_existing_wing() {
    let conn = test_db();
    mem(
        &conn,
        "m1",
        "k1",
        "no wing keywords here",
        Some("general"),
        Some("fact"),
    );
    spectro(&conn, "m1", Some(0.9), None, None);

    let out = suggest_reclassifications(&conn, &[]).unwrap();
    let s = out.iter().find(|s| s.key == "k1").unwrap();
    assert_eq!(
        s.suggested_wing.as_deref(),
        Some("general"),
        "a hall-only suggestion blanked the wing; applying it would unfile the \
         memory"
    );
}

#[test]
fn an_empty_database_yields_no_suggestions() {
    let conn = test_db();
    assert!(suggest_reclassifications(&conn, &[]).unwrap().is_empty());
}
