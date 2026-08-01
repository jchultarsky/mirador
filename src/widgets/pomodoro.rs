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

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::config::PomodoroConfig;
use crate::frame::{Binding, FRAME_HEIGHT, FRAME_WIDTH};
use crate::glyphs::{self, BigText};
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

/// A pomodoro timer panel.
pub struct PomodoroPanel {
    config: PomodoroConfig,
    phase: Phase,
    /// When the current phase ends. `None` whenever the timer is not running.
    ends_at: Option<Instant>,
    /// What is left of the phase while stopped. Meaningless while running —
    /// [`PomodoroPanel::remaining`] reads the deadline instead, so a laptop
    /// that slept through a phase wakes up knowing the time has gone.
    paused_remaining: Duration,
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
        Self {
            config,
            phase: Phase::Focus,
            ends_at: None,
            paused_remaining: first,
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
            Some(deadline) => deadline.saturating_duration_since(Instant::now()),
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
            self.ends_at = Some(Instant::now() + self.paused_remaining);
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
    /// The next phase always starts from its full length against a fresh
    /// `Instant`, never chained from the deadline that just passed. That is
    /// what stops a machine resumed from an hour of sleep from racing through
    /// six phases to catch up: at most one phase ends per tick.
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
            Some(Instant::now() + self.paused_remaining)
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
        let minutes = match self.phase {
            Phase::Focus => &mut self.config.focus_minutes,
            Phase::ShortBreak => &mut self.config.short_break_minutes,
            Phase::LongBreak => &mut self.config.long_break_minutes,
        };

        let before = *minutes;
        let after = before.saturating_add_signed(delta).clamp(1, MAX_MINUTES);
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
            Some(_) => self.ends_at = Some(Instant::now() + adjusted),
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

const BINDINGS: &[Binding] = &[
    Binding::primary("space", "start/pause"),
    Binding::primary("n", "next phase"),
    Binding::primary("+/-", "length"),
    Binding::extra("r", "reset phase"),
];

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

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
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
        // rather than as another value.
        if cursor < bottom {
            let label = glyphs::utility(self.phase.label());
            let x = area.x + area.width.saturating_sub(display_width(&label)) / 2;
            frame.render_widget(
                Paragraph::new(Span::styled(
                    label.clone(),
                    Style::default()
                        .fg(phase_colour)
                        .add_modifier(Modifier::BOLD),
                )),
                Rect::new(x, cursor, display_width(&label).min(area.width), 1),
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
        // Ctrl-modified keys belong to the shell; only Shift is meaningful here
        // and only because `+` arrives that way on most layouts.
        if key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT)
        {
            return KeyOutcome::Ignored;
        }

        match key.code {
            KeyCode::Char(' ') => self.toggle(),
            KeyCode::Char('n') => self.advance(),
            KeyCode::Char('r') => self.reset_phase(),
            KeyCode::Char('+' | '=') => self.adjust(1),
            KeyCode::Char('-' | '_') => self.adjust(-1),
            _ => return KeyOutcome::Ignored,
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
        let width = display_width(time);
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

/// Width in display cells, as a `u16` because every caller here is doing
/// arithmetic on `Rect` fields. The measurement itself is `grid::display_width`.
fn display_width(text: &str) -> u16 {
    u16::try_from(crate::grid::display_width(text)).unwrap_or(u16::MAX)
}

/// One row of meter: filled cells in `fill`, the rest in `track`, same glyph
/// throughout so the footprint never moves.
fn meter_line(percent: u16, width: u16, fill: Color, track: Color) -> Vec<Span<'static>> {
    let width = usize::from(width);
    let filled = usize::from(percent.min(100)) * width / 100;
    (0..width)
        .map(|i| {
            Span::styled(
                "■",
                Style::default().fg(if i < filled { fill } else { track }),
            )
        })
        .collect()
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
        p.ends_at = Some(Instant::now());

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
        p.ends_at = Some(Instant::now() + Duration::from_secs(30));

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
        expired.ends_at = Some(Instant::now());
        expired.tick();
        assert!(
            expired.chime_error.is_some(),
            "a phase that ran out must announce itself"
        );
        assert_eq!(expired.phase, Phase::ShortBreak, "and still advance");
    }

    /// Every key the panel answers to, paired with the binding documenting it.
    const DOCUMENTED_KEYS: &[(KeyCode, &str)] = &[
        (KeyCode::Char(' '), "space"),
        (KeyCode::Char('n'), "n"),
        (KeyCode::Char('r'), "r"),
        (KeyCode::Char('+'), "+/-"),
        (KeyCode::Char('='), "+/-"),
        (KeyCode::Char('-'), "+/-"),
        (KeyCode::Char('_'), "+/-"),
    ];

    #[test]
    fn every_documented_key_works_and_every_working_key_is_documented() {
        for (code, key) in DOCUMENTED_KEYS {
            assert!(
                BINDINGS.iter().any(|b| b.key == *key),
                "`{key}` is handled but missing from BINDINGS"
            );
            let mut p = panel();
            assert_eq!(
                p.handle_key(KeyEvent::new(*code, KeyModifiers::NONE)),
                KeyOutcome::Consumed,
                "`{key}` is documented but the panel ignores it"
            );
        }

        for binding in BINDINGS {
            assert!(
                DOCUMENTED_KEYS.iter().any(|(_, key)| *key == binding.key),
                "`{}` is in BINDINGS but nothing here proves it works",
                binding.key
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
