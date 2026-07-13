//! BM25F column-weight experiment — is the FTS `key` column helping or hurting
//! recall? `fts_search` orders the candidate pool by
//! `bm25(memories_fts, 1.0, 1.0, 0.5)` (columns: key, content, description), so
//! these weights decide WHICH memories enter the re-ranking pool at all — a
//! pure recall lever. Structural keys (`s2:turn:2:user`) tokenize into
//! `user`/`turn`/`assistant`, which can inject noise on content queries; semantic
//! keys (`project:alpha:decision`) can inject rare high-IDF tokens that
//! over-promote. This bench measures answer-in-pool recall across weight schemes
//! on a realistic mixed corpus, with NO production change. Deterministic, $0.
//!
//! Run: `cargo run -p spectral-bench-real --bin bm25_weights_experiment`

use rusqlite::{Connection, OpenFlags};
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
use std::path::{Path, PathBuf};

fn open(dir: &Path) -> (Brain, PathBuf) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("ontology.toml"), "version = 1\n").unwrap();
    let db = dir.join("memory.db");
    let brain = Brain::open(BrainConfig {
        data_dir: dir.to_path_buf(),
        ontology_path: dir.join("ontology.toml"),
        memory_db_path: Some(db.clone()),
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
    (brain, db)
}

/// Faithful mirror of the FTS MATCH-query construction in `fts_search`:
/// lowercase, strip trailing possessive, keep word chars, len>1, quote, OR-join.
fn match_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|w| {
            let base = w.strip_suffix("'s").or_else(|| w.strip_suffix('\u{2019}')).unwrap_or(w);
            base.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect::<String>()
        })
        .filter(|w| w.len() > 1)
        .map(|w| format!("\"{}\"", w))
        .collect::<Vec<_>>()
        .join(" OR ")
}

struct Case {
    query: &'static str,
    answer_key: &'static str,
}

fn main() {
    let dir = std::env::temp_dir().join("spectral-bm25-weights");
    let _ = std::fs::remove_dir_all(&dir);
    let (brain, db) = open(&dir);

    // ── Corpus ──
    // Multi-session chat history with STRUCTURAL keys (LongMemEval style): the
    // key carries role/turn tokens (user/assistant/turn) that repeat across the
    // whole corpus. Plus a few SEMANTIC keys (Permagent style) whose tokens are
    // rare and content-like. Answer turns are seeded among distractors so pool
    // truncation matters.
    let turns: &[(&str, &str)] = &[
        // Session 1 — the answer-bearing turns we will query for.
        ("s1:turn:0:user", "I finally moved apartments this weekend, the new place is in Oakland."),
        ("s1:turn:1:assistant", "Congratulations on the move. How is the new neighborhood?"),
        ("s1:turn:2:user", "My sister Priya just started her residency in pediatric cardiology."),
        ("s1:turn:3:assistant", "That is a demanding specialty. She must have worked hard for it."),
        // Session 2 — work reorg, an answer turn, plus role-token noise.
        ("s2:turn:0:user", "The Q2 roadmap got reshuffled and nobody told the team until the last minute."),
        ("s2:turn:1:assistant", "That kind of last-minute change is demoralizing. Was a reason given?"),
        ("s2:turn:2:user", "Reorg. Marcus got bumped up to Director of Engineering, which he earned."),
        ("s2:turn:3:assistant", "Congrats to Marcus, but a leaderless squad mid-roadmap is tough."),
        // Session 3 — a food thread, all distractors, dense role tokens.
        ("s3:turn:0:user", "Tried a new ramen place downtown, the tonkotsu broth was incredible."),
        ("s3:turn:1:assistant", "Tonkotsu done well is a labor of love. Did you get an egg?"),
        ("s3:turn:2:user", "Yes, a perfect ajitama. The user next to me ordered three bowls."),
        ("s3:turn:3:assistant", "Three bowls is serious dedication from that user at the ramen bar."),
        // Session 4 — travel, distractors that mention 'engineering' and 'residency'
        // in passing to stress-test high-IDF collisions.
        ("s4:turn:0:user", "Booked a trip to Tokyo, want to see the engineering behind the bullet trains."),
        ("s4:turn:1:assistant", "The Shinkansen is an engineering marvel. Any residency of interest there?"),
        ("s4:turn:2:user", "Might stay in a residency-style long-term hotel in Shibuya."),
        // Semantic-key memories (Permagent style): rare, content-like key tokens.
        ("project:alpha:decision", "We decided to ship the search rewrite in Q4 after the review."),
        ("project:alpha:owner", "Sofia owns the alpha project and reports status every Friday."),
        ("person:marcus:role", "Marcus is the new Director of Engineering as of the Q2 reorg."),
    ];
    for (k, c) in turns {
        brain.remember(k, c, Visibility::Private).unwrap();
    }

    // ── Query set with known answers ──
    // Each query's answer lives in ONE turn; other turns are distractors. Some
    // queries deliberately contain role words ("user") or high-IDF terms that
    // also appear in irrelevant KEYS, to expose key-column noise/over-promotion.
    let cases = &[
        Case { query: "What is Marcus's new job title?", answer_key: "s2:turn:2:user" },
        Case { query: "What did the user say about their sister's medical career?", answer_key: "s1:turn:2:user" },
        Case { query: "Where did the user move to?", answer_key: "s1:turn:0:user" },
        Case { query: "When will the search rewrite ship?", answer_key: "project:alpha:decision" },
        Case { query: "Who owns the alpha project?", answer_key: "project:alpha:owner" },
        Case { query: "What food did the user try downtown?", answer_key: "s3:turn:0:user" },
    ];

    // ── Weight schemes (key, content, description) ──
    let schemes: &[(&str, f64, f64, f64)] = &[
        ("current  (1.0,1.0,0.5)", 1.0, 1.0, 0.5),
        ("key=0.5  (0.5,1.0,0.5)", 0.5, 1.0, 0.5),
        ("key=0.25 (0.25,1.0,.5)", 0.25, 1.0, 0.5),
        ("key=0.1  (0.1,1.0,0.5)", 0.1, 1.0, 0.5),
        ("key=0    (0.0,1.0,0.5)", 0.0, 1.0, 0.5),
        ("desc=1.0 (1.0,1.0,1.0)", 1.0, 1.0, 1.0),
        ("cont-hi  (0.0,1.0,0.3)", 0.0, 1.0, 0.3),
    ];

    // Flush all writes (drop the brain → connection closed, WAL checkpointed),
    // then open our own read-only connection for direct bm25 probing. FTS5 and
    // the porter tokenizer are compiled into the same SQLite the store used.
    drop(brain);
    let conn = Connection::open_with_flags(
        &db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();

    // Pool sizes that matter: recall@pool. A tight pool (small k) is where noise
    // in the key column can crowd out the answer.
    let pools = [3usize, 5, 10];

    println!("=== BM25F column-weight experiment ===");
    println!("corpus: {} memories ({} structural-key turns + semantic keys)\n", turns.len(), 16);
    println!("answer-in-pool recall across {} queries (higher = better):\n", cases.len());
    print!("{:<24}", "scheme");
    for p in &pools {
        print!("  recall@{p:<3}");
    }
    println!();

    let mut per_scheme_at5: Vec<(String, f64)> = Vec::new();
    for (label, wk, wc, wd) in schemes {
        print!("{label:<24}");
        for &pool in &pools {
            let mut hit = 0usize;
            for case in cases {
                let mq = match_query(case.query);
                let sql = format!(
                    "SELECT key FROM memories_fts WHERE memories_fts MATCH ?1 \
                     ORDER BY bm25(memories_fts, {wk}, {wc}, {wd}) LIMIT ?2"
                );
                let mut stmt = conn.prepare(&sql).unwrap();
                let keys: Vec<String> = stmt
                    .query_map(rusqlite::params![mq, pool as i64], |r| r.get::<_, String>(0))
                    .unwrap()
                    .filter_map(|r| r.ok())
                    .collect();
                if keys.iter().any(|k| k == case.answer_key) {
                    hit += 1;
                }
            }
            let recall = hit as f64 / cases.len() as f64;
            print!("  {:>7.2}   ", recall);
            if pool == 5 {
                per_scheme_at5.push((label.to_string(), recall));
            }
        }
        println!();
    }

    // ── Per-query breakdown at the tight pool (k=3) for the current scheme vs
    //    the best key<1 scheme, to see exactly which queries move. ──
    println!("\n--- per-query rank of the answer (pool ordered by bm25), pool=10 ---");
    println!("{:<40}{:>14}{:>14}", "query", "key=1.0 rank", "key=0.0 rank");
    for case in cases {
        let mq = match_query(case.query);
        let rank_of = |wk: f64| -> String {
            let sql = format!(
                "SELECT key FROM memories_fts WHERE memories_fts MATCH ?1 \
                 ORDER BY bm25(memories_fts, {wk}, 1.0, 0.5) LIMIT 50"
            );
            let mut stmt = conn.prepare(&sql).unwrap();
            let keys: Vec<String> = stmt
                .query_map(rusqlite::params![mq, ], |r| r.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();
            match keys.iter().position(|k| k == case.answer_key) {
                Some(i) => format!("{}", i + 1),
                None => "MISS".to_string(),
            }
        };
        let q = if case.query.len() > 38 { &case.query[..38] } else { case.query };
        println!("{:<40}{:>14}{:>14}", q, rank_of(1.0), rank_of(0.0));
    }

    println!("\nDeterministic, $0, no LLM. Weights decide the recall pool; this");
    println!("measures whether the `key` column earns its place before we change a default.");
}
