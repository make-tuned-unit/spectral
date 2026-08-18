//! R36 — does Librarian enrichment give the recognition engine more to work with?
//!
//! The companion to R35, which measured the *spectrogram*'s seven dimensions.
//! Recognition is a different engine with different needs, so the brief for the
//! Librarian cannot be assumed to transfer: the spectrogram wants memories to be
//! **spread apart** in a feature space, while recognition wants **rare, stable
//! landmarks** that survive re-encounter.
//!
//! Landmarks are IDF-ranked salient features — rare stems, numbers, identifiers,
//! entities. `Landmark::anchor` marks the ones preserved verbatim (numbers,
//! error codes, identifiers), which are the strongest evidence recognition has,
//! because they are both rare and exactly repeatable.
//!
//! Two arms over the same memories:
//!   A (status quo): content
//!   B (enriched):   content + "\n" + description
//!
//! Run:
//! ```text
//! cargo run -p spectral-recognition --example enrichment_landmarks -- ~/.permagent/brain/memory.db
//! ```
//!
//! **Privacy: statistics only.** No content, description, landmark text, key or
//! id is printed or written. The store is opened read-only.

use spectral_recognition::{extract_landmarks, Landmark, RecognitionConfig};

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn median(xs: &mut [f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    xs[xs.len() / 2]
}

fn anchors(ls: &[Landmark]) -> usize {
    ls.iter().filter(|l| l.anchor).count()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let db = args
        .get(1)
        .ok_or("usage: enrichment_landmarks <memory.db>")?;

    let conn = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;

    let mut stmt = conn.prepare(
        "SELECT content, description FROM memories
         WHERE content IS NOT NULL AND TRIM(content) <> ''
           AND description IS NOT NULL AND TRIM(description) <> ''",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;

    let cfg = RecognitionConfig::default();

    let mut n = 0usize;
    let (mut la, mut lb) = (Vec::new(), Vec::new());
    let (mut aa, mut ab) = (Vec::new(), Vec::new());
    let mut gained_anchor = 0usize;
    let mut lost_anchor = 0usize;
    let mut anchor_set_identical = 0usize;
    let mut density_a = Vec::new();
    let mut density_b = Vec::new();

    for row in rows {
        let (content, desc) = row?;
        let enriched = format!("{content}\n{desc}");

        let a = extract_landmarks(&content, &cfg);
        let b = extract_landmarks(&enriched, &cfg);

        // Landmarks per 1,000 characters — the density that matters, since a
        // longer text trivially yields more landmarks.
        density_a.push(1000.0 * a.len() as f64 / content.len().max(1) as f64);
        density_b.push(1000.0 * b.len() as f64 / enriched.len().max(1) as f64);

        let (ca, cb) = (anchors(&a), anchors(&b));
        if cb > ca {
            gained_anchor += 1;
        } else if cb < ca {
            lost_anchor += 1;
        }

        // Do the verbatim anchors from the raw content SURVIVE enrichment?
        // Recognition matches an incoming raw stimulus against what was
        // enrolled, so an anchor present in content but absent after enrichment
        // is evidence the engine can no longer use.
        let set_a: std::collections::HashSet<&str> = a
            .iter()
            .filter(|l| l.anchor)
            .map(|l| l.key.as_str())
            .collect();
        let set_b: std::collections::HashSet<&str> = b
            .iter()
            .filter(|l| l.anchor)
            .map(|l| l.key.as_str())
            .collect();
        if !set_a.is_empty() && set_a.is_subset(&set_b) {
            anchor_set_identical += 1;
        }

        la.push(a.len() as f64);
        lb.push(b.len() as f64);
        aa.push(ca as f64);
        ab.push(cb as f64);
        n += 1;
    }

    println!("R36 landmark probe (recognition engine)");
    println!("  enriched memories analysed : {n}");
    if n == 0 {
        return Ok(());
    }

    println!("\n── landmarks per memory ──");
    println!("  mean, arm A (content)          : {:.2}", mean(&la));
    println!("  mean, arm B (content+desc)     : {:.2}", mean(&lb));
    println!(
        "  change                         : {:+.1}%",
        100.0 * (mean(&lb) - mean(&la)) / mean(&la).max(f64::EPSILON)
    );

    println!("\n── landmark DENSITY (per 1k chars — the fair comparison) ──");
    println!("  mean, arm A                    : {:.2}", mean(&density_a));
    println!("  mean, arm B                    : {:.2}", mean(&density_b));
    println!(
        "  median, arm A / B              : {:.2} / {:.2}",
        median(&mut density_a.clone()),
        median(&mut density_b.clone())
    );
    println!(
        "  change in mean density         : {:+.1}%",
        100.0 * (mean(&density_b) - mean(&density_a)) / mean(&density_a).max(f64::EPSILON)
    );

    println!("\n── verbatim anchors (numbers, ids, error codes — strongest evidence) ──");
    println!("  mean per memory, arm A         : {:.2}", mean(&aa));
    println!("  mean per memory, arm B         : {:.2}", mean(&ab));
    println!(
        "  change                         : {:+.1}%",
        100.0 * (mean(&ab) - mean(&aa)) / mean(&aa).max(f64::EPSILON)
    );
    println!(
        "  memories gaining an anchor     : {gained_anchor} ({:.1}%)",
        100.0 * gained_anchor as f64 / n as f64
    );
    println!(
        "  memories LOSING an anchor      : {lost_anchor} ({:.1}%)",
        100.0 * lost_anchor as f64 / n as f64
    );
    println!(
        "  content anchors all preserved  : {anchor_set_identical} ({:.1}%)",
        100.0 * anchor_set_identical as f64 / n as f64
    );

    println!("\n── reading ──");
    println!("  Density UP   => enrichment adds salient, rare material.");
    println!("  Density DOWN => enrichment dilutes: more text, proportionally");
    println!("                  less that recognition can key on.");
    println!("  Anchors lost => enrichment is paraphrasing away identifiers,");
    println!("                  which is the strongest evidence the engine has.");
    Ok(())
}
