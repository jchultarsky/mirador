//! The agenda: what is actually next, out of a local `.ics` file.
//!
//! The gap this fills was named early and left open for a long time: tasks are
//! self-paced, and a meeting is not. A dashboard that can tell you four things
//! and none of them is "you are in a call in ten minutes" is missing the one
//! with a deadline attached.
//!
//! Deliberately **offline**. mirador does not sign into a calendar server, and
//! is not going to — that is an account, a token to refresh, and a background
//! process with opinions about your credentials. It reads a file. Whatever put
//! the file there is your business: an export, a `vdirsyncer` run, a cron job
//! with `curl`, a symlink into a synced folder.
//!
//! The `calendar` widget beside this one is a date grid and stays that way. One
//! answers "what is the date", the other "what is next", and squeezing both
//! into one panel does neither well.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Zoned};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span as TextSpan};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::config::AgendaConfig;
use crate::frame::{Binding, FRAME_HEIGHT, FRAME_WIDTH};
use crate::ical;
use crate::panel::{KeyOutcome, Panel, RenderContext, describe_age};

const BINDINGS: &[Binding] = &[
    Binding::primary("r", "reload"),
    Binding::extra("↑ / ↓", "scroll"),
    Binding::extra("j / k", "scroll"),
    Binding::extra("g / G", "first / last"),
    Binding::extra("Home / End", "first / last"),
    Binding::extra("o", "show file path"),
];

/// Width at which the panel stops gaining anything: a time, a generous summary
/// and a location beside it.
const USEFUL_WIDTH: u16 = 52;

/// What the reader thread has produced.
#[derive(Debug, Default)]
struct State {
    events: Vec<ical::Event>,
    /// Why the last read failed, if it did.
    error: Option<String>,
    /// Lines the parser could not make sense of, so a half-read calendar says
    /// so rather than looking merely empty.
    skipped: usize,
    /// When this was read.
    read_at: Option<Instant>,
    /// The day the window was built around, so a dashboard left open overnight
    /// notices that "today" moved.
    built_for: Option<Date>,
}

#[derive(Debug)]
pub struct AgendaPanel {
    state: Arc<Mutex<State>>,
    /// Set to ask the reader thread for an immediate re-read.
    reload: Arc<Mutex<bool>>,
    generation: Arc<AtomicU64>,
    seen: u64,
    stop: Arc<AtomicBool>,
    path: PathBuf,
    days: u16,
    show_location: bool,
    scroll: ListState,
    status: Option<String>,
    list_area: Option<Rect>,
}

impl Drop for AgendaPanel {
    /// See `Drop for StocksPanel`: the picker can drop a panel without calling
    /// `shutdown`, and a reader thread with no way to end is a leak.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl AgendaPanel {
    pub fn new(config: &AgendaConfig, path: PathBuf) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let reload = Arc::new(Mutex::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));

        let days = config.days.clamp(1, 365);
        // A local file is cheap to read, but not free — a year of a shared work
        // calendar is megabytes. The floor keeps a hand-edited `0` from
        // spinning the thread.
        let interval = Duration::from_secs(config.refresh_secs.max(5));

        let shared = (
            Arc::clone(&state),
            Arc::clone(&reload),
            Arc::clone(&stop),
            Arc::clone(&generation),
        );
        let thread_path = path.clone();
        std::thread::Builder::new()
            .name("mirador-agenda".into())
            .spawn(move || {
                let (state, reload, stop, generation) = shared;
                read_loop(
                    &thread_path,
                    days,
                    interval,
                    &state,
                    &reload,
                    &stop,
                    &generation,
                );
            })
            .expect("spawning the agenda thread");

        Self {
            state,
            reload,
            generation,
            seen: 0,
            stop,
            path,
            days,
            show_location: config.show_location,
            scroll: ListState::default(),
            status: None,
            list_area: None,
        }
    }

    fn snapshot(&self) -> State {
        match self.state.lock() {
            Ok(guard) => State {
                events: guard.events.clone(),
                error: guard.error.clone(),
                skipped: guard.skipped,
                read_at: guard.read_at,
                built_for: guard.built_for,
            },
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                State {
                    events: guard.events.clone(),
                    error: guard.error.clone(),
                    skipped: guard.skipped,
                    read_at: guard.read_at,
                    built_for: guard.built_for,
                }
            }
        }
    }

    fn ask_for_reload(&self) {
        match self.reload.lock() {
            Ok(mut flag) => *flag = true,
            Err(poisoned) => *poisoned.into_inner() = true,
        }
    }

    /// The rows to draw: a heading per day, then that day's events.
    ///
    /// Built as a flat list rather than a tree because a heading and an event
    /// scroll together and a heading is never selectable — the distinction only
    /// matters to whoever writes the arrow keys, and here it does not.
    fn rows(state: &State, now: &Zoned, show_location: bool, width: u16) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut current: Option<Date> = None;
        for event in &state.events {
            let day = event.start.date();
            if current != Some(day) {
                rows.push(Row::Day(day));
                current = Some(day);
            }
            rows.push(Row::Event {
                event: event.clone(),
                in_progress: event.contains(now),
                show_location,
                width,
            });
        }
        rows
    }
}

enum Row {
    Day(Date),
    Event {
        event: ical::Event,
        in_progress: bool,
        show_location: bool,
        width: u16,
    },
}

/// How a day is introduced. "Today" and "Tomorrow" beat a date you have to
/// work out, and the date is still there for everything else.
fn day_label(day: Date, today: Date) -> String {
    let delta = (day - today).get_days();
    match delta {
        0 => "TODAY".to_string(),
        1 => "TOMORROW".to_string(),
        _ => format!("{} {}", weekday_name(day), day.strftime("%-d %b")),
    }
}

fn weekday_name(day: Date) -> &'static str {
    match day.weekday() {
        jiff::civil::Weekday::Monday => "MONDAY",
        jiff::civil::Weekday::Tuesday => "TUESDAY",
        jiff::civil::Weekday::Wednesday => "WEDNESDAY",
        jiff::civil::Weekday::Thursday => "THURSDAY",
        jiff::civil::Weekday::Friday => "FRIDAY",
        jiff::civil::Weekday::Saturday => "SATURDAY",
        jiff::civil::Weekday::Sunday => "SUNDAY",
    }
}

/// Read the file, parse it, and publish — then wait, and do it again.
fn read_loop(
    path: &PathBuf,
    days: u16,
    interval: Duration,
    state: &Arc<Mutex<State>>,
    reload: &Arc<Mutex<bool>>,
    stop: &Arc<AtomicBool>,
    generation: &Arc<AtomicU64>,
) {
    while !stop.load(Ordering::Relaxed) {
        let tz = TimeZone::system();
        let today = ical::today(&tz);
        let from = ical::local_midnight(today, &tz);
        let until = today
            .checked_add(Span::new().days(i64::from(days)))
            .ok()
            .and_then(|d| ical::local_midnight(d, &tz));

        let next = match (from, until) {
            (Some(from), Some(until)) => match std::fs::read_to_string(path) {
                Ok(text) => {
                    let calendar = ical::parse(&text, &tz, &from, &until);
                    State {
                        events: calendar.events,
                        error: None,
                        skipped: calendar.skipped.len(),
                        read_at: Some(Instant::now()),
                        built_for: Some(today),
                    }
                }
                // Kept as text rather than a typed error: the message the OS
                // gives ("No such file or directory") is the one that tells
                // the user what to do.
                Err(e) => State {
                    error: Some(format!("{e}")),
                    read_at: Some(Instant::now()),
                    built_for: Some(today),
                    ..State::default()
                },
            },
            _ => State {
                error: Some("could not work out today's date".into()),
                read_at: Some(Instant::now()),
                built_for: Some(today),
                ..State::default()
            },
        };

        match state.lock() {
            Ok(mut guard) => *guard = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
        generation.fetch_add(1, Ordering::Release);

        let woke = crate::poll::wait(interval, stop, || match reload.lock() {
            Ok(mut flag) => std::mem::replace(&mut *flag, false),
            Err(poisoned) => std::mem::replace(&mut *poisoned.into_inner(), false),
        });
        if woke == crate::poll::Wake::Stop {
            return;
        }
    }
}

impl Panel for AgendaPanel {
    fn title(&self) -> String {
        "Agenda".to_string()
    }

    fn counter(&self) -> Option<String> {
        let state = self.snapshot();
        if state.error.is_some() {
            return Some("no file".into());
        }
        let today = state.built_for?;
        let n = state
            .events
            .iter()
            .filter(|e| e.start.date() == today)
            .count();
        Some(match n {
            0 => "clear".to_string(),
            1 => "1 today".to_string(),
            n => format!("{n} today"),
        })
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn max_width(&self) -> Option<u16> {
        // Past this the summaries stop being the constraint and the row just
        // grows a gap in the middle; the graphs next door can use it.
        Some(USEFUL_WIDTH + FRAME_WIDTH)
    }

    fn refresh_interval(&self) -> Duration {
        // The reader thread owns the real cadence. This only decides how
        // quickly a completed read reaches the screen, and how often the
        // in-progress marker is re-evaluated.
        Duration::from_secs(20)
    }

    fn tick(&mut self) -> bool {
        // Two things can change without a keypress: a read landing, and an
        // event starting or ending. The first is a counter; the second is why
        // this returns true on the day rolling over even when nothing was read.
        let now = self.generation.load(Ordering::Acquire);
        let moved = now != self.seen;
        self.seen = now;
        moved
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        self.status = None;
        let len = self.snapshot().events.len();
        match key.code {
            KeyCode::Char('r') => {
                self.ask_for_reload();
                self.status = Some("reloading…".into());
            }
            KeyCode::Char('o') => {
                self.status = Some(self.path.display().to_string());
            }
            KeyCode::Down | KeyCode::Char('j') => crate::selection::down(&mut self.scroll, 1, len),
            KeyCode::Up | KeyCode::Char('k') => crate::selection::up(&mut self.scroll, 1, len),
            KeyCode::PageDown => crate::selection::down(&mut self.scroll, 10, len),
            KeyCode::PageUp => crate::selection::up(&mut self.scroll, 10, len),
            KeyCode::Char('G') | KeyCode::End => {
                crate::selection::down(&mut self.scroll, usize::MAX, len);
            }
            KeyCode::Char('g') | KeyCode::Home => {
                crate::selection::up(&mut self.scroll, usize::MAX, len);
            }
            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> KeyOutcome {
        let len = self.snapshot().events.len();
        match event.kind {
            MouseEventKind::ScrollDown => crate::selection::down(&mut self.scroll, 1, len),
            MouseEventKind::ScrollUp => crate::selection::up(&mut self.scroll, 1, len),
            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.width == 0 || area.height == 0 {
            return;
        }

        let state = self.snapshot();
        let now = Zoned::now();

        // Notices take rows from the bottom, and the list is given what is
        // left. Drawing the list across the whole panel and painting over it
        // afterwards is what put `1 entry could not be readndred` on screen:
        // the notice landed on top of an event instead of beside it.
        let mut notices: Vec<Line<'static>> = Vec::new();
        if state.skipped > 0 {
            notices.push(Line::from(TextSpan::styled(
                format!(
                    "{} entr{} could not be read",
                    state.skipped,
                    if state.skipped == 1 { "y" } else { "ies" }
                ),
                Style::default().fg(theme.error),
            )));
        }
        if let Some(message) = &self.status {
            notices.push(Line::from(TextSpan::styled(
                message.clone(),
                Style::default().fg(theme.muted),
            )));
        }

        let (list_area, notice_area) =
            split_for_notices(area, u16::try_from(notices.len()).unwrap_or(0));

        self.draw_list(frame, list_area, &state, &now, theme);

        if notice_area.height > 0 {
            frame.render_widget(Paragraph::new(notices), notice_area);
        }
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl AgendaPanel {
    fn draw_list(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        state: &State,
        now: &Zoned,
        theme: &crate::theme::Theme,
    ) {
        if let Some(error) = &state.error {
            let age = state.read_at.map(|at| describe_age(at.elapsed()));
            let mut lines = vec![
                Line::from(TextSpan::styled(
                    "No calendar to read",
                    Style::default()
                        .fg(theme.error)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(TextSpan::styled(
                    error.clone(),
                    Style::default().fg(theme.muted),
                )),
                Line::from(""),
                Line::from(TextSpan::styled(
                    format!("Looking for {}", self.path.display()),
                    Style::default().fg(theme.muted),
                )),
                Line::from(TextSpan::styled(
                    "Point [agenda].file at an .ics you already have.",
                    Style::default().fg(theme.muted),
                )),
            ];
            if let Some(age) = age {
                lines.push(Line::from(TextSpan::styled(
                    format!("checked {age}"),
                    Style::default().fg(theme.muted),
                )));
            }
            frame.render_widget(Paragraph::new(lines), area);
            return;
        }

        if state.events.is_empty() {
            let horizon = if self.days == 1 {
                "today".to_string()
            } else {
                format!("the next {} days", self.days)
            };
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(TextSpan::styled(
                        "Nothing scheduled",
                        Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(TextSpan::styled(
                        format!("in {horizon}."),
                        Style::default().fg(theme.muted),
                    )),
                ]),
                area,
            );
            return;
        }

        let today = state.built_for.unwrap_or_else(|| now.date());
        let rows = Self::rows(state, now, self.show_location, area.width);

        let items: Vec<ListItem> = rows
            .iter()
            .map(|row| match row {
                Row::Day(day) => ListItem::new(Line::from(TextSpan::styled(
                    crate::glyphs::utility(&day_label(*day, today)),
                    Style::default()
                        .fg(theme.label)
                        .add_modifier(Modifier::BOLD),
                ))),
                Row::Event {
                    event,
                    in_progress,
                    show_location,
                    width,
                } => ListItem::new(event_line(
                    event,
                    *in_progress,
                    *show_location,
                    *width,
                    theme,
                )),
            })
            .collect();

        self.list_area = Some(area);
        frame.render_widget(List::new(items), area);
    }
}

/// Divide the panel between the list and the notices below it.
///
/// The two never overlap and together cover `area`. Getting that wrong does not
/// look like a layout bug — the notice lands on top of the last event and the
/// tail of it shows through, which reads as a corrupt terminal:
///
/// ```text
///   20:00  Meeting at 20 hundred
/// 1 entry could not be readndred
/// ```
///
/// The list always keeps at least one row: a panel showing only a complaint has
/// hidden the thing the complaint is about.
fn split_for_notices(area: Rect, notices: u16) -> (Rect, Rect) {
    let reserved = notices.min(area.height.saturating_sub(1));
    let list = Rect {
        height: area.height - reserved,
        ..area
    };
    let notice = Rect {
        y: area.y + list.height,
        height: reserved,
        ..area
    };
    (list, notice)
}

/// One event, as a row.
fn event_line(
    event: &ical::Event,
    in_progress: bool,
    show_location: bool,
    width: u16,
    theme: &crate::theme::Theme,
) -> Line<'static> {
    // A fixed-width time column, so the summaries line up whether or not the
    // day mixes all-day entries with timed ones.
    const TIME_WIDTH: usize = 6;

    let time = if event.all_day {
        "all day".to_string()
    } else {
        event.start.strftime("%H:%M").to_string()
    };

    let marker = if in_progress { "▸ " } else { "  " };
    let time_style = if in_progress {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.muted)
    };
    let summary_style = if in_progress {
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text)
    };

    let used = marker.len() + TIME_WIDTH.max(crate::grid::display_width(&time)) + 1;
    let room = usize::from(width).saturating_sub(used);

    let mut text = event.summary.clone();
    if show_location && let Some(location) = &event.location {
        // Only when there is genuinely room: a location that squeezes the
        // summary to nothing has cost more than it gave.
        let combined = format!("{text}  ·  {location}");
        if crate::grid::display_width(&combined) <= room {
            text = combined;
        }
    }

    Line::from(vec![
        TextSpan::styled(marker.to_string(), time_style),
        TextSpan::styled(format!("{time:<TIME_WIDTH$} "), time_style),
        TextSpan::styled(crate::grid::truncate(&text, room), summary_style),
    ])
}

/// Rows the frame costs, re-exported so `max_height` reads the same as the
/// other panels even though this one does not declare a maximum.
#[allow(dead_code)]
const _: u16 = FRAME_HEIGHT;

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn tz() -> TimeZone {
        TimeZone::get("America/New_York").unwrap()
    }

    fn event(day: Date, hour: i8, summary: &str, all_day: bool) -> ical::Event {
        let start = day.at(hour, 0, 0, 0).to_zoned(tz()).unwrap();
        ical::Event {
            summary: summary.to_string(),
            location: None,
            end: start.checked_add(Span::new().hours(1)).ok(),
            start,
            all_day,
        }
    }

    #[test]
    fn a_notice_never_lands_on_top_of_an_event() {
        // The bug this exists for, seen on screen before it was found in the
        // code: the notice was painted over the last row and the tail of the
        // event showed through it — `1 entry could not be readndred`.
        for height in 0u16..12 {
            for notices in 0u16..4 {
                let area = Rect::new(3, 5, 40, height);
                let (list, notice) = split_for_notices(area, notices);

                assert_eq!(
                    list.height + notice.height,
                    height,
                    "{height} rows, {notices} notices: the split loses rows"
                );
                assert_eq!(
                    notice.y,
                    list.y + list.height,
                    "{height} rows, {notices} notices: the notice overlaps the list"
                );
                if height > 0 {
                    assert!(
                        list.height >= 1,
                        "{height} rows, {notices} notices: the list was squeezed out"
                    );
                }
            }
        }
    }

    #[test]
    fn today_and_tomorrow_are_named_rather_than_dated() {
        let today = date(2026, 8, 1);
        assert_eq!(day_label(today, today), "TODAY");
        assert_eq!(day_label(date(2026, 8, 2), today), "TOMORROW");
        // Anything further out gets a weekday and a date, because "in 4 days"
        // is arithmetic the reader should not have to do either way.
        assert_eq!(day_label(date(2026, 8, 5), today), "WEDNESDAY 5 Aug");
    }

    #[test]
    fn each_day_gets_one_heading_however_many_events_it_has() {
        let state = State {
            events: vec![
                event(date(2026, 8, 1), 9, "a", false),
                event(date(2026, 8, 1), 11, "b", false),
                event(date(2026, 8, 2), 9, "c", false),
            ],
            ..State::default()
        };
        let now = date(2026, 8, 1).at(8, 0, 0, 0).to_zoned(tz()).unwrap();
        let rows = AgendaPanel::rows(&state, &now, true, 60);

        let headings = rows.iter().filter(|r| matches!(r, Row::Day(_))).count();
        assert_eq!(headings, 2, "one heading per day, not per event");
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn an_event_happening_now_is_marked() {
        let state = State {
            events: vec![event(date(2026, 8, 1), 9, "standup", false)],
            ..State::default()
        };
        let during = date(2026, 8, 1).at(9, 30, 0, 0).to_zoned(tz()).unwrap();
        let after = date(2026, 8, 1).at(10, 30, 0, 0).to_zoned(tz()).unwrap();

        let marked = |now: &Zoned| match &AgendaPanel::rows(&state, now, true, 60)[1] {
            Row::Event { in_progress, .. } => *in_progress,
            Row::Day(_) => unreachable!("row 1 is the event"),
        };
        assert!(marked(&during), "the meeting you are in must stand out");
        assert!(!marked(&after));
    }

    #[test]
    fn an_all_day_event_says_so_instead_of_showing_midnight() {
        let theme = crate::theme::Theme::default();
        let e = event(date(2026, 8, 1), 0, "Holiday", true);
        let line = event_line(&e, false, true, 60, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("all day"), "got `{text}`");
        assert!(!text.contains("00:00"), "midnight is not a time here");
    }

    #[test]
    fn a_row_never_outgrows_the_panel() {
        // Invariant 9: the budget is display cells, and a CJK summary is the
        // case that catches a `chars()` count.
        let theme = crate::theme::Theme::default();
        for summary in [
            "short",
            "an extremely long summary that will not fit in a narrow panel at all",
            "日本語のとても長い予定のタイトルです",
        ] {
            for width in [10u16, 20, 40, 80] {
                let mut e = event(date(2026, 8, 1), 9, summary, false);
                e.location = Some("Room 12, second floor".into());
                let line = event_line(&e, false, true, width, &theme);
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert!(
                    crate::grid::display_width(&text) <= usize::from(width),
                    "{width} cells: `{text}` is {}",
                    crate::grid::display_width(&text)
                );
            }
        }
    }

    #[test]
    fn a_location_is_dropped_rather_than_squeezing_the_summary_out() {
        let theme = crate::theme::Theme::default();
        let mut e = event(date(2026, 8, 1), 9, "Design review", false);
        e.location = Some("The very long name of a meeting room".into());
        let narrow: String = event_line(&e, false, true, 30, &theme)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(narrow.contains("Design"), "the summary lost: `{narrow}`");
        assert!(!narrow.contains("very long name"), "got `{narrow}`");

        let wide: String = event_line(&e, false, true, 70, &theme)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(wide.contains("meeting room"), "got `{wide}`");
    }
}
