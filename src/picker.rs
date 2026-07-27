//! The panel picker: every widget mirador has, and whether your layout places
//! it.
//!
//! Switching a widget on used to mean finding your config file and editing
//! `[layout]` by hand, which is what the first person to meet the pomodoro
//! panel actually had to do.
//!
//! The picker owns its cursor and its drawing and nothing else. Toggling a
//! widget means rewriting the layout, rebuilding every panel and possibly
//! putting it all back when that fails — decisions that belong to the shell.
//! So `handle_key` returns an [`Action`] describing what was asked for, and the
//! shell decides whether it can be done. Handing the picker a `&mut App` would
//! have moved the code without moving the responsibility.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph};

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::theme::Theme;

/// What a keypress asked the shell to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing the shell needs to act on; the cursor may have moved.
    None,
    /// Turn this widget on if it is off, and off if it is on.
    Toggle(&'static str),
    /// Close the dialog and commit whatever changed.
    Close,
}

/// An open picker.
#[derive(Debug)]
pub struct Picker {
    selected: usize,
}

impl Picker {
    /// Open on the first widget.
    pub fn new() -> Self {
        Self { selected: 0 }
    }

    /// Which row the cursor is on. Exposed for tests.
    #[cfg(test)]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Move the cursor, or report what the shell has to do.
    ///
    /// Unlike the help overlay this is a real dialog, so it reads keys rather
    /// than dismissing on any of them — a stray keystroke must not close it and
    /// leave the user wondering whether the toggle took.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        let names = crate::widgets::WIDGET_NAMES;
        let last = names.len().saturating_sub(1);

        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'w') | KeyCode::Enter => Action::Close,
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = self.selected.saturating_add(1).min(last);
                Action::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                Action::None
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = 0;
                Action::None
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = last;
                Action::None
            }
            KeyCode::Char(' ') => names
                .get(self.selected)
                .copied()
                .map_or(Action::None, Action::Toggle),
            _ => Action::None,
        }
    }

    /// Draw the dialog.
    ///
    /// `placed` answers "is this widget in the layout" rather than the picker
    /// holding a copy of the layout, which would go stale the moment a toggle
    /// was refused.
    pub fn render(
        &self,
        frame: &mut ratatui::Frame,
        area: Rect,
        theme: &Theme,
        placed: impl Fn(&str) -> bool,
        error: Option<&str>,
    ) {
        let names = crate::widgets::WIDGET_NAMES;

        let mut lines: Vec<Line> = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let on = placed(name);
            let here = index == self.selected;
            // A filled mark and an empty one in the track colour, the same
            // vocabulary the meters use, so "on" is legible without relying on
            // the word beside it.
            let mark = if on { "■" } else { "□" };
            let mark_style = Style::default().fg(if on { theme.accent } else { theme.track });
            let name_style = if here {
                Style::default()
                    .fg(theme.text)
                    .add_modifier(Modifier::REVERSED)
            } else if on {
                Style::default().fg(theme.text)
            } else {
                Style::default().fg(theme.muted)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if here { " ▸ " } else { "   " },
                    Style::default().fg(theme.accent),
                ),
                Span::styled(format!("{mark} "), mark_style),
                Span::styled(format!("{name:<10}"), name_style),
            ]));
        }

        lines.push(Line::from(""));
        match error {
            Some(error) => lines.push(Line::from(Span::styled(
                format!("  {error}"),
                Style::default().fg(theme.error),
            ))),
            None => lines.push(Line::from(Span::styled(
                "  written to your config on close",
                Style::default().fg(theme.muted),
            ))),
        }
        lines.push(Line::from(vec![
            Span::styled(
                "  space",
                Style::default().fg(theme.key).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" toggle   ", Style::default().fg(theme.muted)),
            Span::styled(
                "esc",
                Style::default().fg(theme.key).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" close", Style::default().fg(theme.muted)),
        ]));

        let height = u16::try_from(lines.len())
            .unwrap_or(u16::MAX)
            .saturating_add(crate::frame::FRAME_HEIGHT);
        let popup = crate::frame::centred(area, 40, height);

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_focused))
                    .padding(Padding::horizontal(1))
                    .title_top(Line::from(Span::styled(
                        "PANELS",
                        Style::default()
                            .fg(theme.title)
                            .add_modifier(Modifier::BOLD),
                    ))),
            ),
            popup,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(picker: &mut Picker, code: KeyCode) -> Action {
        picker.handle_key(KeyEvent::from(code))
    }

    #[test]
    fn the_cursor_clamps_at_both_ends() {
        let last = crate::widgets::WIDGET_NAMES.len() - 1;
        let mut picker = Picker::new();

        assert_eq!(picker.selected(), 0, "opens on the first widget");
        press(&mut picker, KeyCode::Up);
        assert_eq!(picker.selected(), 0, "the top does not wrap");

        press(&mut picker, KeyCode::End);
        assert_eq!(picker.selected(), last);
        press(&mut picker, KeyCode::Down);
        assert_eq!(picker.selected(), last, "the end does not wrap");
    }

    #[test]
    fn space_names_the_widget_under_the_cursor() {
        let mut picker = Picker::new();
        assert_eq!(
            press(&mut picker, KeyCode::Char(' ')),
            Action::Toggle(crate::widgets::WIDGET_NAMES[0])
        );

        press(&mut picker, KeyCode::Down);
        assert_eq!(
            press(&mut picker, KeyCode::Char(' ')),
            Action::Toggle(crate::widgets::WIDGET_NAMES[1])
        );
    }

    #[test]
    fn only_the_documented_keys_close_it() {
        for code in [
            KeyCode::Esc,
            KeyCode::Char('q'),
            KeyCode::Char('w'),
            KeyCode::Enter,
        ] {
            assert_eq!(press(&mut Picker::new(), code), Action::Close, "{code:?}");
        }
        // A dialog that closed on any key would leave the user unsure whether
        // their last toggle was taken.
        for code in [KeyCode::Char('x'), KeyCode::Tab, KeyCode::Backspace] {
            assert_eq!(press(&mut Picker::new(), code), Action::None, "{code:?}");
        }
    }
}
