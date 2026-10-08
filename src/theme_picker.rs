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
use ratatui::widgets::{Clear, Paragraph};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::cell::Cell;
use std::path::Path;

use crate::frame::Binding;
use crate::keymap::{KeysConfig, Meta, PanelKeymap};
use crate::picker::{Step, list_rows, window};
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
        // The footer offers Esc for this instead, under this label: Esc puts
        // the theme back too, and is the one key no table can take away.
        primary: false,
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

/// The most rows of the list on screen at once.
///
/// Nineteen bundled themes plus however many the user wrote is already more
/// than fits comfortably in a dialog, so this scrolls rather than growing. A
/// terminal too short for it gets fewer, decided where the dialog is drawn.
const ROWS: usize = 12;

/// The rows drawn under the list: a blank and the footer.
const TRAILER: usize = 2;

/// `keys` with Esc's hint beside them. Esc puts the theme back as `put_back`
/// does and is in no table, so its hint carries `put_back`'s label.
fn with_esc(keys: PanelKeymap<ThemePickerAction>) -> PanelKeymap<ThemePickerAction> {
    let put_back = ACTIONS
        .iter()
        .find(|meta| meta.action == ThemePickerAction::PutBack)
        .expect("put_back is in the table");
    keys.with_fixed(&[Binding::owned(crate::frame::ESC, put_back.label, true)])
}

/// An open theme picker.
#[derive(Debug)]
pub struct ThemePicker {
    names: Vec<String>,
    /// How many of `names` came from the bundled set, so the rest can be
    /// labelled as the user's own.
    bundled: usize,
    selected: usize,
    /// The first name drawn. Only drawing knows how many rows the terminal
    /// leaves the list, so it is drawing that moves the window, as the panel
    /// picker's does — moving it in `handle_key` against [`ROWS`] previewed
    /// themes the terminal had no row to draw.
    offset: Cell<usize>,
    /// How many names were drawn last time, which is how far a page moves.
    page: Cell<usize>,
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

        Self {
            names,
            bundled,
            selected,
            offset: Cell::new(0),
            page: Cell::new(ROWS),
            keys: with_esc(PanelKeymap::defaults("theme_picker", ACTIONS)),
        }
    }

    /// The same picker reading `keys` — `[theme_picker.keys]` as the shell
    /// last loaded it.
    #[must_use]
    pub fn with_keys(mut self, keys: PanelKeymap<ThemePickerAction>) -> Self {
        self.keys = with_esc(keys);
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
        // Esc always puts the theme back, and is in no table.
        if key.code == KeyCode::Esc {
            return Action::Cancel;
        }
        let Some(action) = self.keys.action(key) else {
            return Action::None;
        };
        let page = self.page.get().max(1);
        let step = match action {
            ThemePickerAction::Keep => return Action::Accept,
            ThemePickerAction::PutBack => return Action::Cancel,
            ThemePickerAction::Down => Step::Down(1),
            ThemePickerAction::Up => Step::Up(1),
            ThemePickerAction::First => Step::First,
            ThemePickerAction::Last => Step::Last,
            ThemePickerAction::PageDown => Step::Down(page),
            ThemePickerAction::PageUp => Step::Up(page),
        };

        let moved = step.apply(self.selected, self.names.len());
        if moved == self.selected {
            return Action::None;
        }
        self.selected = moved;
        self.current()
            .map_or(Action::None, |n| Action::Preview(n.to_string()))
    }

    /// The rows to draw inside a dialog `interior` rows tall and `width`
    /// columns wide: the window of the list that keeps the cursor in view, a
    /// blank, and the footer. The list gives way first, then the blank —
    /// never the row the cursor is on.
    ///
    /// Split out of `render` so a test can weigh it. The bug it exists to pin
    /// was invisible on screen — `render` reserved `self.names.len() + 2` lines
    /// and drew at most `ROWS`, so with five thousand themes on disk it
    /// allocated room for 5,012 every frame and pushed 14. Nothing looked
    /// wrong, which is exactly why a test that only checks what reaches the
    /// screen cannot catch it.
    fn lines(&self, theme: &Theme, interior: usize, width: u16) -> Vec<Line<'static>> {
        let (rows, spaced) = list_rows(interior, TRAILER);
        let rows = rows.min(ROWS);
        let offset = window(self.selected, self.offset.get(), rows, self.names.len());
        self.offset.set(offset);
        self.page.set(rows);

        let mut lines: Vec<Line<'static>> = Vec::with_capacity(rows + TRAILER);
        for (index, name) in self.names.iter().enumerate().skip(offset).take(rows) {
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

        if spaced {
            lines.push(Line::default());
        }
        // The primary keys as `[theme_picker.keys]` has them — the keep key,
        // then Esc, which always puts the theme back, whatever else does.
        // Both are spelled as `Key` spells them and never re-cased: `y` and
        // `Y` are different keys, and the label face once turned a
        // `keep = "y"` into a footer naming the one that does nothing.
        lines.push(crate::frame::key_row(
            self.keys.bindings(),
            "  ",
            theme,
            width,
        ));
        lines
    }

    /// Draw the dialog centred over whatever is behind it.
    pub fn render(&self, frame: &mut ratatui::Frame, area: Rect, theme: &Theme) {
        // The list, the blank row and the footer inside the frame. Nothing
        // more: a spare row inside the border reads as a list that has run
        // out rather than as breathing space.
        let rows = self.names.len().min(ROWS);
        let height = u16::try_from(rows + TRAILER)
            .unwrap_or(u16::MAX)
            .saturating_add(crate::frame::FRAME_HEIGHT);
        let rect = crate::frame::centred(area, 44, height);
        let interior = usize::from(rect.height.saturating_sub(crate::frame::FRAME_HEIGHT));

        let lines = self.lines(
            theme,
            interior,
            rect.width.saturating_sub(crate::frame::FRAME_WIDTH),
        );
        let block = crate::frame::dialog_block(theme, "theme", rect.width);

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

    /// The theme picker's window was twelve rows, kept by `handle_key`, in a
    /// dialog sixteen rows tall. On a terminal shorter than that the rows
    /// under the window's foot were cut, footer first, and the cursor walked
    /// on into them: `End` previewed a theme whose name was nowhere on
    /// screen. Swept over every height, with the cursor taken to the end and
    /// back one row at a time, because the window moving down and the window
    /// moving up are different arithmetic.
    #[test]
    fn the_theme_under_the_cursor_is_drawn_at_every_height() {
        let dir = themes_dir("heights");
        for i in 0..10 {
            write(&dir.0, &format!("mine-{i:02}"));
        }
        let draw = |picker: &ThemePicker, height: u16| -> String {
            let mut terminal = Terminal::new(TestBackend::new(60, height)).expect("terminal");
            terminal
                .draw(|f| picker.render(f, f.area(), &Theme::default()))
                .expect("draw");
            let buffer = terminal.backend().buffer().clone();
            (0..height)
                .map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };

        for height in 3..=30u16 {
            let mut picker = ThemePicker::new(None, Some(&dir.0));
            let count = picker.names().len();
            assert!(count > ROWS, "needs more themes than the window holds");
            let visit = |picker: &ThemePicker| {
                let screen = draw(picker, height);
                let name = picker.current().expect("a name");
                assert!(
                    screen.contains(&format!("▸ {name}")),
                    "{name} is previewed at height {height} and not drawn:\n{screen}"
                );
                // The frame, one row of list and the footer make four rows,
                // and at four or more the footer is drawn: the list gives way
                // first, then the blank above the footer.
                if height >= 4 {
                    assert!(
                        screen.contains("Esc put back"),
                        "the footer went before the blank at height {height}:\n{screen}"
                    );
                }
            };
            picker.handle_key(key(KeyCode::End));
            visit(&picker);
            for _ in 0..count {
                picker.handle_key(key(KeyCode::Up));
                visit(&picker);
            }
        }

        // The window moves when the cursor leaves it, not with every key, and
        // then by no more than it must. At height 10 the list has six rows.
        // One row up from the end, the end is still on screen; and one row
        // past the window's top, the window has moved by exactly one row —
        // the cursor on the first row and the old top drawn under it. A
        // window snapped to pages keeps the end on screen too, which is why
        // the second half is here: it puts the cursor fifth of six.
        let mut short = ThemePicker::new(None, Some(&dir.0));
        let names = short.names().to_vec();
        let last = names.len() - 1;
        short.handle_key(key(KeyCode::End));
        let _ = draw(&short, 10);
        short.handle_key(key(KeyCode::Up));
        let screen = draw(&short, 10);
        assert!(
            screen.contains(&format!("  {}", names[last])),
            "the window followed the cursor up:\n{screen}"
        );
        for _ in 0..5 {
            short.handle_key(key(KeyCode::Up));
            let _ = draw(&short, 10);
        }
        let screen = draw(&short, 10);
        let rows: Vec<&str> = screen.lines().collect();
        let here = rows
            .iter()
            .position(|row| row.contains(&format!("▸ {}", names[last - 6])))
            .unwrap_or_else(|| panic!("the cursor is drawn:\n{screen}"));
        assert!(
            rows[here - 1].contains('╭')
                && rows[here + 1].contains(&format!("  {}", names[last - 5])),
            "the window moved by more than the one row the cursor left it by:\n{screen}"
        );
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
            .lines(&Theme::default(), ROWS + TRAILER, 40)
            .last()
            .expect("a footer")
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        // As `Key` spells it, never re-cased: `y` and `Y` are different keys,
        // and the footer used to name the one that does nothing.
        assert_eq!(footer, "y keep  Esc put back");
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
            let lines = picker.lines(&Theme::default(), ROWS + TRAILER, 40);
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
