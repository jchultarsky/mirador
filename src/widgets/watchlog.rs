//! The watch log: what happened while you were not looking.
//!
//! The reasoning about *what belongs in it* is in [`crate::watch`]; this is
//! only how it is drawn. Three things about the drawing are load-bearing and
//! none of them is decoration:
//!
//! - **No counter in the frame.** Every other list panel here carries one —
//!   `4 open`, `3`, `2 today` — and this one deliberately does not. A number in
//!   the border is a badge, a badge accumulates, and an accumulating badge is
//!   precisely the unread-message count this dashboard turned down.
//! - **The rule line is a position, not a quantity.** It says "you have not
//!   been here since this point", which is a fact about the list, rather than
//!   "you have 4 unread", which is a demand.
//! - **The foot says when watching began.** The log cannot know what happened
//!   before mirador started, and a list that quietly begins mid-story implies
//!   otherwise.

use jiff::Zoned;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::frame::Binding;
use crate::panel::{KeyOutcome, Panel, RenderContext};

const BINDINGS: &[Binding] = &[
    Binding::extra("↑ / ↓", "scroll"),
    Binding::extra("j / k", "scroll"),
    Binding::extra("g / G", "first / last"),
];

/// Interior width past which the panel gains nothing: a time, a source and a
/// sentence with room to breathe.
const USEFUL_WIDTH: u16 = 56;

pub struct WatchLogPanel {
    scroll: ListState,
    /// Entries drawn last frame, so `tick` can answer honestly.
    drawn: usize,
    /// Whether a calendar is configured, so the empty panel can say that one
    /// of the two things it watches is not switched on.
    watching_calendar: bool,
}

impl WatchLogPanel {
    pub fn new(config: &crate::config::Config) -> Self {
        Self {
            scroll: ListState::default(),
            drawn: 0,
            watching_calendar: config.agenda.file.is_some(),
        }
    }
}

impl Panel for WatchLogPanel {
    fn title(&self) -> String {
        "Watch log".into()
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn max_width(&self) -> Option<u16> {
        Some(USEFUL_WIDTH + crate::frame::FRAME_WIDTH)
    }

    /// Deliberately `None`.
    ///
    /// Every other panel that returns a figure here does so because more rows
    /// buy it nothing. This one is the opposite: rows *are* the content, and a
    /// log tall enough to hold a day of events is the difference between
    /// scrolling and glancing.
    fn max_height(&self) -> Option<u16> {
        None
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        // The log is read, never edited: nothing here dismisses, acknowledges
        // or clears an entry. An entry you can dismiss is an entry you are
        // expected to dismiss, which is the obligation this panel exists
        // without.
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                crate::selection::down(&mut self.scroll, 1, self.drawn);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                crate::selection::up(&mut self.scroll, 1, self.drawn);
            }
            KeyCode::PageDown => crate::selection::down(&mut self.scroll, 10, self.drawn),
            KeyCode::PageUp => crate::selection::up(&mut self.scroll, 10, self.drawn),
            KeyCode::Char('G') | KeyCode::End => {
                crate::selection::down(&mut self.scroll, usize::MAX, self.drawn);
            }
            KeyCode::Char('g') | KeyCode::Home => {
                crate::selection::up(&mut self.scroll, usize::MAX, self.drawn);
            }
            _ => return KeyOutcome::Ignored,
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
        let theme = ctx.theme;
        if area.width == 0 || area.height == 0 {
            return;
        }

        let log = ctx.watch;
        let unseen = log.unseen();
        let mut items: Vec<ListItem> = Vec::new();

        for (index, entry) in log.entries().enumerate() {
            // Drawn *before* the entry it precedes, so it sits between the
            // newer entries above and the older ones below.
            if unseen == Some(index) {
                items.push(ListItem::new(rule_line(
                    area.width,
                    "since you were here",
                    theme,
                )));
            }
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{} ", entry.at.strftime("%H:%M")),
                    Style::default().fg(theme.muted),
                ),
                Span::styled(entry.text.clone(), Style::default().fg(theme.text)),
            ])));
        }

        if items.is_empty() {
            // An empty log has to say what it is watching, or it reads as
            // broken. It has no refresh key because nothing here is polled —
            // the panels report to it — and "Nothing has happened" alone gives
            // a reader no way to tell the difference between working and dead.
            // Somebody switched this on, waited, and reasonably concluded it
            // was the latter.
            let width = usize::from(area.width);
            let mut lines = vec![
                Line::from(Span::styled(
                    "Nothing has happened",
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!("since {}.", started(log.since())),
                    Style::default().fg(theme.muted),
                )),
                Line::from(""),
            ];

            // Wrapped rather than hand-broken. The first version assumed a
            // width the panel does not have and lost its last line off the
            // bottom, which is a poor way to explain something.
            let explain = |lines: &mut Vec<Line<'static>>, text: &str, colour| {
                for line in crate::grid::wrap(text, width) {
                    lines.push(Line::from(Span::styled(line, Style::default().fg(colour))));
                }
            };
            explain(
                &mut lines,
                "Watching for things you did not do yourself: the day turning, \
                 a task falling overdue, an entry appearing in your calendar.",
                theme.muted,
            );
            if !self.watching_calendar {
                lines.push(Line::from(""));
                // Milder than it was, and correctly so: the day always turns,
                // so the panel is no longer inert without a calendar — it is
                // merely watching one thing fewer.
                explain(
                    &mut lines,
                    "No calendar set, so that last one cannot happen. Press f \
                     on the agenda panel to add one.",
                    theme.muted,
                );
            }
            frame.render_widget(Paragraph::new(lines), area);
            self.drawn = 0;
            return;
        }

        // The foot, always last: the log begins where mirador did and says so
        // rather than looking like a complete history that happens to be short.
        items.push(ListItem::new(Line::from(Span::styled(
            format!("watching from {}", started(log.since())),
            Style::default().fg(theme.muted),
        ))));

        self.drawn = items.len();
        frame.render_stateful_widget(List::new(items), area, &mut self.scroll);
    }
}

/// `08:12`, or `Sat 08:12` once the log has been running past midnight.
fn started(since: &Zoned) -> String {
    if since.date() == Zoned::now().date() {
        since.strftime("%H:%M").to_string()
    } else {
        since.strftime("%a %H:%M").to_string()
    }
}

/// `──────── label ────`, filling the width.
fn rule_line(width: u16, label: &str, theme: &crate::theme::Theme) -> Line<'static> {
    let style = Style::default().fg(theme.rule);
    let label_width = crate::grid::display_width(label) + 2;
    let dashes = usize::from(width).saturating_sub(label_width);
    // Weighted towards the right so the label sits near the entries it
    // separates rather than floating in the middle of the panel.
    let left = dashes.saturating_sub(dashes / 3);
    Line::from(vec![
        Span::styled("─".repeat(left), style),
        Span::styled(format!(" {label} "), Style::default().fg(theme.muted)),
        Span::styled("─".repeat(dashes - left), style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_line_fills_its_width_exactly() {
        let theme = crate::theme::Theme::default();
        for width in 24..90u16 {
            let line = rule_line(width, "since you were here", &theme);
            let drawn: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(
                crate::grid::display_width(&drawn),
                usize::from(width),
                "at {width}"
            );
        }
    }

    /// The frame carries no counter, and that is a decision rather than an
    /// omission. Every other list panel here has one — `4 open`, `2 today` —
    /// so the obvious "improvement" is to give this one `3 new`. That number is
    /// a badge, a badge accumulates, and an accumulating badge is exactly the
    /// unread-message count this dashboard turned down. If this assertion ever
    /// fails, the question to ask is not how to fix the test.
    #[test]
    fn the_panel_never_offers_a_counter() {
        assert_eq!(
            WatchLogPanel::new(&crate::config::Config::default()).counter(),
            None
        );
    }

    /// The log is read, never edited. A key that dismissed an entry would make
    /// the log something you are expected to keep up with, which is the
    /// obligation this panel is designed to avoid.
    #[test]
    fn nothing_dismisses_acknowledges_or_clears_an_entry() {
        let mut panel = WatchLogPanel::new(&crate::config::Config::default());
        panel.drawn = 5;
        for code in [
            KeyCode::Char('d'),
            KeyCode::Char('c'),
            KeyCode::Char('x'),
            KeyCode::Delete,
            KeyCode::Backspace,
            KeyCode::Enter,
            KeyCode::Char(' '),
        ] {
            assert_eq!(
                panel.handle_key(KeyEvent::from(code)),
                KeyOutcome::Ignored,
                "{code:?} must not be a way to act on an entry"
            );
        }
    }
}
