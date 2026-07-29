//! Deletion guarantees proof suite — the tests behind `docs/DELETION_GUARANTEES.md`.
//!
//! Pre-registered in `docs/internal/deletion-guarantees-prereg-2026-07-29.md`
//! (claims D1–D5, expectations, decision rules) BEFORE this file was written.
//! Every public claim in the doc maps to a test here via
//! `docs/deletion-guarantees-inventory.json`, enforced by
//! `tests/deletion_claims_gate.rs`.
//!
//! Extends (does not duplicate) the existing cross-substrate test
//! `brain_tests.rs::forget_hard_deletes_across_substrates_and_verifies`,
//! which pins the per-substrate `ForgetReceipt` counts for a single memory.

use std::path::PathBuf;

use chrono::Utc;
use rusqlite::Connection;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{
    Brain, BrainConfig, ForgetReport, RecallTopKConfig, RememberOpts, VerificationStatus,
};
use spectral_graph::spreading::{associative_spread, AssocSpreadConfig, SpreadMode};
use tempfile::TempDir;

/// Unique, all-lowercase sentinel token. Lowercase-only so every substrate
/// transform that lowercases (FTS tokenizers, recognition feature extraction)
/// preserves the exact byte sequence — one needle covers all substrates.
const SENTINEL: &str = "sentinelzq7vey4x9k";

fn brain_config(tmp: &TempDir) -> BrainConfig {
    // Spectrogram is retired (PR #227, off by default behind `spectrogram-legacy`);
    // the deletion guarantee is proven on the DEFAULT brain. The D1 sweep is
    // schema-derived, so it still cleans a spectrogram substrate if one exists —
    // it just no longer requires the retired substrate to be populated.
    BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
        entity_policy: spectral_graph::brain::EntityPolicy::Strict,
        activity_wing: "activity".into(),
        ..Default::default()
    }
}

/// Mirror of the Brain's key→memory-id derivation (blake3 prefix, 16 hex).
fn key_to_id(key: &str) -> String {
    format!(
        "{:016x}",
        u64::from_be_bytes(
            blake3::hash(key.as_bytes()).as_bytes()[..8]
                .try_into()
                .unwrap()
        )
    )
}

// ── Schema-derived sweep machinery (D1) ─────────────────────────────
//
// The substrate list is DERIVED from the live schema (sqlite_master), not
// hand-maintained: a future substrate table that carries the memory id or
// content — but is not wired into `forget` — enters the post-forget
// assertion set automatically and fails this suite by construction.

fn hex_upper(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02X}")).collect()
}

/// All user tables: (name, create-sql). Includes FTS5 virtual tables and
/// their shadow tables; excludes SQLite internals (`sqlite_*`), which cannot
/// carry user content we seeded.
fn user_tables(conn: &Connection) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT name, COALESCE(sql, '') FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn fts5_virtual_tables(tables: &[(String, String)]) -> Vec<String> {
    tables
        .iter()
        .filter(|(_, sql)| sql.to_lowercase().contains("using fts5"))
        .map(|(n, _)| n.clone())
        .collect()
}

/// Is `name` a physical shadow table of one of the FTS5 virtual tables?
fn is_fts5_shadow(name: &str, fts_tables: &[String]) -> bool {
    const SHADOW_SUFFIXES: [&str; 5] = ["_data", "_idx", "_docsize", "_config", "_content"];
    fts_tables.iter().any(|vt| {
        SHADOW_SUFFIXES
            .iter()
            .any(|suf| name == format!("{vt}{suf}"))
    })
}

/// Rows in `table` where ANY column contains `needle` as a byte sequence.
/// Matching is done on `hex(column)` so it works uniformly over TEXT, BLOB,
/// and numeric columns (including FTS segment blobs) with no NUL/encoding
/// pitfalls.
fn table_needle_rows(conn: &Connection, table: &str, needle: &str) -> u64 {
    let mut cols = Vec::new();
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .unwrap();
    let names = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap();
    for c in names {
        cols.push(c.unwrap());
    }
    if cols.is_empty() {
        return 0;
    }
    let clauses: Vec<String> = cols
        .iter()
        .map(|c| format!("instr(hex(\"{c}\"), ?1) > 0"))
        .collect();
    let sql = format!(
        "SELECT COUNT(*) FROM \"{table}\" WHERE {}",
        clauses.join(" OR ")
    );
    conn.query_row(&sql, [hex_upper(needle)], |r| r.get::<_, i64>(0))
        .unwrap_or(0) as u64
}

/// Logical residue in an FTS5 index: rows the index still RETURNS for a
/// token, queried through the virtual table (the honest way to sweep an
/// index — its shadow tables are physical segment storage, covered by D4).
fn fts_match_rows(conn: &Connection, fts_table: &str, query: &str) -> u64 {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM \"{fts_table}\" WHERE \"{fts_table}\" MATCH ?1"),
        [query],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0) as u64
}

/// Sweep every non-virtual user table of `conn` for `needles`, returning
/// (table, needle, rows) for every hit. Virtual FTS5 tables are swept
/// separately via MATCH (their SELECT view is backed by external content).
fn sweep_carriers(conn: &Connection, needles: &[&str]) -> Vec<(String, String, u64)> {
    let tables = user_tables(conn);
    let fts = fts5_virtual_tables(&tables);
    let mut out = Vec::new();
    for (name, _) in &tables {
        if fts.contains(name) {
            continue; // virtual table: swept via fts_match_rows
        }
        for needle in needles {
            let n = table_needle_rows(conn, name, needle);
            if n > 0 {
                out.push((name.clone(), (*needle).to_string(), n));
            }
        }
    }
    out
}

fn open_raw(path: &std::path::Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    conn
}

// ════════════════════════════════════════════════════════════════════
// D1 — schema-derived completeness
// ════════════════════════════════════════════════════════════════════

/// D1: after `forget`, NO table in ANY of the brain's databases (memory.db,
/// recognition.db, graph.sqlite) holds a row referencing the memory id, the
/// memory key, or the seeded sentinel content — where the table list is
/// enumerated from `sqlite_master` at test time, so an unwired future
/// substrate fails automatically.
#[test]
fn d1_schema_derived_substrate_sweep_is_clean_after_forget() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let victim_key = "victim-doc-2026";
    let victim_id = key_to_id(victim_key);
    let content = format!(
        "kraken reindex incident {SENTINEL} exploded during the nightly batch window \
         and paged the on-call rotation twice before the cap landed"
    );

    // Seed the victim so it touches as many substrates as the write path
    // reaches: episode + session (memory_sessions), spectrogram, recognition
    // enrollment, FTS.
    brain
        .remember_with(
            victim_key,
            &content,
            RememberOpts {
                visibility: Visibility::Private,
                episode_id: Some("ep-d1".into()),
                session_id: Some("sess-d1".into()),
                ..Default::default()
            },
        )
        .unwrap();
    brain
        .remember_with(
            "nbr-alpha",
            "kraken reindex incident follow-up: capped worker memory at 512Mi",
            RememberOpts {
                visibility: Visibility::Private,
                episode_id: Some("ep-d1".into()),
                session_id: Some("sess-d1".into()),
                ..Default::default()
            },
        )
        .unwrap();
    brain
        .remember(
            "nbr-beta",
            "raw shard log for the kraken batch window",
            Visibility::Private,
        )
        .unwrap();

    // memory_annotations substrate (annotation text also carries the sentinel).
    brain
        .annotate(
            &victim_id,
            spectral_ingest::AnnotationInput {
                description: format!("annotation citing {SENTINEL}"),
                who: vec![],
                why: "deletion-guarantees D1 seeding".into(),
                where_: None,
                when_: Utc::now(),
                how: "test".into(),
            },
        )
        .unwrap();

    // consolidation_edges substrate: victim as consolidation TARGET.
    brain
        .consolidate_into(
            &["nbr-beta".to_string()],
            victim_key,
            &spectral_ingest::ConsolidateOpts::default(),
        )
        .unwrap();

    // retrieval_events + co_retrieval_pairs substrates: co-retrieve victim
    // with a neighbor, then materialize the pair index.
    for _ in 0..3 {
        let hits = brain
            .recall_topk_fts(
                "kraken reindex incident",
                &RecallTopKConfig::default(),
                Visibility::Private,
            )
            .unwrap();
        assert!(
            hits.iter().any(|h| h.key == victim_key),
            "victim recallable pre-forget"
        );
    }
    brain.rebuild_co_retrieval_index().unwrap();

    // Presence probes must fire BEFORE forget (the sweep is load-bearing).
    let rec = brain.recognize(&content).unwrap();
    assert!(
        matches!(rec.verdict, spectral_recognition::Verdict::Recognized { ref memory_id } if *memory_id == victim_id),
        "victim recognizable pre-forget"
    );

    let mem_db = open_raw(&tmp.path().join("memory.db"));
    let rec_db = open_raw(&tmp.path().join("recognition.db"));
    let graph_db = open_raw(&tmp.path().join("graph.sqlite"));
    let needles = [victim_id.as_str(), victim_key, SENTINEL];

    let pre_mem = sweep_carriers(&mem_db, &needles);
    let pre_rec = sweep_carriers(&rec_db, &needles);
    let pre_tables: Vec<&str> = pre_mem
        .iter()
        .chain(pre_rec.iter())
        .map(|(t, _, _)| t.as_str())
        .collect();
    // The sweep must actually see the seeded substrates — otherwise the
    // post-forget "zero rows" assertion would be vacuous.
    for expected in [
        "memories",
        "memory_annotations",
        "memory_sessions",
        "consolidation_edges",
        "co_retrieval_pairs",
        "retrieval_events",
        "recognition_enrolled",
    ] {
        assert!(
            pre_tables.contains(&expected),
            "pre-forget sweep should find victim in `{expected}`; carriers: {pre_tables:?}"
        );
    }
    assert!(
        fts_match_rows(&mem_db, "memories_fts", &format!("\"{SENTINEL}\"")) > 0,
        "FTS index should match the sentinel pre-forget"
    );

    // Forget.
    let report = brain.forget(victim_key).unwrap();
    assert!(
        report.fully_forgotten(),
        "forget must verify clean: {report:?}"
    );

    // Post-forget: every enumerated table must hold ZERO rows referencing the
    // id, the key, or the sentinel — except the explicit allowlist below.
    //
    // ALLOWLIST (each entry must be justified here):
    // - `sync_tombstones`, `replicated_set_tombstones`: federation retraction
    //   markers. A tombstone must OUTLIVE the object it retracts (that is its
    //   function — see D5); it carries only an object hash, never content.
    // - FTS5 shadow tables (`<fts>_data/_idx/_docsize/_config/_content`):
    //   physical segment storage of the index. Their LOGICAL view is swept
    //   via MATCH below (zero required); physically-dead bytes inside
    //   segments are exactly the claim-D4 boundary, erased by
    //   `Brain::vacuum` and byte-scanned in the D4 test.
    let allow_names = ["sync_tombstones", "replicated_set_tombstones"];
    for (db_name, conn) in [
        ("memory.db", &mem_db),
        ("recognition.db", &rec_db),
        ("graph.sqlite", &graph_db),
    ] {
        let tables = user_tables(conn);
        let fts = fts5_virtual_tables(&tables);
        for (table, _) in &tables {
            if allow_names.contains(&table.as_str())
                || fts.contains(table)
                || is_fts5_shadow(table, &fts)
            {
                continue;
            }
            for needle in &needles {
                let n = table_needle_rows(conn, table, needle);
                assert_eq!(
                    n, 0,
                    "{db_name}: table `{table}` still holds {n} row(s) containing `{needle}` after forget"
                );
            }
        }
        // Logical FTS sweep: the index must RETURN nothing for the deleted
        // tokens (unjoined — catches dangling index entries the recall path's
        // JOIN would mask).
        for vt in &fts {
            assert_eq!(
                fts_match_rows(conn, vt, &format!("\"{SENTINEL}\"")),
                0,
                "{db_name}: FTS index `{vt}` still matches the sentinel after forget"
            );
        }
    }
}

// ════════════════════════════════════════════════════════════════════
// D2 — verification, not assumption (sabotage detection)
// ════════════════════════════════════════════════════════════════════

/// D2: the `ForgetReport` probes are load-bearing. After a clean forget, this
/// test re-inserts residue rows (raw SQL — simulating a substrate whose
/// delete silently failed) into (1) the primary store and (2) the recognition
/// sidecar, and asserts the verification probes DETECT the residue rather
/// than tautologically reporting clean.
///
/// Boundary (documented, not hidden): `VerificationStatus::ResidualFound`
/// inside a single `forget()` call requires a substrate delete to fail
/// mid-call; there is no fault-injection seam in the store, so this test
/// proves the same probes fire on residue when re-run through their public
/// APIs (`recall_topk_fts` / `recognize` — the exact probes `forget` runs),
/// and that a forget over a sabotaged state never claims `fully_forgotten`.
/// `fully_forgotten()` is fail-closed by construction (pinned below).
#[test]
fn d2_sabotaged_deletion_is_detected_by_probes() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let key = "sabotage-victim";
    let id = key_to_id(key);
    let content = "the payment webhook replayed duplicate charges after the idempotency \
                   cache flush during the friday deploy freeze window";
    brain.remember(key, content, Visibility::Private).unwrap();

    // Snapshot the rows we will later re-insert as "residue".
    let mem_db = open_raw(&tmp.path().join("memory.db"));
    let rec_db = open_raw(&tmp.path().join("recognition.db"));
    let rec_tables = [
        "recognition_enrolled",
        "recognition_pairs",
        "recognition_grams",
        "recognition_minhash_sig",
        "recognition_minhash_bands",
    ];
    let mut rec_snapshot: Vec<(String, Vec<Vec<rusqlite::types::Value>>)> = Vec::new();
    for table in rec_tables {
        let mut stmt = rec_db
            .prepare(&format!("SELECT * FROM {table} WHERE memory_id = ?1"))
            .unwrap();
        let ncols = stmt.column_count();
        let rows: Vec<Vec<rusqlite::types::Value>> = stmt
            .query_map([&id], |r| {
                (0..ncols)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect()
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rec_snapshot.push((table.to_string(), rows));
    }
    assert!(
        rec_snapshot.iter().any(|(_, rows)| !rows.is_empty()),
        "victim must be enrolled in recognition before forget"
    );

    // Clean forget: probes report VerifiedClear.
    let report = brain.forget(key).unwrap();
    assert_eq!(
        report.recall_verification,
        VerificationStatus::VerifiedClear
    );
    assert_eq!(
        report.recognition_verification,
        VerificationStatus::VerifiedClear
    );
    assert!(report.fully_forgotten());

    // Pin the fail-closed semantics: a report carrying ResidualFound (or a
    // probe failure) can NEVER count as fully forgotten.
    let sabotaged_report = ForgetReport {
        store: spectral_ingest::ForgetReceipt {
            existed: true,
            memory_rows: 1,
            ..Default::default()
        },
        recognition_removed: true,
        recall_clear: false,
        recognize_clear: true,
        recall_verification: VerificationStatus::ResidualFound,
        recognition_verification: VerificationStatus::VerifiedClear,
    };
    assert!(
        !sabotaged_report.fully_forgotten(),
        "ResidualFound must fail the report"
    );
    let failed_probe_report = ForgetReport {
        recall_verification: VerificationStatus::VerificationFailed("probe error".into()),
        ..sabotaged_report
    };
    assert!(
        !failed_probe_report.fully_forgotten(),
        "a failed probe must fail closed"
    );

    // ── Sabotage 1: primary-store residue ──
    // Re-insert the memories row (the AFTER INSERT trigger restores its FTS
    // entry) — the state a failed store delete would leave behind.
    mem_db
        .execute(
            "INSERT INTO memories (id, key, content, visibility) VALUES (?1, ?2, ?3, 'private')",
            rusqlite::params![id, key, content],
        )
        .unwrap();
    let hits = brain
        .recall_topk_fts(content, &RecallTopKConfig::default(), Visibility::Private)
        .unwrap();
    assert!(
        hits.iter().any(|h| h.key == key),
        "the recall verification probe must DETECT store residue (it is the \
         condition that yields ResidualFound at probe time)"
    );
    // Re-forget removes the residue and its clean verdict is then truthful.
    let report2 = brain.forget(key).unwrap();
    assert!(report2.store.existed, "re-forget must see the residue row");
    assert!(report2.fully_forgotten());
    let hits = brain
        .recall_topk_fts(content, &RecallTopKConfig::default(), Visibility::Private)
        .unwrap();
    assert!(
        !hits.iter().any(|h| h.key == key),
        "residue actually gone after re-forget"
    );

    // ── Sabotage 2: recognition-sidecar residue ──
    // Restore the victim's recognition rows while the memories row stays
    // deleted — the state a failed sidecar delete would leave behind.
    for (table, rows) in &rec_snapshot {
        for row in rows {
            let placeholders: Vec<String> = (1..=row.len()).map(|i| format!("?{i}")).collect();
            rec_db
                .execute(
                    &format!("INSERT INTO {table} VALUES ({})", placeholders.join(", ")),
                    rusqlite::params_from_iter(row.iter()),
                )
                .unwrap();
        }
    }
    let rec = brain.recognize(content).unwrap();
    assert!(
        matches!(rec.verdict, spectral_recognition::Verdict::Recognized { ref memory_id } if *memory_id == id),
        "the recognition verification probe must DETECT sidecar residue"
    );
    assert!(
        rec.evidence.iter().any(|e| e.memory_id == id),
        "residue evidence must cite the victim"
    );
    // A forget over this sabotaged state must not claim success: the store
    // row is gone (existed = false), so `fully_forgotten()` is false — and
    // the re-forget re-purges the sidecar residue.
    let report3 = brain.forget(key).unwrap();
    assert!(!report3.store.existed);
    assert!(
        report3.recognition_removed,
        "re-forget must purge the sidecar residue"
    );
    assert!(
        !report3.fully_forgotten(),
        "a forget that found no store row never reports success"
    );
    let rec = brain.recognize(content).unwrap();
    assert!(
        !matches!(rec.verdict, spectral_recognition::Verdict::Recognized { ref memory_id } if *memory_id == id),
        "sidecar residue actually gone after re-forget"
    );
}

// ════════════════════════════════════════════════════════════════════
// D3 — adversarial residue (side doors)
// ════════════════════════════════════════════════════════════════════

/// D3(a): post-forget, FTS phrase and prefix probes on distinctive deleted
/// tokens return nothing carrying the key — both through the public recall
/// API and through raw unjoined MATCH (which would expose dangling index
/// entries the recall JOIN masks).
#[test]
fn d3a_fts_phrase_and_prefix_find_nothing_after_forget() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let key = "d3a-victim";
    let content = format!(
        "quarterly ledger reconciliation flagged the {SENTINEL} discrepancy in the \
         offshore clearing account during the audit dry run"
    );
    brain.remember(key, &content, Visibility::Private).unwrap();
    brain
        .remember(
            "d3a-bystander",
            "the audit dry run schedule moved to thursday",
            Visibility::Private,
        )
        .unwrap();

    let mem_db = open_raw(&tmp.path().join("memory.db"));
    let phrase = "\"ledger reconciliation flagged\"";
    let prefix = "sentinelzq*";
    // Probes must fire pre-forget (non-vacuous).
    assert!(fts_match_rows(&mem_db, "memories_fts", phrase) > 0);
    assert!(fts_match_rows(&mem_db, "memories_fts", prefix) > 0);

    brain.forget(key).unwrap();

    for query in [phrase, prefix, &format!("\"{SENTINEL}\"") as &str] {
        assert_eq!(
            fts_match_rows(&mem_db, "memories_fts", query),
            0,
            "raw FTS MATCH `{query}` must return nothing after forget"
        );
    }
    for query in [
        &content as &str,
        SENTINEL,
        "ledger reconciliation flagged offshore clearing",
    ] {
        let hits = brain
            .recall_topk_fts(query, &RecallTopKConfig::default(), Visibility::Private)
            .unwrap();
        assert!(
            !hits.iter().any(|h| h.key == key),
            "recall for `{query}` must not surface the forgotten key"
        );
    }
}

/// D3(b): a recognition probe with the deleted content verbatim AND with a
/// ~30%-token-dropped copy (the re-encounter condition) yields no Recognized
/// verdict naming it, no candidate trace, and no evidence row citing its
/// features.
#[test]
fn d3b_recognition_rejects_verbatim_and_degraded_reencounter() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let key = "d3b-victim";
    let id = key_to_id(key);
    let content = "the ferry timetable rework collapsed the harbour queue by routing the \
                   overnight freight sailings through the northern berth while the dredger \
                   cleared silt from the passenger lanes ahead of the solstice rush";
    brain.remember(key, content, Visibility::Private).unwrap();

    // ~30% token drop: remove every token with index % 10 in {2, 5, 8}.
    let degraded: String = content
        .split_whitespace()
        .enumerate()
        .filter(|(i, _)| !matches!(i % 10, 2 | 5 | 8))
        .map(|(_, w)| w)
        .collect::<Vec<_>>()
        .join(" ");

    // Pre-forget: both the verbatim and the degraded re-encounter name the
    // victim (the probes are strong enough that their post-forget silence
    // means something).
    for probe in [content, degraded.as_str()] {
        let rec = brain.recognize(probe).unwrap();
        assert!(
            matches!(rec.verdict, spectral_recognition::Verdict::Recognized { ref memory_id } if *memory_id == id),
            "pre-forget probe must recognize the victim (probe: {} chars)",
            probe.len()
        );
    }

    brain.forget(key).unwrap();

    for probe in [content, degraded.as_str()] {
        let rec = brain.recognize(probe).unwrap();
        assert!(
            !matches!(rec.verdict, spectral_recognition::Verdict::Recognized { ref memory_id } if *memory_id == id),
            "post-forget probe must not recognize the victim"
        );
        assert!(
            rec.traces.iter().all(|t| t.memory_id != id),
            "no candidate trace may name the forgotten memory"
        );
        assert!(
            rec.evidence.iter().all(|e| e.memory_id != id),
            "no evidence row may cite the forgotten memory's features"
        );
    }
}

/// D3(c): the graph substrate (graph.sqlite: entity/triple/mention/document)
/// carries no residue of a forgotten memory — before OR after forget.
///
/// Scope, stated exactly: `remember()` never mints graph rows. Graph triples
/// and aliases come only from `assert()`/`ingest_*` and are keyed by
/// entity/document, not by memory key (see `Brain::forget` docs). This test
/// proves that scope: with a populated graph (an asserted fact), the
/// schema-derived sweep finds ZERO graph rows referencing the memory id,
/// key, or sentinel at any point.
#[test]
fn d3c_graph_substrate_carries_no_memory_residue() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let key = "d3c-victim";
    let id = key_to_id(key);
    let content = format!("mark drafted the {SENTINEL} thesis outline at the library annex");
    brain.remember(key, &content, Visibility::Private).unwrap();
    // Populate the graph through its real write path so the sweep runs over
    // a non-empty substrate.
    brain
        .assert("Mark", "studies", "Library", 0.9, Visibility::Private)
        .unwrap();

    let graph_db = open_raw(&tmp.path().join("graph.sqlite"));
    let needles = [id.as_str(), key, SENTINEL];
    assert!(
        !user_tables(&graph_db).is_empty(),
        "graph schema must exist for the sweep to mean anything"
    );
    let pre = sweep_carriers(&graph_db, &needles);
    assert!(
        pre.is_empty(),
        "scope claim: remember() must not mint graph rows (found: {pre:?})"
    );

    brain.forget(key).unwrap();

    let post = sweep_carriers(&graph_db, &needles);
    assert!(post.is_empty(), "graph residue after forget: {post:?}");
}

/// D3(d): associative / co-retrieval paths seeded from a NEIGHBOR memory do
/// not return the forgotten one — `related_memories` (co-retrieval),
/// `recommend` (lift ranking), and episode-based `associative_spread`. Also
/// regeneration-resistance: rebuilding the co-retrieval index from the
/// (scrubbed) retrieval-event log post-forget cannot resurrect the pair.
#[test]
fn d3d_associative_paths_from_neighbors_exclude_forgotten() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let victim_key = "d3d-victim";
    let victim_id = key_to_id(victim_key);
    let neighbor_key = "d3d-neighbor";
    let neighbor_id = key_to_id(neighbor_key);
    // Shared rare bigram ("glacier basecamp") for co-retrieval; otherwise
    // lexically disjoint so the spread probe exercises the associative path,
    // not FTS overlap.
    brain
        .remember_with(
            victim_key,
            "silverfin payload obfuscation notes stashed at the glacier basecamp locker",
            RememberOpts {
                visibility: Visibility::Private,
                episode_id: Some("ep-d3d".into()),
                ..Default::default()
            },
        )
        .unwrap();
    brain
        .remember_with(
            neighbor_key,
            "penguin colony tracker deployment near the glacier basecamp ridge",
            RememberOpts {
                visibility: Visibility::Private,
                episode_id: Some("ep-d3d".into()),
                ..Default::default()
            },
        )
        .unwrap();

    // Co-retrieve the pair, then materialize the co-retrieval index.
    for _ in 0..3 {
        let hits = brain
            .recall_topk_fts(
                "glacier basecamp",
                &RecallTopKConfig::default(),
                Visibility::Private,
            )
            .unwrap();
        assert!(hits.iter().any(|h| h.key == victim_key));
        assert!(hits.iter().any(|h| h.key == neighbor_key));
    }
    brain.rebuild_co_retrieval_index().unwrap();

    // All three associative paths must surface the victim pre-forget
    // (otherwise the post-forget emptiness proves nothing).
    assert!(
        brain
            .related_memories(&neighbor_id, 20)
            .unwrap()
            .iter()
            .any(|r| r.memory_id == victim_id),
        "co-retrieval must associate victim with neighbor pre-forget"
    );
    assert!(
        brain
            .recommend(&neighbor_id, 20, 1)
            .unwrap()
            .iter()
            .any(|r| r.memory_id == victim_id),
        "lift recommendation must surface victim pre-forget"
    );
    let spread_cfg = AssocSpreadConfig {
        mode: SpreadMode::Episode,
        ..Default::default()
    };
    let mut hits = brain
        .recall_topk_fts(
            "penguin colony tracker",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();
    associative_spread(&brain, &mut hits, &spread_cfg, Visibility::Private);
    assert!(
        hits.iter().any(|h| h.key == victim_key),
        "episode spread from the neighbor must surface victim pre-forget"
    );

    brain.forget(victim_key).unwrap();

    assert!(
        !brain
            .related_memories(&neighbor_id, 20)
            .unwrap()
            .iter()
            .any(|r| r.memory_id == victim_id),
        "co-retrieval must not return the forgotten memory"
    );
    assert!(
        !brain
            .recommend(&neighbor_id, 20, 1)
            .unwrap()
            .iter()
            .any(|r| r.memory_id == victim_id),
        "recommendation must not return the forgotten memory"
    );
    let mut hits = brain
        .recall_topk_fts(
            "penguin colony tracker",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();
    associative_spread(&brain, &mut hits, &spread_cfg, Visibility::Private);
    assert!(
        !hits
            .iter()
            .any(|h| h.key == victim_key || h.id == victim_id),
        "episode spread must not resurrect the forgotten memory"
    );

    // Regeneration-resistance: forget scrubbed the retrieval events, so a
    // rebuild cannot re-derive the association.
    brain.rebuild_co_retrieval_index().unwrap();
    assert!(
        !brain
            .related_memories(&neighbor_id, 20)
            .unwrap()
            .iter()
            .any(|r| r.memory_id == victim_id),
        "rebuilding the co-retrieval index must not resurrect the pair"
    );
}

// ════════════════════════════════════════════════════════════════════
// D4 — physical residue boundary
// ════════════════════════════════════════════════════════════════════

/// D4: `forget` is logical-immediate; PHYSICAL erasure requires
/// `Brain::vacuum` (the pre-registered API gap, closed on this branch).
///
/// Expected and documented: BEFORE vacuum the sentinel byte-sequence is
/// still present in the raw files (SQLite free pages / WAL frames / FTS
/// segments retain logically-deleted bytes) — asserted below, which also
/// proves the byte-scan itself is load-bearing. AFTER `Brain::vacuum` (FTS
/// 'optimize' + truncating WAL checkpoint + VACUUM across memory.db,
/// recognition.db, graph.sqlite) the sentinel must be absent from every
/// database file and WAL.
#[test]
fn d4_physical_bytes_absent_after_forget_and_vacuum() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(brain_config(&tmp)).unwrap();

    let key = "d4-victim";
    let content = format!(
        "cold-storage rotation plan {SENTINEL} moves the archive shards to the \
         basement rack before the insurance audit"
    );
    brain.remember(key, &content, Visibility::Private).unwrap();
    brain
        .remember(
            "d4-bystander",
            "the basement rack inventory was recounted",
            Visibility::Private,
        )
        .unwrap();

    let report = brain.forget(key).unwrap();
    assert!(report.fully_forgotten());

    let db_files = |dir: &std::path::Path| -> Vec<PathBuf> {
        ["memory.db", "recognition.db", "graph.sqlite"]
            .iter()
            .flat_map(|base| {
                ["", "-wal", "-shm"]
                    .iter()
                    .map(move |suf| dir.join(format!("{base}{suf}")))
            })
            .filter(|p| p.exists())
            .collect()
    };
    let contains_sentinel = |files: &[PathBuf]| -> Vec<PathBuf> {
        let needle = SENTINEL.as_bytes();
        files
            .iter()
            .filter(|p| {
                std::fs::read(p)
                    .unwrap()
                    .windows(needle.len())
                    .any(|w| w == needle)
            })
            .cloned()
            .collect()
    };

    // Pre-vacuum: the bytes are EXPECTED to persist (this is the boundary the
    // public doc states, and it proves the scan can find the needle at all).
    let dirty = contains_sentinel(&db_files(tmp.path()));
    assert!(
        !dirty.is_empty(),
        "logically-deleted bytes should persist pre-vacuum (else this scan proves nothing)"
    );

    brain.vacuum().unwrap();
    drop(brain);

    let residue = contains_sentinel(&db_files(tmp.path()));
    assert!(
        residue.is_empty(),
        "sentinel bytes must be physically absent after forget + vacuum; found in {residue:?}"
    );

    // The bystander memory must survive compaction untouched.
    let brain = Brain::open(brain_config(&tmp)).unwrap();
    let hits = brain
        .recall_topk_fts(
            "basement rack inventory",
            &RecallTopKConfig::default(),
            Visibility::Private,
        )
        .unwrap();
    assert!(
        hits.iter().any(|h| h.key == "d4-bystander"),
        "vacuum must not damage other memories"
    );
}

// ════════════════════════════════════════════════════════════════════
// D5 — federation tombstones (scoped)
// ════════════════════════════════════════════════════════════════════

fn open_store(dir: &TempDir) -> spectral_ingest::sqlite_store::SqliteStore {
    spectral_ingest::sqlite_store::SqliteStore::open(&dir.path().join("memory.db")).unwrap()
}

fn store_write(
    rt: &tokio::runtime::Runtime,
    store: &spectral_ingest::sqlite_store::SqliteStore,
    id: &str,
    key: &str,
    content: &str,
) {
    use spectral_ingest::MemoryStore;
    let memory = spectral_ingest::Memory {
        id: id.into(),
        key: key.into(),
        content: content.into(),
        wing: None,
        hall: None,
        signal_score: 0.5,
        visibility: "team".into(),
        source: None,
        device_id: None,
        confidence: 1.0,
        created_at: Some("2026/01/01 (Thu) 10:00".into()),
        last_reinforced_at: None,
        episode_id: None,
        compaction_tier: None,
        declarative_density: None,
        description: None,
        description_generated_at: None,
        content_hash: None,
        source_brain_id: None,
        signature: None,
    };
    rt.block_on(store.write(&memory, &[])).unwrap();
}

fn store_keys_with_content(
    rt: &tokio::runtime::Runtime,
    store: &spectral_ingest::sqlite_store::SqliteStore,
    needle: &str,
) -> Vec<String> {
    use spectral_ingest::MemoryStore;
    rt.block_on(store.list_memories_by_signal(0.0, 1000))
        .unwrap()
        .into_iter()
        .filter(|m| m.content.contains(needle))
        .map(|m| m.key)
        .collect()
}

/// D5: a retracted shared-wing object does not resurface through have/want
/// replication — the tombstone dominates further sync rounds, including a
/// stale peer re-shipping the original pack (PR #207/#210 semantics).
#[test]
fn d5_federation_tombstone_blocks_resurrection_across_sync_rounds() {
    use spectral_ingest::federation_sync as fed;
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (dir_a, dir_b) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let (a, b) = (open_store(&dir_a), open_store(&dir_b));

    store_write(
        &rt,
        &a,
        "d5aa00000000d5aa",
        "shared-note",
        "the shared kraken incident retro",
    );
    let hash = fed::share(&a, "shared-note", "wing-x").unwrap();

    // Sync round 1: B receives the object.
    let pack_v1 = fed::export_pack(&a, "wing-x").unwrap();
    fed::import_pack(&b, &pack_v1).unwrap();
    assert_eq!(fed::enumerate(&b, "wing-x").unwrap(), vec![hash.clone()]);
    assert!(!store_keys_with_content(&rt, &b, "kraken incident retro").is_empty());

    // A retracts (the federation-scoped forget). Semantics pinned here:
    // `tombstone` removes the object from the wing everywhere, and
    // hard-deletes IMPORTED copies (stored under `id = object_hash`). The
    // AUTHOR's native row has its own id, so it survives as a private,
    // no-longer-exported memory — the author completes local erasure with the
    // ordinary forget path. Full erasure = tombstone (federation) + forget
    // (local). This boundary is stated verbatim in DELETION_GUARANTEES.md.
    fed::tombstone(&a, "wing-x", &hash).unwrap();
    assert!(fed::enumerate(&a, "wing-x").unwrap().is_empty());
    assert!(
        !store_keys_with_content(&rt, &a, "kraken incident retro").is_empty(),
        "author's native copy survives tombstone (documented boundary); \
         if this starts failing, update DELETION_GUARANTEES.md"
    );
    {
        use spectral_ingest::MemoryStore;
        let receipt = rt.block_on(a.delete_memory_by_key("shared-note")).unwrap();
        assert!(receipt.existed);
    }
    assert!(store_keys_with_content(&rt, &a, "kraken incident retro").is_empty());

    // Sync round 2: the tombstone reaches B and deletes its copy.
    let pack_v2 = fed::export_pack(&a, "wing-x").unwrap();
    fed::import_pack(&b, &pack_v2).unwrap();
    assert!(fed::enumerate(&b, "wing-x").unwrap().is_empty());
    assert!(
        store_keys_with_content(&rt, &b, "kraken incident retro").is_empty(),
        "tombstone must hard-delete the peer's local copy"
    );

    // Sync round 3 (both directions), plus a STALE peer re-shipping the
    // pre-retraction pack: nothing may resurface anywhere.
    let pack_b = fed::export_pack(&b, "wing-x").unwrap();
    fed::import_pack(&a, &pack_b).unwrap();
    fed::import_pack(&b, &pack_v1).unwrap(); // stale pack still carries the object
    for (name, store) in [("A", &a), ("B", &b)] {
        assert!(
            fed::enumerate(store, "wing-x").unwrap().is_empty(),
            "{name}: retracted object resurfaced in the wing manifest"
        );
        assert!(
            store_keys_with_content(&rt, store, "kraken incident retro").is_empty(),
            "{name}: retracted content resurfaced in the memory store"
        );
    }
}

/// D5 scope boundary (documented, load-bearing): a PLAIN local forget
/// (`delete_memory_by_key`, what `Brain::forget` runs) writes no federation
/// tombstone. Observed semantics, pinned here: the forgotten content does
/// NOT resurface when the peer re-ships the original pack — the local wing
/// manifest entry survives the row delete and import dedups against it —
/// but this protection is a side effect of manifest bookkeeping, not a
/// retraction. The peer still holds and re-advertises the object, and the
/// local manifest now advertises an object this store cannot serve. The
/// robust federation-wide retraction is `federation_sync::tombstone`; the
/// public doc scopes the claim exactly this way, and this test pins the
/// behavior so the scope statement cannot silently rot.
#[test]
fn d5_scope_plain_forget_is_single_brain_only() {
    use spectral_ingest::federation_sync as fed;
    use spectral_ingest::MemoryStore;
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (dir_a, dir_b) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let (a, b) = (open_store(&dir_a), open_store(&dir_b));

    store_write(
        &rt,
        &a,
        "d5bb00000000d5bb",
        "leaky-note",
        "the migration password rotation memo",
    );
    fed::share(&a, "leaky-note", "wing-y").unwrap();
    let pack = fed::export_pack(&a, "wing-y").unwrap();
    fed::import_pack(&b, &pack).unwrap();

    // B forgets locally WITHOUT a federation tombstone.
    let b_key = store_keys_with_content(&rt, &b, "password rotation memo")
        .pop()
        .expect("B holds the replicated copy");
    let receipt = rt.block_on(b.delete_memory_by_key(&b_key)).unwrap();
    assert!(receipt.existed);
    assert!(store_keys_with_content(&rt, &b, "password rotation memo").is_empty());

    // Peer re-delivery of the original pack: content must NOT resurface at B.
    fed::import_pack(&b, &pack).unwrap();
    assert!(
        store_keys_with_content(&rt, &b, "password rotation memo").is_empty(),
        "locally-forgotten content resurfaced from a re-delivered pack — the \
         single-brain forget claim in DELETION_GUARANTEES.md no longer holds"
    );
    // But the retraction did NOT propagate: the author still holds and still
    // exports the object (why the public claim scopes plain forget to one
    // brain, and federation retraction to `tombstone`).
    assert!(!store_keys_with_content(&rt, &a, "password rotation memo").is_empty());
    assert_eq!(fed::export_pack(&a, "wing-y").unwrap().objects.len(), 1);
}
