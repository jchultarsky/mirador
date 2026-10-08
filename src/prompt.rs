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
use ratatui::widgets::{Clear, Paragraph};

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use std::cell::Cell;

use crate::picker::{Step, list_rows, window};
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

/// Rows of the list shown at once, on a terminal with room for them.
///
/// A ceiling, not the window: the window is however many of these the
/// terminal leaves the dialog, which only drawing knows. This was once the
/// window itself, moved in `handle_key`, and on a short terminal the cursor
/// walked into rows the dialog had no room to draw.
const LIST_ROWS: usize = 10;

/// The rows drawn under the list: a blank and the help line.
const TRAILER: usize = 2;

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
    /// the window and kept going, invisibly. Only drawing knows how many rows
    /// the terminal leaves the list, so it is drawing that moves it, as the
    /// two pickers' does.
    offset: Cell<usize>,
    /// How many rows of the list were drawn last time, which is how far a page
    /// moves — and when it is none, Enter has no city on screen to choose.
    page: Cell<usize>,
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
    ///
    /// `help` is ` · `-separated parts ending in the way out, and has a
    /// budget: the dialog is at most 64 wide, so 60 cells of text — less the
    /// `10 of 143 · ` count in front of it when a list scrolls. Over budget,
    /// parts drop whole and the last part goes last; see `help_line`.
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
            offset: Cell::new(0),
            page: Cell::new(LIST_ROWS),
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

        let len = self.listed.len();
        let page = self.page.get().max(1);
        let step = match key.code {
            KeyCode::Esc => return Outcome::Cancelled,
            KeyCode::Down => Step::Down(1),
            KeyCode::Up => Step::Up(1),
            KeyCode::PageDown => Step::Down(page),
            KeyCode::PageUp => Step::Up(page),
            KeyCode::Home if len > 0 => Step::First,
            KeyCode::End if len > 0 => Step::Last,
            KeyCode::Enter => {
                return match self.listed.get(self.selected) {
                    // The terminal left the list no row, so the city under
                    // the cursor is not on screen, and Enter does not choose
                    // what nobody can see. It waits for a taller terminal;
                    // Esc still leaves.
                    Some(_) if self.page.get() == 0 => Outcome::Editing,
                    Some(place) => Outcome::Chose {
                        label: place.city,
                        value: place.tz,
                    },
                    // Nothing matched, so the reader knows something the list
                    // does not. Hand back what they typed rather than
                    // refusing it.
                    None => Outcome::Submitted(self.field.trimmed().to_string()),
                };
            }
            _ => return self.edit(key),
        };
        // The window follows at the next draw, which is the one place that
        // knows how many rows it has.
        self.selected = step.apply(self.selected, len);
        Outcome::Editing
    }

    /// A key for the text rather than the list.
    fn edit(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
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
            // Home and End are absent because `handle_key` claims them
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
                self.offset.set(0);
                Outcome::Editing
            }
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

    /// The help row: the list's count when it scrolls, then the help's
    /// ` · `-separated parts, fitted to `width` by dropping whole parts.
    ///
    /// The help was ellipsised as prose, and the part that fell off was the
    /// last — `Esc cancels`, the way out, cut to `Esc can…`. Parts are keys
    /// and values, not prose (invariant 19), so they drop whole; and the last
    /// part is the way out, so it is the last to go: the parts before it drop
    /// first, from the right. Alone and still too wide, it is ellipsised.
    fn help_line(&self, count: Option<String>, width: u16, style: Style) -> Line<'static> {
        let parts = count
            .into_iter()
            .chain(self.help.split(" · ").map(str::to_string))
            .collect();
        crate::grid::assemble(way_out_last(parts, " · ", width, style), width)
    }

    /// Draw the prompt over the middle of `screen`.
    ///
    /// `screen` is the whole terminal rather than the calling panel's slice of
    /// it. A dialog confined to its panel is as narrow as the panel, which for
    /// the agenda meant a long path scrolled inside about forty columns.
    pub fn render(&self, frame: &mut ratatui::Frame, screen: Rect, theme: &Theme) {
        let listed = self.matches();
        let wanted = listed.len().min(LIST_ROWS);

        let width = screen.width.clamp(20, 64);
        let height = u16::try_from(1 + wanted + TRAILER)
            .unwrap_or(u16::MAX)
            .saturating_add(crate::frame::FRAME_HEIGHT);
        let popup = crate::frame::centred(screen, width, height);

        // What the terminal leaves under the field. The list gives way first,
        // down to the row the cursor is on, then the blank above the help,
        // which is spacing, then the help. The help says how to leave, and
        // goes before nothing but the field and the city under the cursor:
        // with the city gone Enter would choose a row nobody can see, so
        // that row is kept, and Esc works whether its hint is drawn or not.
        // With no list the blank still goes before the help.
        let interior = usize::from(popup.height.saturating_sub(crate::frame::FRAME_HEIGHT));
        let under_field = interior.saturating_sub(1);
        let (rows, spaced) = if wanted == 0 {
            (0, under_field >= TRAILER)
        } else {
            let (rows, spaced) = list_rows(under_field, TRAILER);
            (rows.min(wanted), spaced)
        };
        let offset = window(self.selected, self.offset.get(), rows, listed.len());
        self.offset.set(offset);
        // The rows actually on screen: `list_rows` never offers fewer than
        // one, and under four rows that one falls outside the dialog.
        self.page.set(rows.min(under_field));

        // Room for the text, inside the border and its padding — of the popup
        // as drawn. `centred` clamps it to the screen, and measured from the
        // twenty columns asked for, a screen narrower than that cut the help
        // row's way out to `Esc can` with no mark.
        let inner = usize::from(popup.width).saturating_sub(usize::from(crate::frame::FRAME_WIDTH));
        let (visible, cursor) = self.field.visible(inner);

        let mut lines = vec![Line::from(Span::styled(
            visible,
            Style::default().fg(theme.text),
        ))];

        // The list, if there is one, between the field and the help line.
        //
        // The city column is measured in cells, never bytes (invariant 9), and
        // over the whole list rather than the rows on screen, so it holds still
        // as the list scrolls. Capped at eighteen, and at what the dialog
        // leaves beside the marker: the city is the name a row is chosen by,
        // so it is the last thing to give way, and says so with `…` when it
        // does.
        let city_width = listed
            .iter()
            .map(|p| crate::grid::display_width(p.city))
            .max()
            .unwrap_or(0)
            .min(18)
            .min(inner.saturating_sub(2));
        // The identifier column is decided once for the draw, like the city
        // column and over the same list (invariant 5): every row carries one
        // when the widest fits beside the cities, and none does otherwise.
        // Dropped row by row, Melbourne sat with no zone between two rows
        // that had theirs, which reads as a place without one or a column
        // that is broken (invariant 11), and the gaps moved as it scrolled.
        let tz_width = listed
            .iter()
            .map(|p| crate::grid::display_width(p.tz))
            .max()
            .unwrap_or(0);
        let with_tz = 2 + city_width + 2 + tz_width <= inner;
        let row_width = u16::try_from(inner).unwrap_or(u16::MAX);
        // `enumerate` before `skip`, so `index` is the row's place in the whole
        // list and can be compared against `selected`. Enumerating after
        // skipping restarts the count at zero and puts the highlight on the top
        // visible row whatever is actually selected.
        for (index, place) in listed.iter().enumerate().skip(offset).take(rows) {
            let here = index == self.selected;
            let city = crate::grid::truncate(place.city, city_width);
            let pad = city_width.saturating_sub(crate::grid::display_width(&city));
            let mut parts = vec![vec![
                Span::styled(
                    if here { "▸ " } else { "  " },
                    Style::default().fg(theme.accent),
                ),
                Span::styled(
                    format!("{city}{:pad$}", ""),
                    if here {
                        Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text)
                    },
                ),
            ]];
            // The identifier is shown as well as the city, because it is what
            // ends up in your config and in zones.toml — picking Seattle and
            // finding `America/Los_Angeles` written down later should not be a
            // surprise. That makes it a value, so it is drawn whole or not at
            // all (invariant 19): the terminal cut it at the dialog's edge,
            // and `Australia/Melbou` is not a zone. `with_tz` already knows
            // it fits; `assemble` is the guard if that ever drifts.
            if with_tz {
                parts.push(vec![Span::styled(
                    format!("  {}", place.tz),
                    Style::default().fg(theme.muted),
                )]);
            }
            lines.push(crate::grid::assemble(parts, row_width));
        }

        if spaced {
            lines.push(Line::from(""));
        }
        lines.push(match &self.error {
            Some(error) => Line::from(Span::styled(
                crate::grid::truncate(error, inner),
                Style::default().fg(theme.error),
            )),
            None => self.help_line(
                (listed.len() > rows).then(|| format!("{rows} of {}", listed.len())),
                row_width,
                Style::default().fg(theme.muted),
            ),
        });

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines).block(crate::frame::dialog_block(theme, self.label, popup.width)),
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
        //
        // The furthest the caret goes is the last cell inside the padding,
        // three from the popup's right edge: past the border and the padding
        // there. That is the cell `visible` keeps free for it once the text
        // fills the field. Clamped one further in, at the inner width
        // measured from the border, it sat on the last character typed.
        let right = popup.x.saturating_add(popup.width.saturating_sub(3));
        let x = popup
            .x
            .saturating_add(2)
            .saturating_add(u16::try_from(cursor).unwrap_or(u16::MAX));
        frame.set_cursor_position((x.min(right), popup.y.saturating_add(1)));
    }
}

/// Key hints ending in the way out, as `grid::assemble` parts that fit
/// `width`: whole parts drop, and the last part is the last to go.
///
/// The plain-text form of [`crate::grid::way_out_last`], every part and its
/// `separator` in one `style`. A prompt's help and the task and note forms'
/// key rows fit this way.
pub(crate) fn way_out_last(
    parts: Vec<String>,
    separator: &str,
    width: u16,
    style: Style,
) -> Vec<Vec<Span<'static>>> {
    crate::grid::way_out_last(
        parts
            .into_iter()
            .map(|part| vec![Span::styled(part, style)])
            .collect(),
        &Span::styled(separator.to_string(), style),
        width,
    )
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
    use crate::store::testing::TempDir;

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

    /// The prompt drawn on a `width` by `height` terminal, as text.
    fn draw(p: &Prompt, width: u16, height: u16) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| p.render(f, f.area(), &Theme::default()))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The fix the theme picker and the panel picker had, in the third
    /// dialog with a list cursor. The zone list kept a fixed ten-row window in
    /// `handle_key`, inside a dialog fifteen rows tall: on a shorter terminal
    /// the bottom rows were cut, `Esc cancels` first, while the cursor walked
    /// on into them, and Enter added a zone nobody could see. Swept over
    /// every height with the cursor taken to the end and back a row at a
    /// time, because the window moving down and moving up are different
    /// arithmetic — and Enter is asked which city it would choose, which
    /// must be the one drawn beside the marker.
    ///
    /// From height 1, because under four rows the terminal leaves no row for
    /// the list at all, and Enter there added a city nobody could see. It
    /// waits instead, and Esc still leaves.
    #[test]
    fn the_row_under_the_cursor_is_drawn_at_every_height() {
        let places = crate::zones::PLACES;
        for height in 1..=30u16 {
            let mut p = Prompt::new(
                "ZONE",
                "Enter adds · Esc cancels",
                "",
                Completion::Places(places),
            );
            let visit = |p: &mut Prompt| {
                let screen = draw(p, 64, height);
                match press(p, KeyCode::Enter) {
                    Outcome::Chose { label, .. } => assert!(
                        screen.contains(&format!("▸ {label}")),
                        "Enter chooses {label} at height {height} and it is not drawn:\n{screen}"
                    ),
                    // Nothing chosen is only right where nothing is drawn:
                    // the frame and the field make four rows with one of list.
                    Outcome::Editing => assert!(
                        height < 4 && !screen.contains('▸'),
                        "Enter chose nothing at height {height}:\n{screen}"
                    ),
                    other => panic!("Enter at height {height} gave {other:?}:\n{screen}"),
                }
                // The frame, the field, one row of list and the help line
                // make five rows, and at five or more the way out is drawn:
                // the list gives way first, then the blank above the help,
                // then the help, before the city under the cursor.
                if height >= 5 {
                    assert!(
                        screen.contains("Esc cancels"),
                        "the help went before the list at height {height}:\n{screen}"
                    );
                }
            };
            press(&mut p, KeyCode::End);
            visit(&mut p);
            for _ in 0..places.len() {
                press(&mut p, KeyCode::Up);
                visit(&mut p);
            }
            // Enter waiting is not a trap: the way out works at every height.
            assert_eq!(press(&mut p, KeyCode::Esc), Outcome::Cancelled);
        }
    }

    /// A page is the rows drawn, the `N` in `N of 143`, and not the ten a tall
    /// terminal has room for — which on a short one jumped the cursor past
    /// rows nobody had seen.
    #[test]
    fn a_page_moves_by_the_rows_drawn() {
        let places = crate::zones::PLACES;
        let mut p = Prompt::new("ZONE", "Esc cancels", "", Completion::Places(places));
        let screen = draw(&p, 64, 8);
        let shown: usize = screen
            .split(&format!(" of {}", places.len()))
            .next()
            .and_then(|head| head.split_whitespace().last())
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("no count:\n{screen}"));
        assert!(shown < LIST_ROWS, "the terminal is short enough to matter");
        press(&mut p, KeyCode::PageDown);
        assert_eq!(p.selected, shown, "down a page from the top");
        draw(&p, 64, 8);
        press(&mut p, KeyCode::PageDown);
        assert_eq!(p.selected, 2 * shown, "and another");
        draw(&p, 64, 8);
        press(&mut p, KeyCode::PageUp);
        assert_eq!(p.selected, shown, "and back up one");
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
            draw(&p, 64, 24);
        }
        let offset = p.offset.get();
        assert!(
            p.selected >= offset && p.selected < offset + LIST_ROWS,
            "selected {} left the window at {offset}",
            p.selected,
        );

        assert!(offset > 0, "the window actually moved");

        for _ in 0..real.len() {
            press(&mut p, KeyCode::Up);
            draw(&p, 64, 24);
        }
        assert_eq!(p.selected, 0, "back to the first");
        assert_eq!(p.offset.get(), 0, "and the window came back with it");
    }

    /// The window only moves when it has to, so the cursor travels across a
    /// stationary list rather than dragging it along one row at a time.
    #[test]
    fn the_window_stays_put_while_the_selection_is_inside_it() {
        let mut p = Prompt::new("ZONE", "help", "", Completion::Places(crate::zones::PLACES));
        press(&mut p, KeyCode::Down);
        draw(&p, 64, 24);
        assert_eq!(p.offset.get(), 0, "second row is already visible");

        // And one row up from the end, the end is still on screen.
        press(&mut p, KeyCode::End);
        draw(&p, 64, 24);
        let bottom = p.offset.get();
        press(&mut p, KeyCode::Up);
        draw(&p, 64, 24);
        assert_eq!(p.offset.get(), bottom, "the window followed the cursor up");
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

    /// Path completion, against a real directory: Tab extends what was typed
    /// to the longest start every match shares, a directory completes with
    /// its slash so the next Tab goes into it, and no match leaves the text
    /// alone. `candidates`, `common_prefix` and `expand_tilde` were all
    /// unexecuted by the suite.
    #[test]
    fn tab_completes_a_path_to_the_shared_prefix_and_marks_directories() {
        let dir = TempDir::new("complete");
        std::fs::create_dir_all(dir.join("alps")).unwrap();
        std::fs::write(dir.join("alpha.ics"), "").unwrap();
        std::fs::write(dir.join("alphabet.ics"), "").unwrap();
        let base = format!("{}/", dir.display());

        let mut p = Prompt::new("FILE", "help", &format!("{base}al"), Completion::Paths);
        press(&mut p, KeyCode::Tab);
        assert_eq!(
            p.value(),
            format!("{base}alp"),
            "the three matches share `alp`"
        );

        let mut p = Prompt::new("FILE", "help", &format!("{base}alph"), Completion::Paths);
        press(&mut p, KeyCode::Tab);
        assert_eq!(p.value(), format!("{base}alpha"), "two files share `alpha`");

        let mut p = Prompt::new("FILE", "help", &format!("{base}alps"), Completion::Paths);
        press(&mut p, KeyCode::Tab);
        assert_eq!(
            p.value(),
            format!("{base}alps/"),
            "a directory completes into itself"
        );

        let mut p = Prompt::new("FILE", "help", &format!("{base}zzz"), Completion::Paths);
        press(&mut p, KeyCode::Tab);
        assert_eq!(
            p.value(),
            format!("{base}zzz"),
            "nothing matching changes nothing"
        );
    }

    #[test]
    fn a_tilde_means_home_and_nothing_else_is_touched() {
        let home = dirs::home_dir().expect("a home directory");
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("~/cal.ics"), home.join("cal.ics"));
        assert_eq!(
            expand_tilde("/abs/cal.ics"),
            std::path::PathBuf::from("/abs/cal.ics")
        );
        assert_eq!(
            expand_tilde("rel/cal.ics"),
            std::path::PathBuf::from("rel/cal.ics")
        );
        assert_eq!(expand_tilde(""), std::path::PathBuf::from(""));
    }

    #[test]
    fn the_common_prefix_is_measured_in_characters_not_bytes() {
        let s = |items: &[&str]| items.iter().map(|i| (*i).to_string()).collect::<Vec<_>>();
        assert_eq!(common_prefix(&s(&[])), None);
        assert_eq!(common_prefix(&s(&["alpha"])).as_deref(), Some("alpha"));
        assert_eq!(
            common_prefix(&s(&["alpha", "alphabet"])).as_deref(),
            Some("alpha")
        );
        assert_eq!(common_prefix(&s(&["a", "b"])).as_deref(), Some(""));
        // A cut inside `ü` would be a byte offset that is not a char boundary,
        // and slicing there panics.
        assert_eq!(common_prefix(&s(&["über", "übel"])).as_deref(), Some("übe"));
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

    /// The caret sits in the cell after the text, and stays there once the
    /// text is longer than the field. The field keeps a cell free for it, and
    /// the clamp that kept it inside the dialog stopped one cell short of
    /// that one, measuring the field from the border rather than from the
    /// text — so a full field drew the caret over its own last character,
    /// and the reader could not see what they had just typed.
    #[test]
    fn the_caret_follows_the_text_past_the_width_of_the_field() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (width, height) = (64u16, 10u16);
        // Either side of the field's sixty cells, and well past them.
        for typed in 50..=75 {
            let mut p = Prompt::new("FILE", "Enter keeps · Esc cancels", "", Completion::None);
            for _ in 0..typed {
                press(&mut p, KeyCode::Char('x'));
            }
            let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
            terminal
                .draw(|f| p.render(f, f.area(), &Theme::default()))
                .expect("draw");
            let caret = terminal.get_cursor_position().expect("a caret");
            let buffer = terminal.backend().buffer();
            let last = (0..width)
                .rev()
                .find(|&x| buffer[(x, caret.y)].symbol() == "x")
                .unwrap_or_else(|| panic!("nothing typed on the caret's row at {typed}"));
            assert_eq!(
                caret.x,
                last + 1,
                "with {typed} typed the caret is not in the cell after the last one drawn"
            );
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

    /// The list's rows as drawn, each beside the place it should show: the
    /// text between the borders, marker and padding trimmed off.
    fn drawn_list(
        p: &Prompt,
        width: u16,
        height: u16,
    ) -> Vec<(&'static crate::zones::Place, String)> {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| p.render(f, f.area(), &Theme::default()))
            .expect("draw");
        let rows = crate::widgets::testing::rows(terminal.backend().buffer());
        let Some(top) = rows.iter().position(|row| row.contains('╭')) else {
            return Vec::new();
        };
        let offset = p.offset.get();
        (0..p.page.get())
            .filter_map(|i| {
                let place = p.matches().get(offset + i)?;
                let inside = rows.get(top + 2 + i)?.split('│').nth(1)?;
                Some((
                    *place,
                    inside.trim().trim_start_matches('▸').trim().to_string(),
                ))
            })
            .collect()
    }

    /// Invariant 19: the identifier beside each city is a value — it is
    /// what ends up in `zones.toml` — so it is drawn whole or not at all.
    /// It was drawn at its natural width after the city column, and the
    /// terminal cut it at the dialog's edge with nothing to say so:
    /// `Australia/Melbou` at 40 columns, which is not a zone, and
    /// `America/Argentina/Buenos_Aires` on anything under 54. The city is
    /// the name the row is chosen by, so it gives way last and says so.
    #[test]
    fn the_zone_list_never_cuts_an_identifier_silently() {
        for filter in ["", "Argentina", "Australia"] {
            let mut whole = 0;
            for width in 1..=64u16 {
                let p = Prompt::new(
                    "ZONE",
                    "Enter adds · Esc cancels",
                    filter,
                    Completion::Places(crate::zones::PLACES),
                );
                let drawn = drawn_list(&p, width, 18);
                let mut with_tz = 0;
                for (place, row) in &drawn {
                    let mut parts = row.split("  ").map(str::trim).filter(|s| !s.is_empty());
                    let city = parts.next().unwrap_or_default();
                    let tz = parts.collect::<Vec<_>>().join("  ");
                    assert!(
                        tz.is_empty() || tz == place.tz,
                        "{} at width {width} is drawn beside {tz:?}, not {}",
                        place.city,
                        place.tz
                    );
                    // Nothing at all is no fragment: a dialog one column wide
                    // has no inside to draw a city in.
                    assert!(
                        city.is_empty() || city == place.city || city.ends_with('…'),
                        "{} at width {width} is drawn as {city:?}",
                        place.city
                    );
                    if tz == place.tz {
                        with_tz += 1;
                    }
                    if width == 64 {
                        assert_eq!(tz, place.tz, "at full width nothing gives way");
                    }
                }
                // Invariants 5 and 11: the column is decided once for the
                // draw. Dropped row by row, Melbourne sat with no zone
                // between Canberra and Brisbane with theirs, which reads as
                // a place with no zone or a column that is broken.
                assert!(
                    with_tz == 0 || with_tz == drawn.len(),
                    "{filter:?} at width {width}: {with_tz} of {} rows carry an identifier: \
                     {drawn:?}",
                    drawn.len()
                );
                whole += with_tz;
            }
            assert!(whole > 0, "no identifier was drawn for {filter:?}");
        }
    }

    /// The latent half of the same fault: the city column was sized in
    /// bytes and padded in characters, and a city is neither — a CJK name
    /// is three bytes and two cells a glyph. Every zone shipped is ASCII,
    /// which is why nothing showed, but `zones.rs` is a table anyone may
    /// extend.
    #[test]
    fn the_city_column_is_measured_in_cells() {
        const WIDE: &[crate::zones::Place] = &[
            crate::zones::Place {
                city: "東京都",
                tz: "Asia/Tokyo",
            },
            crate::zones::Place {
                city: "Paris",
                tz: "Europe/Paris",
            },
        ];
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (width, height) = (60u16, 12u16);
        let p = Prompt::new("ZONE", "help", "", Completion::Places(WIDE));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| p.render(f, f.area(), &Theme::default()))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let column = |tz: &str| {
            let len = u16::try_from(tz.len()).expect("short");
            (0..height).find_map(|y| {
                (0..=width - len).find(|&x| {
                    tz.char_indices().all(|(i, c)| {
                        let at = x + u16::try_from(i).expect("short");
                        buffer[(at, y)].symbol() == c.to_string()
                    })
                })
            })
        };
        let tokyo = column("Asia/Tokyo");
        assert!(tokyo.is_some(), "the identifier is drawn at all");
        assert_eq!(
            tokyo,
            column("Europe/Paris"),
            "the identifiers start in one column"
        );
        // And that column is where a city column measured in cells puts it:
        // the marker, the widest city in cells, the gap. Aligned is not
        // enough, since a column sized in bytes and padded in cells aligns
        // too, three cells further out — nine bytes for 東京都's six cells.
        let marker = (0..height)
            .find_map(|y| (0..width).find(|&x| buffer[(x, y)].symbol() == "▸"))
            .expect("the selected row's marker is drawn");
        let city = crate::grid::display_width("東京都").max(crate::grid::display_width("Paris"));
        assert_eq!(
            tokyo,
            Some(marker + 2 + u16::try_from(city).expect("short") + 2),
            "the identifier starts after a city column {city} cells wide"
        );
    }

    /// The prompt's last row as drawn, between the border and its padding.
    fn help_row(p: &Prompt, width: u16) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let height = 24;
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| p.render(f, f.area(), &Theme::default()))
            .expect("draw");
        let rows = crate::widgets::testing::rows(terminal.backend().buffer());
        let bottom = rows
            .iter()
            .rposition(|row| row.contains('╰'))
            .expect("a bottom border");
        rows[bottom - 1].trim().trim_matches('│').trim().to_string()
    }

    /// Every prompt the program opens, read from the source the way `docs.rs`
    /// reads the README: the label, the help, and whether a list is drawn —
    /// which puts a `10 of 143 · ` count in front of the help.
    fn shipped_prompts() -> Vec<(&'static str, &'static str, bool)> {
        let mut found = Vec::new();
        for source in [
            include_str!("widgets/weather.rs"),
            include_str!("widgets/clocks.rs"),
            include_str!("widgets/agenda.rs"),
        ] {
            for (at, _) in source.match_indices("prompt::Prompt::new(") {
                let call = &source[at..];
                let mut literals = call.split('"').skip(1).step_by(2);
                let label = literals.next().expect("a label");
                let help = literals.next().expect("a help line");
                // Every call names its completion after the value, so the
                // first one after the call's start is its own.
                let completion = call.find("Completion::").expect("a completion");
                let listed = call[completion..].starts_with("Completion::Places");
                found.push((label, help, listed));
            }
        }
        found
    }

    /// The help line had a budget nobody wrote down — the dialog is at most
    /// 64 wide, so 60 cells of text — and two prompts overran it at every
    /// terminal size: the weather prompt's help by three cells and the
    /// add-clock prompt's, behind its count, by eight. What fell off the end
    /// was `Esc cancels`, cut mid-word. Each shipped help now fits whole.
    #[test]
    fn every_shipped_prompts_help_is_drawn_whole() {
        let prompts = shipped_prompts();
        assert_eq!(prompts.len(), 4, "the scan found every prompt: {prompts:?}");
        for (label, help, listed) in prompts {
            let completion = if listed {
                Completion::Places(crate::zones::PLACES)
            } else {
                Completion::None
            };
            let p = Prompt::new("X", help, "", completion);
            let whole = if listed {
                format!("{LIST_ROWS} of {} · {help}", crate::zones::PLACES.len())
            } else {
                help.to_string()
            };
            // Exactly, not merely "has Esc and no `…`": with parts dropping
            // whole, an over-budget help loses `Enter saves` and still passes
            // that.
            assert_eq!(help_row(&p, 80), whole, "{label}");
        }
    }

    /// A help line too long for the dialog loses whole parts rather than
    /// being cut wherever the edge falls, and the way out is the last part
    /// to go: the parts before it drop first, from the right.
    #[test]
    fn a_long_help_drops_whole_parts_and_keeps_esc_longest() {
        let help = "Type to narrow · ↑↓ to choose · Enter adds · Esc cancels";
        let parts = [
            "Type to narrow",
            "↑↓ to choose",
            "Enter adds",
            "Esc cancels",
        ];
        let p = Prompt::new("ZONE", help, "", Completion::Places(crate::zones::PLACES));
        for width in 20..=80u16 {
            let row = help_row(&p, width);
            let pieces: Vec<&str> = row.split(" · ").collect();
            if pieces.len() == 1 {
                assert!(
                    row == "Esc cancels" || row.ends_with('…'),
                    "{width}: alone, the way out, or marked: {row:?}"
                );
                continue;
            }
            assert!(
                pieces[0].ends_with(" of 143") || parts.contains(&pieces[0]),
                "{width}: {row:?}"
            );
            assert_eq!(pieces.last(), Some(&"Esc cancels"), "{width}: {row:?}");
            for piece in &pieces[1..] {
                assert!(parts.contains(piece), "{width}: {piece:?} in {row:?}");
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
        draw(&p, 64, 24);
        let (selected, offset) = (p.selected, p.offset.get());
        assert!(selected > 0, "there is a selection to lose");

        press(&mut p, KeyCode::Left);
        draw(&p, 64, 24);
        assert_eq!(p.selected, selected, "Left moved the list");
        assert_eq!(p.offset.get(), offset, "Left scrolled the list");

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
