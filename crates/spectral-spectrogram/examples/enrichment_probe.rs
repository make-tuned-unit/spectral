//! R35 — does Librarian enrichment change the spectrogram's dimensions?
//!
//! Prereg: `docs/internal/r35-spectrogram-enrichment-prereg-2026-08-17.md`.
//!
//! `SpectrogramAnalyzer::analyze` reads `memory.content` and nothing else;
//! `memory.description` — the Librarian's gloss — is never touched anywhere in
//! this crate. The ORACLE_TIER0 null that retired spectrogram-as-recall was
//! therefore measured on content-only fingerprints, against a corpus that is
//! now 96.6% enriched. This probe asks whether feeding the enrichment in would
//! move the dimensions at all, before anything is spent on a retrieval arm.
//!
//! Two arms over the same memories, same analyzer, same config:
//!   A (status quo): content
//!   B (enriched):   content + "\n" + description
//!
//! Run:
//! ```text
//! cargo run -p spectral-spectrogram --example enrichment_probe -- ~/.permagent/brain/memory.db
//! ```
//!
//! **Privacy: this prints statistics only.** No content, description, key or
//! id is ever printed or written. The store is opened read-only.

use spectral_ingest::Memory;
use spectral_spectrogram::{
    AnalysisContext, AnalyzerConfig, SpectralFingerprint, SpectrogramAnalyzer,
};
use std::collections::HashMap;

/// The six continuous dimensions, in a fixed order. `action_type` is
/// categorical and is compared separately.
const DIMS: [&str; 6] = [
    "entity_density",
    "decision_polarity",
    "causal_depth",
    "emotional_valence",
    "temporal_specificity",
    "novelty",
];

fn vector(f: &SpectralFingerprint) -> [f64; 6] {
    [
        f.entity_density,
        f.decision_polarity,
        f.causal_depth,
        f.emotional_valence,
        f.temporal_specificity,
        f.novelty,
    ]
}

fn euclid(a: &[f64; 6], b: &[f64; 6]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f64>()
        .sqrt()
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn variance(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let m = mean(xs);
    xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (xs.len() - 1) as f64
}

/// Mean pairwise distance over a bounded sample of pairs, so the cost stays
/// linear-ish rather than O(n^2) on a few thousand memories. Deterministic
/// stride, no RNG.
fn mean_pairwise_distance(vs: &[[f64; 6]]) -> f64 {
    let n = vs.len();
    if n < 2 {
        return 0.0;
    }
    let mut total = 0.0;
    let mut count = 0usize;
    // Compare each item against a fixed set of strides ahead of it. Covers the
    // whole set evenly without materialising n^2 pairs.
    for stride in [1usize, 7, 53, 211, 1009] {
        if stride >= n {
            continue;
        }
        for i in 0..n {
            let j = (i + stride) % n;
            total += euclid(&vs[i], &vs[j]);
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

fn blank_memory(id: &str, content: String, wing: Option<String>) -> Memory {
    Memory {
        id: id.to_string(),
        key: String::new(),
        content,
        wing,
        hall: None,
        signal_score: 1.0,
        visibility: "private".into(),
        source: None,
        device_id: None,
        confidence: 1.0,
        created_at: None,
        last_reinforced_at: None,
        episode_id: None,
        compaction_tier: None,
        declarative_density: None,
        // Deliberately left None in BOTH arms: the point of the probe is what
        // the analyzer derives from text, so the description is fed in via
        // `content` for arm B rather than through this field, which the
        // analyzer does not read at all.
        description: None,
        description_generated_at: None,
        content_hash: None,
        source_brain_id: None,
        signature: None,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let db = args
        .get(1)
        .ok_or("usage: enrichment_probe <memory.db>")?
        .clone();

    // Read-only: this probe must not mutate the brain it measures.
    let conn = rusqlite::Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;

    let mut stmt = conn.prepare(
        "SELECT id, content, description, wing FROM memories
         WHERE content IS NOT NULL AND TRIM(content) <> ''",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;

    let mut all: Vec<(String, String, Option<String>, Option<String>)> = Vec::new();
    for row in rows {
        all.push(row?);
    }
    let total = all.len();
    let enriched: Vec<_> = all
        .iter()
        .filter(|(_, _, d, _)| d.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false))
        .cloned()
        .collect();

    println!("R35 enrichment probe");
    println!("  memories with content : {total}");
    println!(
        "  of which enriched     : {} ({:.1}%)",
        enriched.len(),
        100.0 * enriched.len() as f64 / total.max(1) as f64
    );
    if enriched.is_empty() {
        println!("nothing enriched — nothing to compare");
        return Ok(());
    }

    // Wing corpora are built INCREMENTALLY, matching production's write-time
    // semantics: `AnalysisContext.wing_corpus` is the content of memories that
    // *already exist* in the wing, so a memory is never part of the corpus its
    // own novelty is scored against.
    //
    // The first version of this probe built each corpus from every memory
    // including the one under analysis. Every word was then trivially "seen",
    // novelty collapsed to ~0 for all 2,712 memories with zero variance, and a
    // sixth of the fingerprint contributed nothing to the separation figure.
    // That was an artefact of the harness, not a property of the data.
    let mut corpus_a: HashMap<String, String> = HashMap::new();
    let mut corpus_b: HashMap<String, String> = HashMap::new();

    // Cap the corpus a memory is scored against. `novelty` does substring
    // containment over the whole string, so an unbounded corpus is both
    // quadratic and — past a point — saturated: every short word appears
    // somewhere. Production has the same property; the cap keeps the two arms
    // comparable and the run tractable.
    const WING_CORPUS_CAP: usize = 200_000;

    let analyzer = SpectrogramAnalyzer::new(AnalyzerConfig {
        peak_dimension_count: 3,
    });

    let mut va: Vec<[f64; 6]> = Vec::with_capacity(enriched.len());
    let mut vb: Vec<[f64; 6]> = Vec::with_capacity(enriched.len());
    let mut peaks_changed = 0usize;
    let mut action_changed = 0usize;
    let mut any_dim_moved = 0usize;
    let mut per_dim_delta: [Vec<f64>; 6] = Default::default();

    for (id, content, desc, wing) in &enriched {
        let w = wing.clone().unwrap_or_else(|| "_none".into());
        let mem_a = blank_memory(id, content.clone(), wing.clone());
        let mem_b = blank_memory(
            id,
            format!("{}\n{}", content, desc.as_deref().unwrap_or("")),
            wing.clone(),
        );

        let ctx_a = AnalysisContext {
            wing_corpus: corpus_a.get(&w).cloned().unwrap_or_default(),
        };
        let ctx_b = AnalysisContext {
            wing_corpus: corpus_b.get(&w).cloned().unwrap_or_default(),
        };

        let fa = analyzer.analyze(&mem_a, &ctx_a);
        let fb = analyzer.analyze(&mem_b, &ctx_b);

        let (a, b) = (vector(&fa), vector(&fb));
        for k in 0..6 {
            per_dim_delta[k].push(b[k] - a[k]);
        }
        if a.iter().zip(b.iter()).any(|(x, y)| (x - y).abs() > 0.05) {
            any_dim_moved += 1;
        }
        if fa.peak_dimensions != fb.peak_dimensions {
            peaks_changed += 1;
        }
        if fa.action_type != fb.action_type {
            action_changed += 1;
        }
        va.push(a);
        vb.push(b);

        // Only now does this memory join the corpus, so the NEXT memory in the
        // wing is scored against it — never itself.
        let ca = corpus_a.entry(w.clone()).or_default();
        if ca.len() < WING_CORPUS_CAP {
            ca.push_str(content);
            ca.push(' ');
        }
        let cb = corpus_b.entry(w).or_default();
        if cb.len() < WING_CORPUS_CAP {
            cb.push_str(content);
            cb.push(' ');
            if let Some(d) = desc {
                cb.push_str(d);
                cb.push(' ');
            }
        }
    }

    let n = enriched.len() as f64;
    println!("\n── per-dimension shift (arm B minus arm A) ──");
    println!(
        "  {:<22} {:>9} {:>9} {:>9} {:>9}",
        "dimension", "meanA", "meanB", "mean d", "var B/A"
    );
    for (k, name) in DIMS.iter().enumerate() {
        let ma = mean(&va.iter().map(|v| v[k]).collect::<Vec<_>>());
        let mb = mean(&vb.iter().map(|v| v[k]).collect::<Vec<_>>());
        let var_a = variance(&va.iter().map(|v| v[k]).collect::<Vec<_>>());
        let var_b = variance(&vb.iter().map(|v| v[k]).collect::<Vec<_>>());
        let ratio = if var_a > 0.0 { var_b / var_a } else { f64::NAN };
        println!(
            "  {:<22} {:>9.4} {:>9.4} {:>+9.4} {:>9.2}",
            name,
            ma,
            mb,
            mean(&per_dim_delta[k]),
            ratio
        );
    }

    let sep_a = mean_pairwise_distance(&va);
    let sep_b = mean_pairwise_distance(&vb);
    let sep_gain = if sep_a > 0.0 {
        100.0 * (sep_b - sep_a) / sep_a
    } else {
        f64::NAN
    };

    println!("\n── separation in 7-space (the thing resonance needs) ──");
    println!("  mean pairwise distance, arm A : {sep_a:.4}");
    println!("  mean pairwise distance, arm B : {sep_b:.4}");
    println!("  change                        : {sep_gain:+.1}%");

    println!("\n── how much the fingerprint moved ──");
    println!(
        "  any dimension moved >0.05     : {any_dim_moved} ({:.1}%)",
        100.0 * any_dim_moved as f64 / n
    );
    println!(
        "  peak_dimensions set changed   : {peaks_changed} ({:.1}%)",
        100.0 * peaks_changed as f64 / n
    );
    println!(
        "  action_type changed           : {action_changed} ({:.1}%)",
        100.0 * action_changed as f64 / n
    );

    println!("\n── preregistered decision rule ──");
    let sep_ok = sep_gain >= 25.0;
    let peaks_ok = 100.0 * peaks_changed as f64 / n >= 40.0;
    println!("  separation >= +25%            : {sep_ok}");
    println!("  peak set changed >= 40%       : {peaks_ok}");
    println!(
        "  VERDICT                       : {}",
        if sep_ok && peaks_ok {
            "ADVANCE to a preregistered retrieval arm"
        } else {
            "STOP — enrichment does not separate the space; retirement stands"
        }
    );
    Ok(())
}
