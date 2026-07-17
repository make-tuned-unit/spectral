//! Fusion at scale — honest recall@k, porter vs fusion, on a large noisy corpus.
//!
//! The fusion micro-bench proved the *mechanism* on 18 memories. This scales to
//! a ~300-memory corpus where distractors compete for the same query vocabulary,
//! so top-k truncation actually bites — the regime where fusion can help OR
//! hurt. Queries are a realistic MIX, NOT rigged for fusion: `collision` (porter
//! over-stems the answer term into a distractor's term), `inflection` (query is
//! an inflected form of a root in the answer — porter wins), and `neutral`
//! (ordinary multi-word queries with no stemming quirk).
//! Neutral dominates (as in real workloads), so a fusion regression on ordinary
//! queries would drag the aggregate down and be visible. Deterministic ($0, no
//! LLM): a fixed-seed LCG builds the corpus; both brains see identical data.
//!
//! Run: `cargo run -p spectral-bench-real --bin fusion_scale_bench`

use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RecallTopKConfig};
use std::path::Path;

fn open(dir: &Path, fusion: bool) -> Brain {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("ontology.toml"), "version = 1\n").unwrap();
    if fusion {
        std::env::set_var("SPECTRAL_FTS_FUSION", "1");
    }
    let brain = Brain::open(BrainConfig {
        data_dir: dir.to_path_buf(),
        ontology_path: dir.join("ontology.toml"),
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
    std::env::remove_var("SPECTRAL_FTS_FUSION");
    brain
}

/// Tiny deterministic LCG (reproducible corpus, no external rand).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 16
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() as usize) % xs.len()]
    }
}

struct Case {
    query: String,
    // Opaque content-hash-style key (NOT derived from the query term) — the FTS
    // `key` column is weighted in bm25, so a query-derived key would rig recall.
    answer_key: String,
    bucket: &'static str,
}

fn main() {
    // Common filler vocabulary — distractors are built from these so ordinary
    // query terms ("report", "review", "team", "update") have real competition.
    let subjects = [
        "The team",
        "Our group",
        "The committee",
        "The vendor",
        "The client",
        "My manager",
        "The squad",
        "Finance",
        "The board",
        "Engineering",
    ];
    let verbs = [
        "reviewed",
        "updated",
        "flagged",
        "shipped",
        "postponed",
        "approved",
        "revised",
        "discussed",
        "audited",
        "finalized",
    ];
    let objects = [
        "the quarterly report",
        "the budget review",
        "the status update",
        "the launch plan",
        "the hiring pipeline",
        "the roadmap",
        "the incident postmortem",
        "the vendor contract",
        "the design doc",
        "the migration",
    ];
    let tails = [
        "ahead of the deadline",
        "after a long debate",
        "during the offsite",
        "before the release window",
        "with no objections",
        "over the weekend",
        "in the Monday sync",
        "against the forecast",
    ];

    // Answer memories + their queries. Collision/inflection queries are SINGLE
    // operative words (so the answer can't win on an extra distinctive term —
    // ranking is decided purely on the quirk term). Neutral queries are ordinary
    // multi-word phrases. `collide` = a short distractor sentence sharing the
    // answer term's porter stem; it is inserted many times to crowd the top
    // ranks under porter (the regime where fusion earns its keep).
    let answers: &[(&str, &str, &str, &str, &str)] = &[
        // (key, content, query, bucket, collide-distractor or "")
        (
            "ans:university",
            "The state university confirmed its national research ranking rose again this year",
            "university",
            "collision",
            "The universe is vast and old",
        ),
        (
            "ans:organization",
            "The new organization chart restructured every internal reporting line across teams",
            "organization",
            "collision",
            "The pipe organ boomed in the nave",
        ),
        (
            "ans:policy",
            "The remote-work policy document was finalized after a long and contentious debate",
            "policy",
            "collision",
            "The police arrived within minutes",
        ),
        (
            "ans:operative",
            "The operative clause in the settlement was struck by outside legal counsel",
            "operative",
            "collision",
            "The operation began at dawn",
        ),
        (
            "ans:generalize",
            "The paper tried to generalize the finding beyond the narrow sampled population",
            "generalize",
            "collision",
            "The general addressed the troops",
        ),
        (
            "ans:marketing",
            "Marketing rebuilt the entire landing page ahead of the delayed product launch",
            "marketing",
            "collision",
            "The open-air market was crowded today",
        ),
        (
            "ans:relativity",
            "A guest seminar on the theory of relativity reshaped the physics curriculum",
            "relativity",
            "collision",
            "My relative called on Sunday night",
        ),
        (
            "ans:doctors",
            "She finally consulted a doctor about the lingering cough last week",
            "doctors",
            "inflection",
            "",
        ),
        (
            "ans:apartments",
            "We spent the whole weekend packing the apartment into cardboard boxes",
            "apartments",
            "inflection",
            "",
        ),
        (
            "ans:engineers",
            "The startup keeps trying to recruit one more senior backend engineer",
            "engineers",
            "inflection",
            "",
        ),
        (
            "ans:studies",
            "The longitudinal study tracked the same cohort for a full decade",
            "studies",
            "inflection",
            "",
        ),
        (
            "ans:running",
            "He liked to run the coastal trail every morning before work",
            "running",
            "inflection",
            "",
        ),
        (
            "ans:negotiations",
            "The contract negotiation stalled badly over the indemnity clause",
            "negotiations",
            "inflection",
            "",
        ),
        (
            "ans:deploy",
            "The blue-green deploy cut the production rollout window to minutes",
            "blue-green deploy rollout",
            "neutral",
            "",
        ),
        (
            "ans:sauna",
            "The lakeside cabin had a wood-fired sauna right by the dock",
            "lakeside cabin sauna",
            "neutral",
            "",
        ),
        (
            "ans:tax",
            "The accountant flagged a missed quarterly estimated tax payment",
            "estimated tax payment",
            "neutral",
            "",
        ),
        (
            "ans:violin",
            "She restrung the antique violin the night before the recital",
            "antique violin recital",
            "neutral",
            "",
        ),
        (
            "ans:kayak",
            "We portaged the kayak around the second waterfall on the river",
            "portaged kayak waterfall",
            "neutral",
            "",
        ),
        (
            "ans:compost",
            "The community garden finally started a shared compost program",
            "community garden compost",
            "neutral",
            "",
        ),
        (
            "ans:telescope",
            "The rooftop telescope caught the lunar eclipse remarkably clearly",
            "rooftop telescope eclipse",
            "neutral",
            "",
        ),
        (
            "ans:pension",
            "HR clarified the vesting schedule for the frozen pension plan",
            "pension vesting schedule",
            "neutral",
            "",
        ),
        (
            "ans:allergy",
            "The clinic confirmed a seasonal pollen allergy after the panel",
            "seasonal pollen allergy",
            "neutral",
            "",
        ),
        (
            "ans:mural",
            "A local artist painted a huge mural on the old depot wall",
            "artist mural depot",
            "neutral",
            "",
        ),
    ];

    // Opaque keys (a00, a01, …) so the FTS key column never leaks the query term.
    let cases: Vec<Case> = answers
        .iter()
        .enumerate()
        .map(|(i, (_, _, q, b, _))| Case {
            query: (*q).to_string(),
            answer_key: format!("a{i:02}"),
            bucket: b,
        })
        .collect();

    let seed_corpus = |brain: &Brain| {
        let mut rng = Lcg(0x5157_3ADE_1234_9001);
        // 300 filler distractors from the common vocabulary.
        for i in 0..300 {
            let c = format!(
                "{} {} {} {}",
                rng.pick(&subjects),
                rng.pick(&verbs),
                rng.pick(&objects),
                rng.pick(&tails)
            );
            brain
                .remember(&format!("distractor:{i}"), &c, Visibility::Private)
                .unwrap();
        }
        // For each collision answer, insert several DISTINCT short sentences that
        // share the answer term's porter stem (a realistic collision: different
        // documents, not identical floods — so context-dedup can't collapse them).
        // They crowd the top ranks under porter, pushing the answer past a tight
        // k; the unstemmed channel matches only the true answer, so fusion pulls
        // it back up.
        let suffixes = [
            "today",
            "this week",
            "again",
            "as usual",
            "reportedly",
            "overnight",
            "near dawn",
            "by all accounts",
        ];
        for (ai, (_, _, _, _, collide)) in answers.iter().enumerate() {
            if collide.is_empty() {
                continue;
            }
            for (j, sfx) in suffixes.iter().enumerate() {
                let c = format!("{collide} {sfx}");
                brain
                    .remember(&format!("coll:{ai}:{j}"), &c, Visibility::Private)
                    .unwrap();
            }
        }
        // Answer memories under opaque keys.
        for (i, (_, content, _, _, _)) in answers.iter().enumerate() {
            brain
                .remember(&format!("a{i:02}"), content, Visibility::Private)
                .unwrap();
        }
    };

    eprintln!("ingesting porter brain (~330 memories)...");
    let porter = open(
        &std::env::temp_dir().join("spectral-fusion-scale-porter"),
        false,
    );
    seed_corpus(&porter);
    eprintln!("ingesting fusion brain (~330 memories)...");
    let fusion = open(
        &std::env::temp_dir().join("spectral-fusion-scale-fusion"),
        true,
    );
    seed_corpus(&fusion);

    // Include K=40 — LongMemEval's actual retrieval operating point. Fusion's
    // tight-k reordering only helps the ACTOR if it moves an answer from OUTSIDE
    // top-K to INSIDE; if porter already has it within K=40, fusion is inert.
    let ks = [1usize, 5, 10, 20, 40];
    let recall_at = |brain: &Brain, case: &Case, k: usize| -> bool {
        let hits = brain
            .recall_topk_fts(
                &case.query,
                &RecallTopKConfig {
                    k: 40,
                    ..Default::default()
                },
                Visibility::Private,
            )
            .unwrap();
        hits.iter().take(k).any(|h| h.key == case.answer_key)
    };

    println!("\n=== Fusion at scale — recall@k (porter vs fusion), by query bucket ===");
    println!("corpus: 330 memories; {} queries\n", cases.len());

    let buckets = ["collision", "inflection", "neutral", "ALL"];
    for &k in &ks {
        println!("-- recall@{k} --");
        println!(
            "{:<12}{:>10}{:>10}{:>10}",
            "bucket", "porter", "fusion", "delta"
        );
        for &bkt in &buckets {
            let subset: Vec<&Case> = cases
                .iter()
                .filter(|c| bkt == "ALL" || c.bucket == bkt)
                .collect();
            let n = subset.len() as f64;
            let p = subset.iter().filter(|c| recall_at(&porter, c, k)).count() as f64 / n;
            let f = subset.iter().filter(|c| recall_at(&fusion, c, k)).count() as f64 / n;
            println!("{:<12}{:>10.2}{:>10.2}{:>+10.2}", bkt, p, f, f - p);
        }
        println!();
    }

    // ── Integrated cascade path (cascade_retrieve = TACT + FTS supplement) ──
    // The production path Permagent uses. It only reaches FTS (and thus fusion)
    // when TACT underfills — common for short keyword queries where fingerprint/
    // wing matching is weak. Confirm fusion's collision recovery survives the
    // full integrated pipeline, not just the direct topk path.
    println!("-- integrated cascade path (cascade_retrieve): recall@5 by bucket --");
    println!(
        "{:<12}{:>10}{:>10}{:>10}",
        "bucket", "porter", "fusion", "delta"
    );
    let cascade_at = |brain: &Brain, case: &Case, k: usize| -> bool {
        brain
            .cascade_retrieve(&case.query, 40)
            .unwrap()
            .iter()
            .take(k)
            .any(|h| h.key == case.answer_key)
    };
    for &bkt in &["collision", "inflection", "neutral", "ALL"] {
        let subset: Vec<&Case> = cases
            .iter()
            .filter(|c| bkt == "ALL" || c.bucket == bkt)
            .collect();
        let n = subset.len() as f64;
        let p = subset.iter().filter(|c| cascade_at(&porter, c, 5)).count() as f64 / n;
        let f = subset.iter().filter(|c| cascade_at(&fusion, c, 5)).count() as f64 / n;
        println!("{bkt:<12}{p:>10.2}{f:>10.2}{:>+10.2}", f - p);
    }
    println!();

    // ── Latency cost (the "least expensive" half of the tradeoff) ──
    // Fusion runs a second FTS query + an id-fetch, so it must earn its recall
    // lift without an unacceptable latency cost. Time all queries × repeats,
    // report per-recall median and mean. Warm the page cache first.
    let bench_latency = |brain: &Brain| -> (f64, f64) {
        for c in &cases {
            let _ = brain.recall_topk_fts(
                &c.query,
                &RecallTopKConfig {
                    k: 40,
                    ..Default::default()
                },
                Visibility::Private,
            );
        }
        let mut samples: Vec<f64> = Vec::new();
        for _ in 0..20 {
            for c in &cases {
                let t = std::time::Instant::now();
                let _ = brain
                    .recall_topk_fts(
                        &c.query,
                        &RecallTopKConfig {
                            k: 40,
                            ..Default::default()
                        },
                        Visibility::Private,
                    )
                    .unwrap();
                samples.push(t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = samples[samples.len() / 2];
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        (median, mean)
    };
    let (pm, pa) = bench_latency(&porter);
    let (fm, fa) = bench_latency(&fusion);
    println!(
        "-- recall latency per query (ms), {} queries × 20 reps --",
        cases.len()
    );
    println!("  porter  median={pm:.3}  mean={pa:.3}");
    println!("  fusion  median={fm:.3}  mean={fa:.3}");
    println!(
        "  overhead: median {:+.1}%  mean {:+.1}%\n",
        (fm / pm - 1.0) * 100.0,
        (fa / pa - 1.0) * 100.0
    );

    // Diagnostic: strip the re-ranking pipeline (signal/recency/entity/dedup) so
    // the ordering is driven by raw bm25 pool order. If porter drops collision
    // answers HERE but fusion keeps them, the re-ranker — not fusion — is what
    // neutralizes over-stemming in the full pipeline above.
    let raw_cfg = RecallTopKConfig {
        k: 40,
        apply_signal_score_weighting: false,
        apply_recency_weighting: false,
        apply_entity_resolution: false,
        apply_context_dedup: false,
        apply_declarative_boost: false,
        ..Default::default()
    };
    let raw_recall_at1 = |brain: &Brain, case: &Case| -> bool {
        brain
            .recall_topk_fts(&case.query, &raw_cfg, Visibility::Private)
            .unwrap()
            .first()
            .map(|h| h.key == case.answer_key)
            .unwrap_or(false)
    };
    println!("-- diagnostic: collision recall@1 with re-ranking OFF (raw bm25 order) --");
    let coll: Vec<&Case> = cases.iter().filter(|c| c.bucket == "collision").collect();
    let n = coll.len() as f64;
    let pr = coll.iter().filter(|c| raw_recall_at1(&porter, c)).count() as f64 / n;
    let fr = coll.iter().filter(|c| raw_recall_at1(&fusion, c)).count() as f64 / n;
    println!("  porter={pr:.2}  fusion={fr:.2}  delta={:+.2}", fr - pr);
    println!("  (if porter<fusion here but they tie above, the re-ranker absorbs the collision)\n");

    // Misses explained: for any query where fusion still misses the answer at
    // recall@1, show the answer's rank and what outranks it — surfaces recall
    // gaps neither channel fixes (e.g. an aggressive-stem flood the unstemmed
    // channel also can't disambiguate).
    println!("-- misses explained (fusion answer not rank 1), top-3 competitors --");
    let mut explained = false;
    for case in &cases {
        let hits = fusion
            .recall_topk_fts(
                &case.query,
                &RecallTopKConfig {
                    k: 40,
                    ..Default::default()
                },
                Visibility::Private,
            )
            .unwrap();
        let rank = hits.iter().position(|h| h.key == case.answer_key);
        if rank != Some(0) {
            explained = true;
            let top3: Vec<String> = hits
                .iter()
                .take(3)
                .map(|h| {
                    let c = &h.content;
                    format!("{}={:?}", h.key, &c[..c.len().min(32)])
                })
                .collect();
            let r = rank
                .map(|i| (i + 1).to_string())
                .unwrap_or_else(|| "MISS".into());
            println!(
                "  [{}] {:?} -> answer rank {r}; top3 {top3:?}",
                case.bucket, case.query
            );
        }
    }
    if !explained {
        println!("  none — every answer is rank 1 under fusion.");
    }
    println!();

    // Regression watch: any query where fusion drops the answer but porter kept it.
    println!("-- regressions (fusion worse than porter), recall@10 --");
    let mut any = false;
    for case in &cases {
        let p = recall_at(&porter, case, 10);
        let f = recall_at(&fusion, case, 10);
        if p && !f {
            any = true;
            println!(
                "  REGRESSED [{}] {:?} (answer {})",
                case.bucket, case.query, case.answer_key
            );
        }
    }
    if !any {
        println!("  none — fusion never lost an answer porter retrieved.");
    }
    println!("\nDeterministic, $0, no LLM. Honest mix (neutral-dominant); a fusion");
    println!("regression on ordinary queries would show in the ALL delta and above.");
}
