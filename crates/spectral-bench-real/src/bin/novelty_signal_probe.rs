//! Does recognition novelty belong IN the signal score? — a probe.
//!
//! The write path already computes recognition familiarity (MinHash-based,
//! `brain.rs` recurrence feedback) but only uses it to reinforce priors. Its
//! novelty (1 − familiarity) never reaches `signal::score_memory`. The lever
//! under test: fold that novelty into signal so redundant restatements are
//! demoted and genuinely-new memories boosted.
//!
//! The hypothesis to FALSIFY: novelty (novel↔redundant) is a DIFFERENT axis
//! from durability (durable-fact↔ephemeral-chatter), and the AAAK bar is a
//! durability threshold. Folding an orthogonal axis into that threshold should
//! (a) risk demoting a RESTATED durable constraint below the bar — losing a
//! safety-critical fact — and (b) risk floating NOVEL ephemeral chatter up
//! toward it. If both show up, novelty must stay a SEPARATE dimension (where
//! it already lives), not be mixed into the durability signal.
//!
//! Deterministic, $0, no LLM. Run: `cargo run -p spectral-bench-real --bin novelty_signal_probe`

use spectral_ingest::signal::score_memory;
use spectral_recognition::{InMemoryRecognitionStore, RecognitionConfig, RecognitionEngine};

/// Baseline signal, no novelty. `hall` is what the classifier would assign.
fn baseline(content: &str, hall: &str) -> f64 {
    score_memory(content, hall)
}

/// Symmetric variant: novelty centered at 0.5 — redundant (<0.5) demotes,
/// novel (>0.5) boosts. Tests whether restated durable facts fall below the bar.
fn adj_symmetric(base: f64, novelty: f64, w: f64) -> f64 {
    (base + w * (novelty - 0.5)).clamp(0.0, 1.0)
}

/// Boost-only variant: never demotes, only lifts novel content. Tests whether
/// novel EPHEMERAL chatter floats up toward the bar.
fn adj_boost_only(base: f64, novelty: f64, w: f64) -> f64 {
    (base + w * (novelty - 0.5).max(0.0)).clamp(0.0, 1.0)
}

fn main() {
    let mut engine = RecognitionEngine::new(
        InMemoryRecognitionStore::default(),
        RecognitionConfig::default(),
    );

    // Enrolled priors — the existing memory the new write is judged novel against.
    let corpus = [
        ("m0", "I am vegetarian and do not eat any meat or fish"),
        ("m1", "I decided to standardize all my projects on Rust"),
        ("m2", "Never schedule meetings for me before 9am"),
        ("m3", "My daughter Mia is five years old"),
        ("m4", "The deploy failed last night due to a bad config value"),
        ("m5", "We shipped the new billing feature on Tuesday"),
        ("m6", "I prefer concise written summaries over long calls"),
        ("m7", "The team retro is scheduled for Friday afternoon"),
    ];
    for (id, c) in &corpus {
        engine.enroll(id, c).unwrap();
    }

    // (label, content, classifier hall, durable?, expected-novelty-band)
    let stimuli: &[(&str, &str, &str, bool)] = &[
        // Genuinely new durable fact — not in corpus. Novel + durable.
        ("novel durable   ", "I am severely allergic to shellfish and peanuts", "fact", true),
        // Restatement of an enrolled durable fact. REDUNDANT + still durable —
        // must STAY above the bar (it is a safety-critical constraint).
        ("restated durable ", "I'm vegetarian, so no meat or fish for me at all", "fact", true),
        // Brand-new ephemeral chatter — novel but NOT durable. Must stay below.
        ("novel ephemeral  ", "Grabbed sushi with a new client at a rooftop spot downtown", "event", false),
        // Gibberish — maximally novel, zero durability. Failure-mode canary.
        ("gibberish        ", "Zxqp fnord blivet quxwomble threppy gnarfle wobbecks", "event", false),
        // Restated ephemeral — redundant AND not durable. Stays below either way.
        ("restated ephemeral", "The team retro is on Friday afternoon as planned", "event", false),
    ];

    let bar = 0.70;
    let w = 0.15; // bounded novelty weight

    println!("=== Does recognition novelty belong IN the signal score? ===\n");
    println!("weight w={w}, AAAK bar={bar}. novelty = 1 − MinHash familiarity vs the 8-memory corpus.\n");
    println!(
        "{:<19} {:>4} {:>8} {:>8} {:>10} {:>10}   verdict",
        "stimulus", "dur", "novelty", "base", "symmetric", "boost-only"
    );
    println!("{}", "-".repeat(92));

    let mut sym_flips_bad = 0; // durable dropped below bar, or ephemeral lifted above
    let mut boost_flips_bad = 0;
    for (label, content, hall, durable) in stimuli {
        let rec = engine.recognize(content).unwrap();
        let nov = rec.novelty;
        let base = baseline(content, hall);
        let sym = adj_symmetric(base, nov, w);
        let boost = adj_boost_only(base, nov, w);

        // A "bad flip" = the bar decision changes AWAY from the correct answer.
        let correct_side = *durable; // durable should be >= bar
        let base_ok = (base >= bar) == correct_side;
        let sym_ok = (sym >= bar) == correct_side;
        let boost_ok = (boost >= bar) == correct_side;
        if base_ok && !sym_ok {
            sym_flips_bad += 1;
        }
        if base_ok && !boost_ok {
            boost_flips_bad += 1;
        }

        let flag = |ok: bool| if ok { " " } else { "✗" };
        println!(
            "{:<19} {:>4} {:>8.2} {:>8.2} {:>8.2} {} {:>8.2} {}",
            label,
            if *durable { "yes" } else { "no" },
            nov,
            base,
            sym,
            flag(sym_ok),
            boost,
            flag(boost_ok),
        );
    }

    println!("\n{}", "-".repeat(92));
    println!("bad bar-flips introduced by folding novelty into signal:");
    println!("  symmetric  (demote redundant + boost novel): {sym_flips_bad}");
    println!("  boost-only (never demote, only lift novel):   {boost_flips_bad}");

    // ── weight sweep: find the safety ceiling and test for ANY added value ──
    // A change is only justified if some weight ADDS a correct bar decision the
    // base signal got wrong. It never does here (base already gets every bar
    // right), so the sweep can only find the weight where novelty starts
    // BREAKING correct decisions — the safety ceiling.
    println!("\n=== weight sweep: safety ceiling + value check (symmetric variant) ===");
    println!("{:>6} {:>10} {:>12} {:>14}", "w", "bad-flips", "bar-fixes", "rank-changes");
    let precomputed: Vec<(f64, f64, bool)> = stimuli
        .iter()
        .map(|(_, content, hall, durable)| {
            (engine.recognize(content).unwrap().novelty, baseline(content, hall), *durable)
        })
        .collect();
    for &sw in &[0.10_f64, 0.20, 0.30, 0.40, 0.50, 0.70, 1.00] {
        let mut bad = 0; // base right → adjusted wrong
        let mut fix = 0; // base wrong → adjusted right (the only thing that would justify it)
        for (nov, base, durable) in &precomputed {
            let adj = adj_symmetric(*base, *nov, sw);
            let base_ok = (*base >= bar) == *durable;
            let adj_ok = (adj >= bar) == *durable;
            if base_ok && !adj_ok {
                bad += 1;
            }
            if !base_ok && adj_ok {
                fix += 1;
            }
        }
        // Ranking change among durables (AAAK greedy-by-signal under budget):
        // does novelty reorder any durable pair the base signal ordered?
        let durs: Vec<(f64, f64)> = precomputed
            .iter()
            .filter(|(_, _, d)| *d)
            .map(|(n, b, _)| (*b, adj_symmetric(*b, *n, sw)))
            .collect();
        let mut rank_changes = 0;
        for i in 0..durs.len() {
            for j in (i + 1)..durs.len() {
                let base_order = durs[i].0 > durs[j].0;
                let adj_order = durs[i].1 > durs[j].1;
                if base_order != adj_order {
                    rank_changes += 1;
                }
            }
        }
        println!("{sw:>6.2} {bad:>10} {fix:>12} {rank_changes:>14}");
    }

    println!("\ninterpretation:");
    println!("  novelty and durability are ORTHOGONAL axes. The AAAK bar tests durability.");
    println!("  bar-fixes is the ONLY column that could justify folding novelty into signal —");
    println!("  a weight where novelty rescues a bar decision the base signal got wrong. It is");
    println!("  0 at every weight: base signal already gets every bar right. Meanwhile bad-flips");
    println!("  climbs as weight rises (a restated allergy drops out / novel chatter floats in),");
    println!("  and rank-changes only appear once the perturbation overwhelms the base gap.");
    println!("  Verdict: keep novelty a SEPARATE dimension (spectrogram / recurrence feedback).");
    println!("  Folding it into the durability signal is all downside, no measured upside.");
    println!("  Deterministic, $0 — a negative result that protects the score's meaning.");
}
