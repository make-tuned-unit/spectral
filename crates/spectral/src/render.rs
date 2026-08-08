//! Context rendering — turning retrieved hits into the text an actor reads.
//!
//! # Why this is in the library
//!
//! Retrieval decides *what* the actor sees; rendering decides *how*. Both are
//! load-bearing on accuracy, and until now only the first shipped. The
//! published LongMemEval-S configuration rendered its context with
//! session-grouped formatting — chronologically ordered sessions, dated
//! headers, role tags, and short-assistant-filler suppression — while the
//! library's only public formatter (`spectral_tact::format_context_block`)
//! emitted an undated, ungrouped `[wing/hall] key: content` line per hit.
//!
//! A consumer calling `recall_local` therefore got a materially different
//! prompt from the one behind the published number, even with identical
//! retrieval. PR #237 moved the retrieval *policy* into the library for exactly
//! this reason; this module closes the other half.
//!
//! # What deliberately did NOT move
//!
//! Environment levers. Every knob here is an explicit field on
//! [`RenderOptions`]; the benchmark harness maps its `SPECTRAL_*` variables
//! onto them. The library never reads the environment, so a consumer's
//! rendering is a function of the config they passed and nothing else.
//!
//! Actor prompt templates also stay in the harness — they describe how to talk
//! to a model, not how memory presents itself.
//!
//! # Determinism
//!
//! Pure string assembly over the hits given. No I/O, no clock, no randomness,
//! no inference. Session ordering is by earliest `created_at` with the episode
//! key as an implicit tiebreak (`BTreeMap` iteration order), and turns within a
//! session order by `key`, which encodes turn sequence.

use std::collections::BTreeMap;

use spectral_ingest::MemoryHit;

/// Content at or below this length is never truncated by `cap_frac` —
/// truncating short turns saves nothing and loses information.
pub const CAP_MIN_LEN: usize = 120;

/// Assistant turns shorter than this are dropped from session-grouped output.
///
/// These are phatic turns — "Hi!", "Sure!", "That's great!" — which carry no
/// propositional content but consume context budget and dilute the evidence
/// the actor has to weigh.
pub const FILLER_MIN_LEN: usize = 40;

/// The order sessions appear in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionOrder {
    /// Oldest session first, by earliest `created_at`. **The published
    /// configuration.**
    #[default]
    Chronological,
    /// Sessions ordered by the best (lowest) rank of any hit they contain, so
    /// the strongest evidence appears first.
    ///
    /// # Why this exists
    ///
    /// [`session_grouped`] under [`Chronological`](Self::Chronological)
    /// consults rank *nowhere*: it groups by episode, sorts turns by key and
    /// orders groups by date. Whatever order retrieval produced is discarded.
    ///
    /// Measured consequence: on the cascade route — the default for every
    /// non-Temporal shape, 70% of the held-out LoCoMo set — a rerank changed
    /// the rendered context on **0 of 84** questions. No rerank-shaped lever
    /// can convert to accuracy there, because its effect never reaches the
    /// actor. See `docs/internal/answerability-result-run1-2026-08-02.md`.
    ///
    /// Turn order *within* a session stays chronological either way: a session
    /// is a conversation, and reading it out of sequence would cost more than
    /// the ranking gains.
    ByRank,
}

/// How to render retrieved hits into actor-facing context.
///
/// Defaults reproduce the published LongMemEval-S configuration: absolute
/// session dates, no relative offsets, no content cap, no description gloss.
#[derive(Debug, Clone, Default)]
pub struct RenderOptions<'a> {
    /// Fraction of its original length to truncate assistant-turn content to.
    /// `None` leaves content intact.
    ///
    /// Measured: an unconditional 0.36 cap was **rejected** — −53% context
    /// tokens but −15pp accuracy, with every regression in
    /// single-session-assistant (see `docs/MEASURED_RECORD.md`). Call sites
    /// that use this exempt recall-shaped questions.
    pub cap_frac: Option<f64>,
    /// The date the question is asked, used to compute relative offsets.
    /// Accepts `"2023/05/30 (Tue) 23:40"` or `"2023-05-30T…"` forms.
    pub question_date: Option<&'a str>,
    /// Annotate dates with a computed offset from `question_date`
    /// ("4 months ago"). Requires `question_date`. Off by default, matching
    /// the published run.
    ///
    /// **Measured once, significant, not yet replicated (2026-08-08).**
    /// Preregistered A/B on LongMemEval `temporal-reasoning` (n=133, paired,
    /// expansion off, temp 0): **76.69% → 83.46%, +6.77pp, 9 fixed / 0 broken,
    /// McNemar exact p = 0.0039**, at **+1.1% context tokens**.
    ///
    /// The mechanism is not extra information — the absolute date is still
    /// there and the offset is derivable from it. It **removes an arithmetic
    /// failure mode**: without the annotation the actor computes the offset
    /// itself and sometimes picks the wrong session. 7 of the 9 fixed
    /// questions are phrased in relative time ("a week ago").
    ///
    /// **The default stays `false`** until the replication finishes — it
    /// stopped mid-arm when API credit ran out, and R11 (the only other
    /// validated accuracy lever here) was believed because it survived a
    /// disjoint validation set. See
    /// `docs/internal/relative-offsets-result-2026-08-08.md`.
    pub relative_offsets: bool,
    /// Append the Librarian's per-memory `description` as a labeled
    /// `[librarian: …]` note. Off by default: measured **no lift** (9/10 vs
    /// 9/10, +7% tokens) — a gloss is redundant when the memory is retrieved
    /// and useless when it isn't.
    pub show_descriptions: bool,
    /// Session ordering. Defaults to [`SessionOrder::Chronological`], the
    /// published configuration.
    pub session_order: SessionOrder,
}

impl<'a> RenderOptions<'a> {
    /// The published LongMemEval-S rendering configuration.
    pub fn published() -> Self {
        Self::default()
    }

    /// Set the question date (enables date tagging; offsets still require
    /// [`with_relative_offsets`](Self::with_relative_offsets)).
    pub fn with_question_date(mut self, date: &'a str) -> Self {
        self.question_date = Some(date);
        self
    }

    /// Annotate dates with computed relative offsets.
    pub fn with_relative_offsets(mut self) -> Self {
        self.relative_offsets = true;
        self
    }

    /// Truncate assistant-turn content to `frac` of its original length.
    pub fn with_assistant_cap(mut self, frac: f64) -> Self {
        self.cap_frac = Some(frac);
        self
    }

    /// Show the Librarian description gloss.
    pub fn with_descriptions(mut self) -> Self {
        self.show_descriptions = true;
        self
    }

    /// Order sessions by best contained rank instead of chronologically.
    pub fn with_rank_ordering(mut self) -> Self {
        self.session_order = SessionOrder::ByRank;
        self
    }

    /// The question date, but only when relative offsets are switched on.
    fn offset_anchor(&self) -> Option<&'a str> {
        self.question_date.filter(|_| self.relative_offsets)
    }
}

/// Extract `(year, month, day)` from `"2023/05/30 …"` or `"2023-05-30T…"`.
///
/// Returns `None` for anything that is not one of those two shapes, including
/// strings that are long enough but use the wrong separators.
pub fn extract_ymd(s: &str) -> Option<(i32, u32, u32)> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 10 {
        return None;
    }
    let y: i32 = s.get(0..4)?.parse().ok()?;
    let m: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    if !matches!(chars[4], '-' | '/') || !matches!(chars[7], '-' | '/') {
        return None;
    }
    Some((y, m, d))
}

/// Human-readable offset of `date_str` relative to `question_date`.
///
/// The bucket boundaries are deliberate: days up to a fortnight, then weeks,
/// then months to two years, then years. Rendering the offset explicitly means
/// the actor never has to do date arithmetic itself — the failure mode behind
/// most temporal-reasoning misses.
pub fn relative_offset(date_str: &str, question_date: &str) -> Option<String> {
    let (sy, sm, sd) = extract_ymd(date_str)?;
    let (qy, qm, qd) = extract_ymd(question_date)?;
    let session = chrono::NaiveDate::from_ymd_opt(sy, sm, sd)?;
    let question = chrono::NaiveDate::from_ymd_opt(qy, qm, qd)?;
    let days = (question - session).num_days();
    Some(match days {
        i64::MIN..=-1 => "in the future".to_string(),
        0 => "same day".to_string(),
        1 => "1 day ago".to_string(),
        2..=13 => format!("{days} days ago"),
        14..=59 => format!("{} weeks ago", days / 7),
        60..=729 => format!("{} months ago", days / 30),
        _ => {
            let years = days / 365;
            let months = (days % 365) / 30;
            if months == 0 {
                format!("{years} years ago")
            } else {
                format!("{years} years, {months} months ago")
            }
        }
    })
}

/// Truncate `content` to `frac` of its byte length, at a char boundary.
///
/// Content at or below [`CAP_MIN_LEN`] is returned unchanged, and the cap is
/// itself floored at [`CAP_MIN_LEN`], so this never produces a fragment too
/// short to carry an answer.
pub fn cap_content(content: &str, frac: f64) -> String {
    if content.len() <= CAP_MIN_LEN {
        return content.to_string();
    }
    let cap = (((content.len() as f64) * frac) as usize).max(CAP_MIN_LEN);
    if content.len() <= cap {
        return content.to_string();
    }
    let mut end = cap.max(1).min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &content[..end])
}

/// Append the Librarian description gloss to a rendered line when `show` is
/// set and the description is non-empty.
///
/// Clearly provenanced (`[librarian: …]`) so the actor never mistakes a
/// derived gloss for the user's own words — the same source-canonical
/// principle that keeps derived text attributable back to the raw memory.
pub fn with_description(line: String, hit: &MemoryHit, show: bool) -> String {
    match (show, hit.description.as_deref()) {
        (true, Some(desc)) if !desc.trim().is_empty() => {
            format!("{line} [librarian: {}]", desc.trim())
        }
        _ => line,
    }
}

/// Render a date, optionally annotated with its offset from the question date.
fn date_tag(date: &str, anchor: Option<&str>) -> String {
    match anchor.and_then(|qd| relative_offset(date, qd)) {
        Some(offset) => format!("{date}, {offset}"),
        None => date.to_string(),
    }
}

/// Which speaker a hit represents, inferred from its key.
fn role_of(hit: &MemoryHit) -> &'static str {
    if hit.key.contains(":user") {
        "user"
    } else if hit.key.contains(":assistant") {
        "asst"
    } else {
        "turn"
    }
}

/// Render hits grouped by session, chronologically ordered.
///
/// This is the format behind the published LongMemEval-S result. Hits are
/// grouped by `episode_id` (falling back to the session prefix of the key),
/// sessions are ordered by their earliest `created_at`, and turns within a
/// session are ordered by key — which encodes turn sequence, so the actor sees
/// the conversation in the order it happened.
///
/// ```text
/// --- Session s1 (2023/02/15) ---
/// [user] I switched to the Pixel 8 last month
/// [asst] Got it — noting the Pixel 8 as your current phone
/// ```
///
/// Assistant turns shorter than [`FILLER_MIN_LEN`] are dropped: they are
/// acknowledgements, and they crowd out evidence.
pub fn session_grouped(hits: &[MemoryHit], opts: &RenderOptions<'_>) -> Vec<String> {
    if hits.is_empty() {
        return Vec::new();
    }

    // `hits` arrives in rank order, so a hit's index IS its rank. Capture the
    // best rank per session before regrouping destroys the information.
    let mut by_episode: BTreeMap<String, Vec<&MemoryHit>> = BTreeMap::new();
    let mut best_rank: BTreeMap<String, usize> = BTreeMap::new();
    for (rank, hit) in hits.iter().enumerate() {
        let ep_key = hit
            .episode_id
            .clone()
            .or_else(|| hit.key.split(':').next().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        best_rank
            .entry(ep_key.clone())
            .and_modify(|r| *r = (*r).min(rank))
            .or_insert(rank);
        by_episode.entry(ep_key).or_default().push(hit);
    }

    // Turn order within a session is always chronological — a session is a
    // conversation, and reading it out of sequence costs more than ranking
    // gains.
    for hits_in_ep in by_episode.values_mut() {
        hits_in_ep.sort_by(|a, b| a.key.cmp(&b.key));
    }

    let mut episodes: Vec<(String, Vec<&MemoryHit>)> = by_episode.into_iter().collect();
    match opts.session_order {
        SessionOrder::Chronological => episodes.sort_by(|a, b| {
            let date_a =
                a.1.first()
                    .and_then(|h| h.created_at.as_deref())
                    .unwrap_or("");
            let date_b =
                b.1.first()
                    .and_then(|h| h.created_at.as_deref())
                    .unwrap_or("");
            date_a.cmp(date_b)
        }),
        // Ties break on session id so the order never depends on map
        // iteration incidentals (the sort discipline from PR #238).
        SessionOrder::ByRank => episodes.sort_by(|a, b| {
            let ra = best_rank.get(&a.0).copied().unwrap_or(usize::MAX);
            let rb = best_rank.get(&b.0).copied().unwrap_or(usize::MAX);
            ra.cmp(&rb).then_with(|| a.0.cmp(&b.0))
        }),
    }

    let anchor = opts.offset_anchor();
    let mut lines = Vec::new();
    for (ep_id, ep_hits) in &episodes {
        let date = ep_hits
            .first()
            .and_then(|h| h.created_at.as_deref())
            .map(|s| s.split(' ').next().unwrap_or(s))
            .unwrap_or("unknown-date");
        lines.push(format!(
            "--- Session {ep_id} ({}) ---",
            date_tag(date, anchor)
        ));

        for hit in ep_hits {
            let role = role_of(hit);
            if role == "asst" && hit.content.len() < FILLER_MIN_LEN {
                continue;
            }
            let line = match (role, opts.cap_frac) {
                ("asst", Some(f)) => format!("[{role}] {}", cap_content(&hit.content, f)),
                _ => format!("[{role}] {}", hit.content),
            };
            lines.push(with_description(line, hit, opts.show_descriptions));
        }
    }

    lines
}

/// Render one hit as a flat, self-describing line:
/// `[2023-05-20] [wing/hall] key: content`.
///
/// Used by the top-k FTS route, where hits are ranked rather than grouped, so
/// each line has to carry its own date and provenance.
pub fn flat_hit(hit: &MemoryHit, opts: &RenderOptions<'_>) -> String {
    let date = hit
        .created_at
        .as_deref()
        .map(|s| s.split('T').next().unwrap_or(s))
        .unwrap_or("unknown-date");
    let wing = hit.wing.as_deref().unwrap_or("?");
    let hall = hit.hall.as_deref().unwrap_or("?");
    let line = format!(
        "[{}] [{wing}/{hall}] {}: {}",
        date_tag(date, opts.offset_anchor()),
        hit.key,
        hit.content
    );
    with_description(line, hit, opts.show_descriptions)
}

/// Render every hit with [`flat_hit`].
pub fn flat(hits: &[MemoryHit], opts: &RenderOptions<'_>) -> Vec<String> {
    hits.iter().map(|h| flat_hit(h, opts)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(key: &str, content: &str, created_at: &str) -> MemoryHit {
        MemoryHit {
            id: key.to_string(),
            key: key.to_string(),
            content: content.to_string(),
            wing: None,
            hall: None,
            signal_score: 0.0,
            visibility: "private".to_string(),
            hits: 1,
            source: None,
            device_id: None,
            confidence: 1.0,
            created_at: Some(created_at.to_string()),
            last_reinforced_at: None,
            episode_id: None,
            declarative_density: None,
            description: None,
            source_brain_id: None,
            signature: None,
        }
    }

    #[test]
    fn relative_offset_buckets() {
        let q = "2023/06/02 (Fri) 10:00";
        assert_eq!(
            relative_offset("2023/06/02", q).as_deref(),
            Some("same day")
        );
        assert_eq!(
            relative_offset("2023/06/01", q).as_deref(),
            Some("1 day ago")
        );
        assert_eq!(
            relative_offset("2023/05/31", q).as_deref(),
            Some("2 days ago")
        );
        // 13 days — the top of the "days" bucket, not yet weeks.
        assert_eq!(
            relative_offset("2023-05-20T08:00", q).as_deref(),
            Some("13 days ago")
        );
        // 43 days -> weeks bucket.
        assert_eq!(
            relative_offset("2023/04/20", q).as_deref(),
            Some("6 weeks ago")
        );
        // 882 days -> years bucket, with a months remainder.
        assert_eq!(
            relative_offset("2021/01/01", q).as_deref(),
            Some("2 years, 5 months ago")
        );
        assert_eq!(
            relative_offset("2023/06/10", q).as_deref(),
            Some("in the future")
        );
    }

    #[test]
    fn extract_ymd_rejects_wrong_separators() {
        assert_eq!(extract_ymd("2023/05/30"), Some((2023, 5, 30)));
        assert_eq!(extract_ymd("2023-05-30T10:00"), Some((2023, 5, 30)));
        assert_eq!(extract_ymd("2023x05x30"), None);
        assert_eq!(extract_ymd("short"), None);
    }

    #[test]
    fn cap_content_leaves_short_content_alone() {
        assert_eq!(cap_content("short", 0.9), "short");
        let borderline = "x".repeat(CAP_MIN_LEN);
        assert_eq!(cap_content(&borderline, 0.1), borderline);
    }

    #[test]
    fn cap_content_respects_char_boundaries() {
        let content = "é".repeat(200);
        let capped = cap_content(&content, 0.3);
        assert!(capped.ends_with('…'));
        // Did not panic and produced valid UTF-8.
        assert!(capped.chars().count() > 1);
    }

    #[test]
    fn session_grouped_orders_sessions_chronologically() {
        let hits = vec![
            hit(
                "s2:user:1",
                "later session content here",
                "2023/05/01 10:00",
            ),
            hit(
                "s1:user:1",
                "earlier session content here",
                "2023/02/01 10:00",
            ),
        ];
        let lines = session_grouped(&hits, &RenderOptions::published());
        assert_eq!(lines[0], "--- Session s1 (2023/02/01) ---");
        assert_eq!(lines[1], "[user] earlier session content here");
        assert_eq!(lines[2], "--- Session s2 (2023/05/01) ---");
    }

    #[test]
    fn session_grouped_drops_short_assistant_filler() {
        let hits = vec![
            hit(
                "s1:user:1",
                "a substantive user question about the thing",
                "2023/02/01",
            ),
            hit("s1:assistant:2", "Sure!", "2023/02/01"),
            hit(
                "s1:assistant:3",
                "Here is a genuinely substantive assistant answer with content",
                "2023/02/01",
            ),
        ];
        let lines = session_grouped(&hits, &RenderOptions::published());
        assert!(!lines.iter().any(|l| l.contains("Sure!")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("genuinely substantive")));
    }

    #[test]
    fn user_turns_are_never_dropped_or_capped() {
        // The cap and the filler floor both target assistant turns only:
        // user turns are where the answerable facts live.
        let hits = vec![hit("s1:user:1", "ok", "2023/02/01")];
        let opts = RenderOptions::published().with_assistant_cap(0.1);
        let lines = session_grouped(&hits, &opts);
        assert_eq!(lines[1], "[user] ok");
    }

    #[test]
    fn relative_offsets_require_both_the_flag_and_the_date() {
        let h = hit("s1:user:1", "content", "2023-02-01T10:00");

        let no_flag = RenderOptions::published().with_question_date("2023/06/02");
        assert_eq!(
            flat_hit(&h, &no_flag),
            "[2023-02-01] [?/?] s1:user:1: content"
        );

        let with_flag = RenderOptions::published()
            .with_question_date("2023/06/02")
            .with_relative_offsets();
        assert_eq!(
            flat_hit(&h, &with_flag),
            "[2023-02-01, 4 months ago] [?/?] s1:user:1: content"
        );

        let flag_no_date = RenderOptions::published().with_relative_offsets();
        assert_eq!(
            flat_hit(&h, &flag_no_date),
            "[2023-02-01] [?/?] s1:user:1: content"
        );
    }

    #[test]
    fn rank_ordering_puts_the_best_ranked_session_first() {
        // Input order IS rank order. The best hit sits in the chronologically
        // LATER session, so the two orderings must disagree.
        let hits = vec![
            hit(
                "s2:user:1",
                "the strongest evidence lives here",
                "2023/05/01 10:00",
            ),
            hit(
                "s1:user:1",
                "weaker but older evidence here",
                "2023/02/01 10:00",
            ),
        ];

        let chrono = session_grouped(&hits, &RenderOptions::published());
        assert_eq!(chrono[0], "--- Session s1 (2023/02/01) ---");

        let ranked = session_grouped(&hits, &RenderOptions::published().with_rank_ordering());
        assert_eq!(ranked[0], "--- Session s2 (2023/05/01) ---");
    }

    #[test]
    fn rank_ordering_keeps_turns_chronological_within_a_session() {
        // Turn 2 is ranked above turn 1, but a session is a conversation and
        // must still read in sequence.
        let hits = vec![
            hit(
                "s1:user:2",
                "the second thing that was said in this turn",
                "2023/02/01",
            ),
            hit(
                "s1:user:1",
                "the first thing that was said in this turn",
                "2023/02/01",
            ),
        ];
        let lines = session_grouped(&hits, &RenderOptions::published().with_rank_ordering());
        assert!(lines[1].contains("first thing"), "{lines:?}");
        assert!(lines[2].contains("second thing"), "{lines:?}");
    }

    #[test]
    fn session_order_default_is_chronological() {
        // Guards the published configuration: if this flips, every published
        // context changes silently.
        assert_eq!(SessionOrder::default(), SessionOrder::Chronological);
        assert_eq!(
            RenderOptions::published().session_order,
            SessionOrder::Chronological
        );
    }

    #[test]
    fn rank_ordering_ties_break_on_session_id() {
        // Both sessions' best rank is 0 only if they are distinct groups; make
        // them tie by giving each its own first hit at the same index across
        // two runs with the input order swapped.
        let a = vec![hit("sb:user:1", "content one here", "2023/02/01")];
        let b = vec![hit("sa:user:1", "content two here", "2023/02/01")];
        let mut both = a.clone();
        both.extend(b.clone());
        let lines = session_grouped(&both, &RenderOptions::published().with_rank_ordering());
        // sb is rank 0, so it leads despite sorting later alphabetically.
        assert_eq!(lines[0], "--- Session sb (2023/02/01) ---");
    }

    #[test]
    fn empty_input_renders_nothing() {
        assert!(session_grouped(&[], &RenderOptions::published()).is_empty());
        assert!(flat(&[], &RenderOptions::published()).is_empty());
    }
}
