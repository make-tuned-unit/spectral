//! Opt-in per-stage timing for the write path (Track C).
//!
//! `ingest-gap-decomposition-2026-08-03.md` attributed **73% of Spectral's
//! per-event ingest cost (0.233 ms/event)** to "classification, signal scoring,
//! episode/session handling, content hashing" and called it a black box.
//! Reading `remember_with` suggests that attribution is largely wrong: after the
//! single `ingest_with` call that contains classify/score/hash, the method
//! performs a session-association write, a separate declarative-density UPDATE,
//! a read-back of the row just written, an Ed25519 signature, and a signature
//! write — **four extra round trips and one asymmetric-crypto operation.**
//!
//! This measures which it is, instead of arguing about it.
//!
//! **Off unless `SPECTRAL_INGEST_PROFILE` is set.** When off, the only cost is
//! one relaxed atomic load per stage, so the shipped write path is unchanged.
//! Timings accumulate per-thread and are printed by [`report`].

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

static ENABLED: OnceLock<AtomicBool> = OnceLock::new();

fn enabled() -> bool {
    ENABLED
        .get_or_init(|| AtomicBool::new(std::env::var("SPECTRAL_INGEST_PROFILE").is_ok()))
        .load(Ordering::Relaxed)
}

thread_local! {
    /// (stage, total_nanos, calls), kept as a Vec because there are <10 stages
    /// and a linear scan beats hashing at this size.
    static STAGES: RefCell<Vec<(&'static str, u128, u64)>> = const { RefCell::new(Vec::new()) };
}

/// Time `f` under `stage`. A no-op wrapper when profiling is off.
pub fn time<T>(stage: &'static str, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let t0 = Instant::now();
    let out = f();
    let dt = t0.elapsed().as_nanos();
    STAGES.with(|s| {
        let mut s = s.borrow_mut();
        match s.iter_mut().find(|(name, _, _)| *name == stage) {
            Some(entry) => {
                entry.1 += dt;
                entry.2 += 1;
            }
            None => s.push((stage, dt, 1)),
        }
    });
    out
}

/// Per-stage totals for this thread: (stage, total_nanos, calls).
pub fn snapshot() -> Vec<(&'static str, u128, u64)> {
    STAGES.with(|s| s.borrow().clone())
}

/// Human-readable per-stage report in ms/event, sorted by cost.
///
/// Reports the share of measured time only — it deliberately does NOT claim to
/// account for the full 0.233 ms, because unmeasured code between stages is
/// exactly the sort of gap that made the original decomposition misleading.
pub fn report() -> String {
    let mut rows = snapshot();
    if rows.is_empty() {
        return "ingest profile: no samples (is SPECTRAL_INGEST_PROFILE set?)".into();
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    let total: u128 = rows.iter().map(|r| r.1).sum();
    let mut out = String::from("ingest per-stage profile (measured stages only):\n");
    for (stage, nanos, calls) in &rows {
        let ms_per_call = *nanos as f64 / 1e6 / (*calls).max(1) as f64;
        let share = *nanos as f64 / total as f64 * 100.0;
        out.push_str(&format!(
            "  {stage:<22} {ms_per_call:>8.4} ms/event  {share:>5.1}%  ({calls} calls)\n"
        ));
    }
    out.push_str(&format!(
        "  {:<22} {:>8.4} ms/event  (sum of measured stages)\n",
        "TOTAL",
        total as f64 / 1e6 / rows.iter().map(|r| r.2).max().unwrap_or(1) as f64
    ));
    out
}
