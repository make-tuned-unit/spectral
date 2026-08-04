//! Micro-benchmark for `QuestionShape::classify` — the compile-once regex fix.
//!
//! Run: `cargo run -p spectral --release --example policy_classify_bench`
//!
//! Measures the shipped (cached) classifier against an inline replica of the
//! pre-fix implementation that compiles its patterns per call. Both arms run in
//! the same binary over the same question mix, so the comparison isolates regex
//! compilation from everything else.
//!
//! Method rules from `docs/internal/ingest-cost-profile-2026-07-31.md`: warm
//! only, discard a warm-up pass, report more than one run.

use std::time::Instant;

use regex::Regex;
use spectral::policy::QuestionShape;

/// Representative mix: one question per classifier arm, so the measurement
/// covers both early returns (cheap) and full fall-through to `General`
/// (worst case — every pattern compiled).
const QUESTIONS: &[&str] = &[
    "How many days ago did I book the flight?",
    "How many times did I go running last month?",
    "How many pets do I currently have?",
    "Where did I park last week?",
    "Where do I currently live?",
    "When did I start the new job?",
    "What is my sister's name?",
    "What did I most recently order?",
    "Can you recommend a good restaurant?",
    "Any tips for staying focused?",
    "Remind me what we discussed about the budget",
    "Tell me about my hiking trip",
];

/// Verbatim replica of the pre-fix classifier: `Regex::new` on every call.
/// Kept byte-identical to the patterns in `policy.rs` so the arms differ only
/// in *when* compilation happens.
fn classify_uncached(question: &str) -> u8 {
    let q = question.to_lowercase();

    if Regex::new(r"how many (?:days|weeks|months|years) (?:ago|since|passed|before|after|between|had passed|have passed|did it take)|how old")
        .unwrap()
        .is_match(&q)
    {
        return 0;
    }
    if Regex::new(r"how many|how much|total|in total|altogether")
        .unwrap()
        .is_match(&q)
    {
        if Regex::new(r"\b(currently|right now|most recent|latest|newest|do i still|now)\b")
            .unwrap()
            .is_match(&q)
        {
            return 1;
        }
        return 2;
    }
    if Regex::new(r"^where\b").unwrap().is_match(&q) {
        if Regex::new(r"\b(currently|right now|most recent|latest|newest|do i still|now|recent)\b")
            .unwrap()
            .is_match(&q)
        {
            return 3;
        }
        return 4;
    }
    if Regex::new(r"when did|how long|(?:^|\W)first\b|(?:^|\W)last\b|before|after|ago|since|order.+(?:earliest|latest)|from earliest|chronological|(?:^|\W)order of\b")
        .unwrap()
        .is_match(&q)
    {
        return 0;
    }
    if Regex::new(r"^(?:what|where|who|which)\b")
        .unwrap()
        .is_match(&q)
    {
        if Regex::new(
            r"\b(currently|right now|most recent|most recently|latest|newest|do i still|now)\b",
        )
        .unwrap()
        .is_match(&q)
        {
            return 3;
        }
        return 4;
    }
    if Regex::new(r"\b(suggest|recommend|tips?|advice|recommendations?|what should i)\b")
        .unwrap()
        .is_match(&q)
    {
        return 5;
    }
    if Regex::new(r"\bany (tips?|advice|suggestions?|ideas?|thoughts?|recommendations?)\b")
        .unwrap()
        .is_match(&q)
    {
        return 5;
    }
    if Regex::new(r"\b(remind me|going back to|previous|earlier conversation|we (discussed|talked about)|can you remind me)\b")
        .unwrap()
        .is_match(&q)
    {
        return 6;
    }
    7
}

fn time_cached(reps: usize) -> f64 {
    let start = Instant::now();
    let mut sink = 0usize;
    for _ in 0..reps {
        for q in QUESTIONS {
            sink += QuestionShape::classify(q) as usize;
        }
    }
    std::hint::black_box(sink);
    start.elapsed().as_secs_f64() / (reps * QUESTIONS.len()) as f64
}

fn time_uncached(reps: usize) -> f64 {
    let start = Instant::now();
    let mut sink = 0usize;
    for _ in 0..reps {
        for q in QUESTIONS {
            sink += classify_uncached(q) as usize;
        }
    }
    std::hint::black_box(sink);
    start.elapsed().as_secs_f64() / (reps * QUESTIONS.len()) as f64
}

fn main() {
    // Warm-up pass, discarded: primes the OnceLock cells and the allocator.
    time_cached(200);
    time_uncached(20);

    println!(
        "QuestionShape::classify — per-call cost, warm, {} questions/rep\n",
        QUESTIONS.len()
    );
    println!(
        "{:<12} {:>14} {:>14} {:>10}",
        "run", "uncached (us)", "cached (us)", "speedup"
    );

    for run in 1..=3 {
        let uncached = time_uncached(500) * 1e6;
        let cached = time_cached(20_000) * 1e6;
        println!(
            "{:<12} {:>14.3} {:>14.3} {:>9.1}x",
            format!("run {run}"),
            uncached,
            cached,
            uncached / cached
        );
    }

    println!(
        "\nEvery routed question pays this once. The `General` fall-through \n\
         (no pattern matches) is the worst case: all 11 patterns compiled."
    );
}
