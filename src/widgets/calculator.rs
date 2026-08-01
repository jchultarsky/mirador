//! A calculator, drawn as an adding machine's tape.
//!
//! The arithmetic lives in [`crate::calc`]; this is the instrument around it.
//!
//! # Why this panel exists
//!
//! It answers none of the four questions the dashboard was built for, which is
//! the same thing that is true of the pomodoro timer. Both are here because
//! they were asked for — this one by a reader, and endorsed. Recorded so the
//! next person does not conclude the filter was forgotten.
//!
//! There is an argument for it beyond that, and it is about *reaching* rather
//! than reading. mirador's whole premise is a tab left open all day; the thing
//! you actually reach for during that day is a quick sum, and the alternative
//! is `bc` or `python3 -c` in a shell you have to go and find. This is the
//! first panel that is purely an instrument, with no data source behind it at
//! all.
//!
//! # Why a tape and not a big number
//!
//! **The first version drew the answer in block numerals, and it was wrong.**
//! The reasoning behind it was "the clock and the pomodoro use them", which is
//! a resemblance rather than a reason. Those two show one continuously changing
//! value that you *glance* at, and the numerals exist so it reads from across
//! the room. A calculated answer is not glanced at — it is read once and
//! checked against the working that produced it. Different job, different
//! instrument.
//!
//! There was an arithmetic tell as well: block numerals cost six cells a digit,
//! so a twelve-digit result wants ninety-four columns. At any ordinary panel
//! width the numerals only ever appeared for answers small enough not to need
//! them.
//!
//! What replaced it is the adding machine's tape, and that is not nostalgia.
//! The reason accountants still buy printing calculators is *audit*: the tape
//! is what lets you check the entry you made three lines ago. It also belongs
//! to the design thesis, which is a watch station — chronometer, weather glass,
//! watch log. A tape sits on that bench; a seven-segment display does not.
//!
//! Built on [`crate::grid`] rather than hand-composed, which is what every
//! other list panel here does and what makes it look like part of the same
//! instrument. It also means "never show a truncated number" is enforced by the
//! shared column machinery rather than by hand.
//!
//! # The input problem, and why `captures_input` stays false
//!
//! A calculator needs the digits, and `1`–`9` are the shell's jump-to-panel
//! keys. The obvious fix is [`Panel::captures_input`], and it is a trap: that
//! is an *absolute* veto (invariant 2), meant for transient modal states like
//! typing a task title. A panel that captured permanently would kill `Tab`,
//! `q` and `?` for as long as it held focus — a room with no door.
//!
//! It is not needed. `App::dispatch_key` offers every key to the focused panel
//! *first* and consults the global table only if the panel returns `Ignored`.
//! So this panel consumes what it needs and ignores the rest, and every global
//! key except the digits keeps working.
//!
//! The price, stated plainly because it is a real one: **while this panel is
//! focused, `1`–`9` type digits instead of jumping to panels.** `Tab` still
//! cycles, which is the way out.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::calc::{self, CalcError};
use crate::config::CalculatorConfig;
use crate::frame::{Binding, FRAME_HEIGHT, FRAME_WIDTH};
use crate::grid::{Column, Grid, display_width};
use crate::panel::{KeyOutcome, Panel, RenderContext};

/// This panel's bindings, written once, with the tape actions optional.
///
/// A macro rather than two arrays. The two lists share seven entries, and a
/// `Binding` declaration feeds three surfaces — border hint, status bar, help
/// overlay — so a list duplicated is hint text that will drift (invariant 3).
/// Only the two lines that genuinely differ are written twice, which is to say
/// once each.
///
/// The order matters because `frame::hint_line` fills the border with
/// *primaries* until it runs out of room: whatever is first is what a reader
/// sees without pressing `?`. Extras never reach the border, so their position
/// is free.
macro_rules! calculator_bindings {
    ($($tape:expr),* $(,)?) => {
        &[
            $($tape,)*
            Binding::primary("0-9 + - * /", "type"),
            Binding::primary("Enter", "keep"),
            Binding::primary("c", "clear"),
            Binding::extra("( )", "group"),
            Binding::extra("x", "multiply"),
            Binding::extra("C", "clear the tape too"),
            Binding::extra("Backspace", "rub out"),
            Binding::extra("\u{2191} / \u{2193}", "select on the tape"),
        ]
    };
}

/// Before anything has been worked out, the only thing worth advertising is
/// that the number keys type.
const ENTRY_BINDINGS: &[Binding] = calculator_bindings!();

/// Once there is a tape, its result actions outrank another reminder that the
/// number keys type. At the default width this puts `y copy \u{00b7} p paste` in
/// the border together instead of leaving both behind `?`.
const TAPE_BINDINGS: &[Binding] = calculator_bindings!(
    Binding::primary("y", "copy"),
    Binding::primary("p", "paste"),
);

/// Widest the result column is drawn.
///
/// Twelve significant digits plus a sign and a point is the most `calc` can
/// produce in decimal; past that it moves to scientific notation, which is
/// shorter. So this is a ceiling derived from the formatter rather than a
/// guess at what looks right.
const RESULT_WIDTH: u16 = 14;

/// The narrowest grid at which the working is still worth a column.
///
/// Below this the tape shows answers alone, which is the honest degradation:
/// an answer with no visible sum is still an answer, where a sum with no answer
/// is nothing at all.
const WORKING_MIN: u16 = RESULT_WIDTH + crate::grid::GUTTER + 8;

pub(crate) const COLUMNS: &[Column] = &[
    Column::flex("working", 1).drops_below(WORKING_MIN),
    Column::fixed("result", RESULT_WIDTH).right(),
];

/// Entries kept on the tape.
///
/// A bound rather than a target — the tape shows what fits, and this only stops
/// a long session growing without limit. Nothing reads past what is drawn.
const MAX_TAPE: usize = 200;

/// Interior width past which the panel gains nothing.
///
/// Past this, extra cells only widen the gap between a sum and its answer, and
/// taking room the task list could use would be worse. See invariant 15.
const USEFUL_WIDTH: u16 = 46;

/// Rows the panel always spends: the column header, the rule, the live entry.
const CHROME_ROWS: u16 = 3;

/// The live-entry marker, matching the task and notes lists.
const MARKER: &str = "\u{25b8} ";

/// One finished calculation.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    expression: String,
    result: f64,
}

pub struct CalculatorPanel {
    #[allow(dead_code)]
    config: CalculatorConfig,
    /// What is being typed.
    typing: String,
    /// What `typing` currently works out to, recomputed on every keystroke.
    ///
    /// Cached rather than evaluated at draw time: `render` runs every frame and
    /// nothing guards it, so parsing there would put the parser on the frame
    /// budget for a value that changes only when a key is pressed. Same
    /// reasoning as the agenda's cache.
    preview: Result<f64, CalcError>,
    /// Finished calculations, oldest first — the order a tape feeds.
    tape: Vec<Entry>,
    /// How far back through the tape the view is scrolled, in rows.
    scrolled: usize,
    /// Selected tape entry, counted back from the newest result.
    selected_back: usize,
    /// Tape rows drawn last frame, so selection scrolls by the visible window.
    drawn: usize,
    /// What the last `y` did. Cleared by the next keystroke.
    ///
    /// OSC 52 is write-only, so this says what was *sent* rather than claiming
    /// the clipboard changed — the same honesty the news panel's copy uses.
    action: Option<String>,
}

impl std::fmt::Debug for CalculatorPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalculatorPanel")
            .field("typing", &self.typing)
            .field("tape", &self.tape.len())
            .finish_non_exhaustive()
    }
}

impl CalculatorPanel {
    pub fn new(config: CalculatorConfig) -> Self {
        Self {
            config,
            typing: String::new(),
            preview: Err(CalcError::Empty),
            tape: Vec::new(),
            scrolled: 0,
            selected_back: 0,
            drawn: 0,
            action: None,
        }
    }

    /// The answer on the tape's last line, used for operator chaining.
    fn last_result(&self) -> Option<f64> {
        self.tape.last().map(|entry| entry.result)
    }

    /// The answer the full-weight tape row identifies.
    fn selected_result(&self) -> Option<f64> {
        let last = self.tape.len().checked_sub(1)?;
        self.tape
            .get(last.saturating_sub(self.selected_back))
            .map(|entry| entry.result)
    }

    /// Re-derive the preview. Called wherever `typing` changes, so the two
    /// cannot drift.
    fn reparse(&mut self) {
        self.preview = calc::evaluate(&self.typing);
    }

    /// Add a character to the expression being typed.
    ///
    /// An operator typed straight after an answer continues from it — `57`,
    /// then `+10`, reads as `57 + 10`. A digit starts fresh instead. That is
    /// how a desk calculator chains, and it is what removes the need for a
    /// memory key: "and now add the tax" costs no extra concept.
    fn push(&mut self, c: char) {
        if self.typing.is_empty()
            && let Some(value) = self.last_result()
            && matches!(c, '+' | '-' | '*' | '/' | 'x' | '\u{00D7}' | '\u{00F7}')
        {
            self.typing = calc::format_result(value);
        }
        // Past this the expression cannot be evaluated anyway.
        if self.typing.chars().count() < calc::MAX_LEN {
            self.typing.push(c);
        }
        self.reparse();
    }

    /// Commit the current entry to the tape, if it works out.
    fn keep(&mut self) {
        if let Ok(value) = self.preview {
            self.tape.push(Entry {
                expression: self.typing.trim().to_string(),
                result: value,
            });
            // Oldest goes first: the part of a tape you have already scrolled
            // past is the part you are least likely to want back.
            if self.tape.len() > MAX_TAPE {
                self.tape.remove(0);
            }
            self.typing.clear();
            self.reparse();
            // Back to the foot of the tape, where the new entry is.
            self.scrolled = 0;
            self.selected_back = 0;
        }
    }

    /// The rows of the tape currently in view, oldest first.
    ///
    /// Only what fits is produced. A panel may allocate in proportion to what
    /// is on screen and must not allocate in proportion to how much it holds.
    fn visible(&self, rows: usize) -> &[Entry] {
        if rows == 0 || self.tape.is_empty() {
            return &[];
        }
        let end = self.tape.len().saturating_sub(self.scrolled).max(1);
        let start = end.saturating_sub(rows);
        &self.tape[start..end]
    }

    /// Keep the selected result inside the visible slice of tape.
    fn scroll_selection_into_view(&mut self, rows: usize) {
        let Some(last) = self.tape.len().checked_sub(1) else {
            self.selected_back = 0;
            self.scrolled = 0;
            return;
        };
        self.selected_back = self.selected_back.min(last);
        let rows = rows.max(1).min(self.tape.len());
        if self.selected_back < self.scrolled {
            self.scrolled = self.selected_back;
        } else if self.selected_back >= self.scrolled.saturating_add(rows) {
            self.scrolled = self.selected_back + 1 - rows;
        }
        self.scrolled = self.scrolled.min(self.tape.len().saturating_sub(rows));
    }

    fn select_older(&mut self) {
        let Some(last) = self.tape.len().checked_sub(1) else {
            return;
        };
        self.selected_back = self.selected_back.saturating_add(1).min(last);
        self.scroll_selection_into_view(self.drawn);
    }

    fn select_newer(&mut self) {
        self.selected_back = self.selected_back.saturating_sub(1);
        self.scroll_selection_into_view(self.drawn);
    }

    fn copy_selected(&mut self) {
        self.copy_selected_with(crate::clipboard::copy);
    }

    fn copy_selected_with(&mut self, copy: impl FnOnce(&str) -> std::io::Result<()>) {
        let Some(value) = self.selected_result() else {
            return;
        };
        let text = calc::format_result(value);
        self.action = Some(match copy(&text) {
            // OSC 52 is write-only: the terminal never answers, so this says
            // what was sent rather than that anything was copied.
            Ok(()) => format!("sent {text}"),
            Err(error) => format!("clipboard failed: {error}"),
        });
    }

    /// Paste the selected answer at the end of the live expression.
    fn paste_selected(&mut self) {
        let Some(value) = self.selected_result() else {
            return;
        };
        let text = calc::format_result(value);
        let length = self.typing.chars().count() + text.chars().count();
        if length > calc::MAX_LEN {
            self.action = Some("entry is full".to_string());
            return;
        }
        self.typing.push_str(&text);
        self.reparse();
        self.selected_back = 0;
        self.scrolled = 0;
    }
}

/// Results padded so their decimal points sit in the same column.
///
/// Right-alignment alone lines up the last *character*, which puts the `5` of
/// `7.5` where the units of `384` are and makes a column of answers unreadable
/// at a glance. Accounting tables have always aligned on the point, and this is
/// the detail that makes a tape scan.
///
/// Padding goes on the right and the grid then right-aligns the whole cell, so
/// the points land together whatever the column width.
fn align_points(results: &[String], column: usize) -> Vec<String> {
    let fraction = |text: &str| text.rfind('.').map_or(0, |at| text.len() - at - 1);
    let padding = |text: &str, widest: usize| {
        let pad = widest.saturating_sub(fraction(text));
        // A whole number needs a cell for the point it has not got, or it sits
        // one place right of everything that has a fraction.
        pad + usize::from(pad > 0 && !text.contains('.'))
    };

    // Scientific notation stays out of the reckoning: the point in `1.2e11`
    // means something else, and padding to it would shove every ordinary answer
    // sideways to line up with a case that is already exceptional.
    let ordinary = || results.iter().filter(|text| !text.contains('e'));
    let mut widest = ordinary().map(|text| fraction(text)).max().unwrap_or(0);

    // **Padding counts against the column.** The numbers arrive here already
    // fitted, so adding cells to line up the points can push one back out —
    // and the grid then ellipsises it, which undoes the whole reason
    // `fit_result` exists. A ten-cell answer beside a two-place fraction came
    // out as `123456789…`: the number rescued from truncation, truncated by
    // the thing meant to make it readable.
    //
    // So the alignment gives way rather than the number. Losing a place of
    // alignment costs neatness; losing a digit costs the answer.
    while widest > 0 && ordinary().any(|text| text.chars().count() + padding(text, widest) > column)
    {
        widest -= 1;
    }

    results
        .iter()
        .map(|text| {
            if text.contains('e') {
                return text.clone();
            }
            format!("{text}{}", " ".repeat(padding(text, widest)))
        })
        .collect()
}

impl Panel for CalculatorPanel {
    fn title(&self) -> String {
        "Calculator".to_string()
    }

    /// Deliberately `None`.
    ///
    /// A count of tape entries is a badge, and a badge that only goes up is the
    /// unread counter this dashboard turned down.
    fn counter(&self) -> Option<String> {
        None
    }

    fn bindings(&self) -> &'static [Binding] {
        if self.tape.is_empty() {
            ENTRY_BINDINGS
        } else {
            TAPE_BINDINGS
        }
    }

    fn max_width(&self) -> Option<u16> {
        Some(USEFUL_WIDTH + FRAME_WIDTH)
    }

    /// Deliberately `None`: the tape is content, and rows are what it is made
    /// of. Same reasoning as the watch log.
    fn max_height(&self) -> Option<u16> {
        None
    }

    /// Nothing here changes on its own.
    fn tick(&mut self) -> bool {
        false
    }

    /// Never. See the module header: this panel gets the keys it needs by
    /// consuming them, not by vetoing the shell.
    fn captures_input(&self) -> bool {
        false
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        // Ctrl and Alt belong to the shell — Ctrl+arrows resize, Ctrl+C quits.
        if key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT)
        {
            return KeyOutcome::Ignored;
        }

        // Any key retires the copy notice; it has been read or it has not.
        let had_action = self.action.take().is_some();

        match key.code {
            KeyCode::Char(
                c @ ('0'..='9'
                | '.'
                | '+'
                | '-'
                | '*'
                | '/'
                | '('
                | ')'
                | 'x'
                | '\u{00D7}'
                | '\u{00F7}'),
            ) => self.push(c),
            KeyCode::Char('=') | KeyCode::Enter => self.keep(),
            KeyCode::Backspace => {
                self.typing.pop();
                self.reparse();
            }
            // `c` clears the entry and `C` the tape with it — the CE and AC of
            // every desk calculator, a convention worth inheriting rather than
            // inventing around. The first version had only `Esc`, listed as a
            // secondary binding, which is why nobody could find it.
            KeyCode::Char('c') => {
                self.typing.clear();
                self.reparse();
            }
            KeyCode::Char('C') => {
                self.typing.clear();
                self.tape.clear();
                self.scrolled = 0;
                self.selected_back = 0;
                self.reparse();
            }
            KeyCode::Esc => {
                // Consumed only while there is something to clear, so Esc on an
                // empty calculator falls through to the shell as it does
                // everywhere else.
                if self.typing.is_empty() && !had_action {
                    return KeyOutcome::Ignored;
                }
                self.typing.clear();
                self.reparse();
            }
            KeyCode::Char('y') if !self.tape.is_empty() => self.copy_selected(),
            KeyCode::Char('p') if !self.tape.is_empty() => self.paste_selected(),
            KeyCode::Up => self.select_older(),
            KeyCode::Down => self.select_newer(),
            _ => {
                // A key this panel does not want goes to the shell — unless it
                // only served to retire the copy notice, which is a visible
                // change and so counts as having been used.
                if had_action {
                    return KeyOutcome::Consumed;
                }
                return KeyOutcome::Ignored;
            }
        }
        KeyOutcome::Consumed
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> KeyOutcome {
        match event.kind {
            MouseEventKind::ScrollUp => self.select_older(),
            MouseEventKind::ScrollDown => self.select_newer(),
            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        if area.width == 0 || area.height == 0 {
            self.drawn = 0;
            return;
        }
        let theme = ctx.theme;

        // The marker's width comes off the grid and goes back as an indent, so
        // the header lines up with the rows under it.
        let marker = u16::try_from(display_width(MARKER)).unwrap_or(2);
        let grid = Grid::new(COLUMNS, area.width.saturating_sub(marker));
        let indent = " ".repeat(usize::from(marker));

        let bottom = area.y + area.height;
        let mut cursor = area.y;

        // The header, in the utility face, exactly as the other list panels
        // label their columns. It is also what makes an untouched calculator
        // look ready rather than broken.
        if cursor < bottom {
            let mut spans = vec![Span::raw(indent.clone())];
            spans.extend(grid.header(theme).spans);
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect::new(area.x, cursor, area.width, 1),
            );
            cursor += 1;
        }

        let room = usize::from(
            bottom
                .saturating_sub(cursor)
                .saturating_sub(CHROME_ROWS - 1),
        );
        self.scroll_selection_into_view(room);
        // Cloned rather than borrowed: `self.drawn` has to be recorded before
        // the rows are drawn, and holding a slice of `self.tape` across that
        // would borrow the whole panel. The clone is of what is on screen, not
        // of the tape — the rule is that a panel may allocate in proportion to
        // what it displays.
        let entries: Vec<Entry> = self.visible(room).to_vec();
        self.drawn = entries.len();

        // Fitted to the column *before* the grid sees it. The grid ellipsises a
        // cell that will not fit, which is right for a name and wrong for a
        // number — a narrow panel drew `123456789…` for 123456789000, which is
        // the one thing this panel must never do. `fit_result` moves to
        // scientific notation instead, and the grid then has nothing to cut.
        let column = usize::from(grid.column_width("result"));
        // The live answer is aligned *with* the tape, not separately. Aligning
        // the two independently left the result being typed one cell right of
        // the column above it, which is exactly the misalignment the padding
        // exists to remove — and the most visible one, since those two rows sit
        // either side of the rule.
        let mut figures: Vec<String> = entries
            .iter()
            .map(|entry| calc::fit_result(entry.result, column))
            .collect();
        let live = self
            .preview
            .as_ref()
            .ok()
            .map(|v| calc::fit_result(*v, column));
        if let Some(value) = &live {
            figures.push(value.clone());
        }
        let mut aligned = align_points(&figures, column);
        let live = live.map(|_| aligned.pop().unwrap_or_default());

        // Attention runs *downwards*, to the line being typed. The tape recedes
        // to muted, its selected entry stays at full body weight, and the live
        // row below is brighter than either. The first version had this
        // backwards and put the faintest thing on screen where the cursor was.
        let selected = self
            .scrolled
            .saturating_add(entries.len().saturating_sub(1))
            .saturating_sub(self.selected_back);
        for (index, entry) in entries.iter().enumerate() {
            if cursor >= bottom {
                break;
            }
            let style = Style::default().fg(if index == selected {
                theme.text
            } else {
                theme.muted
            });
            let mut spans = vec![Span::raw(indent.clone())];
            spans.extend(
                grid.row(&[
                    Span::styled(entry.expression.clone(), style),
                    Span::styled(aligned[index].clone(), style),
                ])
                .spans,
            );
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect::new(area.x, cursor, area.width, 1),
            );
            cursor += 1;
        }

        // The live entry sits at the foot of the panel, where a tape feeds
        // from, rather than wandering up and down as the tape fills.
        cursor = bottom.saturating_sub(2).max(cursor);

        // The rule between what is kept and what is being typed. The notes
        // panel uses the same device between its list and its detail.
        if cursor < bottom {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "\u{2500}".repeat(usize::from(area.width)),
                    Style::default().fg(theme.rule),
                )),
                Rect::new(area.x, cursor, area.width, 1),
            );
            cursor += 1;
        }

        if cursor < bottom {
            self.draw_entry(frame, area, cursor, &grid, theme, live.as_deref());
        }
    }
}

impl CalculatorPanel {
    /// The live entry: the brightest row in the panel, because it is the one
    /// the cursor is on.
    fn draw_entry(
        &self,
        frame: &mut Frame,
        area: Rect,
        row: u16,
        grid: &Grid,
        theme: &crate::theme::Theme,
        live: Option<&str>,
    ) {
        // A half-typed sum is the state this panel spends most of its life in,
        // so those say nothing at all rather than complaining. Only a sum that
        // cannot ever work gets a word.
        let complaint = match &self.preview {
            Ok(_) | Err(CalcError::Empty | CalcError::Incomplete) => None,
            Err(error) => Some(error.message()),
        };

        let column = usize::from(grid.column_width("result"));
        let (working, result, style) = if let Some(notice) = &self.action {
            (
                notice.clone(),
                String::new(),
                Style::default().fg(theme.label),
            )
        } else if let Some(message) = complaint {
            (
                format!("{}\u{258f}", self.typing),
                crate::grid::truncate(&message, column),
                Style::default().fg(theme.error),
            )
        } else {
            // Already fitted and aligned with the tape by the caller.
            let answer = live.unwrap_or_default().to_string();
            (
                format!("{}\u{258f}", self.typing),
                answer,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
        };

        let mut spans = vec![Span::styled(MARKER, Style::default().fg(theme.accent))];
        spans.extend(
            grid.row(&[
                Span::styled(working, Style::default().fg(theme.text)),
                Span::styled(result, style),
            ])
            .spans,
        );
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(area.x, row, area.width, 1),
        );
    }
}

/// Rows the frame costs, named so `max_height`'s reasoning is checkable.
#[allow(dead_code)]
const _: u16 = FRAME_HEIGHT;

// Exact comparison is the point: these are answers a calculator must get
// exactly right, not measurements to be compared within a tolerance.
#[allow(clippy::float_cmp)]
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn new_panel() -> CalculatorPanel {
        CalculatorPanel::new(CalculatorConfig::default())
    }

    fn press(panel: &mut CalculatorPanel, c: char) -> KeyOutcome {
        panel.handle_key(KeyEvent::from(KeyCode::Char(c)))
    }

    fn type_in(panel: &mut CalculatorPanel, text: &str) {
        for c in text.chars() {
            press(panel, c);
        }
    }

    fn enter(panel: &mut CalculatorPanel) {
        panel.handle_key(KeyEvent::from(KeyCode::Enter));
    }

    fn buffer_of(panel: &mut CalculatorPanel, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                panel.render(
                    frame,
                    frame.area(),
                    RenderContext {
                        theme: &config.theme,
                        gradients: &gradients,
                        focused: true,
                        watch: &crate::watch::WatchLog::default(),
                    },
                );
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn draw(panel: &mut CalculatorPanel, width: u16, height: u16) -> String {
        let buffer = buffer_of(panel, width, height);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The whole input design in one test.
    ///
    /// Digits must reach the calculator, and everything the shell owns must
    /// still reach the shell. Get the first half wrong and the panel is
    /// useless; get the second half wrong and it is a room with no door.
    #[test]
    fn digits_are_taken_and_the_shell_keys_are_left_alone() {
        let mut panel = new_panel();
        for c in "0123456789.+-*/()x".chars() {
            assert_eq!(
                press(&mut panel, c),
                KeyOutcome::Consumed,
                "{c:?} has to reach the calculator"
            );
        }

        let mut panel = new_panel();
        for code in [
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Char('q'),
            KeyCode::Char('?'),
            KeyCode::Char('w'),
            KeyCode::Char('t'),
            KeyCode::Char('m'),
        ] {
            assert_eq!(
                panel.handle_key(KeyEvent::from(code)),
                KeyOutcome::Ignored,
                "{code:?} belongs to the shell; consuming it locks the user in"
            );
        }
        assert!(!panel.captures_input());
    }

    #[test]
    fn the_answer_forms_as_you_type_and_enter_keeps_it() {
        let mut panel = new_panel();
        type_in(&mut panel, "2+3*4");
        assert_eq!(panel.preview, Ok(14.0), "the answer forms before Enter");
        assert!(panel.tape.is_empty(), "but nothing is kept until Enter");

        enter(&mut panel);
        assert_eq!(panel.tape.len(), 1);
        assert_eq!(panel.tape[0].expression, "2+3*4");
        assert!(panel.typing.is_empty());
    }

    /// The tape feeds like a tape: newest at the bottom, beside the cursor.
    #[test]
    fn the_newest_entry_is_at_the_foot_of_the_tape() {
        let mut panel = new_panel();
        for sum in ["1+1", "2+2", "3+3"] {
            type_in(&mut panel, sum);
            enter(&mut panel);
        }
        assert_eq!(panel.last_result(), Some(6.0));

        let screen = draw(&mut panel, 36, 9);
        let rows: Vec<&str> = screen.lines().collect();
        let at = |needle: &str| rows.iter().position(|r| r.contains(needle)).unwrap();
        assert!(
            at("1+1") < at("2+2") && at("2+2") < at("3+3"),
            "the tape must read oldest to newest downwards:\n{screen}"
        );
    }

    /// The correction that came out of looking at it on screen.
    ///
    /// The live row is where the cursor is, so it cannot be the faintest thing
    /// in the panel. Only that row carries the accent; the tape must not.
    #[test]
    fn only_the_live_row_is_accented() {
        let mut panel = new_panel();
        type_in(&mut panel, "1+1");
        enter(&mut panel);
        type_in(&mut panel, "9*9");

        let config = crate::config::Config::default();
        let buffer = buffer_of(&mut panel, 36, 9);

        let mut accented = Vec::new();
        for y in 0..9u16 {
            if (0..36u16).any(|x| buffer[(x, y)].style().fg == Some(config.theme.accent)) {
                accented.push(y);
            }
        }
        assert_eq!(
            accented.len(),
            1,
            "exactly one row should be accented, got {accented:?}"
        );
        assert_eq!(
            accented[0], 8,
            "and it should be the live entry at the foot of the panel"
        );
    }

    #[test]
    fn c_clears_the_entry_and_shift_c_clears_the_tape() {
        let mut panel = new_panel();
        type_in(&mut panel, "1+1");
        enter(&mut panel);
        type_in(&mut panel, "99");

        press(&mut panel, 'c');
        assert!(panel.typing.is_empty(), "c clears what is being typed");
        assert_eq!(panel.tape.len(), 1, "and leaves the tape alone");

        press(&mut panel, 'C');
        assert!(panel.tape.is_empty(), "C clears the tape as well");
    }

    #[test]
    fn an_operator_after_an_answer_carries_it_forward() {
        let mut panel = new_panel();
        type_in(&mut panel, "50+7");
        enter(&mut panel);
        press(&mut panel, '+');
        assert_eq!(panel.typing, "57+");
        type_in(&mut panel, "10");
        enter(&mut panel);
        assert_eq!(panel.last_result(), Some(67.0));
    }

    #[test]
    fn a_digit_after_an_answer_starts_over() {
        let mut panel = new_panel();
        type_in(&mut panel, "50+7");
        enter(&mut panel);
        press(&mut panel, '9');
        assert_eq!(panel.typing, "9");
    }

    #[test]
    fn arrows_select_which_tape_result_is_copied() {
        let mut panel = new_panel();
        for sum in ["1+1", "2+2", "3+3"] {
            type_in(&mut panel, sum);
            enter(&mut panel);
        }
        draw(&mut panel, 36, 9);

        panel.handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(panel.selected_result(), Some(4.0));
        panel.copy_selected_with(|text| {
            assert_eq!(text, "4");
            Ok(())
        });
        assert_eq!(panel.action.as_deref(), Some("sent 4"));

        panel.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(panel.selected_result(), Some(6.0));
    }

    #[test]
    fn p_pastes_the_selected_result_into_the_live_expression() {
        let mut panel = new_panel();
        for sum in ["1+1", "2+2", "3+3"] {
            type_in(&mut panel, sum);
            enter(&mut panel);
        }
        draw(&mut panel, 36, 9);
        panel.handle_key(KeyEvent::from(KeyCode::Up));
        panel.handle_key(KeyEvent::from(KeyCode::Up));
        type_in(&mut panel, "10+");

        assert_eq!(press(&mut panel, 'p'), KeyOutcome::Consumed);
        assert_eq!(panel.typing, "10+2");
        assert_eq!(panel.preview, Ok(12.0));
        assert_eq!(panel.selected_back, 0, "the view returns to the tape foot");
        assert_eq!(panel.scrolled, 0);
    }

    /// The two binding lists must differ only by the tape actions.
    ///
    /// They were two hand-written arrays sharing seven entries, which is the
    /// shape invariant 3 warns about: a `Binding` feeds the border hint, the
    /// status bar and the help overlay, so a list kept in two places is hint
    /// text waiting to disagree with itself. Now one macro emits both. This
    /// pins the property so a future edit cannot quietly unpick it.
    #[test]
    fn the_two_binding_lists_share_everything_but_the_tape_actions() {
        let tail: Vec<(&str, &str)> = TAPE_BINDINGS
            .iter()
            .skip(2)
            .map(|b| (b.key, b.action))
            .collect();
        let entry: Vec<(&str, &str)> = ENTRY_BINDINGS.iter().map(|b| (b.key, b.action)).collect();
        assert_eq!(
            entry, tail,
            "the shared part of the two lists has drifted apart"
        );
        assert_eq!(
            TAPE_BINDINGS[..2].iter().map(|b| b.key).collect::<Vec<_>>(),
            vec!["y", "p"],
            "the tape actions lead, because the border fills with primaries in order"
        );
    }

    #[test]
    fn result_actions_reach_the_border_at_the_default_width() {
        let mut panel = new_panel();
        assert_eq!(panel.bindings()[0].action, "type");

        type_in(&mut panel, "2+2");
        enter(&mut panel);
        let line = crate::frame::hint_line(panel.bindings(), &crate::theme::Theme::default(), 24)
            .expect("a 32-column panel leaves a 24-column hint budget");
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("y copy · p paste"), "got {text:?}");
    }

    #[test]
    fn a_sum_that_cannot_work_says_why_on_screen() {
        let mut panel = new_panel();
        type_in(&mut panel, "1/0");
        let screen = draw(&mut panel, 44, 9);
        assert!(
            screen.contains("\u{00f7} by zero"),
            "the reason has to be on screen, not just in the state:\n{screen}"
        );
        enter(&mut panel);
        assert!(panel.tape.is_empty(), "nothing that failed is kept");
        assert_eq!(panel.typing, "1/0", "and it stays there to be corrected");
    }

    /// Half-typed is the normal state, so it must not look like a failure.
    #[test]
    fn a_half_typed_sum_is_not_scolded() {
        let mut panel = new_panel();
        type_in(&mut panel, "2+");
        let screen = draw(&mut panel, 44, 9);
        assert!(
            !screen.contains("unfinished"),
            "an expression mid-typing must not be complained about:\n{screen}"
        );
    }

    /// Decimal points line up, which is what makes a column of answers scan.
    #[test]
    fn results_are_aligned_on_the_decimal_point() {
        let aligned = align_points(
            &[
                "384".to_string(),
                "7.5".to_string(),
                "0.25".to_string(),
                "60".to_string(),
            ],
            14,
        );
        for (text, original) in aligned.iter().zip(["384", "7.5", "0.25", "60"]) {
            assert!(
                text.starts_with(original),
                "{text:?} should keep {original:?}"
            );
        }
        // The property that matters is visual, so it is checked visually:
        // right-align the padded cells the way the grid will, then every units
        // digit must land in the same column. An earlier version of this test
        // compared fraction widths instead, and failed on `384` — which has no
        // decimal point at all, and was nonetheless aligned correctly.
        let width = aligned.iter().map(String::len).max().unwrap();
        let placed: Vec<String> = aligned.iter().map(|t| format!("{t:>width$}")).collect();
        let units = |row: &str| {
            row.rfind('.')
                .unwrap_or_else(|| row.trim_end().len())
                .saturating_sub(1)
        };
        let columns: Vec<usize> = placed.iter().map(|r| units(r)).collect();
        assert!(
            columns.windows(2).all(|w| w[0] == w[1]),
            "units digits land in different columns:\n{}",
            placed.join("\n")
        );
    }

    /// Alignment padding must not push a result back out of its column.
    ///
    /// `fit_result` fits the number, then `align_points` adds cells to line the
    /// decimal points up — and those cells count. A ten-cell answer beside a
    /// two-place fraction was padded to thirteen, which the grid then cut back
    /// to `123456789…`: the number rescued from truncation, truncated by the
    /// thing that was meant to make it readable.
    #[test]
    fn alignment_padding_never_pushes_a_result_out_of_its_column() {
        let mut panel = new_panel();
        type_in(&mut panel, "0.25*1");
        enter(&mut panel);
        type_in(&mut panel, "1234567890*1");
        enter(&mut panel);

        // The *result* is what may never be ellipsised. The working is prose
        // and is allowed to be cut — an earlier version of this assertion did
        // not distinguish them and failed on `1234567890*1` in the left column,
        // which was behaving correctly.
        for width in 12u16..=40 {
            for row in draw(&mut panel, width, 8).lines() {
                assert!(
                    !row.trim_end().ends_with('\u{2026}'),
                    "the result column was ellipsised at width {width}: {row:?}"
                );
            }
        }
    }

    #[test]
    fn scientific_results_are_left_out_of_the_alignment() {
        let aligned = align_points(&["1.2346e11".to_string(), "7.5".to_string()], 14);
        assert_eq!(
            aligned[0], "1.2346e11",
            "an exponent's point is not a place"
        );
    }

    /// The rule this panel is held to more strictly than any other.
    #[test]
    fn no_row_is_ever_wider_than_the_panel() {
        let mut panel = new_panel();
        for sum in ["123456789 * 1000", "1/3", "2+2", "0.1+0.2", "999999*999999"] {
            type_in(&mut panel, sum);
            enter(&mut panel);
        }
        type_in(&mut panel, "42*42");
        for width in 1u16..=50 {
            for row in draw(&mut panel, width, 10).lines() {
                assert!(
                    display_width(row) <= usize::from(width),
                    "a {}-cell row in a {width}-cell panel: {row:?}",
                    display_width(row)
                );
            }
        }
    }

    #[test]
    fn a_result_too_wide_is_never_shown_truncated() {
        let mut panel = new_panel();
        type_in(&mut panel, "123456789*1000");
        enter(&mut panel);
        for row in draw(&mut panel, 12, 8).lines() {
            let stripped: String = row.chars().filter(|c| !c.is_whitespace()).collect();
            assert!(
                !stripped.contains("123456789") || stripped.contains('e'),
                "a prefix of the answer reached the screen: {row:?}"
            );
        }
    }

    #[test]
    fn the_tape_is_bounded() {
        let mut panel = new_panel();
        for n in 0..MAX_TAPE + 50 {
            panel.typing = format!("{n}+1");
            panel.reparse();
            panel.keep();
        }
        assert_eq!(panel.tape.len(), MAX_TAPE);
        assert_eq!(
            panel.tape.last().unwrap().expression,
            format!("{}+1", MAX_TAPE + 49),
            "the newest survives; the oldest is dropped"
        );
    }

    #[test]
    fn only_the_rows_that_fit_are_built() {
        let mut panel = new_panel();
        for n in 0..MAX_TAPE {
            panel.typing = format!("{n}+1");
            panel.reparse();
            panel.keep();
        }
        draw(&mut panel, 36, 10);
        assert!(
            panel.drawn <= 10,
            "{} rows for a ten-row panel",
            panel.drawn
        );
        assert!(panel.drawn > 0);
    }

    /// Selection and its scroll window cannot run off either end of the tape.
    #[test]
    fn scrolling_is_bounded_at_both_ends() {
        let mut panel = new_panel();
        for n in 0..40 {
            panel.typing = format!("{n}+1");
            panel.reparse();
            panel.keep();
        }
        draw(&mut panel, 36, 12);
        for _ in 0..200 {
            panel.handle_key(KeyEvent::from(KeyCode::Up));
        }
        assert_eq!(panel.selected_back, panel.tape.len() - 1);
        assert_eq!(panel.scrolled, panel.tape.len() - panel.drawn);
        draw(&mut panel, 36, 12);
        for _ in 0..200 {
            panel.handle_key(KeyEvent::from(KeyCode::Down));
        }
        assert_eq!(panel.selected_back, 0, "the newest answer is selected");
        assert_eq!(panel.scrolled, 0, "back at the foot of the tape");
    }

    #[test]
    fn the_panel_draws_at_any_size_without_panicking() {
        let mut panel = new_panel();
        type_in(&mut panel, "12345+6789");
        enter(&mut panel);
        type_in(&mut panel, "1/0");
        for width in [1u16, 2, 3, 7, 12, 20, 36, 46, 120] {
            for height in [1u16, 2, 3, 5, 9, 30] {
                let _ = draw(&mut panel, width, height);
            }
        }
    }

    /// An empty panel must look ready rather than broken (invariant 11).
    #[test]
    fn an_untouched_calculator_shows_its_columns() {
        let mut panel = new_panel();
        let screen = draw(&mut panel, 36, 9);
        assert!(
            screen.contains("WORKING") && screen.contains("RESULT"),
            "the header is what says the panel is ready:\n{screen}"
        );
    }
}
