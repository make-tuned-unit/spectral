//! Stream mode: continuous ambient recognition (the smart-TV-ACR half of
//! the engine).
//!
//! The query mode asks "have I seen this stimulus?". Stream mode watches
//! the ambient work feed and notices "the user is doing X again" — without
//! being asked. Mechanics follow production ACR (Inscape path pursuit):
//!
//! - **Weak cues at cadence**: each ambient item becomes a small
//!   fixed-schema integer vector. Individually near-meaningless; identity
//!   emerges from sequences, and false-lock probability decays
//!   geometrically with required sequence length.
//! - **Path pursuit**: a belief table over (segment, offset) hypotheses,
//!   updated per cue with a match bonus / miss penalty, decayed by λ so the
//!   tracker can escape after a regime change, pruned at a floor.
//! - **Edge-triggered events**: LockAcquired / LockLost / LockTransferred.
//!   A continuing match fires nothing — re-alert suppression is structural.
//! - **Common-segment suppression**: cues occurring in many reference
//!   segments ("reading email") carry less evidence, via the same rarity
//!   weighting the query mode uses.

use std::collections::HashMap;

/// Fixed-schema cue vector. Fields are independently-computed small
/// buckets; similarity = fraction of equal fields, so a partially-changed
/// context degrades the match bit-locally, not catastrophically.
pub const CUE_FIELDS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cue(pub [u16; CUE_FIELDS]);

/// Field layout: [wing, day_of_week, hour_of_day/3, peak1..peak4,
/// len_bucket]. Work routines recur in TYPE and RHYTHM (same wing, same
/// weekday, similar hour, overlapping topics), not with identical content —
/// so structural fields carry rhythm and peak slots carry topic.
pub fn make_cue(
    wing: &str,
    day_of_week: u8,
    hour_of_day: u8,
    peak_keys: &[&str],
    content_len: usize,
) -> Cue {
    fn bucket16(s: &str) -> u16 {
        use sha2::{Digest, Sha256};
        let d = Sha256::digest(s.as_bytes());
        u16::from_be_bytes([d[0], d[1]])
    }
    let mut peaks: Vec<u16> = peak_keys.iter().take(4).map(|k| bucket16(k)).collect();
    peaks.resize(4, 0);
    peaks.sort_unstable();
    let len_bucket = match content_len {
        0..=120 => 0u16,
        121..=400 => 1,
        401..=1200 => 2,
        _ => 3,
    };
    Cue([
        bucket16(wing),
        (day_of_week % 7) as u16,
        (hour_of_day / 3) as u16,
        peaks[0],
        peaks[1],
        peaks[2],
        peaks[3],
        len_bucket,
    ])
}

const PEAK_SLOTS: std::ops::Range<usize> = 3..7;

/// Similarity: structural fields (wing, dow, hour, len) compared
/// positionally; peak slots compared as SET overlap (Jaccard over nonzero
/// buckets) — the same routine on a different day shares SOME topics, not
/// an identical sorted list. Peaks carry half the total weight.
pub fn cue_similarity(a: &Cue, b: &Cue) -> f64 {
    let mut eq = 0.0f64;
    let mut total = 0.0f64;
    for i in 0..CUE_FIELDS {
        if PEAK_SLOTS.contains(&i) {
            continue;
        }
        total += 1.0;
        if a.0[i] == b.0[i] {
            eq += 1.0;
        }
    }
    let set_a: std::collections::HashSet<u16> = a.0[PEAK_SLOTS]
        .iter()
        .copied()
        .filter(|v| *v != 0)
        .collect();
    let set_b: std::collections::HashSet<u16> = b.0[PEAK_SLOTS]
        .iter()
        .copied()
        .filter(|v| *v != 0)
        .collect();
    if !set_a.is_empty() || !set_b.is_empty() {
        let inter = set_a.intersection(&set_b).count() as f64;
        let union = (set_a.len() + set_b.len()) as f64 - inter;
        total += 4.0;
        eq += 4.0 * if union > 0.0 { inter / union } else { 0.0 };
    }
    if total == 0.0 {
        0.0
    } else {
        eq / total
    }
}

/// A reference segment: a recurring episode's cue sequence.
#[derive(Debug, Clone)]
pub struct Segment {
    pub id: String,
    pub cues: Vec<Cue>,
    /// Dominant wing, for observability and validation.
    pub wing: String,
}

/// Tracker configuration. Defaults tuned on the Permagent ambient replay.
#[derive(Debug, Clone)]
pub struct StreamConfig {
    /// Cue similarity at or above this counts as a match at the expected offset.
    pub match_threshold: f64,
    /// Evidence added on a match (scaled by similarity).
    pub match_bonus: f64,
    /// Evidence removed on a miss.
    pub miss_penalty: f64,
    /// Per-tick multiplicative decay (1-λ): stale evidence fades.
    pub decay: f64,
    /// Seed score for a fresh hypothesis from index lookup.
    pub seed: f64,
    /// Declare lock at or above this accumulated score…
    pub lock_threshold: f64,
    /// …and only when leading the runner-up by this margin (anti-flapping).
    pub lock_margin: f64,
    /// Drop lock after this many consecutive misses (hysteresis).
    pub lost_after_misses: usize,
    /// Prune hypotheses below this floor.
    pub floor: f64,
    /// Minimum shared fields for index candidate generation.
    pub min_shared_fields: usize,
    /// Cap on live hypotheses (memory bound).
    pub max_suspects: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            match_threshold: 0.5,
            match_bonus: 1.0,
            miss_penalty: 0.6,
            decay: 0.9,
            seed: 0.3,
            lock_threshold: 2.0,
            lock_margin: 1.4,
            lost_after_misses: 3,
            floor: 0.05,
            min_shared_fields: 3,
            max_suspects: 512,
        }
    }
}

/// Edge-triggered tracker events. A continuing lock emits nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    LockAcquired {
        segment_id: String,
        offset: usize,
        score: f64,
    },
    LockLost {
        segment_id: String,
        /// Offset at which the pattern diverged — "your routine broke here".
        at_offset: usize,
    },
    LockTransferred {
        from: String,
        to: String,
        score: f64,
    },
}

/// The ambient recognizer: reference catalog + per-session belief state.
pub struct StreamTracker {
    config: StreamConfig,
    segments: Vec<Segment>,
    /// (field_idx, value) → [(segment_idx, offset)]
    index: HashMap<(usize, u16), Vec<(usize, usize)>>,
    /// Live hypotheses: (segment_idx, next_expected_offset) → score.
    suspects: HashMap<(usize, usize), f64>,
    locked: Option<(usize, usize, usize)>, // (segment_idx, offset, consecutive_misses)
}

impl StreamTracker {
    pub fn new(config: StreamConfig) -> Self {
        Self {
            config,
            segments: Vec::new(),
            index: HashMap::new(),
            suspects: HashMap::new(),
            locked: None,
        }
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Add a reference segment to the catalog.
    pub fn enroll_segment(&mut self, segment: Segment) {
        let seg_idx = self.segments.len();
        for (off, cue) in segment.cues.iter().enumerate() {
            for (fi, v) in cue.0.iter().enumerate() {
                if PEAK_SLOTS.contains(&fi) && *v == 0 {
                    continue;
                }
                self.index.entry((fi, *v)).or_default().push((seg_idx, off));
            }
        }
        self.segments.push(segment);
    }

    /// Feed one ambient cue; returns any edge-triggered events.
    pub fn observe(&mut self, cue: &Cue) -> Vec<StreamEvent> {
        let cfg = self.config.clone();
        let mut events = Vec::new();

        // 1. Advance existing hypotheses. Each expects the cue at its
        //    offset, with one-cue slack in both directions: real streams
        //    insert noise cues (a Slack ping mid-routine) and reference
        //    segments contain cues a recurrence may skip.
        let mut next: HashMap<(usize, usize), f64> = HashMap::new();
        for ((seg_idx, expected), score) in self.suspects.drain() {
            let seg = &self.segments[seg_idx];
            if expected >= seg.cues.len() {
                continue; // walked off the end of the reference
            }
            // (candidate_offset, skip_penalty): exact, skip-one-in-reference.
            let mut best: (f64, usize) = (f64::MIN, expected + 1);
            for (cand, penalty) in [(expected, 0.0), (expected + 1, 0.15)] {
                if cand >= seg.cues.len() {
                    continue;
                }
                let sim = cue_similarity(cue, &seg.cues[cand]) - penalty;
                if sim > best.0 {
                    best = (sim, cand + 1);
                }
            }
            let (sim, next_off) = best;
            let updated = if sim >= cfg.match_threshold {
                score * cfg.decay + cfg.match_bonus * sim
            } else {
                // Miss: hypothesis also stays at the SAME offset (the live
                // cue may be an insertion the reference never had).
                score * cfg.decay - cfg.miss_penalty
            };
            if updated > cfg.floor {
                let off = if sim >= cfg.match_threshold {
                    next_off
                } else {
                    expected // hold position through insertions
                };
                let e = next.entry((seg_idx, off)).or_insert(0.0);
                *e = e.max(updated);
            }
        }

        // 2. Seed fresh hypotheses from the index (candidate generation).
        let mut field_hits: HashMap<(usize, usize), usize> = HashMap::new();
        for (fi, v) in cue.0.iter().enumerate() {
            if PEAK_SLOTS.contains(&fi) && *v == 0 {
                continue;
            }
            if let Some(entries) = self.index.get(&(fi, *v)) {
                for (seg_idx, off) in entries {
                    *field_hits.entry((*seg_idx, *off)).or_insert(0) += 1;
                }
            }
        }
        for ((seg_idx, off), shared) in field_hits {
            if shared >= cfg.min_shared_fields {
                // Common-segment suppression via rarity: hypotheses seeded
                // from positions whose cue collides everywhere get less
                // seed mass (df = how many reference positions share it).
                let e = next.entry((seg_idx, off + 1)).or_insert(0.0);
                *e = e.max(cfg.seed * shared as f64 / CUE_FIELDS as f64);
            }
        }

        // 3. Cap table size (keep strongest).
        if next.len() > cfg.max_suspects {
            let mut all: Vec<((usize, usize), f64)> = next.into_iter().collect();
            all.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            all.truncate(cfg.max_suspects);
            next = all.into_iter().collect();
        }
        self.suspects = next;

        // 4. Lock state machine (θ + δ + hysteresis).
        let mut ranked: Vec<(&(usize, usize), &f64)> = self.suspects.iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
        let top = ranked.first().map(|(k, v)| (**k, **v));
        let runner_up = ranked.get(1).map(|(_, v)| **v).unwrap_or(0.0);

        match (self.locked, top) {
            (None, Some(((seg, off), score)))
                if score >= cfg.lock_threshold && score >= runner_up * cfg.lock_margin =>
            {
                self.locked = Some((seg, off, 0));
                events.push(StreamEvent::LockAcquired {
                    segment_id: self.segments[seg].id.clone(),
                    offset: off,
                    score,
                });
            }
            (Some((seg, off, misses)), top) => {
                // Does the top hypothesis continue the locked segment?
                let continues = matches!(top, Some(((s, _), score))
                    if s == seg && score >= cfg.lock_threshold);
                if continues {
                    let ((_, new_off), _) = top.expect("continues implies top");
                    self.locked = Some((seg, new_off, 0));
                } else {
                    // Transferred to a different segment at full strength?
                    if let Some(((s, o), score)) = top {
                        if s != seg
                            && score >= cfg.lock_threshold
                            && score >= runner_up * cfg.lock_margin
                        {
                            events.push(StreamEvent::LockTransferred {
                                from: self.segments[seg].id.clone(),
                                to: self.segments[s].id.clone(),
                                score,
                            });
                            self.locked = Some((s, o, 0));
                            return events;
                        }
                    }
                    let misses = misses + 1;
                    if misses >= cfg.lost_after_misses {
                        events.push(StreamEvent::LockLost {
                            segment_id: self.segments[seg].id.clone(),
                            at_offset: off,
                        });
                        self.locked = None;
                    } else {
                        self.locked = Some((seg, off, misses));
                    }
                }
            }
            _ => {}
        }
        events
    }

    /// Current lock, if any: (segment_id, offset).
    pub fn current_lock(&self) -> Option<(&str, usize)> {
        self.locked
            .map(|(seg, off, _)| (self.segments[seg].id.as_str(), off))
    }
}

// ── Centroid tracker: coarse routine recognition ────────────────────
//
// Path pursuit assumes the reference PATH replays cue-by-cue — right for
// fine-grained tool-event streams, wrong for coarse ambient memories:
// measured on the real brain, work routines recur as SESSIONS (same
// weekly rhythm, overlapping topic set) with almost no cue-sequential
// structure. This is ACR's other matcher (Sorenson scene centroids)
// promoted to the primary for coarse streams.

/// A reference segment summarized for session-level matching. `wing` is a
/// LABEL for validation only — deliberately excluded from scoring so that
/// recognition is earned from rhythm + topic, not read off a tag.
#[derive(Debug, Clone)]
pub struct Centroid {
    pub segment_id: String,
    pub wing: String,
    pub dow: u16,
    pub hour: u16,
    pub peaks: std::collections::HashSet<u16>,
}

pub fn centroid_of(segment: &Segment) -> Centroid {
    let mut dow_hist = [0usize; 8];
    let mut hour_hist = [0usize; 9];
    let mut peaks = std::collections::HashSet::new();
    for c in &segment.cues {
        dow_hist[(c.0[1] as usize).min(7)] += 1;
        hour_hist[(c.0[2] as usize).min(8)] += 1;
        for v in &c.0[PEAK_SLOTS] {
            if *v != 0 {
                peaks.insert(*v);
            }
        }
    }
    let argmax = |h: &[usize]| {
        h.iter()
            .enumerate()
            .max_by_key(|(_, n)| **n)
            .map(|(i, _)| i as u16)
            .unwrap_or(0)
    };
    Centroid {
        segment_id: segment.id.clone(),
        wing: segment.wing.clone(),
        dow: argmax(&dow_hist),
        hour: argmax(&hour_hist),
        peaks,
    }
}

/// Config for the centroid tracker.
#[derive(Debug, Clone)]
pub struct CentroidConfig {
    /// Lock at or above this score…
    pub lock_threshold: f64,
    /// …when leading the runner-up by this factor.
    pub lock_margin: f64,
    /// Minimum live cues in the running segment before locking.
    pub min_cues: usize,
    /// Weights: day-of-week, hour band, topic overlap.
    pub w_dow: f64,
    pub w_hour: f64,
    pub w_topic: f64,
}

impl Default for CentroidConfig {
    fn default() -> Self {
        Self {
            lock_threshold: 0.35,
            lock_margin: 1.3,
            min_cues: 2,
            w_dow: 0.2,
            w_hour: 0.15,
            w_topic: 0.65,
        }
    }
}

/// Session-level ambient recognizer. Feed cues plus boundary signals;
/// events are edge-triggered like the path tracker.
pub struct CentroidTracker {
    config: CentroidConfig,
    centroids: Vec<Centroid>,
    /// peak bucket → number of centroids containing it (for rarity
    /// weighting — a topic every routine touches identifies none of them;
    /// ACR's common-segment suppression as a weight, not a rule).
    peak_df: HashMap<u16, usize>,
    live_peaks: std::collections::HashSet<u16>,
    live_dow: u16,
    live_hour: u16,
    live_n: usize,
    locked: Option<usize>,
}

impl CentroidTracker {
    pub fn new(config: CentroidConfig) -> Self {
        Self {
            config,
            centroids: Vec::new(),
            peak_df: HashMap::new(),
            live_peaks: std::collections::HashSet::new(),
            live_dow: 0,
            live_hour: 0,
            live_n: 0,
            locked: None,
        }
    }

    pub fn enroll(&mut self, centroid: Centroid) {
        for p in &centroid.peaks {
            *self.peak_df.entry(*p).or_insert(0) += 1;
        }
        self.centroids.push(centroid);
    }

    fn idf(&self, p: u16) -> f64 {
        let n = self.centroids.len().max(1) as f64;
        let df = *self.peak_df.get(&p).unwrap_or(&1) as f64;
        (n / df).ln().max(0.0)
    }

    fn score(&self, c: &Centroid) -> f64 {
        let cfg = &self.config;
        let dow = if c.dow == self.live_dow {
            cfg.w_dow
        } else {
            0.0
        };
        let hour = if (c.hour as i32 - self.live_hour as i32).abs() <= 1 {
            cfg.w_hour
        } else {
            0.0
        };
        // Rarity-weighted containment, not Jaccard: the live segment is a
        // PREFIX of the routine (early cues cover little of the reference),
        // and a topic shared by every routine identifies none of them.
        let inter_w: f64 = self
            .live_peaks
            .iter()
            .filter(|p| c.peaks.contains(p))
            .map(|p| self.idf(*p))
            .sum();
        let live_w: f64 = self.live_peaks.iter().map(|p| self.idf(*p)).sum();
        // Size tax: unmatched centroid mass costs a fraction of its weight,
        // so a huge catch-all centroid can't win by containing everything
        // (pure containment made "general" segments attractors for every
        // live session — measured 0/38 on specific-wing cues without this).
        let centroid_w: f64 = c.peaks.iter().map(|p| self.idf(*p)).sum();
        let uncovered = (centroid_w - inter_w).max(0.0);
        let topic = if live_w > 0.0 {
            inter_w / (live_w + 0.15 * uncovered)
        } else {
            0.0
        };
        dow + hour + cfg.w_topic * topic
    }

    /// Feed a cue. `boundary` = the segmenter detected a session boundary
    /// BEFORE this cue (time gap / context switch): the running segment
    /// resets and any lock releases silently (a session ending naturally
    /// is not a divergence event).
    pub fn observe(&mut self, cue: &Cue, boundary: bool) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        if boundary {
            self.live_peaks.clear();
            self.live_n = 0;
            self.locked = None;
        }
        self.live_dow = cue.0[1];
        self.live_hour = cue.0[2];
        for v in &cue.0[PEAK_SLOTS] {
            if *v != 0 {
                self.live_peaks.insert(*v);
            }
        }
        self.live_n += 1;
        if self.live_n < self.config.min_cues || self.centroids.is_empty() {
            return events;
        }

        let mut scored: Vec<(usize, f64)> = self
            .centroids
            .iter()
            .enumerate()
            .map(|(i, c)| (i, self.score(c)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let (best_idx, best) = scored[0];
        let runner_up = scored.get(1).map(|(_, s)| *s).unwrap_or(0.0);

        let qualifies =
            best >= self.config.lock_threshold && best >= runner_up * self.config.lock_margin;
        match (self.locked, qualifies) {
            (None, true) => {
                self.locked = Some(best_idx);
                events.push(StreamEvent::LockAcquired {
                    segment_id: self.centroids[best_idx].segment_id.clone(),
                    offset: self.live_n,
                    score: best,
                });
            }
            (Some(cur), true) if cur != best_idx => {
                events.push(StreamEvent::LockTransferred {
                    from: self.centroids[cur].segment_id.clone(),
                    to: self.centroids[best_idx].segment_id.clone(),
                    score: best,
                });
                self.locked = Some(best_idx);
            }
            _ => {} // continuing lock or still unrecognized — silence
        }
        events
    }

    pub fn current_lock(&self) -> Option<&Centroid> {
        self.locked.map(|i| &self.centroids[i])
    }
}

/// Segment a chronological cue stream into episodes: boundary on wing
/// change or a time gap above `gap_minutes`.
pub fn segment_stream(
    items: &[(Cue, String, i64)], // (cue, wing, epoch_seconds)
    gap_minutes: i64,
    min_len: usize,
    max_len: usize,
) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut current: Vec<Cue> = Vec::new();
    let mut current_wing = String::new();
    let mut last_ts = 0i64;
    let mut seq = 0usize;

    let flush = |cues: &mut Vec<Cue>, wing: &str, seq: &mut usize, out: &mut Vec<Segment>| {
        if cues.len() >= min_len {
            for chunk in cues.chunks(max_len) {
                out.push(Segment {
                    id: format!("seg-{:04}-{wing}", *seq),
                    cues: chunk.to_vec(),
                    wing: wing.to_string(),
                });
                *seq += 1;
            }
        }
        cues.clear();
    };

    for (cue, wing, ts) in items {
        let boundary =
            !current.is_empty() && (*wing != current_wing || ts - last_ts > gap_minutes * 60);
        if boundary {
            flush(&mut current, &current_wing, &mut seq, &mut segments);
        }
        current_wing = wing.clone();
        last_ts = *ts;
        current.push(*cue);
    }
    flush(&mut current, &current_wing, &mut seq, &mut segments);
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(wing: &str, hour: u8, peaks: &[&str]) -> Cue {
        make_cue(wing, 2, hour, peaks, 200)
    }

    fn routine() -> Vec<Cue> {
        vec![
            cue("grocery", 9, &["costco", "list", "budget"]),
            cue("grocery", 9, &["costco", "bulk", "split"]),
            cue("grocery", 10, &["receipt", "total", "saved"]),
            cue("grocery", 10, &["neighbor", "split", "payment"]),
        ]
    }

    fn tracker_with_routine() -> StreamTracker {
        let mut t = StreamTracker::new(StreamConfig::default());
        t.enroll_segment(Segment {
            id: "grocery-run".into(),
            cues: routine(),
            wing: "grocery".into(),
        });
        t
    }

    #[test]
    fn replaying_a_known_routine_locks_once() {
        let mut t = tracker_with_routine();
        let mut all_events = Vec::new();
        for c in routine() {
            all_events.extend(t.observe(&c));
        }
        let locks = all_events
            .iter()
            .filter(|e| matches!(e, StreamEvent::LockAcquired { .. }))
            .count();
        assert_eq!(
            locks, 1,
            "one routine replay = exactly one lock: {all_events:?}"
        );
        assert!(
            t.current_lock().is_some(),
            "lock should persist through the routine"
        );
    }

    #[test]
    fn continuing_match_is_silent() {
        let mut t = tracker_with_routine();
        let r = routine();
        // Walk to lock.
        let mut events = Vec::new();
        for c in &r {
            events.extend(t.observe(c));
        }
        let after_lock = events
            .iter()
            .skip_while(|e| !matches!(e, StreamEvent::LockAcquired { .. }))
            .skip(1)
            .count();
        assert_eq!(
            after_lock, 0,
            "no events after lock while matching: {events:?}"
        );
    }

    #[test]
    fn unrelated_stream_never_locks() {
        let mut t = tracker_with_routine();
        for i in 0..20 {
            let c = cue("emailwing", 14, &[&format!("random{i}"), "totally", "new"]);
            let events = t.observe(&c);
            assert!(
                events.is_empty(),
                "unrelated cues must not fire events: {events:?}"
            );
        }
        assert!(t.current_lock().is_none());
    }

    #[test]
    fn diverging_mid_routine_loses_lock_with_hysteresis() {
        let mut t = tracker_with_routine();
        let r = routine();
        let mut events = Vec::new();
        events.extend(t.observe(&r[0]));
        events.extend(t.observe(&r[1]));
        events.extend(t.observe(&r[2]));
        assert!(
            t.current_lock().is_some(),
            "should be locked after 3 matching cues"
        );
        // Diverge for lost_after_misses cues.
        for i in 0..3 {
            events.extend(t.observe(&cue("otherwing", 22, &[&format!("div{i}")])));
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::LockLost { .. })),
            "sustained divergence must lose the lock: {events:?}"
        );
        assert!(t.current_lock().is_none());
    }

    #[test]
    fn single_odd_cue_does_not_lose_lock() {
        let mut t = tracker_with_routine();
        let r = routine();
        t.observe(&r[0]);
        t.observe(&r[1]);
        t.observe(&r[2]);
        assert!(t.current_lock().is_some());
        // One interruption (a Slack ping mid-routine) — hysteresis holds.
        let events = t.observe(&cue("comms", 10, &["slack", "ping"]));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, StreamEvent::LockLost { .. })),
            "one odd cue must not break the lock (hysteresis)"
        );
        assert!(t.current_lock().is_some());
    }

    #[test]
    fn segmentation_splits_on_wing_and_gap() {
        let base = 1_700_000_000i64;
        let items: Vec<(Cue, String, i64)> = vec![
            (cue("a", 9, &["x1"]), "a".into(), base),
            (cue("a", 9, &["x2"]), "a".into(), base + 60),
            (cue("a", 9, &["x3"]), "a".into(), base + 120),
            // wing change
            (cue("b", 9, &["y1"]), "b".into(), base + 180),
            (cue("b", 9, &["y2"]), "b".into(), base + 240),
            // 2h gap
            (cue("b", 11, &["y3"]), "b".into(), base + 7500),
            (cue("b", 11, &["y4"]), "b".into(), base + 7560),
        ];
        let segs = segment_stream(&items, 45, 2, 32);
        assert_eq!(segs.len(), 3, "wing change + gap = 3 segments: {segs:?}");
        assert_eq!(segs[0].cues.len(), 3);
        assert_eq!(segs[0].wing, "a");
    }
}
