//! A one-line question a panel can ask.
//!
//! Most of what the dashboard lets you change is a toggle — units, sort order,
//! whether seconds show — and a toggle needs no interface beyond the key that
//! flips it. Three settings are not toggles: the agenda's `.ics` file, the
//! weather location, and the name of a timezone to add to the clock. Each of
//! those is free text, and each was stuck behind "edit your config file"
//! for want of somewhere to type.
//!
//! This is that somewhere, once, rather than three times. A panel opens it with
//! a label and the value as it stands, reads [`Outcome`] back out of
//! `handle_key`, and decides for itself whether the answer is any good — the
//! prompt has no idea what a timezone is and does not want one. A rejected
//! answer comes back with [`Prompt::reject`] and the dialog stays open with the
//! text still in it, because retyping a long path to fix one character is the
//! kind of thing that stops people using a feature at all.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph};

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::textfield::TextField;
use crate::theme::Theme;

/// What `Tab` offers to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    /// Nothing; `Tab` does nothing.
    None,
    /// Filesystem paths, for the agenda file.
    Paths,
    /// A list to choose from, shown under the field and narrowed as you type.
    ///
    /// For a value the reader is not expected to know by heart. A timezone is
    /// the case that prompted it: the identifier names a city, and it is very
    /// often not the city the reader means.
    Places(&'static [crate::zones::Place]),
}

/// What a keypress did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Still typing.
    Editing,
    /// Backed out; the panel should close the prompt and change nothing.
    Cancelled,
    /// Enter, with nothing selected from a list: whatever was typed. The panel
    /// decides whether to accept it.
    Submitted(String),
    /// Enter, on a row of the list. Carries the label the reader recognised as
    /// well as the value, because the two differ and the label is the better
    /// name for the thing — someone who picked Seattle wants a clock that says
    /// Seattle, not one that says Los Angeles.
    Chose {
        label: &'static str,
        value: &'static str,
    },
}

/// Rows of the list shown at once.
///
/// The list is drawn inside a dialog rather than a panel, so this is a fixed
/// window rather than something the layout decides — and it has to be a
/// constant both `handle_key` and `render` can see, because the first has to
/// know how far it may scroll and the second has to draw the same window.
const LIST_ROWS: usize = 10;

/// An open prompt.
#[derive(Debug)]
pub struct Prompt {
    label: &'static str,
    help: &'static str,
    field: TextField,
    error: Option<String>,
    completion: Completion,
    /// Which row of the filtered list is selected, if any.
    selected: usize,
    /// The first row on screen. Without it the cursor walked off the bottom of
    /// the window and kept going, invisibly.
    offset: usize,
    /// The rows matching what has been typed, recomputed only when the text
    /// changes.
    ///
    /// This used to be derived on demand — and `render` derived it too, every
    /// frame, lowercasing all 143 cities and all 143 identifiers each time. The
    /// absolute cost was small; the shape was the problem. A dialog must
    /// allocate in proportion to what is on screen, which is ten rows, not in
    /// proportion to how large the table behind it happens to be.
    listed: Vec<&'static crate::zones::Place>,
}

impl Prompt {
    /// Ask `label`, starting from `value`.
    pub fn new(
        label: &'static str,
        help: &'static str,
        value: &str,
        completion: Completion,
    ) -> Self {
        let mut prompt = Self {
            label,
            help,
            field: TextField::with_value(value),
            error: None,
            completion,
            selected: 0,
            offset: 0,
            listed: Vec::new(),
        };
        prompt.refilter();
        prompt
    }

    /// The list rows matching what has been typed so far.
    ///
    /// Matched against the city *and* the identifier, case-insensitively and
    /// anywhere in either — so `seattle` finds `America/Los_Angeles`, `asia`
    /// finds every Asian zone, and `kolkata` finds the identifier directly.
    /// Prefix-only matching would answer half these and is the reason a plain
    /// completion was not enough.
    pub fn matches(&self) -> &[&'static crate::zones::Place] {
        &self.listed
    }

    /// Recompute [`Prompt::matches`] from the text as it now stands.
    ///
    /// Called when the text changes and at no other time — in particular not
    /// from `render`, which is the whole point.
    fn refilter(&mut self) {
        let Completion::Places(places) = self.completion else {
            self.listed.clear();
            return;
        };
        let needle = self.field.trimmed().to_lowercase();
        self.listed = places
            .iter()
            .filter(|place| {
                needle.is_empty()
                    || place.city.to_lowercase().contains(&needle)
                    || place.tz.to_lowercase().contains(&needle)
            })
            .collect();
    }

    /// Refuse the answer and say why, leaving the text in place to be fixed.
    pub fn reject(&mut self, why: impl Into<String>) {
        self.error = Some(why.into());
    }

    /// The text as it stands. Exposed for tests.
    #[cfg(test)]
    pub fn value(&self) -> &str {
        self.field.value()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        // A keystroke means the user is dealing with the complaint, so the
        // complaint goes.
        self.error = None;

        let listed = self.matches();
        match key.code {
            KeyCode::Esc => Outcome::Cancelled,
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(listed.len().saturating_sub(1));
                self.scroll_into_view();
                Outcome::Editing
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                self.scroll_into_view();
                Outcome::Editing
            }
            KeyCode::PageDown => {
                self.selected = (self.selected + LIST_ROWS).min(listed.len().saturating_sub(1));
                self.scroll_into_view();
                Outcome::Editing
            }
            KeyCode::PageUp => {
                self.selected = self.selected.saturating_sub(LIST_ROWS);
                self.scroll_into_view();
                Outcome::Editing
            }
            KeyCode::Home if !listed.is_empty() => {
                self.selected = 0;
                self.scroll_into_view();
                Outcome::Editing
            }
            KeyCode::End if !listed.is_empty() => {
                self.selected = listed.len() - 1;
                self.scroll_into_view();
                Outcome::Editing
            }
            KeyCode::Enter => match listed.get(self.selected) {
                Some(place) => Outcome::Chose {
                    label: place.city,
                    value: place.tz,
                },
                // Nothing matched, so the reader knows something the list does
                // not. Hand back what they typed rather than refusing it.
                None => Outcome::Submitted(self.field.trimmed().to_string()),
            },
            KeyCode::Tab => {
                self.complete();
                self.refilter();
                Outcome::Editing
            }
            // Moving within the text is not editing it, so the list is left
            // alone. Lumped in with typing, these reset the selection: you
            // scrolled to row thirty, pressed Left to fix a typo, and the
            // highlight jumped back to the top of the list.
            //
            // Home and End are absent because the arms above claim them
            // whenever there is a list to move through; they reach the field
            // only when there is not, and then there is no selection to lose.
            KeyCode::Left | KeyCode::Right => {
                self.field.handle_key(key);
                Outcome::Editing
            }
            _ => {
                self.field.handle_key(key);
                self.refilter();
                // Typing narrows the list under the cursor, so a selection two
                // rows down would otherwise land on something unrelated — or
                // past the end of what is left.
                self.selected = 0;
                self.offset = 0;
                Outcome::Editing
            }
        }
    }

    /// Keep the selected row inside the window that is drawn.
    ///
    /// Only moves the window when the selection would otherwise leave it, so
    /// the list stays put while the cursor travels across it and scrolls by one
    /// at the edges. Recomputing the window from the selection each time would
    /// be stateless and simpler, and would also jump the whole list whenever
    /// you crossed a page boundary going back up.
    fn scroll_into_view(&mut self) {
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + LIST_ROWS {
            self.offset = self.selected + 1 - LIST_ROWS;
        }
    }

    /// Extend the text as far as every candidate agrees.
    ///
    /// Completing only to the common prefix is what a shell does, and it is the
    /// behaviour that never surprises: it either finishes the job or stops at
    /// the point where a choice genuinely has to be made. Completing to the
    /// first match instead would silently pick one of several.
    fn complete(&mut self) {
        let typed = self.field.value().to_string();
        let (fixed, stem) = match self.completion {
            // A list is chosen from rather than completed into, so neither of
            // these has anything for Tab to do.
            Completion::None | Completion::Places(_) => return,
            Completion::Paths => match typed.rfind('/') {
                Some(cut) => (typed[..=cut].to_string(), typed[cut + 1..].to_string()),
                None => (String::new(), typed.clone()),
            },
        };

        let candidates = self.candidates(&fixed, &stem);
        let Some(shared) = common_prefix(&candidates) else {
            return;
        };
        if shared.len() <= stem.len() {
            return;
        }

        self.field = TextField::with_value(format!("{fixed}{shared}"));
        self.field.end();
    }

    fn candidates(&self, fixed: &str, stem: &str) -> Vec<String> {
        match self.completion {
            Completion::None | Completion::Places(_) => Vec::new(),
            Completion::Paths => {
                let directory = expand_tilde(if fixed.is_empty() { "./" } else { fixed });
                let Ok(entries) = std::fs::read_dir(&directory) else {
                    return Vec::new();
                };
                entries
                    .flatten()
                    .filter_map(|entry| {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if !name.starts_with(stem) {
                            return None;
                        }
                        // A trailing slash on a directory means one Tab gets you
                        // into it rather than up to its edge.
                        let directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
                        Some(if directory { format!("{name}/") } else { name })
                    })
                    .collect()
            }
        }
    }

    /// Draw the prompt over the middle of `screen`.
    ///
    /// `screen` is the whole terminal rather than the calling panel's slice of
    /// it. A dialog confined to its panel is as narrow as the panel, which for
    /// the agenda meant a long path scrolled inside about forty columns.
    pub fn render(&self, frame: &mut ratatui::Frame, screen: Rect, theme: &Theme) {
        let listed = self.matches();
        let rows = u16::try_from(listed.len().min(LIST_ROWS)).unwrap_or(0);

        let width = screen.width.clamp(20, 64);
        let height = 3 + rows + crate::frame::FRAME_HEIGHT;
        let popup = crate::frame::centred(screen, width, height);

        // Room for the text, inside the border and its padding.
        let inner = usize::from(width).saturating_sub(usize::from(crate::frame::FRAME_WIDTH));
        let (visible, cursor) = self.field.visible(inner);

        let mut lines = vec![Line::from(Span::styled(
            visible,
            Style::default().fg(theme.text),
        ))];

        // The list, if there is one, between the field and the help line.
        let city_width = listed
            .iter()
            .map(|p| p.city.len())
            .max()
            .unwrap_or(0)
            .min(18);
        // `enumerate` before `skip`, so `index` is the row's place in the whole
        // list and can be compared against `selected`. Enumerating after
        // skipping restarts the count at zero and puts the highlight on the top
        // visible row whatever is actually selected.
        for (index, place) in listed
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(usize::from(rows))
        {
            let here = index == self.selected;
            lines.push(Line::from(vec![
                Span::styled(
                    if here { "▸ " } else { "  " },
                    Style::default().fg(theme.accent),
                ),
                Span::styled(
                    format!("{:<city_width$}  ", place.city),
                    if here {
                        Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text)
                    },
                ),
                // The identifier is shown as well as the city, because it is
                // what ends up in your config and in zones.toml — picking
                // Seattle and finding `America/Los_Angeles` written down later
                // should not be a surprise.
                Span::styled(place.tz, Style::default().fg(theme.muted)),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(match &self.error {
            Some(error) => Line::from(Span::styled(
                crate::grid::truncate(error, inner),
                Style::default().fg(theme.error),
            )),
            None if listed.len() > LIST_ROWS => Line::from(Span::styled(
                crate::grid::truncate(
                    &format!("{} of {} · {}", rows, listed.len(), self.help),
                    inner,
                ),
                Style::default().fg(theme.muted),
            )),
            None => Line::from(Span::styled(
                crate::grid::truncate(self.help, inner),
                Style::default().fg(theme.muted),
            )),
        });

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_focused))
                    .padding(Padding::horizontal(1))
                    .title_top(Line::from(Span::styled(
                        self.label,
                        Style::default()
                            .fg(theme.title)
                            .add_modifier(Modifier::BOLD),
                    ))),
            ),
            popup,
        );

        // A visible cursor, because a text field without one reads as a label.
        //
        // Every step of this is saturating, and that is not defensive habit.
        // `centred` clamps the popup to the screen, so on a terminal one column
        // wide `popup.width` is 1 and the old `popup.x + popup.width - 2`
        // underflowed — a panic in a debug build and a wrap to 65535 in a
        // release one. Reachable by resizing the terminal while a prompt is
        // open, which is exactly when somebody would.
        let right = popup
            .x
            .saturating_add(popup.width.saturating_sub(crate::frame::FRAME_WIDTH));
        let x = popup
            .x
            .saturating_add(2)
            .saturating_add(u16::try_from(cursor).unwrap_or(u16::MAX));
        frame.set_cursor_position((x.min(right), popup.y.saturating_add(1)));
    }
}

/// Replace a leading `~` with the user's home directory.
pub fn expand_tilde(path: &str) -> std::path::PathBuf {
    let Some(rest) = path.strip_prefix('~') else {
        return std::path::PathBuf::from(path);
    };
    let Some(home) = dirs::home_dir() else {
        return std::path::PathBuf::from(path);
    };
    home.join(rest.trim_start_matches('/'))
}

/// The longest start every candidate shares, or `None` if there are none.
fn common_prefix(candidates: &[String]) -> Option<String> {
    let first = candidates.first()?;
    let mut shared = first.len();
    for other in &candidates[1..] {
        shared = shared.min(
            first
                .char_indices()
                .zip(other.char_indices())
                .take_while(|((_, a), (_, b))| a == b)
                .map(|((index, a), _)| index + a.len_utf8())
                .last()
                .unwrap_or(0),
        );
    }
    Some(first[..shared].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLACES: &[crate::zones::Place] = &[
        crate::zones::Place {
            city: "London",
            tz: "Europe/London",
        },
        crate::zones::Place {
            city: "Lisbon",
            tz: "Europe/Lisbon",
        },
        crate::zones::Place {
            city: "Seattle",
            tz: "America/Los_Angeles",
        },
        crate::zones::Place {
            city: "Tokyo",
            tz: "Asia/Tokyo",
        },
    ];

    fn prompt(value: &str) -> Prompt {
        Prompt::new("ZONE", "help", value, Completion::Places(PLACES))
    }

    fn press(prompt: &mut Prompt, code: KeyCode) -> Outcome {
        prompt.handle_key(KeyEvent::from(code))
    }

    #[test]
    fn esc_abandons_it() {
        assert_eq!(press(&mut prompt("x"), KeyCode::Esc), Outcome::Cancelled);
    }

    /// The headline case, and the reason a plain completion was not enough:
    /// Seattle keeps time in `America/Los_Angeles`, and nobody should have to
    /// know that before they can add a clock. Prefix matching cannot answer it.
    #[test]
    fn a_city_finds_the_zone_it_is_actually_in() {
        let mut p = prompt("");
        for c in "seattle".chars() {
            p.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let found: Vec<&str> = p.matches().iter().map(|place| place.tz).collect();
        assert_eq!(found, ["America/Los_Angeles"]);
    }

    /// And the identifier still works, for someone who knows it.
    #[test]
    fn the_identifier_matches_as_well_as_the_city() {
        let mut p = prompt("");
        for c in "europe/".chars() {
            p.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let found: Vec<&str> = p.matches().iter().map(|place| place.city).collect();
        assert_eq!(found, ["London", "Lisbon"]);
    }

    /// Choosing carries the label as well as the value, because they differ:
    /// someone who picked Seattle wants a clock that says Seattle.
    #[test]
    fn choosing_a_row_returns_the_city_as_the_label() {
        let mut p = prompt("seattle");
        assert_eq!(
            press(&mut p, KeyCode::Enter),
            Outcome::Chose {
                label: "Seattle",
                value: "America/Los_Angeles"
            }
        );
    }

    /// A reader who types a zone the list has never heard of knows something
    /// the list does not. Refusing them would make a convenience into a cage.
    #[test]
    fn text_matching_nothing_is_handed_back_rather_than_refused() {
        let mut p = prompt("  Antarctica/Troll  ");
        assert!(p.matches().is_empty(), "nothing in the list matches");
        assert_eq!(
            press(&mut p, KeyCode::Enter),
            Outcome::Submitted("Antarctica/Troll".into())
        );
    }

    /// The list draws a fixed window of rows. Moving the selection past the
    /// bottom of it used to keep moving an invisible cursor: the highlight
    /// vanished and the row that Enter would take was anybody's guess.
    #[test]
    fn the_window_follows_the_selection_off_the_bottom_and_back() {
        // The real table, not the four-entry fixture: with fewer places than
        // LIST_ROWS nothing ever scrolls and this test cannot fail. It did not,
        // the first time it was written.
        let real = crate::zones::PLACES;
        assert!(real.len() > LIST_ROWS, "there is something to scroll");
        let mut p = Prompt::new("ZONE", "help", "", Completion::Places(real));

        for _ in 0..real.len() {
            press(&mut p, KeyCode::Down);
        }
        assert!(
            p.selected >= p.offset && p.selected < p.offset + LIST_ROWS,
            "selected {} left the window at {}",
            p.selected,
            p.offset
        );

        assert!(p.offset > 0, "the window actually moved");

        for _ in 0..real.len() {
            press(&mut p, KeyCode::Up);
        }
        assert_eq!(p.selected, 0, "back to the first");
        assert_eq!(p.offset, 0, "and the window came back with it");
    }

    /// The window only moves when it has to, so the cursor travels across a
    /// stationary list rather than dragging it along one row at a time.
    #[test]
    fn the_window_stays_put_while_the_selection_is_inside_it() {
        let mut p = prompt("");
        press(&mut p, KeyCode::Down);
        assert_eq!(p.offset, 0, "second row is already visible");
    }

    #[test]
    fn the_selection_clamps_and_returns_to_the_top_when_the_list_narrows() {
        let mut p = prompt("");
        for _ in 0..20 {
            press(&mut p, KeyCode::Down);
        }
        assert_eq!(p.selected, PLACES.len() - 1, "clamped at the end");

        // Typing narrows the list under the cursor, so a stale index would
        // leave the highlight on something unrelated — or past the end.
        p.handle_key(KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(p.selected, 0);
    }

    /// A list is chosen from, not completed into — `Tab` would be a second
    /// way to do what the arrows already do, and a worse one.
    #[test]
    fn tab_does_nothing_when_the_prompt_offers_a_list() {
        let mut p = prompt("Lis");
        press(&mut p, KeyCode::Tab);
        assert_eq!(p.value(), "Lis");
    }

    /// The whole reason a rejection keeps the text: a path is long, and being
    /// sent back to an empty box to fix one character is how a feature stops
    /// getting used.
    #[test]
    fn a_rejected_answer_keeps_what_was_typed_and_the_next_key_clears_the_complaint() {
        let mut p = prompt("Europe/Lundon");
        p.reject("no such timezone");
        assert!(p.error.is_some());

        press(&mut p, KeyCode::Backspace);
        assert!(p.error.is_none(), "typing dismisses the complaint");
        assert_eq!(p.value(), "Europe/Lundo", "and the text is still there");
    }

    /// `centred` clamps the popup to the screen, so a terminal narrower than
    /// the dialog produced a popup narrower than the arithmetic assumed:
    /// `popup.x + popup.width - 2` underflowed at width 1. A panic in a debug
    /// build, a wrap to 65535 in a release one, and reachable by resizing the
    /// terminal while the prompt is open.
    #[test]
    fn a_terminal_too_small_for_the_dialog_does_not_bring_the_dashboard_down() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        for width in 1..8u16 {
            for height in 1..8u16 {
                let mut terminal =
                    Terminal::new(TestBackend::new(width, height)).expect("terminal");
                let p = Prompt::new("FILE", "help", "some/long/path.ics", Completion::Paths);
                terminal
                    .draw(|f| p.render(f, f.area(), &Theme::default()))
                    .unwrap_or_else(|e| panic!("{width}x{height} failed to draw: {e}"));
            }
        }
    }

    /// And the same for the variant that draws a list under the field.
    #[test]
    fn a_list_prompt_survives_a_tiny_terminal_too() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        for width in 1..8u16 {
            for height in 1..8u16 {
                let mut terminal =
                    Terminal::new(TestBackend::new(width, height)).expect("terminal");
                let p = Prompt::new("ZONE", "help", "", Completion::Places(crate::zones::PLACES));
                terminal
                    .draw(|f| p.render(f, f.area(), &Theme::default()))
                    .unwrap_or_else(|e| panic!("{width}x{height} failed to draw: {e}"));
            }
        }
    }

    /// Moving the text cursor is not editing the text. Left and Right used to
    /// fall through to the same arm as typing, which reset the list to the top:
    /// scroll to row thirty, press Left to fix a typo, lose your place.
    #[test]
    fn moving_the_text_cursor_does_not_throw_away_the_list_selection() {
        let mut p = Prompt::new("ZONE", "help", "", Completion::Places(crate::zones::PLACES));
        for _ in 0..15 {
            press(&mut p, KeyCode::Down);
        }
        let (selected, offset) = (p.selected, p.offset);
        assert!(selected > 0, "there is a selection to lose");

        press(&mut p, KeyCode::Left);
        assert_eq!(p.selected, selected, "Left moved the list");
        assert_eq!(p.offset, offset, "Left scrolled the list");

        press(&mut p, KeyCode::Right);
        assert_eq!(p.selected, selected, "Right moved the list");

        // Typing still does reset it, which is the behaviour that was correct.
        p.handle_key(KeyEvent::from(KeyCode::Char('z')));
        assert_eq!(p.selected, 0, "typing narrows the list and starts again");
    }

    #[test]
    fn completion_can_be_turned_off_entirely() {
        let mut p = Prompt::new("CITY", "help", "Bost", Completion::None);
        press(&mut p, KeyCode::Tab);
        assert_eq!(
            p.value(),
            "Bost",
            "a city name has nothing to complete from"
        );
    }
}
