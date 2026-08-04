//! $0 latency gate for the turn contract.
//!
//! The turn-contract preregistration (`docs/internal/turn-contract-prereg-2026-07-30.md`,
//! systems kill line) states:
//!
//! > Recall-only p95 may regress by at most 5%; combined recall+recognition p95
//! > must not exceed today's two sequential calls. Otherwise keep the APIs typed
//! > but do **not** fuse execution.
//!
//! This binary measures exactly that. Three arms:
//!
//! * **legacy recall** — `recall_cascade_scoped`, which auto-reinforces and logs
//!   a retrieval event inline (the historical write-back path).
//! * **turn (uncommitted)** — `Brain::turn`, read-only retrieval plus a ledger
//!   delivery insert. This is the honest recall-only comparison: a turn that is
//!   never committed still writes its delivery row.
//! * **turn + outcome commit** — the full cycle, including the transactional
//!   outcome write. Reported separately because the commit happens *after* the
//!   actor responds and is therefore off the response-critical path.
//!
//! Run WARM and more than once — see `ingest_profile.rs` for why. Ratios travel
//! between machines; absolute numbers do not.
//!
//! `cargo run -p spectral --release --example turn_latency [ITERS]`

use std::time::{Duration, Instant};

use spectral::{
    Brain, MemoryOutcome, RecognitionContext, TurnPolicyVersion, TurnRequest, Visibility,
};
use spectral_graph::cascade_layers::CascadePipelineConfig;
use tempfile::TempDir;

const WARMUP: usize = 20;
const CORPUS: usize = 400;

fn content_for(i: usize) -> String {
    const S: [&str; 6] = [
        "the deploy pipeline",
        "the billing reconciliation job",
        "the incident retrospective",
        "the capacity forecast",
        "the schema migration",
        "the access audit",
    ];
    const V: [&str; 5] = [
        "was updated by",
        "was blocked on",
        "was rolled back after",
        "was escalated to",
        "was deferred pending",
    ];
    format!(
        "{} {} the platform team (record {i}, ref {:04x})",
        S[i % S.len()],
        V[(i / 6) % V.len()],
        i.wrapping_mul(2654435761) & 0xffff,
    )
}

fn query_for(i: usize) -> String {
    format!("deploy pipeline rollback platform team {}", i % 17)
}

/// p50/p95 in milliseconds from a sample of durations.
fn percentiles(mut d: Vec<Duration>) -> (f64, f64) {
    d.sort();
    let ms = |x: Duration| x.as_secs_f64() * 1000.0;
    let idx = |p: f64| ((d.len() as f64 * p) as usize).min(d.len() - 1);
    (ms(d[idx(0.50)]), ms(d[idx(0.95)]))
}

fn main() {
    let iters: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(300);

    if cfg!(debug_assertions) {
        eprintln!("WARNING: debug build — numbers are meaningless. Use --release.\n");
    }

    let tmp = TempDir::new().unwrap();
    let brain = Brain::open(tmp.path()).unwrap();
    for i in 0..CORPUS {
        brain
            .remember(&format!("m-{i}"), &content_for(i), Visibility::Private)
            .unwrap();
    }

    let cfg = CascadePipelineConfig::default();

    // ── Arm 1: legacy recall (inline write-back) ───────────────────────
    for i in 0..WARMUP {
        let _ = brain.recall_cascade_scoped(
            &query_for(i),
            &RecognitionContext::empty(),
            &cfg,
            Visibility::Private,
        );
    }
    let mut legacy = Vec::with_capacity(iters);
    for i in 0..iters {
        let q = query_for(i);
        let t = Instant::now();
        brain
            .recall_cascade_scoped(&q, &RecognitionContext::empty(), &cfg, Visibility::Private)
            .unwrap();
        legacy.push(t.elapsed());
    }

    // ── Arm 2: turn, never committed (recall-only comparison) ──────────
    for i in 0..WARMUP {
        let _ = brain.turn(&TurnRequest::query(&query_for(i), Visibility::Private));
    }
    let mut turn_only = Vec::with_capacity(iters);
    for i in 0..iters {
        let q = query_for(i);
        let req = TurnRequest::query(&q, Visibility::Private);
        let t = Instant::now();
        brain.turn(&req).unwrap();
        turn_only.push(t.elapsed());
    }

    // ── Arm 2b: turn under V2Shaped (the published retrieval policy) ───
    //
    // V1 ignores the query entirely, so arm 2 never runs question-shape
    // classification. V2Shaped classifies twice per turn (once for the cascade
    // profile, once for the route), which is the configuration a consumer runs
    // if they want the policy behind the published accuracy number. Measured
    // separately so the classifier's cost is attributed to the arm that pays it.
    fn v2_request<'q>(q: &'q str) -> TurnRequest<'q> {
        let mut r = TurnRequest::query(q, Visibility::Private);
        r.policy = TurnPolicyVersion::V2Shaped;
        r
    }
    for i in 0..WARMUP {
        let q = query_for(i);
        let _ = brain.turn(&v2_request(&q));
    }
    let mut turn_v2 = Vec::with_capacity(iters);
    for i in 0..iters {
        let q = query_for(i);
        let req = v2_request(&q);
        let t = Instant::now();
        brain.turn(&req).unwrap();
        turn_v2.push(t.elapsed());
    }

    // ── Arm 2c: turn with deferred delivery write (R8 candidate) ───────
    //
    // The failed gate's diagnosis: the whole regression is the synchronous
    // delivery-write commit. This arm measures the preregistered fix —
    // `set_async_turn_delivery(true)` spawns that write off the read path
    // with per-occurrence ordering (see deferred-delivery-prereg-2026-08-04).
    // A separate Brain over the same corpus dir so the mode flag cannot leak
    // into the other arms.
    let mut deferred_brain = Brain::open(tmp.path()).unwrap();
    deferred_brain.set_async_turn_delivery(true);
    for i in 0..WARMUP {
        let _ = deferred_brain.turn(&TurnRequest::query(&query_for(i), Visibility::Private));
    }
    let mut turn_deferred = Vec::with_capacity(iters);
    for i in 0..iters {
        let q = query_for(i);
        let req = TurnRequest::query(&q, Visibility::Private);
        let t = Instant::now();
        deferred_brain.turn(&req).unwrap();
        turn_deferred.push(t.elapsed());
    }
    deferred_brain.flush_turn_deliveries().unwrap();
    drop(deferred_brain);

    // ── Arm 3: turn + outcome commit (full cycle) ──────────────────────
    let mut turn_full = Vec::with_capacity(iters);
    for i in 0..iters {
        let q = query_for(i);
        let req = TurnRequest::query(&q, Visibility::Private);
        let t = Instant::now();
        let r = brain.turn(&req).unwrap();
        if let Some(hit) = r.hits.first() {
            let key = hit.key.clone();
            brain
                .record_turn_outcome(&r.receipt, &[(key.as_str(), MemoryOutcome::Used)])
                .unwrap();
        }
        turn_full.push(t.elapsed());
    }

    let (lp50, lp95) = percentiles(legacy);
    let (tp50, tp95) = percentiles(turn_only);
    let (v2p50, v2p95) = percentiles(turn_v2);
    let (dp50, dp95) = percentiles(turn_deferred);
    let (fp50, fp95) = percentiles(turn_full);

    println!("turn latency — corpus={CORPUS}, iters={iters}\n");
    println!("{:<34} {:>9} {:>9}", "arm", "p50 (ms)", "p95 (ms)");
    println!("{:-<54}", "");
    println!(
        "{:<34} {:>9.3} {:>9.3}",
        "legacy recall_cascade_scoped", lp50, lp95
    );
    println!(
        "{:<34} {:>9.3} {:>9.3}",
        "turn (uncommitted, V1)", tp50, tp95
    );
    println!(
        "{:<34} {:>9.3} {:>9.3}",
        "turn (uncommitted, V2Shaped)", v2p50, v2p95
    );
    println!(
        "{:<34} {:>9.3} {:>9.3}",
        "turn (uncommitted, deferred write)", dp50, dp95
    );
    println!(
        "{:<34} {:>9.3} {:>9.3}",
        "turn + outcome commit", fp50, fp95
    );

    let delta = (tp95 - lp95) / lp95 * 100.0;
    println!("\nrecall-only p95 delta (sync): {delta:+.1}%  (kill line: +5.0%)");
    println!(
        "VERDICT (sync): {}",
        if delta <= 5.0 {
            "PASS — turn may be recommended as the default recall path"
        } else {
            "FAIL — keep the APIs typed but do NOT fuse execution"
        }
    );

    let ddelta = (dp95 - lp95) / lp95 * 100.0;
    println!("\nrecall-only p95 delta (deferred): {ddelta:+.1}%  (kill line: +5.0%)");
    println!(
        "VERDICT (deferred): {}",
        if ddelta <= 5.0 {
            "PASS — deferred mode meets the gate; see deferred-delivery-prereg-2026-08-04"
        } else {
            "FAIL — deferred mode misses the gate; turn stays non-default"
        }
    );
    println!(
        "\nfull-cycle p95 vs legacy: {:+.1}% (commit runs AFTER the actor responds,\n\
         so it is off the response-critical path — reported for completeness)",
        (fp95 - lp95) / lp95 * 100.0
    );
}
