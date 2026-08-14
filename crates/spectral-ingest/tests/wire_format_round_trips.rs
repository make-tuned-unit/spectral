//! String/DB round-trips for every persisted enum, and the safe defaults they
//! fall back to.
//!
//! These conversions are `as_str` / `parse` / `from_db` pairs. An asymmetry —
//! a variant added to one side and not the other — is invisible at compile
//! time and corrupts data on the next read: a row written as X comes back as
//! Y. That is why every variant is round-tripped exhaustively here rather than
//! spot-checked.
//!
//! Two of the fallbacks are *documented safety properties*, not conveniences,
//! and both are asserted with the reason attached:
//!
//! - `FieldSource::from_db` maps an unknown value to `Enriched` — the
//!   NON-protected source — "so a corrupt row can never masquerade as a manual
//!   value and block legitimate writes".
//! - `LedgerOutcome::from_db_str` maps an unknown value to `Unreported`,
//!   "never `Used`, which is the only variant that grants reinforcement".
//!
//! Flip either default and the mistake is a silent privilege change, so both
//! directions are pinned.

use spectral_ingest::{
    CompactionTier, FieldSource, LedgerOutcome, MemoryOutcomeEvidence, TimeBucket,
};

// ── CompactionTier ─────────────────────────────────────────────────

#[test]
fn every_compaction_tier_round_trips_through_its_string() {
    for tier in [
        CompactionTier::Raw,
        CompactionTier::HourlyRollup,
        CompactionTier::DailyRollup,
        CompactionTier::WeeklyRollup,
    ] {
        let s = tier.as_str();
        assert_eq!(
            CompactionTier::parse(s),
            Some(tier),
            "{tier:?} serialises to {s:?} but does not parse back — a row \
             written as {tier:?} would read back as something else"
        );
    }
}

#[test]
fn an_unknown_compaction_tier_is_rejected_rather_than_guessed() {
    assert_eq!(CompactionTier::parse("not_a_tier"), None);
    assert_eq!(CompactionTier::parse(""), None);
    // Case matters: the column stores the canonical lowercase form.
    assert_eq!(CompactionTier::parse("RAW"), None);
}

// ── FieldSource ────────────────────────────────────────────────────

#[test]
fn every_field_source_round_trips_through_its_db_string() {
    for source in [FieldSource::Manual, FieldSource::Enriched] {
        assert_eq!(
            FieldSource::from_db(source.as_str()),
            source,
            "{source:?} does not survive a DB round-trip"
        );
    }
}

/// The documented safety property: an unrecognised value must become
/// `Enriched`, the non-protected source. If it became `Manual`, a corrupt row
/// would be permanently unwritable, since an `Enriched` write never overwrites
/// a `Manual` field.
#[test]
fn an_unknown_field_source_defaults_to_the_non_protected_variant() {
    for junk in ["", "MANUAL", "user", "garbage", "manual "] {
        assert_eq!(
            FieldSource::from_db(junk),
            FieldSource::Enriched,
            "{junk:?} parsed as Manual; a corrupt row could then masquerade as \
             a protected value and block legitimate writes"
        );
    }
}

// ── LedgerOutcome ──────────────────────────────────────────────────

#[test]
fn every_ledger_outcome_round_trips_through_its_db_string() {
    for outcome in [
        LedgerOutcome::Used,
        LedgerOutcome::Wrong,
        LedgerOutcome::Ignored,
        LedgerOutcome::Unreported,
    ] {
        assert_eq!(
            LedgerOutcome::from_db_str(outcome.as_str()),
            outcome,
            "{outcome:?} does not survive a DB round-trip"
        );
    }
}

/// The other documented safety property: an unattributable outcome reads as
/// `Unreported`, never `Used` — `Used` is the only variant that grants
/// reinforcement, so a corrupt row must not be able to earn it.
#[test]
fn an_unknown_ledger_outcome_never_reads_as_used() {
    for junk in ["", "USED", "success", "garbage", "used!"] {
        let parsed = LedgerOutcome::from_db_str(junk);
        assert_eq!(
            parsed,
            LedgerOutcome::Unreported,
            "{junk:?} parsed as {parsed:?}"
        );
        assert_ne!(
            parsed,
            LedgerOutcome::Used,
            "an unrecognised outcome earned reinforcement"
        );
    }
}

// ── TimeBucket ─────────────────────────────────────────────────────

#[test]
fn every_time_bucket_has_a_distinct_string() {
    let buckets = [
        TimeBucket::SameDay,
        TimeBucket::SameWeek,
        TimeBucket::SameMonth,
        TimeBucket::Older,
        TimeBucket::Unknown,
    ];
    let mut seen = std::collections::HashSet::new();
    for b in buckets {
        assert!(
            seen.insert(b.as_str()),
            "{b:?} shares its string with another variant, so the two are \
             indistinguishable once stored"
        );
        // Display must agree with as_str, since both reach persisted output.
        assert_eq!(b.to_string(), b.as_str());
    }
}

/// The bucket boundaries are exclusive upper bounds (`<`), so a delta of
/// exactly one day is `SameWeek`, not `SameDay`. Pinned because an off-by-one
/// here silently re-buckets every fingerprint at the boundary.
#[test]
fn time_bucket_boundaries_are_exclusive_upper_bounds() {
    const DAY: f64 = 86_400.0;
    const WEEK: f64 = 604_800.0;
    const MONTH: f64 = 2_592_000.0;

    assert_eq!(TimeBucket::from_delta_secs(0.0), TimeBucket::SameDay);
    assert_eq!(TimeBucket::from_delta_secs(DAY - 1.0), TimeBucket::SameDay);
    assert_eq!(TimeBucket::from_delta_secs(DAY), TimeBucket::SameWeek);
    assert_eq!(
        TimeBucket::from_delta_secs(WEEK - 1.0),
        TimeBucket::SameWeek
    );
    assert_eq!(TimeBucket::from_delta_secs(WEEK), TimeBucket::SameMonth);
    assert_eq!(
        TimeBucket::from_delta_secs(MONTH - 1.0),
        TimeBucket::SameMonth
    );
    assert_eq!(TimeBucket::from_delta_secs(MONTH), TimeBucket::Older);
}

/// Bucketing is on the ABSOLUTE delta, so the order of the two timestamps
/// cannot change the bucket.
#[test]
fn time_bucket_is_symmetric_in_the_sign_of_the_delta() {
    for magnitude in [0.0, 3_600.0, 86_400.0, 604_800.0, 5_000_000.0] {
        assert_eq!(
            TimeBucket::from_delta_secs(magnitude),
            TimeBucket::from_delta_secs(-magnitude),
            "a negative delta of {magnitude}s bucketed differently"
        );
    }
}

/// `Unknown` is documented as retained only for deserialising pre-PR#65
/// fingerprints — `from_delta_secs` must never produce it, or live data would
/// be tagged with a legacy sentinel.
#[test]
fn from_delta_secs_never_produces_the_legacy_unknown_bucket() {
    for d in [0.0, 1.0, 86_400.0, 604_800.0, 2_592_000.0, 1e12] {
        assert_ne!(TimeBucket::from_delta_secs(d), TimeBucket::Unknown);
        assert_ne!(TimeBucket::from_delta_secs(-d), TimeBucket::Unknown);
    }
}

// ── MemoryOutcomeEvidence ──────────────────────────────────────────

/// `delivered_never_used` is the query the ledger exists to answer. Both
/// clauses matter: enough deliveries AND zero uses.
#[test]
fn delivered_never_used_requires_both_clauses() {
    let ev = |delivered: u64, used: u64| MemoryOutcomeEvidence {
        memory_id: "id".into(),
        memory_key: "key".into(),
        delivered,
        used,
        wrong: 0,
        ignored: 0,
        unreported: 0,
        best_rank: None,
    };

    assert!(
        ev(5, 0).delivered_never_used(5),
        "at the threshold, no uses"
    );
    assert!(ev(9, 0).delivered_never_used(5), "above the threshold");
    assert!(
        !ev(4, 0).delivered_never_used(5),
        "below the delivery threshold must not qualify"
    );
    assert!(
        !ev(9, 1).delivered_never_used(5),
        "a single use disqualifies, however many deliveries"
    );
    assert!(
        !ev(0, 0).delivered_never_used(1),
        "never delivered is not evidence of never used"
    );
}

// ── hash_query ─────────────────────────────────────────────────────

#[test]
fn hash_query_is_deterministic_full_length_and_input_sensitive() {
    let a = spectral_ingest::hash_query("what is the deploy runbook");
    let b = spectral_ingest::hash_query("what is the deploy runbook");
    let c = spectral_ingest::hash_query("what is the deploy runbook ");

    assert_eq!(a, b, "the same query hashed differently across calls");
    assert_ne!(a, c, "a trailing space produced the same grouping key");
    assert_eq!(a.len(), 64, "expected full blake3 hex");
    assert!(a.chars().all(|ch| ch.is_ascii_hexdigit()));
    // Empty input is a valid key, not a panic.
    assert_eq!(spectral_ingest::hash_query("").len(), 64);
}
