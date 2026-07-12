//! Recognition-quality benchmark: familiar-vs-novel discrimination AUC,
//! measuring the lift from adding the MinHash channel.
//!
//! Scores the recognition engine on the task it is FOR — re-encounter
//! detection ("have I seen this error / decision / person before?") — and
//! reports ROC-AUC with the MinHash channel OFF (peak-pair + winnowed-gram
//! only) vs ON (the shipped default), on the same corpus.
//!
//! The corpus is deliberately hard for the geometric channel: positives
//! include HEAVILY degraded re-encounters (many tokens dropped, so pair/gram
//! geometry weakens) and negatives are lexical NEAR-MISSES that share
//! structural vocabulary with enrolled items. MinHash's token-set overlap is
//! the widely-accepted lexical sketch that lifts separation here
//! (RECOGNITION_BASELINE: MinHash AUC ~0.998 vs peak-pair ~0.941) — kept
//! auditable via per-match evidence ("minhash: jaccard 0.87").
//!
//! Run: `cargo run -p spectral-bench-real --bin recognition_bench`

use spectral_recognition::{
    InMemoryRecognitionStore, MinHashConfig, RecognitionConfig, RecognitionEngine, Verdict,
};

const CORPUS: &[&str] = &[
    "Pod task-runner-7f9 OOMKilled at 512Mi during the nightly batch reindex job",
    "TLS certificate for api.acme.dev expired causing 502 errors from the edge proxy",
    "Decided to migrate the billing service from MySQL to Postgres 16 in the third quarter",
    "Alice reported the checkout flow times out when Stripe webhook retries exceed five attempts",
    "Deploy 2024-11 rolled back after p99 latency spiked to 4200ms on the us-east-1 cluster",
    "Kafka consumer group orders-worker lag hit 1.2M during the flash sale window on Friday",
    "Postmortem Redis eviction under maxmemory 8gb dropped session tokens for twelve minutes",
    "The gRPC gateway rejected requests over 4MB after the protobuf schema version bump",
    "Bob merged the feature flag rollout for dark mode behind cohort experiment 214",
    "Nightly ETL failed schema drift in the events table added a nullable column device_id",
    "Incident 88 DNS resolver cache poisoning suspected on the staging kubernetes cluster",
    "Chose NATS JetStream over RabbitMQ for at-least-once delivery in the ingestion pipeline",
];

/// Lexical NEAR-MISS negatives: same domain and shared structural vocabulary
/// as enrolled items, but genuinely different specifics (different entities,
/// numbers, outcomes). These are the hard negatives — a shallow channel that
/// keys on structure alone can be fooled; token-set overlap is low.
const NEAR_MISS: &[&str] = &[
    "Pod web-frontend-2a1 CrashLooped at 256Mi during the morning smoke test run",
    "TLS certificate for cdn.other.dev renewed successfully avoiding errors at the edge proxy",
    "Decided to keep the billing service on MySQL and skip the Postgres migration this quarter",
    "Bob reported the signup flow succeeds even when the webhook retries exceed five attempts",
    "Deploy 2025-03 shipped cleanly after p99 latency dropped to 120ms on the eu-west-1 cluster",
    "Kafka consumer group emails-worker lag stayed near zero during the quiet window on Sunday",
    "Postmortem Redis kept all session tokens intact under maxmemory 16gb for twelve hours",
    "The REST gateway accepted requests over 8MB before the protobuf schema version bump",
    "Carol reverted the feature flag rollout for light mode behind cohort experiment 215",
    "Nightly ETL succeeded after schema drift removed a nullable column from the events table",
    "Incident 91 TLS handshake failures suspected on the production kubernetes cluster",
    "Chose RabbitMQ over NATS JetStream for exactly-once delivery in the analytics pipeline",
];

/// Deterministically drop ~`drop_pct`% of tokens (partial/degraded re-encounter).
fn degrade(content: &str, seed: u64, drop_pct: u64) -> String {
    let toks: Vec<&str> = content.split_whitespace().collect();
    toks.iter()
        .enumerate()
        .filter(|(i, _)| {
            let h = seed
                .wrapping_mul(1099511628211)
                .wrapping_add(*i as u64)
                .wrapping_mul(1099511628211);
            (h % 100) >= drop_pct
        })
        .map(|(_, t)| *t)
        .collect::<Vec<_>>()
        .join(" ")
}

fn roc_auc(scored: &[(f64, bool)]) -> f64 {
    let pos: Vec<f64> = scored.iter().filter(|(_, l)| *l).map(|(s, _)| *s).collect();
    let neg: Vec<f64> = scored.iter().filter(|(_, l)| !*l).map(|(s, _)| *s).collect();
    if pos.is_empty() || neg.is_empty() {
        return f64::NAN;
    }
    let mut wins = 0.0;
    for &p in &pos {
        for &n in &neg {
            if p > n {
                wins += 1.0;
            } else if (p - n).abs() < 1e-12 {
                wins += 0.5;
            }
        }
    }
    wins / (pos.len() * neg.len()) as f64
}

fn engine(minhash_on: bool) -> RecognitionEngine<InMemoryRecognitionStore> {
    let mut cfg = RecognitionConfig::default();
    if !minhash_on {
        cfg.minhash = MinHashConfig { weight: 0.0, ..MinHashConfig::default() };
    }
    let mut e = RecognitionEngine::new(InMemoryRecognitionStore::default(), cfg);
    for (i, c) in CORPUS.iter().enumerate() {
        e.enroll(&format!("mem-{i}"), c).unwrap();
    }
    e
}

/// Score familiar (exact + degraded@72%) vs near-miss novel, return AUC and
/// the verdict breakdown counts.
fn evaluate(minhash_on: bool) -> (f64, usize, usize, usize) {
    let e = engine(minhash_on);
    let mut scored: Vec<(f64, bool)> = Vec::new();
    let (mut exact_reco, mut degraded_fam, mut nearmiss_novel) = (0, 0, 0);

    for c in CORPUS {
        let r = e.recognize(c).unwrap();
        scored.push((r.familiarity, true));
        if matches!(r.verdict, Verdict::Recognized { .. }) {
            exact_reco += 1;
        }
    }
    for (i, c) in CORPUS.iter().enumerate() {
        let probe = degrade(c, i as u64 + 1, 72);
        let r = e.recognize(&probe).unwrap();
        scored.push((r.familiarity, true));
        if !matches!(r.verdict, Verdict::Novel) {
            degraded_fam += 1;
        }
    }
    for c in NEAR_MISS {
        let r = e.recognize(c).unwrap();
        scored.push((r.familiarity, false));
        if matches!(r.verdict, Verdict::Novel) {
            nearmiss_novel += 1;
        }
    }
    (roc_auc(&scored), exact_reco, degraded_fam, nearmiss_novel)
}

fn main() {
    let (auc_off, _, deg_off, nm_off) = evaluate(false);
    let (auc_on, reco_on, deg_on, nm_on) = evaluate(true);

    println!("=== Recognition re-encounter benchmark (MinHash lift) ===");
    println!(
        "corpus: {} enrolled; probes: {} familiar ({} exact + {} degraded@72%) vs {} lexical near-miss negatives",
        CORPUS.len(),
        CORPUS.len() * 2,
        CORPUS.len(),
        CORPUS.len(),
        NEAR_MISS.len()
    );
    println!();
    println!("Familiar-vs-near-miss ROC-AUC:");
    println!("  peak-pair + gram only (MinHash OFF): {auc_off:.3}");
    println!("  + MinHash channel (shipped default): {auc_on:.3}");
    println!("  lift: {:+.3}", auc_on - auc_off);
    println!();
    println!("With MinHash ON:");
    println!("  exact re-encounters Recognized:      {}/{}", reco_on, CORPUS.len());
    println!("  degraded@72% still Familiar+:        {}/{}", deg_on, CORPUS.len());
    println!("  near-miss correctly flagged Novel:   {}/{}", nm_on, NEAR_MISS.len());
    println!("  (MinHash OFF: {}/{} degraded familiar, {}/{} near-miss novel)", deg_off, CORPUS.len(), nm_off, NEAR_MISS.len());
    println!();
    println!("Deterministic, embedding-free, every verdict carries matched-feature evidence.");
    println!(
        "MinHash is a widely-accepted lexical sketch; the differentiated auditable\n\
         verdict/evidence layer sits on top. Semantic paraphrase is NOT claimed here\n\
         (a known weak regime; see docs/internal/RECOGNITION_BASELINE.md)."
    );
}
