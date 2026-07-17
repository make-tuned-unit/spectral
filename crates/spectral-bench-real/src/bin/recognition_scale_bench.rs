//! Recognition at scale — does familiar-vs-novel discrimination hold, and does
//! the FALSE-POSITIVE rate stay low, when hundreds of items with overlapping
//! domain vocabulary are enrolled?
//!
//! The 12-item recognition_bench proves the mechanism; it cannot expose the
//! real scale risk: with a large enrolled set sharing structural vocabulary, a
//! NOVEL probe can accidentally clear the containment threshold against *some*
//! enrolled item (a chance bigram overlap). This enrolls ~300 templated
//! incidents and measures AUC + false-positive rate against two hard negative
//! classes (near-miss = same template, different specifics; unrelated =
//! different domain), sweeping `min_similarity` and `shingle` to locate the
//! precision/recall knee at scale. Deterministic ($0, no LLM).
//!
//! Run: `cargo run -p spectral-bench-real --bin recognition_scale_bench`

use spectral_recognition::{
    InMemoryRecognitionStore, MinHashConfig, RecognitionConfig, RecognitionEngine, Verdict,
};

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

const SERVICES: &[&str] = &[
    "task-runner",
    "web-frontend",
    "billing-api",
    "orders-worker",
    "edge-proxy",
    "auth-svc",
    "search-idx",
    "cache-node",
    "etl-runner",
    "api-gateway",
    "payment-svc",
    "notif-worker",
];
const EVENTS: &[&str] = &[
    "OOMKilled at 512Mi",
    "CrashLooped repeatedly",
    "timed out after 30s",
    "returned 502 errors",
    "certificate expired",
    "dropped connections",
    "consumer lag spiked",
    "rejected oversized requests",
    "rolled back the deploy",
    "hit a schema drift",
];
const CONTEXTS: &[&str] = &[
    "during the nightly reindex job",
    "on the us-east-1 cluster",
    "after the protobuf schema bump",
    "in the friday flash sale window",
    "under maxmemory pressure",
    "while draining the node pool",
    "during the blue-green rollout",
    "after the dns cache flush",
];

/// One templated incident. `salt` varies the numeric specifics so items are
/// distinct even when they share a template.
fn incident(rng: &mut Lcg, salt: usize) -> String {
    format!(
        "Incident {salt}: {}-{} {} {}",
        rng.pick(SERVICES),
        1000 + salt,
        rng.pick(EVENTS),
        rng.pick(CONTEXTS)
    )
}

const UNRELATED: &[&str] = &[
    "The lakeside cabin had a wood-fired sauna right by the dock",
    "She restrung the antique violin the night before the recital",
    "We portaged the kayak around the second waterfall on the river",
    "The community garden finally started a shared compost program",
    "A local artist painted a huge mural on the old depot wall",
    "The bakery sold out of sourdough before nine in the morning",
    "He planted three rows of heirloom tomatoes along the fence",
    "The trail switchbacked steeply up to the alpine meadow",
    "Grandma's recipe called for browned butter and toasted pecans",
    "The tide pools were full of anemones and hermit crabs at dawn",
];

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
    let neg: Vec<f64> = scored
        .iter()
        .filter(|(_, l)| !*l)
        .map(|(s, _)| *s)
        .collect();
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

struct Corpus {
    enrolled: Vec<String>,
    near_miss: Vec<String>,
}

fn build_corpus() -> Corpus {
    let mut rng = Lcg(0xC0FF_EE12_3456_7890);
    let enrolled: Vec<String> = (0..300).map(|i| incident(&mut rng, i)).collect();
    // Near-miss: the HONEST hard negative for a LEXICAL recognizer — same domain
    // and shared individual words (pod, memory, cluster, latency…) but DIFFERENT
    // phrasing, so verbatim multi-token overlap with any enrolled item is low.
    // A lexical near-dup detector should flag these Novel; if it doesn't, the
    // containment threshold is too loose. (Reusing enrolled event+context spans
    // verbatim — the earlier design — is a textual near-duplicate, which a
    // lexical recognizer correctly calls familiar; that is not a fair negative.)
    let nm_templates: &[&str] = &[
        "The {s} pod was terminated for exceeding its configured memory limit overnight",
        "A {s} node grew unhealthy and the operator cordoned it before the peak",
        "Users saw elevated tail latency talking to {s} for about ten minutes",
        "The on-call engineer restarted {s} after a burst of failed health checks",
        "Traffic to {s} was shed by the load balancer while a replica recovered",
        "A config push to {s} was reverted once error budgets started burning",
        "The {s} queue backed up until an extra worker was scaled in by hand",
        "Someone rotated the credentials for {s} after the quarterly audit finding",
    ];
    let mut rng2 = Lcg(0x00BA_D5EE_D999_9000);
    let near_miss: Vec<String> = (0..60)
        .map(|_| rng2.pick(nm_templates).replace("{s}", rng2.pick(SERVICES)))
        .collect();
    Corpus {
        enrolled,
        near_miss,
    }
}

fn build_engine(
    corpus: &Corpus,
    shingle: usize,
    min_similarity: f64,
) -> RecognitionEngine<InMemoryRecognitionStore> {
    let cfg = RecognitionConfig {
        minhash: MinHashConfig {
            shingle,
            min_similarity,
            ..MinHashConfig::default()
        },
        ..RecognitionConfig::default()
    };
    let mut e = RecognitionEngine::new(InMemoryRecognitionStore::default(), cfg);
    for (i, c) in corpus.enrolled.iter().enumerate() {
        e.enroll(&format!("mem-{i}"), c).unwrap();
    }
    e
}

/// Returns (auc, exact_recognized, degraded72_familiar, nearmiss_novel, unrelated_novel).
fn evaluate(corpus: &Corpus, shingle: usize, min_similarity: f64) -> (f64, f64, f64, f64, f64) {
    let e = build_engine(corpus, shingle, min_similarity);
    let mut scored: Vec<(f64, bool)> = Vec::new();

    // Positives: exact + degraded@72%, sampled every 5th enrolled item.
    let sample: Vec<&String> = corpus.enrolled.iter().step_by(5).collect();
    let mut exact_reco = 0usize;
    for c in &sample {
        let r = e.recognize(c).unwrap();
        scored.push((r.familiarity, true));
        if matches!(r.verdict, Verdict::Recognized { .. }) {
            exact_reco += 1;
        }
    }
    let mut deg_fam = 0usize;
    for (i, c) in sample.iter().enumerate() {
        let r = e.recognize(&degrade(c, i as u64 + 1, 72)).unwrap();
        scored.push((r.familiarity, true));
        if !matches!(r.verdict, Verdict::Novel) {
            deg_fam += 1;
        }
    }
    // Negatives: near-miss (hard) + unrelated (easy). FP = not flagged Novel.
    let mut nm_novel = 0usize;
    for c in &corpus.near_miss {
        let r = e.recognize(c).unwrap();
        scored.push((r.familiarity, false));
        if matches!(r.verdict, Verdict::Novel) {
            nm_novel += 1;
        }
    }
    let mut un_novel = 0usize;
    for c in UNRELATED {
        let r = e.recognize(c).unwrap();
        scored.push((r.familiarity, false));
        if matches!(r.verdict, Verdict::Novel) {
            un_novel += 1;
        }
    }
    (
        roc_auc(&scored),
        exact_reco as f64 / sample.len() as f64,
        deg_fam as f64 / sample.len() as f64,
        nm_novel as f64 / corpus.near_miss.len() as f64,
        un_novel as f64 / UNRELATED.len() as f64,
    )
}

fn main() {
    let corpus = build_corpus();
    println!("=== Recognition at scale (300 enrolled, overlapping domain vocab) ===");
    println!(
        "probes: 60 exact + 60 degraded@72% positives; 60 near-miss + 10 unrelated negatives\n"
    );

    println!(
        "{:<24}{:>7}{:>10}{:>10}{:>11}{:>11}",
        "config", "AUC", "exact", "deg@72", "nm-novel", "un-novel"
    );
    let configs = [
        ("shingle=2 sim=0.15 (def)", 2usize, 0.15f64),
        ("shingle=2 sim=0.25", 2, 0.25),
        ("shingle=2 sim=0.35", 2, 0.35),
        ("shingle=3 sim=0.15", 3, 0.15),
        ("shingle=3 sim=0.25", 3, 0.25),
        ("shingle=3 sim=0.35", 3, 0.35),
    ];
    for (label, sh, sim) in configs {
        let (auc, ex, deg, nm, un) = evaluate(&corpus, sh, sim);
        println!("{label:<24}{auc:>7.3}{ex:>10.2}{deg:>10.2}{nm:>11.2}{un:>11.2}");
    }
    println!("\nexact/deg = positive recall (higher better); nm-novel/un-novel = negative");
    println!("precision, i.e. 1 - false-positive-rate (higher better). Deterministic, $0.");
}
