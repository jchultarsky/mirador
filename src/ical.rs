//! Reading events out of a local `.ics` file.
//!
//! Enough of RFC 5545 to answer "what is next", and no more. mirador does not
//! talk to a calendar server — the agenda panel reads a file you already have,
//! whatever put it there.
//!
//! # Why this is hand-written
//!
//! `icalendar` and `rrule` are good crates, and between them they would add 30
//! transitive dependencies and **2.3 MB** to a 3.4 MB binary — measured, not
//! guessed. Most of that is `chrono-tz` carrying a timezone database, which is
//! the part that decides it: mirador already has one, because `jiff` ships it,
//! and shipping two in a prebuilt binary for four platforms to read a text file
//! is not a trade this project makes. It is the same reasoning that keeps the
//! pomodoro chime on the terminal bell instead of an audio stack.
//!
//! # What is supported
//!
//! `VEVENT` with `SUMMARY`, `LOCATION`, `DTSTART`, and `DTEND` or `DURATION`.
//! Times in all three forms the wild produces: a floating local time, a UTC
//! time ending in `Z`, and a zoned time with `TZID`. All-day events via
//! `VALUE=DATE`. Line folding and the `\n \, \; \\` escapes.
//!
//! Recurrence is a **documented subset** — see [`Recurrence::expand`]. A rule
//! using anything outside it yields only the event's own start, because a
//! calendar that invents a meeting is worse than one that misses it.

use std::collections::BTreeSet;

use jiff::civil::{Date, DateTime, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Zoned};

/// One occurrence of an event, resolved to the local timezone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub summary: String,
    pub location: Option<String>,
    pub start: Zoned,
    /// When it ends. `None` for an event with no end and no duration, which
    /// RFC 5545 says lasts an instant (or, for an all-day event, the day).
    pub end: Option<Zoned>,
    /// Drawn without a time, and sorted before timed events on the same day.
    pub all_day: bool,
}

impl Event {
    /// Whether `now` falls inside this event.
    pub fn contains(&self, now: &Zoned) -> bool {
        if *now < self.start {
            return false;
        }
        match &self.end {
            Some(end) => now < end,
            None => false,
        }
    }
}

/// A parsed calendar: the events, and anything that could not be read.
#[derive(Debug, Default)]
pub struct Calendar {
    pub events: Vec<Event>,
    /// Events that parsed far enough to be counted but not to be shown, with
    /// the reason. Surfaced rather than swallowed: a calendar quietly missing
    /// half its entries is worse than one that says it could not read them.
    pub skipped: Vec<String>,
}

/// Parse `text` and expand recurrences into `[from, until]`.
///
/// The window is required rather than optional because an unbounded `RRULE` has
/// infinitely many occurrences, and the panel only ever draws a few days.
pub fn parse(text: &str, tz: &TimeZone, from: &Zoned, until: &Zoned) -> Calendar {
    let mut calendar = Calendar::default();

    let lines = unfold(text);
    if lines.len() >= MAX_LINES {
        calendar.skipped.push(format!(
            "this calendar has more than {MAX_LINES} lines; only the first \
             {MAX_LINES} were read"
        ));
    }

    for block in vevents(&lines) {
        match event_from(&block, tz) {
            Ok(Some((event, recurrence, exceptions))) => {
                calendar
                    .events
                    .extend(recurrence.expand(&event, &exceptions, from, until));
            }
            // A VEVENT with no DTSTART is not an event; the spec requires one.
            Ok(None) => {}
            Err(why) => calendar.skipped.push(why),
        }
    }

    calendar.events.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| a.summary.cmp(&b.summary))
    });
    calendar
}

/// Most content lines read from one file.
///
/// The agenda panel already refuses a `.ics` over 10MB, and that bound turns out
/// not to bound what matters: the cost here is per *line*, not per byte. Every
/// line becomes a `String` in a `Vec`, and a `String` is 24 bytes before it holds
/// anything — so a 10MB file of bare newlines produced **240MB** of `Vec` before
/// a single event was parsed. Measured, at a flat 24x whatever the size.
///
/// RFC 5545 folds at 75 octets, so a maximally-folded 10MB calendar holds about
/// 138,000 lines and a typical one perhaps 300,000. This admits every real
/// calendar that fits under the byte cap and holds the `Vec` itself to under
/// 10MB. Hitting it is reported rather than swallowed, on the same principle as
/// `Calendar::skipped`: a calendar quietly missing half its entries is worse than
/// one that says it could not read them.
const MAX_LINES: usize = 400_000;

/// Undo RFC 5545 line folding.
///
/// A long line is split with CRLF followed by a space or tab, and the
/// continuation begins at the character *after* that whitespace. Getting this
/// wrong does not fail loudly — it silently truncates every long summary at
/// roughly 75 characters, which looks like the calendar's fault.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        if lines.len() >= MAX_LINES {
            break;
        }
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match line.strip_prefix([' ', '\t']) {
            Some(rest) if !lines.is_empty() => {
                if let Some(last) = lines.last_mut() {
                    last.push_str(rest);
                }
            }
            _ => lines.push(line.to_string()),
        }
    }
    lines
}

/// Split the unfolded lines into `VEVENT` blocks.
///
/// Nested components — a `VALARM` inside a `VEVENT`, most often — are dropped,
/// because a reminder's `TRIGGER` is not the event's start and confusing the
/// two would put every event on screen at the wrong time.
fn vevents(lines: &[String]) -> Vec<Vec<&str>> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    let mut nested = 0usize;

    for line in lines {
        let line = line.as_str();
        if line.eq_ignore_ascii_case("BEGIN:VEVENT") {
            current = Some(Vec::new());
            nested = 0;
            continue;
        }
        let Some(block) = current.as_mut() else {
            continue;
        };
        if line.eq_ignore_ascii_case("END:VEVENT") {
            blocks.push(std::mem::take(block));
            current = None;
            continue;
        }
        if starts_with_ci(line, "BEGIN:") {
            nested += 1;
        } else if starts_with_ci(line, "END:") {
            nested = nested.saturating_sub(1);
        } else if nested == 0 {
            block.push(line);
        }
    }
    blocks
}

/// Case-insensitively, does `line` begin with `prefix` and carry something more?
///
/// Compares **bytes**, which is the whole point. This was written as
/// `line.len() > 6 && line[..6].eq_ignore_ascii_case("BEGIN:")`, and slicing a
/// `&str` at a byte offset panics when that offset falls inside a character —
/// so any line in a `VEVENT` with a multi-byte character straddling byte 4 or 6
/// brought the dashboard down. `日本語日本語` does it, and so does `abcé`.
///
/// A calendar is somebody else's file, and international text in one is not an
/// edge case. The Phase 1 pass over this module tested it at *scale* — two
/// thousand recurring events, twenty thousand nested components — and never
/// varied the alphabet, which is why the property tests at the foot of this file
/// generate content rather than volume.
///
/// `prefix` is ASCII at every call site; the byte comparison is only correct
/// because of that, and there is a test.
fn starts_with_ci(line: &str, prefix: &str) -> bool {
    line.len() > prefix.len()
        && line.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// One `NAME;PARAM=VALUE:VALUE` line.
struct Property<'a> {
    name: &'a str,
    params: Vec<(&'a str, &'a str)>,
    value: &'a str,
}

impl<'a> Property<'a> {
    /// Split a content line.
    ///
    /// The colon that ends the name-and-parameters section is the first one
    /// *outside* a quoted string: a parameter may legitimately contain one, as
    /// in `TZID="GMT+01:00"`, and splitting on the first colon puts half the
    /// timezone into the value.
    fn parse(line: &'a str) -> Option<Self> {
        let mut quoted = false;
        let mut split = None;
        for (index, ch) in line.char_indices() {
            match ch {
                '"' => quoted = !quoted,
                ':' if !quoted => {
                    split = Some(index);
                    break;
                }
                _ => {}
            }
        }
        let split = split?;
        let (head, value) = line.split_at(split);
        let value = &value[1..];

        let mut parts = head.split(';');
        let name = parts.next()?;
        let params = parts
            .filter_map(|p| {
                let (k, v) = p.split_once('=')?;
                Some((k, v.trim_matches('"')))
            })
            .collect();
        Some(Self {
            name,
            params,
            value,
        })
    }

    fn param(&self, key: &str) -> Option<&'a str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| *v)
    }
}

/// Turn a `VEVENT` block into an event, its recurrence rule and its exceptions.
///
/// `Ok(None)` means the block had no `DTSTART`, which RFC 5545 requires — such
/// a block is not an event and is not worth reporting as a failure.
type Parsed = (Event, Recurrence, Vec<Timestamp>);

fn event_from(block: &[&str], tz: &TimeZone) -> Result<Option<Parsed>, String> {
    let mut summary = None;
    let mut location = None;
    let mut start: Option<(Zoned, bool)> = None;
    let mut end: Option<Zoned> = None;
    let mut duration: Option<Span> = None;
    let mut rule = Recurrence::Once;
    let mut exceptions = Vec::new();

    for line in block {
        let Some(property) = Property::parse(line) else {
            continue;
        };
        match property.name.to_ascii_uppercase().as_str() {
            "SUMMARY" => summary = Some(unescape(property.value)),
            "LOCATION" => location = Some(unescape(property.value)),
            "DTSTART" => start = Some(moment(&property, tz)?),
            "DTEND" => end = Some(moment(&property, tz)?.0),
            "DURATION" => duration = parse_duration(property.value),
            "RRULE" => rule = Recurrence::parse(property.value),
            "EXDATE" => {
                for one in property.value.split(',') {
                    let single = Property {
                        name: "EXDATE",
                        params: property.params.clone(),
                        value: one,
                    };
                    if let Ok((moment, _)) = moment(&single, tz) {
                        exceptions.push(moment.timestamp());
                    }
                }
            }
            _ => {}
        }
    }

    let Some((start, all_day)) = start else {
        return Ok(None);
    };

    let end = end.or_else(|| {
        duration
            .and_then(|span| start.checked_add(span).ok())
            // An all-day event with neither DTEND nor DURATION covers its day.
            .or_else(|| all_day.then(|| start.tomorrow()).and_then(Result::ok))
    });

    Ok(Some((
        Event {
            // An event with no SUMMARY is legal and does happen. A dash beats
            // an empty row, which reads as a broken panel.
            summary: summary.unwrap_or_else(|| "–".to_string()),
            location: location.filter(|l| !l.trim().is_empty()),
            start,
            end,
            all_day,
        },
        rule,
        exceptions,
    )))
}

/// Resolve a `DTSTART`-shaped property to a moment, and whether it is all-day.
fn moment(property: &Property<'_>, tz: &TimeZone) -> Result<(Zoned, bool), String> {
    let value = property.value.trim();

    if property
        .param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
        || (value.len() == 8 && !value.contains('T'))
    {
        let day = parse_date(value).ok_or_else(|| format!("unreadable date `{value}`"))?;
        let zoned = day
            .to_datetime(Time::midnight())
            .to_zoned(tz.clone())
            .map_err(|e| format!("`{value}`: {e}"))?;
        return Ok((zoned, true));
    }

    let civil = parse_datetime(value).ok_or_else(|| format!("unreadable time `{value}`"))?;

    // A trailing `Z` means UTC, and beats any TZID — a line carrying both is
    // malformed, and the `Z` is the one with a single meaning.
    if value.ends_with('Z') {
        let stamp: Timestamp = civil
            .to_zoned(TimeZone::UTC)
            .map_err(|e| format!("`{value}`: {e}"))?
            .timestamp();
        return Ok((stamp.to_zoned(tz.clone()), false));
    }

    // A named zone, if we recognise it. An unknown TZID falls back to local
    // rather than failing: the wrong offset is bad, but dropping the event
    // entirely is worse, and most unknown names are Windows-style aliases for
    // roughly the right place.
    let zone = property
        .param("TZID")
        .and_then(|name| TimeZone::get(name).ok())
        .unwrap_or_else(|| tz.clone());

    let zoned = civil
        .to_zoned(zone)
        .map_err(|e| format!("`{value}`: {e}"))?
        .with_time_zone(tz.clone());
    Ok((zoned, false))
}

fn parse_date(value: &str) -> Option<Date> {
    if value.len() < 8 {
        return None;
    }
    let year: i16 = value.get(0..4)?.parse().ok()?;
    let month: i8 = value.get(4..6)?.parse().ok()?;
    let day: i8 = value.get(6..8)?.parse().ok()?;
    Date::new(year, month, day).ok()
}

fn parse_datetime(value: &str) -> Option<DateTime> {
    let day = parse_date(value)?;
    let rest = value.get(8..)?.strip_prefix('T')?;
    let hour: i8 = rest.get(0..2)?.parse().ok()?;
    let minute: i8 = rest.get(2..4)?.parse().ok()?;
    let second: i8 = rest.get(4..6).unwrap_or("00").parse().ok()?;
    // Leap seconds exist in the wild and jiff will not accept :60.
    let second = second.min(59);
    Some(day.at(hour, minute, second, 0))
}

/// Parse an RFC 5545 duration, e.g. `PT1H30M`, `P2D`, `-PT15M`.
fn parse_duration(value: &str) -> Option<Span> {
    let (negative, rest) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let rest = rest.strip_prefix('P')?;

    let mut span = Span::new();
    let mut digits = String::new();
    let mut in_time = false;
    for ch in rest.chars() {
        match ch {
            'T' => in_time = true,
            '0'..='9' => digits.push(ch),
            unit => {
                let n: i64 = digits.parse().ok()?;
                digits.clear();
                span = match (unit, in_time) {
                    ('W', _) => span.try_weeks(n).ok()?,
                    ('D', _) => span.try_days(n).ok()?,
                    ('H', true) => span.try_hours(n).ok()?,
                    ('M', true) => span.try_minutes(n).ok()?,
                    ('S', true) => span.try_seconds(n).ok()?,
                    _ => return None,
                };
            }
        }
    }
    Some(if negative { -span } else { span })
}

/// Undo the four escapes RFC 5545 defines for TEXT values.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push(' '),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out.trim().to_string()
}

/// How an event repeats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recurrence {
    /// It does not.
    Once,
    Every {
        freq: Freq,
        interval: i32,
        count: Option<u32>,
        until: Option<Timestamp>,
        /// Weekdays, for `FREQ=WEEKLY;BYDAY=MO,WE`. Empty means "the same
        /// weekday as the start".
        weekdays: Vec<Weekday>,
    },
    /// The rule used a part this parser does not implement. Only the event's
    /// own start is produced — see the note on [`Recurrence::expand`].
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Recurrence {
    /// Parse an `RRULE` value, or report that it is outside the subset.
    ///
    /// The parts handled are `FREQ`, `INTERVAL`, `COUNT`, `UNTIL` and — for
    /// weekly rules — `BYDAY` without an ordinal prefix. `WKST` is accepted and
    /// ignored, because it only matters for the parts we do not support.
    ///
    /// Anything else is [`Recurrence::Unsupported`]. That includes `BYSETPOS`,
    /// `BYMONTHDAY`, `BYMONTH` and an ordinal `BYDAY` such as `1MO`, all of
    /// which *narrow* the set of occurrences — so guessing without them would
    /// put meetings on screen that are not happening.
    fn parse(value: &str) -> Self {
        let mut freq = None;
        let mut interval = 1i32;
        let mut count = None;
        let mut until = None;
        let mut weekdays = Vec::new();

        for part in value.split(';') {
            let Some((key, val)) = part.split_once('=') else {
                continue;
            };
            match key.to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = match val.to_ascii_uppercase().as_str() {
                        "DAILY" => Some(Freq::Daily),
                        "WEEKLY" => Some(Freq::Weekly),
                        "MONTHLY" => Some(Freq::Monthly),
                        "YEARLY" => Some(Freq::Yearly),
                        // HOURLY, MINUTELY and SECONDLY exist and are not
                        // things a dashboard should try to draw.
                        _ => return Self::Unsupported,
                    };
                }
                "INTERVAL" => match val.parse::<i32>() {
                    Ok(n) if n >= 1 => interval = n,
                    _ => return Self::Unsupported,
                },
                "COUNT" => match val.parse::<u32>() {
                    Ok(n) => count = Some(n),
                    Err(_) => return Self::Unsupported,
                },
                "UNTIL" => {
                    let stamp = parse_datetime(val)
                        .and_then(|dt| dt.to_zoned(TimeZone::UTC).ok())
                        .map(|z| z.timestamp())
                        .or_else(|| {
                            parse_date(val)
                                .and_then(|d| {
                                    d.to_datetime(Time::midnight()).to_zoned(TimeZone::UTC).ok()
                                })
                                .map(|z| z.timestamp())
                        });
                    match stamp {
                        Some(stamp) => until = Some(stamp),
                        None => return Self::Unsupported,
                    }
                }
                "BYDAY" => {
                    for day in val.split(',') {
                        // An ordinal prefix (`1MO`, `-1FR`) selects one week of
                        // the month, which this does not implement.
                        match weekday(day) {
                            Some(w) => weekdays.push(w),
                            None => return Self::Unsupported,
                        }
                    }
                }
                "WKST" => {}
                _ => return Self::Unsupported,
            }
        }

        match freq {
            Some(freq) => Self::Every {
                freq,
                interval,
                count,
                until,
                weekdays,
            },
            None => Self::Unsupported,
        }
    }

    /// Every occurrence overlapping `[from, until]`.
    ///
    /// [`Recurrence::Unsupported`] yields only the event's own start. That is
    /// the deliberate answer: an occurrence at `DTSTART` definitely happens,
    /// and everything else the rule implies is a guess. A dashboard that
    /// invents a meeting costs its reader a wasted trip; one that misses a
    /// repeat costs them a glance at the real calendar.
    fn expand(
        &self,
        event: &Event,
        exceptions: &[Timestamp],
        from: &Zoned,
        until: &Zoned,
    ) -> Vec<Event> {
        let skip: BTreeSet<Timestamp> = exceptions.iter().copied().collect();
        let length: Option<Span> = event
            .end
            .as_ref()
            .map(|end| end.timestamp() - event.start.timestamp());

        let occurrence = |start: Zoned| -> Option<Event> {
            if skip.contains(&start.timestamp()) {
                return None;
            }
            let end: Option<Zoned> = length.and_then(|span| start.checked_add(span).ok());
            // Overlapping the window, not merely starting in it: a meeting that
            // began before you looked is the one you most want to see.
            let visible_until = end.as_ref().unwrap_or(&start);
            if visible_until < from || &start > until {
                return None;
            }
            Some(Event {
                start,
                end,
                ..event.clone()
            })
        };

        let Self::Every {
            freq,
            interval,
            count,
            until: rule_until,
            weekdays,
        } = self
        else {
            // `Once` and `Unsupported` agree here, for different reasons.
            return occurrence(event.start.clone()).into_iter().collect();
        };

        let mut out = Vec::new();
        let mut produced = 0u32;
        let mut cursor = event.start.date();
        let time = event.start.time();
        let tz = event.start.time_zone().clone();

        for _ in 0..MAX_STEPS {
            if cursor > until.date() {
                break;
            }

            // For a weekly rule with BYDAY, the cursor names a week and each
            // listed weekday in it is an occurrence. Offsets are taken from
            // Monday so the set is produced in calendar order regardless of
            // which day the event started on.
            let days: Vec<Date> = if *freq == Freq::Weekly && !weekdays.is_empty() {
                let cursor_offset = i64::from(cursor.weekday().to_monday_zero_offset());
                let mut days: Vec<Date> = weekdays
                    .iter()
                    .filter_map(|w| {
                        let delta = i64::from(w.to_monday_zero_offset()) - cursor_offset;
                        cursor.checked_add(Span::new().days(delta)).ok()
                    })
                    .collect();
                days.sort_unstable();
                days.dedup();
                days
            } else {
                vec![cursor]
            };

            for day in days {
                if day < event.start.date() {
                    continue;
                }
                let Ok(start) = day.to_datetime(time).to_zoned(tz.clone()) else {
                    continue;
                };
                if let Some(limit) = rule_until
                    && start.timestamp() > *limit
                {
                    return out;
                }
                if let Some(limit) = count
                    && produced >= *limit
                {
                    return out;
                }
                produced += 1;
                if let Some(event) = occurrence(start) {
                    out.push(event);
                }
            }

            // The fallible setters, not `days`/`weeks`/`months`/`years`. Those
            // **panic** when the figure is outside what a `Span` can hold, and
            // `INTERVAL` comes straight out of the file: `FREQ=DAILY` with
            // `INTERVAL=999999999` is 999 million days against a limit of about
            // seven million, and it brought the dashboard down.
            //
            // Deliberately not a constant of our own. Encoding jiff's bounds
            // here would be a second copy of a number that belongs to a
            // dependency, and would go quietly wrong the first time that
            // dependency changed it — the same reasoning as the TOML nesting
            // bound in `themes`. Ask jiff instead, and treat a refusal as a rule
            // that has run out of road.
            let step = Span::new();
            let step = match freq {
                Freq::Daily => step.try_days(i64::from(*interval)),
                Freq::Weekly => step.try_weeks(i64::from(*interval)),
                Freq::Monthly => step.try_months(i64::from(*interval)),
                Freq::Yearly => step.try_years(i64::from(*interval)),
            };
            let Ok(step) = step else {
                break;
            };
            match cursor.checked_add(step) {
                Ok(next) => cursor = next,
                Err(_) => break,
            }
        }

        out
    }
}

/// Most steps a single rule may take while expanding.
///
/// Bounds a malformed rule with a tiny interval and no `COUNT` or `UNTIL`,
/// which would otherwise spin for as long as the window is wide — on the
/// reader thread, where nothing would notice.
const MAX_STEPS: usize = 4_000;

/// The two-letter weekday codes, without an ordinal prefix.
fn weekday(code: &str) -> Option<Weekday> {
    Some(match code.to_ascii_uppercase().as_str() {
        "MO" => Weekday::Monday,
        "TU" => Weekday::Tuesday,
        "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday,
        "FR" => Weekday::Friday,
        "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday,
        _ => return None,
    })
}

/// A date in the local zone, for callers building a window.
pub fn local_midnight(day: Date, tz: &TimeZone) -> Option<Zoned> {
    day.to_datetime(Time::midnight()).to_zoned(tz.clone()).ok()
}

/// Today, in `tz`.
pub fn today(tz: &TimeZone) -> Date {
    Timestamp::now().to_zoned(tz.clone()).date()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Phase 2: the untrusted-input boundary
    //
    // A `.ics` file is somebody else's, and this module is the largest thing in
    // mirador that reads one. The tests above check that it reads *correct*
    // calendars correctly. These check the other half — that no input at all,
    // however malformed, produces a panic, a hang, or unbounded memory.
    //
    // The generator is deliberately hand-rolled and deterministic. `proptest`
    // and `arbitrary` both do this better, and neither is worth a dependency
    // and a build-time cost for one module: what matters here is the *corpus*,
    // not the shrinking, because the fragments below are chosen from how this
    // parser actually works rather than sampled from all possible bytes.
    // -----------------------------------------------------------------------

    /// xorshift64*, so a failing seed can be re-run by hand.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }

        fn pick<'a, T>(&mut self, from: &'a [T]) -> &'a T {
            &from[self.below(from.len())]
        }
    }

    /// Fragments a line can be built from.
    ///
    /// Every entry is here because of something in the parser: the multi-byte
    /// strings for the byte-slicing panic that `starts_with_ci` now prevents,
    /// the quoted colons for `Property::parse`'s quote tracking, the enormous
    /// `COUNT` and `INTERVAL` for the expansion bound, the lone `BEGIN:` and
    /// `END:` for the nesting counter.
    const FRAGMENTS: &[&str] = &[
        "BEGIN:VEVENT",
        "END:VEVENT",
        "BEGIN:VALARM",
        "END:VALARM",
        "BEGIN:",
        "END:",
        "BEGIN",
        "END",
        "DTSTART:20260601T120000Z",
        "DTSTART;VALUE=DATE:20260601",
        "DTSTART;TZID=\"GMT+01:00\":20260601T120000",
        "DTSTART;TZID=Nowhere/Nothing:20260601T120000",
        "DTSTART:not-a-date",
        "DTSTART:",
        "DTEND:20260601T130000Z",
        "DTEND:19700101T000000Z",
        "DURATION:PT1H",
        "DURATION:P999999999D",
        "DURATION:garbage",
        "SUMMARY:ok",
        "SUMMARY:日本語日本語",
        "SUMMARY:abcé",
        "SUMMARY:a\\,b\\;c\\nd",
        "LOCATION:🦀🦀🦀",
        "RRULE:FREQ=DAILY",
        "RRULE:FREQ=SECONDLY;COUNT=999999999",
        "RRULE:FREQ=DAILY;INTERVAL=0",
        "RRULE:FREQ=DAILY;INTERVAL=999999999",
        "RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR,SA,SU",
        "RRULE:FREQ=YEARLY;UNTIL=99991231T235959Z",
        "RRULE:FREQ=NONSENSE",
        "RRULE:",
        "EXDATE:20260601T120000Z",
        "EXDATE:nonsense",
        "abcé",
        "ab日本",
        "日本語日本語",
        "  folded continuation",
        "\tfolded with a tab",
        ":",
        ";",
        "=",
        "\"",
        "\"unclosed",
        "",
        "\u{0}\u{1}\u{7f}",
        "X-WR-CALNAME:x",
    ];

    /// Build one calendar of up to `lines` lines from the fragments.
    fn generate(rng: &mut Rng, lines: usize) -> String {
        let mut out = String::new();
        for _ in 0..lines {
            let fragment = *rng.pick(FRAGMENTS);
            // Occasionally repeat a fragment, which is how a real broken file
            // goes wrong — a loop in an exporter, not one odd byte.
            let repeat = match rng.below(20) {
                0 => 40,
                1 => 5,
                _ => 1,
            };
            for _ in 0..repeat {
                out.push_str(fragment);
                out.push_str(if rng.below(4) == 0 { "\n" } else { "\r\n" });
            }
        }
        out
    }

    fn utc_window() -> (TimeZone, Zoned, Zoned) {
        let tz = TimeZone::UTC;
        let from: Zoned = "2026-01-01T00:00:00[UTC]".parse().expect("from");
        let until: Zoned = "2027-01-01T00:00:00[UTC]".parse().expect("until");
        (tz, from, until)
    }

    /// No generated calendar may panic, and none may take long enough to be
    /// felt. The dashboard re-reads its `.ics` on a tick, so "slow" here is a
    /// hang by another name.
    #[test]
    #[ignore = "measures memory amplification; not an assertion"]
    fn probe_memory_amplification() {
        // The worst case for `unfold`: a file that is nothing but line endings,
        // so every byte of input becomes a `String` in the returned Vec.
        for mb in [1usize, 5, 10] {
            let text = "\n".repeat(mb * 1024 * 1024);
            let lines = unfold(&text);
            let vec_bytes = lines.len() * std::mem::size_of::<String>();
            let heap: usize = lines.iter().map(String::capacity).sum();
            println!(
                "{mb} MB of newlines -> {} lines, {} MB of Vec + {} MB of strings = {:.1}x",
                lines.len(),
                vec_bytes / 1024 / 1024,
                heap / 1024 / 1024,
                (vec_bytes + heap) as f64 / (mb * 1024 * 1024) as f64
            );
        }
    }

    #[test]
    #[ignore = "the wide sweep; the committed test is a subset that fits CI"]
    fn probe_wide_sweep() {
        let (tz, from, until) = utc_window();
        let mut worst = std::time::Duration::ZERO;
        let mut worst_seed = 0u64;
        for seed in 1..=40_000u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let lines = 1 + rng.below(300);
            let text = generate(&mut rng, lines);
            let start = std::time::Instant::now();
            let calendar = parse(&text, &tz, &from, &until);
            let elapsed = start.elapsed();
            if elapsed > worst {
                worst = elapsed;
                worst_seed = seed;
                println!(
                    "seed {seed}: {elapsed:?}, {} events, {} skipped, {} bytes in",
                    calendar.events.len(),
                    calendar.skipped.len(),
                    text.len()
                );
            }
        }
        println!("worst of 40,000: seed {worst_seed} at {worst:?}");
    }

    #[test]
    fn no_generated_calendar_panics_or_stalls() {
        let (tz, from, until) = utc_window();
        let mut worst = std::time::Duration::ZERO;
        let mut worst_seed = 0u64;

        for seed in 1..=600u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let text = generate(&mut rng, 60);

            let start = std::time::Instant::now();
            let calendar = parse(&text, &tz, &from, &until);
            let elapsed = start.elapsed();

            if elapsed > worst {
                worst = elapsed;
                worst_seed = seed;
            }
            // Expansion is bounded, so the output must be too — whatever the
            // input asked for.
            assert!(
                calendar.events.len() <= MAX_STEPS * 40,
                "seed {seed} expanded to {} events",
                calendar.events.len()
            );
        }

        assert!(
            worst < std::time::Duration::from_millis(500),
            "seed {worst_seed} took {worst:?} to parse"
        );
    }

    /// The same, on input that is not iCalendar at all — the case of pointing
    /// `[agenda].file` at the wrong thing, which is far more likely than a
    /// hostile calendar.
    #[test]
    fn arbitrary_bytes_are_not_a_calendar_and_must_not_be_a_crash() {
        let (tz, from, until) = utc_window();
        for seed in 1..=300u64 {
            let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
            // Printable and not, including the characters that steer the parser.
            let text: String = (0..2_000)
                .map(|_| {
                    let choices = [
                        'a', 'Z', '0', ':', ';', '=', '"', ',', '\\', '\n', '\r', ' ', '\t', 'é',
                        '日', '🦀', '\u{0}', '\u{7f}',
                    ];
                    *rng.pick(&choices)
                })
                .collect();
            let calendar = parse(&text, &tz, &from, &until);
            // Nothing to assert about the contents; the assertion is that we
            // got here at all.
            assert!(calendar.events.len() < 1_000_000);
        }
    }

    /// Every fragment on its own, inside a `VEVENT` — so a failure names the
    /// line that caused it rather than a seed.
    #[test]
    fn every_fragment_survives_on_its_own_inside_an_event() {
        let (tz, from, until) = utc_window();
        for fragment in FRAGMENTS {
            let text =
                format!("BEGIN:VEVENT\r\nDTSTART:20260601T120000Z\r\n{fragment}\r\nEND:VEVENT\r\n");
            let result = std::panic::catch_unwind(|| {
                parse(&text, &tz, &from, &until);
            });
            assert!(
                result.is_ok(),
                "the line {fragment:?} brought the parser down"
            );
        }
    }

    /// The specific shape that was live: a line inside a `VEVENT` whose fourth
    /// or sixth byte falls inside a character. `line[..6]` panicked on it.
    #[test]
    fn a_line_of_international_text_inside_an_event_does_not_panic() {
        let (tz, from, until) = utc_window();
        // Every offset at which a multi-byte character can straddle byte 4 or 6.
        for pad in 0..8usize {
            for tail in ["é", "日", "🦀"] {
                let line = format!("{}{tail}", "a".repeat(pad));
                let text =
                    format!("BEGIN:VEVENT\r\nDTSTART:20260601T120000Z\r\n{line}\r\nEND:VEVENT\r\n");
                let result = std::panic::catch_unwind(|| {
                    parse(&text, &tz, &from, &until);
                });
                assert!(result.is_ok(), "{line:?} ({} bytes) panicked", line.len());
            }
        }
    }

    /// The byte cap in the agenda panel does not bound memory, because the cost
    /// of reading a calendar is per *line*: every line becomes a `String` in a
    /// `Vec`, and a `String` is 24 bytes before it holds anything. A 10MB file
    /// of bare newlines produced 240MB of `Vec` before an event was parsed.
    #[test]
    fn a_calendar_of_nothing_but_line_endings_does_not_eat_the_machine() {
        let (tz, from, until) = utc_window();
        let text = "\n".repeat(2 * 1024 * 1024);

        let lines = unfold(&text);
        assert!(
            lines.len() <= MAX_LINES,
            "{} lines read from a file with no content in it",
            lines.len()
        );

        // And the reader is told, rather than quietly losing the rest.
        let calendar = parse(&text, &tz, &from, &until);
        assert!(
            calendar.skipped.iter().any(|s| s.contains("more than")),
            "truncation was silent: {:?}",
            calendar.skipped
        );
    }

    /// A calendar that fits is read whole — a bound that clips real calendars
    /// would be worse than the problem it solves.
    #[test]
    fn an_ordinary_calendar_is_read_to_the_end() {
        use std::fmt::Write as _;

        let (tz, from, until) = utc_window();
        let mut text = String::from("BEGIN:VCALENDAR\r\n");
        for day in 1..=28 {
            let _ = write!(
                text,
                "BEGIN:VEVENT\r\nDTSTART:202606{day:02}T120000Z\r\n\
                 SUMMARY:day {day}\r\nEND:VEVENT\r\n"
            );
        }
        text.push_str("END:VCALENDAR\r\n");

        let calendar = parse(&text, &tz, &from, &until);
        assert_eq!(calendar.events.len(), 28, "events were lost");
        assert!(calendar.skipped.is_empty(), "{:?}", calendar.skipped);
    }

    /// `starts_with_ci` compares bytes, which is only correct because every
    /// prefix it is given is ASCII.
    #[test]
    fn the_prefix_test_is_case_insensitive_and_never_splits_a_character() {
        assert!(starts_with_ci("BEGIN:VEVENT", "BEGIN:"));
        assert!(starts_with_ci("begin:vevent", "BEGIN:"));
        assert!(starts_with_ci("BeGiN:x", "begin:"));
        assert!(
            !starts_with_ci("BEGIN:", "BEGIN:"),
            "needs something after it"
        );
        assert!(!starts_with_ci("BEGI", "BEGIN:"));
        assert!(
            !starts_with_ci("日本語日本語", "BEGIN:"),
            "and does not panic"
        );
        assert!(!starts_with_ci("abcé", "END:"));
    }

    use jiff::civil::date;

    fn tz() -> TimeZone {
        TimeZone::get("America/New_York").expect("a zone every tzdb has")
    }

    fn window(from: (i16, i8, i8), to: (i16, i8, i8)) -> (Zoned, Zoned) {
        let tz = tz();
        (
            local_midnight(date(from.0, from.1, from.2), &tz).unwrap(),
            local_midnight(date(to.0, to.1, to.2), &tz).unwrap(),
        )
    }

    fn wrap(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}\r\nEND:VCALENDAR\r\n")
    }

    fn parse_all(body: &str) -> Calendar {
        let (from, to) = window((2026, 1, 1), (2030, 1, 1));
        parse(&wrap(body), &tz(), &from, &to)
    }

    /// A calendar is untrusted input, and the review that produced this test
    /// found its one hang at exactly that boundary in another module. These
    /// are the shapes a hostile or merely broken `.ics` takes.
    ///
    /// Asserts bounds rather than elapsed time: a timing assertion is a flaky
    /// test on a loaded CI runner, and what actually matters is that the work
    /// is *bounded*, not that a particular machine finished it quickly.
    #[test]
    fn pathological_calendars_stay_bounded() {
        let tz = TimeZone::UTC;
        let from = "2026-07-01T00:00:00Z[UTC]".parse::<Zoned>().expect("from");
        let until = "2026-07-08T00:00:00Z[UTC]".parse::<Zoned>().expect("until");

        // A rule that would run past the heat death of the universe. The window
        // clips it; `COUNT` is not permitted to decide how much work happens.
        let forever = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20260701T090000Z\n\
                       SUMMARY:X\nRRULE:FREQ=DAILY;COUNT=999999999\nEND:VEVENT\n\
                       END:VCALENDAR\n";
        let out = parse(forever, &tz, &from, &until);
        assert!(
            out.events.len() <= 8,
            "a week-long window cannot hold {} daily events",
            out.events.len()
        );

        // Fifty thousand folded continuations: one very long summary, not fifty
        // thousand events, and no quadratic rebuild.
        let mut folded =
            String::from("BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20260701T090000Z\nSUMMARY:start");
        for _ in 0..50_000 {
            folded.push_str("\n more");
        }
        folded.push_str("\nEND:VEVENT\nEND:VCALENDAR\n");
        let out = parse(&folded, &tz, &from, &until);
        assert_eq!(out.events.len(), 1, "one event, however folded");

        // Deeply nested components. A VALARM is not an event and nesting is not
        // recursion here, so neither the stack nor the event list grows.
        let mut nested =
            String::from("BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20260701T090000Z\nSUMMARY:N\n");
        for _ in 0..20_000 {
            nested.push_str("BEGIN:VALARM\n");
        }
        for _ in 0..20_000 {
            nested.push_str("END:VALARM\n");
        }
        nested.push_str("END:VEVENT\nEND:VCALENDAR\n");
        let out = parse(&nested, &tz, &from, &until);
        assert_eq!(out.events.len(), 1, "nesting does not multiply events");

        // Truncated mid-event, which is what a half-written sync leaves behind.
        let truncated = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:2026070";
        let _ = parse(truncated, &tz, &from, &until);

        // Not a calendar at all.
        let _ = parse("\u{0}\u{1}\u{2}not a calendar", &tz, &from, &until);
    }

    #[test]
    fn a_folded_line_is_rejoined_at_the_character_after_the_space() {
        // The failure mode is silent: every summary over ~75 characters gets
        // truncated, and it looks like the calendar exported badly.
        let lines = unfold("SUMMARY:Quarterly planning with\r\n  the platform team\r\nEND");
        assert_eq!(
            lines[0],
            "SUMMARY:Quarterly planning with the platform team"
        );
        assert_eq!(lines[1], "END");

        // A tab folds too, and the continuation keeps its own leading space.
        let lines = unfold("A:one\r\n\ttwo");
        assert_eq!(lines[0], "A:onetwo");
    }

    #[test]
    fn a_leading_space_on_the_first_line_is_not_a_continuation() {
        // There is nothing to continue, and treating it as one would panic or
        // silently drop the line depending on how it is written.
        let lines = unfold(" SUMMARY:odd");
        assert_eq!(lines, vec![" SUMMARY:odd"]);
    }

    #[test]
    fn a_quoted_parameter_may_contain_a_colon() {
        let p = Property::parse(r#"DTSTART;TZID="GMT+01:00":20260801T140000"#).unwrap();
        assert_eq!(p.name, "DTSTART");
        assert_eq!(p.param("TZID"), Some("GMT+01:00"));
        assert_eq!(p.value, "20260801T140000");
    }

    #[test]
    fn the_three_time_forms_all_land_on_the_same_instant() {
        // 14:00 New York on 1 August 2026 is 18:00 UTC. A calendar that gets
        // this wrong shows every meeting at the wrong hour, which is the single
        // worst thing this panel could do.
        let utc =
            parse_all("BEGIN:VEVENT\r\nDTSTART:20260801T180000Z\r\nSUMMARY:utc\r\nEND:VEVENT");
        let zoned = parse_all(
            "BEGIN:VEVENT\r\nDTSTART;TZID=America/New_York:20260801T140000\r\nSUMMARY:zoned\r\nEND:VEVENT",
        );
        let floating =
            parse_all("BEGIN:VEVENT\r\nDTSTART:20260801T140000\r\nSUMMARY:floating\r\nEND:VEVENT");

        let at = |c: &Calendar| c.events[0].start.timestamp();
        assert_eq!(at(&utc), at(&zoned), "Z and TZID disagree");
        assert_eq!(at(&zoned), at(&floating), "floating is not local");
        assert_eq!(utc.events[0].start.hour(), 14);
    }

    #[test]
    fn a_zone_in_another_country_is_converted_rather_than_relabelled() {
        let c = parse_all(
            "BEGIN:VEVENT\r\nDTSTART;TZID=Europe/London:20260801T180000\r\nSUMMARY:london\r\nEND:VEVENT",
        );
        // 18:00 London in August (BST, +1) is 13:00 New York.
        assert_eq!(c.events[0].start.hour(), 13);
    }

    #[test]
    fn an_unknown_timezone_falls_back_to_local_rather_than_dropping_the_event() {
        // Windows exporters emit names no tzdb has. The wrong offset is bad;
        // silently losing the meeting is worse.
        let c = parse_all(
            "BEGIN:VEVENT\r\nDTSTART;TZID=Romance Standard Time:20260801T140000\r\nSUMMARY:x\r\nEND:VEVENT",
        );
        assert_eq!(c.events.len(), 1);
        assert_eq!(c.events[0].start.hour(), 14);
    }

    #[test]
    fn an_all_day_event_has_no_time_and_covers_its_day() {
        let c = parse_all(
            "BEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20260801\r\nSUMMARY:holiday\r\nEND:VEVENT",
        );
        let e = &c.events[0];
        assert!(e.all_day);
        assert_eq!(e.start.date(), date(2026, 8, 1));
        assert_eq!(e.end.as_ref().unwrap().date(), date(2026, 8, 2));
    }

    #[test]
    fn a_duration_stands_in_for_a_missing_end() {
        let c = parse_all(
            "BEGIN:VEVENT\r\nDTSTART:20260801T140000\r\nDURATION:PT1H30M\r\nSUMMARY:x\r\nEND:VEVENT",
        );
        let e = &c.events[0];
        assert_eq!(e.end.as_ref().unwrap().hour(), 15);
        assert_eq!(e.end.as_ref().unwrap().minute(), 30);
    }

    #[test]
    fn text_escapes_are_undone() {
        let c = parse_all(
            r"BEGIN:VEVENT
DTSTART:20260801T140000
SUMMARY:Review\, then ship\; carefully\nSecond line
LOCATION:Room A\, floor 2
END:VEVENT",
        );
        let e = &c.events[0];
        assert_eq!(e.summary, "Review, then ship; carefully Second line");
        assert_eq!(e.location.as_deref(), Some("Room A, floor 2"));
    }

    #[test]
    fn an_alarm_inside_an_event_does_not_become_its_time() {
        // A VALARM carries its own TRIGGER and, in some exports, a DTSTART.
        // Reading it as the event's would put everything on screen 15 minutes
        // early, which looks like a timezone bug and is not one.
        let c = parse_all(
            "BEGIN:VEVENT\r\nDTSTART:20260801T140000\r\nSUMMARY:standup\r\n\
             BEGIN:VALARM\r\nTRIGGER:-PT15M\r\nDTSTART:20260801T134500\r\nEND:VALARM\r\n\
             END:VEVENT",
        );
        assert_eq!(c.events.len(), 1);
        assert_eq!(c.events[0].start.hour(), 14);
    }

    #[test]
    fn an_event_without_a_start_is_skipped_without_being_called_an_error() {
        let c = parse_all("BEGIN:VEVENT\r\nSUMMARY:no start\r\nEND:VEVENT");
        assert!(c.events.is_empty());
        assert!(c.skipped.is_empty(), "not worth reporting: {:?}", c.skipped);
    }

    #[test]
    fn an_event_without_a_summary_still_draws_something() {
        // Invariant 11: never render an empty cell.
        let c = parse_all("BEGIN:VEVENT\r\nDTSTART:20260801T140000\r\nEND:VEVENT");
        assert_eq!(c.events[0].summary, "–");
    }

    #[test]
    fn an_unreadable_time_is_reported_rather_than_swallowed() {
        let c = parse_all("BEGIN:VEVENT\r\nDTSTART:not-a-date\r\nSUMMARY:x\r\nEND:VEVENT");
        assert!(c.events.is_empty());
        assert_eq!(c.skipped.len(), 1, "the reason must survive");
        assert!(c.skipped[0].contains("not-a-date"), "{:?}", c.skipped);
    }

    #[test]
    fn a_daily_rule_repeats_and_counts() {
        let (from, to) = window((2026, 8, 1), (2026, 8, 31));
        let c = parse(
            &wrap(
                "BEGIN:VEVENT\r\nDTSTART:20260801T090000\r\nDURATION:PT30M\r\n\
                 RRULE:FREQ=DAILY;COUNT=5\r\nSUMMARY:standup\r\nEND:VEVENT",
            ),
            &tz(),
            &from,
            &to,
        );
        assert_eq!(c.events.len(), 5);
        assert_eq!(c.events[0].start.date(), date(2026, 8, 1));
        assert_eq!(c.events[4].start.date(), date(2026, 8, 5));
    }

    #[test]
    fn an_interval_skips_and_until_stops() {
        let (from, to) = window((2026, 8, 1), (2026, 9, 30));
        let c = parse(
            &wrap(
                "BEGIN:VEVENT\r\nDTSTART:20260801T090000\r\n\
                 RRULE:FREQ=DAILY;INTERVAL=3;UNTIL=20260810T000000Z\r\nSUMMARY:x\r\nEND:VEVENT",
            ),
            &tz(),
            &from,
            &to,
        );
        let days: Vec<_> = c.events.iter().map(|e| e.start.day()).collect();
        assert_eq!(days, vec![1, 4, 7]);
    }

    #[test]
    fn a_weekly_rule_with_byday_produces_each_named_day() {
        let (from, to) = window((2026, 8, 1), (2026, 8, 22));
        let c = parse(
            &wrap(
                "BEGIN:VEVENT\r\nDTSTART:20260803T100000\r\n\
                 RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=6\r\nSUMMARY:x\r\nEND:VEVENT",
            ),
            &tz(),
            &from,
            &to,
        );
        // 3 August 2026 is a Monday.
        let days: Vec<_> = c.events.iter().map(|e| e.start.date()).collect();
        assert_eq!(
            days,
            vec![
                date(2026, 8, 3),
                date(2026, 8, 5),
                date(2026, 8, 7),
                date(2026, 8, 10),
                date(2026, 8, 12),
                date(2026, 8, 14),
            ]
        );
    }

    #[test]
    fn exdate_removes_an_occurrence_without_shifting_the_rest() {
        let (from, to) = window((2026, 8, 1), (2026, 8, 31));
        let c = parse(
            &wrap(
                "BEGIN:VEVENT\r\nDTSTART:20260801T090000\r\n\
                 RRULE:FREQ=DAILY;COUNT=4\r\nEXDATE:20260802T090000\r\nSUMMARY:x\r\nEND:VEVENT",
            ),
            &tz(),
            &from,
            &to,
        );
        let days: Vec<_> = c.events.iter().map(|e| e.start.day()).collect();
        assert_eq!(days, vec![1, 3, 4], "the 2nd should be gone, not shifted");
    }

    #[test]
    fn a_rule_outside_the_subset_yields_only_the_first_occurrence() {
        // The deliberate answer. `BYSETPOS` narrows the set; expanding without
        // it would put meetings on screen that are not happening, and a
        // calendar that invents a meeting is worse than one that misses one.
        for rule in [
            "FREQ=MONTHLY;BYSETPOS=1;BYDAY=MO",
            "FREQ=MONTHLY;BYDAY=1MO",
            "FREQ=MONTHLY;BYMONTHDAY=1,15",
            "FREQ=HOURLY",
        ] {
            let (from, to) = window((2026, 8, 1), (2027, 8, 1));
            let c = parse(
                &wrap(&format!(
                    "BEGIN:VEVENT\r\nDTSTART:20260803T090000\r\nRRULE:{rule}\r\nSUMMARY:x\r\nEND:VEVENT"
                )),
                &tz(),
                &from,
                &to,
            );
            assert_eq!(
                c.events.len(),
                1,
                "{rule} produced {} events",
                c.events.len()
            );
            assert_eq!(c.events[0].start.date(), date(2026, 8, 3));
        }
    }

    #[test]
    fn an_event_in_progress_is_inside_the_window_even_though_it_started_before_it() {
        // The meeting you are already late for is the one you most want to see,
        // so the window test is overlap rather than start.
        let tz = tz();
        let from = date(2026, 8, 1)
            .at(14, 30, 0, 0)
            .to_zoned(tz.clone())
            .unwrap();
        let to = local_midnight(date(2026, 8, 8), &tz).unwrap();
        let c = parse(
            &wrap(
                "BEGIN:VEVENT\r\nDTSTART:20260801T140000\r\nDURATION:PT2H\r\nSUMMARY:long\r\nEND:VEVENT",
            ),
            &tz,
            &from,
            &to,
        );
        assert_eq!(c.events.len(), 1, "an in-progress event was dropped");
        assert!(c.events[0].contains(&from));
    }

    #[test]
    fn an_unbounded_rule_cannot_run_away() {
        // No COUNT, no UNTIL, a one-day interval and a wide window. The cap is
        // what stops a malformed calendar from hanging the fetch thread.
        let (from, to) = window((2026, 1, 1), (2400, 1, 1));
        let started = std::time::Instant::now();
        let c = parse(
            &wrap(
                "BEGIN:VEVENT\r\nDTSTART:20260101T090000\r\nRRULE:FREQ=DAILY\r\nSUMMARY:x\r\nEND:VEVENT",
            ),
            &tz(),
            &from,
            &to,
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "expansion did not terminate promptly"
        );
        assert!(!c.events.is_empty());
        assert!(c.events.len() <= 4_000, "the cap did not hold");
    }

    #[test]
    fn events_come_back_in_time_order() {
        let c = parse_all(
            "BEGIN:VEVENT\r\nDTSTART:20260803T090000\r\nSUMMARY:second\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nDTSTART:20260801T090000\r\nSUMMARY:first\r\nEND:VEVENT",
        );
        let names: Vec<_> = c.events.iter().map(|e| e.summary.as_str()).collect();
        assert_eq!(names, vec!["first", "second"]);
    }

    #[test]
    fn an_empty_or_junk_file_produces_nothing_rather_than_panicking() {
        for text in ["", "not a calendar", "BEGIN:VCALENDAR", "BEGIN:VEVENT"] {
            let (from, to) = window((2026, 1, 1), (2027, 1, 1));
            let c = parse(text, &tz(), &from, &to);
            assert!(c.events.is_empty(), "{text:?} produced events");
        }
    }
}
