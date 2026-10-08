//! The key map: every key the shell reads and every panel key a config can
//! move, what each does, where it came from, and how to change it — reached
//! by pressing the help key a second time.
//!
//! Same shape as [`crate::theme_picker`]: it owns its scroll position and its
//! drawing, and reports what it wants as a [`Request`] so the shell keeps the
//! decisions. Reading the config and rewriting it are the shell's to do,
//! because the shell is what holds the path and the live keymap.
//!
//! Two requests make this more than a table. **Reload** reads the key tables
//! again — `[keys]`, each mode's and each panel's `[<widget>.keys]` —
//! so a reader can edit the config in another pane and try the result without
//! restarting — and a mistake is shown here, with the keys they had still in
//! force, rather than at the next launch as a dashboard that will not start.
//! **Defaults** comments out their key lines, after asking, for the
//! reader who has lost track of what they changed. The same reset is
//! `mirador --reset-keys` for the case this dialog cannot reach: a keymap so
//! broken that mirador refuses to start.
//!
//! The dialog's own keys are fixed. A key map that could lose the key that
//! closes the key map is the trap invariant 2 exists to rule out. They are
//! declared all the same, in [`ACTIONS`], so the keys it reads and the keys
//! its footer offers come from one table, as the pickers' do — and never
//! from the config.

use std::path::Path;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::frame::Binding;
use crate::grid::{Column, Grid};
use crate::keymap::{Action, Key, Keymap, Listed, Meta, PanelKeymap};
use crate::theme::Theme;

/// What the dialog's own keys do. Esc is not here: it always closes, as Esc
/// backs out of everything, and the reset question reads its own answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogAction {
    Reload,
    Defaults,
    Close,
    Down,
    Up,
    PageDown,
    PageUp,
    First,
    Last,
}

const NONE: KeyModifiers = KeyModifiers::NONE;

/// The dialog's actions and their keys, which are also the only keys they
/// have: this table is laid over nothing. `q` closes it as it closes both
/// pickers.
const ACTIONS: &[Meta<DialogAction>] = &[
    Meta {
        action: DialogAction::Reload,
        name: "reload",
        defaults: &[(KeyCode::Char('r'), NONE)],
        label: "reload",
        primary: true,
        joins: false,
        about: "read every key table from the config again",
    },
    Meta {
        action: DialogAction::Defaults,
        name: "defaults",
        defaults: &[(KeyCode::Char('d'), NONE)],
        label: "defaults",
        primary: true,
        joins: false,
        about: "put every key back to its default, after asking",
    },
    Meta {
        action: DialogAction::Close,
        name: "close",
        defaults: &[(KeyCode::Char('q'), NONE)],
        label: "close",
        // The footer offers Esc for this instead, under this label: Esc
        // closes too, and is the one key every dialog keeps.
        primary: false,
        joins: false,
        about: "close the key map",
    },
    Meta {
        action: DialogAction::Down,
        name: "down",
        defaults: &[(KeyCode::Down, NONE), (KeyCode::Char('j'), NONE)],
        label: "scroll",
        primary: false,
        joins: false,
        about: "scroll down a line",
    },
    Meta {
        action: DialogAction::Up,
        name: "up",
        defaults: &[(KeyCode::Up, NONE), (KeyCode::Char('k'), NONE)],
        label: "scroll",
        primary: false,
        joins: true,
        about: "scroll up a line",
    },
    Meta {
        action: DialogAction::PageDown,
        name: "page_down",
        defaults: &[(KeyCode::PageDown, NONE)],
        label: "page",
        primary: false,
        joins: false,
        about: "scroll down a page",
    },
    Meta {
        action: DialogAction::PageUp,
        name: "page_up",
        defaults: &[(KeyCode::PageUp, NONE)],
        label: "page",
        primary: false,
        joins: true,
        about: "scroll up a page",
    },
    Meta {
        action: DialogAction::First,
        name: "first",
        defaults: &[(KeyCode::Home, NONE)],
        label: "first",
        primary: false,
        joins: false,
        about: "scroll to the top",
    },
    Meta {
        action: DialogAction::Last,
        name: "last",
        defaults: &[(KeyCode::End, NONE)],
        label: "last",
        primary: false,
        joins: true,
        about: "scroll to the foot",
    },
];

/// The dialog's keys, with Esc's hint beside them under `close`'s label.
/// Built from [`ACTIONS`] alone: no `[key_map.keys]` exists to read.
fn keys() -> PanelKeymap<DialogAction> {
    let close = ACTIONS
        .iter()
        .find(|meta| meta.action == DialogAction::Close)
        .expect("close is in the table");
    PanelKeymap::defaults("key_map", ACTIONS).with_fixed(&[Binding::owned(
        crate::frame::ESC,
        close.label,
        true,
    )])
}

/// What a keypress asked the shell to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Nothing the shell needs to act on.
    None,
    /// Close the dialog.
    Close,
    /// Read every key table from the config again — `[keys]`, each mode's and
    /// each panel's `[<widget>.keys]` — and use them if they all check out.
    Reload,
    /// Comment out every key table in the config and go back to the defaults.
    /// Only sent after the reader has confirmed it.
    Reset,
}

/// The key table. The action column is sized to its longest name,
/// `priority_previous`, since that is the word a reader copies into a key
/// table — `every_listed_action_name_fits_its_column_whole` holds every
/// listed name to it; the explanation goes first when the dialog is squeezed,
/// then the defaults.
pub const COLUMNS: &[Column] = &[
    Column::fixed("action", 17),
    Column::flex("keys", 2),
    Column::flex("default", 2).drops_below(44),
    Column::flex("does", 5).drops_below(64),
];

/// The widest the dialog draws: wide enough that every explanation arrives
/// whole in its column, which at 96 cells inside the frame is 44.
/// `every_explanation_arrives_whole_at_the_dialogs_widest` holds the two
/// together, so a longer explanation is shortened rather than cut. A
/// narrower terminal still gets a narrower dialog, and the explanations an
/// `…` where they no longer fit.
const WIDTH: u16 = 100;

/// The key table: a header, the shell's keys, then each panel's under the
/// heading of the table its keys are written in, since that heading is the
/// one thing the reader has to copy.
fn table(
    keymap: &Keymap,
    panels: &[(&'static str, Vec<Listed>)],
    theme: &Theme,
    width: u16,
) -> Vec<Line<'static>> {
    let body = Style::default().fg(theme.text);
    let muted = Style::default().fg(theme.muted);
    let changed = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let heading = Style::default()
        .fg(theme.label)
        .add_modifier(Modifier::BOLD);
    let words = |keys: &[Key]| {
        if keys.is_empty() {
            // Never a blank cell: an unbound action says so.
            "none".to_string()
        } else {
            keys.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        }
    };
    let grid = Grid::new(COLUMNS, width);
    let row = |name: &'static str, keys: &[Key], defaults: &[Key], about: &'static str| {
        let default = keys == defaults;
        grid.row(&[
            Span::styled(name, body),
            Span::styled(words(keys), if default { body } else { changed }),
            Span::styled(words(defaults), muted),
            Span::styled(about, muted),
        ])
    };

    let mut lines = vec![grid.header(theme)];
    for action in Action::LISTED {
        lines.push(row(
            action.name(),
            keymap.keys(action),
            &action.defaults(),
            action.about(),
        ));
    }
    for (widget, listing) in panels {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            crate::grid::truncate(&format!("[{widget}.keys]"), usize::from(width)),
            heading,
        )));
        for listed in listing {
            lines.push(row(
                listed.name,
                &listed.keys,
                &listed.defaults,
                listed.about,
            ));
        }
    }
    lines
}

/// A line to show under the instructions until the next keypress.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Notice {
    text: String,
    failed: bool,
}

/// An open key map.
#[derive(Debug)]
pub struct KeymapDialog {
    /// First line of the body on screen.
    scroll: u16,
    /// How far the body can scroll, measured by the last draw.
    overflow: u16,
    /// How many body lines fit, measured by the last draw.
    viewport: u16,
    /// Asking whether to reset; the next key answers.
    confirming: bool,
    notice: Option<Notice>,
    keys: PanelKeymap<DialogAction>,
}

impl Default for KeymapDialog {
    fn default() -> Self {
        Self {
            scroll: 0,
            overflow: 0,
            viewport: 0,
            confirming: false,
            notice: None,
            keys: keys(),
        }
    }
}

impl KeymapDialog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Say how a reload or a reset went.
    pub fn report(&mut self, text: impl Into<String>, failed: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            failed,
        });
    }

    /// The last report, as its text and whether it was a failure.
    #[cfg(test)]
    pub fn notice(&self) -> Option<(&str, bool)> {
        self.notice
            .as_ref()
            .map(|notice| (notice.text.as_str(), notice.failed))
    }

    /// Whether the dialog is waiting for a yes or no to the reset.
    #[cfg(test)]
    pub fn is_confirming(&self) -> bool {
        self.confirming
    }

    /// Scroll, ask, or report what the shell has to do.
    pub fn handle_key(&mut self, key: KeyEvent) -> Request {
        // A notice answers the last thing asked. The next key is a new
        // question, and an old answer left on screen would read as an answer
        // to that one.
        self.notice = None;

        if self.confirming {
            self.confirming = false;
            // Anything but an explicit yes is a no, including Enter — the
            // same rule `--reset-config` keeps, because the default has to be
            // the answer that loses nothing.
            if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                return Request::Reset;
            }
            self.report("Kept your keys as they are.", false);
            return Request::None;
        }

        if key.code == KeyCode::Esc {
            return Request::Close;
        }
        // Looked up by the key alone, whatever is held with it, which is how
        // this dialog has always read its keys: declaring them changed where
        // they are written, not which presses reach them.
        let Some(action) = self.keys.action(Key::new(key.code, NONE)) else {
            return Request::None;
        };
        let page = self.viewport.max(1);
        let scroll = match action {
            DialogAction::Close => return Request::Close,
            DialogAction::Reload => return Request::Reload,
            DialogAction::Defaults => {
                self.confirming = true;
                return Request::None;
            }
            DialogAction::Down => self.scroll.saturating_add(1),
            DialogAction::Up => self.scroll.saturating_sub(1),
            DialogAction::PageDown => self.scroll.saturating_add(page),
            DialogAction::PageUp => self.scroll.saturating_sub(page),
            DialogAction::First => 0,
            DialogAction::Last => self.overflow,
        };
        self.scroll = scroll.min(self.overflow);
        Request::None
    }

    /// The body, wrapped to `width`: the table, the keys that never change,
    /// how to change the rest, and the notice or question if there is one.
    fn lines(
        &self,
        keymap: &Keymap,
        panels: &[(&'static str, Vec<Listed>)],
        config: Option<&Path>,
        theme: &Theme,
        width: u16,
    ) -> Vec<Line<'static>> {
        let muted = Style::default().fg(theme.muted);
        let key_style = Style::default().fg(theme.key).add_modifier(Modifier::BOLD);
        let mut lines = table(keymap, panels, theme, width);

        lines.push(Line::default());
        lines.push(crate::grid::assemble(
            vec![
                vec![Span::styled("Always", muted)],
                vec![
                    Span::styled("  Ctrl+C", key_style),
                    Span::styled(" quit", muted),
                ],
                vec![
                    Span::styled(format!("  {}", crate::frame::ESC), key_style),
                    Span::styled(" back", muted),
                ],
                vec![
                    Span::styled("  1-9", key_style),
                    Span::styled(" jump", muted),
                ],
            ],
            width,
        ));
        lines.push(Line::default());

        let file = config.map_or_else(
            || "your config file (`mirador --config-path` prints where it is)".to_string(),
            |path| path.display().to_string(),
        );
        let how = format!(
            "To change a key, edit [keys] in {file} — or the table headed \
             above it, for a mode or a panel — and press r here to load \
             it; no restart needed. \
             Keys are written in words, as in resize_wider = \"alt+right\". A \
             list gives an action several keys and [] gives it none. A panel \
             key is offered to that panel first, so it wins while the panel is \
             focused."
        );
        let prose = |text: &str, style: Style, lines: &mut Vec<Line<'static>>| {
            lines.extend(
                crate::grid::wrap(text, usize::from(width))
                    .into_iter()
                    .map(|row| Line::from(Span::styled(row, style))),
            );
        };
        prose(&how, muted, &mut lines);

        if self.confirming {
            lines.push(Line::default());
            prose(
                "Put every key back to its default? Your key lines stay in the \
                 config, commented out. y resets, any other key keeps them.",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
                &mut lines,
            );
        } else if let Some(notice) = &self.notice {
            lines.push(Line::default());
            let colour = if notice.failed {
                theme.error
            } else {
                theme.success
            };
            prose(&notice.text, Style::default().fg(colour), &mut lines);
        }
        lines
    }

    /// The pinned last row: the dialog's keys, and where the body is scrolled
    /// to when it does not fit, Esc the last of them to go. It dropped from
    /// the end, so a narrow key map said how to reload and how to scroll and
    /// not how to leave.
    ///
    /// While the reset question is open, the keys it takes instead. Every key
    /// answers it, so the dialog's own keys and the arrows are all a no, and
    /// offering them said each did what it does the rest of the time.
    ///
    /// Otherwise the primary keys of [`ACTIONS`], then Esc: the hints come
    /// from the table the keys are read from, so the two cannot disagree.
    fn footer(&self, theme: &Theme, width: u16) -> Line<'static> {
        let key_style = Style::default().fg(theme.key).add_modifier(Modifier::BOLD);
        let muted = Style::default().fg(theme.muted);
        let hint = |key: &str, action: &str| {
            vec![
                Span::styled(key.to_string(), key_style),
                Span::styled(format!(" {action}"), muted),
            ]
        };
        let parts = if self.confirming {
            vec![hint("y", "reset"), hint(crate::frame::ESC, "keep")]
        } else {
            let mut parts = Vec::new();
            if self.overflow > 0 {
                let arrows = match (self.scroll > 0, self.scroll < self.overflow) {
                    (true, true) => "↑↓",
                    (true, false) => "↑",
                    _ => "↓",
                };
                parts.push(hint(arrows, "more"));
            }
            parts.extend(
                self.keys
                    .bindings()
                    .iter()
                    .filter(|binding| binding.primary)
                    .map(|binding| hint(&binding.key, &binding.action)),
            );
            parts
        };
        crate::grid::assemble(
            crate::grid::way_out_last(parts, &Span::styled("  ", muted), width),
            width,
        )
    }

    /// Draw the dialog centred over whatever is behind it.
    pub fn render(
        &mut self,
        frame: &mut ratatui::Frame,
        area: Rect,
        keymap: &Keymap,
        panels: &[(&'static str, Vec<Listed>)],
        config: Option<&Path>,
        theme: &Theme,
    ) {
        let width = WIDTH.min(area.width);
        let text_width = width.saturating_sub(crate::frame::FRAME_WIDTH).max(1);
        let lines = self.lines(keymap, panels, config, theme, text_width);
        let text_height = u16::try_from(lines.len()).unwrap_or(u16::MAX);

        // Borders, a blank row and the footer.
        let height = text_height
            .saturating_add(2 + crate::frame::FRAME_HEIGHT)
            .min(area.height);
        let popup = crate::frame::centred(area, width, height);

        let block = crate::frame::dialog_block(theme, "key map", popup.width);
        let inner = block.inner(popup);
        frame.render_widget(Clear, popup);
        frame.render_widget(block, popup);
        if inner.height == 0 || inner.width == 0 {
            self.overflow = 0;
            self.viewport = 0;
            return;
        }

        // The footer gets the last row and a blank above it, but never at
        // the cost of showing no body at all.
        let footer_rows = 2.min(inner.height.saturating_sub(1));
        let viewport = Rect {
            height: inner.height - footer_rows,
            ..inner
        };
        self.viewport = viewport.height;
        self.overflow = text_height.saturating_sub(viewport.height);
        // A question or an answer is drawn at the foot, and is no use below
        // the fold, so either one scrolls to it. Both last one keypress.
        self.scroll = if self.confirming || self.notice.is_some() {
            self.overflow
        } else {
            self.scroll.min(self.overflow)
        };
        frame.render_widget(Paragraph::new(lines).scroll((self.scroll, 0)), viewport);

        if footer_rows > 0 {
            let footer = Rect {
                y: inner.y + inner.height - 1,
                height: 1,
                ..inner
            };
            frame.render_widget(Paragraph::new(self.footer(theme, inner.width)), footer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn keymap(toml_text: &str) -> Keymap {
        Keymap::new(&crate::keymap::keys_config(toml_text)).expect("valid keymap")
    }

    /// Every panel's keys as the dialog lists them, from a config's text.
    fn panel_keys(toml_text: &str) -> crate::keymap::PanelListing {
        let mut config: crate::config::Config = toml::from_str(toml_text).expect("valid config");
        config.theme = Theme::default();
        crate::keymap::KeyTables::from_config(&config)
            .check()
            .expect("valid keys")
            .1
    }

    /// The dialog drawn at `width` x `height`, one string per row.
    fn drawn(dialog: &mut KeymapDialog, keymap: &Keymap, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        let theme = Theme::default();
        terminal
            .draw(|frame| {
                let area = frame.area();
                dialog.render(
                    frame,
                    area,
                    keymap,
                    &panel_keys(""),
                    Some(Path::new("/home/me/.config/mirador/config.toml")),
                    &theme,
                );
            })
            .expect("draw");
        crate::widgets::testing::rows(terminal.backend().buffer())
    }

    #[test]
    fn every_action_is_listed_with_its_keys_and_its_default() {
        let map = keymap("[keys]\nresize_wider = \"alt+right\"");
        // Tall enough for every panel's table above the closing instructions.
        let rows = drawn(&mut KeymapDialog::new(), &map, 100, 250);
        let screen = rows.join("\n");
        for action in Action::LISTED {
            assert!(
                screen.contains(action.name()),
                "{} missing:\n{screen}",
                action.name()
            );
        }
        let wider = rows
            .iter()
            .find(|row| row.contains("resize_wider"))
            .expect("the row");
        assert!(
            wider.contains("Alt+→") && wider.contains("Ctrl+→"),
            "the current key and the default it replaced: {wider}"
        );
        assert!(screen.contains("config.toml"), "names the file to edit");
    }

    /// Each panel whose keys can move is listed under the heading of the
    /// table they are written in, with a moved key drawn as changed.
    #[test]
    fn panel_keys_are_listed_under_the_table_they_are_written_in() {
        let panels = panel_keys("[memory.keys]\nswap = \"w\"");
        let theme = Theme::default();
        let lines = KeymapDialog::new().lines(&Keymap::default(), &panels, None, &theme, 80);
        let text: Vec<String> = lines
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        for scope in crate::widgets::KEY_SCOPES {
            let heading = format!("[{}.keys]", scope.widget);
            assert!(text.contains(&heading), "{heading}: {text:#?}");
        }
        let at = |name: &str| {
            text.iter()
                .position(|row| row.starts_with(name))
                .unwrap_or_else(|| panic!("{name}: {text:#?}"))
        };
        let swap = at("swap");
        assert!(at("[memory.keys]") < swap && swap < at("[disk.keys]"));
        assert!(
            text[swap].contains('w') && text[swap].contains('s'),
            "{}",
            text[swap]
        );
        let keys_style = |row: usize| {
            lines[row]
                .spans
                .iter()
                .filter(|s| !s.content.trim().is_empty())
                .nth(1)
                .map(|s| s.style)
                .expect("a keys cell")
        };
        assert_ne!(
            keys_style(swap),
            keys_style(at("per_core")),
            "a moved key stands out"
        );
    }

    /// The action column holds the word a reader copies into a key table, so
    /// every name the dialog lists has to arrive whole. `priority_previous`
    /// is two cells wider than the column was, and read `priority_previ…` —
    /// a name that, pasted, the reload refuses. Every listed name is checked,
    /// the shell's and every panel's and mode's, so the next long one fails
    /// here rather than on somebody's config.
    #[test]
    fn every_listed_action_name_fits_its_column_whole() {
        let panels = panel_keys("");
        let theme = Theme::default();
        // Inside the frame, which is the width the dialog draws its lines at.
        let width = WIDTH - crate::frame::FRAME_WIDTH;
        let text: Vec<String> = KeymapDialog::new()
            .lines(&Keymap::default(), &panels, None, &theme, width)
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let names: Vec<&str> = Action::LISTED
            .iter()
            .map(|action| action.name())
            .chain(
                panels
                    .iter()
                    .flat_map(|(_, listing)| listing.iter().map(|listed| listed.name)),
            )
            .collect();
        assert!(
            names.contains(&"priority_previous"),
            "the sweep reaches the panels"
        );
        for name in names {
            assert!(
                text.iter().any(|row| row.starts_with(&format!("{name} "))),
                "{name} is cut: {text:#?}"
            );
        }
    }

    /// A changed key is the one thing a reader opening this after a bad edit
    /// is looking for, so it must not look like the others.
    #[test]
    fn a_changed_key_is_drawn_differently_from_a_default_one() {
        let map = keymap("[keys]\nquit = \"x\"");
        let theme = Theme::default();
        let dialog = KeymapDialog::new();
        let lines = dialog.lines(&map, &[], None, &theme, 80);
        let style_of = |name: &str| {
            let line = lines
                .iter()
                .find(|line| line.spans.iter().any(|s| s.content.starts_with(name)))
                .expect("a row");
            line.spans
                .iter()
                .filter(|s| !s.content.trim().is_empty())
                .nth(1)
                .map(|s| s.style)
                .expect("a keys cell")
        };
        assert_ne!(style_of("quit"), style_of("help"));
        assert_eq!(style_of("help"), style_of("theme"), "defaults all alike");
    }

    #[test]
    fn an_unbound_action_says_none_rather_than_leaving_a_blank() {
        let map = keymap("[keys]\ntheme = []");
        let rows = drawn(&mut KeymapDialog::new(), &map, 100, 40);
        let theme_row = rows.iter().find(|row| row.contains("theme ")).expect("row");
        assert!(theme_row.contains("none"), "{theme_row}");
    }

    /// Reset is asked, never assumed, and only `y` means yes.
    #[test]
    fn defaults_asks_first_and_only_y_resets() {
        let mut dialog = KeymapDialog::new();
        assert_eq!(dialog.handle_key(key(KeyCode::Char('d'))), Request::None);
        assert!(dialog.is_confirming());
        assert_eq!(dialog.handle_key(key(KeyCode::Enter)), Request::None);
        assert!(!dialog.is_confirming(), "any other key is a no");

        dialog.handle_key(key(KeyCode::Char('d')));
        assert_eq!(dialog.handle_key(key(KeyCode::Char('y'))), Request::Reset);
    }

    /// Esc during the question cancels the question, not the dialog — the
    /// reader asked to back out of one thing.
    #[test]
    fn esc_while_asking_backs_out_of_the_question_only() {
        let mut dialog = KeymapDialog::new();
        dialog.handle_key(key(KeyCode::Char('d')));
        assert_eq!(dialog.handle_key(key(KeyCode::Esc)), Request::None);
        assert_eq!(dialog.handle_key(key(KeyCode::Esc)), Request::Close);
    }

    /// Every key the dialog acts on is one [`ACTIONS`] declares, or Esc, and
    /// every key it declares does something. The keys used to be matched by
    /// hand beside a footer and a README written by hand, and `q` closed the
    /// dialog with neither of them saying so.
    #[test]
    fn every_key_the_dialog_acts_on_is_one_it_declares() {
        let declared = PanelKeymap::defaults("key_map", ACTIONS);
        let mut codes: Vec<KeyCode> = (' '..='~').map(KeyCode::Char).collect();
        codes.extend([
            KeyCode::Esc,
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::F(5),
        ]);
        for code in codes {
            let mut dialog = KeymapDialog::new();
            // Mid-scroll, so a scroll key in either direction moves it.
            dialog.overflow = 20;
            dialog.viewport = 4;
            dialog.scroll = 10;
            let request = dialog.handle_key(key(code));
            let acted = request != Request::None || dialog.scroll != 10 || dialog.is_confirming();
            let ours = code == KeyCode::Esc || declared.action(Key::new(code, NONE)).is_some();
            assert_eq!(acted, ours, "{code:?} acts: {acted}, declared: {ours}");
        }
        assert_eq!(
            KeymapDialog::new().handle_key(key(KeyCode::Char('q'))),
            Request::Close,
            "q closes the key map, as it closes both pickers"
        );
    }

    #[test]
    fn the_question_is_on_screen_when_it_is_asked() {
        let map = Keymap::default();
        let mut dialog = KeymapDialog::new();
        // Short enough that the body scrolls, so the question would be below
        // the fold if asking it did not bring it into view.
        drawn(&mut dialog, &map, 80, 16);
        dialog.handle_key(key(KeyCode::Char('d')));
        let screen = drawn(&mut dialog, &map, 80, 16).join("\n");
        assert!(screen.contains("y resets"), "{screen}");
    }

    #[test]
    fn a_notice_lasts_until_the_next_key() {
        let map = Keymap::default();
        let mut dialog = KeymapDialog::new();
        dialog.report("Loaded your keys.", false);
        assert!(
            drawn(&mut dialog, &map, 100, 40)
                .join("\n")
                .contains("Loaded your keys.")
        );
        dialog.handle_key(key(KeyCode::Down));
        assert!(
            !drawn(&mut dialog, &map, 100, 40)
                .join("\n")
                .contains("Loaded your keys.")
        );
    }

    /// While the reset question is open every key answers it: `y` resets and
    /// anything else keeps the keys, Esc included. The footer went on
    /// offering `r reload  d defaults  Esc close` through the question —
    /// three keys that each did something else — and the arrows, which
    /// answer it too.
    #[test]
    fn the_footer_offers_the_keys_the_reset_question_takes() {
        let map = Keymap::default();
        let mut dialog = KeymapDialog::new();
        let footer = |dialog: &mut KeymapDialog| {
            let rows = drawn(dialog, &map, 100, 24);
            let bottom = rows
                .iter()
                .rposition(|row| row.contains('╰'))
                .expect("a bottom border");
            rows[bottom - 1].clone()
        };
        let calm = footer(&mut dialog);
        assert!(
            calm.contains("more") && calm.contains("Esc close"),
            "short enough to scroll, so the arrows are offered: {calm}"
        );

        dialog.handle_key(key(KeyCode::Char('d')));
        let asking = footer(&mut dialog);
        assert!(
            asking.contains("y reset") && asking.contains("Esc keep"),
            "{asking}"
        );
        for gone in ["reload", "defaults", "close", "more"] {
            assert!(
                !asking.contains(gone),
                "{gone} offered mid-question: {asking}"
            );
        }

        // And Esc does what the footer said: keeps the keys, and the dialog.
        assert_eq!(dialog.handle_key(key(KeyCode::Esc)), Request::None);
        assert!(footer(&mut dialog).contains("Esc close"));
    }

    #[test]
    fn it_draws_without_panicking_at_every_size_down_to_one_cell() {
        let map = keymap("[keys]\nquit = [\"q\", \"x\"]");
        for (w, h) in [(120u16, 40u16), (80, 24), (40, 12), (10, 5), (2, 2), (1, 1)] {
            let mut dialog = KeymapDialog::new();
            dialog.report("x".repeat(500), true);
            for _ in 0..3 {
                drawn(&mut dialog, &map, w, h);
                dialog.handle_key(key(KeyCode::PageDown));
            }
        }
    }

    /// Invariant 19: nothing wider than the space it was given, at any width.
    #[test]
    fn no_line_is_built_wider_than_the_dialog() {
        let map = keymap("[keys]\nquit = [\"q\", \"x\", \"ctrl+alt+shift+f12\"]");
        let theme = Theme::default();
        let mut dialog = KeymapDialog::new();
        dialog.report(
            "a notice long enough to wrap across several rows of the dialog",
            true,
        );
        for width in 1..=90u16 {
            for line in dialog
                .lines(
                    &map,
                    &panel_keys("[cpu.keys]\nper_core = [\"p\", \"ctrl+alt+shift+f11\"]"),
                    Some(Path::new("/a/long/path/to/config.toml")),
                    &theme,
                    width,
                )
                .iter()
                .chain([&dialog.footer(&theme, width)])
            {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert!(
                    crate::grid::display_width(&text) <= usize::from(width),
                    "{width}: {text:?}"
                );
            }
        }
    }

    /// The explanation is the column that says what a key does, and at the
    /// dialog's widest it arrived as `move the panel down a row, or pas…`
    /// — nineteen of them cut, at a width where nothing was squeezing them,
    /// so no terminal was wide enough to read them whole. Drawn on a screen
    /// wider than the cap, every one has to be on it in full: the cap and the
    /// longest explanation are held to each other here, so a longer one
    /// fails rather than arriving with an `…` on every screen.
    #[test]
    fn every_explanation_arrives_whole_at_the_dialogs_widest() {
        let rows = drawn(
            &mut KeymapDialog::new(),
            &Keymap::default(),
            WIDTH + 40,
            400,
        );
        let panels = panel_keys("");
        let cut: Vec<(&str, &str)> = Action::LISTED
            .iter()
            .map(|action| ("keys", action.about()))
            .chain(panels.iter().flat_map(|(widget, listing)| {
                listing.iter().map(move |listed| (*widget, listed.about))
            }))
            .filter(|(_, about)| !rows.iter().any(|row| row.contains(about)))
            .collect();
        assert!(
            cut.is_empty(),
            "{} cut at the dialog's widest: {cut:#?}",
            cut.len()
        );
    }
}
