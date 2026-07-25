//! Forgiving due-date parsing for the task editor.
//!
//! Typing `2026-07-28` works, but so do `today`, `tomorrow`, `fri`, `+3d` and
//! `2w`. An empty string clears the due date. Anything unrecognised is an
//! error, so a typo never silently becomes the wrong date.

use anyhow::{Result, bail};
use jiff::civil::{Date, Weekday};

/// Parse a due-date input relative to `today`.
///
/// Returns `Ok(None)` for empty input, meaning "no due date".
pub fn parse_due(input: &str, today: Date) -> Result<Option<Date>> {
    let raw = input.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return Ok(None);
    }

    // Keywords.
    match raw.as_str() {
        "today" | "tod" => return Ok(Some(today)),
        "tomorrow" | "tom" | "tmr" => return Ok(Some(add_days(today, 1)?)),
        "yesterday" | "yest" => return Ok(Some(add_days(today, -1)?)),
        _ => {}
    }

    // ISO: 2026-07-28
    if let Ok(date) = raw.parse::<Date>() {
        return Ok(Some(date));
    }

    // Offsets: +3d, 3d, -1w, 2w, +1m, 1y
    if let Some(date) = parse_offset(&raw, today)? {
        return Ok(Some(date));
    }

    // Weekday names resolve to the next occurrence, never today.
    if let Some(weekday) = parse_weekday(&raw) {
        return Ok(Some(next_weekday(today, weekday)?));
    }

    bail!(
        "`{}` is not a date. Try 2026-07-28, today, tomorrow, fri, +3d or 2w.",
        input.trim()
    );
}

/// Parse a relative offset such as `+3d`, `2w`, `-1m` or `1y`.
fn parse_offset(raw: &str, today: Date) -> Result<Option<Date>> {
    let body = raw.strip_prefix('+').unwrap_or(raw);
    let (negative, body) = match body.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, body),
    };

    let Some(unit) = body.chars().last() else {
        return Ok(None);
    };
    if !matches!(unit, 'd' | 'w' | 'm' | 'y') {
        return Ok(None);
    }

    let digits = &body[..body.len() - unit.len_utf8()];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Ok(None);
    }

    let Ok(n) = digits.parse::<i32>() else {
        bail!("`{raw}` has too large a number");
    };
    let n = if negative { -n } else { n };

    let result = match unit {
        'd' => add_days(today, n),
        'w' => add_days(today, n.saturating_mul(7)),
        'm' => today
            .checked_add(jiff::Span::new().months(n))
            .map_err(|e| anyhow::anyhow!("{raw} is out of range: {e}")),
        _ => today
            .checked_add(jiff::Span::new().years(n))
            .map_err(|e| anyhow::anyhow!("{raw} is out of range: {e}")),
    }?;
    Ok(Some(result))
}

/// Match full or three-letter weekday names.
fn parse_weekday(raw: &str) -> Option<Weekday> {
    let name = match raw {
        "mon" | "monday" => Weekday::Monday,
        "tue" | "tues" | "tuesday" => Weekday::Tuesday,
        "wed" | "weds" | "wednesday" => Weekday::Wednesday,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thursday,
        "fri" | "friday" => Weekday::Friday,
        "sat" | "saturday" => Weekday::Saturday,
        "sun" | "sunday" => Weekday::Sunday,
        _ => return None,
    };
    Some(name)
}

/// The next occurrence of `target` strictly after `today`.
fn next_weekday(today: Date, target: Weekday) -> Result<Date> {
    let current = today.weekday().to_monday_zero_offset();
    let wanted = target.to_monday_zero_offset();
    let mut delta = i32::from(wanted) - i32::from(current);
    if delta <= 0 {
        delta += 7;
    }
    add_days(today, delta)
}

/// Add whole days, reporting range errors instead of panicking.
fn add_days(date: Date, days: i32) -> Result<Date> {
    date.checked_add(jiff::Span::new().days(days))
        .map_err(|e| anyhow::anyhow!("date is out of range: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    /// 2026-07-25 is a Saturday.
    fn today() -> Date {
        date(2026, 7, 25)
    }

    #[test]
    fn empty_clears_the_due_date() {
        assert_eq!(parse_due("", today()).unwrap(), None);
        assert_eq!(parse_due("   ", today()).unwrap(), None);
    }

    #[test]
    fn iso_dates_parse() {
        assert_eq!(
            parse_due("2026-07-28", today()).unwrap(),
            Some(date(2026, 7, 28))
        );
    }

    #[test]
    fn keywords_resolve_relative_to_today() {
        assert_eq!(parse_due("today", today()).unwrap(), Some(today()));
        assert_eq!(
            parse_due("tomorrow", today()).unwrap(),
            Some(date(2026, 7, 26))
        );
        assert_eq!(
            parse_due("yesterday", today()).unwrap(),
            Some(date(2026, 7, 24))
        );
    }

    #[test]
    fn offsets_parse_with_and_without_a_sign() {
        assert_eq!(parse_due("+3d", today()).unwrap(), Some(date(2026, 7, 28)));
        assert_eq!(parse_due("3d", today()).unwrap(), Some(date(2026, 7, 28)));
        assert_eq!(parse_due("2w", today()).unwrap(), Some(date(2026, 8, 8)));
        assert_eq!(parse_due("-1d", today()).unwrap(), Some(date(2026, 7, 24)));
        assert_eq!(parse_due("1m", today()).unwrap(), Some(date(2026, 8, 25)));
        assert_eq!(parse_due("1y", today()).unwrap(), Some(date(2027, 7, 25)));
    }

    #[test]
    fn offsets_cross_month_and_year_boundaries() {
        assert_eq!(
            parse_due("10d", date(2026, 12, 28)).unwrap(),
            Some(date(2027, 1, 7))
        );
    }

    #[test]
    fn weekdays_resolve_to_the_next_occurrence_never_today() {
        // Today is Saturday; "sat" must mean next Saturday, not today.
        assert_eq!(parse_due("sat", today()).unwrap(), Some(date(2026, 8, 1)));
        assert_eq!(parse_due("sun", today()).unwrap(), Some(date(2026, 7, 26)));
        assert_eq!(
            parse_due("friday", today()).unwrap(),
            Some(date(2026, 7, 31))
        );
    }

    #[test]
    fn input_is_case_insensitive() {
        assert_eq!(
            parse_due("ToMoRRoW", today()).unwrap(),
            Some(date(2026, 7, 26))
        );
        assert_eq!(parse_due("FRI", today()).unwrap(), Some(date(2026, 7, 31)));
    }

    #[test]
    fn garbage_is_rejected_with_examples() {
        let err = parse_due("next thursday-ish", today()).expect_err("must fail");
        let msg = err.to_string();
        assert!(msg.contains("is not a date"), "got: {msg}");
        assert!(
            msg.contains("+3d"),
            "error should show valid forms, got: {msg}"
        );
    }

    #[test]
    fn partial_offsets_are_rejected_rather_than_guessed() {
        assert!(parse_due("d", today()).is_err());
        assert!(parse_due("+d", today()).is_err());
        assert!(parse_due("3x", today()).is_err());
        assert!(parse_due("2026-13-01", today()).is_err());
    }
}
