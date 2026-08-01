//! A calculator: type an expression, press Enter, get the answer.
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
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::calc::{self, CalcError};
use crate::config::CalculatorConfig;
use crate::frame::{Binding, FRAME_HEIGHT, FRAME_WIDTH};
use crate::glyphs::{self, BigText};
use crate::grid::display_width;
use crate::panel::{KeyOutcome, Panel, RenderContext};

const BINDINGS: &[Binding] = &[
    Binding::primary("0-9 + - * /", "type"),
    Binding::primary("Enter", "work it out"),
    Binding::primary("y", "copy"),
    Binding::extra("( )", "group"),
    Binding::extra("Backspace", "rub out"),
    Binding::extra("Esc", "clear"),
    Binding::extra("↑ / ↓", "scroll the tape"),
    Binding::extra("x", "multiply"),
];

/// Numerals are never drawn larger than this.
///
/// One, for the reason the pomodoro's is one: a result occupying half a
/// dashboard reads as an alarm. It also keeps `max_width` small enough to be
/// worth declaring.
const MAX_SCALE: u16 = 1;

/// Entries kept on the tape.
///
/// A bound rather than a target — the tape shows what fits, and this only stops
/// a long session growing without limit. Nothing reads past what is drawn.
const MAX_TAPE: usize = 200;

/// Interior width past which the panel gains nothing.
///
/// An expression, a result and a tape line all fit comfortably; wider only buys
/// whitespace, and taking room the task list could use would be worse. See
/// invariant 15.
const USEFUL_WIDTH: u16 = 44;

/// Rows the panel needs besides the numerals: the expression line above, and
/// one tape row below.
const EXPRESSION_AND_TAPE: u16 = 2;

/// One finished calculation.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    expression: String,
    result: f64,
}

/// What is on the display right now.
#[derive(Debug, Clone, PartialEq)]
enum Shown {
    /// Nothing worked out yet this session.
    Nothing,
    /// The answer to the expression on the tape.
    Answer(f64),
    /// Why the last attempt did not work.
    Failed(CalcError),
}

pub struct CalculatorPanel {
    #[allow(dead_code)]
    config: CalculatorConfig,
    /// What is being typed.
    typing: String,
    shown: Shown,
    /// Finished calculations, newest first.
    tape: Vec<Entry>,
    scroll: ListState,
    /// Tape rows drawn last frame, so scrolling cannot leave what is on screen.
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
            .field("shown", &self.shown)
            .field("tape", &self.tape.len())
            .finish_non_exhaustive()
    }
}

impl CalculatorPanel {
    pub fn new(config: CalculatorConfig) -> Self {
        Self {
            config,
            typing: String::new(),
            shown: Shown::Nothing,
            tape: Vec::new(),
            scroll: ListState::default(),
            drawn: 0,
            action: None,
        }
    }

    /// The answer currently on display, if there is one.
    fn answer(&self) -> Option<f64> {
        match self.shown {
            Shown::Answer(value) => Some(value),
            _ => None,
        }
    }

    /// Add a character to the expression being typed.
    ///
    /// An operator typed straight after an answer continues from it — `57`,
    /// then `+10`, reads as `57 + 10`. A digit starts fresh instead. That is
    /// how a desk calculator chains, and it is what removes the need for a
    /// memory key: "and now add the tax" costs no extra concept.
    fn push(&mut self, c: char) {
        if self.typing.is_empty()
            && let Some(value) = self.answer()
            && matches!(c, '+' | '-' | '*' | '/' | 'x' | '\u{00D7}' | '\u{00F7}')
        {
            self.typing = calc::format_result(value);
        }
        // Refused rather than silently dropped: past this the expression cannot
        // be evaluated anyway, and a key that does nothing with no explanation
        // reads as a stuck terminal.
        if self.typing.chars().count() >= calc::MAX_LEN {
            self.shown = Shown::Failed(CalcError::TooLong);
            return;
        }
        self.typing.push(c);
    }

    fn evaluate(&mut self) {
        match calc::evaluate(&self.typing) {
            Ok(value) => {
                let expression = self.typing.trim().to_string();
                self.tape.insert(
                    0,
                    Entry {
                        expression,
                        result: value,
                    },
                );
                self.tape.truncate(MAX_TAPE);
                self.shown = Shown::Answer(value);
                self.typing.clear();
                // A new entry belongs at the top of the tape, and the cursor
                // with it; leaving it where it was would scroll the newest
                // result off the moment it arrived.
                self.scroll.select(None);
            }
            Err(error) => self.shown = Shown::Failed(error),
        }
    }

    /// The line above the numerals: what is being typed, or what went wrong.
    fn expression_line(&self, theme: &crate::theme::Theme) -> (String, Color) {
        if let Some(action) = &self.action {
            return (action.clone(), theme.label);
        }
        if !self.typing.is_empty() {
            return (self.typing.clone(), theme.text);
        }
        match &self.shown {
            // Never blank: a panel showing nothing at all reads as broken
            // rather than as idle (invariant 11).
            Shown::Nothing => ("type a sum".to_string(), theme.muted),
            Shown::Failed(error) => (error.message(), theme.error),
            Shown::Answer(_) => (
                self.tape
                    .first()
                    .map_or_else(String::new, |entry| entry.expression.clone()),
                theme.muted,
            ),
        }
    }

    /// What fills the display area, and whether it is a number.
    ///
    /// The distinction matters because only a number is drawn in block
    /// numerals. Two bugs came out of not making it, and both were found by
    /// running the thing rather than by any test:
    ///
    /// - `glyphs` has no glyph for `–`, and an unknown character renders as a
    ///   *blank cell* rather than as anything visible. So an idle calculator
    ///   drew five empty rows, which reads as a broken panel.
    /// - A failed sum kept its text on the line above so it could be corrected,
    ///   and the error message was written to that same line — where the text
    ///   took priority. The message was unreachable, so `1/0` looked like a key
    ///   that did nothing.
    ///
    /// Putting the reason in the display area fixes both at once, and it is the
    /// better arrangement anyway: the expression stays where it is being edited
    /// and the large empty space says what went wrong.
    fn display(&self, width: u16) -> (String, bool) {
        match &self.shown {
            Shown::Answer(value) => (calc::fit_result(*value, usize::from(width)), true),
            Shown::Failed(error) => (error.message(), false),
            // A dash rather than a zero. Zero is an answer, and nothing has
            // been asked yet — the same reason the markets panel shows `–`
            // rather than a price it has not got.
            Shown::Nothing => ("\u{2013}".to_string(), false),
        }
    }
}

impl Panel for CalculatorPanel {
    fn title(&self) -> String {
        "Calculator".to_string()
    }

    /// Deliberately `None`.
    ///
    /// A count of tape entries is a badge, and a badge that only goes up is the
    /// unread counter this dashboard turned down. Nothing here accumulates that
    /// you are expected to deal with.
    fn counter(&self) -> Option<String> {
        None
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
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
    ///
    /// The tick exists for panels with a data source behind them, and this one
    /// has none — it moves only when a key is pressed. Reporting `false`
    /// unconditionally is what keeps an idle dashboard from repainting for it.
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
            KeyCode::Char('=') | KeyCode::Enter => self.evaluate(),
            KeyCode::Backspace => {
                // With nothing typed, this clears the answer rather than doing
                // nothing — the display is the only thing left to rub out.
                if self.typing.pop().is_none() {
                    self.shown = Shown::Nothing;
                }
            }
            KeyCode::Esc => {
                // Esc backs out of something everywhere in this program. Here
                // there are two things to back out of, innermost first: what is
                // half-typed, then the answer. It is consumed only while there
                // is something to clear, so Esc on an empty calculator falls
                // through to the shell like Esc anywhere else.
                if !self.typing.is_empty() {
                    self.typing.clear();
                } else if self.shown != Shown::Nothing {
                    self.shown = Shown::Nothing;
                } else if !had_action {
                    return KeyOutcome::Ignored;
                }
            }
            KeyCode::Char('y') => {
                let Some(value) = self.answer() else {
                    return KeyOutcome::Consumed;
                };
                let text = calc::format_result(value);
                self.action = Some(match crate::clipboard::copy(&text) {
                    // OSC 52 is write-only: the terminal never answers, so this
                    // says what was sent rather than that anything was copied.
                    Ok(()) => format!("sent {text} to the clipboard"),
                    Err(e) => format!("clipboard failed: {e}"),
                });
            }
            KeyCode::Down | KeyCode::Char('j') => {
                crate::selection::down(&mut self.scroll, 1, self.drawn);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                crate::selection::up(&mut self.scroll, 1, self.drawn);
            }
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
            MouseEventKind::ScrollDown => crate::selection::down(&mut self.scroll, 1, self.drawn),
            MouseEventKind::ScrollUp => crate::selection::up(&mut self.scroll, 1, self.drawn),
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

        // What the display will say, decided before anything is laid out: the
        // numerals are sized to the text, so the text has to exist first.
        let (text, is_number) = self.display(area.width);
        let colour = match self.shown {
            Shown::Failed(_) => theme.error,
            _ => theme.accent,
        };

        // Only a number is drawn in block numerals. Anything else — a reason,
        // a resting dash — goes as plain text, because `glyphs` renders a
        // character it does not know as a blank cell rather than as a
        // substitute, so an error in numerals is an error nobody can read.
        //
        // The numerals give way before the expression line does. They are the
        // decoration; the line above says what is being worked out, and losing
        // that leaves a number with no question attached to it.
        let scale = is_number
            .then(|| {
                glyphs::fitting_scale(
                    &text,
                    area.width,
                    area.height.saturating_sub(EXPRESSION_AND_TAPE).max(1),
                    MAX_SCALE,
                )
            })
            .flatten();
        let numeral_rows = scale.map_or(1, |s| BigText::new(&text, s).height);

        let bottom = area.y + area.height;
        let mut cursor = area.y;

        // The expression, or the error, or what the last copy did.
        if cursor < bottom {
            let (line, line_colour) = self.expression_line(theme);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    crate::grid::truncate(&line, usize::from(area.width)),
                    Style::default().fg(line_colour),
                )),
                Rect::new(area.x, cursor, area.width, 1),
            );
            cursor += 1;
        }

        // The answer.
        if let Some(scale) = scale {
            let big = BigText::new(&text, scale);
            let x = area.x + area.width.saturating_sub(big.width) / 2;
            for (index, row) in big.rows.iter().enumerate() {
                let y = cursor + u16::try_from(index).unwrap_or(0);
                if y >= bottom {
                    break;
                }
                frame.render_widget(
                    Paragraph::new(Span::styled(row.clone(), Style::default().fg(colour))),
                    Rect::new(x, y, big.width.min(area.width), 1),
                );
            }
            cursor += numeral_rows;
        } else if cursor < bottom {
            // Everything the numerals cannot take: an answer too long for them,
            // a reason a sum failed, the resting dash. Wrapped rather than cut,
            // since a reason is prose and half a reason is no use — and wrapped
            // here rather than by ratatui, which panics on text it was not
            // given by this program (see `grid::wrapped`).
            //
            // Centred line by line so a one-line answer still sits under the
            // expression the way the numerals would.
            let rows = crate::grid::wrap(&text, usize::from(area.width));
            let style = Style::default().fg(colour).add_modifier(Modifier::BOLD);
            for row in rows {
                if cursor >= bottom {
                    break;
                }
                let width = u16::try_from(display_width(&row)).unwrap_or(area.width);
                let x = area.x + area.width.saturating_sub(width) / 2;
                frame.render_widget(
                    Paragraph::new(Span::styled(row, style)),
                    Rect::new(x, cursor, width.min(area.width), 1),
                );
                cursor += 1;
            }
        }

        self.draw_tape(frame, area, cursor, bottom, theme);
    }
}

impl CalculatorPanel {
    /// The tape, filling whatever is left below the display.
    ///
    /// Rows are built only for the space there is, never for the whole buffer:
    /// a panel may allocate in proportion to what is on screen, and must not
    /// allocate in proportion to how much data it holds.
    fn draw_tape(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        cursor: u16,
        bottom: u16,
        theme: &crate::theme::Theme,
    ) {
        let room = bottom.saturating_sub(cursor);
        self.drawn = usize::from(room).min(self.tape.len());
        if room == 0 || self.tape.is_empty() {
            if room > 0 {
                self.drawn = 0;
            }
            return;
        }

        let width = usize::from(area.width);
        let items: Vec<ListItem> = self
            .tape
            .iter()
            .take(self.drawn)
            .map(|entry| {
                // The rule that governs the display governs the tape too, and
                // it is easier to break here. Composing `expr = result` and
                // trimming the whole line cut the *result* — a tape row reading
                // ` = 123456789` for an answer of 123456789000, which is the
                // exact lie this panel exists not to tell. So the answer is
                // fitted first and never cut, and the expression takes what is
                // left over.
                const SEP: &str = " = ";
                let sep_width = display_width(SEP);
                let result = calc::fit_result(entry.result, width);
                let used = display_width(&result);

                let line = if used + sep_width < width {
                    let expression =
                        crate::grid::truncate(&entry.expression, width - used - sep_width);
                    if expression.is_empty() {
                        result
                    } else {
                        format!("{expression}{SEP}{result}")
                    }
                } else {
                    // No room for both. The answer is why the tape is here.
                    result
                };
                ListItem::new(Span::styled(line, Style::default().fg(theme.muted)))
            })
            .collect();

        frame.render_stateful_widget(
            List::new(items).highlight_style(Style::default().fg(theme.text)),
            Rect::new(area.x, cursor, area.width, room),
            &mut self.scroll,
        );
    }
}

/// Rows the frame costs, named so `max_height`'s reasoning is checkable.
#[allow(dead_code)]
const _: u16 = FRAME_HEIGHT;

// Exact comparison is the point: these are answers a calculator must get
// exactly right, not measurements to be compared within a tolerance. `2 + 3 * 4`
// is 14 or the panel is broken.
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

    fn draw(panel: &mut CalculatorPanel, width: u16, height: u16) -> String {
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
        let buffer = terminal.backend().buffer().clone();
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
    /// useless; get the second half wrong and it is a room with no door — no
    /// `Tab` to leave by, no `q` to quit with, no `?` for help.
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
        assert!(
            !panel.captures_input(),
            "capturing input would veto every global key, which is the trap this design avoids"
        );
    }

    #[test]
    fn typing_a_sum_and_pressing_enter_works_it_out() {
        let mut panel = new_panel();
        type_in(&mut panel, "2+3*4");
        panel.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(panel.answer(), Some(14.0));
        assert_eq!(panel.tape.len(), 1);
        assert_eq!(panel.tape[0].expression, "2+3*4");
        assert!(panel.typing.is_empty(), "the line clears for the next sum");
    }

    /// Chaining, which is what removes the need for a memory key.
    #[test]
    fn an_operator_after_an_answer_carries_it_forward() {
        let mut panel = new_panel();
        type_in(&mut panel, "50+7");
        panel.handle_key(KeyEvent::from(KeyCode::Enter));
        press(&mut panel, '+');
        assert_eq!(panel.typing, "57+");
        type_in(&mut panel, "10");
        panel.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(panel.answer(), Some(67.0));
    }

    #[test]
    fn a_digit_after_an_answer_starts_over() {
        let mut panel = new_panel();
        type_in(&mut panel, "50+7");
        panel.handle_key(KeyEvent::from(KeyCode::Enter));
        press(&mut panel, '9');
        assert_eq!(
            panel.typing, "9",
            "a digit begins a new sum, not a longer one"
        );
    }

    #[test]
    fn a_failed_sum_says_why_and_keeps_what_was_typed() {
        let mut panel = new_panel();
        type_in(&mut panel, "1/0");
        panel.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(
            panel.shown,
            Shown::Failed(CalcError::DivideByZero)
        ));
        assert_eq!(
            panel.typing, "1/0",
            "a rejected sum must stay on the line so it can be corrected"
        );
        assert!(
            panel.tape.is_empty(),
            "nothing that failed reaches the tape"
        );
    }

    #[test]
    fn esc_clears_what_is_typed_then_the_answer_then_gives_up_the_key() {
        let mut panel = new_panel();
        type_in(&mut panel, "12+3");
        panel.handle_key(KeyEvent::from(KeyCode::Enter));
        type_in(&mut panel, "99");

        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Esc)),
            KeyOutcome::Consumed
        );
        assert!(panel.typing.is_empty());

        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Esc)),
            KeyOutcome::Consumed
        );
        assert_eq!(panel.shown, Shown::Nothing);

        // With nothing left to clear, Esc means what it means everywhere else.
        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Esc)),
            KeyOutcome::Ignored
        );
    }

    #[test]
    fn the_tape_is_bounded() {
        let mut panel = new_panel();
        for n in 0..MAX_TAPE + 50 {
            panel.typing = format!("{n}+1");
            panel.evaluate();
        }
        assert_eq!(panel.tape.len(), MAX_TAPE);
        assert_eq!(
            panel.tape[0].expression,
            format!("{}+1", MAX_TAPE + 49),
            "newest first"
        );
    }

    #[test]
    fn an_over_long_expression_is_refused_rather_than_swallowed() {
        let mut panel = new_panel();
        for _ in 0..calc::MAX_LEN + 20 {
            press(&mut panel, '1');
        }
        assert_eq!(panel.typing.chars().count(), calc::MAX_LEN);
        assert!(
            matches!(panel.shown, Shown::Failed(CalcError::TooLong)),
            "a key that does nothing with no explanation reads as a stuck terminal"
        );
    }

    /// The rule this panel is held to more strictly than any other.
    ///
    /// Everywhere else in mirador a clipped value reads as a narrow terminal.
    /// Here it reads as a different answer, because a prefix of a number is a
    /// number. Checked on screen rather than on the formatter, since the screen
    /// is where the lie would appear.
    #[test]
    fn a_result_too_wide_for_the_panel_is_never_shown_truncated() {
        let mut panel = new_panel();
        panel.typing = "123456789 * 1000".to_string();
        panel.evaluate();
        assert_eq!(calc::format_result(panel.answer().unwrap()), "123456789000");

        // Narrow enough that twelve digits cannot fit, tall enough that the
        // tape is drawn as well as the display. Both are checked, because the
        // first version of this only looked at the display and the bug was in
        // the tape: composing `expr = result` and trimming the finished line
        // cut the answer down to ` = 123456789`, which is a real number and
        // the wrong one.
        for row in draw(&mut panel, 8, 6).lines() {
            let stripped: String = row.chars().filter(|c| !c.is_whitespace()).collect();
            assert!(
                !stripped.contains("123456789") || stripped.contains('e'),
                "a prefix of the answer reached the screen: {row:?}"
            );
        }

        // And the honest form did arrive.
        assert!(
            draw(&mut panel, 8, 6).contains('e'),
            "the answer should have become scientific"
        );
    }

    /// Every row a tape draws has to fit the panel, for the same reason.
    #[test]
    fn no_tape_row_is_ever_wider_than_the_panel() {
        let mut panel = new_panel();
        for expression in [
            "123456789 * 1000",
            "1/3",
            "2+2",
            "999999 * 999999 * 999999",
            "0.1+0.2",
        ] {
            panel.typing = expression.to_string();
            panel.evaluate();
        }
        for width in 1u16..=44 {
            for row in draw(&mut panel, width, 10).lines() {
                assert!(
                    crate::grid::display_width(row) <= usize::from(width),
                    "a {}-cell row was drawn into a {width}-cell panel: {row:?}",
                    crate::grid::display_width(row)
                );
            }
        }
    }

    #[test]
    fn the_panel_draws_at_any_size_without_panicking() {
        let mut panel = new_panel();
        type_in(&mut panel, "12345+6789");
        panel.handle_key(KeyEvent::from(KeyCode::Enter));
        for width in [1u16, 2, 3, 7, 12, 20, 44, 120] {
            for height in [1u16, 2, 3, 5, 9, 30] {
                let _ = draw(&mut panel, width, height);
            }
        }
    }

    /// Both bugs a real terminal found, and neither test existed before it did.
    ///
    /// The unit tests all passed while `1/0` looked like a key that did
    /// nothing: the reason was written to the line above, where the text being
    /// corrected already sat and took priority, so it was never drawn. And the
    /// resting `–` was handed to the block numerals, which render an unknown
    /// character as a *blank cell* — five empty rows that read as a panel that
    /// had crashed.
    ///
    /// Neither is visible from the state; both are obvious on screen. That is
    /// the argument for driving the thing.
    #[test]
    fn a_failed_sum_says_why_on_screen() {
        let mut panel = new_panel();
        type_in(&mut panel, "1/0");
        panel.handle_key(KeyEvent::from(KeyCode::Enter));

        let screen = draw(&mut panel, 30, 9);
        assert!(
            screen.contains("cannot divide by zero"),
            "the reason has to reach the screen, not just the state:\n{screen}"
        );
        assert!(
            screen.contains("1/0"),
            "and what was typed has to stay there to be corrected:\n{screen}"
        );
    }

    #[test]
    fn the_resting_display_is_visible_rather_than_blank() {
        let mut panel = new_panel();
        let screen = draw(&mut panel, 30, 9);
        assert!(
            screen.contains('\u{2013}'),
            "an idle calculator draws a dash, not five blank rows — `glyphs` has \
             no numeral for it and renders unknown characters as empty cells:\n{screen}"
        );
    }

    /// A panel with nothing in it must not look broken (invariant 11).
    #[test]
    fn an_untouched_calculator_says_what_to_do() {
        let mut panel = new_panel();
        let screen = draw(&mut panel, 30, 8);
        assert!(
            screen.contains("type a sum"),
            "an empty calculator has to invite the first keystroke:\n{screen}"
        );
    }

    /// The tape must cost what is on screen, not what is in the buffer.
    #[test]
    fn only_the_tape_rows_that_fit_are_built() {
        let mut panel = new_panel();
        for n in 0..MAX_TAPE {
            panel.typing = format!("{n}+1");
            panel.evaluate();
        }
        draw(&mut panel, 30, 9);
        assert!(
            panel.drawn <= 9,
            "{} rows were built for a nine-row panel",
            panel.drawn
        );
        assert!(panel.drawn > 0, "a tall panel should show some of the tape");
    }

    /// Scrolling is bounded by what was drawn, so a shorter panel cannot leave
    /// the cursor past the end — the defect `selection::up` had.
    #[test]
    fn the_cursor_cannot_leave_the_visible_tape() {
        let mut panel = new_panel();
        for n in 0..40 {
            panel.typing = format!("{n}+1");
            panel.evaluate();
        }
        draw(&mut panel, 30, 12);
        for _ in 0..100 {
            panel.handle_key(KeyEvent::from(KeyCode::Down));
        }
        assert!(panel.scroll.selected().is_some_and(|i| i < panel.drawn));
    }
}
