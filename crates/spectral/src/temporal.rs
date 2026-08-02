//! Deterministic resolution of relative date expressions against an anchor.
//!
//! # Why this is in the library
//!
//! Conversational memories are full of relative time — "yesterday", "last
//! month", "the Friday before". The absolute date is knowable: every memory
//! carries a `created_at`, so "yesterday" in a 2023-07-03 session means
//! 2023-07-02. But nothing resolved it. The benchmark harness computes offsets
//! *between* two dates (session vs question) for display; no component ever
//! resolved a phrase *inside* content into a date.
//!
//! Measured consequence on the LoCoMo held-out runs: a cluster of failures
//! where all required evidence was retrieved and the actor still got the date
//! wrong by doing the arithmetic itself —
//!
//! * context says on 2023-07-03 that a crash happened "yesterday"; the actor
//!   answered July 3 instead of July 2
//! * "the Friday before Sunday 2022-01-23" was answered as Jan 14, not Jan 21
//! * a project "wrapped up last month" said on June 6 was answered as June
//!
//! See `docs/internal/locomo-k-lever-prereg-2026-08-01.md`.
//!
//! # What this is and is NOT
//!
//! It is a **deterministic primitive**: regex patterns plus calendar
//! arithmetic. No LLM, no inference, no I/O. The same text and anchor always
//! produce the same result.
//!
//! It is **not** wired into recall or context assembly, and no accuracy claim
//! is attached to it. Retrieval-side improvements have failed to convert to
//! accuracy three times in this project (ACR, K=60→80, cascade-`k`), so a
//! capability is shipped here on its own merits — *resolution correctness*,
//! which is directly testable — and any end-to-end use must be measured
//! separately.
//!
//! # Ambiguity is reported, never guessed
//!
//! "last Tuesday" spoken on a Sunday can mean 5 days ago or 12, depending on
//! convention. Such cases resolve to the nearest prior occurrence and are
//! flagged [`Certainty::Ambiguous`]. A caller must be able to tell a derived
//! date from a guessed one.

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use regex::Regex;

/// How confident the resolution is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Certainty {
    /// Calendar arithmetic with exactly one correct answer ("yesterday",
    /// "3 days ago").
    Exact,
    /// More than one reading is defensible; the nearest prior occurrence was
    /// chosen ("last Tuesday", "last month" with no day).
    Ambiguous,
}

/// One relative expression, resolved against the anchor date.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedDate {
    /// The matched source phrase, lowercased.
    pub phrase: String,
    /// The resolved calendar date.
    pub resolved: NaiveDate,
    /// Whether the resolution is unambiguous.
    pub certainty: Certainty,
}

fn weekday_from(name: &str) -> Option<Weekday> {
    Some(match name {
        "monday" => Weekday::Mon,
        "tuesday" => Weekday::Tue,
        "wednesday" => Weekday::Wed,
        "thursday" => Weekday::Thu,
        "friday" => Weekday::Fri,
        "saturday" => Weekday::Sat,
        "sunday" => Weekday::Sun,
        _ => return None,
    })
}

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
    // Clamp: 31 Mar minus one month is 28/29 Feb, not an invalid date.
    let mut day = anchor.day();
    loop {
        if let Some(d) = NaiveDate::from_ymd_opt(y, m, day) {
            return d;
        }
        day -= 1;
    }
}

/// Resolve every recognised relative date expression in `text` against
/// `anchor` (normally the memory's own `created_at` date).
///
/// Returns one entry per match, in order of appearance. Unrecognised phrasing
/// yields nothing — this never guesses at forms it does not model.
pub fn resolve_relative_dates(text: &str, anchor: NaiveDate) -> Vec<ResolvedDate> {
    let t = text.to_lowercase();
    let mut out: Vec<ResolvedDate> = Vec::new();
    let mut push = |phrase: &str, resolved: NaiveDate, certainty: Certainty| {
        out.push(ResolvedDate {
            phrase: phrase.to_string(),
            resolved,
            certainty,
        });
    };

    // ── day words ──────────────────────────────────────────────────
    for (word, delta) in [("yesterday", -1i64), ("today", 0), ("tomorrow", 1)] {
        let re = Regex::new(&format!(r"\b{word}\b")).unwrap();
        for _ in re.find_iter(&t) {
            push(word, anchor + Duration::days(delta), Certainty::Exact);
        }
    }

    // ── "N days/weeks/months/years ago" ────────────────────────────
    let ago = Regex::new(r"\b(\d{1,3}|a|an|one|two|three|four|five|six|seven|eight|nine|ten)\s+(day|week|month|year)s?\s+ago\b").unwrap();
    for c in ago.captures_iter(&t) {
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
            d => d.parse().unwrap_or(0),
        };
        let resolved = match &c[2] {
            "day" => anchor - Duration::days(n),
            "week" => anchor - Duration::weeks(n),
            "month" => shift_months(anchor, -(n as i32)),
            _ => shift_months(anchor, -(n as i32) * 12),
        };
        // Month/year arithmetic on a specific day is exact; the phrase itself
        // is precise even though the referent may be a whole period.
        push(&c[0], resolved, Certainty::Exact);
    }

    // ── "last/next <weekday>" and "the <weekday> before" ───────────
    let wd_last =
        Regex::new(r"\b(last|past)\s+(monday|tuesday|wednesday|thursday|friday|saturday|sunday)\b")
            .unwrap();
    for c in wd_last.captures_iter(&t) {
        if let Some(wd) = weekday_from(&c[2]) {
            // "last Tuesday" said on a Sunday can mean 5 or 12 days back.
            push(&c[0], previous_weekday(anchor, wd), Certainty::Ambiguous);
        }
    }
    let wd_before = Regex::new(
        r"\bthe\s+(monday|tuesday|wednesday|thursday|friday|saturday|sunday)\s+before\b",
    )
    .unwrap();
    for c in wd_before.captures_iter(&t) {
        if let Some(wd) = weekday_from(&c[1]) {
            push(&c[0], previous_weekday(anchor, wd), Certainty::Exact);
        }
    }
    let wd_next = Regex::new(
        r"\b(next|this\s+coming)\s+(monday|tuesday|wednesday|thursday|friday|saturday|sunday)\b",
    )
    .unwrap();
    for c in wd_next.captures_iter(&t) {
        if let Some(wd) = weekday_from(&c[2]) {
            push(&c[0], next_weekday(anchor, wd), Certainty::Ambiguous);
        }
    }

    // ── "last/next week|month|year" ────────────────────────────────
    for (phrase, months, days) in [
        ("last month", -1i32, 0i64),
        ("next month", 1, 0),
        ("last year", -12, 0),
        ("next year", 12, 0),
        ("last week", 0, -7),
        ("next week", 0, 7),
    ] {
        let re = Regex::new(&format!(r"\b{}\b", phrase.replace(' ', r"\s+"))).unwrap();
        for _ in re.find_iter(&t) {
            let resolved = if months != 0 {
                shift_months(anchor, months)
            } else {
                anchor + Duration::days(days)
            };
            // "last month" names a period, not a day — the day-of-month is
            // carried over from the anchor and is a convention, not a fact.
            push(phrase, resolved, Certainty::Ambiguous);
        }
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

    /// The exact failure from LoCoMo `locomo_2_38`: context says on July 3 that
    /// the crash happened "yesterday"; the actor answered July 3.
    #[test]
    fn locomo_2_38_yesterday_resolves_to_the_prior_day() {
        let r = only("the crash happened yesterday", d(2023, 7, 3));
        assert_eq!(r.resolved, d(2023, 7, 2));
        assert_eq!(r.certainty, Certainty::Exact);
    }

    /// `locomo_3_7`: "the Friday before" Sunday 2022-01-23 is Jan 21. The
    /// actor answered Jan 14 — it skipped a week.
    #[test]
    fn locomo_3_7_the_friday_before_a_sunday() {
        let anchor = d(2022, 1, 23);
        assert_eq!(
            anchor.weekday(),
            Weekday::Sun,
            "the reported case is a Sunday"
        );
        let r = only("it happened the friday before", anchor);
        assert_eq!(r.resolved, d(2022, 1, 21));
        assert_eq!(r.certainty, Certainty::Exact);
    }

    /// `locomo_7_45`: "wrapped up last month", said 2023-06-06, is May.
    #[test]
    fn locomo_7_45_last_month_lands_in_the_prior_month() {
        let r = only("i wrapped it up last month", d(2023, 6, 6));
        assert_eq!(r.resolved.month(), 5);
        assert_eq!(r.resolved.year(), 2023);
        assert_eq!(
            r.certainty,
            Certainty::Ambiguous,
            "'last month' names a period, so the day is a convention"
        );
    }

    /// `locomo_8_74`: "last Tuesday" said on Sunday 2023-12-17 resolves to
    /// Dec 12 — and is flagged ambiguous, because Dec 5 is also defensible.
    #[test]
    fn last_weekday_is_nearest_prior_and_flagged_ambiguous() {
        let anchor = d(2023, 12, 17);
        assert_eq!(anchor.weekday(), Weekday::Sun);
        let r = only("that was last tuesday", anchor);
        assert_eq!(r.resolved, d(2023, 12, 12));
        assert_eq!(r.certainty, Certainty::Ambiguous);
    }

    #[test]
    fn numeric_and_word_offsets_agree() {
        let anchor = d(2023, 7, 10);
        assert_eq!(only("3 days ago", anchor).resolved, d(2023, 7, 7));
        assert_eq!(only("three days ago", anchor).resolved, d(2023, 7, 7));
        assert_eq!(only("2 weeks ago", anchor).resolved, d(2023, 6, 26));
        assert_eq!(only("a year ago", anchor).resolved, d(2022, 7, 10));
    }

    /// Month arithmetic must clamp rather than produce an invalid date.
    #[test]
    fn month_shift_clamps_short_months() {
        assert_eq!(only("last month", d(2023, 3, 31)).resolved, d(2023, 2, 28));
        assert_eq!(only("last month", d(2024, 3, 31)).resolved, d(2024, 2, 29));
    }

    /// Determinism: same text and anchor, same answer, always.
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

    /// Unrecognised phrasing yields nothing — this never guesses.
    #[test]
    fn unmodelled_phrasing_produces_no_output() {
        assert!(resolve_relative_dates("a while back", d(2023, 7, 3)).is_empty());
        assert!(resolve_relative_dates("some time last spring", d(2023, 7, 3)).is_empty());
        assert!(resolve_relative_dates("no dates here at all", d(2023, 7, 3)).is_empty());
    }
}
