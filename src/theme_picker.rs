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

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::Path;

use crate::keymap::{KeysConfig, Meta, PanelKeymap};
use crate::theme::Theme;

/// What the theme picker's keys do, under `[theme_picker.keys]`. Esc is not
/// here: it always puts the theme back, as Esc backs out of everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePickerAction {
    Up,
    Down,
    First,
    Last,
    PageUp,
    PageDown,
    Keep,
    PutBack,
}

const NONE: KeyModifiers = KeyModifiers::NONE;

/// The theme picker's actions and their default keys.
///
/// `t` closes the dialog the same way it opened it, and `q` is the habit
/// everything else in mirador has taught. Both put the theme back rather than
/// keep it, because both are reflexes rather than decisions — the keys that
/// keep a theme are the ones you have to mean.
pub const ACTIONS: &[Meta<ThemePickerAction>] = &[
    Meta {
        action: ThemePickerAction::Up,
        name: "up",
        defaults: &[(KeyCode::Up, NONE), (KeyCode::Char('k'), NONE)],
        label: "move",
        primary: false,
        joins: false,
        about: "preview the theme above",
    },
    Meta {
        action: ThemePickerAction::Down,
        name: "down",
        defaults: &[(KeyCode::Down, NONE), (KeyCode::Char('j'), NONE)],
        label: "move",
        primary: false,
        joins: true,
        about: "preview the theme below",
    },
    Meta {
        action: ThemePickerAction::First,
        name: "first",
        defaults: &[(KeyCode::Home, NONE)],
        label: "first",
        primary: false,
        joins: false,
        about: "preview the first theme",
    },
    Meta {
        action: ThemePickerAction::Last,
        name: "last",
        defaults: &[(KeyCode::End, NONE)],
        label: "last",
        primary: false,
        joins: true,
        about: "preview the last theme",
    },
    Meta {
        action: ThemePickerAction::PageUp,
        name: "page_up",
        defaults: &[(KeyCode::PageUp, NONE)],
        label: "page",
        primary: false,
        joins: false,
        about: "preview the theme a page up",
    },
    Meta {
        action: ThemePickerAction::PageDown,
        name: "page_down",
        defaults: &[(KeyCode::PageDown, NONE)],
        label: "page",
        primary: false,
        joins: true,
        about: "preview the theme a page down",
    },
    Meta {
        action: ThemePickerAction::Keep,
        name: "keep",
        defaults: &[(KeyCode::Enter, NONE), (KeyCode::Char(' '), NONE)],
        label: "keep",
        primary: true,
        joins: false,
        about: "keep the theme under the cursor",
    },
    Meta {
        action: ThemePickerAction::PutBack,
        name: "put_back",
        defaults: &[(KeyCode::Char('q'), NONE), (KeyCode::Char('t'), NONE)],
        label: "put back",
        primary: true,
        joins: false,
        about: "close and put back the theme you had",
    },
];

/// `[theme_picker.keys]` laid over [`ACTIONS`], or why it cannot be.
pub fn keymap(keys: &KeysConfig) -> Result<PanelKeymap<ThemePickerAction>, String> {
    PanelKeymap::new("theme_picker", ACTIONS, keys)
}

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
    keys: PanelKeymap<ThemePickerAction>,
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
            keys: PanelKeymap::defaults("theme_picker", ACTIONS),
        };
        picker.scroll_into_view();
        picker
    }

    /// The same picker reading `keys` — `[theme_picker.keys]` as the shell
    /// last loaded it.
    #[must_use]
    pub fn with_keys(mut self, keys: PanelKeymap<ThemePickerAction>) -> Self {
        self.keys = keys;
        self
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
        // Esc always puts the theme back, and is in no table.
        if key.code == KeyCode::Esc {
            return Action::Cancel;
        }
        let Some(action) = self.keys.action(key) else {
            return Action::None;
        };
        let moved = match action {
            ThemePickerAction::Keep => return Action::Accept,
            ThemePickerAction::PutBack => return Action::Cancel,
            ThemePickerAction::Down => self.selected.saturating_add(1).min(last),
            ThemePickerAction::Up => self.selected.saturating_sub(1),
            ThemePickerAction::First => 0,
            ThemePickerAction::Last => last,
            ThemePickerAction::PageDown => self.selected.saturating_add(ROWS).min(last),
            ThemePickerAction::PageUp => self.selected.saturating_sub(ROWS),
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

    /// The rows to draw: the visible slice of the list, a blank, and the footer.
    ///
    /// Split out of `render` so a test can weigh it. The bug it exists to pin
    /// was invisible on screen — `render` reserved `self.names.len() + 2` lines
    /// and drew at most `ROWS`, so with five thousand themes on disk it
    /// allocated room for 5,012 every frame and pushed 14. Nothing looked
    /// wrong, which is exactly why a test that only checks what reaches the
    /// screen cannot catch it.
    fn lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::with_capacity(ROWS + 2);
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
        // The keep key is the one `[theme_picker.keys]` gave it; Esc always
        // puts the theme back, whatever else does.
        let key_style = Style::default().fg(theme.key).add_modifier(Modifier::BOLD);
        let mut footer = Vec::new();
        if let Some(keep) = self.keys.keys(ThemePickerAction::Keep).first() {
            footer.push(Span::styled(
                crate::glyphs::utility(&keep.to_string()),
                key_style,
            ));
            footer.push(Span::styled(" keep  ", Style::default().fg(theme.muted)));
        }
        footer.push(Span::styled(crate::glyphs::utility("esc"), key_style));
        footer.push(Span::styled(" put back", Style::default().fg(theme.muted)));
        lines.push(Line::from(footer));
        lines
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

        let lines = self.lines(theme);

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

    /// Every size down to one cell. `prompt` had a panic that only appeared at
    /// width 1, found by sweeping rather than by reasoning, so this sweeps —
    /// and it holds with a list long enough to scroll, which is when the
    /// arithmetic has something to get wrong.
    /// The golden test for the `t` picker: the default map answers every key
    /// the old `match` answered.
    #[test]
    fn the_default_theme_picker_map_is_the_keys_it_always_had() {
        let map = keymap(&KeysConfig::default()).expect("valid");
        for (code, action) in [
            (KeyCode::Enter, ThemePickerAction::Keep),
            (KeyCode::Char(' '), ThemePickerAction::Keep),
            (KeyCode::Char('q'), ThemePickerAction::PutBack),
            (KeyCode::Char('t'), ThemePickerAction::PutBack),
            (KeyCode::Down, ThemePickerAction::Down),
            (KeyCode::Char('j'), ThemePickerAction::Down),
            (KeyCode::Up, ThemePickerAction::Up),
            (KeyCode::Char('k'), ThemePickerAction::Up),
            (KeyCode::Home, ThemePickerAction::First),
            (KeyCode::End, ThemePickerAction::Last),
            (KeyCode::PageDown, ThemePickerAction::PageDown),
            (KeyCode::PageUp, ThemePickerAction::PageUp),
        ] {
            assert_eq!(map.action(key(code)), Some(action), "{code:?}");
        }
    }

    /// A moved key previews and keeps as the old one did, the footer names
    /// it, and Esc puts the theme back whatever the table says.
    #[test]
    fn a_moved_theme_picker_key_works_and_esc_still_puts_back() {
        let keys =
            keymap(&toml::from_str("down = \"n\"\nkeep = \"y\"\nput_back = []").expect("a table"))
                .expect("valid");
        let mut picker = ThemePicker::new(None, None).with_keys(keys);
        assert_eq!(picker.handle_key(key(KeyCode::Char('j'))), Action::None);
        assert!(matches!(
            picker.handle_key(key(KeyCode::Char('n'))),
            Action::Preview(_)
        ));
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), Action::None);
        assert_eq!(picker.handle_key(key(KeyCode::Char('y'))), Action::Accept);
        assert_eq!(picker.handle_key(key(KeyCode::Char('t'))), Action::None);
        assert_eq!(picker.handle_key(key(KeyCode::Esc)), Action::Cancel);

        let footer: String = picker
            .lines(&Theme::default())
            .last()
            .expect("a footer")
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(footer, "Y keep  ESC put back");
    }

    #[test]
    fn it_draws_without_panicking_at_every_size_down_to_one_cell() {
        let dir = themes_dir("tiny");
        for i in 0..30 {
            write(&dir.0, &format!("mine-{i:02}"));
        }

        for (w, h) in [(80u16, 24u16), (30, 10), (10, 5), (2, 2), (1, 1)] {
            for themes in [None, Some(&dir.0)] {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
                let mut picker = ThemePicker::new(None, themes.map(std::path::PathBuf::as_path));
                // Draw at the top, part-way down, and at the end, so the
                // scrolled window is exercised as well as the initial one.
                for _ in 0..3 {
                    terminal
                        .draw(|f| picker.render(f, f.area(), &Theme::default()))
                        .unwrap_or_else(|e| panic!("{w}x{h} failed to draw: {e}"));
                    for _ in 0..15 {
                        picker.handle_key(key(KeyCode::Down));
                    }
                }
            }
        }
    }

    /// A dialog may allocate in proportion to what it draws. It must not
    /// allocate in proportion to how many themes happen to be on disk.
    ///
    /// Weighs the buffer rather than the screen, deliberately. `take(ROWS)`
    /// caps what is *drawn* whichever way the reservation is written, so a test
    /// that counted rows on screen would pass with the bug in place — the trap
    /// this repository keeps walking into. What was wrong was the size of the
    /// allocation, so that is what this measures.
    #[test]
    fn drawing_reserves_room_for_the_window_not_for_the_whole_list() {
        let dir = themes_dir("many");
        for i in 0..400 {
            write(&dir.0, &format!("mine-{i:03}"));
        }
        let mut picker = ThemePicker::new(None, Some(&dir.0));
        assert!(picker.names().len() > 400, "there is a big list behind it");

        // At the top, part-way down, and at the end.
        for _ in 0..4 {
            let lines = picker.lines(&Theme::default());
            assert!(
                lines.len() <= ROWS + 2,
                "{} lines built for a {ROWS}-row window",
                lines.len()
            );
            assert!(
                lines.capacity() <= ROWS + 2,
                "reserved room for {} lines to draw {}",
                lines.capacity(),
                lines.len()
            );
            for _ in 0..150 {
                picker.handle_key(key(KeyCode::Down));
            }
        }
    }
}
