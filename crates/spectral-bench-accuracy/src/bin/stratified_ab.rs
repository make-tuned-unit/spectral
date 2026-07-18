//! Internal-federation / stratified-retrieval A/B — does partitioning ONE user's
//! memory improve multi-session recall, with content held constant?
//!
//! The federation accuracy A/B showed team-merge lifts multi-hop — but its arms
//! differed in *content*. This experiment isolates the *mechanism* (guaranteed
//! per-session representation + rank fusion) on a single user's corpus:
//!
//!   Arm M (monolith):   one brain, all turns; cascade recall (status quo).
//!   Arm S (sharded):    one brain PER SESSION; FederationCoordinator fan-out
//!                       (RRF + per-child cap) — "internal federation".
//!   Arm T (stratified): one brain; top-k pool re-ranked round-robin per session
//!                       — the in-DB, no-extra-infra version of the same idea.
//!
//! All arms emit the same context budget (K hits). Metrics are the $0 oracle
//! pair: answer-session recall and answer-key recall over the LoCoMo
//! multi-session slice (the measured weak spot: 78.6% session-recall).
//!
//! Usage: stratified_ab <converted_locomo.json> [--max-q N] [--k K]

use anyhow::{Context, Result};
use spectral_bench_accuracy::actor::{Actor, AnthropicActor};
use spectral_bench_accuracy::dataset::{Category, Question};
use spectral_bench_accuracy::judge::{AnthropicJudge, Judge};
use spectral_bench_accuracy::retrieval::QuestionType;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RecallTopKConfig, RememberOpts};
use spectral_graph::cascade_layers::CascadePipelineConfig;
use spectral_graph::federation::FederationCoordinator;
use spectral_ingest::MemoryHit;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tempfile::TempDir;

fn brain(dir: &std::path::Path) -> Result<Brain> {
    Brain::open(BrainConfig {
        data_dir: dir.to_path_buf(),
        ontology_path: PathBuf::from("crates/spectral-graph/tests/fixtures/brain_ontology.toml"),
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
    .map_err(|e| anyhow::anyhow!("brain open: {e}"))
}

fn parse_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y/%m/%d (%a) %H:%M")
        .ok()
        .map(|dt| dt.and_utc())
}

fn remember_turns(br: &Brain, q: &Question, only_session: Option<usize>) -> Result<()> {
    for (si, session) in q.haystack_sessions.iter().enumerate() {
        if let Some(only) = only_session {
            if si != only {
                continue;
            }
        }
        let sid = q.haystack_session_ids[si].clone();
        let date = q.haystack_dates.get(si).map(String::as_str).unwrap_or("");
        for (ti, turn) in session.iter().enumerate() {
            br.remember_with(
                &format!("{sid}:turn:{ti}:{}", turn.role),
                &turn.content,
                RememberOpts {
                    visibility: Visibility::Private,
                    episode_id: Some(sid.clone()),
                    created_at: parse_date(date),
                    ..Default::default()
                },
            )
            .map_err(|e| anyhow::anyhow!("remember: {e}"))?;
        }
    }
    Ok(())
}

fn session_of(key: &str) -> &str {
    key.split(":turn:").next().unwrap_or(key)
}

/// (session_recall, key_recall, zero) for a retrieved key list.
fn score(q: &Question, keys: &[String]) -> (f64, f64, bool) {
    let answer_sessions: HashSet<&str> = q
        .haystack_session_ids
        .iter()
        .filter(|s| s.starts_with("answer_"))
        .map(String::as_str)
        .collect();
    let total_keys: usize = q
        .haystack_session_ids
        .iter()
        .zip(&q.haystack_sessions)
        .filter(|(sid, _)| sid.starts_with("answer_"))
        .map(|(_, sess)| sess.len())
        .sum();
    let hit_sessions: HashSet<&str> = keys
        .iter()
        .map(|k| session_of(k))
        .filter(|s| answer_sessions.contains(s))
        .collect();
    let hit_keys = keys
        .iter()
        .filter(|k| answer_sessions.contains(session_of(k)))
        .count();
    let sr = hit_sessions.len() as f64 / answer_sessions.len().max(1) as f64;
    let kr = hit_keys as f64 / total_keys.max(1) as f64;
    (sr, kr, hit_keys == 0)
}

/// Round-robin per-session stratification of a ranked pool: sessions ordered by
/// their best hit's rank, then take each session's next-best in rounds until k.
fn stratify(pool: &[MemoryHit], k: usize) -> Vec<MemoryHit> {
    let mut per: HashMap<String, Vec<&MemoryHit>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for h in pool {
        let s = h
            .episode_id
            .clone()
            .unwrap_or_else(|| session_of(&h.key).to_string());
        if !per.contains_key(&s) {
            order.push(s.clone());
        }
        per.entry(s).or_default().push(h);
    }
    let mut out = Vec::new();
    let mut round = 0usize;
    while out.len() < k {
        let mut any = false;
        for s in &order {
            if out.len() >= k {
                break;
            }
            if let Some(h) = per[s].get(round) {
                out.push((*h).clone());
                any = true;
            }
        }
        if !any {
            break;
        }
        round += 1;
    }
    out
}

fn context_lines(hits: &[MemoryHit]) -> Vec<String> {
    hits.iter()
        .map(|h| {
            let date = h.created_at.as_deref().unwrap_or("undated");
            format!("[{date}] {}: {}", h.key, h.content)
        })
        .collect()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dataset_path = args
        .get(1)
        .context("usage: stratified_ab <dataset.json> [--max-q N] [--k K]")?;
    let mut max_q = 40usize;
    let mut k = 40usize;
    let mut accuracy = false;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--max-q" => {
                max_q = args[i + 1].parse()?;
                i += 2;
            }
            "--k" => {
                k = args[i + 1].parse()?;
                i += 2;
            }
            "--accuracy" => {
                accuracy = true;
                i += 1;
            }
            other => anyhow::bail!("unknown arg {other}"),
        }
    }
    // Accuracy leg: actor+judge over arm M vs arm T contexts (T is the shippable
    // in-DB variant; S matches T on retrieval, so we spend the calls on M vs T).
    let clients = if accuracy {
        let api_key = std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY not set")?;
        let base = "https://api.anthropic.com".to_string();
        let model = "claude-sonnet-4-6".to_string();
        Some((
            AnthropicActor::new(api_key.clone(), model.clone(), base.clone()),
            AnthropicJudge::new(api_key, model, base),
        ))
    } else {
        None
    };
    let all: Vec<Question> =
        serde_json::from_str(&std::fs::read_to_string(dataset_path)?).context("parse dataset")?;
    let sample: Vec<&Question> = all
        .iter()
        .filter(|q| q.question_type == "multi-session")
        .take(max_q)
        .collect();
    eprintln!(
        "stratified A/B over {} multi-session questions, K={k}",
        sample.len()
    );

    let ctx = spectral_cascade::RecognitionContext::empty();
    let mut sums = [[0.0f64; 2]; 3]; // arm -> [sess_sum, key_sum]
    let mut zeros = [0usize; 3];
    let mut acc = [0usize; 2]; // [monolith_ok, stratified_ok] (accuracy leg)
    let names = ["monolith", "sharded", "stratified"];

    for (qi, q) in sample.iter().enumerate() {
        // Arm M + T corpus: one brain with everything.
        let tm = TempDir::new()?;
        let mono = brain(tm.path())?;
        remember_turns(&mono, q, None)?;

        // Arm M: cascade recall, top-k (status quo).
        let m_hits = mono
            .recall_cascade(&q.question, &ctx, &CascadePipelineConfig::default())
            .map_err(|e| anyhow::anyhow!("cascade: {e}"))?;
        let m_keys: Vec<String> = m_hits
            .merged_hits
            .iter()
            .take(k)
            .map(|h| h.key.clone())
            .collect();

        // Arm T: wide top-k pool from the same brain, stratified per session.
        let pool = mono
            .recall_topk_fts(
                &q.question,
                &RecallTopKConfig {
                    k: 200,
                    ..RecallTopKConfig::default()
                },
                Visibility::Private,
            )
            .map_err(|e| anyhow::anyhow!("topk: {e}"))?;
        let t_hits = stratify(&pool, k);
        let t_keys: Vec<String> = t_hits.iter().map(|h| h.key.clone()).collect();

        // Arm S: one brain per session, coordinator fan-out (internal federation).
        let mut dirs: Vec<TempDir> = Vec::new();
        let mut coord = FederationCoordinator::new();
        for si in 0..q.haystack_sessions.len() {
            let td = TempDir::new()?;
            let b = brain(td.path())?;
            remember_turns(&b, q, Some(si))?;
            coord.add_brain(b, td.path().to_path_buf());
            dirs.push(td);
        }
        let fan = coord
            .fan_out_recall(
                &q.question,
                &ctx,
                &CascadePipelineConfig::default(),
                Visibility::Private,
            )
            .map_err(|e| anyhow::anyhow!("fan_out: {e}"))?;
        let mut seen = HashSet::new();
        let s_keys: Vec<String> = fan
            .ranked
            .iter()
            .map(|lh| lh.hit.key.clone())
            .filter(|kk| seen.insert(kk.clone()))
            .take(k)
            .collect();

        for (ai, keys) in [&m_keys, &s_keys, &t_keys].iter().enumerate() {
            let (sr, kr, z) = score(q, keys);
            sums[ai][0] += sr;
            sums[ai][1] += kr;
            zeros[ai] += z as usize;
        }
        let (msr, _, _) = score(q, &m_keys);
        let (ssr, _, _) = score(q, &s_keys);
        let (tsr, _, _) = score(q, &t_keys);

        // Accuracy leg: same actor/judge/question, only the context differs.
        let mut acc_note = String::new();
        if let Some((actor, judge)) = &clients {
            let shape = QuestionType::classify(&q.question);
            let category = Category::from_question_type(&q.question_type)?;
            let qdate = q.question_date.as_deref().unwrap_or("unknown");
            let m_ctx = context_lines(
                &m_hits
                    .merged_hits
                    .iter()
                    .take(k)
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            let t_ctx = context_lines(&t_hits);
            let (m_ans, _) = actor.answer(&q.question, qdate, &m_ctx, shape)?;
            let (m_grade, _) = judge.grade(&q.question, &m_ans, &q.answer_text(), category)?;
            let (t_ans, _) = actor.answer(&q.question, qdate, &t_ctx, shape)?;
            let (t_grade, _) = judge.grade(&q.question, &t_ans, &q.answer_text(), category)?;
            acc[0] += m_grade.correct as usize;
            acc[1] += t_grade.correct as usize;
            acc_note = format!(
                " | acc M={} T={}",
                if m_grade.correct { "PASS" } else { "fail" },
                if t_grade.correct { "PASS" } else { "fail" }
            );
        }
        eprintln!(
            "[{}/{}] {} sess-rec M={msr:.2} S={ssr:.2} T={tsr:.2}{acc_note}",
            qi + 1,
            sample.len(),
            q.question_id
        );
    }

    let n = sample.len() as f64;
    println!(
        "\n=== INTERNAL-FEDERATION / STRATIFIED RETRIEVAL A/B (multi-session, K={k}, n={}) ===",
        sample.len()
    );
    println!(
        "{:<14}{:>12}{:>10}{:>7}",
        "arm", "sess-recall", "key-rec", "zero"
    );
    for ai in 0..3 {
        println!(
            "{:<14}{:>11.1}%{:>9.1}%{:>7}",
            names[ai],
            100.0 * sums[ai][0] / n,
            100.0 * sums[ai][1] / n,
            zeros[ai]
        );
    }
    if clients.is_some() {
        println!("\n=== ACCURACY (same actor/judge, context is the only variable) ===");
        println!(
            "monolith   {:>3}/{}  ({:.0}%)\nstratified {:>3}/{}  ({:.0}%)   net {:+}",
            acc[0],
            sample.len(),
            100.0 * acc[0] as f64 / n,
            acc[1],
            sample.len(),
            100.0 * acc[1] as f64 / n,
            acc[1] as isize - acc[0] as isize
        );
    }
    Ok(())
}
