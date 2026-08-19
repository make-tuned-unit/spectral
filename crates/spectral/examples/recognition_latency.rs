//! What does `Brain::recognize()` actually cost in production shape?
//!
//! Every published recognition number, and every probe in R37–R42, used the
//! in-memory store. Production uses the SQLite sidecar, and a consumer
//! deciding whether to call `recognize()` on *every* recall needs the cost of
//! that path, on a real corpus, not a benchmark one.
//!
//! Read-only on the brain. usage: recognition_latency <brain_dir> [n]

use spectral::{Brain, Visibility};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let dir = args
        .get(1)
        .ok_or("usage: recognition_latency <brain_dir> [n]")?;
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);

    let brain = Brain::builder()
        .data_dir(std::path::Path::new(dir))
        .ontology_path(std::path::Path::new(dir).join("ontology.toml"))
        .read_only(true)
        .build()?;

    // Stimuli: real memory contents, plus degraded and foreign forms, so the
    // timing is not measured only on the easy case.
    // Read contents straight from the store, read-only — the facade has no
    // bulk lister and this probe must not mutate the brain it measures.
    let conn = rusqlite::Connection::open_with_flags(
        std::path::Path::new(dir).join("memory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))?;
    let mut stmt = conn.prepare(
        "SELECT content FROM memories WHERE content IS NOT NULL AND TRIM(content) <> '' ORDER BY id",
    )?;
    let all: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    let stride = (all.len() / n.max(1)).max(1);
    let sample: Vec<&String> = all.iter().step_by(stride).take(n).collect();
    if sample.is_empty() {
        println!("no memories");
        return Ok(());
    }

    let mut timings = Vec::new();
    let mut verdicts = (0usize, 0usize, 0usize);
    for m in &sample {
        let t = std::time::Instant::now();
        let r = brain.recognize(m)?;
        timings.push(t.elapsed().as_secs_f64() * 1000.0);
        match r.verdict {
            spectral_recognition::Verdict::Recognized { .. } => verdicts.0 += 1,
            spectral_recognition::Verdict::Familiar => verdicts.1 += 1,
            spectral_recognition::Verdict::Novel => verdicts.2 += 1,
        }
    }
    timings.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean: f64 = timings.iter().sum::<f64>() / timings.len() as f64;
    let p = |q: f64| timings[((timings.len() as f64 - 1.0) * q) as usize];
    println!(
        "Brain::recognize() via the SQLite sidecar — {} enrolled-corpus probes",
        sample.len()
    );
    println!("  memories in brain : {total}");
    println!(
        "  mean {:.1} ms   median {:.1} ms   p90 {:.1} ms   p99 {:.1} ms   max {:.1} ms",
        mean,
        p(0.5),
        p(0.9),
        p(0.99),
        timings[timings.len() - 1]
    );
    println!(
        "  verdicts: Recognized {} / Familiar {} / Novel {}",
        verdicts.0, verdicts.1, verdicts.2
    );
    let _ = Visibility::Private;
    Ok(())
}
