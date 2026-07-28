//! The `t` dialog: every theme mirador can find, previewed as you move.
//!
//! Same shape as [`crate::picker`] — it owns its cursor and its drawing and
//! nothing else, and reports what it wants via an [`Action`] so the shell keeps
//! the decisions. What is different is that moving the cursor is itself an
//! action here. A theme you cannot see is a theme you cannot choose, and the
//! alternative — pick blind, close, look, reopen — is how you end up cycling
//! through nine themes to find the one you already had.
//!
//! That makes `Esc` load-bearing rather than decorative: it puts back whatever
//! was in force when the dialog opened, so browsing costs nothing. `Enter`
//! keeps what is under the cursor.
//!
//! The list is read from disk **once, when the dialog opens**. A directory
//! listing on every frame would be the panels' no-blocking rule broken by the
//! shell instead of by a widget, and the set of themes does not change while
//! you are looking at it.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph};

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::path::Path;

use crate::theme::Theme;

/// What a keypress asked the shell to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing the shell needs to act on.
    None,
    /// Show this theme now. Sent on every cursor move, not only on Enter.
    Preview(String),
    /// Close, keeping whatever is currently previewed.
    Accept,
    /// Close, putting back the theme that was in force when this opened.
    Cancel,
}

/// How many rows of the list are on screen at once.
///
/// Nine bundled themes plus however many the user wrote is already more than
/// fits comfortably in a dialog, so this scrolls rather than growing.
const ROWS: usize = 12;

/// An open theme picker.
#[derive(Debug)]
pub struct ThemePicker {
    names: Vec<String>,
    /// How many of `names` came from the bundled set, so the rest can be
    /// labelled as the user's own.
    bundled: usize,
    selected: usize,
    offset: usize,
}

impl ThemePicker {
    /// Open on `current`, listing the bundled themes and any in `themes_dir`.
    ///
    /// A user theme that shares a bundled name appears once: `themes::resolve`
    /// searches the directory first, so the file on disk is the one that would
    /// load, and showing both would offer a choice that does not exist.
    pub fn new(current: Option<&str>, themes_dir: Option<&Path>) -> Self {
        let mut names: Vec<String> = crate::themes::bundled_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        names.sort_unstable();
        let bundled = names.len();

        let mut mine = themes_dir.map(user_themes).unwrap_or_default();
        mine.retain(|name| !names.contains(name));
        mine.sort_unstable();
        names.extend(mine);

        // Landing on the theme in force is the whole reason the dialog knows
        // what it is. A config with an inline `[theme]` table has no name, so
        // there is nothing to land on and the top of the list is as good as
        // anywhere.
        let selected = current
            .and_then(|name| names.iter().position(|listed| listed == name))
            .unwrap_or(0);

        let mut picker = Self {
            names,
            bundled,
            selected,
            offset: 0,
        };
        picker.scroll_into_view();
        picker
    }

    /// The themes available, for tests.
    #[cfg(test)]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The name under the cursor, if the list is not empty.
    pub fn current(&self) -> Option<&str> {
        self.names.get(self.selected).map(String::as_str)
    }

    /// Which row the cursor is on. Exposed for tests.
    #[cfg(test)]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Move the cursor, or report what the shell has to do.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        let last = self.names.len().saturating_sub(1);
        let moved = match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => return Action::Accept,
            // `t` closes the dialog the same way it opened it, and `q` is the
            // habit everything else in mirador has taught. Both cancel rather
            // than accept, because both are reflexes rather than decisions —
            // the one key that keeps a theme is the one you have to mean.
            KeyCode::Esc | KeyCode::Char('q' | 't') => return Action::Cancel,
            KeyCode::Down | KeyCode::Char('j') => self.selected.saturating_add(1).min(last),
            KeyCode::Up | KeyCode::Char('k') => self.selected.saturating_sub(1),
            KeyCode::Home => 0,
            KeyCode::End => last,
            KeyCode::PageDown => self.selected.saturating_add(ROWS).min(last),
            KeyCode::PageUp => self.selected.saturating_sub(ROWS),
            _ => return Action::None,
        };

        if moved == self.selected {
            return Action::None;
        }
        self.selected = moved;
        self.scroll_into_view();
        self.current()
            .map_or(Action::None, |n| Action::Preview(n.to_string()))
    }

    /// Keep the cursor inside the visible window.
    fn scroll_into_view(&mut self) {
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + ROWS {
            self.offset = self.selected + 1 - ROWS;
        }
    }

    /// Draw the dialog centred over whatever is behind it.
    pub fn render(&self, frame: &mut ratatui::Frame, area: Rect, theme: &Theme) {
        let width = 44u16.min(area.width);
        let rows = u16::try_from(self.names.len().min(ROWS)).unwrap_or(u16::MAX);
        // List, the two frame rows, the blank row and the footer. Nothing more:
        // a spare row inside the border reads as a list that has run out rather
        // than as breathing space.
        let height = rows.saturating_add(4).min(area.height);
        let rect = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };

        let mut lines: Vec<Line<'static>> = Vec::with_capacity(self.names.len() + 2);
        for (index, name) in self.names.iter().enumerate().skip(self.offset).take(ROWS) {
            let chosen = index == self.selected;
            let marker = if chosen { "▸ " } else { "  " };
            let style = if chosen {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            let mut spans = vec![
                Span::styled(marker, Style::default().fg(theme.accent)),
                Span::styled(name.clone(), style),
            ];
            if index >= self.bundled {
                spans.push(Span::styled("  yours", Style::default().fg(theme.muted)));
            }
            lines.push(Line::from(spans));
        }

        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled(
                crate::glyphs::utility("enter"),
                Style::default().fg(theme.key).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" keep  ", Style::default().fg(theme.muted)),
            Span::styled(
                crate::glyphs::utility("esc"),
                Style::default().fg(theme.key).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" put back", Style::default().fg(theme.muted)),
        ]));

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_focused))
            .padding(Padding::horizontal(1))
            .title(Line::from(vec![
                Span::styled("┤", Style::default().fg(theme.border_focused)),
                Span::styled(
                    crate::glyphs::utility("theme"),
                    Style::default()
                        .fg(theme.title)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("├", Style::default().fg(theme.border_focused)),
            ]));

        frame.render_widget(Clear, rect);
        frame.render_widget(Paragraph::new(lines).block(block), rect);
    }
}

/// Theme names in `dir`, from `*.toml` files.
///
/// Anything the resolver would refuse is left out rather than listed and then
/// rejected on selection: a name with a slash or a space in it cannot be loaded
/// by name at all, so offering it would be offering a dead end.
fn user_themes(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != "toml" {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            crate::themes::is_listable(&name).then_some(name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    /// A directory of its own per test. Sharing one is how the zone tests came
    /// to read each other's files on Windows.
    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn themes_dir(name: &str) -> TempDir {
        let dir =
            std::env::temp_dir().join(format!("mirador-picker-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test directory");
        TempDir(dir)
    }

    fn write(dir: &Path, name: &str) {
        std::fs::write(dir.join(format!("{name}.toml")), "accent = \"red\"\n").expect("write");
    }

    #[test]
    fn it_opens_on_the_theme_already_in_force() {
        let picker = ThemePicker::new(Some("nord"), None);
        assert_eq!(picker.current(), Some("nord"));
    }

    #[test]
    fn an_inline_theme_table_has_no_name_to_land_on_and_starts_at_the_top() {
        let picker = ThemePicker::new(None, None);
        assert_eq!(picker.selected(), 0);
    }

    #[test]
    fn moving_the_cursor_previews_rather_than_waiting_for_enter() {
        let mut picker = ThemePicker::new(Some("ansi"), None);
        let first = picker.current().expect("a name").to_string();
        let action = picker.handle_key(key(KeyCode::Down));
        let Action::Preview(name) = action else {
            panic!("a cursor move must preview, got {action:?}");
        };
        assert_ne!(name, first, "the preview is the row moved to");
        assert_eq!(picker.current(), Some(name.as_str()));
    }

    #[test]
    fn the_cursor_stops_at_both_ends_without_previewing_again() {
        let mut picker = ThemePicker::new(None, None);
        assert_eq!(picker.handle_key(key(KeyCode::Up)), Action::None);
        picker.handle_key(key(KeyCode::End));
        assert_eq!(picker.handle_key(key(KeyCode::Down)), Action::None);
    }

    #[test]
    fn enter_keeps_and_esc_puts_back() {
        let mut picker = ThemePicker::new(None, None);
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), Action::Accept);
        assert_eq!(picker.handle_key(key(KeyCode::Esc)), Action::Cancel);
        assert_eq!(picker.handle_key(key(KeyCode::Char('t'))), Action::Cancel);
        assert_eq!(picker.handle_key(key(KeyCode::Char('q'))), Action::Cancel);
    }

    /// The bug the clock's zone picker shipped with: the cursor walked past the
    /// bottom of the window and the list stopped following it.
    #[test]
    fn the_window_follows_the_cursor_past_the_bottom_of_the_list() {
        let dir = themes_dir("scroll");
        for i in 0..20 {
            write(&dir.0, &format!("mine-{i:02}"));
        }
        let mut picker = ThemePicker::new(None, Some(&dir.0));
        assert!(picker.names().len() > ROWS, "needs more rows than fit");

        for _ in 0..picker.names().len() {
            picker.handle_key(key(KeyCode::Down));
            assert!(
                picker.selected() >= picker.offset && picker.selected() < picker.offset + ROWS,
                "cursor {} left the window at offset {}",
                picker.selected(),
                picker.offset
            );
        }
    }

    #[test]
    fn a_user_theme_shadowing_a_bundled_name_is_listed_once() {
        let dir = themes_dir("shadow");
        write(&dir.0, "nord");
        let picker = ThemePicker::new(None, Some(&dir.0));
        let nords = picker.names().iter().filter(|n| *n == "nord").count();
        assert_eq!(nords, 1, "the file on disk is the one that would load");
    }

    /// A name the resolver would refuse must not be offered, or selecting it is
    /// a dead end the user cannot diagnose.
    #[test]
    fn a_file_the_resolver_could_never_load_is_not_offered() {
        let dir = themes_dir("unlistable");
        write(&dir.0, "has space");
        write(&dir.0, "fine-one");
        let picker = ThemePicker::new(None, Some(&dir.0));
        assert!(picker.names().iter().any(|n| n == "fine-one"));
        assert!(
            !picker.names().iter().any(|n| n.contains(' ')),
            "got: {:?}",
            picker.names()
        );
    }

    #[test]
    fn a_missing_themes_directory_is_not_an_error() {
        let picker = ThemePicker::new(None, Some(Path::new("/nope/not/here")));
        assert_eq!(picker.names().len(), crate::themes::bundled_names().len());
    }

    #[test]
    fn it_draws_without_panicking_in_a_small_terminal() {
        for (w, h) in [(80, 24), (30, 10), (10, 5)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
            let picker = ThemePicker::new(None, None);
            terminal
                .draw(|f| picker.render(f, f.area(), &Theme::default()))
                .expect("draw");
        }
    }
}
