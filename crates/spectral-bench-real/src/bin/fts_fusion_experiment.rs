//! Stemmed + unstemmed RRF fusion — ceiling measurement (research lever #1).
//!
//! Porter stemming (our default) is a RECALL device that trades away PRECISION:
//! it bridges "doctors"→"doctor" (a real recall win) but also conflates
//! "university"→"univers"←"universe" (a precision loss — a distractor competes
//! with the answer and can push it past a tight k). The accepted deterministic
//! fix (Cormack 2009 RRF; Benham, "Improving Recall in Text Retrieval Using
//! Rank Fusion") is to keep BOTH representations and fuse by rank — different
//! representations satisfy the Beitzel precondition where fusion actually helps.
//!
//! This measures the CEILING before building a production double-index: on a
//! corpus with BOTH inflection cases (porter wins) and over-stemming collisions
//! (unstemmed wins), does RRF(porter, unstemmed) beat either channel alone on
//! recall@k? Deterministic, $0, no LLM, no Brain — raw FTS5.
//!
//! Run: `cargo run -p spectral-bench-real --bin fts_fusion_experiment`

use rusqlite::Connection;

/// (key, content). Answers and distractors are interleaved so a tight top-k
/// truncation is where a precision loss actually drops the answer.
const CORPUS: &[(&str, &str)] = &[
    // ── Inflection cases: query uses a plural/inflected form, content the root.
    //    Porter matches; plain unicode61 MISSES entirely (recall loss).
    ("m01", "She finally consulted a doctor about the persistent cough last week."),
    ("m02", "We spent all weekend packing the apartment into cardboard boxes."),
    ("m03", "The startup keeps trying to recruit one more backend engineer."),
    // ── Over-stemming collisions. The ANSWER term and a SHORT distractor term
    //    stem to the same root, so under porter the short distractor (higher
    //    bm25 via length normalization) outranks the longer answer — pushing the
    //    answer off rank 1. Unstemmed keeps the two terms distinct.
    ("m04", "Our state university announced it had raised its national research ranking again this year."), // answer: "university"
    ("m05", "The universe is vast."),                                                  // distractor: universe→univers
    ("m06", "The new organization quietly restructured its entire internal reporting chart across every team."), // answer: "organization"
    ("m07", "The organ played."),                                                      // distractor: organ→organ
    ("m08", "The police arrived."),                                                    // distractor: police→polic
    ("m09", "The remote-work policy document was finally finalized after a long and contentious debate."), // answer: "policy"
    ("m14", "My relative visited."),                                                   // distractor: relative→rel...
    ("m15", "Einstein's theory of relativity fundamentally reshaped the entire field of modern theoretical physics."), // answer: "relativity"
    // ── Neutral distractors (dense, share incidental tokens).
    ("m10", "The quarterly budget review flagged the cloud bill as too high."),
    ("m11", "We migrated the staging cluster to a cheaper instance family."),
    ("m12", "The team retro surfaced three recurring process complaints."),
    ("m13", "A long hike in the mountains cleared my head over the weekend."),
    ("m16", "Marketing shipped the new landing page ahead of the launch."),
    ("m17", "The open-air market sold fresh produce every Saturday morning."),
    ("m18", "The operating room was prepped for the early surgery slot."),
];

struct Case {
    query: &'static str,
    answer: &'static str,
    note: &'static str,
}

const CASES: &[Case] = &[
    Case { query: "doctors", answer: "m01", note: "inflection (porter wins)" },
    Case { query: "apartments", answer: "m02", note: "inflection (porter wins)" },
    Case { query: "engineers", answer: "m03", note: "inflection (porter wins)" },
    Case { query: "university", answer: "m04", note: "over-stem vs 'universe'" },
    Case { query: "organization", answer: "m06", note: "over-stem vs 'organ'" },
    Case { query: "policy", answer: "m09", note: "over-stem vs 'police'" },
    Case { query: "relativity", answer: "m15", note: "over-stem vs 'relative'" },
];

fn build_fts(conn: &Connection, name: &str, tokenizer: &str) {
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE {name} USING fts5(key, content, tokenize='{tokenizer}');"
    ))
    .unwrap();
    let mut stmt = conn
        .prepare(&format!("INSERT INTO {name}(key, content) VALUES (?1, ?2)"))
        .unwrap();
    for (k, c) in CORPUS {
        stmt.execute(rusqlite::params![k, c]).unwrap();
    }
}

fn match_query(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|w| w.len() > 1)
        .map(|w| format!("\"{}\"", w))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Ranked list of keys for a query against one FTS table (best first).
fn ranked(conn: &Connection, table: &str, query: &str, limit: usize) -> Vec<String> {
    let sql = format!(
        "SELECT key FROM {table} WHERE {table} MATCH ?1 ORDER BY bm25({table}) LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map(rusqlite::params![match_query(query), limit as i64], |r| {
        r.get::<_, String>(0)
    })
    .unwrap()
    .filter_map(|r| r.ok())
    .collect()
}

/// Reciprocal Rank Fusion (Cormack 2009), k=60. Fuses two ranked lists into one
/// ordered by descending fused score. Rank-based → immune to BM25's
/// un-normalized, tokenizer-dependent score scales (no normalization needed).
fn rrf_fuse(lists: &[Vec<String>], k: f64) -> Vec<String> {
    use std::collections::HashMap;
    let mut score: HashMap<&str, f64> = HashMap::new();
    for list in lists {
        for (rank, key) in list.iter().enumerate() {
            *score.entry(key.as_str()).or_insert(0.0) += 1.0 / (k + (rank + 1) as f64);
        }
    }
    let mut fused: Vec<(&str, f64)> = score.into_iter().collect();
    // Deterministic tie-break by key so the output is stable.
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(b.0)));
    fused.into_iter().map(|(k, _)| k.to_string()).collect()
}

fn main() {
    let conn = Connection::open_in_memory().unwrap();
    build_fts(&conn, "fts_porter", "porter unicode61");
    build_fts(&conn, "fts_plain", "unicode61");

    println!("=== Stemmed + unstemmed RRF fusion (ceiling measurement) ===");
    println!("corpus: {} memories; {} queries\n", CORPUS.len(), CASES.len());

    let pools = [1usize, 3, 5];
    let channels = ["porter", "unstemmed", "RRF-fused"];

    // recall@k table.
    print!("{:<12}", "channel");
    for p in &pools {
        print!("  recall@{p}");
    }
    println!();
    let wide = 50;
    for ch in &channels {
        print!("{ch:<12}");
        for &pool in &pools {
            let mut hit = 0usize;
            for case in CASES {
                let p = ranked(&conn, "fts_porter", case.query, wide);
                let u = ranked(&conn, "fts_plain", case.query, wide);
                let list = match *ch {
                    "porter" => p,
                    "unstemmed" => u,
                    _ => rrf_fuse(&[p, u], 60.0),
                };
                if list.iter().take(pool).any(|k| k == case.answer) {
                    hit += 1;
                }
            }
            print!("   {:>6.2} ", hit as f64 / CASES.len() as f64);
        }
        println!();
    }

    // Per-query answer rank in each channel — shows exactly where each wins.
    println!("\n--- answer rank per channel (lower = better; MISS = absent) ---");
    println!("{:<22}{:<28}{:>8}{:>10}{:>10}", "query", "case", "porter", "unstem", "fused");
    for case in CASES {
        let p = ranked(&conn, "fts_porter", case.query, wide);
        let u = ranked(&conn, "fts_plain", case.query, wide);
        let f = rrf_fuse(&[p.clone(), u.clone()], 60.0);
        let rank = |l: &[String]| -> String {
            l.iter().position(|k| k == case.answer).map(|i| (i + 1).to_string()).unwrap_or_else(|| "MISS".into())
        };
        println!(
            "{:<22}{:<28}{:>8}{:>10}{:>10}",
            case.query, case.note, rank(&p), rank(&u), rank(&f)
        );
    }

    // ── RRF-k sensitivity ──
    // RRF's only knob is k (the rank offset). The field default is 60. Sweep it
    // to confirm the fused recall is robust across a wide range for our two
    // channels (not a fragile point-tuned value).
    println!("\n--- RRF-k sensitivity: fused recall@1 / recall@3 across k ---");
    println!("{:<8}{:>12}{:>12}", "k", "recall@1", "recall@3");
    for k in [1.0, 5.0, 10.0, 30.0, 60.0, 100.0, 300.0] {
        let mut hit1 = 0usize;
        let mut hit3 = 0usize;
        for case in CASES {
            let p = ranked(&conn, "fts_porter", case.query, wide);
            let u = ranked(&conn, "fts_plain", case.query, wide);
            let f = rrf_fuse(&[p, u], k);
            if f.first().map(|x| x == case.answer).unwrap_or(false) {
                hit1 += 1;
            }
            if f.iter().take(3).any(|x| x == case.answer) {
                hit3 += 1;
            }
        }
        let n = CASES.len() as f64;
        println!("{k:<8.0}{:>12.2}{:>12.2}", hit1 as f64 / n, hit3 as f64 / n);
    }

    println!("\nDeterministic, $0, no LLM. If fused ≥ max(porter, unstemmed) at every k,");
    println!("the double-index + RRF fusion lever is worth building in production.");
}
