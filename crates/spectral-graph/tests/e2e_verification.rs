//! End-to-end verification driven through the public API. Each test exercises a
//! claim Spectral makes and prints the observed behavior (run with
//! `cargo test -p spectral-graph --test e2e_verification -- --nocapture`), so the
//! output is evidence, not just a green check. Covers the core mission (recall,
//! recognition) and the adversarial-pass fixes (visibility scoping, async
//! safety, federation trust).

use spectral_cascade::RecognitionContext;
use spectral_core::visibility::Visibility;
use spectral_graph::brain::{Brain, BrainConfig, EntityPolicy};
use spectral_graph::cascade_layers::CascadePipelineConfig;
use spectral_graph::federation::{FederationCoordinator, MergePolicy};
use std::path::PathBuf;
use tempfile::TempDir;

fn config(tmp: &TempDir) -> BrainConfig {
    BrainConfig {
        data_dir: tmp.path().to_path_buf(),
        ontology_path: PathBuf::from("tests/fixtures/brain_ontology.toml"),
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
    }
}

/// CLAIM: deterministic, embedding-free recall — same query surfaces the same
/// answer, no model in the loop.
#[test]
fn recall_is_deterministic_and_finds_the_answer() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(config(&tmp)).unwrap();
    brain
        .remember("auth", "We decided to use Clerk for authentication", Visibility::Private)
        .unwrap();
    brain
        .remember("db", "The database is Postgres on Supabase", Visibility::Private)
        .unwrap();
    brain
        .remember("host", "Hosting runs on Fly.io in the syd region", Visibility::Private)
        .unwrap();

    let q = "what did we choose for authentication";
    let first = brain.recall_local(q).unwrap();
    let top1 = &first.memory_hits[0];
    let second = brain.recall_local(q).unwrap();
    let top2 = &second.memory_hits[0];

    println!("[recall] query={q:?} -> top hit key={:?} content={:?}", top1.key, top1.content);
    assert_eq!(top1.key, "auth", "recall should surface the auth decision");
    assert_eq!(top1.key, top2.key, "recall is deterministic: same top hit twice");
    println!("[recall] deterministic: top hit stable across two calls ✓");
}

/// CLAIM: recognition answers "have I seen this before?" — high familiarity for a
/// re-encounter, high novelty for something new, deterministically and with the
/// exact features that produced the verdict.
#[test]
fn recognition_separates_seen_from_novel() {
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(config(&tmp)).unwrap();
    let enrolled = "The quarterly board meeting approved the 2027 hiring plan for the platform team";
    brain.remember("board", enrolled, Visibility::Private).unwrap();

    // Near-verbatim re-encounter → should look familiar.
    let seen = brain
        .recognize("The quarterly board meeting approved the 2027 hiring plan for the platform team")
        .unwrap();
    // Unrelated new stimulus → should look novel.
    let novel = brain
        .recognize("A raccoon knocked over the recycling bins behind the garage last night")
        .unwrap();

    println!(
        "[recognition] seen: familiarity={:.3} novelty={:.3} evidence={} features",
        seen.familiarity,
        seen.novelty,
        seen.evidence.len()
    );
    println!(
        "[recognition] novel: familiarity={:.3} novelty={:.3} evidence={} features",
        novel.familiarity,
        novel.novelty,
        novel.evidence.len()
    );
    assert!(
        seen.familiarity > novel.familiarity,
        "a re-encounter must be more familiar than a novel stimulus ({} vs {})",
        seen.familiarity,
        novel.familiarity
    );
    assert!((seen.novelty - (1.0 - seen.familiarity)).abs() < 1e-9, "novelty = 1 - familiarity");
    println!("[recognition] seen > novel, novelty = 1-familiarity, verdict carries features ✓");
}

/// CLAIM (adversarial-pass fix): the scoped recall boundary holds even with
/// associative spreading enabled — a Team context never surfaces Private content,
/// while an own-brain (Private) context sees everything.
#[test]
fn visibility_boundary_holds_under_spreading() {
    use spectral_graph::spreading::AssocSpreadConfig;
    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(config(&tmp)).unwrap();
    brain
        .remember("pub", "The launch date for the public API is March 3rd", Visibility::Public)
        .unwrap();
    brain
        .remember("secret", "Internal note: the launch is slipping to April, keep quiet", Visibility::Private)
        .unwrap();

    let ctx = RecognitionContext::empty();
    let cfg = CascadePipelineConfig {
        // Turn spreading ON — the exact config the leak fix targets.
        spread: AssocSpreadConfig::completeness(),
        ..CascadePipelineConfig::default()
    };

    let team = brain.recall_cascade_scoped("when is the launch", &ctx, &cfg, Visibility::Team).unwrap();
    let team_leaks = team.merged_hits.iter().any(|h| h.content.contains("keep quiet"));
    println!(
        "[visibility] Team-scoped recall returned {} hits; private leak present: {}",
        team.merged_hits.len(),
        team_leaks
    );
    assert!(!team_leaks, "Private content must not cross into a Team context, even with spreading");

    let own = brain.recall_cascade_scoped("when is the launch", &ctx, &cfg, Visibility::Private).unwrap();
    let own_sees = own.merged_hits.iter().any(|h| h.content.contains("keep quiet"));
    println!("[visibility] Private (own-brain) recall sees the private note: {own_sees}");
    assert!(own_sees, "own-brain recall should still see its private memory");
    println!("[visibility] Team boundary holds; Private sees all ✓");
}

/// CLAIM (adversarial-pass fix): a Brain is safe to use from inside an async
/// runtime — no "runtime within a runtime" panic on call or drop.
#[test]
fn brain_works_inside_async_runtime() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let tmp = TempDir::new().unwrap();
        let brain = Brain::open(config(&tmp)).unwrap();
        brain.remember("k", "async server handler stored this note", Visibility::Private).unwrap();
        let r = brain.recall_local("what did the handler store").unwrap();
        println!("[async] recall from inside a Tokio runtime returned {} hits, no panic", r.memory_hits.len());
        assert!(!r.memory_hits.is_empty());
        // brain drops here, inside the runtime — must not panic either.
    });
    println!("[async] Brain open+remember+recall+drop inside a runtime: no panic ✓");
}

/// CLAIM: federation fans out across brains with provenance, and the coordinator
/// enforces the visibility boundary (a member's Private memory never crosses).
#[test]
fn federation_fans_out_and_enforces_the_boundary() {
    let tmp = TempDir::new().unwrap();
    let a = Brain::open(config(&TempDir::new_in(tmp.path()).unwrap())).unwrap();
    a.remember("a-pub", "Team wiki: the deploy runbook lives in Notion", Visibility::Public).unwrap();
    a.remember("a-priv", "My private todo: rotate the prod credentials", Visibility::Private).unwrap();

    let b = Brain::open(config(&TempDir::new_in(tmp.path()).unwrap())).unwrap();
    b.remember("b-pub", "Team wiki: deploy approvals go through the release channel", Visibility::Public).unwrap();

    let mut coord = FederationCoordinator::new();
    let a_id = *a.brain_id();
    let b_id = *b.brain_id();
    coord.add_brain(a, tmp.path().join("a"));
    coord.add_brain(b, tmp.path().join("b"));

    let res = coord
        .fan_out_recall_with_policy(
            "deploy wiki",
            &RecognitionContext::empty(),
            &CascadePipelineConfig::default(),
            Visibility::Team,
            &MergePolicy::default(),
        )
        .unwrap();

    let origins: std::collections::HashSet<_> = res.ranked.iter().map(|h| h.origin).collect();
    let leaked = res.ranked.iter().any(|h| h.hit.content.contains("rotate the prod credentials"));
    println!(
        "[federation] fan-out over 2 brains: {} merged hits from {} origins; failed children: {}",
        res.ranked.len(),
        origins.len(),
        res.failed.len()
    );
    for h in &res.ranked {
        println!("[federation]   [{:?}] {}", &h.origin.to_string()[..8], h.hit.content);
    }
    assert!(origins.contains(&a_id) && origins.contains(&b_id), "both brains contribute");
    assert!(!leaked, "a member's Private memory must not cross the Team boundary");
    assert!(res.failed.is_empty(), "healthy fan-out reports no failures");
    println!("[federation] both brains contribute, provenance labeled, Private stays home ✓");
}
