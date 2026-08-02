//! Deterministic resolution of relative date expressions against an anchor.
//!
//! # Why this is in the library
//!
//! Conversational memory is full of relative time — "yesterday", "last month",
//! "the Friday before" — and the absolute date is knowable, because every
//! memory carries `created_at`. Nothing resolved it. The benchmark harness
//! computes offsets *between* two dates (session vs question) for display; no
//! component ever resolved a phrase *inside* content into a date.
//!
//! Measured consequence on the LoCoMo held-out runs: a cluster of failures
//! where all required evidence was retrieved and the actor still did the
//! arithmetic wrong —
//!
//! * context says on 2023-07-03 that a crash happened "yesterday"; the actor
//!   answered July 3 instead of July 2
//! * "the Friday before Sunday 2022-01-23" was answered as Jan 14, not Jan 21
//! * a project "wrapped up last month" said on June 6 was answered as June
//!
//! # What this is and is NOT
//!
//! A **deterministic primitive**: precompiled regexes plus calendar
//! arithmetic. No LLM, no inference, no I/O. Same text and anchor, same result.
//!
//! It is **not** wired into recall or context assembly, and no accuracy claim
//! is attached. Retrieval-side improvements have failed to convert to accuracy
//! three times here (ACR, K=60→80, cascade-`k`), so this ships on *resolution
//! correctness*, which is directly testable, and any end-to-end use must be
//! measured separately.
//!
//! # Coverage is partial, and deliberately so
//!
//! Measured against the LoCoMo corpus (5,882 unique turns, 531 concrete
//! calendar expressions), the modelled forms cover roughly three quarters.
//! Genuinely vague expressions — "recently", "lately", "soon", "the other
//! day", "a few years back" — are **not** modelled and produce nothing. A
//! resolver that guessed at those would manufacture false precision, which is
//! worse on a memory corpus than silence.
//!
//! # Ambiguity is reported, never guessed
//!
//! "last Tuesday" on a Sunday can mean 5 days back or 12. Such cases resolve
//! to the nearest prior occurrence and are flagged [`Certainty::Ambiguous`].
//! Month and year offsets are also `Ambiguous`: they name a *period*, and the
//! day-of-month is carried from the anchor by convention — worse, it may be
//! clamped ("3 months ago" from 31 March lands on 31 December only because
//! February has no 31st).
//!
//! # Longest match wins
//!
//! Matches are collected with their source offsets, sorted by position, and
//! overlaps resolved longest-first. This is load-bearing: "day before
//! yesterday" contains "yesterday", and resolving the inner match would return
//! a confidently wrong date one day off.

use std::sync::OnceLock;

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use regex::Regex;

/// How confident the resolution is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Certainty {
    /// Calendar arithmetic with exactly one correct answer ("yesterday",
    /// "3 days ago", "the Friday before").
    Exact,
    /// More than one reading is defensible, or the phrase names a period
    /// rather than a day ("last Tuesday", "last month", "last weekend").
    Ambiguous,
}

/// One relative expression, resolved against the anchor date.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedDate {
    /// The matched source phrase, lowercased.
    pub phrase: String,
    /// Byte offset of the match in the lowercased input.
    pub offset: usize,
    /// The resolved calendar date.
    pub resolved: NaiveDate,
    /// Whether the resolution is unambiguous.
    pub certainty: Certainty,
}

fn re(slot: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    slot.get_or_init(|| Regex::new(pattern).expect("static pattern must compile"))
}

fn weekday_from(name: &str) -> Option<Weekday> {
    Some(match name {
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" | "tues" => Weekday::Tue,
        "wednesday" | "wed" => Weekday::Wed,
        "thursday" | "thu" | "thurs" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "saturday" | "sat" => Weekday::Sat,
        "sunday" | "sun" => Weekday::Sun,
        _ => return None,
    })
}

const WEEKDAY_ALT: &str = "monday|mon|tuesday|tues|tue|wednesday|wed|thursday|thurs|thu|friday|fri|saturday|sat|sunday|sun";

/// Most recent occurrence of `wd` strictly before `anchor`.
fn previous_weekday(anchor: NaiveDate, wd: Weekday) -> NaiveDate {
    let mut d = anchor - Duration::days(1);
    while d.weekday() != wd {
        d -= Duration::days(1);
    }
    d
}

/// First occurrence of `wd` strictly after `anchor`.
fn next_weekday(anchor: NaiveDate, wd: Weekday) -> NaiveDate {
    let mut d = anchor + Duration::days(1);
    while d.weekday() != wd {
        d += Duration::days(1);
    }
    d
}

/// Shift `anchor` by `months` calendar months, clamping the day of month.
fn shift_months(anchor: NaiveDate, months: i32) -> NaiveDate {
    let total = anchor.year() * 12 + (anchor.month0() as i32) + months;
    let (y, m0) = (total.div_euclid(12), total.rem_euclid(12));
    let m = (m0 + 1) as u32;
    let mut day = anchor.day();
    loop {
        if let Some(d) = NaiveDate::from_ymd_opt(y, m, day) {
            return d;
        }
        day -= 1;
    }
}

struct Hit {
    start: usize,
    end: usize,
    phrase: String,
    resolved: NaiveDate,
    certainty: Certainty,
}

/// Resolve every modelled relative date expression in `text` against `anchor`
/// (normally the memory's own `created_at` date).
///
/// Results are returned **in order of appearance**, with overlapping matches
/// resolved longest-first. Unmodelled phrasing yields nothing.
pub fn resolve_relative_dates(text: &str, anchor: NaiveDate) -> Vec<ResolvedDate> {
    let t = text.to_lowercase();
    let mut hits: Vec<Hit> = Vec::new();

    let mut add = |m: regex::Match<'_>, resolved: NaiveDate, certainty: Certainty| {
        hits.push(Hit {
            start: m.start(),
            end: m.end(),
            phrase: m.as_str().to_string(),
            resolved,
            certainty,
        });
    };

    // ── multi-word day phrases (must out-rank the bare day words) ──
    static R_DBY: OnceLock<Regex> = OnceLock::new();
    for m in re(&R_DBY, r"\b(?:the\s+)?day\s+before\s+yesterday\b").find_iter(&t) {
        add(m, anchor - Duration::days(2), Certainty::Exact);
    }
    static R_DAT: OnceLock<Regex> = OnceLock::new();
    for m in re(&R_DAT, r"\b(?:the\s+)?day\s+after\s+tomorrow\b").find_iter(&t) {
        add(m, anchor + Duration::days(2), Certainty::Exact);
    }

    // ── day words ──────────────────────────────────────────────────
    static R_YEST: OnceLock<Regex> = OnceLock::new();
    for m in re(&R_YEST, r"\byesterday\b").find_iter(&t) {
        add(m, anchor - Duration::days(1), Certainty::Exact);
    }
    static R_TODAY: OnceLock<Regex> = OnceLock::new();
    for m in re(&R_TODAY, r"\btoday\b").find_iter(&t) {
        add(m, anchor, Certainty::Exact);
    }
    static R_TMRW: OnceLock<Regex> = OnceLock::new();
    for m in re(&R_TMRW, r"\btomorrow\b").find_iter(&t) {
        add(m, anchor + Duration::days(1), Certainty::Exact);
    }
    // "last night" is the previous calendar day; "tonight"/"this evening" today.
    static R_LNIGHT: OnceLock<Regex> = OnceLock::new();
    for m in re(&R_LNIGHT, r"\blast\s+night\b").find_iter(&t) {
        add(m, anchor - Duration::days(1), Certainty::Exact);
    }
    static R_TONIGHT: OnceLock<Regex> = OnceLock::new();
    for m in re(&R_TONIGHT, r"\b(?:tonight|this\s+evening)\b").find_iter(&t) {
        add(m, anchor, Certainty::Exact);
    }

    // ── "N days/weeks/months/years ago" ────────────────────────────
    static R_AGO: OnceLock<Regex> = OnceLock::new();
    let ago = re(
        &R_AGO,
        r"\b(\d{1,3}|a|an|one|two|three|four|five|six|seven|eight|nine|ten)\s+(day|week|month|year)s?\s+ago\b",
    );
    for c in ago.captures_iter(&t) {
        let whole = c.get(0).unwrap();
        let n: i64 = match &c[1] {
            "a" | "an" | "one" => 1,
            "two" => 2,
            "three" => 3,
            "four" => 4,
            "five" => 5,
            "six" => 6,
            "seven" => 7,
            "eight" => 8,
            "nine" => 9,
            "ten" => 10,
            d => match d.parse() {
                Ok(v) => v,
                Err(_) => continue,
            },
        };
        let (resolved, certainty) = match &c[2] {
            "day" => (anchor - Duration::days(n), Certainty::Exact),
            "week" => (anchor - Duration::weeks(n), Certainty::Exact),
            // Month/year offsets name a period and may clamp the day-of-month,
            // so the exact day is a convention, not a fact.
            "month" => (shift_months(anchor, -(n as i32)), Certainty::Ambiguous),
            _ => (shift_months(anchor, -(n as i32) * 12), Certainty::Ambiguous),
        };
        add(whole, resolved, certainty);
    }

    // ── weekday phrases ────────────────────────────────────────────
    static R_WD_BEFORE: OnceLock<Regex> = OnceLock::new();
    let wd_before = re(
        &R_WD_BEFORE,
        &format!(r"\bthe\s+({WEEKDAY_ALT})\s+before\b"),
    );
    for c in wd_before.captures_iter(&t) {
        if let Some(wd) = weekday_from(&c[1]) {
            add(
                c.get(0).unwrap(),
                previous_weekday(anchor, wd),
                Certainty::Exact,
            );
        }
    }
    static R_WD_LAST: OnceLock<Regex> = OnceLock::new();
    let wd_last = re(&R_WD_LAST, &format!(r"\b(?:last|past)\s+({WEEKDAY_ALT})\b"));
    for c in wd_last.captures_iter(&t) {
        if let Some(wd) = weekday_from(&c[1]) {
            add(
                c.get(0).unwrap(),
                previous_weekday(anchor, wd),
                Certainty::Ambiguous,
            );
        }
    }
    static R_WD_NEXT: OnceLock<Regex> = OnceLock::new();
    let wd_next = re(
        &R_WD_NEXT,
        &format!(r"\b(?:next|this\s+coming)\s+({WEEKDAY_ALT})\b"),
    );
    for c in wd_next.captures_iter(&t) {
        if let Some(wd) = weekday_from(&c[1]) {
            add(
                c.get(0).unwrap(),
                next_weekday(anchor, wd),
                Certainty::Ambiguous,
            );
        }
    }

    // ── weekends and period words ──────────────────────────────────
    // A weekend spans two days; the Saturday is reported and flagged.
    static R_LWKND: OnceLock<Regex> = OnceLock::new();
    for m in re(&R_LWKND, r"\b(?:last|past)\s+weekend\b").find_iter(&t) {
        add(
            m,
            previous_weekday(anchor, Weekday::Sat),
            Certainty::Ambiguous,
        );
    }
    static R_TWKND: OnceLock<Regex> = OnceLock::new();
    for m in re(&R_TWKND, r"\b(?:this|next)\s+weekend\b").find_iter(&t) {
        add(m, next_weekday(anchor, Weekday::Sat), Certainty::Ambiguous);
    }

    static R_PERIOD: OnceLock<Regex> = OnceLock::new();
    let period = re(&R_PERIOD, r"\b(last|next|this)\s+(week|month|year)\b");
    for c in period.captures_iter(&t) {
        let resolved = match (&c[1], &c[2]) {
            ("last", "week") => anchor - Duration::days(7),
            ("next", "week") => anchor + Duration::days(7),
            ("this", "week") => anchor,
            ("last", "month") => shift_months(anchor, -1),
            ("next", "month") => shift_months(anchor, 1),
            ("this", "month") => anchor,
            ("last", "year") => shift_months(anchor, -12),
            ("next", "year") => shift_months(anchor, 12),
            _ => anchor,
        };
        // Every one of these names a period, not a day.
        add(c.get(0).unwrap(), resolved, Certainty::Ambiguous);
    }

    // ── order by appearance, longest match wins on overlap ─────────
    hits.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| (b.end - b.start).cmp(&(a.end - a.start)))
    });
    let mut out: Vec<ResolvedDate> = Vec::new();
    let mut covered_to = 0usize;
    for h in hits {
        if h.start < covered_to {
            continue; // overlapped by an earlier, longer match
        }
        covered_to = h.end;
        out.push(ResolvedDate {
            phrase: h.phrase,
            offset: h.start,
            resolved: h.resolved,
            certainty: h.certainty,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn only(text: &str, anchor: NaiveDate) -> ResolvedDate {
        let r = resolve_relative_dates(text, anchor);
        assert_eq!(r.len(), 1, "expected exactly one match, got {r:?}");
        r.into_iter().next().unwrap()
    }

    /// `locomo_2_38`: context says on July 3 the crash was "yesterday"; the
    /// actor answered July 3.
    #[test]
    fn locomo_2_38_yesterday_resolves_to_the_prior_day() {
        let r = only("the crash happened yesterday", d(2023, 7, 3));
        assert_eq!(r.resolved, d(2023, 7, 2));
        assert_eq!(r.certainty, Certainty::Exact);
    }

    /// `locomo_3_7`: "the Friday before" Sunday 2022-01-23 is Jan 21.
    #[test]
    fn locomo_3_7_the_friday_before_a_sunday() {
        let anchor = d(2022, 1, 23);
        assert_eq!(anchor.weekday(), Weekday::Sun);
        let r = only("it happened the friday before", anchor);
        assert_eq!(r.resolved, d(2022, 1, 21));
        assert_eq!(r.certainty, Certainty::Exact);
    }

    /// `locomo_7_45`: "wrapped up last month", said 2023-06-06, is May.
    #[test]
    fn locomo_7_45_last_month_lands_in_the_prior_month() {
        let r = only("i wrapped it up last month", d(2023, 6, 6));
        assert_eq!(r.resolved.month(), 5);
        assert_eq!(r.certainty, Certainty::Ambiguous);
    }

    /// REGRESSION: "day before yesterday" contains "yesterday". Resolving the
    /// inner match returns a confidently wrong date one day off — worse than
    /// no match at all. Longest-match-wins must suppress it.
    #[test]
    fn day_before_yesterday_is_not_read_as_yesterday() {
        let r = only("we met the day before yesterday", d(2023, 7, 3));
        assert_eq!(r.resolved, d(2023, 7, 1), "must be anchor-2, not anchor-1");
        assert!(r.phrase.contains("day before yesterday"));
        // And the symmetric case.
        let r2 = only("it lands the day after tomorrow", d(2023, 7, 3));
        assert_eq!(r2.resolved, d(2023, 7, 5));
    }

    /// REGRESSION: results must be in order of appearance. The previous
    /// implementation scanned by pattern class and returned them reordered.
    #[test]
    fn results_are_in_order_of_appearance() {
        let anchor = d(2023, 7, 10);
        let r = resolve_relative_dates("two days ago and then yesterday", anchor);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].resolved, d(2023, 7, 8), "'two days ago' appears first");
        assert_eq!(r[1].resolved, d(2023, 7, 9));
        assert!(r[0].offset < r[1].offset);
    }

    /// Month and year offsets name a period and may clamp — they are not
    /// exact days, and must not claim to be.
    #[test]
    fn month_and_year_offsets_are_ambiguous_not_exact() {
        let anchor = d(2023, 3, 31);
        assert_eq!(only("3 months ago", anchor).certainty, Certainty::Ambiguous);
        assert_eq!(only("a year ago", anchor).certainty, Certainty::Ambiguous);
        // Day and week offsets remain exact.
        assert_eq!(only("3 days ago", anchor).certainty, Certainty::Exact);
        assert_eq!(only("2 weeks ago", anchor).certainty, Certainty::Exact);
    }

    #[test]
    fn month_shift_clamps_short_months() {
        assert_eq!(only("last month", d(2023, 3, 31)).resolved, d(2023, 2, 28));
        assert_eq!(only("last month", d(2024, 3, 31)).resolved, d(2024, 2, 29));
    }

    #[test]
    fn added_conversational_forms_resolve() {
        let anchor = d(2023, 7, 5); // Wednesday
        assert_eq!(only("last night", anchor).resolved, d(2023, 7, 4));
        assert_eq!(only("tonight", anchor).resolved, anchor);
        assert_eq!(only("last weekend", anchor).resolved, d(2023, 7, 1)); // prior Sat
        assert_eq!(only("this weekend", anchor).resolved, d(2023, 7, 8)); // next Sat
        assert_eq!(only("this month", anchor).certainty, Certainty::Ambiguous);
        // Abbreviated weekdays appear in the corpus.
        assert_eq!(only("last fri", anchor).resolved, d(2023, 6, 30));
    }

    #[test]
    fn numeric_and_word_offsets_agree() {
        let anchor = d(2023, 7, 10);
        assert_eq!(only("3 days ago", anchor).resolved, d(2023, 7, 7));
        assert_eq!(only("three days ago", anchor).resolved, d(2023, 7, 7));
        assert_eq!(only("2 weeks ago", anchor).resolved, d(2023, 6, 26));
    }

    #[test]
    fn resolution_is_deterministic() {
        let anchor = d(2023, 7, 3);
        let first = resolve_relative_dates("yesterday and 2 weeks ago", anchor);
        for _ in 0..5 {
            assert_eq!(
                resolve_relative_dates("yesterday and 2 weeks ago", anchor),
                first
            );
        }
        assert_eq!(first.len(), 2);
    }

    /// Vague expressions are NOT modelled. Guessing a date for "recently"
    /// would manufacture false precision — worse than silence on a memory
    /// corpus. These are the highest-frequency vague forms in LoCoMo.
    #[test]
    fn vague_expressions_produce_nothing() {
        let anchor = d(2023, 7, 3);
        for phrase in [
            "recently",
            "lately",
            "soon",
            "the other day",
            "a few years back",
            "a while back",
            "some time last spring",
            "no dates here at all",
        ] {
            assert!(
                resolve_relative_dates(phrase, anchor).is_empty(),
                "{phrase:?} must not resolve"
            );
        }
    }
}
