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

use std::cell::Cell;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::frame::Binding;
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
        // The footer offers Esc for this instead, under this label: Esc
        // closes too, and is the one key no table can take away.
        primary: false,
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

/// `keys` with Esc's hint beside them. Esc closes as `close` does and is in
/// no table, so its hint carries `close`'s label.
fn with_esc(keys: PanelKeymap<PickerAction>) -> PanelKeymap<PickerAction> {
    let close = ACTIONS
        .iter()
        .find(|meta| meta.action == PickerAction::Close)
        .expect("close is in the table");
    keys.with_fixed(&[Binding::owned(crate::frame::ESC, close.label, true)])
}

/// A move of a dialog's list cursor.
///
/// The panel picker, the theme picker and the prompt's list each own a
/// cursor, and each wrote the clamps out by hand — three copies of one piece
/// of arithmetic, which is how one of them came to have no window at all.
/// This is the one copy, with [`window`] beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Up(usize),
    Down(usize),
    First,
    Last,
}

impl Step {
    /// Where a cursor at `selected` lands in a list of `len`: clamped at both
    /// ends, never wrapping. A cursor left past the end of a list that has
    /// since got shorter is brought back before it moves, whichever way —
    /// `selection::up` once walked one down a row at a time with nothing
    /// highlighted the whole way.
    pub(crate) fn apply(self, selected: usize, len: usize) -> usize {
        let last = len.saturating_sub(1);
        let selected = selected.min(last);
        match self {
            Self::Up(rows) => selected.saturating_sub(rows),
            Self::Down(rows) => selected.saturating_add(rows).min(last),
            Self::First => 0,
            Self::Last => last,
        }
    }
}

/// An open picker.
#[derive(Debug)]
pub struct Picker {
    selected: usize,
    /// The first name drawn. Only `render` knows how many rows the terminal
    /// leaves the list, so it is `render` that moves the window — and keeps
    /// where it put it, so the window moves when the cursor leaves it rather
    /// than following the cursor row by row.
    offset: Cell<usize>,
    names: Vec<String>,
    keys: PanelKeymap<PickerAction>,
}

impl Picker {
    /// Open on the first widget, with the default keys.
    pub fn new(names: Vec<String>) -> Self {
        Self {
            selected: 0,
            offset: Cell::new(0),
            names,
            keys: with_esc(PanelKeymap::defaults("panel_picker", ACTIONS)),
        }
    }

    /// The same picker reading `keys` — `[panel_picker.keys]` as the shell
    /// last loaded it.
    #[must_use]
    pub fn with_keys(mut self, keys: PanelKeymap<PickerAction>) -> Self {
        self.keys = with_esc(keys);
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
        if key.code == KeyCode::Esc {
            return Action::Close;
        }
        let Some(action) = self.keys.action(key) else {
            return Action::None;
        };
        let step = match action {
            PickerAction::Close => return Action::Close,
            PickerAction::Toggle => {
                return self
                    .names
                    .get(self.selected)
                    .cloned()
                    .map_or(Action::None, Action::Toggle);
            }
            PickerAction::Down => Step::Down(1),
            PickerAction::Up => Step::Up(1),
            PickerAction::First => Step::First,
            PickerAction::Last => Step::Last,
        };
        self.selected = step.apply(self.selected, self.names.len());
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
        // The list, then a blank, the status line and the footer. Every name is
        // drawn when there is room for it; where there is not, the list
        // scrolls rather than the rows under it being cut, and keeps at least
        // the row the cursor is on. Below that it is the blank that goes,
        // being spacing, then the status, so that the footer with the way out
        // gives way to nothing but the row under the cursor.
        let height = u16::try_from(self.names.len().saturating_add(TRAILER))
            .unwrap_or(u16::MAX)
            .saturating_add(crate::frame::FRAME_HEIGHT);
        let popup = crate::frame::centred(area, 40, height);
        let interior = usize::from(popup.height.saturating_sub(crate::frame::FRAME_HEIGHT));
        let (rows, spaced) = list_rows(interior, TRAILER);
        let offset = window(self.selected, self.offset.get(), rows, self.names.len());
        self.offset.set(offset);

        let mut lines: Vec<Line> = Vec::with_capacity(rows + TRAILER);
        for (index, name) in self.names.iter().enumerate().skip(offset).take(rows) {
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

        if spaced {
            lines.push(Line::from(""));
        }
        // The status and the footer sit under the names, two columns in, and
        // are fitted to the dialog as drawn: `centred` clamps it to the
        // screen, and the terminal cut both there with nothing to say so
        // (invariant 19). The status is prose, as often as not an error from
        // writing the config, and is ellipsised; the footer's hints drop
        // whole, Esc last. Either gives up the indent, being padding, before
        // a letter of what it says.
        let inner = usize::from(popup.width.saturating_sub(crate::frame::FRAME_WIDTH));
        let indent = |width: usize| if width + 2 <= inner { "  " } else { "" };
        let (status, colour) = match error {
            Some(error) => (error, theme.error),
            None => ("written to your config on close", theme.muted),
        };
        let status = crate::grid::truncate(status, inner);
        // Pushed only with room for the footer under it. Without that check
        // the status took the footer's row and the way out was the line the
        // dialog cut, with nothing to mark it, in favour of a constant string
        // about where a change goes.
        if interior >= rows + 2 {
            lines.push(Line::from(vec![
                Span::raw(indent(crate::grid::display_width(&status))),
                Span::styled(status, Style::default().fg(colour)),
            ]));
        }
        // The primary keys as `[panel_picker.keys]` has them — the toggle
        // key, then Esc, which always closes, whatever else does.
        let footer = crate::frame::key_row(
            self.keys.bindings(),
            "   ",
            theme,
            u16::try_from(inner).unwrap_or(u16::MAX),
        );
        lines.push(Line::from(
            std::iter::once(Span::raw(indent(footer.width())))
                .chain(footer.spans)
                .collect::<Vec<_>>(),
        ));

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines).block(crate::frame::dialog_block(theme, "panels", popup.width)),
            popup,
        );
    }
}

/// The rows drawn under the list: a blank, the status line and the footer.
const TRAILER: usize = 3;

/// How a dialog's `interior` rows are shared between its list and the
/// `trailer` rows drawn under it, the first of which is a blank: the rows
/// the list can have, and whether the blank still fits.
///
/// The list gets every row the trailer can spare and never fewer than the
/// one the cursor is on. Below that it is the blank that goes, being
/// spacing, before anything there is to read.
pub(crate) fn list_rows(interior: usize, trailer: usize) -> (usize, bool) {
    let rows = interior.saturating_sub(trailer).max(1);
    (rows, interior >= rows + trailer)
}

/// The first row of a `rows`-high window over `len` names that keeps
/// `selected` in view, moving the window from `offset` as little as it can.
///
/// Shared by every dialog that owns a list cursor. The theme picker and the
/// prompt each kept a copy that moved the window in `handle_key` against a
/// fixed row count, which is what let the theme picker preview a theme its
/// terminal had no row to draw.
pub(crate) fn window(selected: usize, offset: usize, rows: usize, len: usize) -> usize {
    let offset = if selected < offset {
        selected
    } else if selected >= offset.saturating_add(rows) {
        selected.saturating_add(1).saturating_sub(rows)
    } else {
        offset
    };
    // A terminal taller than last time shows the rows above the window
    // rather than a gap below it.
    offset.min(len.saturating_sub(rows))
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
        assert!(calm.contains("space toggle   Esc close"), "{calm}");

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

    /// The bug the theme picker and the zone prompt were fixed for, in the one
    /// dialog that never was. On a terminal shorter than the list the bottom
    /// rows were cut, footer first, while the cursor walked on into them —
    /// `End` then `space` switched `calculator` with nothing on screen saying
    /// so. Swept over every height, with the cursor taken to the end and back
    /// one row at a time, because the window moving down and the window moving
    /// up are different arithmetic.
    #[test]
    fn the_row_under_the_cursor_is_drawn_at_every_height() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::default();
        let names = crate::widgets::WIDGET_NAMES;
        let draw = |picker: &Picker, height: u16| -> String {
            let mut terminal = Terminal::new(TestBackend::new(60, height)).unwrap();
            terminal
                .draw(|frame| picker.render(frame, frame.area(), &theme, |_| false, None))
                .unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..height)
                .map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };

        for height in 3..=30u16 {
            let mut picker = picker();
            let visit = |picker: &Picker| {
                let screen = draw(picker, height);
                let name = names[picker.selected()];
                assert!(
                    screen.contains(&format!("▸ □ {name}")),
                    "{name} is under the cursor at height {height} and not drawn:\n{screen}"
                );
                // The frame, one row of list and the footer make four rows,
                // and at four or more the way out is drawn: the list gives
                // way first, then the blank above the status, which is
                // spacing, then the status, which says where a change goes
                // and is no use to somebody looking for the way out.
                if height >= 4 {
                    assert!(
                        screen.contains("Esc close"),
                        "the footer went before the status at height {height}:\n{screen}"
                    );
                }
                if height >= 5 {
                    assert!(
                        screen.contains("written to your config on close"),
                        "the status went before the blank at height {height}:\n{screen}"
                    );
                }
            };
            press(&mut picker, KeyCode::End);
            visit(&picker);
            for _ in 0..names.len() {
                press(&mut picker, KeyCode::Up);
                visit(&picker);
            }
        }

        // The window moves when the cursor leaves it, not with every key:
        // one row up from the end, the end is still on screen.
        let mut short = picker();
        press(&mut short, KeyCode::End);
        let _ = draw(&short, 12);
        press(&mut short, KeyCode::Up);
        let screen = draw(&short, 12);
        let last = names[names.len() - 1];
        assert!(
            screen.contains(&format!("  □ {last}")),
            "the window followed the cursor up:\n{screen}"
        );

        // With room for everything nothing scrolls: the whole list is drawn,
        // as it always was.
        let tall = draw(&picker(), 30);
        for name in names {
            assert!(tall.contains(&format!("□ {name}")), "{name}:\n{tall}");
        }

        // A terminal that grows after the list has scrolled shows the rows
        // above the window, rather than keeping the window where it was with
        // a gap under it.
        let grown = {
            let mut picker = picker();
            press(&mut picker, KeyCode::End);
            let _ = draw(&picker, 12);
            draw(&picker, 30)
        };
        assert!(
            grown.contains(&format!("□ {}", names[0])),
            "the window stayed scrolled on a taller terminal:\n{grown}"
        );
    }

    /// The one copy of the dialogs' cursor arithmetic: clamped at both ends,
    /// never wrapping, a page that would pass an end stopping at it, and a
    /// cursor left past the end of a list that has since got shorter brought
    /// back before it moves, whichever way.
    #[test]
    fn a_step_stays_inside_the_list() {
        assert_eq!(Step::Up(1).apply(0, 5), 0);
        assert_eq!(Step::Down(1).apply(4, 5), 4);
        assert_eq!(Step::Down(1).apply(2, 5), 3);
        assert_eq!(Step::Down(12).apply(2, 5), 4);
        assert_eq!(Step::Up(12).apply(3, 5), 0);
        assert_eq!(Step::First.apply(3, 5), 0);
        assert_eq!(Step::Last.apply(0, 5), 4);
        assert_eq!(Step::Down(1).apply(0, 0), 0);
        assert_eq!(Step::Last.apply(0, 0), 0);
        // A list of nine cut to five under a cursor on row eight.
        assert_eq!(Step::Up(1).apply(8, 5), 3);
        assert_eq!(Step::Down(1).apply(8, 5), 4);
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
