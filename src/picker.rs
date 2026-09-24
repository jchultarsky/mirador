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

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keymap::{KeysConfig, Meta, PanelKeymap};
use crate::theme::Theme;

/// What the picker's keys do, under `[panel_picker.keys]`. Esc is not here:
/// it always closes the dialog, as Esc backs out of everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerAction {
    Up,
    Down,
    First,
    Last,
    Toggle,
    Close,
}

const NONE: KeyModifiers = KeyModifiers::NONE;

/// The picker's actions and their default keys. `w` closes it the way it
/// opened, and `q` is the habit everything else in mirador teaches.
pub const ACTIONS: &[Meta<PickerAction>] = &[
    Meta {
        action: PickerAction::Up,
        name: "up",
        defaults: &[(KeyCode::Up, NONE), (KeyCode::Char('k'), NONE)],
        label: "move",
        primary: false,
        joins: false,
        about: "move to the panel above",
    },
    Meta {
        action: PickerAction::Down,
        name: "down",
        defaults: &[(KeyCode::Down, NONE), (KeyCode::Char('j'), NONE)],
        label: "move",
        primary: false,
        joins: true,
        about: "move to the panel below",
    },
    Meta {
        action: PickerAction::First,
        name: "first",
        defaults: &[(KeyCode::Home, NONE), (KeyCode::Char('g'), NONE)],
        label: "first",
        primary: false,
        joins: false,
        about: "move to the first panel",
    },
    Meta {
        action: PickerAction::Last,
        name: "last",
        defaults: &[(KeyCode::End, NONE), (KeyCode::Char('G'), NONE)],
        label: "last",
        primary: false,
        joins: true,
        about: "move to the last panel",
    },
    Meta {
        action: PickerAction::Toggle,
        name: "toggle",
        defaults: &[(KeyCode::Char(' '), NONE)],
        label: "toggle",
        primary: true,
        joins: false,
        about: "switch the panel under the cursor on or off",
    },
    Meta {
        action: PickerAction::Close,
        name: "close",
        defaults: &[
            (KeyCode::Enter, NONE),
            (KeyCode::Char('q'), NONE),
            (KeyCode::Char('w'), NONE),
        ],
        label: "close",
        primary: true,
        joins: false,
        about: "close, writing any change to the config",
    },
];

/// `[panel_picker.keys]` laid over [`ACTIONS`], or why it cannot be.
pub fn keymap(keys: &KeysConfig) -> Result<PanelKeymap<PickerAction>, String> {
    PanelKeymap::new("panel_picker", ACTIONS, keys)
}

/// What a keypress asked the shell to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing the shell needs to act on; the cursor may have moved.
    None,
    /// Turn this widget on if it is off, and off if it is on.
    Toggle(String),
    /// Close the dialog and commit whatever changed.
    Close,
}

/// An open picker.
#[derive(Debug)]
pub struct Picker {
    selected: usize,
    names: Vec<String>,
    keys: PanelKeymap<PickerAction>,
}

impl Picker {
    /// Open on the first widget, with the default keys.
    pub fn new(names: Vec<String>) -> Self {
        Self {
            selected: 0,
            names,
            keys: PanelKeymap::defaults("panel_picker", ACTIONS),
        }
    }

    /// The same picker reading `keys` — `[panel_picker.keys]` as the shell
    /// last loaded it.
    #[must_use]
    pub fn with_keys(mut self, keys: PanelKeymap<PickerAction>) -> Self {
        self.keys = keys;
        self
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
        let last = self.names.len().saturating_sub(1);

        if key.code == KeyCode::Esc {
            return Action::Close;
        }
        let Some(action) = self.keys.action(key) else {
            return Action::None;
        };
        match action {
            PickerAction::Close => return Action::Close,
            PickerAction::Toggle => {
                return self
                    .names
                    .get(self.selected)
                    .cloned()
                    .map_or(Action::None, Action::Toggle);
            }
            PickerAction::Down => self.selected = self.selected.saturating_add(1).min(last),
            PickerAction::Up => self.selected = self.selected.saturating_sub(1),
            PickerAction::First => self.selected = 0,
            PickerAction::Last => self.selected = last,
        }
        Action::None
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
        let mut lines: Vec<Line> = Vec::new();
        for (index, name) in self.names.iter().enumerate() {
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
        // The toggle key is the one `[panel_picker.keys]` gave it; Esc always
        // closes, whatever else does.
        let key_style = Style::default().fg(theme.key).add_modifier(Modifier::BOLD);
        let mut footer = Vec::new();
        if let Some(toggle) = self.keys.keys(PickerAction::Toggle).first() {
            footer.push(Span::styled(format!("  {toggle}"), key_style));
            footer.push(Span::styled(" toggle   ", Style::default().fg(theme.muted)));
        } else {
            footer.push(Span::raw("  "));
        }
        footer.push(Span::styled("esc", key_style));
        footer.push(Span::styled(" close", Style::default().fg(theme.muted)));
        lines.push(Line::from(footer));

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

    fn picker() -> Picker {
        Picker::new(
            crate::widgets::WIDGET_NAMES
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        )
    }

    /// The golden test for the `w` picker: the default map answers every key
    /// the old `match` answered.
    #[test]
    fn the_default_picker_map_is_the_keys_it_always_had() {
        let map = keymap(&KeysConfig::default()).expect("valid");
        for (code, action) in [
            (KeyCode::Up, PickerAction::Up),
            (KeyCode::Char('k'), PickerAction::Up),
            (KeyCode::Down, PickerAction::Down),
            (KeyCode::Char('j'), PickerAction::Down),
            (KeyCode::Home, PickerAction::First),
            (KeyCode::Char('g'), PickerAction::First),
            (KeyCode::End, PickerAction::Last),
            (KeyCode::Char('G'), PickerAction::Last),
            (KeyCode::Char(' '), PickerAction::Toggle),
            (KeyCode::Enter, PickerAction::Close),
            (KeyCode::Char('q'), PickerAction::Close),
            (KeyCode::Char('w'), PickerAction::Close),
        ] {
            assert_eq!(map.action(KeyEvent::from(code)), Some(action), "{code:?}");
        }
    }

    /// A moved toggle key toggles, the old one does nothing, the footer says
    /// the new one — and Esc closes whatever the table says.
    #[test]
    fn a_moved_picker_key_works_and_esc_still_closes() {
        let keys =
            keymap(&toml::from_str("toggle = \"x\"\nclose = []").expect("a table")).expect("valid");
        let mut picker = picker().with_keys(keys);
        assert_eq!(press(&mut picker, KeyCode::Char(' ')), Action::None);
        assert!(matches!(
            press(&mut picker, KeyCode::Char('x')),
            Action::Toggle(_)
        ));
        assert_eq!(
            press(&mut picker, KeyCode::Char('q')),
            Action::None,
            "q was unbound"
        );
        assert_eq!(
            press(&mut picker, KeyCode::Esc),
            Action::Close,
            "Esc always closes"
        );

        let theme = Theme::default();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 40)).unwrap();
        terminal
            .draw(|frame| picker.render(frame, frame.area(), &theme, |_| false, None))
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(screen.contains("x toggle"), "the footer follows the key");
        assert!(!screen.contains("space toggle"));
    }

    /// The dialog as drawn. `render` was never executed by a test, and it is
    /// the whole of what `w` shows: a mark per widget saying whether it is on,
    /// the cursor, and a line saying where the change goes — or, in place of
    /// that line, why it did not.
    #[test]
    fn the_picker_shows_a_mark_per_widget_and_says_where_the_change_goes() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::default();
        let draw = |picker: &Picker, error: Option<&str>, w: u16, h: u16| -> String {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|frame| picker.render(frame, frame.area(), &theme, |n| n == "clocks", error))
                .unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..h)
                .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let picker = Picker::new(vec!["clocks".into(), "weather".into(), "cpu".into()]);

        let calm = draw(&picker, None, 80, 24);
        assert!(
            calm.contains("▸ ■ clocks"),
            "the placed widget is marked and under the cursor:\n{calm}"
        );
        assert!(
            calm.contains("  □ weather"),
            "an unplaced one is hollow:\n{calm}"
        );
        assert!(calm.contains("written to your config on close"), "{calm}");
        assert!(
            calm.contains("space") && calm.contains("toggle") && calm.contains("esc"),
            "{calm}"
        );

        let failing = draw(&picker, Some("no `[layout]` rows found"), 80, 24);
        assert!(
            failing.contains("no `[layout]` rows found"),
            "the error takes the line:\n{failing}"
        );
        assert!(
            !failing.contains("written to your config"),
            "and the promise is withdrawn:\n{failing}"
        );

        // Too small for the dialog: drawn as far as it can be, never a panic.
        let _ = draw(&picker, None, 12, 4);
        let _ = draw(&picker, None, 1, 1);
    }

    #[test]
    fn the_cursor_clamps_at_both_ends() {
        let last = crate::widgets::WIDGET_NAMES.len() - 1;
        let mut picker = picker();

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
        let mut picker = picker();
        assert_eq!(
            press(&mut picker, KeyCode::Char(' ')),
            Action::Toggle(crate::widgets::WIDGET_NAMES[0].to_string())
        );

        press(&mut picker, KeyCode::Down);
        assert_eq!(
            press(&mut picker, KeyCode::Char(' ')),
            Action::Toggle(crate::widgets::WIDGET_NAMES[1].to_string())
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
            assert_eq!(press(&mut picker(), code), Action::Close, "{code:?}");
        }
        // A dialog that closed on any key would leave the user unsure whether
        // their last toggle was taken.
        for code in [KeyCode::Char('x'), KeyCode::Tab, KeyCode::Backspace] {
            assert_eq!(press(&mut picker(), code), Action::None, "{code:?}");
        }
    }

    #[test]
    fn runtime_plugin_names_are_selectable() {
        let mut picker = Picker::new(vec!["clocks".into(), "terminal".into()]);
        press(&mut picker, KeyCode::End);
        assert_eq!(
            press(&mut picker, KeyCode::Char(' ')),
            Action::Toggle("terminal".into())
        );
    }
}
