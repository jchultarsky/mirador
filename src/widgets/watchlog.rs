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
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::frame::Binding;
use crate::keymap::{KeysConfig, Meta, PanelKeymap};
use crate::panel::{KeyOutcome, Panel, RenderContext};

/// What the watch log's keys do. It only scrolls: nothing here dismisses an
/// entry, since an entry you can dismiss is one you are expected to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchlogAction {
    Up,
    Down,
    First,
    Last,
    PageUp,
    PageDown,
}

/// Every key the panel responds to, under `[watchlog.keys]`. The border hint, the
/// status bar and the help overlay are derived from it, so a key the panel
/// reads is a key it advertises.
pub const ACTIONS: &[Meta<WatchlogAction>] = &[
    Meta {
        action: WatchlogAction::Up,
        name: "up",
        defaults: &[
            (KeyCode::Up, KeyModifiers::NONE),
            (KeyCode::Char('k'), KeyModifiers::NONE),
        ],
        label: "scroll",
        primary: false,
        joins: false,
        about: "scroll up",
    },
    Meta {
        action: WatchlogAction::Down,
        name: "down",
        defaults: &[
            (KeyCode::Down, KeyModifiers::NONE),
            (KeyCode::Char('j'), KeyModifiers::NONE),
        ],
        label: "scroll",
        primary: false,
        joins: true,
        about: "scroll down",
    },
    Meta {
        action: WatchlogAction::First,
        name: "first",
        defaults: &[
            (KeyCode::Char('g'), KeyModifiers::NONE),
            (KeyCode::Home, KeyModifiers::NONE),
        ],
        label: "first",
        primary: false,
        joins: false,
        about: "scroll to the top",
    },
    Meta {
        action: WatchlogAction::Last,
        name: "last",
        defaults: &[
            (KeyCode::Char('G'), KeyModifiers::NONE),
            (KeyCode::End, KeyModifiers::NONE),
        ],
        label: "last",
        primary: false,
        joins: true,
        about: "scroll to the bottom",
    },
    Meta {
        action: WatchlogAction::PageUp,
        name: "page_up",
        defaults: &[(KeyCode::PageUp, KeyModifiers::NONE)],
        label: "scroll ten rows",
        primary: false,
        joins: false,
        about: "scroll ten rows up",
    },
    Meta {
        action: WatchlogAction::PageDown,
        name: "page_down",
        defaults: &[(KeyCode::PageDown, KeyModifiers::NONE)],
        label: "scroll ten rows",
        primary: false,
        joins: true,
        about: "scroll ten rows down",
    },
];

/// `[watchlog.keys]` laid over [`ACTIONS`], or why it cannot be.
pub fn keymap(keys: &KeysConfig) -> Result<PanelKeymap<WatchlogAction>, String> {
    PanelKeymap::new("watchlog", ACTIONS, keys)
}

/// The keys the panel starts with, until `build` hands it the config's
/// through [`Panel::set_keys`].
fn default_keys() -> PanelKeymap<WatchlogAction> {
    PanelKeymap::defaults("watchlog", ACTIONS)
}

/// Interior width past which the panel gains nothing: a time, a source and a
/// sentence with room to breathe.
const USEFUL_WIDTH: u16 = 56;

pub struct WatchLogPanel {
    /// `[watchlog.keys]` over the defaults.
    keys: PanelKeymap<WatchlogAction>,
    /// The agenda panel's key for setting its file, which the empty log
    /// names; `None` when `[agenda.keys]` leaves it unbound.
    agenda_file_key: Option<crate::keymap::Key>,
    scroll: ListState,
    /// Entries drawn last frame, so `tick` can answer honestly.
    drawn: usize,
}

impl WatchLogPanel {
    /// Takes no configuration, and that is the point.
    ///
    /// It used to take `&Config`, for one `bool` recording whether the agenda
    /// had a file. Reading another panel's settings is the coupling this design
    /// avoids: the value was true or false for ever from the moment it was
    /// read, so a calendar set later through `f` was never noticed. Panels stay
    /// independent, so the log describes what it watches rather than reporting
    /// on a panel it cannot see.
    ///
    /// The one thing it does take from the agenda is a key, in
    /// [`Panel::set_keys`], and that is a different kind of fact: the file is
    /// the agenda's to change at any moment, while its keys change only when
    /// the key tables are read again — and every panel is handed them then.
    pub fn new() -> Self {
        Self {
            keys: default_keys(),
            agenda_file_key: crate::widgets::agenda::file_key(&KeysConfig::default()),
            scroll: ListState::default(),
            drawn: 0,
        }
    }
}

impl Default for WatchLogPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl Panel for WatchLogPanel {
    fn title(&self) -> String {
        "Watch log".into()
    }

    fn bindings(&self) -> &[Binding] {
        self.keys.bindings()
    }

    fn set_keys(&mut self, config: &crate::config::Config) {
        self.keys = PanelKeymap::or_defaults("watchlog", ACTIONS, &config.watchlog.keys);
        self.agenda_file_key = crate::widgets::agenda::file_key(&config.agenda.keys);
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
        let Some(action) = self.keys.action(key) else {
            return KeyOutcome::Ignored;
        };
        let (down, rows) = match action {
            WatchlogAction::Down => (true, 1),
            WatchlogAction::Up => (false, 1),
            WatchlogAction::PageDown => (true, 10),
            WatchlogAction::PageUp => (false, 10),
            WatchlogAction::Last => (true, usize::MAX),
            WatchlogAction::First => (false, usize::MAX),
        };
        if down {
            crate::selection::down(&mut self.scroll, rows, self.drawn);
        } else {
            crate::selection::up(&mut self.scroll, rows, self.drawn);
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
                // Truncated here rather than by the list. An entry is written
                // by whichever panel reported it — a task title, a calendar
                // summary — so its length is the user's, not this panel's, and
                // `Renew the domain went over` reads as a sentence that
                // happens to end there.
                Span::styled(
                    crate::grid::truncate(&entry.text, usize::from(area.width).saturating_sub(6)),
                    Style::default().fg(theme.text),
                ),
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
            // Wrapped like the explanation below it, rather than trusted to
            // fit. Hand-broken, the first row lost `happened` in a narrow panel
            // and the two rows still read as a sentence — `Nothing has` above
            // `since 00:30.` is a claim with a word missing from the middle,
            // which is worse than one that takes an extra row.
            let bold = Style::default().fg(theme.text).add_modifier(Modifier::BOLD);
            let muted = Style::default().fg(theme.muted);
            let mut lines: Vec<(String, Style)> = crate::grid::wrap("Nothing has happened", width)
                .into_iter()
                .map(|row| (row, bold))
                .collect();
            lines.extend(
                crate::grid::wrap(&format!("since {}.", started(log.since())), width)
                    .into_iter()
                    .map(|row| (row, muted)),
            );
            lines.push((String::new(), muted));

            // Wrapped rather than hand-broken. The first version assumed a
            // width the panel does not have and lost its last line off the
            // bottom, which is a poor way to explain something.
            let explain = |lines: &mut Vec<(String, Style)>, text: &str| {
                lines.extend(
                    crate::grid::wrap(text, width)
                        .into_iter()
                        .map(|row| (row, muted)),
                );
            };
            explain(
                &mut lines,
                "Watching for things you did not do yourself: the day turning, \
                 a task falling overdue, an entry appearing in your calendar.",
            );
            lines.push((String::new(), muted));
            // Says where calendar entries come from, and asserts nothing about
            // whether you have one. The previous wording — "No calendar set, so
            // that last one cannot happen. Press f on the agenda panel to add
            // one." — was decided once at construction, so setting a calendar
            // with `f` left this panel telling you to set the calendar you had
            // just set, until a restart.
            //
            // Not fixed by re-deriving the flag, because a *correct* version of
            // that sentence is still the wrong shape. A hint aimed at someone
            // who has not set a calendar reaches someone who decided against one
            // just as often, and the dashboard cannot tell them apart — the
            // reasoning that retired the unused-widget notice. A statement of
            // where the entries come from is useful to the first reader and
            // merely true for the second.
            //
            // The key is the one `[agenda.keys]` gave `file`: this said `f`
            // whatever that table said. With `file` unbound there is no key
            // to name, and the sentence says where the entries come from.
            let source = match self.agenda_file_key {
                Some(key) => format!(
                    "Calendar entries come from [agenda].file, which pressing {key} \
                     on the agenda panel sets."
                ),
                None => "Calendar entries come from [agenda].file.".to_string(),
            };
            explain(&mut lines, &source);
            // Fitted to the height as well as the width (invariant 19). Handed
            // whole to the `Paragraph`, the rows past the panel's foot were
            // dropped in silence: the shipped dashboard's log stopped at `day
            // turning, a task`, and a taller one at `Calendar entries come
            // from`, before the key the sentence exists to name. The rows that
            // fit are kept, a blank spacer is not left as the last, and the
            // last kept row says the rest is missing.
            let height = usize::from(area.height);
            if lines.len() > height {
                lines.truncate(height);
                while lines.last().is_some_and(|(row, _)| row.is_empty()) {
                    lines.pop();
                }
                if let Some((last, _)) = lines.last_mut() {
                    *last = crate::grid::truncate(&format!("{}…", last.trim_end()), width);
                }
            }
            let lines: Vec<Line<'static>> = lines
                .into_iter()
                .map(|(row, style)| Line::from(Span::styled(row, style)))
                .collect();
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
    if label_width > usize::from(width) {
        // No room for a dash or the label's padding: the label alone, cut
        // with an ellipsis where it has to be. It used to be emitted whole
        // with no dashes, wider than the panel, and the terminal cut it.
        return Line::from(Span::styled(
            crate::grid::truncate(label, usize::from(width)),
            Style::default().fg(theme.muted),
        ));
    }
    let dashes = usize::from(width) - label_width;
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

    /// Every key in the map works and is advertised; see
    /// [`crate::keymap::assert_every_key_works`].
    #[test]
    fn every_key_in_the_map_works_and_is_advertised() {
        crate::keymap::assert_every_key_works(&default_keys(), |event| {
            WatchLogPanel::new().handle_key(event)
        });
    }

    #[test]
    fn a_rule_line_fills_its_width_exactly() {
        let theme = crate::theme::Theme::default();
        let label = "since you were here";
        // From one cell: the sweep used to start at 24, above the 21 cells the
        // padded label needs, so it never saw the label emitted whole into a
        // narrower panel and cut by the terminal (invariant 19).
        for width in 1..90u16 {
            let line = rule_line(width, label, &theme);
            let drawn: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let cells = crate::grid::display_width(&drawn);
            assert!(cells <= usize::from(width), "at {width}: {drawn:?}");
            if usize::from(width) >= crate::grid::display_width(label) + 2 {
                assert_eq!(cells, usize::from(width), "at {width}");
            } else {
                assert!(
                    drawn == label || drawn.ends_with('…'),
                    "at {width}: {drawn:?} is whole or says it was cut"
                );
            }
        }
    }

    /// Render the empty panel, given its keys from `config` as the shell
    /// gives them, and read the words back off the screen — rows joined by a
    /// space, so a sentence reads the same wherever the wrap fell.
    fn empty_panel_text(config: &crate::config::Config) -> String {
        let mut panel = WatchLogPanel::new();
        panel.set_keys(config);
        let buffer = crate::widgets::testing::render_in(&mut panel, 60, 14, &config.theme, false);
        crate::widgets::testing::rows(&buffer)
            .iter()
            .map(|row| row.trim())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Invariant 19 along the height: the empty log's explanation arrives
    /// whole wherever the panel has the rows for it, and otherwise its last
    /// row ends in `…`.
    ///
    /// It was one `Paragraph` of wrapped rows with nothing fitting them to
    /// the panel, so the terminal kept the rows that fitted and dropped the
    /// rest in silence: at 30x10 it ended `Calendar entries come from`, a
    /// sentence missing the half that names the key, and the shipped
    /// dashboard's log stopped at `day turning, a task`. A width sweep cannot
    /// see this, because a row that is never drawn has nothing to difference.
    /// Every height is compared with the same width drawn tall enough to
    /// hold everything; the clock in `since 13:04.` is masked, since a sweep
    /// this long can cross a minute.
    #[test]
    fn the_empty_log_says_when_its_explanation_has_been_cut() {
        let read = |panel: &mut WatchLogPanel, width: u16, height: u16| -> Vec<String> {
            let mut rows: Vec<String> = crate::widgets::testing::screen(panel, width, height)
                .into_iter()
                .map(|row| row.replace(|c: char| c.is_ascii_digit(), "0"))
                .collect();
            while rows.last().is_some_and(String::is_empty) {
                rows.pop();
            }
            rows
        };
        let mut panel = WatchLogPanel::new();
        let (mut cut, mut whole) = (0, 0);
        for width in 6..=60u16 {
            let everything = read(&mut panel, width, 60);
            for height in 1..=24u16 {
                let drawn = read(&mut panel, width, height);
                if everything.len() <= usize::from(height) {
                    whole += 1;
                    assert_eq!(drawn, everything, "at {width}x{height}");
                    continue;
                }
                cut += 1;
                let (last, kept) = drawn.split_last().expect("something is drawn");
                assert!(
                    last.ends_with('…'),
                    "at {width}x{height} the log was cut without saying so: {drawn:#?}"
                );
                assert_ne!(
                    last, "…",
                    "at {width}x{height} the ellipsis stands on a spacer row of its own"
                );
                assert_eq!(
                    kept,
                    &everything[..kept.len()],
                    "at {width}x{height} every row before the cut is the log's own"
                );
            }
        }
        assert!(cut > 0, "the sweep reached a size that cuts the log");
        assert!(whole > 0, "the sweep reached a size that holds all of it");
    }

    /// The empty log says where calendar entries come from and offers the
    /// agenda's key for setting it, which `[agenda.keys]` can move: it said
    /// `f` whatever that table said. With `file` unbound there is no key to
    /// offer, and the sentence says where the entries come from and stops.
    #[test]
    fn the_empty_panel_names_the_agenda_file_key_it_has() {
        let text = |keys: &str| {
            let config: crate::config::Config =
                toml::from_str(&format!("[agenda.keys]\n{keys}")).expect("a config");
            empty_panel_text(&config)
        };
        let moved = text("file = \"i\"");
        assert!(moved.contains("pressing i on the agenda panel"), "{moved}");
        assert!(!moved.contains("pressing f"), "{moved}");

        let unbound = text("file = []");
        assert!(
            unbound.contains("Calendar entries come from [agenda].file."),
            "{unbound}"
        );
        assert!(!unbound.contains("pressing"), "{unbound}");

        assert!(text("").contains("pressing f on the agenda panel sets."));
    }

    /// The empty panel must not claim anything about whether a calendar is set,
    /// because it cannot know: panels are independent, and the agenda owns its
    /// own path.
    ///
    /// The bug this replaces: the wording was chosen from `config.agenda.file`
    /// once at construction, so setting a calendar with `f` left the log saying
    /// "No calendar set... Press f on the agenda panel to add one" — telling you
    /// to do the thing you had just done — until a restart.
    ///
    /// Re-deriving the flag would have fixed the staleness and kept the wrong
    /// shape. A hint aimed at someone who has not set a calendar reaches someone
    /// who decided against one just as often, and this dashboard cannot tell
    /// them apart; that is what retired the unused-widget notice.
    ///
    /// The obvious test — render with and without `agenda.file` and assert the
    /// text matches — was written first and **deleted**, because it cannot fail:
    /// `WatchLogPanel::new` takes no configuration, so nothing about the agenda's
    /// file can reach this panel to differ in the first place — `set_keys`
    /// carries the agenda's keys and nothing else. It passed with the old
    /// wording pasted back in, which is the tell. The assertion below is the one
    /// that goes red when the claim returns, and it was checked by restoring the
    /// old sentence and watching it fail.
    #[test]
    fn the_empty_panel_still_says_where_calendar_entries_come_from() {
        let text = empty_panel_text(&crate::config::Config::default());
        assert!(
            text.contains("[agenda].file"),
            "the empty log should name the setting; got:\n{text}"
        );
        assert!(
            !text.contains("No calendar set"),
            "the empty log must not assert whether a calendar is set; got:\n{text}"
        );
    }

    /// The frame carries no counter, and that is a decision rather than an
    /// omission. Every other list panel here has one — `4 open`, `2 today` —
    /// so the obvious "improvement" is to give this one `3 new`. That number is
    /// a badge, a badge accumulates, and an accumulating badge is exactly the
    /// unread-message count this dashboard turned down. If this assertion ever
    /// fails, the question to ask is not how to fix the test.
    #[test]
    fn the_panel_never_offers_a_counter() {
        assert_eq!(WatchLogPanel::new().counter(), None);
    }

    /// The log is read, never edited. A key that dismissed an entry would make
    /// the log something you are expected to keep up with, which is the
    /// obligation this panel is designed to avoid.
    #[test]
    fn nothing_dismisses_acknowledges_or_clears_an_entry() {
        let mut panel = WatchLogPanel::new();
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
