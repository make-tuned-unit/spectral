//! Federation accuracy A/B — the eval that gates shipping federation.
//!
//! Question: does merging a teammate's shared wing into a user's brain change
//! end-to-end answer accuracy? Same actor, same judge, same questions; the only
//! variable is whether the shared wing is merged.
//!
//! Setup per question (from a converted LoCoMo file): the conversation's two
//! speakers become two brains. The `user`-role turns are the user's PRIVATE
//! memories; the `assistant`-role turns are the TEAMMATE's brain, shared into
//! wing "team" and exported as a pack.
//!
//!   Arm P (private-only): recall_scoped(All) on the user's brain, pre-import.
//!   Arm F (federated):    import the pack, recall_scoped(All) again.
//!
//! Both arms run the real federation recall path (provenance-tagged, spreading
//! on), so this also end-to-end validates the sync wiring. Instrumented so a
//! regression attributes to flooding: each F context records how many shared
//! memories entered and how many private ones were displaced vs arm P.
//!
//! Ordering note: arm P's recall auto-reinforces its hits before arm F runs —
//! a small, realistic bias (a used memory strengthens) shared by both arms'
//! corpus; noted rather than eliminated.
//!
//! Usage:
//!   ANTHROPIC_API_KEY=... federation_ab <converted_locomo.json> [--per-cat N] [--output out.json]

use anyhow::{Context, Result};
use spectral_bench_accuracy::actor::{Actor, AnthropicActor};
use spectral_bench_accuracy::dataset::{Category, Question};
use spectral_bench_accuracy::judge::{AnthropicJudge, Judge};
use spectral_bench_accuracy::retrieval::QuestionType;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy, RememberOpts};
use spectral_graph::federation_recall::RealmScope;
use spectral_ingest::federation_sync::Origin;
use std::collections::HashMap;
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

/// Ingest one role's turns into a brain. Returns the number ingested.
fn ingest_role(q: &Question, br: &Brain, role: &str, vis: Visibility) -> Result<usize> {
    let mut n = 0;
    for (si, session) in q.haystack_sessions.iter().enumerate() {
        let sid = q
            .haystack_session_ids
            .get(si)
            .cloned()
            .unwrap_or_else(|| format!("s{si}"));
        let date = q.haystack_dates.get(si).map(String::as_str).unwrap_or("");
        for (ti, turn) in session.iter().enumerate() {
            if turn.role != role {
                continue;
            }
            br.remember_with(
                &format!("{sid}:turn:{ti}:{role}"),
                &turn.content,
                RememberOpts {
                    visibility: vis,
                    episode_id: Some(sid.clone()),
                    created_at: parse_date(date),
                    ..Default::default()
                },
            )
            .map_err(|e| anyhow::anyhow!("remember: {e}"))?;
            n += 1;
        }
    }
    Ok(n)
}

fn context_lines(hits: &[(spectral_ingest::MemoryHit, Origin)]) -> Vec<String> {
    hits.iter()
        .map(|(h, _)| {
            let date = h.created_at.as_deref().unwrap_or("undated");
            format!("[{date}] {}: {}", h.key, h.content)
        })
        .collect()
}

#[derive(serde::Serialize)]
struct Row {
    question_id: String,
    category: String,
    p_correct: bool,
    f_correct: bool,
    p_context: usize,
    f_context: usize,
    f_shared_in_context: usize,
    f_private_displaced: usize,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dataset_path = args
        .get(1)
        .context("usage: federation_ab <dataset.json> [--per-cat N] [--output out.json]")?;
    let mut per_cat = 10usize;
    let mut output = "federation-ab.json".to_string();
    let mut i = 2;
    while i + 1 < args.len() {
        match args[i].as_str() {
            "--per-cat" => per_cat = args[i + 1].parse()?,
            "--output" => output = args[i + 1].clone(),
            other => anyhow::bail!("unknown arg {other}"),
        }
        i += 2;
    }
    let api_key = std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY not set")?;
    let base = "https://api.anthropic.com".to_string();
    let model = "claude-sonnet-4-6".to_string();
    let actor = AnthropicActor::new(api_key.clone(), model.clone(), base.clone());
    let judge = AnthropicJudge::new(api_key, model, base);

    let all: Vec<Question> =
        serde_json::from_str(&std::fs::read_to_string(dataset_path)?).context("parse dataset")?;
    // Deterministic sample: first per_cat per category, in file order.
    let mut taken: HashMap<String, usize> = HashMap::new();
    let sample: Vec<&Question> = all
        .iter()
        .filter(|q| {
            let c = taken.entry(q.question_type.clone()).or_insert(0);
            *c += 1;
            *c <= per_cat
        })
        .collect();
    eprintln!(
        "federation A/B over {} questions ({per_cat}/category)",
        sample.len()
    );

    let mut rows: Vec<Row> = Vec::new();
    for (qi, q) in sample.iter().enumerate() {
        let category = Category::from_question_type(&q.question_type)?;
        let shape = QuestionType::classify(&q.question);
        let qdate = q.question_date.as_deref().unwrap_or("unknown");

        // User's brain: private = user-role turns.
        let tu = TempDir::new()?;
        let user = brain(tu.path())?;
        let n_priv = ingest_role(q, &user, "user", Visibility::Private)?;

        // Teammate's brain: assistant-role turns, shared into wing "team".
        let tt = TempDir::new()?;
        let mate = brain(tt.path())?;
        let n_shared = ingest_role(q, &mate, "assistant", Visibility::Team)?;
        for (si, session) in q.haystack_sessions.iter().enumerate() {
            let sid = q
                .haystack_session_ids
                .get(si)
                .cloned()
                .unwrap_or_else(|| format!("s{si}"));
            for (ti, turn) in session.iter().enumerate() {
                if turn.role == "assistant" {
                    mate.share_memory(&format!("{sid}:turn:{ti}:assistant"), "team")
                        .map_err(|e| anyhow::anyhow!("share: {e}"))?;
                }
            }
        }
        let pack = mate
            .export_shared_wing("team")
            .map_err(|e| anyhow::anyhow!("export: {e}"))?;

        // Arm P: private-only recall (federation path, nothing imported yet).
        let p_hits = user
            .recall_scoped(&q.question, RealmScope::All)
            .map_err(|e| anyhow::anyhow!("recall P: {e}"))?;
        let p_ctx = context_lines(&p_hits);
        let (p_ans, _) = actor.answer(&q.question, qdate, &p_ctx, shape)?;
        let (p_grade, _) = judge.grade(&q.question, &p_ans, &q.answer_text(), category)?;

        // Import the teammate's pack; Arm F: federated recall.
        let merged = user
            .import_shared_wing(&pack)
            .map_err(|e| anyhow::anyhow!("import: {e}"))?;
        let f_hits = user
            .recall_scoped(&q.question, RealmScope::All)
            .map_err(|e| anyhow::anyhow!("recall F: {e}"))?;
        let f_shared = f_hits
            .iter()
            .filter(|(_, o)| matches!(o, Origin::Shared { .. }))
            .count();
        // Flooding attribution: private hits present in P's context but pushed out of F's.
        let f_priv_keys: std::collections::HashSet<&str> = f_hits
            .iter()
            .filter(|(_, o)| matches!(o, Origin::Private))
            .map(|(h, _)| h.key.as_str())
            .collect();
        let displaced = p_hits
            .iter()
            .filter(|(h, _)| !f_priv_keys.contains(h.key.as_str()))
            .count();
        let f_ctx = context_lines(&f_hits);
        let (f_ans, _) = actor.answer(&q.question, qdate, &f_ctx, shape)?;
        let (f_grade, _) = judge.grade(&q.question, &f_ans, &q.answer_text(), category)?;

        eprintln!(
            "[{}/{}] {} priv={n_priv} shared={n_shared} merged={merged} | P {} ({} mem) -> F {} ({} mem, {} shared, {} displaced)",
            qi + 1,
            sample.len(),
            q.question_id,
            if p_grade.correct { "PASS" } else { "fail" },
            p_ctx.len(),
            if f_grade.correct { "PASS" } else { "fail" },
            f_ctx.len(),
            f_shared,
            displaced,
        );
        rows.push(Row {
            question_id: q.question_id.clone(),
            category: q.question_type.clone(),
            p_correct: p_grade.correct,
            f_correct: f_grade.correct,
            p_context: p_ctx.len(),
            f_context: f_ctx.len(),
            f_shared_in_context: f_shared,
            f_private_displaced: displaced,
        });
    }

    // Summary
    let mut by: HashMap<&str, (usize, usize, usize)> = HashMap::new(); // (n, p_ok, f_ok)
    for r in &rows {
        let e = by.entry(r.category.as_str()).or_default();
        e.0 += 1;
        e.1 += r.p_correct as usize;
        e.2 += r.f_correct as usize;
    }
    println!("\n=== FEDERATION ACCURACY A/B (private-only vs private+shared-merged) ===");
    println!(
        "{:<26}{:>4}{:>12}{:>12}{:>7}",
        "category", "n", "private", "federated", "net"
    );
    let (mut n, mut p, mut f) = (0, 0, 0);
    let mut cats: Vec<_> = by.iter().collect();
    cats.sort();
    for (c, (cn, cp, cf)) in cats {
        println!(
            "{c:<26}{cn:>4}{:>11.0}%{:>11.0}%{:>+7}",
            100.0 * *cp as f64 / *cn as f64,
            100.0 * *cf as f64 / *cn as f64,
            *cf as isize - *cp as isize
        );
        n += cn;
        p += cp;
        f += cf;
    }
    println!(
        "{:<26}{n:>4}{:>11.0}%{:>11.0}%{:>+7}",
        "TOTAL",
        100.0 * p as f64 / n as f64,
        100.0 * f as f64 / n as f64,
        f as isize - p as isize
    );
    let fixed = rows.iter().filter(|r| !r.p_correct && r.f_correct).count();
    let broke = rows.iter().filter(|r| r.p_correct && !r.f_correct).count();
    println!("fixed by federation: {fixed}   broken by federation: {broke}");

    std::fs::write(&output, serde_json::to_string_pretty(&rows)?)?;
    eprintln!("rows written to {output}");
    Ok(())
}
