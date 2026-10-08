//! A pomodoro timer: focus intervals separated by breaks.
//!
//! The panel owns a small state machine — a phase, and either a deadline or a
//! frozen remainder — and derives everything it draws from it. Nothing is
//! persisted: a timer that survived a restart would be lying about how long you
//! have been sitting there.
//!
//! Durations are adjustable from the panel and outlive the session: `[pomodoro]`
//! seeds them and [`crate::state`] records where you moved since, so mirador
//! never reserialises the config. Only a phase you actually adjusted is
//! remembered — see [`Panel::remember`].

use std::time::{Duration, Instant, SystemTime};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::chart::meter_line;
use crate::config::PomodoroConfig;
use crate::frame::{Binding, FRAME_HEIGHT, FRAME_WIDTH};
use crate::glyphs::{self, BigText};
use crate::grid::cell_width;
use crate::keymap::{KeysConfig, Meta, PanelKeymap};
use crate::panel::{KeyOutcome, Panel, RenderContext};

/// The numerals are never drawn larger than this.
///
/// One, deliberately. `MM:SS` at scale 1 is 38 columns by 5 rows, which is
/// already a chunky readout and is as much as this panel is worth — scale 2 is
/// 68 columns and scale 3 is 98, and a timer occupying half a dashboard reads
/// as an alarm rather than as an instrument. Capping the scale is also what
/// makes `max_width` small enough to matter: without it the panel claims 102
/// columns it cannot use, and takes them from the task list next door.
const MAX_SCALE: u16 = 1;
/// Longest interval the panel will let you dial in, in minutes. Well past any
/// real pomodoro, but a bound stops `+` held down from producing a timer that
/// no longer fits its own display.
///
/// A bound on the dial, not on the config: `[pomodoro]` may ask for longer,
/// and then `+` stops at the config's own length instead, so a phase set past
/// this can be shortened from the panel and put back, but not lengthened.
pub const MAX_MINUTES: u64 = 180;

/// Rows the panel needs besides the numerals: the phase label above, and the
/// meter and pip line below.
const LABEL_AND_FOOTER: u16 = 3;

/// How a finished phase announces itself.
///
/// There is deliberately no audio crate behind this. Playing a sound means
/// talking to the platform's audio stack — `rodio` reaches `cpal` reaches
/// `alsa-sys`, a C library needing dev headers on every Linux builder — and a
/// terminal dashboard is not worth that. `\x07` costs nothing and hands the
/// decision to the terminal, which already knows whether this user wants a
/// sound, a flash, or silence.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Chime {
    /// The terminal bell.
    Bell,
    /// A program and its arguments, for someone who wants a specific sound.
    Run(Vec<String>),
}

/// What the config asks for when a phase ends, if anything.
///
/// Split from the doing of it so the decision can be tested without making a
/// noise in CI.
fn chime_for(config: &PomodoroConfig) -> Option<Chime> {
    if !config.chime {
        return None;
    }
    if config.chime_command.is_empty() {
        Some(Chime::Bell)
    } else {
        Some(Chime::Run(config.chime_command.clone()))
    }
}

/// Which part of the cycle the timer is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    /// The label above the numerals.
    fn label(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::ShortBreak => "short break",
            Self::LongBreak => "long break",
        }
    }

    /// Whether this phase is work rather than rest. Breaks share a colour and a
    /// shape, because the distinction that matters at a glance is "am I meant
    /// to be working", not which of the two breaks this is.
    fn is_focus(self) -> bool {
        self == Self::Focus
    }
}

/// How far off a deadline goes when the length asked for is further than a
/// clock can count: a century, which for a timer is never, and well inside
/// what every platform's clocks can hold. The tightest is `SystemTime` on
/// Windows, which runs out in the year 30828 — far sooner than `Instant`
/// there, so reading the wall clock brought the limit within reach.
const HORIZON: Duration = Duration::from_hours(100 * 365 * 24);

/// When a running phase ends, by two clocks, because neither is right alone.
///
/// `Instant` is monotonic, which is why a timer reaches for it — and on macOS
/// (`CLOCK_UPTIME_RAW`) and Linux (`CLOCK_MONOTONIC`) it is also uptime: it
/// stops while the machine sleeps. Timed by it alone, a phase with ten minutes
/// left when the lid closed still had ten minutes left an hour later.
/// `SystemTime` counts the sleep, and can be stepped by hand or by NTP. The
/// phase ends by whichever says less is left; see [`Deadline::remaining_at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Deadline {
    monotonic: Instant,
    wall: SystemTime,
}

impl Deadline {
    /// `left` from now, by both clocks, or [`HORIZON`] from now by a clock
    /// that cannot count that far. `+` would panic there, and `[pomodoro]`
    /// bounds a length only from below.
    fn after(left: Duration) -> Self {
        let monotonic = Instant::now();
        let wall = SystemTime::now();
        Self {
            monotonic: monotonic
                .checked_add(left)
                .or_else(|| monotonic.checked_add(HORIZON))
                .unwrap_or(monotonic),
            wall: wall
                .checked_add(left)
                .or_else(|| wall.checked_add(HORIZON))
                .unwrap_or(wall),
        }
    }

    /// What is left now.
    fn remaining(self) -> Duration {
        self.remaining_at(Instant::now(), SystemTime::now())
    }

    /// What is left when the clocks read `monotonic` and `wall`: the smaller
    /// of the two remainders.
    ///
    /// After a sleep the wall remainder is the smaller, so the phase has
    /// elapsed through it. A wall clock set back gives the larger, which is
    /// ignored, so it can neither end a phase early nor lengthen it. One set
    /// forward reads as time gone by: from here it cannot be told from a
    /// sleep, and that is the price of counting sleep at all.
    fn remaining_at(self, monotonic: Instant, wall: SystemTime) -> Duration {
        let by_monotonic = self.monotonic.saturating_duration_since(monotonic);
        let by_wall = self.wall.duration_since(wall).unwrap_or(Duration::ZERO);
        by_monotonic.min(by_wall)
    }
}

/// A pomodoro timer panel.
pub struct PomodoroPanel {
    /// `[pomodoro.keys]` over the defaults.
    keys: PanelKeymap<PomodoroAction>,
    config: PomodoroConfig,
    phase: Phase,
    /// When the current phase ends. `None` whenever the timer is not running.
    ends_at: Option<Deadline>,
    /// What is left of the phase while stopped. Meaningless while running —
    /// [`PomodoroPanel::remaining`] reads the deadline instead, by the wall
    /// clock as well as the monotonic one, so a laptop that slept through a
    /// phase wakes up knowing the time has gone. A stopped phase is frozen
    /// here and sleep does not touch it, which is what pausing means.
    paused_remaining: Duration,
    /// How far `+` may take each phase — focus, short break, long break:
    /// [`MAX_MINUTES`], or the config's own length where that is longer. The
    /// config's, not the length this panel was built with, which may be a
    /// remembered shortening; see [`PomodoroConfig::as_configured`].
    ceiling: [u64; 3],
    /// Focus intervals finished since the last long break, which is what the
    /// pips count and what decides when the long break falls due.
    completed: u32,
    /// Focus intervals finished in total, kept only so the counter can say
    /// something after a long break resets the pips.
    total_completed: u32,
    /// Why the last chime did not happen, if it did not. Shown in the panel:
    /// a notification that silently stopped working is worse than none, because
    /// you go on trusting it.
    chime_error: Option<String>,
}

impl PomodoroPanel {
    pub fn new(config: PomodoroConfig) -> Self {
        let first = Duration::from_secs(config.focus_minutes * 60);
        let configured = config.as_configured.unwrap_or([
            config.focus_minutes,
            config.short_break_minutes,
            config.long_break_minutes,
        ]);
        Self {
            keys: default_keys(),
            config,
            phase: Phase::Focus,
            ends_at: None,
            paused_remaining: first,
            ceiling: configured.map(|minutes| MAX_MINUTES.max(minutes)),
            completed: 0,
            total_completed: 0,
            chime_error: None,
        }
    }

    /// Announce the end of a phase, if the config asked for it.
    ///
    /// Never blocks and never fails loudly: the bell is a write to stdout, and
    /// a command is spawned and reaped on its own thread so a slow or wedged
    /// player cannot stall the dashboard. A player that will not start is
    /// recorded and shown rather than swallowed.
    fn sound_chime(&mut self) {
        let Some(chime) = chime_for(&self.config) else {
            return;
        };

        match chime {
            Chime::Bell => {
                use std::io::Write;
                let mut out = std::io::stdout();
                // BEL. It moves no cursor and occupies no cell, so it cannot
                // disturb what is already drawn.
                self.chime_error = match out.write_all(b"\x07").and_then(|()| out.flush()) {
                    Ok(()) => None,
                    Err(e) => Some(format!("bell failed: {e}")),
                };
            }
            Chime::Run(command) => {
                let (program, rest) = command.split_first().expect("non-empty by construction");
                match std::process::Command::new(program)
                    .args(rest)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(mut child) => {
                        self.chime_error = None;
                        // Reaped on a thread it can take as long as it likes on.
                        // Without this the zombies pile up one per phase.
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                    }
                    Err(e) => {
                        self.chime_error = Some(format!("{program}: {e}"));
                    }
                }
            }
        }
    }

    /// Configured length of a phase.
    fn length_of(&self, phase: Phase) -> Duration {
        let minutes = match phase {
            Phase::Focus => self.config.focus_minutes,
            Phase::ShortBreak => self.config.short_break_minutes,
            Phase::LongBreak => self.config.long_break_minutes,
        };
        Duration::from_secs(minutes * 60)
    }

    /// Time left in the current phase.
    fn remaining(&self) -> Duration {
        match self.ends_at {
            Some(deadline) => deadline.remaining(),
            None => self.paused_remaining,
        }
    }

    fn is_running(&self) -> bool {
        self.ends_at.is_some()
    }

    fn start(&mut self) {
        if self.ends_at.is_none() {
            // A phase that has run down to nothing restarts rather than
            // starting already finished.
            if self.paused_remaining.is_zero() {
                self.paused_remaining = self.length_of(self.phase);
            }
            self.ends_at = Some(Deadline::after(self.paused_remaining));
        }
    }

    fn pause(&mut self) {
        if self.ends_at.is_some() {
            self.paused_remaining = self.remaining();
            self.ends_at = None;
        }
    }

    fn toggle(&mut self) {
        if self.is_running() {
            self.pause();
        } else {
            self.start();
        }
    }

    /// Put the current phase back to its full length, without changing which
    /// phase it is or how many rounds are done.
    fn reset_phase(&mut self) {
        self.paused_remaining = self.length_of(self.phase);
        self.ends_at = None;
    }

    /// Which phase follows this one.
    fn next_phase(&self) -> Phase {
        if self.phase.is_focus() {
            // `completed` has already been incremented for the phase that just
            // ended, so a set of four lands the long break on the fourth.
            if self.completed > 0
                && self
                    .completed
                    .is_multiple_of(self.config.rounds_before_long_break)
            {
                Phase::LongBreak
            } else {
                Phase::ShortBreak
            }
        } else {
            Phase::Focus
        }
    }

    /// Finish the current phase and move to the next one.
    ///
    /// The next phase always gets its full length and, when `auto_start` runs
    /// it, a fresh deadline — never one chained from the deadline that just
    /// passed. A machine opened after an hour asleep is, by the wall clock,
    /// several phases on; what it does is end the phase that ran out — one
    /// chime, one round counted — and leave the next waiting for a key, or
    /// with `auto_start` run it from the moment you came back, rather than
    /// racing through six phases to catch up. At most one phase ends per tick,
    /// and an hour away is not counted as focus nobody sat through.
    fn advance(&mut self) {
        if self.phase.is_focus() {
            self.completed += 1;
            self.total_completed += 1;
        } else if self.phase == Phase::LongBreak {
            // The long break closes the set; the pips start over.
            self.completed = 0;
        }

        self.phase = self.next_phase();
        self.paused_remaining = self.length_of(self.phase);
        self.ends_at = if self.config.auto_start {
            Some(Deadline::after(self.paused_remaining))
        } else {
            None
        };
    }

    /// Lengthen or shorten the current phase by `delta` minutes.
    ///
    /// Both the configured length and the time left move together, so `+` while
    /// eighteen minutes into a twenty-five minute focus adds a minute to what is
    /// left rather than silently rewinding you to the start.
    fn adjust(&mut self, delta: i64) {
        let (minutes, ceiling) = match self.phase {
            Phase::Focus => (&mut self.config.focus_minutes, self.ceiling[0]),
            Phase::ShortBreak => (&mut self.config.short_break_minutes, self.ceiling[1]),
            Phase::LongBreak => (&mut self.config.long_break_minutes, self.ceiling[2]),
        };

        // The ceiling stretches to a longer phase from the config, so `-`
        // takes a minute off such a phase rather than cutting it to the cap,
        // and `+` can put it back to what the config says — which is what
        // retracts the remembered shortening — but no further.
        let before = *minutes;
        let after = before.saturating_add_signed(delta).clamp(1, ceiling);
        *minutes = after;

        if after == before {
            return;
        }

        let shift = Duration::from_secs(after.abs_diff(before) * 60);
        let grew = after > before;
        let remaining = self.remaining();
        let adjusted = if grew {
            remaining + shift
        } else {
            remaining.saturating_sub(shift)
        };

        match self.ends_at {
            Some(_) => self.ends_at = Some(Deadline::after(adjusted)),
            None => self.paused_remaining = adjusted,
        }
    }

    /// Fraction of the phase already spent, 0 to 100.
    fn elapsed_percent(&self) -> u16 {
        let total = self.length_of(self.phase).as_secs();
        if total == 0 {
            return 0;
        }
        let left = self.remaining().as_secs().min(total);
        let done = total - left;
        u16::try_from(done * 100 / total).unwrap_or(100)
    }
}

/// `MM:SS`, counting minutes past an hour rather than rolling over — a
/// ninety-minute phase reads `90:00`, not `30:00`.
fn clock_text(remaining: Duration) -> String {
    let secs = remaining.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// What the pomodoro's keys do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PomodoroAction {
    Toggle,
    Next,
    Longer,
    Shorter,
    Reset,
}

/// Every key the panel responds to, under `[pomodoro.keys]`. The border hint, the
/// status bar and the help overlay are derived from it, so a key the panel
/// reads is a key it advertises.
pub const ACTIONS: &[Meta<PomodoroAction>] = &[
    Meta {
        action: PomodoroAction::Toggle,
        name: "toggle",
        defaults: &[(KeyCode::Char(' '), KeyModifiers::NONE)],
        label: "start/pause",
        primary: true,
        joins: false,
        about: "start or pause the timer",
    },
    Meta {
        action: PomodoroAction::Next,
        name: "next",
        defaults: &[(KeyCode::Char('n'), KeyModifiers::NONE)],
        label: "next phase",
        primary: true,
        joins: false,
        about: "skip to the next phase",
    },
    Meta {
        action: PomodoroAction::Longer,
        name: "longer",
        defaults: &[
            (KeyCode::Char('+'), KeyModifiers::NONE),
            (KeyCode::Char('='), KeyModifiers::NONE),
        ],
        label: "length",
        primary: true,
        joins: false,
        about: "lengthen this phase by a minute",
    },
    Meta {
        action: PomodoroAction::Shorter,
        name: "shorter",
        defaults: &[
            (KeyCode::Char('-'), KeyModifiers::NONE),
            (KeyCode::Char('_'), KeyModifiers::NONE),
        ],
        label: "length",
        primary: true,
        joins: true,
        about: "shorten this phase by a minute",
    },
    Meta {
        action: PomodoroAction::Reset,
        name: "reset",
        defaults: &[(KeyCode::Char('r'), KeyModifiers::NONE)],
        label: "reset phase",
        primary: false,
        joins: false,
        about: "start this phase over",
    },
];

/// `[pomodoro.keys]` laid over [`ACTIONS`], or why it cannot be.
pub fn keymap(keys: &KeysConfig) -> Result<PanelKeymap<PomodoroAction>, String> {
    PanelKeymap::new("pomodoro", ACTIONS, keys)
}

/// The keys the panel starts with, until `build` hands it the config's
/// through [`Panel::set_keys`].
fn default_keys() -> PanelKeymap<PomodoroAction> {
    PanelKeymap::defaults("pomodoro", ACTIONS)
}

impl Panel for PomodoroPanel {
    fn title(&self) -> String {
        "Pomodoro".into()
    }

    fn counter(&self) -> Option<String> {
        Some(if self.is_running() {
            self.phase.label().into()
        } else if self.remaining() == self.length_of(self.phase) {
            "ready".into()
        } else {
            "paused".into()
        })
    }

    fn bindings(&self) -> &[Binding] {
        self.keys.bindings()
    }

    fn set_keys(&mut self, config: &crate::config::Config) {
        self.keys = PanelKeymap::or_defaults("pomodoro", ACTIONS, &config.pomodoro.keys);
    }

    fn max_width(&self) -> Option<u16> {
        // 42 columns: the numerals plus the frame. Everything else the panel
        // draws is narrower, so past this the extra only pads the sides — and
        // a timer is a small fact. The surplus goes to the task list, which
        // can always use another column.
        Some(glyphs::width_of("00:00", MAX_SCALE) + FRAME_WIDTH)
    }

    fn max_height(&self) -> Option<u16> {
        // Label, numerals, meter, pip line, and the frame. Nothing here scrolls
        // or scales, so a taller panel is pure empty space; declaring the
        // ceiling lets a row hand it downward instead.
        Some(1 + 5 * MAX_SCALE + 2 + FRAME_HEIGHT)
    }

    fn refresh_interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn tick(&mut self) -> bool {
        if !self.is_running() {
            // A paused or unstarted timer shows a fixed number. Nothing moves
            // until a key does, and a key already forces a redraw.
            return false;
        }
        if self.remaining().is_zero() {
            // Only a phase that ran out announces itself. Pressing `n` to skip
            // is you already knowing, and a bell for something you just did is
            // noise.
            self.sound_chime();
            self.advance();
        }
        // A running timer's displayed minute:second changes every tick, which
        // is exactly why `refresh_interval` is one second.
        true
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let theme = ctx.theme;

        // Focus is the brass of every other instrument; a break is the moss
        // used for "fine" elsewhere. Colour is the only thing separating them,
        // so the label spells it out too.
        let phase_colour = if self.phase.is_focus() {
            theme.accent
        } else {
            theme.success
        };

        let time = clock_text(self.remaining());
        let bottom = area.y + area.height;

        // Centre the whole block vertically rather than letting it sit at the
        // top of whatever height the row hands over. The timer is the panel's
        // subject; leaving it pinned to the ceiling with a void underneath
        // reads as a rendering accident rather than as an instrument face.
        let scale = clock_scale(
            &time,
            area.width,
            area.height.saturating_sub(LABEL_AND_FOOTER),
        );
        let content = LABEL_AND_FOOTER + clock_rows(&time, scale);
        let mut cursor = area.y + area.height.saturating_sub(content) / 2;

        // The label, in the utility face so it reads as an instrument legend
        // rather than as another value. Cut with `…` where the panel is
        // narrower: left to the terminal, `SHORT BREAK` came out as a
        // whole-looking `SHORT BRE`.
        if cursor < bottom {
            let label = crate::grid::truncate(
                &glyphs::utility(self.phase.label()),
                usize::from(area.width),
            );
            let x = area.x + area.width.saturating_sub(cell_width(&label)) / 2;
            frame.render_widget(
                Paragraph::new(Span::styled(
                    label.clone(),
                    Style::default()
                        .fg(phase_colour)
                        .add_modifier(Modifier::BOLD),
                )),
                Rect::new(x, cursor, cell_width(&label).min(area.width), 1),
            );
            cursor += 1;
        }

        // A paused timer recedes rather than flashing. The dashboard is meant
        // to be leavable; a blinking clock is the opposite of that.
        let clock_colour = if self.is_running() {
            phase_colour
        } else {
            theme.muted
        };
        cursor += draw_clock(frame, area, cursor, bottom, &time, scale, clock_colour);

        // The meter. Always a full-width track so the panel does not change
        // shape as the phase runs down.
        if cursor < bottom && area.width > 4 {
            let cells = meter_line(
                self.elapsed_percent(),
                area.width,
                phase_colour,
                theme.track,
            );
            frame.render_widget(
                Paragraph::new(Line::from(cells)),
                Rect::new(area.x, cursor, area.width, 1),
            );
            cursor += 1;
        }

        // Pips for the set, and what the two dials are currently set to.
        if cursor < bottom {
            let mut pips = Vec::new();
            for index in 0..self.config.rounds_before_long_break {
                let done = index < self.completed;
                pips.push(Span::styled(
                    "■",
                    Style::default().fg(if done { theme.accent } else { theme.track }),
                ));
                pips.push(Span::raw(" "));
            }
            let pips_width =
                usize::try_from(self.config.rounds_before_long_break).unwrap_or(usize::MAX) * 2;
            let mut parts = vec![pips];

            // A broken chime displaces the dial readout rather than sitting
            // beside it: the durations are visible on the timer anyway, and a
            // notification you think is working but is not is the failure worth
            // the space.
            if let Some(error) = &self.chime_error {
                // Truncated to what is left rather than dropped whole, unlike
                // everything else on this line. The dials are readable off the
                // timer, so losing them costs nothing — but this is the only
                // place a broken chime is reported anywhere in the program, and
                // a panel narrow enough to drop it is exactly the one where
                // silence would be permanent.
                parts.push(vec![Span::styled(
                    crate::grid::truncate(
                        &format!(" chime: {error}"),
                        usize::from(area.width).saturating_sub(pips_width),
                    ),
                    Style::default().fg(theme.error),
                )]);
            } else {
                let muted = Style::default().fg(theme.muted);
                parts.push(vec![Span::styled(
                    format!(" {}m focus", self.config.focus_minutes),
                    muted,
                )]);
                parts.push(vec![Span::styled(
                    format!(" · {}m break", self.config.short_break_minutes),
                    muted,
                )]);
                // The pips reset with every long break, so they cannot answer
                // "how much have I actually done today". This can.
                if self.total_completed > 0 {
                    parts.push(vec![Span::styled(
                        format!(" · {} done", self.total_completed),
                        muted,
                    )]);
                }
            }

            frame.render_widget(
                Paragraph::new(crate::grid::assemble(parts, area.width)),
                Rect::new(area.x, cursor, area.width, 1),
            );
        }
    }

    fn remember(&self, state: &mut crate::state::UiState) {
        state.pomodoro_focus_minutes = Some(self.config.focus_minutes);
        state.pomodoro_short_break_minutes = Some(self.config.short_break_minutes);
        state.pomodoro_long_break_minutes = Some(self.config.long_break_minutes);
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        // The map compares modifiers exactly, so Ctrl+N is not `n` — those
        // keys stay the shell's — while `+`, which arrives shifted on most
        // layouts, matches, because a key's Shift is folded into its
        // character.
        let Some(action) = self.keys.action(key) else {
            return KeyOutcome::Ignored;
        };
        match action {
            PomodoroAction::Toggle => self.toggle(),
            PomodoroAction::Next => self.advance(),
            PomodoroAction::Reset => self.reset_phase(),
            PomodoroAction::Longer => self.adjust(1),
            PomodoroAction::Shorter => self.adjust(-1),
        }
        KeyOutcome::Consumed
    }
}

/// The largest scale the numerals fit at, or `None` for the plain-text
/// fallback.
///
/// `budget` is the height left after the label and the two rows under the
/// numerals, so those are never what gets pushed off the bottom — the numerals
/// give way first, being the part with room to shrink.
fn clock_scale(time: &str, width: u16, budget: u16) -> Option<u16> {
    // Both dimensions at once. Filtering a width-only answer by height rejects
    // where it should step down, and this panel's text changes length as the
    // timer runs — `25:00` down to `9:59` earns a bigger scale, which could then
    // be too tall and drop the whole thing to plain text mid-session. Same
    // defect as the clock's, found with it.
    glyphs::fitting_scale(time, width, budget.max(1), MAX_SCALE)
}

/// Rows the time will occupy at `scale`. One, when it falls back to text.
fn clock_rows(time: &str, scale: Option<u16>) -> u16 {
    scale.map_or(1, |s| BigText::new(time, s).height)
}

/// Draw the remaining time and report the rows used.
///
/// Below the width even scale 1 needs, the time falls back to plain text
/// rather than vanishing: it is the one thing this panel cannot do without.
fn draw_clock(
    frame: &mut Frame,
    area: Rect,
    top: u16,
    bottom: u16,
    time: &str,
    scale: Option<u16>,
    colour: Color,
) -> u16 {
    let Some(scale) = scale else {
        if top >= bottom {
            return 0;
        }
        // `05:0` would be a time nobody set; `05:…` says it was cut.
        let time = crate::grid::truncate(time, usize::from(area.width));
        let time = time.as_str();
        let width = cell_width(time);
        let x = area.x + area.width.saturating_sub(width) / 2;
        frame.render_widget(
            Paragraph::new(Span::styled(time.to_owned(), Style::default().fg(colour))),
            Rect::new(x, top, width.min(area.width), 1),
        );
        return 1;
    };

    let big = BigText::new(time, scale);
    let x = area.x + area.width.saturating_sub(big.width) / 2;
    for (index, row) in big.rows.iter().enumerate() {
        let y = top + u16::try_from(index).unwrap_or(0);
        if y >= bottom {
            break;
        }
        frame.render_widget(
            Paragraph::new(Span::styled(row.clone(), Style::default().fg(colour))),
            Rect::new(x, y, big.width.min(area.width), 1),
        );
    }
    big.height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> PomodoroPanel {
        PomodoroPanel::new(PomodoroConfig::default())
    }

    #[test]
    fn a_fresh_timer_is_stopped_at_a_full_focus_interval() {
        let p = panel();
        assert_eq!(p.phase, Phase::Focus);
        assert!(!p.is_running());
        assert_eq!(p.remaining(), Duration::from_mins(25));
        assert_eq!(p.elapsed_percent(), 0);
    }

    #[test]
    fn space_starts_and_pauses_without_losing_the_remainder() {
        let mut p = panel();
        p.paused_remaining = Duration::from_secs(90);

        p.toggle();
        assert!(p.is_running());

        p.toggle();
        assert!(!p.is_running());
        // Within a second of where it was: the point is that pausing keeps the
        // remainder rather than resetting the phase.
        let left = p.remaining().as_secs();
        assert!((89..=90).contains(&left), "remaining was {left}s");
    }

    #[test]
    fn a_phase_that_runs_out_advances_exactly_one_step() {
        let mut p = panel();
        p.paused_remaining = Duration::ZERO;
        p.ends_at = Some(Deadline::after(Duration::ZERO));

        p.tick();
        assert_eq!(p.phase, Phase::ShortBreak);
        assert_eq!(p.completed, 1);
        // auto_start is off, so it waits for you rather than starting a break
        // you did not ask for.
        assert!(!p.is_running());
        assert_eq!(p.remaining(), Duration::from_mins(5));

        // A second tick must not cascade: the new phase has its own full length.
        p.tick();
        assert_eq!(p.phase, Phase::ShortBreak);
    }

    /// Ten minutes left when the lid closed, an hour asleep. `Instant` stops
    /// with the machine on macOS and Linux, so by it alone the phase still had
    /// ten minutes to go on waking; the wall clock says it ended fifty minutes
    /// ago, and that is the one that is right.
    #[test]
    fn a_running_phase_counts_through_sleep() {
        let mut p = panel();
        p.start();
        p.ends_at = Some(Deadline {
            monotonic: Instant::now() + Duration::from_mins(10),
            wall: SystemTime::now() - Duration::from_mins(50),
        });
        assert_eq!(p.remaining(), Duration::ZERO, "an hour asleep spent it");

        p.tick();
        assert_eq!(p.phase, Phase::ShortBreak, "so the phase ends on waking");
        assert_eq!(p.completed, 1);
    }

    /// Each clock is wrong in one direction, and the phase ends by whichever
    /// says less is left. The monotonic one misses sleep; the wall one can be
    /// set back, which must neither end a phase nor lengthen it.
    #[test]
    fn the_phase_ends_by_whichever_clock_says_less_is_left() {
        let mono = Instant::now();
        let wall = SystemTime::now();
        let deadline = Deadline {
            monotonic: mono + Duration::from_mins(10),
            wall: wall + Duration::from_mins(10),
        };
        let at = |m: u64, w: u64| {
            deadline.remaining_at(mono + Duration::from_mins(m), wall + Duration::from_mins(w))
        };
        assert_eq!(at(4, 4), Duration::from_mins(6), "awake, they agree");
        assert_eq!(at(0, 60), Duration::ZERO, "an hour asleep");
        assert_eq!(at(1, 4), Duration::from_mins(6), "three minutes asleep");
        assert_eq!(
            deadline.remaining_at(
                mono + Duration::from_mins(4),
                wall - Duration::from_mins(56)
            ),
            Duration::from_mins(6),
            "a wall clock set back an hour neither ends the phase nor lengthens it"
        );
    }

    /// An hour asleep with `auto_start` on is, by the wall clock, several
    /// phases on. The timer does not catch up: the phase that ran out ends
    /// once, and the next starts from its full length at the moment you came
    /// back — an hour away is not an hour of focus nobody sat through.
    #[test]
    fn waking_ends_one_phase_and_starts_the_next_afresh() {
        let mut p = PomodoroPanel::new(PomodoroConfig {
            auto_start: true,
            ..PomodoroConfig::default()
        });
        p.start();
        p.ends_at = Some(Deadline {
            monotonic: Instant::now() + Duration::from_mins(10),
            wall: SystemTime::now() - Duration::from_mins(50),
        });

        p.tick();
        assert_eq!(p.phase, Phase::ShortBreak);
        assert!(p.is_running(), "auto_start carries on into the break");
        assert!(
            p.remaining() > Duration::from_secs(5 * 60 - 2),
            "the break starts whole, not from the deadline that passed: {:?}",
            p.remaining()
        );

        for _ in 0..3 {
            p.tick();
        }
        assert_eq!(p.phase, Phase::ShortBreak, "and nothing cascades after it");
        assert_eq!(p.total_completed, 1, "one round, the one that ran out");
    }

    #[test]
    fn the_long_break_falls_on_the_fourth_focus_and_resets_the_set() {
        let mut p = panel();
        for round in 1..=3 {
            p.advance(); // focus -> short break
            assert_eq!(p.phase, Phase::ShortBreak, "round {round}");
            p.advance(); // break -> focus
            assert_eq!(p.phase, Phase::Focus, "round {round}");
        }

        p.advance(); // the fourth focus ends
        assert_eq!(p.phase, Phase::LongBreak);
        assert_eq!(p.completed, 4);

        p.advance(); // the long break ends
        assert_eq!(p.phase, Phase::Focus);
        assert_eq!(p.completed, 0, "a long break starts the set over");
        assert_eq!(p.total_completed, 4, "the running total does not reset");
    }

    #[test]
    fn adjusting_moves_the_length_and_the_remainder_together() {
        let mut p = panel();
        p.paused_remaining = Duration::from_mins(10);

        p.adjust(1);
        assert_eq!(p.config.focus_minutes, 26);
        assert_eq!(
            p.remaining(),
            Duration::from_mins(11),
            "adding a minute must not rewind to the start of the phase"
        );

        p.adjust(-1);
        assert_eq!(p.config.focus_minutes, 25);
        assert_eq!(p.remaining(), Duration::from_mins(10));
    }

    #[test]
    fn a_length_cannot_be_driven_below_a_minute_or_past_the_cap() {
        let mut p = panel();
        for _ in 0..40 {
            p.adjust(-1);
        }
        assert_eq!(p.config.focus_minutes, 1);
        assert!(p.remaining() <= Duration::from_mins(1));

        for _ in 0..MAX_MINUTES + 10 {
            p.adjust(1);
        }
        assert_eq!(p.config.focus_minutes, MAX_MINUTES);
    }

    /// `+` stops at `MAX_MINUTES`, but a config may ask for longer, and the
    /// first press of either key used to cut such a phase to the cap — taking
    /// an hour off a 240-minute focus, and off the time left with it, and then
    /// remembering the 180 over the config's 240.
    #[test]
    fn a_phase_configured_past_the_cap_keeps_its_length() {
        let mut p = PomodoroPanel::new(PomodoroConfig {
            focus_minutes: 240,
            ..PomodoroConfig::default()
        });
        p.paused_remaining = Duration::from_mins(50);

        p.adjust(1);
        assert_eq!(p.config.focus_minutes, 240, "`+` past the cap does nothing");
        assert_eq!(
            p.remaining(),
            Duration::from_mins(50),
            "and takes nothing off what is left"
        );
        let mut state = crate::state::UiState::default();
        p.remember(&mut state);
        assert_eq!(
            state.pomodoro_focus_minutes,
            Some(240),
            "so what is remembered still matches the config"
        );

        p.adjust(-1);
        assert_eq!(p.config.focus_minutes, 239, "`-` shortens it a minute");
        assert_eq!(p.remaining(), Duration::from_mins(49));
    }

    /// One launch against a config that sets a 240-minute focus, the way
    /// `main` runs it: the baseline taken from the config as written, `saved`
    /// laid over it, the panel built from the result, `deltas` pressed, and
    /// what would reach the state file.
    fn launch_at_240(
        saved: &crate::state::UiState,
        deltas: &[i64],
    ) -> (u64, crate::state::UiState) {
        let mut config = crate::config::Config::default();
        config.pomodoro.focus_minutes = 240;
        let baseline = crate::state::UiState::from_config(&config);
        config.apply_state(saved);
        let mut p = PomodoroPanel::new(config.pomodoro.clone());
        for &delta in deltas {
            p.adjust(delta);
        }
        let mut state = crate::state::UiState::default();
        p.remember(&mut state);
        (p.config.focus_minutes, state.only_changes_from(&baseline))
    }

    /// A 240-minute focus shortened to 239 is remembered as 239, and that is
    /// what the next launch starts at. It started at 180: the state file was
    /// clamped to the dial's cap, not to the config's own length, so the
    /// shortening the panel had just allowed was undone by a restart.
    #[test]
    fn a_phase_configured_past_the_cap_is_remembered_shortened() {
        let (_, saved) = launch_at_240(&crate::state::UiState::default(), &[-1]);
        assert_eq!(saved.pomodoro_focus_minutes, Some(239));

        let (minutes, again) = launch_at_240(&saved, &[]);
        assert_eq!(minutes, 239, "the restart keeps what was set");
        assert_eq!(again.pomodoro_focus_minutes, Some(239));
    }

    /// And `+` takes it back to the config's 240, which retracts the entry —
    /// in the session that shortened it and after a restart, when the panel is
    /// built from the remembered 239 and has only the config to say where
    /// back is. A preference that cannot be set back to the config cannot be
    /// unset at all (invariant 17).
    #[test]
    fn a_phase_configured_past_the_cap_can_be_put_back() {
        let (minutes, saved) = launch_at_240(&crate::state::UiState::default(), &[-1, 1]);
        assert_eq!(minutes, 240, "`+` undoes a `-` in the same session");
        assert_eq!(saved.pomodoro_focus_minutes, None, "{saved:?}");

        let remembered = crate::state::UiState {
            pomodoro_focus_minutes: Some(239),
            ..crate::state::UiState::default()
        };
        let (minutes, saved) = launch_at_240(&remembered, &[1]);
        assert_eq!(minutes, 240, "and after a restart");
        assert_eq!(
            saved.pomodoro_focus_minutes, None,
            "back at the config, so nothing is remembered: {saved:?}"
        );
        let (minutes, _) = launch_at_240(&remembered, &[1, 1]);
        assert_eq!(minutes, 240, "but no further than the config");
    }

    /// `Config::validate` refuses a phase longer than a year, but the panel is
    /// built from whatever `PomodoroConfig` it is handed and checks nothing
    /// itself, so it keeps its own guard behind that bound: a length the
    /// config refuses still starts, up to the most minutes a `u64` of seconds
    /// can hold. Past what a clock can count, `+` on an
    /// `Instant` or a `SystemTime` panics. `SystemTime` on Windows runs out
    /// in the year 30828, far sooner than `Instant` there, so reading the wall
    /// clock as well brought the panic within reach of a figure the config
    /// used to accept. Such a phase runs to the horizon instead.
    #[test]
    fn a_phase_too_long_for_the_clocks_starts_rather_than_panicking() {
        let mut p = PomodoroPanel::new(PomodoroConfig {
            focus_minutes: u64::MAX / 60,
            ..PomodoroConfig::default()
        });
        p.start();
        assert!(p.is_running());
        assert!(
            p.remaining() + Duration::from_mins(1) > HORIZON,
            "{:?}",
            p.remaining()
        );

        p.tick();
        assert_eq!(p.phase, Phase::Focus, "and it is not over as it starts");
    }

    #[test]
    fn adjusting_targets_the_phase_you_are_in() {
        let mut p = panel();
        p.advance(); // now on a short break
        p.adjust(1);
        assert_eq!(p.config.short_break_minutes, 6);
        assert_eq!(
            p.config.focus_minutes, 25,
            "adjusting a break must not touch the focus length"
        );
    }

    #[test]
    fn reset_restores_the_phase_without_changing_the_set() {
        let mut p = panel();
        p.completed = 2;
        p.paused_remaining = Duration::from_secs(30);
        p.ends_at = Some(Deadline::after(Duration::from_secs(30)));

        p.reset_phase();
        assert!(!p.is_running());
        assert_eq!(p.remaining(), Duration::from_mins(25));
        assert_eq!(p.completed, 2, "reset is for the phase, not the set");
    }

    #[test]
    fn starting_a_run_down_timer_restarts_it_rather_than_finishing_instantly() {
        let mut p = panel();
        p.paused_remaining = Duration::ZERO;
        p.start();
        assert!(p.is_running());
        assert!(p.remaining() > Duration::from_mins(24));
    }

    #[test]
    fn auto_start_runs_the_next_phase_without_a_keypress() {
        let mut p = panel();
        p.config.auto_start = true;
        p.advance();
        assert!(p.is_running());
        assert_eq!(p.phase, Phase::ShortBreak);
    }

    #[test]
    fn the_clock_reads_as_minutes_and_seconds_past_an_hour() {
        assert_eq!(clock_text(Duration::from_secs(0)), "00:00");
        assert_eq!(clock_text(Duration::from_secs(59)), "00:59");
        assert_eq!(clock_text(Duration::from_mins(25)), "25:00");
        assert_eq!(
            clock_text(Duration::from_mins(90)),
            "90:00",
            "a long phase must not roll over into an hours field it does not have"
        );
    }

    #[test]
    fn the_meter_paints_a_full_width_track_at_every_value() {
        for percent in [0, 1, 50, 99, 100, 250] {
            let cells = meter_line(percent, 20, Color::Red, Color::Black);
            assert_eq!(cells.len(), 20, "meter changed width at {percent}%");
        }
    }

    #[test]
    fn elapsed_percent_spans_the_phase() {
        let mut p = panel();
        assert_eq!(p.elapsed_percent(), 0);

        p.paused_remaining = Duration::from_secs(0);
        assert_eq!(p.elapsed_percent(), 100);

        p.paused_remaining = Duration::from_secs(25 * 60 / 2);
        assert_eq!(p.elapsed_percent(), 50);
    }

    #[test]
    fn the_counter_distinguishes_ready_from_paused() {
        let mut p = panel();
        assert_eq!(p.counter().as_deref(), Some("ready"));

        p.paused_remaining = Duration::from_mins(1);
        assert_eq!(
            p.counter().as_deref(),
            Some("paused"),
            "a part-spent phase that is not running is paused, not ready"
        );

        p.start();
        assert_eq!(p.counter().as_deref(), Some("focus"));
    }

    #[test]
    fn the_chime_is_off_unless_asked_for() {
        let config = PomodoroConfig::default();
        assert!(!config.chime, "a dashboard must not make noise by default");
        assert_eq!(chime_for(&config), None);
    }

    #[test]
    fn chime_falls_back_to_the_terminal_bell_with_no_command() {
        let config = PomodoroConfig {
            chime: true,
            ..PomodoroConfig::default()
        };
        assert_eq!(chime_for(&config), Some(Chime::Bell));
    }

    #[test]
    fn a_chime_command_replaces_the_bell() {
        let config = PomodoroConfig {
            chime: true,
            chime_command: vec!["afplay".into(), "/System/Library/Sounds/Glass.aiff".into()],
            ..PomodoroConfig::default()
        };
        assert_eq!(
            chime_for(&config),
            Some(Chime::Run(vec![
                "afplay".into(),
                "/System/Library/Sounds/Glass.aiff".into()
            ]))
        );
    }

    #[test]
    fn a_chime_command_that_does_not_exist_is_reported_rather_than_swallowed() {
        let mut p = PomodoroPanel::new(PomodoroConfig {
            chime: true,
            chime_command: vec!["mirador-no-such-player".into()],
            ..PomodoroConfig::default()
        });
        p.sound_chime();
        let error = p.chime_error.expect("a missing player must be reported");
        assert!(
            error.contains("mirador-no-such-player"),
            "the message must name the program that failed: {error}"
        );
    }

    #[test]
    fn advancing_with_the_chime_off_touches_nothing() {
        let mut p = panel();
        p.advance();
        assert_eq!(p.chime_error, None);
    }

    #[test]
    fn a_phase_that_runs_out_chimes_but_skipping_by_hand_does_not() {
        // A player that cannot start is the observable proof a chime was
        // attempted, without making CI audible.
        let config = PomodoroConfig {
            chime: true,
            chime_command: vec!["mirador-no-such-player".into()],
            ..PomodoroConfig::default()
        };

        let mut skipped = PomodoroPanel::new(config.clone());
        skipped.advance();
        assert_eq!(
            skipped.chime_error, None,
            "pressing `n` is you already knowing; it must not ring"
        );

        let mut expired = PomodoroPanel::new(config);
        expired.ends_at = Some(Deadline::after(Duration::ZERO));
        expired.tick();
        assert!(
            expired.chime_error.is_some(),
            "a phase that ran out must announce itself"
        );
        assert_eq!(expired.phase, Phase::ShortBreak, "and still advance");
    }

    /// Every key in the map works and is advertised; see
    /// [`crate::keymap::assert_every_key_works`].
    #[test]
    fn every_key_in_the_map_works_and_is_advertised() {
        crate::keymap::assert_every_key_works(&default_keys(), |event| panel().handle_key(event));
    }

    /// Ctrl and Alt with a pomodoro key are the shell's, not the timer's.
    #[test]
    fn a_modified_key_is_not_the_bare_one() {
        let mut p = panel();
        for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            assert_eq!(
                p.handle_key(KeyEvent::new(KeyCode::Char('n'), modifiers)),
                KeyOutcome::Ignored
            );
        }
    }

    #[test]
    fn the_panel_claims_only_the_space_it_can_actually_use() {
        use crate::panel::Panel as _;
        let p = panel();

        // Pinned as figures rather than as a formula, so a change to either has
        // to be deliberate. A timer is a small fact and should not be sized like
        // the thing you are working on: at scale 3 this declared 102 columns and
        // 21 rows, and took them from the task list.
        assert_eq!(p.max_width(), Some(42));
        assert_eq!(p.max_height(), Some(10));

        // And what it declares must be enough to draw what it draws.
        let time = "88:88";
        let scale = clock_scale(time, 42 - FRAME_WIDTH, 10 - FRAME_HEIGHT - LABEL_AND_FOOTER);
        assert!(
            scale.is_some(),
            "the declared width must fit the numerals, or the panel caps itself \
             into its own plain-text fallback"
        );
        assert_eq!(
            LABEL_AND_FOOTER + clock_rows(time, scale) + FRAME_HEIGHT,
            10,
            "the declared height must be exactly what the rows add up to"
        );
    }

    #[test]
    fn the_panel_reports_its_current_durations() {
        use crate::panel::Panel as _;

        // Reported unconditionally, on purpose. Deciding what counts as a
        // *change* is `UiState::only_changes_from`'s job, because a panel built
        // from an already-remembered value cannot tell the difference — which is
        // how a preference used to become impossible to retract. See
        // `a_preference_toggled_back_is_forgotten` in `state.rs`.
        let mut p = panel();
        let mut state = crate::state::UiState::default();
        p.remember(&mut state);
        assert_eq!(state.pomodoro_focus_minutes, Some(25));

        p.adjust(1);
        let mut state = crate::state::UiState::default();
        p.remember(&mut state);
        assert_eq!(state.pomodoro_focus_minutes, Some(26));
        assert_eq!(
            state.pomodoro_short_break_minutes,
            Some(5),
            "the untouched break is still reported; the diff drops it"
        );
    }

    fn screen(p: &mut PomodoroPanel, width: u16, height: u16) -> Vec<String> {
        crate::widgets::testing::rows(&crate::widgets::testing::rendered(p, width, height))
    }

    /// The phase label, and the time where it falls back to text, are cut
    /// with `…` when the panel is narrower than they are — never by the
    /// terminal, which left `SHORT BRE` at nine columns: a label that looks
    /// whole and is not. The silent-clip sweep in `widgets` cannot see it,
    /// because it builds the panel in focus, and `FOCUS` is short.
    #[test]
    fn the_label_and_the_time_say_when_they_have_been_cut() {
        // One step to a short break; seven to the long break after a set.
        for steps in [1, 7] {
            let mut p = panel();
            for _ in 0..steps {
                p.advance();
            }
            let label = glyphs::utility(p.phase.label());
            let time = clock_text(p.remaining());
            let whole_or_marked = |drawn: &str, full: &str| {
                drawn == full
                    || drawn
                        .strip_suffix('…')
                        .is_some_and(|kept| full.starts_with(kept))
            };
            let mut cut = 0;
            for width in 1..=14u16 {
                // Six rows is too short for the numerals: the label, the
                // time as text, the meter and the pips, from the second row.
                let rows = screen(&mut p, width, 6);
                let (drawn_label, drawn_time) = (rows[1].trim(), rows[2].trim());
                assert!(
                    whole_or_marked(drawn_label, &label),
                    "{label} drawn as {drawn_label:?} at width {width}"
                );
                assert!(
                    whole_or_marked(drawn_time, &time),
                    "{time} drawn as {drawn_time:?} at width {width}"
                );
                cut += usize::from(drawn_label != label) + usize::from(drawn_time != time);
            }
            assert!(cut > 0, "the sweep must reach widths that cut something");
        }
    }

    #[test]
    fn global_keys_are_not_swallowed() {
        let mut p = panel();
        let q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(p.handle_key(q), KeyOutcome::Ignored);
        assert!(
            !p.captures_input(),
            "the timer has no text entry to protect"
        );
    }
}
