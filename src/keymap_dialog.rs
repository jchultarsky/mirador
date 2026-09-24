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
//! again — `[keys]` and each panel's `[<widget>.keys]` —
//! so a reader can edit the config in another pane and try the result without
//! restarting — and a mistake is shown here, with the keys they had still in
//! force, rather than at the next launch as a dashboard that will not start.
//! **Defaults** comments out their key lines, after asking, for the
//! reader who has lost track of what they changed. The same reset is
//! `mirador --reset-keys` for the case this dialog cannot reach: a keymap so
//! broken that mirador refuses to start.
//!
//! The dialog's own keys are fixed. A key map that could lose the key that
//! closes the key map is the trap invariant 2 exists to rule out.

use std::path::Path;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph};

use crate::grid::{Column, Grid};
use crate::keymap::{Action, Key, Keymap, Listed};
use crate::theme::Theme;

/// What a keypress asked the shell to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Nothing the shell needs to act on.
    None,
    /// Close the dialog.
    Close,
    /// Read `[keys]` from the config again and use it if it is valid.
    Reload,
    /// Comment out `[keys]` in the config and go back to the defaults. Only
    /// sent after the reader has confirmed it.
    Reset,
}

/// The key table. The action column is sized to its longest name,
/// `resize_narrower`, since that is the word a reader copies into a key table;
/// the explanation goes first when the dialog is squeezed, then the defaults.
pub const COLUMNS: &[Column] = &[
    Column::fixed("action", 15),
    Column::flex("keys", 2),
    Column::flex("default", 2).drops_below(44),
    Column::flex("does", 5).drops_below(64),
];

/// The widest the dialog draws. Wide enough for every column at ordinary
/// terminal widths; wider would only stretch the explanations.
const WIDTH: u16 = 84;

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
#[derive(Debug, Default)]
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

        let page = self.viewport.max(1);
        let scroll = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Request::Close,
            KeyCode::Char('r') => return Request::Reload,
            KeyCode::Char('d') => {
                self.confirming = true;
                return Request::None;
            }
            KeyCode::Down | KeyCode::Char('j') => self.scroll.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll.saturating_sub(1),
            KeyCode::PageDown => self.scroll.saturating_add(page),
            KeyCode::PageUp => self.scroll.saturating_sub(page),
            KeyCode::Home => 0,
            KeyCode::End => self.overflow,
            _ => return Request::None,
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
                    Span::styled("  Esc", key_style),
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
             above it, for arrange mode or a panel — and press r here to load \
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
    /// to when it does not fit.
    fn footer(&self, theme: &Theme, width: u16) -> Line<'static> {
        let key_style = Style::default().fg(theme.key).add_modifier(Modifier::BOLD);
        let muted = Style::default().fg(theme.muted);
        let mut parts = Vec::new();
        if self.overflow > 0 {
            let arrows = match (self.scroll > 0, self.scroll < self.overflow) {
                (true, true) => "↑↓",
                (true, false) => "↑",
                _ => "↓",
            };
            parts.push(vec![
                Span::styled(arrows, key_style),
                Span::styled(" more  ", muted),
            ]);
        }
        for (index, (key, action)) in [("r", "reload"), ("d", "defaults"), ("Esc", "close")]
            .into_iter()
            .enumerate()
        {
            let gap = if index == 0 { "" } else { "  " };
            parts.push(vec![
                Span::styled(format!("{gap}{key}"), key_style),
                Span::styled(format!(" {action}"), muted),
            ]);
        }
        crate::grid::assemble(parts, width)
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
        let height = text_height.saturating_add(4).min(area.height);
        let popup = crate::frame::centred(area, width, height);

        let border = Style::default().fg(theme.border_focused);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border)
            .padding(Padding::horizontal(1))
            .title(Line::from(vec![
                Span::styled("┤", border),
                Span::styled(
                    crate::glyphs::utility("key map"),
                    Style::default()
                        .fg(theme.title)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("├", border),
            ]));
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
        let mut tables: std::collections::BTreeMap<String, crate::keymap::KeysConfig> =
            toml::from_str(toml_text).expect("valid TOML");
        Keymap::new(&tables.remove("keys").unwrap_or_default()).expect("valid keymap")
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
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
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
}
