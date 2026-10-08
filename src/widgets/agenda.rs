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
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Zoned};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span as TextSpan};
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::config::AgendaConfig;
use crate::frame::{Binding, FRAME_HEIGHT, FRAME_WIDTH};
use crate::ical;
use crate::keymap::{KeysConfig, Meta, PanelKeymap};
use crate::panel::{KeyOutcome, Panel, RenderContext, describe_age};

/// The status shown while a reload is in flight.
///
/// Named rather than repeated because `tick` has to recognise it: a reload that
/// has landed must take its own message down, and only its own. A path put up
/// by `o` has to survive the next background read.
const RELOADING: &str = "reloading…";

/// What the agenda's keys do. The file dialog keeps its own keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgendaAction {
    File,
    Reload,
    Up,
    Down,
    First,
    Last,
    PageUp,
    PageDown,
    ShowPath,
}

/// Every key the panel responds to, under `[agenda.keys]`. The border hint, the
/// status bar and the help overlay are derived from it, so a key the panel
/// reads is a key it advertises.
pub const ACTIONS: &[Meta<AgendaAction>] = &[
    Meta {
        action: AgendaAction::File,
        name: "file",
        defaults: &[(KeyCode::Char('f'), KeyModifiers::NONE)],
        label: "file",
        primary: true,
        joins: false,
        about: "choose the calendar file",
    },
    Meta {
        action: AgendaAction::Reload,
        name: "reload",
        defaults: &[(KeyCode::Char('r'), KeyModifiers::NONE)],
        label: "reload",
        primary: true,
        joins: false,
        about: "read the calendar file again",
    },
    Meta {
        action: AgendaAction::Up,
        name: "up",
        defaults: &[
            (KeyCode::Up, KeyModifiers::NONE),
            (KeyCode::Char('k'), KeyModifiers::NONE),
        ],
        label: "scroll",
        primary: false,
        joins: false,
        about: "scroll up",
    },
    Meta {
        action: AgendaAction::Down,
        name: "down",
        defaults: &[
            (KeyCode::Down, KeyModifiers::NONE),
            (KeyCode::Char('j'), KeyModifiers::NONE),
        ],
        label: "scroll",
        primary: false,
        joins: true,
        about: "scroll down",
    },
    Meta {
        action: AgendaAction::First,
        name: "first",
        defaults: &[
            (KeyCode::Char('g'), KeyModifiers::NONE),
            (KeyCode::Home, KeyModifiers::NONE),
        ],
        label: "first",
        primary: false,
        joins: false,
        about: "scroll to the first event",
    },
    Meta {
        action: AgendaAction::Last,
        name: "last",
        defaults: &[
            (KeyCode::Char('G'), KeyModifiers::NONE),
            (KeyCode::End, KeyModifiers::NONE),
        ],
        label: "last",
        primary: false,
        joins: true,
        about: "scroll to the last event",
    },
    Meta {
        action: AgendaAction::PageUp,
        name: "page_up",
        defaults: &[(KeyCode::PageUp, KeyModifiers::NONE)],
        label: "scroll ten rows",
        primary: false,
        joins: false,
        about: "scroll ten rows up",
    },
    Meta {
        action: AgendaAction::PageDown,
        name: "page_down",
        defaults: &[(KeyCode::PageDown, KeyModifiers::NONE)],
        label: "scroll ten rows",
        primary: false,
        joins: true,
        about: "scroll ten rows down",
    },
    Meta {
        action: AgendaAction::ShowPath,
        name: "show_path",
        defaults: &[(KeyCode::Char('o'), KeyModifiers::NONE)],
        label: "show file path",
        primary: false,
        joins: false,
        about: "show which calendar file is read",
    },
];

/// `[agenda.keys]` laid over [`ACTIONS`], or why it cannot be.
pub fn keymap(keys: &KeysConfig) -> Result<PanelKeymap<AgendaAction>, String> {
    PanelKeymap::new("agenda", ACTIONS, keys)
}

/// The keys the panel starts with, until `build` hands it the config's
/// through [`Panel::set_keys`].
fn default_keys() -> PanelKeymap<AgendaAction> {
    PanelKeymap::defaults("agenda", ACTIONS)
}

/// How close an event has to be before it is worth interrupting for.
///
/// Ten minutes is about the time it takes to finish a thought, find the link
/// and get there. Much longer and the signal is lit for a large part of a
/// working day, which is the failure this whole feature is built to avoid.
const IMMINENT: std::time::Duration = std::time::Duration::from_mins(10);

/// Width at which the panel stops gaining anything: a time, a generous summary
/// and a location beside it.
const USEFUL_WIDTH: u16 = 52;

/// Why the panel has no events.
///
/// Separated because the two want opposite treatments. A panel nobody has
/// pointed at a file is not broken — it is a panel nobody has set up, the same
/// standing as an empty task list — and painting it red teaches the reader to
/// ignore red.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Trouble {
    /// `[agenda].file` names nothing that exists.
    NoFile,
    /// The file is there and reading it went wrong.
    Unreadable(String),
}

/// What the reader thread has produced.
#[derive(Debug, Default)]
struct State {
    events: Vec<ical::Event>,
    /// Why there is nothing to show, if there is nothing to show.
    error: Option<Trouble>,
    /// Lines the parser could not make sense of, so a half-read calendar says
    /// so rather than looking merely empty.
    skipped: usize,
    /// When this was read.
    read_at: Option<Instant>,
    /// The day the window was built around. The reader works it out afresh on
    /// every pass, and `tick` compares it across reads, which is how a
    /// dashboard left open overnight notices that "today" moved.
    built_for: Option<Date>,
    /// Where the window ended: events from here on were not read at all, so
    /// the next read finding one there has not found anything new.
    built_until: Option<jiff::Timestamp>,
    /// The file this was read from. Set by the reader rather than looked up
    /// by the panel, because a read already under way when `f` swaps the path
    /// still lands afterwards, and it describes the calendar it read.
    source: Option<PathBuf>,
}

#[derive(Debug)]
pub struct AgendaPanel {
    /// `[agenda.keys]` over the defaults.
    keys: PanelKeymap<AgendaAction>,
    state: Arc<Mutex<State>>,
    /// Set to ask the reader thread for an immediate re-read.
    reload: Arc<Mutex<bool>>,
    generation: Arc<AtomicU64>,
    seen: u64,
    stop: Arc<AtomicBool>,
    /// Shared with the reader thread, which re-reads it every cycle, so
    /// pointing the panel at a different calendar is a swap here and a reload
    /// rather than tearing the thread down and starting another.
    path: Arc<Mutex<PathBuf>>,
    days: u16,
    show_location: bool,
    /// The first row on screen, moved by the scroll keys and the wheel.
    offset: usize,
    /// How far `offset` may go, as of the last draw: the rows that did not fit.
    ///
    /// Bounded by what was drawn, the rule `news` and `watchlog` follow,
    /// because only `render` knows the panel's height. It was a `ListState`
    /// whose selection the keys moved and the list was never drawn with, so
    /// the agenda never scrolled at all.
    max_offset: usize,
    status: Option<String>,
    /// The `f` dialog, while it is open.
    asking: Option<crate::prompt::Prompt>,
    /// What the calendar held at the last read, so a new entry can be spotted.
    ///
    /// `None` until the first successful read. That is what stops the whole
    /// calendar being announced at startup: on a first read there is nothing to
    /// have changed *from*, and a log opening with forty entries is a log
    /// nobody reads twice.
    known: Option<std::collections::HashSet<String>>,
    /// Where the window of the read behind `known` ended.
    ///
    /// The reader builds its window from today on every pass, so each
    /// midnight brings a day into it that the last read never looked at. An
    /// event from here on is new to the window, not to the calendar, and
    /// announcing it logged the next instance of every repeating meeting as
    /// having appeared, every night.
    known_until: Option<jiff::Timestamp>,
    /// The file `known` was read from. A read of any other — another calendar,
    /// chosen with `f` — has nothing to have changed *from*, and is no more
    /// news than the first read at startup.
    known_from: Option<PathBuf>,
    /// Events waiting to be drained by the watch log.
    pending: Vec<crate::watch::Event>,
    /// What the reader thread had published at the last tick.
    ///
    /// Copied once when the generation moves, rather than on every draw. The
    /// panel used to call `snapshot` from `render` and from two key handlers —
    /// two of those cloning the entire event list only to read its `len` —
    /// which measured at 210,000 event clones in thirty idle seconds against a
    /// calendar of three hundred daily meetings. A recurring rule expands, so
    /// the list is far longer than the file suggests.
    shown: State,
    /// The clock at the last tick, so the next can tell what it has passed.
    checked: Option<Zoned>,
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
        let path = Arc::new(Mutex::new(path));
        let thread_path = Arc::clone(&path);
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
            keys: default_keys(),
            state,
            reload,
            generation,
            seen: 0,
            stop,
            path,
            days,
            show_location: config.show_location,
            offset: 0,
            max_offset: 0,
            status: None,
            asking: None,
            known: None,
            known_until: None,
            known_from: None,
            pending: Vec::new(),
            shown: State::default(),
            checked: None,
        }
    }

    /// Deal with a keypress while the file prompt is open.
    ///
    /// The answer is checked before it is taken. A path that cannot be read is
    /// almost always a typo, and accepting it would replace a working calendar
    /// with an error message and no way back to what was there — so the prompt
    /// stays open with the text in it and says what went wrong.
    ///
    /// An empty answer is allowed through, and means "no calendar": that is how
    /// you undo this without having to remember the path you started with.
    fn handle_prompt_key(&mut self, key: KeyEvent) {
        let Some(prompt) = self.asking.as_mut() else {
            return;
        };
        match prompt.handle_key(key) {
            // `Chose` cannot arise: this prompt offers no list. It is grouped
            // with `Editing` rather than given its own empty arm because an
            // arm that does nothing invites someone to make it do something.
            crate::prompt::Outcome::Editing | crate::prompt::Outcome::Chose { .. } => {}
            crate::prompt::Outcome::Cancelled => self.asking = None,
            crate::prompt::Outcome::Submitted(answer) => {
                let path = crate::prompt::expand_tilde(&answer);
                if !answer.is_empty()
                    && let Err(e) = std::fs::metadata(&path)
                {
                    prompt.reject(format!("{e}"));
                    return;
                }
                self.set_path(path);
                self.asking = None;
                self.status = Some(RELOADING.into());
            }
        }
    }

    /// Record anything in the calendar that was not there at the last read.
    ///
    /// This is the clearest case the watch log has: an entry you did not add,
    /// which appeared because somebody else put it in a calendar you sync. It
    /// is worth knowing whether or not you were looking at this panel, which is
    /// the whole test for whether something belongs in the log.
    fn note_new_entries(&mut self) {
        let state = &self.shown;
        // A failed read publishes no events; treating that as "everything was
        // cancelled" and then "everything is new" would fill the log with a
        // network blip.
        if state.error.is_some() {
            return;
        }

        let current: std::collections::HashSet<String> = state
            .events
            .iter()
            .map(|event| format!("{}@{}", event.summary, event.start.timestamp()))
            .collect();

        let horizon = self.known_until.take();
        let known = self
            .known
            .take()
            .filter(|_| self.known_from == state.source);
        if let Some(known) = known {
            for event in &state.events {
                // Past the last window: out of sight then, not absent.
                if horizon.is_some_and(|until| event.start.timestamp() >= until) {
                    continue;
                }
                let key = format!("{}@{}", event.summary, event.start.timestamp());
                if !known.contains(&key) {
                    self.pending.push(crate::watch::Event::new(
                        "agenda",
                        format!(
                            "{} appeared in your calendar, {}",
                            event.summary,
                            event.start.strftime("%a %d %b at %H:%M")
                        ),
                    ));
                }
            }
        }
        self.known = Some(current);
        self.known_until = state.built_until;
        self.known_from.clone_from(&state.source);
    }

    /// Point the panel at a different calendar and read it now.
    ///
    /// Its first read is as quiet as startup's, which is decided by
    /// `known_from` when the read lands rather than by clearing anything here.
    pub fn set_path(&mut self, to: PathBuf) {
        *self.path.lock().unwrap_or_else(PoisonError::into_inner) = to;
        // Another calendar starts at its top, not wherever this one was left.
        self.offset = 0;
        self.ask_for_reload();
    }

    fn snapshot(&self) -> State {
        let guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        State {
            events: guard.events.clone(),
            error: guard.error.clone(),
            skipped: guard.skipped,
            read_at: guard.read_at,
            built_for: guard.built_for,
            built_until: guard.built_until,
            source: guard.source.clone(),
        }
    }

    fn ask_for_reload(&self) {
        *self.reload.lock().unwrap_or_else(PoisonError::into_inner) = true;
    }

    /// The rows to draw: a heading per day, then that day's events.
    ///
    /// A flat sequence rather than a tree because a heading and an event
    /// scroll together, one row each, which is all the offset needs.
    ///
    /// An iterator over borrowed events, not a `Vec` of cloned ones. `render`
    /// counts it to bound the scroll and then builds lines for the rows on
    /// screen only, so a frame costs what fits in the panel rather than what is
    /// in the calendar — it used to clone and format every event in the window,
    /// once a second.
    fn rows<'a>(state: &'a State, now: &'a Zoned) -> impl Iterator<Item = Row<'a>> + 'a {
        let mut current: Option<Date> = None;
        state.events.iter().flat_map(move |event| {
            let day = event.start.date();
            let heading = (current != Some(day)).then(|| {
                current = Some(day);
                Row::Day(day)
            });
            heading.into_iter().chain(std::iter::once(Row::Event {
                event,
                in_progress: event.contains(now),
            }))
        })
    }

    fn scroll_down(&mut self, rows: usize) {
        self.offset = self.offset.saturating_add(rows).min(self.max_offset);
    }

    fn scroll_up(&mut self, rows: usize) {
        self.offset = self.offset.saturating_sub(rows);
    }

    /// [`Panel::tick`] at `now`, so a test can say when it is.
    ///
    /// Two things change what the panel draws. A read landing, counted by
    /// the generation — and the day rolling over arrives that way too, since
    /// the reader works out today on every pass and the headings follow the
    /// day it read for, not the clock. And the clock passing a moment that
    /// moves the `▸` marker or the status bar's countdown, both worked out at
    /// draw time: an event starting or ending, or a minute of the countdown
    /// going by. The second used to go unreported, so on a dashboard with
    /// nothing else redrawing a meeting started unmarked until the next read
    /// (invariant 13).
    fn tick_at(&mut self, clock: &Zoned) -> bool {
        // Every shown event is weighed at the last tick and at this one, so
        // nothing between the two can be missed however far apart they fall.
        // A walk over the cached copy with nothing allocated, every twenty
        // seconds: the cost is in proportion to the calendar, but it is a
        // comparison per event, not a clone.
        let passed = self.checked.as_ref().is_some_and(|then| {
            self.shown
                .events
                .iter()
                .any(|event| standing(event, then) != standing(event, clock))
        });
        self.checked = Some(clock.clone());

        let now = self.generation.load(Ordering::Acquire);
        let moved = now != self.seen;
        self.seen = now;
        if moved {
            // One copy, at the one moment the data can have changed.
            let before = self.shown.built_for;
            self.shown = self.snapshot();
            // A new day is a new agenda. A scroll left over from yesterday
            // would hide the top of today's — the events happening now — on a
            // dashboard left open overnight.
            if self.shown.built_for != before {
                self.offset = 0;
            }
            self.note_new_entries();
            // The generation only moves when a read lands, so this is where a
            // reload finishes. Taking the message down here rather than on the
            // next keypress matters because a dashboard is read without being
            // touched: left up, an idle panel claims to be mid-operation, which
            // reads as a hang rather than as a stale label.
            //
            // Only its own message. A path put up by `o` has to survive the
            // next background read.
            if self.status.as_deref() == Some(RELOADING) {
                self.status = None;
            }
        }
        moved || passed
    }
}

/// How long until `event` starts, while that is soon enough for the status
/// bar to say so — or `None` for an all-day event, one already under way, and
/// one further off than [`IMMINENT`].
///
/// The one place the alert's window is decided, so `alert` and `tick` cannot
/// disagree about when it opens.
fn countdown(event: &ical::Event, now: &Zoned) -> Option<std::time::Duration> {
    if event.all_day {
        return None;
    }
    std::time::Duration::try_from(event.start.duration_since(now))
        .ok()
        .filter(|until| *until <= IMMINENT)
}

/// What the clock decides about `event` at `now`: whether it carries the `▸`
/// marker, and the whole minutes the status bar would count down to it.
///
/// Two instants with the same standing draw the event the same way, so a
/// tick need only redraw when some event's standing has changed since the
/// last one.
fn standing(event: &ical::Event, now: &Zoned) -> (bool, Option<u64>) {
    (
        event.contains(now),
        countdown(event, now).map(|until| until.as_secs() / 60),
    )
}

enum Row<'a> {
    Day(Date),
    Event {
        event: &'a ical::Event,
        in_progress: bool,
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

/// Largest `.ics` that will be read.
///
/// The network side of this program is bounded — `ureq` stops at 10MB — and the
/// local side was not bounded at all. A calendar is a file somebody else's
/// software writes, it grows without anyone deciding to, and reading it costs
/// more than its size: unfolding turns it into a `Vec<String>`, and a recurring
/// rule expands further still. Matching the network figure keeps one number to
/// remember.
///
/// A year of a busy calendar is a few megabytes, so this refuses nothing real.
const MAX_CALENDAR: u64 = 10 * 1024 * 1024;

/// Read the calendar, refusing one too large to be a calendar.
///
/// Checked before reading rather than after: the point is not to notice that
/// something enormous was loaded, it is not to load it.
fn read_calendar(path: &std::path::Path) -> std::io::Result<String> {
    let size = std::fs::metadata(path)?.len();
    if size > MAX_CALENDAR {
        return Err(std::io::Error::other(format!(
            "the calendar is {} MB, over the {} MB limit — mirador reads a \
             calendar into memory, so it will not open one this large",
            size / (1024 * 1024),
            MAX_CALENDAR / (1024 * 1024)
        )));
    }
    std::fs::read_to_string(path)
}

/// The path as it stands, which the panel may have changed since the last pass.
fn current_path(path: &Arc<Mutex<PathBuf>>) -> PathBuf {
    path.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Read the file, parse it, and publish — then wait, and do it again.
fn read_loop(
    path: &Arc<Mutex<PathBuf>>,
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

        let source = current_path(path);
        let next = match (from, until) {
            (Some(from), Some(until)) => match read_calendar(&source) {
                Ok(text) => {
                    let calendar = ical::parse(&text, &tz, &from, &until);
                    State {
                        events: calendar.events,
                        error: None,
                        skipped: calendar.skipped.len(),
                        read_at: Some(Instant::now()),
                        built_for: Some(today),
                        built_until: Some(until.timestamp()),
                        source: Some(source),
                    }
                }
                // A missing file is the unconfigured case, not a failure.
                // Anything else — a permission problem, a directory where a
                // file should be — keeps the message the OS gave, which is the
                // one that says what to do about it.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => State {
                    error: Some(Trouble::NoFile),
                    read_at: Some(Instant::now()),
                    built_for: Some(today),
                    ..State::default()
                },
                Err(e) => State {
                    error: Some(Trouble::Unreadable(format!("{e}"))),
                    read_at: Some(Instant::now()),
                    built_for: Some(today),
                    ..State::default()
                },
            },
            _ => State {
                error: Some(Trouble::Unreadable(
                    "could not work out today's date".into(),
                )),
                read_at: Some(Instant::now()),
                built_for: Some(today),
                ..State::default()
            },
        };

        *state.lock().unwrap_or_else(PoisonError::into_inner) = next;
        generation.fetch_add(1, Ordering::Release);

        let woke = crate::poll::wait(interval, stop, || {
            std::mem::take(&mut *reload.lock().unwrap_or_else(PoisonError::into_inner))
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
        // Reads the cache, not the mutex. `counter` is called on *every* frame
        // by the frame renderer, with nothing guarding it — cloning an expanded
        // recurring calendar here was the most expensive of the three.
        let state = &self.shown;
        match &state.error {
            Some(Trouble::NoFile) => return Some("not set up".into()),
            Some(Trouble::Unreadable(_)) => return Some("unreadable".into()),
            None => {}
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

    fn bindings(&self) -> &[Binding] {
        self.keys.bindings()
    }

    fn set_keys(&mut self, config: &crate::config::Config) {
        self.keys = PanelKeymap::or_defaults("agenda", ACTIONS, &config.agenda.keys);
    }

    fn max_width(&self) -> Option<u16> {
        // Past this the summaries stop being the constraint and the row just
        // grows a gap in the middle; the graphs next door can use it.
        Some(USEFUL_WIDTH + FRAME_WIDTH)
    }

    fn refresh_interval(&self) -> Duration {
        // The reader thread owns the real cadence. This only decides how
        // quickly a completed read reaches the screen.
        Duration::from_secs(20)
    }

    fn tick(&mut self) -> bool {
        self.tick_at(&Zoned::now())
    }

    fn alert(&self) -> Option<crate::panel::Alert> {
        let now = Zoned::now();

        // Reads the events where they lie rather than through `snapshot`,
        // which clones the whole list — and this runs on every draw, measured
        // at 39 calls in thirty idle seconds, so a twelve-event calendar cloned
        // 456 events, each with one or two `String`s, for nothing. A real week
        // of meetings would be several times that, for ever. Panel::alert's own
        // documentation says it must not allocate when it has nothing to say,
        // which is nearly always; this is that promise kept.
        //
        // It still reads them under the lock, which predates `shown`: where
        // `counter` and `render` read the copy taken at the last tick, this
        // sees a read the moment it lands, so for up to a tick afterwards the
        // status bar can name an event the panel has not drawn yet.
        let guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.error.is_some() {
            return None;
        }

        // The nearest event that has not started and starts within the window.
        // An event already under way is deliberately not an alert: you are
        // either in it or you have missed it, and a dashboard telling you about
        // a meeting you are sitting in is noise.
        let (until, event) = guard
            .events
            .iter()
            .filter_map(|event| countdown(event, &now).map(|until| (until, event)))
            .min_by_key(|(until, _)| *until)?;

        let minutes = until.as_secs() / 60;
        let when = if minutes == 0 {
            "now".to_string()
        } else {
            format!("in {minutes}m")
        };
        Some(match &event.location {
            Some(place) => crate::panel::Alert::soon(format!("{} {when} · {place}", event.summary)),
            None => crate::panel::Alert::soon(format!("{} {when}", event.summary)),
        })
    }

    fn events(&mut self) -> Vec<crate::watch::Event> {
        std::mem::take(&mut self.pending)
    }

    fn overlay(&self) -> Option<&crate::prompt::Prompt> {
        self.asking.as_ref()
    }

    fn captures_input(&self) -> bool {
        self.asking.is_some()
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        if self.asking.is_some() {
            self.handle_prompt_key(key);
            return KeyOutcome::Consumed;
        }

        self.status = None;
        let Some(action) = self.keys.action(key) else {
            return KeyOutcome::Ignored;
        };
        match action {
            AgendaAction::File => {
                self.asking = Some(crate::prompt::Prompt::new(
                    "AGENDA FILE",
                    "Tab completes · Enter saves · Esc cancels",
                    &current_path(&self.path).display().to_string(),
                    crate::prompt::Completion::Paths,
                ));
            }
            AgendaAction::Reload => {
                self.ask_for_reload();
                self.status = Some(RELOADING.into());
            }
            AgendaAction::ShowPath => {
                self.status = Some(current_path(&self.path).display().to_string());
            }
            AgendaAction::Down => self.scroll_down(1),
            AgendaAction::Up => self.scroll_up(1),
            AgendaAction::PageDown => self.scroll_down(10),
            AgendaAction::PageUp => self.scroll_up(10),
            AgendaAction::Last => self.scroll_down(usize::MAX),
            AgendaAction::First => self.scroll_up(usize::MAX),
        }
        KeyOutcome::Consumed
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> KeyOutcome {
        match event.kind {
            MouseEventKind::ScrollDown => self.scroll_down(1),
            MouseEventKind::ScrollUp => self.scroll_up(1),
            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.width == 0 || area.height == 0 {
            return;
        }

        let state = &self.shown;
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
        // Wrapped, for the reason the empty state's path is: `o` puts up the
        // calendar's path, which is the whole message, and as one `Line` the
        // terminal cut it at the edge — the default macOS path lost its
        // filename at every width this panel takes.
        if let Some(message) = &self.status {
            notices.extend(wrapped_lines(
                message,
                area.width,
                Style::default().fg(theme.muted),
            ));
        }

        let (list_area, notice_area) =
            split_for_notices(area, u16::try_from(notices.len()).unwrap_or(u16::MAX));
        // The list keeps a row, so a short panel can have fewer rows for the
        // notices than they wrapped to. The last one it has says so, rather
        // than the rows past it going in silence (invariant 19).
        let fits = usize::from(notice_area.height);
        if notices.len() > fits {
            notices.truncate(fits);
            if let Some(last) = notices.last_mut() {
                let text: String = last
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect();
                let style = last
                    .spans
                    .first()
                    .map_or_else(Style::default, |span| span.style);
                *last = Line::from(TextSpan::styled(
                    crate::grid::truncate(
                        &format!("{}…", text.trim_end()),
                        usize::from(area.width),
                    ),
                    style,
                ));
            }
        }

        // Bound the scroll against this frame's rows and height before drawing:
        // a re-read can shorten the calendar and a resize can lengthen the
        // panel, and either leaves the old offset showing blank rows.
        let rows = if state.error.is_some() {
            0
        } else {
            Self::rows(state, &now).count()
        };
        self.max_offset = rows.saturating_sub(usize::from(list_area.height));
        self.offset = self.offset.min(self.max_offset);
        self.draw_list(frame, list_area, state, &now, theme);

        if notice_area.height > 0 {
            frame.render_widget(Paragraph::new(notices), notice_area);
        }
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn remember(&self, state: &mut crate::state::UiState) {
        state.agenda_file = Some(current_path(&self.path).display().to_string());
    }
}

impl AgendaPanel {
    fn draw_list(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &State,
        now: &Zoned,
        theme: &crate::theme::Theme,
    ) {
        if let Some(trouble) = &state.error {
            let muted = Style::default().fg(theme.muted);
            let mut lines = match trouble {
                // Not an error, and not red. This is a panel nobody has set up
                // yet, which is an ordinary state — the same standing as an
                // empty task list.
                Trouble::NoFile => {
                    let mut lines = vec![
                        Line::from(TextSpan::styled(
                            "No agenda file",
                            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(TextSpan::styled("Nothing to show.", muted)),
                        Line::from(""),
                    ];
                    lines.extend(wrapped_lines(
                        "Set [agenda].file to an .ics you already have — an export, \
                         or whatever your calendar syncs to, or press f.",
                        area.width,
                        muted,
                    ));
                    lines.push(Line::from(""));
                    // Wrapped, not truncated. This line exists to tell you
                    // where to put the file, and a path cut off at the panel
                    // edge — `Looked in /var/folders/zj/blsvny` — fails at the
                    // one job the message has.
                    lines.extend(wrapped_lines(
                        &format!("Looked in {}", current_path(&self.path).display()),
                        area.width,
                        muted,
                    ));
                    lines
                }
                // This one *is* a fault: the file is there and something went
                // wrong reading it.
                Trouble::Unreadable(why) => {
                    let mut lines = vec![Line::from(TextSpan::styled(
                        "Cannot read the agenda file",
                        Style::default()
                            .fg(theme.error)
                            .add_modifier(Modifier::BOLD),
                    ))];
                    // The reason comes off the filesystem and can be any
                    // length; the path is the reader's own and can be deep.
                    // Neither is ours to truncate.
                    lines.extend(wrapped_lines(why, area.width, muted));
                    lines.push(Line::from(""));
                    lines.extend(wrapped_lines(
                        &current_path(&self.path).display().to_string(),
                        area.width,
                        muted,
                    ));
                    lines
                }
            };
            if let Some(age) = state.read_at.map(|at| describe_age(at.elapsed())) {
                lines.push(Line::from(TextSpan::styled(
                    format!("checked {age}"),
                    muted,
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
            // Wrapped, not hand-broken. Two lines written to fit a panel of the
            // author's imagination lose a word each in a narrower one, and the
            // pair still reads as a whole sentence — `Nothing schedule` above
            // `in the next 7 da` looks like a rendering glitch, where the same
            // words wrapped honestly just take four rows.
            let mut lines = wrapped_lines(
                "Nothing scheduled",
                area.width,
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            );
            lines.extend(wrapped_lines(
                &format!("in {horizon}."),
                area.width,
                Style::default().fg(theme.muted),
            ));
            frame.render_widget(Paragraph::new(lines), area);
            return;
        }

        self.draw_events(frame, area, state, now, theme);
    }

    /// The events themselves, a heading per day, from the scroll offset down.
    fn draw_events(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &State,
        now: &Zoned,
        theme: &crate::theme::Theme,
    ) {
        let today = state.built_for.unwrap_or_else(|| now.date());

        // Scrolled into a day, its heading stays on top, covering the row that
        // has just scrolled under it: an event on its own says `09:00` and not
        // which of seven days it is. Covering rather than pushing down keeps
        // every press moving the view, and keeps the last row reachable at the
        // same offset as without it.
        //
        // Not when the row below is the next day's heading. The covered event
        // was that day's last, and its heading over nothing would say the day
        // is empty, under a border counting its events; the event shows bare
        // for that one step instead. Nor in a one-row list, where the heading
        // would be all there ever was.
        let height = usize::from(area.height);
        let mut rows = Self::rows(state, now).skip(self.offset).peekable();
        let top = rows.next().map(|row| match row {
            Row::Event { event, .. }
                if self.offset > 0
                    && height >= 2
                    && matches!(rows.peek(), Some(Row::Event { .. })) =>
            {
                Row::Day(event.start.date())
            }
            row => row,
        });
        let items: Vec<ListItem> = top
            .into_iter()
            .chain(rows.take(height.saturating_sub(1)))
            .map(|row| match row {
                Row::Day(day) => ListItem::new(Line::from(TextSpan::styled(
                    crate::grid::truncate(
                        &crate::glyphs::utility(&day_label(day, today)),
                        usize::from(area.width),
                    ),
                    Style::default()
                        .fg(theme.label)
                        .add_modifier(Modifier::BOLD),
                ))),
                Row::Event { event, in_progress } => ListItem::new(event_line(
                    event,
                    in_progress,
                    self.show_location,
                    area.width,
                    theme,
                )),
            })
            .collect();

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

    // Cells, not bytes (invariant 9): `▸` is three bytes and one cell, and a
    // `len` here gave the row under way, the one the marker points at, two
    // cells less than the rows around it.
    let used =
        crate::grid::display_width(marker) + TIME_WIDTH.max(crate::grid::display_width(&time)) + 1;
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

    // The summary is already cut to the room left beside the time; the time
    // and its marker are a value of their own, and below nine columns they
    // drop whole rather than leaving `09:1` for the terminal to finish.
    crate::grid::assemble(
        vec![
            vec![
                TextSpan::styled(marker.to_string(), time_style),
                TextSpan::styled(format!("{time:<TIME_WIDTH$} "), time_style),
            ],
            vec![TextSpan::styled(
                crate::grid::truncate(&text, room),
                summary_style,
            )],
        ],
        width,
    )
}

/// One styled `Line` per row of `text` wrapped to `width`.
///
/// The empty-state messages used to be hand-wrapped into fixed lines and the
/// path printed as one long line, so both were cut at the panel edge by the
/// renderer. A message that says where to put your calendar file is worth
/// nothing if you cannot read the path, and prose broken for a 40-cell panel
/// reads badly in a 30-cell one.
fn wrapped_lines(text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    crate::grid::wrap(text, usize::from(width))
        .into_iter()
        .map(|row| Line::from(TextSpan::styled(row, style)))
        .collect()
}

/// Rows the frame costs, re-exported so `max_height` reads the same as the
/// other panels even though this one does not declare a maximum.
const _: u16 = FRAME_HEIGHT;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    /// Every key in the map works and is advertised; see
    /// [`crate::keymap::assert_every_key_works`].
    #[test]
    fn every_key_in_the_map_works_and_is_advertised() {
        crate::keymap::assert_every_key_works(&default_keys(), |event| {
            idle_panel().handle_key(event)
        });
    }
    use jiff::civil::date;

    /// `f` asks for a path in place. A path that does not exist is refused
    /// *inside the prompt*, keeping what was typed; Esc backs out; a path
    /// that exists is taken and a reload is asked for. None of
    /// `handle_prompt_key`, `set_path` or `ask_for_reload` had been executed
    /// by a test.
    #[test]
    fn f_asks_for_a_path_and_a_missing_file_is_refused_where_it_was_typed() {
        let dir = TempDir::new("agenda-prompt");
        let ics = dir.join("cal.ics");
        let mut panel = AgendaPanel::new(&AgendaConfig::default(), ics.clone());
        let key = |code| KeyEvent::new(code, ratatui::crossterm::event::KeyModifiers::NONE);

        panel.handle_key(key(KeyCode::Char('f')));
        assert!(panel.asking.is_some(), "`f` opens the prompt");
        panel.handle_key(key(KeyCode::Enter));
        let prompt = panel
            .asking
            .as_ref()
            .expect("a missing file keeps the prompt open");
        assert_eq!(
            prompt.value(),
            ics.display().to_string(),
            "and keeps what was typed"
        );
        panel.handle_key(key(KeyCode::Esc));
        assert!(panel.asking.is_none(), "Esc backs out");

        std::fs::write(&ics, "BEGIN:VCALENDAR\nEND:VCALENDAR\n").unwrap();
        panel.handle_key(key(KeyCode::Char('f')));
        panel.handle_key(key(KeyCode::Enter));
        assert!(panel.asking.is_none(), "a file that exists is taken");
        assert_eq!(
            panel.status.as_deref(),
            Some(RELOADING),
            "and a reload is under way"
        );
        assert_eq!(current_path(&panel.path), ics);
    }

    /// The scroll keys and the wheel have to move what is on screen. Until
    /// 1.19.2 they moved a `ListState` the list was never drawn with, and the
    /// test that covered them asserted that state rather than the screen — so
    /// it passed with nothing moving, and every event below the bottom of the
    /// panel was out of reach. This one reads the rendered rows.
    #[test]
    fn scrolling_moves_what_is_on_screen_and_stops_at_both_ends() {
        let mut panel =
            AgendaPanel::new(&AgendaConfig::default(), PathBuf::from("/nonexistent.ics"));
        let today = Zoned::now().date();
        let tomorrow = today.tomorrow().expect("a tomorrow");
        // Two headings and twelve events: fourteen rows in a panel of six.
        panel.shown.events = (0..12)
            .map(|i: i8| {
                let day = if i < 6 { today } else { tomorrow };
                event(day, 8 + i % 6, &format!("event {i:02}"), false)
            })
            .collect();
        panel.shown.built_for = Some(today);
        let key = |panel: &mut AgendaPanel, code| {
            panel.handle_key(KeyEvent::from(code));
        };
        let wheel = |panel: &mut AgendaPanel, kind| {
            let event = MouseEvent {
                kind,
                column: 1,
                row: 1,
                modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            };
            panel.handle_mouse(event, Rect::new(0, 0, 40, 6));
        };

        let first = screen(&mut panel, 40, 6);
        assert!(first[1].contains("event 00"), "{first:#?}");
        assert!(
            !first.iter().any(|row| row.contains("event 05")),
            "{first:#?}"
        );

        key(&mut panel, KeyCode::Char('j'));
        let moved = screen(&mut panel, 40, 6);
        assert!(
            moved[0].contains("TODAY"),
            "the day stays on top: {moved:#?}"
        );
        assert!(
            moved[1].contains("event 01"),
            "j moves the view a row: {moved:#?}"
        );
        assert!(moved[5].contains("event 05"), "{moved:#?}");

        for _ in 0..50 {
            wheel(&mut panel, MouseEventKind::ScrollDown);
        }
        let bottom = screen(&mut panel, 40, 6);
        assert!(
            bottom[5].contains("event 11"),
            "the last event is reachable: {bottom:#?}"
        );
        assert!(
            bottom[0].contains("TOMORROW"),
            "under its day's heading: {bottom:#?}"
        );
        assert!(
            bottom[1].contains("event 07"),
            "and the view stops there: {bottom:#?}"
        );

        key(&mut panel, KeyCode::Char('j'));
        assert_eq!(
            screen(&mut panel, 40, 6),
            bottom,
            "past the end, nothing moves"
        );

        key(&mut panel, KeyCode::Char('g'));
        assert_eq!(screen(&mut panel, 40, 6), first, "g returns to the top");

        key(&mut panel, KeyCode::PageDown);
        assert_eq!(
            screen(&mut panel, 40, 6),
            bottom,
            "ten rows is past the end here"
        );

        for _ in 0..50 {
            wheel(&mut panel, MouseEventKind::ScrollUp);
        }
        assert_eq!(
            screen(&mut panel, 40, 6),
            first,
            "and the wheel stops at the top"
        );

        key(&mut panel, KeyCode::Char('G'));
        assert_eq!(
            screen(&mut panel, 40, 6),
            bottom,
            "G goes to the last event"
        );
    }

    /// At every step from the top to the bottom, no heading sits directly on
    /// another. A day's heading kept on top over its last event would have read
    /// as "nothing today" above `TOMORROW`, under a border counting today's
    /// events; at that one step the event shows instead.
    #[test]
    fn a_heading_never_stands_over_nothing_while_scrolling() {
        let mut panel =
            AgendaPanel::new(&AgendaConfig::default(), PathBuf::from("/nonexistent.ics"));
        let today = Zoned::now().date();
        let tomorrow = today.tomorrow().expect("a tomorrow");
        panel.shown.events = (0..8)
            .map(|i: i8| {
                let day = if i < 4 { today } else { tomorrow };
                event(day, 8 + i % 4, &format!("event {i:02}"), false)
            })
            .collect();
        panel.shown.built_for = Some(today);
        let heading = |row: &str| row.contains("TODAY") || row.contains("TOMORROW");

        for height in 2..=6 {
            panel.handle_key(KeyEvent::from(KeyCode::Char('g')));
            for step in 0..12 {
                let rows = screen(&mut panel, 40, height);
                for pair in rows.windows(2) {
                    assert!(
                        !(heading(&pair[0]) && heading(&pair[1])),
                        "height {height}, step {step}: {rows:#?}"
                    );
                }
                panel.handle_key(KeyEvent::from(KeyCode::Char('j')));
            }
        }
    }

    /// A scroll is bounded again on every draw, because the panel can grow and
    /// the calendar can shrink under it. Left alone, an offset past the new
    /// end shows blank rows with the top of the agenda out of reach of `k`.
    #[test]
    fn a_taller_panel_or_a_shorter_calendar_pulls_the_scroll_back() {
        let mut panel =
            AgendaPanel::new(&AgendaConfig::default(), PathBuf::from("/nonexistent.ics"));
        let today = Zoned::now().date();
        panel.shown.events = (0..12)
            .map(|i: i8| event(today, 8 + i, &format!("event {i:02}"), false))
            .collect();
        panel.shown.built_for = Some(today);
        screen(&mut panel, 40, 6);
        panel.handle_key(KeyEvent::from(KeyCode::Char('G')));
        screen(&mut panel, 40, 6);

        let taller = screen(&mut panel, 40, 13);
        assert!(taller[0].contains("TODAY"), "{taller:#?}");
        assert!(
            taller[1].contains("event 00"),
            "all of it fits again: {taller:#?}"
        );

        panel.handle_key(KeyEvent::from(KeyCode::Char('G')));
        screen(&mut panel, 40, 6);
        panel.shown.events.truncate(3);
        let shorter = screen(&mut panel, 40, 6);
        assert!(shorter[1].contains("event 00"), "{shorter:#?}");
        assert!(shorter[3].contains("event 02"), "{shorter:#?}");
    }

    /// The panel drawn at `width` by `height`, one string per row.
    fn screen(panel: &mut AgendaPanel, width: u16, height: u16) -> Vec<String> {
        crate::widgets::testing::rows(&crate::widgets::testing::rendered(panel, width, height))
    }

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

    /// The empty-state message exists to say where to put your calendar, so a
    /// path cut off at the panel edge fails at the only job it has. It used to
    /// be one long `Line` and the renderer clipped it — the dashboard showed
    /// `Looked in /var/folders/zj/blsvny` and stopped.
    ///
    /// Asserted on the drawn buffer rather than the lines, because clipping
    /// happens at draw time: a test that inspected the `Line` would pass with
    /// the defect in place, which is a trap this repository has fallen into
    /// twice.
    #[test]
    fn the_path_in_the_empty_message_is_wrapped_rather_than_cut_off() {
        use crate::panel::RenderContext;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let dir = TempDir::new("agendapath");
        let missing = dir.join("no-such-calendar.ics");

        let theme = crate::theme::Theme::default();
        let gradients = theme.gradients();

        for width in [24u16, 30, 40, 60] {
            let mut panel =
                AgendaPanel::new(&crate::config::AgendaConfig::default(), missing.clone());
            // Let the reader thread notice the file is absent.
            for _ in 0..50 {
                if panel.tick() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }

            let mut terminal = Terminal::new(TestBackend::new(width, 24)).expect("backend");
            terminal
                .draw(|f| {
                    panel.render(
                        f,
                        Rect::new(0, 0, width, 24),
                        RenderContext {
                            theme: &theme,
                            gradients: &gradients,
                            focused: true,
                            watch: &crate::watch::WatchLog::default(),
                        },
                    );
                })
                .expect("draws");

            let rows = crate::widgets::testing::rows(terminal.backend().buffer());
            let screen = rows.join("\n");

            // Nothing may be drawn wider than the panel.
            for row in &rows {
                assert!(
                    crate::grid::display_width(row.trim_end()) <= usize::from(width),
                    "a row overflowed at width {width}: {row:?}"
                );
            }

            // The end of the path has to survive. A clipped path stops before
            // its filename, which is exactly the part that tells you what to
            // create.
            let joined: String = rows.iter().map(|r| r.trim_end()).collect();
            assert!(
                joined.contains("no-such-calendar.ics"),
                "the path was cut before its filename at width {width}:\n{screen}"
            );
        }
    }

    /// A calendar is written by somebody else's software and grows without
    /// anyone deciding to. Reading it costs more than its size — unfolding
    /// makes a `Vec<String>` of it and a recurring rule expands further — so
    /// the size is checked before the read rather than regretted after it.
    #[test]
    fn a_calendar_too_large_to_be_a_calendar_is_refused_before_it_is_read() {
        let dir = TempDir::new("ics");
        let path = dir.join("big.ics");

        // Sparse where the filesystem allows it, so this costs no real disk.
        let file = std::fs::File::create(&path).expect("create");
        file.set_len(MAX_CALENDAR + 1).expect("grow");
        drop(file);

        let err = read_calendar(&path).expect_err("must refuse");
        let message = err.to_string();
        assert!(message.contains("limit"), "says why: {message}");
        assert!(
            message.contains("10 MB"),
            "and what the limit is: {message}"
        );

        // And one of an ordinary size is read.
        let small = dir.join("small.ics");
        std::fs::write(&small, "BEGIN:VCALENDAR\nEND:VCALENDAR\n").expect("write");
        assert!(read_calendar(&small).is_ok());
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
        let rows: Vec<Row> = AgendaPanel::rows(&state, &now).collect();

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

        let marked = |now: &Zoned| match AgendaPanel::rows(&state, now).nth(1) {
            Some(Row::Event { in_progress, .. }) => in_progress,
            _ => unreachable!("row 1 is the event"),
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

    /// A panel pointed at nothing. The reader thread finds no file and says so,
    /// which is all these two need — they are about the status line, not the
    /// calendar.
    fn idle_panel() -> AgendaPanel {
        AgendaPanel::new(&AgendaConfig::default(), PathBuf::from("/nonexistent.ics"))
    }

    /// Pretend a read has just landed, the way the reader thread does.
    fn land_a_read(panel: &mut AgendaPanel) {
        panel
            .generation
            .store(panel.seen + 1, std::sync::atomic::Ordering::Release);
        panel.tick();
    }

    /// The bug: `reloading…` was set when the reload was *asked for* and cleared
    /// only by the next keypress, so a panel nobody touched went on claiming to
    /// be mid-operation after the reload had landed. Measured at 83 seconds in a
    /// real terminal, with the reloaded events already on screen above it.
    ///
    /// This is the whole point of a dashboard you leave open: the reader is not
    /// pressing keys, so "cleared on the next keypress" is "never".
    #[test]
    fn a_landed_reload_takes_its_own_message_down() {
        let mut panel = idle_panel();
        panel.status = Some(RELOADING.into());
        land_a_read(&mut panel);
        assert_eq!(
            panel.status, None,
            "a reload that has landed must stop saying it is reloading"
        );
    }

    /// A panel whose reader thread has finished, so nothing lands while a
    /// test winds the clock. The reader is stopped and waited out — it holds
    /// a share of the generation until it returns — and any read it landed
    /// on the way out is taken in before the test begins.
    fn still_panel() -> AgendaPanel {
        let mut panel = idle_panel();
        panel.stop.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(10);
        while Arc::strong_count(&panel.generation) > 1 {
            assert!(Instant::now() < deadline, "the reader thread did not stop");
            std::thread::sleep(Duration::from_millis(5));
        }
        panel.tick();
        panel
    }

    /// Invariant 13. The `▸` marker and the status bar's `in 3m` are worked
    /// out when the panel is drawn, and `tick` reported only a read landing,
    /// so on a dashboard with nothing else redrawing — no clock placed — a
    /// meeting started without its marker and the alert counted down behind
    /// the clock until the next read: up to an hour late at
    /// `refresh_secs = 3600`. The default layout hid it, because the clock
    /// redraws every minute and takes the agenda with it.
    #[test]
    fn an_event_starting_or_ending_asks_for_a_redraw_without_a_read() {
        let mut panel = still_panel();
        let day = date(2026, 8, 1);
        panel.shown.events = vec![event(day, 9, "standup", false)];
        panel.shown.error = None;
        let at = |hour: i8, minute: i8, second: i8| {
            day.at(hour, minute, second, 0).to_zoned(tz()).unwrap()
        };

        let steps = [
            (at(8, 40, 0), false, "where the test starts"),
            (at(8, 45, 0), false, "twenty minutes out, nothing is drawn"),
            (at(8, 50, 0), true, "ten minutes out the alert appears"),
            (at(8, 50, 30), true, "and counts down a minute"),
            (at(8, 50, 40), false, "but not between minutes"),
            (
                at(9, 0, 1),
                true,
                "it starts: the marker comes, the alert goes",
            ),
            (at(9, 30, 0), false, "under way, nothing moves"),
            (at(10, 0, 0), true, "it ends: the marker goes"),
            (at(10, 30, 0), false, "over, nothing moves"),
        ];
        for (now, expected, why) in steps {
            assert_eq!(panel.tick_at(&now), expected, "{why}, at {now}");
        }
    }

    /// The other half, and the reason `tick` matches on the message rather than
    /// clearing whatever is there: `o` puts the calendar's path up, and a
    /// background re-read must not wipe it out from under the reader.
    #[test]
    fn a_path_shown_by_o_survives_a_background_read() {
        let mut panel = idle_panel();
        panel.status = Some("/home/someone/calendar.ics".into());
        land_a_read(&mut panel);
        assert_eq!(
            panel.status.as_deref(),
            Some("/home/someone/calendar.ics"),
            "a read landing must not clear a status it did not put up"
        );
    }

    /// `o` exists to show the path, so a path cut at the panel's edge is the
    /// whole message lost. It went up as one `Line` and the terminal cut it:
    /// even at the 52 cells the panel stops growing at, the default macOS
    /// path stopped in the middle of `calendar.ics` with nothing to say so. It
    /// wraps now, and in a panel too short for all of it the last row it gets
    /// ends in `…`.
    #[test]
    fn the_path_shown_by_o_is_wrapped_rather_than_cut_off() {
        let path = PathBuf::from("/Users/someone/Library/Application Support/mirador/calendar.ics");
        let mut panel = AgendaPanel::new(&AgendaConfig::default(), path);
        panel.handle_key(KeyEvent::from(KeyCode::Char('o')));

        let rows = screen(&mut panel, 30, 12);
        let joined: String = rows.iter().map(|row| row.trim_end()).collect();
        assert!(
            joined.contains("mirador/calendar.ics"),
            "the path lost its end at 30 columns: {rows:#?}"
        );
        assert!(
            !joined.contains('…'),
            "and a path that fits says nothing was cut: {rows:#?}"
        );

        let short = screen(&mut panel, 30, 3);
        assert!(
            short[2].trim_end().ends_with('…'),
            "a path with fewer rows than it needs says so: {short:#?}"
        );
    }

    /// The marker beside the meeting under way is two cells, and it was
    /// measured in bytes — four, since `▸` takes three — so that row, the one
    /// the marker exists to point at, lost a location every other row kept.
    #[test]
    fn the_meeting_under_way_has_the_room_any_other_row_has() {
        let theme = crate::theme::Theme::default();
        let mut e = event(date(2026, 8, 1), 9, "Design review", false);
        e.location = Some("Room 12".into());
        // Two cells of marker, a six-cell time and a space leave twenty-five:
        // the summary and its location exactly.
        for in_progress in [false, true] {
            let text: String = event_line(&e, in_progress, true, 34, &theme)
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            assert!(
                text.contains("Room 12"),
                "in progress {in_progress}: `{text}`"
            );
            assert_eq!(crate::grid::display_width(&text), 34, "`{text}`");
        }
    }

    /// Midnight moves the reader's window on a day, and the day it reaches
    /// was never in the last read. Taking its events for new ones logged the
    /// next instance of every repeating meeting as having "appeared in your
    /// calendar", every night, on a dashboard left open overnight — which is
    /// the way this one is used. Only an event inside the last window can
    /// have been added since; one past it was out of sight, not absent.
    #[test]
    fn a_day_coming_into_the_window_is_not_news() {
        let midnight = |day: Date| day.at(0, 0, 0, 0).to_zoned(tz()).unwrap().timestamp();
        let read = |first: Date, events: Vec<ical::Event>| State {
            events,
            built_for: Some(first),
            built_until: Some(midnight(first + Span::new().days(2))),
            ..State::default()
        };
        let (wed, thu, fri) = (date(2026, 10, 7), date(2026, 10, 8), date(2026, 10, 9));
        let standup = |day| event(day, 9, "Standup", false);
        let mut panel = idle_panel();

        panel.shown = read(wed, vec![standup(wed), standup(thu)]);
        panel.note_new_entries();
        assert!(panel.pending.is_empty(), "a first read is never news");

        panel.shown = read(thu, vec![standup(thu), standup(fri)]);
        panel.note_new_entries();
        let logged: Vec<&str> = panel.pending.iter().map(|e| e.text.as_str()).collect();
        assert!(logged.is_empty(), "Friday came into view: {logged:?}");

        panel.shown = read(
            thu,
            vec![standup(thu), event(fri, 15, "Dentist", false), standup(fri)],
        );
        panel.note_new_entries();
        let logged: Vec<&str> = panel.pending.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(logged.len(), 1, "{logged:?}");
        assert!(
            logged[0].starts_with("Dentist appeared"),
            "an event added inside the window is still news: {logged:?}"
        );
    }

    /// Pointing the panel at another calendar with `f` is a first read of
    /// that calendar, and has to be as quiet as startup. It was compared
    /// against the old calendar's events, so every event in the new file was
    /// logged as having "appeared in your calendar". Clearing what was known
    /// when the path changes is not enough on its own: a read of the old file
    /// already under way lands after the swap, becomes the baseline, and the
    /// new calendar is then compared against it all the same.
    #[test]
    fn another_calendar_is_read_as_quietly_as_the_first() {
        let wed = date(2026, 10, 7);
        let until = wed.at(0, 0, 0, 0).to_zoned(tz()).unwrap().timestamp() + Span::new().hours(72);
        let read = |file: &str, events: Vec<ical::Event>| State {
            events,
            built_for: Some(wed),
            built_until: Some(until),
            source: Some(PathBuf::from(file)),
            ..State::default()
        };
        let logged = |panel: &AgendaPanel| -> Vec<String> {
            panel.pending.iter().map(|e| e.text.clone()).collect()
        };
        let mut panel = idle_panel();

        panel.shown = read("home.ics", vec![event(wed, 9, "Standup", false)]);
        panel.note_new_entries();
        panel.set_path(PathBuf::from("work.ics"));
        // The read that was under way when the path changed.
        panel.shown = read("home.ics", vec![event(wed, 9, "Standup", false)]);
        panel.note_new_entries();
        let work = vec![
            event(wed, 10, "Planning", false),
            event(wed, 14, "Review", false),
        ];
        panel.shown = read("work.ics", work.clone());
        panel.note_new_entries();
        assert!(logged(&panel).is_empty(), "{:?}", logged(&panel));

        let mut more = work;
        more.push(event(wed, 16, "Retro", false));
        panel.shown = read("work.ics", more);
        panel.note_new_entries();
        let logged = logged(&panel);
        assert_eq!(logged.len(), 1, "{logged:?}");
        assert!(
            logged[0].starts_with("Retro appeared"),
            "the new calendar's own additions are still news: {logged:?}"
        );
    }

    /// The horizon `note_new_entries` relies on is the reader's to publish:
    /// without it every read looks unbounded and the test above proves
    /// nothing about a running panel.
    #[test]
    fn a_read_says_where_its_window_ends() {
        let dir = TempDir::new("agenda-until");
        let ics = dir.join("cal.ics");
        std::fs::write(&ics, "BEGIN:VCALENDAR\nEND:VCALENDAR\n").unwrap();
        let config = AgendaConfig {
            days: 3,
            ..AgendaConfig::default()
        };
        let mut panel = AgendaPanel::new(&config, ics);
        for _ in 0..100 {
            if panel.tick() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let today = panel.shown.built_for.expect("a read landed");
        let until = panel.shown.built_until.expect("and said where it stops");
        let tz = TimeZone::system();
        let expected = ical::local_midnight(today + Span::new().days(3), &tz).unwrap();
        assert_eq!(until, expected.timestamp());
    }
}
