//! Ambient stream replay against a real brain: enroll the first 60% of the
//! timeline as reference segments, track the last 40% as a live stream.
//!
//! Reports: locks, lock wing-precision (does the locked routine's wing
//! match the live cue's wing — a proxy for "recognized the right kind of
//! work"), events/day (flapping check), and latency.
//!
//! Usage: stream_replay --db <path/to/memory.db>

use anyhow::{Context, Result};
use spectral_recognition::{
    extract_landmarks, make_cue, segment_stream, RecognitionConfig, StreamConfig, StreamEvent,
    StreamTracker,
};

fn parse_ts(s: &str) -> Option<i64> {
    chrono_free_parse(s)
}

/// Parse "YYYY-MM-DD HH:MM:SS" or RFC3339-ish without a chrono dependency.
fn chrono_free_parse(s: &str) -> Option<i64> {
    let s = s.replace('T', " ");
    let (date, time) = s.split_once(' ')?;
    let mut d = date.split('-');
    let (y, mo, da): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let mut t = time.trim_end_matches('Z').split(':');
    let (h, mi): (i64, i64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    let sec: i64 = t
        .next()
        .and_then(|x| x.split(&['+', '.'][..]).next())
        .and_then(|x| x.parse().ok())
        .unwrap_or(0);
    // Days since epoch (civil, Howard Hinnant's algorithm, deterministic).
    let (y2, mo2) = if mo <= 2 { (y - 1, mo + 12) } else { (y, mo) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (mo2 - 3) + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + sec)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let db_path = args
        .iter()
        .position(|a| a == "--db")
        .and_then(|i| args.get(i + 1))
        .context("--db <path> required")?;

    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut stmt = conn.prepare(
        "SELECT COALESCE(wing,'general'), content, created_at FROM memories
         WHERE created_at IS NOT NULL ORDER BY created_at",
    )?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;

    let rc = RecognitionConfig::default();
    let items: Vec<(spectral_recognition::Cue, String, i64)> = rows
        .iter()
        .filter_map(|(wing, content, ts)| {
            let epoch = parse_ts(ts)?;
            let hour = ((epoch % 86400) / 3600) as u8;
            // Epoch day 0 (1970-01-01) was a Thursday = 4.
            let dow = ((epoch / 86400 + 4) % 7) as u8;
            let peaks = extract_landmarks(content, &rc);
            let peak_keys: Vec<&str> = peaks.iter().map(|l| l.key.as_str()).collect();
            Some((
                make_cue(wing, dow, hour, &peak_keys, content.len()),
                wing.clone(),
                epoch,
            ))
        })
        .collect();
    if items.len() < 50 {
        anyhow::bail!("not enough timestamped memories ({})", items.len());
    }

    let span_days =
        (items.last().unwrap().2 - items.first().unwrap().2) as f64 / 86400.0;
    let split = items.first().unwrap().2
        + ((items.last().unwrap().2 - items.first().unwrap().2) as f64 * 0.6) as i64;
    let (past, live): (Vec<_>, Vec<_>) = items.into_iter().partition(|(_, _, t)| *t <= split);
    eprintln!(
        "stream: {} cues over {:.0} days — enroll {} / track {}",
        past.len() + live.len(),
        span_days,
        past.len(),
        live.len()
    );

    // Enroll reference centroids from history (wing labels are allowed in
    // the CATALOG; they are excluded from live scoring).
    let segments = segment_stream(&past, 45, 3, 32);
    let mut tracker =
        spectral_recognition::CentroidTracker::new(spectral_recognition::CentroidConfig::default());
    let n_segments = segments.len();
    for s in &segments {
        tracker.enroll(spectral_recognition::centroid_of(s));
    }
    eprintln!("reference catalog: {n_segments} segment centroids");

    // Track the live tail. Boundaries are WING-BLIND (time gap only) so the
    // ground-truth label can't leak into recognition.
    let live_days =
        (live.last().map(|x| x.2).unwrap_or(split) - split) as f64 / 86400.0;
    let mut locks = 0usize;
    let mut wing_correct = 0usize;
    let mut transfers = 0usize;
    let mut locked_cues = 0usize;
    let mut locked_correct_cues = 0usize;
    let mut specific_cues = 0usize;
    let mut specific_correct = 0usize;
    let mut last_ts: i64 = 0;
    let t = std::time::Instant::now();
    for (cue, wing, ts) in &live {
        let boundary = last_ts != 0 && ts - last_ts > 45 * 60;
        last_ts = *ts;
        for ev in tracker.observe(cue, boundary) {
            match ev {
                StreamEvent::LockAcquired { segment_id, .. } => {
                    locks += 1;
                    if segment_id.ends_with(wing.as_str()) {
                        wing_correct += 1;
                    }
                }
                StreamEvent::LockTransferred { to, .. } => {
                    transfers += 1;
                    if to.ends_with(wing.as_str()) {
                        wing_correct += 1;
                    }
                }
                StreamEvent::LockLost { .. } => {}
            }
        }
        if let Some(c) = tracker.current_lock() {
            locked_cues += 1;
            if c.wing == *wing {
                locked_correct_cues += 1;
            }
            // Wing labels come from a weak regex classifier that defaults
            // most content to "general" — precision against SPECIFIC wings
            // is the trustworthy slice of this metric.
            if *wing != "general" {
                specific_cues += 1;
                if c.wing == *wing {
                    specific_correct += 1;
                }
            }
        }
    }
    let per_cue_us = t.elapsed().as_secs_f64() * 1e6 / live.len().max(1) as f64;
    let losses = 0usize;
    let _ = losses;

    println!("== ambient stream replay (centroid tracker, wing-blind) ==");
    println!("reference segments:     {n_segments}");
    println!("live cues tracked:      {} over {live_days:.0} days", live.len());
    println!(
        "lock events:            {} acquired + {transfers} transferred ({:.1}/day)",
        locks,
        (locks + transfers) as f64 / live_days.max(0.1)
    );
    println!(
        "event wing-precision:   {:.1}% ({wing_correct}/{})",
        100.0 * wing_correct as f64 / (locks + transfers).max(1) as f64,
        locks + transfers
    );
    println!(
        "time-in-lock:           {:.1}% of live cues, {:.1}% of locked cues wing-correct",
        100.0 * locked_cues as f64 / live.len().max(1) as f64,
        100.0 * locked_correct_cues as f64 / locked_cues.max(1) as f64
    );
    println!(
        "specific-wing slice:    {:.1}% wing-correct while locked ({specific_correct}/{specific_cues}; 'general' labels excluded as classifier noise)",
        100.0 * specific_correct as f64 / specific_cues.max(1) as f64
    );
    println!("latency:                {per_cue_us:.0} µs/cue");
    Ok(())
}
