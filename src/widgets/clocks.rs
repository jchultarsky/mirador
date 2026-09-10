//! The chronometer: one large local clock, with secondary zones beneath it.
//!
//! The hierarchy is the point. A dashboard answers "what time is it" dozens of
//! times a day and "what time is it in Tokyo" rarely, so the local time is set
//! in block numerals and everything else is a labelled list. Reading the local
//! time should not require focusing on the panel at all.

use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use crate::config::ClocksConfig;
use crate::frame::{Binding, FRAME_HEIGHT, FRAME_WIDTH};
use crate::glyphs::{self, BigText};
use crate::grid::{Column, Grid};
use crate::panel::{KeyOutcome, Panel, RenderContext};

/// Keys this panel responds to.
///
/// **The order of the primaries decides which survive a narrow panel**, because
/// [`crate::frame::hint_line`] fills the border in order and stops at the first
/// that will not fit. The clock's border holds about four.
///
/// `Shift+↑↓` is a primary and sits above `d`, which is a deliberate swap made
/// after #109 shipped. Reordering is the thing that issue was filed for — "no
/// way to change order of TZ list via kbd" — and as an `extra` it appeared only
/// in the help overlay, so the border advertised `e edit` and said nothing about
/// the headline capability. The owner went looking for it in a released build
/// and concluded it had not been implemented. A key nobody can find is a key
/// that does not exist.
///
/// `a add` rather than `a add zone` for the same reason: the four together need
/// 42 cells against a budget of `width - 8`, which is 43 on the default layout.
/// The panel is a list of zones and the word buys nothing.
///
/// `d remove` is the one that drops at that width, and it is the right one to
/// lose: `d` deletes in the task, notes and watchlist panels too, so a reader who
/// has used any of them already knows it. `s` and the move keys are idiosyncratic
/// to this panel and guessable from nothing. A wider clock shows all five.
const BINDINGS: &[Binding] = &[
    Binding::primary("s", "seconds"),
    Binding::primary("a", "add"),
    Binding::primary("e", "edit"),
    Binding::primary("Shift+↑↓", "move"),
    Binding::primary("d", "remove"),
    Binding::extra("↑ / ↓", "select a clock"),
    Binding::extra("j / k", "select a clock"),
    Binding::extra("J / K", "move it"),
    Binding::extra("o", "show file path"),
];

/// The largest scale the numerals are ever drawn at. Past this a clock stops
/// being readable-from-across-the-room and starts being a poster.
const MAX_CLOCK_SCALE: u16 = 3;

/// Rows the numerals occupy at that scale: glyphs are five rows tall at 1.
const BIG_CLOCK_ROWS: u16 = 5 * MAX_CLOCK_SCALE;

/// Columns of the secondary zone list.
///
/// `time` holds the formatted time *and* the day marker, so it has to be wide
/// enough for both: `HH:MM:SS +1d` is twelve cells. It was nine, which fits the
/// time and truncates the marker away — and the marker is the half that carries
/// the warning, so the column silently dropped the only part of the row a reader
/// could get wrong. See the note on `day_marker` below.
pub(crate) const COLUMNS: &[Column] = &[
    Column::flex("zone", 1),
    Column::fixed("time", 12),
    Column::fixed("vs local", 9).right().drops_below(30),
];

/// A resolved clock: either a working timezone or the error from resolving it.
#[derive(Debug)]
struct Clock {
    label: String,
    zone: Result<TimeZone, String>,
}

/// The world clocks panel.
#[derive(Debug)]
pub struct ClocksPanel {
    config: ClocksConfig,
    /// The clock rendered large. Always the first configured zone.
    primary: Clock,
    /// Everything else, rendered as a labelled list.
    secondary: Vec<Clock>,
    show_seconds: bool,
    /// The live list, which the panel edits. `[clocks].zones` seeds it once.
    zones: crate::zones::Zones,
    /// Which secondary clock is selected, for `d`. The primary is index 0 and
    /// cannot be selected, because it cannot be removed.
    selected: usize,
    /// The `a` or `e` dialog, while it is open.
    asking: Option<crate::prompt::Prompt>,
    /// Which entry `e` opened the dialog on, when it was `e` rather than `a`.
    ///
    /// The two dialogs are the same prompt, and this is what tells them apart
    /// on submit — an add appends, an edit replaces. `None` means `a`.
    editing: Option<usize>,
    status: Option<String>,
    /// The instant the last frame showed, in whichever unit is on screen.
    ///
    /// This panel is why the whole dashboard used to repaint four times a
    /// second: it asked for a 250ms tick and had no `tick` at all, so every one
    /// of them counted as a change. With `show_seconds = false` the visible
    /// content moves once a minute and idle CPU was identical either way.
    last_shown: Option<i64>,
}

impl ClocksPanel {
    /// Resolve every zone once, at construction.
    pub fn new(config: ClocksConfig, path: std::path::PathBuf) -> anyhow::Result<Self> {
        let zones = crate::zones::Zones::load(path, &config.zones)?;
        let mut clocks: Vec<Clock> = zones
            .zones()
            .iter()
            .map(|zone| Clock {
                label: if zone.label.is_empty() {
                    zone.timezone.clone()
                } else {
                    zone.label.clone()
                },
                zone: resolve_zone(&zone.timezone),
            })
            .collect();

        // A panel with no configured zones still shows local time rather than
        // an empty box: the clock is the one thing that should never be blank.
        let primary = if clocks.is_empty() {
            Clock {
                label: "Local".into(),
                zone: Ok(TimeZone::system()),
            }
        } else {
            clocks.remove(0)
        };

        let show_seconds = config.show_seconds;
        Ok(Self {
            config,
            primary,
            secondary: clocks,
            show_seconds,
            zones,
            selected: 1,
            asking: None,
            editing: None,
            status: None,
            last_shown: None,
        })
    }

    /// Rebuild the clocks from the zone list after it changes.
    fn reload(&mut self) {
        let mut clocks: Vec<Clock> = self
            .zones
            .zones()
            .iter()
            .map(|zone| Clock {
                label: if zone.label.is_empty() {
                    zone.timezone.clone()
                } else {
                    zone.label.clone()
                },
                zone: resolve_zone(&zone.timezone),
            })
            .collect();
        if !clocks.is_empty() {
            self.primary = clocks.remove(0);
        }
        self.secondary = clocks;
        // The cursor indexes the zone list, whose first entry is the big clock
        // and is not selectable — so the range is `1..=secondary.len()`, and
        // clamping against the *zone* count instead leaves it one past the end
        // after a removal, where `d` silently does nothing.
        self.selected = self.selected.clamp(1, self.secondary.len().max(1));
        // The zone list is what is on screen now, so a failure to write it has
        // to be visible: the clock would silently be gone next launch.
        self.zones.save_reporting();
        // Forces a redraw even when the minute has not changed.
        self.last_shown = None;
    }

    /// Move the selected clock one row up, keeping the cursor on it.
    ///
    /// The cursor follows the clock rather than staying put, because the reader
    /// is moving a *thing*: holding the key should walk one entry up the table,
    /// not shuffle a different entry on every press.
    fn move_selected_up(&mut self) {
        if self.zones.move_up(self.selected) {
            self.selected -= 1;
            self.reload();
        } else {
            // The only way to fail from a valid selection is trying to displace
            // the primary, so this says the same thing `d` does.
            self.status = Some("the big clock stays".into());
        }
    }

    /// Move the selected clock one row down, keeping the cursor on it.
    ///
    /// Silent at the bottom, unlike [`Self::move_selected_up`]: running out of
    /// list is obvious from the screen, where the rule protecting the big clock
    /// is not.
    fn move_selected_down(&mut self) {
        if self.zones.move_down(self.selected) {
            self.selected += 1;
            self.reload();
        }
    }

    /// Deal with a keypress while the add-or-edit prompt is open.
    fn handle_prompt_key(&mut self, key: KeyEvent) {
        let Some(prompt) = self.asking.as_mut() else {
            return;
        };
        match prompt.handle_key(key) {
            crate::prompt::Outcome::Editing => {}
            crate::prompt::Outcome::Cancelled => {
                self.asking = None;
                self.editing = None;
            }
            // Picked from the list: the city the reader recognised becomes
            // the clock's label, which is the whole reason the outcome carries
            // both. Nothing needs validating — the list only holds zones that
            // resolve, and a test holds it to that.
            crate::prompt::Outcome::Chose { label, value } => {
                let applied = match self.editing {
                    Some(index) => self.zones.edit(index, label, value),
                    None => self.zones.add(label, value),
                };
                if applied {
                    self.asking = None;
                    self.editing = None;
                    self.reload();
                } else if let Some(prompt) = self.asking.as_mut() {
                    prompt.reject("that clock is already on the panel");
                }
            }
            crate::prompt::Outcome::Submitted(answer) => {
                // `Label = Zone` if you want to name it, otherwise just the
                // zone and the city out of it becomes the label.
                let (label, timezone) = match answer.split_once('=') {
                    Some((label, zone)) => (label.trim(), zone.trim()),
                    None => ("", answer.trim()),
                };
                if resolve_zone(timezone).is_err() {
                    prompt.reject(format!("unknown timezone `{timezone}`"));
                    return;
                }
                let applied = match self.editing {
                    Some(index) => self.zones.edit(index, label, timezone),
                    None => self.zones.add(label, timezone),
                };
                if !applied {
                    prompt.reject("that clock is already on the panel");
                    return;
                }
                self.asking = None;
                self.editing = None;
                self.reload();
            }
        }
    }
}

/// `format` with its seconds removed, for when `s` has hidden them.
///
/// `s` is bound to "seconds", not "seconds on the big clock", and the panel is
/// one panel — a clock with no seconds sitting above a table that still has
/// them reads as the key not having worked.
///
/// The awkwardness is that `[clocks].time_format` is the user's, and can be
/// anything. Switching to a fixed seconds-less format would silently discard
/// their choice — someone who set `%I:%M:%S %p` would find `s` turning their
/// clock to 24-hour. So the seconds specifier is removed from *their* format,
/// along with the separator immediately before it, and everything else is left
/// alone.
///
/// A format with no seconds in it is returned unchanged, which is the right
/// answer rather than a special case: there is nothing for `s` to hide, and the
/// table simply looks the same either way.
fn without_seconds(format: &str) -> String {
    // Longest first: `%:S` and `%.f` would otherwise be half-matched by `%S`.
    for token in ["%:S", "%S", "%T"] {
        if let Some(at) = format.find(token) {
            let replacement = if token == "%T" { "%H:%M" } else { "" };
            // Take the separator with it, so `%H:%M:%S` does not leave `%H:%M:`.
            let mut start = at;
            if replacement.is_empty()
                && let Some(before) = format[..at].chars().next_back()
                && matches!(before, ':' | '.' | '-')
            {
                start -= before.len_utf8();
            }
            return format!(
                "{}{replacement}{}",
                &format[..start],
                &format[at + token.len()..]
            );
        }
    }
    format.to_string()
}

/// Look up an IANA zone, treating `local` as the system zone.
fn resolve_zone(name: &str) -> Result<TimeZone, String> {
    if name.eq_ignore_ascii_case("local") || name.is_empty() {
        return Ok(TimeZone::system());
    }
    TimeZone::get(name).map_err(|_| format!("unknown timezone `{name}`"))
}

/// Format a UTC offset as `+09:30`, which `Offset`'s own Display does not do.
///
/// The zone table shows offsets relative to the primary clock instead, so this
/// is kept for the absolute form a future detail view will want.
#[cfg_attr(not(test), allow(dead_code))]
fn format_offset(offset: jiff::tz::Offset) -> String {
    let total = offset.seconds();
    let sign = if total < 0 { '-' } else { '+' };
    let abs = total.abs();
    format!("{sign}{:02}:{:02}", abs / 3600, (abs % 3600) / 60)
}

/// The offset of `other` relative to the primary zone, as `+9h` or `+5h30`.
///
/// Relative offsets answer the question people actually have about a foreign
/// clock — are they ahead or behind me, and by how much — which a raw UTC
/// offset makes you compute yourself.
fn relative_offset(primary: jiff::tz::Offset, other: jiff::tz::Offset) -> String {
    let delta = i64::from(other.seconds()) - i64::from(primary.seconds());
    if delta == 0 {
        return "same".to_string();
    }
    let sign = if delta < 0 { '-' } else { '+' };
    let abs = delta.abs();
    let (hours, minutes) = (abs / 3600, (abs % 3600) / 60);
    if minutes == 0 {
        format!("{sign}{hours}h")
    } else {
        format!("{sign}{hours}h{minutes:02}")
    }
}

/// Which face the clock wears at this size: the text drawn in numerals, the
/// small seconds beside them if any, and the scale — `None` meaning plain text.
///
/// The ladder is full numerals, then short numerals with small seconds, then
/// plain text. The middle rung fits the short form against what is left after
/// the suffix's cells, not against the whole width; and when that fails the
/// ladder goes to plain text *with* the seconds rather than to numerals
/// without them, because `s` is on and a clock that dropped its seconds by
/// itself reads as the key not having worked.
fn choose_face(
    show_seconds: bool,
    full: &str,
    short: &str,
    seconds: &str,
    width: u16,
    rows: u16,
) -> (String, Option<String>, Option<u16>) {
    let fits =
        |text: &str, columns: u16| glyphs::fitting_scale(text, columns, rows, MAX_CLOCK_SCALE);
    if !show_seconds {
        return (short.to_string(), None, fits(short, width));
    }
    if let Some(scale) = fits(full, width) {
        return (full.to_string(), None, Some(scale));
    }
    let suffix_cells = u16::try_from(seconds.chars().count()).unwrap_or(0) + 1;
    if let Some(scale) = fits(short, width.saturating_sub(suffix_cells)) {
        return (short.to_string(), Some(seconds.to_string()), Some(scale));
    }
    (full.to_string(), None, None)
}

impl Panel for ClocksPanel {
    fn title(&self) -> String {
        "Clock".to_string()
    }

    fn counter(&self) -> Option<String> {
        let zone = self.primary.zone.as_ref().ok()?;
        Some(
            jiff::Timestamp::now()
                .to_zoned(zone.clone())
                .strftime("%Z")
                .to_string(),
        )
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn max_width(&self) -> Option<u16> {
        // The numerals stop growing at scale 3, so past the width that
        // `HH:MM:SS` needs there the clock is the same size with more blank
        // around it. Wide enough to matter, though: below this the panel falls
        // back to plain text, which is the one thing this panel exists not to
        // do. It is a taker of surplus far longer than most panels.
        Some(glyphs::width_of("00:00:00", MAX_CLOCK_SCALE) + FRAME_WIDTH)
    }

    fn max_height(&self) -> Option<u16> {
        // Numerals at their largest, the date beneath, then the zone table:
        // a blank separator, a header, and a row per zone. Past this the panel
        // is centred numerals with a growing void underneath them.
        let date = u16::from(!self.config.date_format.is_empty());
        let zones = if self.secondary.is_empty() {
            0
        } else {
            u16::try_from(self.secondary.len()).unwrap_or(0) + 2
        };
        Some(BIG_CLOCK_ROWS + date + zones + FRAME_HEIGHT)
    }

    fn refresh_interval(&self) -> std::time::Duration {
        // Often enough to land on the boundary promptly, and no more often than
        // the smallest unit on screen needs.
        if self.show_seconds {
            std::time::Duration::from_millis(250)
        } else {
            std::time::Duration::from_secs(1)
        }
    }

    fn tick(&mut self) -> bool {
        // Every zone's minute turns on the same instant — UTC offsets are whole
        // minutes, including the 30- and 45-minute ones — so one comparison
        // covers the big clock, the zone list and the date line together.
        let second = jiff::Timestamp::now().as_second();
        let unit = if self.show_seconds {
            second
        } else {
            second / 60
        };
        let moved = self.last_shown != Some(unit);
        self.last_shown = Some(unit);
        moved
    }

    fn overlay(&self) -> Option<&crate::prompt::Prompt> {
        self.asking.as_ref()
    }

    fn alert(&self) -> Option<crate::panel::Alert> {
        self.zones.last_error.as_ref().map(|why| {
            crate::panel::Alert::failing(format!("The world clocks could not be saved — {why}"))
        })
    }

    fn captures_input(&self) -> bool {
        self.asking.is_some()
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        if self.asking.is_some() {
            self.handle_prompt_key(key);
            return KeyOutcome::Consumed;
        }

        self.status = None;
        // The primary clock is index 0 and is not selectable, so the cursor
        // lives in 1..=secondary.len().
        let last = self.secondary.len();
        match key.code {
            KeyCode::Char('s') => self.show_seconds = !self.show_seconds,
            KeyCode::Char('a') => {
                self.editing = None;
                self.asking = Some(crate::prompt::Prompt::new(
                    "ADD A CLOCK",
                    "Type to narrow · ↑↓ to choose · Enter adds · Esc cancels",
                    "",
                    crate::prompt::Completion::Places(crate::zones::PLACES),
                ));
            }
            KeyCode::Char('e') => {
                // Pre-filled with what the entry already says, in the same
                // `Label = Zone` the add dialog accepts. Relabelling is the
                // common case, so the text you want to change is already there
                // rather than something you retype from scratch.
                if let Some(zone) = self.zones.zones().get(self.selected) {
                    self.editing = Some(self.selected);
                    self.asking = Some(crate::prompt::Prompt::new(
                        "EDIT THIS CLOCK",
                        "`Label = Zone` · Enter saves · Esc cancels",
                        &format!("{} = {}", zone.label, zone.timezone),
                        crate::prompt::Completion::Places(crate::zones::PLACES),
                    ));
                }
            }
            KeyCode::Char('o') => {
                self.status = Some(self.zones.path().display().to_string());
            }
            // Shift moves the clock rather than the cursor, which is the same
            // shape as `Ctrl+←→↑↓` resizing a panel rather than moving focus.
            // `J`/`K` do it too, because this panel already offers arrows and
            // `j`/`k` as equals for the selection and it would be strange for
            // only one of the pair to gain the modifier.
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => self.move_selected_up(),
            KeyCode::Char('K') => self.move_selected_up(),
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.move_selected_down();
            }
            KeyCode::Char('J') => self.move_selected_down(),
            KeyCode::Char('d') => {
                if self.zones.remove(self.selected) {
                    self.reload();
                } else {
                    self.status = Some("the big clock stays".into());
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = self.selected.saturating_add(1).min(last.max(1));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1).max(1);
            }
            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    fn remember(&self, state: &mut crate::state::UiState) {
        state.clocks_show_seconds = Some(self.show_seconds);
    }

    #[allow(clippy::too_many_lines)] // One panel, drawn top to bottom; the
    // sub-steps share so much local state that splitting them would mean
    // threading half a dozen parameters through private helpers.
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.width == 0 || area.height == 0 {
            return;
        }

        let now = jiff::Timestamp::now();
        let primary_zone = match &self.primary.zone {
            Ok(zone) => zone.clone(),
            Err(message) => {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        message.clone(),
                        Style::default().fg(theme.error),
                    )),
                    area,
                );
                return;
            }
        };
        let local = now.to_zoned(primary_zone);

        // Seconds are wanted, but "HH:MM:SS" at block size needs about 62
        // columns and the clock panel rarely has that. So: try the full string
        // large, and if it will not fit, set HH:MM large with the seconds
        // riding small at the baseline. The hour and minute stay readable from
        // across the room either way, which is the whole point of the panel.
        let full = local.strftime("%H:%M:%S").to_string();
        let short = local.strftime("%H:%M").to_string();
        let seconds = local.strftime("%S").to_string();

        // Budget the panel before sizing the clock. The zone table is the
        // reason this panel exists beyond telling the time, so it gets its
        // rows first and the numerals take what is left. Sizing the clock
        // first is what pushed the zones off the bottom.
        let date_rows = u16::from(!self.config.date_format.is_empty());
        let zone_rows = if self.secondary.is_empty() {
            0
        } else {
            // One header, one row per zone, one blank line to separate them
            // from the numerals.
            u16::try_from(self.secondary.len()).unwrap_or(0) + 2
        };
        let clock_budget = area.height.saturating_sub(date_rows + zone_rows).max(1);

        // Width and height together, inside `choose_face`: filtering a
        // width-only answer by height rejects instead of stepping down a scale,
        // and a *shorter* string earns a bigger one. That is how hiding the
        // seconds used to make the clock smaller rather than larger.
        // Small seconds ride beside the numerals and take cells of their own —
        // a space and two digits — so the short form is fitted against what is
        // left *after* them. It used to be fitted alone, and the pair was then
        // centred as a unit: at 40 columns the numerals fitted, the suffix did
        // not, and the terminal cut `26` down to `2`, a fragment of a value.
        // Found by the silent-clip sweep in `widgets`, the day it was written.
        // When even the pair does not fit, the ladder steps straight down to
        // plain text with the seconds intact rather than to numerals without
        // them: `s` is on, and a clock that dropped its seconds on its own
        // would read as the key not having worked (#106, the other way round).
        let (time_text, small_seconds, scale) = choose_face(
            self.show_seconds,
            &full,
            &short,
            &seconds,
            area.width,
            clock_budget,
        );
        let mut cursor = area.y;

        if let Some(scale) = scale {
            let big = BigText::new(&time_text, scale);
            // Reserve room for the small seconds so the pair stays centred as
            // a unit rather than the big block jumping when seconds appear.
            let suffix = small_seconds
                .as_ref()
                .map_or(0, |s| u16::try_from(s.chars().count()).unwrap_or(0) + 1);
            let total = big.width + suffix;
            let x = area.x + (area.width.saturating_sub(total)) / 2;

            for (index, row) in big.rows.iter().enumerate() {
                let y = area.y + u16::try_from(index).unwrap_or(0);
                if y >= area.y + area.height {
                    break;
                }
                frame.render_widget(
                    Paragraph::new(Span::styled(row.clone(), Style::default().fg(theme.accent))),
                    Rect::new(x, y, big.width.min(area.width), 1),
                );
            }

            if let Some(seconds) = &small_seconds {
                // Sat on the baseline of the big digits, dimmer, so it reads as
                // a subscript rather than as another number.
                let y = area.y + big.height.saturating_sub(1);
                let sx = x + big.width + 1;
                // The fit above guarantees the room; the clamp is what makes a
                // future mistake there show as missing seconds rather than as
                // a cut one, which is the failure a reader cannot see.
                let room = (area.x + area.width).saturating_sub(sx);
                if room >= suffix.saturating_sub(1) && y < area.y + area.height {
                    frame.render_widget(
                        Paragraph::new(Span::styled(
                            seconds.clone(),
                            Style::default().fg(theme.muted),
                        )),
                        Rect::new(sx, y, suffix.saturating_sub(1).min(room), 1),
                    );
                }
            }
            cursor += big.height;
        } else {
            // Too small for block digits at any scale: fall back to plain text
            // rather than clipping — and truncate the plain text too, since a
            // rect the width of the panel cuts `15:54:33` to `15:54:3` at seven
            // columns, which is a different time.
            let text = if self.show_seconds { full } else { short };
            let text = crate::grid::truncate(&text, usize::from(area.width));
            frame.render_widget(
                Paragraph::new(Span::styled(
                    text,
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )),
                Rect::new(area.x, cursor, area.width, 1),
            );
            cursor += 1;
        }

        // The date sits directly under the numerals in the utility face, so
        // the two read as one object rather than as two separate facts.
        //
        // Truncated here rather than left to the rect, and measured in cells
        // rather than in `chars()`. Both halves were wrong and the pair of them
        // produced the one silent cut on the dashboard: a `Paragraph` in a rect
        // narrower than its text is clipped by the terminal, which cannot tell
        // a value from a fragment, so `WEDNESDAY 09 SEPTEMBER` came out at 80
        // columns as `WEDNESDAY 09 SEPT` — a complete-looking abbreviated month
        // that is not what the day is called. Every other cut on the same
        // screen says `…`; this one did not. `date_format` is the reader's, so
        // it can hold any text at all, which is why `chars()` was the second
        // fault rather than a theoretical one.
        if cursor < area.y + area.height && !self.config.date_format.is_empty() {
            let date = glyphs::utility(&local.strftime(&self.config.date_format).to_string());
            let date = crate::grid::truncate(&date, usize::from(area.width));
            let width = u16::try_from(crate::grid::display_width(&date)).unwrap_or(0);
            let x = area.x + (area.width.saturating_sub(width)) / 2;
            frame.render_widget(
                Paragraph::new(Span::styled(
                    date,
                    Style::default()
                        .fg(theme.label)
                        .add_modifier(Modifier::BOLD),
                )),
                Rect::new(x, cursor, width.min(area.width), 1),
            );
            cursor += 1;
        }

        if self.secondary.is_empty() || cursor >= area.y + area.height {
            return;
        }

        // A blank line separates the numerals from the table, but only when
        // there is room for the whole table underneath it.
        let needed = u16::try_from(self.secondary.len()).unwrap_or(0) + 1;
        if (area.y + area.height).saturating_sub(cursor) > needed {
            cursor += 1;
        }
        let remaining = (area.y + area.height).saturating_sub(cursor);
        if remaining == 0 {
            return;
        }

        let grid = Grid::new(COLUMNS, area.width);
        let mut lines = vec![grid.header(theme)];

        // Once per draw rather than once per row: `s` governs the whole panel,
        // not just the numerals above this table.
        let time_format = if self.show_seconds {
            self.config.time_format.clone()
        } else {
            without_seconds(&self.config.time_format)
        };

        for (index, clock) in self.secondary.iter().enumerate() {
            // The cursor is over the zone list, whose first entry is zone 1.
            let here = ctx.focused && index + 1 == self.selected;
            // Marked by reversing the label rather than by a gutter arrow: the
            // table has three columns and no room to spare, and reversing is
            // what the picker already uses for the same idea.
            let label = if here {
                Style::default()
                    .fg(theme.text)
                    .add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(theme.text)
            };
            match &clock.zone {
                Ok(zone) => {
                    let zoned = now.to_zoned(zone.clone());
                    // A foreign clock on a different calendar day is the thing
                    // people actually get wrong, so it is called out.
                    let day_marker = match zoned.date().cmp(&local.date()) {
                        std::cmp::Ordering::Greater => " +1d",
                        std::cmp::Ordering::Less => " -1d",
                        std::cmp::Ordering::Equal => "",
                    };
                    let offset = if self.config.show_offset {
                        relative_offset(local.offset(), zoned.offset())
                    } else {
                        String::new()
                    };

                    lines.push(grid.row(&[
                        Span::styled(clock.label.clone(), label),
                        Span::styled(
                            format!("{}{day_marker}", zoned.strftime(&time_format)),
                            Style::default().fg(if day_marker.is_empty() {
                                theme.text
                            } else {
                                theme.warning
                            }),
                        ),
                        Span::styled(offset, Style::default().fg(theme.muted)),
                    ]));
                }
                Err(message) => lines.push(grid.row(&[
                    Span::styled(clock.label.clone(), label),
                    Span::styled(message.clone(), Style::default().fg(theme.error)),
                ])),
            }
        }

        frame.render_widget(
            Paragraph::new(lines),
            Rect::new(area.x, cursor, area.width, remaining),
        );

        // A failed write to the zone file has to be seen: the clock is on
        // screen now and would silently be gone at the next launch.
        if let Some(message) = self.status.as_ref().or(self.zones.last_error.as_ref())
            && area.height > 0
        {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    crate::grid::truncate(message, usize::from(area.width)),
                    Style::default().fg(theme.error),
                )),
                Rect::new(area.x, area.y + area.height - 1, area.width, 1),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClockZone;
    use jiff::tz::Offset;

    struct TempDir(std::path::PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Reported as #103: pressing `s` to hide the seconds made the time render
    /// in small text instead of block numerals.
    ///
    /// The cause was in `glyphs::fitting_scale`, which took only a width; the
    /// caller then *filtered* that answer by height, which rejects instead of
    /// stepping down a scale. `HH:MM` is narrower than `HH:MM:SS`, so it earned
    /// a bigger scale — and a bigger scale is `CELL_H` rows taller, so at some
    /// panel sizes it no longer fit the rows and fell all the way back to plain
    /// text. Hiding a character made the clock smaller.
    ///
    /// Swept rather than pinned at one size, because the original was found at
    /// 68x11 and nothing would have led anyone to try that size on purpose.
    #[test]
    fn hiding_the_seconds_never_shrinks_the_clock() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();

        let draws_block_numerals = |panel: &mut ClocksPanel, w: u16, h: u16| -> bool {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|frame| {
                    panel.render(
                        frame,
                        frame.area(),
                        RenderContext {
                            theme: &config.theme,
                            gradients: &gradients,
                            focused: true,
                            watch: &crate::watch::WatchLog::default(),
                        },
                    );
                })
                .unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..h).any(|y| {
                (0..w).any(|x| {
                    buffer
                        .cell((x, y))
                        .is_some_and(|cell| cell.symbol() == "\u{2588}")
                })
            })
        };

        let mut shrank = Vec::new();
        for width in 20..104u16 {
            for height in 6..24u16 {
                let (mut with, _a) = panel_from_named("secs-on", ClocksConfig::default());
                with.show_seconds = true;
                let (mut without, _b) = panel_from_named("secs-off", ClocksConfig::default());
                without.show_seconds = false;

                if draws_block_numerals(&mut with, width, height)
                    && !draws_block_numerals(&mut without, width, height)
                {
                    shrank.push((width, height));
                }
            }
        }

        assert!(
            shrank.is_empty(),
            "at {} panel sizes, hiding the seconds dropped the clock to plain \
             text; first few: {:?}",
            shrank.len(),
            &shrank[..shrank.len().min(6)]
        );
    }

    /// #106: `s` is bound to "seconds", and the panel is one panel — a clock
    /// with no seconds above a table that still has them reads as the key not
    /// having worked. The user's own `time_format` has to survive it, so the
    /// seconds specifier is removed from *their* format rather than swapped for
    /// a fixed one.
    #[test]
    fn hiding_the_seconds_takes_them_out_of_the_zone_table_too() {
        assert_eq!(without_seconds("%H:%M:%S"), "%H:%M");
        // A 12-hour format keeps being a 12-hour format.
        assert_eq!(without_seconds("%I:%M:%S %p"), "%I:%M %p");
        // `%T` is a whole time, so it becomes the seconds-less whole time.
        assert_eq!(without_seconds("%T"), "%H:%M");
        // Nothing to remove is not a special case.
        assert_eq!(without_seconds("%H:%M"), "%H:%M");
        assert_eq!(without_seconds(""), "");
        // The separator goes with it, rather than leaving a trailing colon.
        assert!(!without_seconds("%H:%M:%S").ends_with(':'));
        assert!(!without_seconds("%H.%M.%S").ends_with('.'));
    }

    /// The wiring, not just the helper. Breaking the call site while leaving
    /// `without_seconds` correct passed every other test here — the helper can
    /// be right and simply not be called.
    #[test]
    fn the_rendered_zone_table_loses_its_seconds_when_s_does() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();

        let screen = |panel: &mut ClocksPanel| -> String {
            let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
            terminal
                .draw(|frame| {
                    panel.render(
                        frame,
                        frame.area(),
                        RenderContext {
                            theme: &config.theme,
                            gradients: &gradients,
                            focused: true,
                            watch: &crate::watch::WatchLog::default(),
                        },
                    );
                })
                .unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..20)
                .map(|y| {
                    (0..60)
                        .filter_map(|x| buffer.cell((x, y)).map(|c| c.symbol().to_string()))
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        // A zone table needs at least one secondary clock to exist at all.
        let (mut panel, _guard) = panel_from_named("seconds-wiring", ClocksConfig::default());
        assert!(
            !panel.secondary.is_empty(),
            "the default config seeds secondary zones"
        );

        // `HH:MM:SS` in the table: two colons on a zone row.
        panel.show_seconds = true;
        let with = screen(&mut panel);
        let rows_with_two_colons = with
            .lines()
            .filter(|line| line.matches(':').count() >= 2)
            .count();
        assert!(
            rows_with_two_colons > 0,
            "with seconds on, some zone row shows HH:MM:SS\n{with}"
        );

        panel.show_seconds = false;
        let without = screen(&mut panel);
        let still_two = without
            .lines()
            .filter(|line| {
                // Ignore the big numerals, which are block glyphs, and the
                // footer; a zone row is one with a zone label on it.
                line.matches(':').count() >= 2 && !line.contains('\u{2588}')
            })
            .count();
        assert_eq!(
            still_two, 0,
            "with seconds off, no zone row should still carry them\n{without}"
        );
    }

    /// #107: the column holds the time *and* the day marker, and the marker is
    /// the half that carries the warning. At nine cells `02:43:48 +1d` was
    /// truncated to `02:43:48…`, so the one part a reader could get wrong was
    /// the part that never showed.
    #[test]
    fn the_day_marker_fits_beside_the_time_it_belongs_to() {
        let time = COLUMNS
            .iter()
            .find(|column| column.label == "time")
            .expect("there is a time column");
        let crate::grid::Width::Fixed(width) = time.width else {
            panic!("the time column is fixed width");
        };
        for marker in ["", " +1d", " -1d"] {
            let widest = format!("00:00:00{marker}");
            assert!(
                crate::grid::display_width(&widest) <= usize::from(width),
                "`{widest}` needs {} cells and the column is {width}",
                crate::grid::display_width(&widest)
            );
        }
    }

    /// The date line was the one thing on the dashboard that was cut without
    /// saying so. It was handed to a `Paragraph` in a rect narrower than the
    /// text, so the *terminal* did the cutting, and a terminal cannot tell a
    /// value from a fragment: `WEDNESDAY 09 SEPTEMBER` arrived at 80 columns as
    /// `WEDNESDAY 09 SEPT`, which reads as a whole abbreviated month.
    ///
    /// Two properties, because the old code got two things wrong. Anything cut
    /// has to say `…`, and nothing may be drawn wider than the space it was
    /// given — which the old `chars().count()` could not know, since
    /// `date_format` is the reader's and may hold any text at all.
    #[test]
    fn the_date_line_says_when_it_has_been_cut() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();

        // Literal text passes through `strftime` untouched, so the whole line
        // is known without depending on what today happens to be. The second
        // is double-width throughout: eight cells to six `chars()`, which is
        // the gap the old measurement fell into.
        for format in [
            "MIDWEEK LONG DATE LINE",
            "\u{6c34}\u{66dc}\u{65e5} \u{4e5d}\u{6708}",
        ] {
            let full = crate::glyphs::utility(format);
            let mut ever_cut = false;

            for width in 4..48u16 {
                let (mut panel, _guard) = panel_from_named(
                    "date-cut",
                    ClocksConfig {
                        date_format: format.to_string(),
                        ..ClocksConfig::default()
                    },
                );
                let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
                terminal
                    .draw(|frame| {
                        panel.render(
                            frame,
                            frame.area(),
                            RenderContext {
                                theme: &config.theme,
                                gradients: &gradients,
                                focused: true,
                                watch: &crate::watch::WatchLog::default(),
                            },
                        );
                    })
                    .unwrap();
                // Read the buffer rather than `TestBackend`'s `Display`, which
                // wraps every row in quotation marks — two cells that are not
                // on anybody's screen, and enough to fail a width assertion
                // that is doing its job.
                let buffer = terminal.backend().buffer().clone();
                let rows: Vec<String> = (0..buffer.area.height)
                    .map(|y| {
                        // Walk by each glyph's measured width, the way
                        // `link::linkify` has to: the cell a double-width glyph
                        // covers holds a space, indistinguishable from a real
                        // one, so reading every cell turns `水曜` into `水 曜 `
                        // and inflates the row by a cell per wide glyph.
                        let mut row = String::new();
                        let mut x = 0u16;
                        while x < buffer.area.width {
                            let Some(cell) = buffer.cell((x, y)) else {
                                break;
                            };
                            let symbol = cell.symbol();
                            row.push_str(symbol);
                            x += u16::try_from(crate::grid::display_width(symbol).max(1))
                                .unwrap_or(1);
                        }
                        row
                    })
                    .collect();

                // The date row is the one carrying the *first* character of the
                // format — one character, not two, because a cut short enough
                // to lose the second is exactly the case being tested and a
                // two-character head would quietly skip it. The zone rows are
                // excluded by their colons, and the numerals above are block
                // glyphs. A width too narrow to hold even the first character
                // draws no date at all, which is a row not drawn rather than a
                // row drawn wrong.
                let head: String = full.chars().take(1).collect();
                let Some(row) = rows
                    .iter()
                    .find(|line| line.contains(&head) && !line.contains(':'))
                else {
                    continue;
                };
                let drawn = row.trim();

                assert!(
                    crate::grid::display_width(drawn) <= usize::from(width),
                    "the date overflowed {width} cells: {drawn:?}"
                );
                if drawn == full {
                    continue;
                }
                ever_cut = true;
                assert!(
                    drawn.ends_with('\u{2026}'),
                    "the date was cut at {width} without saying so: {drawn:?}"
                );
            }

            assert!(
                ever_cut,
                "no width in the sweep actually cut `{full}`, so this proves nothing"
            );
        }
    }

    /// The small seconds beside the numerals are a value: `26`, not `2`. At
    /// 40 columns the numerals fitted and the suffix did not, and the terminal
    /// cut it — the fit was measured without the three cells the seconds take.
    ///
    /// Swept across widths and heights rather than pinned at 40, because 40 is
    /// only where it was noticed. Whatever follows the last block glyph on the
    /// numerals' baseline row is either nothing or both digits.
    #[test]
    fn the_small_seconds_are_drawn_whole_or_not_at_all() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();
        let mut seen_small_seconds = false;

        for width in 8..=100u16 {
            for height in 6..=14u16 {
                let (mut panel, _guard) = panel_from_named("small-secs", ClocksConfig::default());
                panel.show_seconds = true;
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| {
                        panel.render(
                            frame,
                            frame.area(),
                            RenderContext {
                                theme: &config.theme,
                                gradients: &gradients,
                                focused: true,
                                watch: &crate::watch::WatchLog::default(),
                            },
                        );
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer().clone();
                let rows: Vec<String> = (0..height)
                    .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
                    .collect();
                // The baseline is the last row that holds a block glyph.
                let Some(baseline) = rows.iter().rev().find(|r| r.contains('\u{2588}')) else {
                    continue;
                };
                let after = baseline.rsplit('\u{2588}').next().unwrap_or("").trim();
                if after.is_empty() {
                    continue;
                }
                seen_small_seconds = true;
                assert!(
                    after.len() == 2 && after.bytes().all(|b| b.is_ascii_digit()),
                    "at {width}x{height} the seconds beside the numerals read {after:?}: {baseline:?}"
                );
            }
        }
        assert!(
            seen_small_seconds,
            "no size in the sweep drew small seconds, so the property was never exercised"
        );
    }

    /// The middle rung of the face ladder is measured with the seconds' cells
    /// taken out. It used to fit the short form alone and then centre the pair,
    /// so at a width that held the numerals but not the suffix the seconds were
    /// cut by the terminal. The alternative — numerals without seconds while `s`
    /// is on — is refused too: the ladder goes to plain text with them intact.
    #[test]
    fn the_short_face_is_only_chosen_when_its_seconds_fit_beside_it() {
        let (full, short, seconds) = ("15:44:26", "15:44", "26");
        let rows = 6;
        let short_width = glyphs::width_of(short, 1);
        let suffix = 3; // a space and two digits

        // Room for the numerals and the seconds: the pair, at scale 1.
        let (text, small, scale) =
            choose_face(true, full, short, seconds, short_width + suffix, rows);
        assert_eq!(
            (text.as_str(), small.as_deref(), scale),
            (short, Some(seconds), Some(1))
        );

        // Room for the numerals alone: not numerals alone. Plain text keeps
        // every digit the reader asked for.
        let (text, small, scale) =
            choose_face(true, full, short, seconds, short_width + suffix - 1, rows);
        assert_eq!(
            (text.as_str(), small.as_deref(), scale),
            (full, None, None),
            "numerals that would cut or drop the seconds must give way to plain text"
        );

        // Seconds off: the short form fits the whole width, no suffix reserved.
        let (text, small, scale) = choose_face(false, full, short, seconds, short_width, rows);
        assert_eq!(
            (text.as_str(), small.as_deref(), scale),
            (short, None, Some(1))
        );
    }

    /// A panel seeded from `config`, with a zone file of its very own.
    ///
    /// It has to be its own: `write_atomic` creates the parent directory it is
    /// given, so a shared path is a shared *file*, and these tests all edit the
    /// zone list. A single `/nonexistent/zones.toml` looked fine on macOS and
    /// Linux — nothing can create `/nonexistent` without root, so every save
    /// failed harmlessly and every panel got the seed — and on Windows the
    /// runner happily made `C:\nonexistent`, after which each test loaded
    /// whatever the last one had written and three of them failed with a
    /// timezone none of them had asked for.
    fn panel_from_named(name: &str, config: ClocksConfig) -> (ClocksPanel, TempDir) {
        let dir = std::env::temp_dir().join(format!("mirador-zones-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test directory");
        let panel = ClocksPanel::new(config, dir.join("zones.toml")).expect("builds from a seed");
        (panel, TempDir(dir))
    }

    fn zone(label: &str, tz: &str) -> ClockZone {
        ClockZone {
            label: label.into(),
            timezone: tz.into(),
        }
    }

    fn press(panel: &mut ClocksPanel, code: KeyCode) {
        panel.handle_key(KeyEvent::from(code));
    }

    fn labels(panel: &ClocksPanel) -> Vec<String> {
        panel.secondary.iter().map(|c| c.label.clone()).collect()
    }

    /// Removing the last clock in the list used to leave the cursor one past
    /// the end, where `d` did nothing at all and the panel looked wedged.
    #[test]
    fn the_cursor_stays_on_a_real_clock_after_a_removal() {
        let (mut panel, _guard) = panel_from_named(
            "the_cursor_stays_on_a_real_c",
            ClocksConfig {
                zones: vec![
                    zone("Home", "local"),
                    zone("UTC", "UTC"),
                    zone("Tokyo", "Asia/Tokyo"),
                ],
                ..ClocksConfig::default()
            },
        );

        // Move to the last secondary clock and remove it.
        press(&mut panel, KeyCode::Down);
        assert_eq!(panel.selected, 2);
        press(&mut panel, KeyCode::Char('d'));
        assert_eq!(labels(&panel), ["UTC"], "Tokyo is gone");

        // The cursor must now be on UTC, not past it, so `d` works again.
        assert_eq!(panel.selected, 1);
        press(&mut panel, KeyCode::Char('d'));
        assert!(labels(&panel).is_empty(), "and UTC goes too");
    }

    /// The primary is index 0 and never selectable, so the last `d` on an
    /// empty list has to say why nothing happened rather than look broken.
    #[test]
    fn the_big_clock_survives_and_says_so() {
        let (mut panel, _guard) = panel_from_named(
            "the_big_clock_survives_and_s",
            ClocksConfig {
                zones: vec![zone("Home", "local")],
                ..ClocksConfig::default()
            },
        );
        press(&mut panel, KeyCode::Char('d'));
        assert_eq!(panel.primary.label, "Home", "still there");
        assert!(panel.status.is_some(), "and the panel says why");
    }

    #[test]
    fn a_zone_that_does_not_resolve_is_refused_with_the_prompt_left_open() {
        let (mut panel, _guard) =
            panel_from_named("a_zone_that_does_not_resolve", ClocksConfig::default());
        press(&mut panel, KeyCode::Char('a'));
        for c in "Mars/Olympus".chars() {
            panel.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let before = labels(&panel);
        press(&mut panel, KeyCode::Enter);

        assert!(panel.asking.is_some(), "the prompt stays open to be fixed");
        assert_eq!(labels(&panel), before, "and nothing was added");
    }

    /// Picking from the list names the clock after the city you recognised,
    /// and the `Label = Zone` form still works for anyone typing a zone the
    /// list does not carry.
    #[test]
    fn a_clock_can_be_added_with_or_without_a_label() {
        let (mut panel, _guard) = panel_from_named(
            "a_clock_can_be_added_with_or",
            ClocksConfig {
                zones: vec![zone("Home", "local")],
                ..ClocksConfig::default()
            },
        );

        press(&mut panel, KeyCode::Char('a'));
        for c in "Bengaluru".chars() {
            panel.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        press(&mut panel, KeyCode::Enter);
        assert_eq!(
            labels(&panel),
            ["Bengaluru"],
            "the city you picked, not the one in the identifier"
        );

        press(&mut panel, KeyCode::Char('a'));
        for c in "HQ = Europe/Berlin".chars() {
            panel.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        press(&mut panel, KeyCode::Enter);
        assert_eq!(labels(&panel), ["Bengaluru", "HQ"], "or named by you");
    }

    #[test]
    fn local_and_utc_always_resolve() {
        assert!(resolve_zone("local").is_ok());
        assert!(resolve_zone("LOCAL").is_ok());
        assert!(resolve_zone("").is_ok());
        assert!(resolve_zone("UTC").is_ok());
    }

    #[test]
    fn unknown_zones_report_the_name_instead_of_panicking() {
        let err = resolve_zone("Mars/Olympus").expect_err("must fail");
        assert!(err.contains("Mars/Olympus"), "got: {err}");
    }

    #[test]
    fn offsets_format_with_sign_and_padding() {
        assert_eq!(format_offset(Offset::from_seconds(0).unwrap()), "+00:00");
        assert_eq!(
            format_offset(Offset::from_seconds(9 * 3600).unwrap()),
            "+09:00"
        );
        assert_eq!(
            format_offset(Offset::from_seconds(-5 * 3600).unwrap()),
            "-05:00"
        );
        // Half-hour and quarter-hour zones must not lose their minutes.
        assert_eq!(
            format_offset(Offset::from_seconds(5 * 3600 + 1800).unwrap()),
            "+05:30"
        );
        assert_eq!(
            format_offset(Offset::from_seconds(5 * 3600 + 2700).unwrap()),
            "+05:45"
        );
    }

    #[test]
    fn relative_offsets_are_expressed_against_the_primary_clock() {
        let utc = Offset::from_seconds(0).unwrap();
        let tokyo = Offset::from_seconds(9 * 3600).unwrap();
        let new_york = Offset::from_seconds(-4 * 3600).unwrap();
        let kolkata = Offset::from_seconds(5 * 3600 + 1800).unwrap();

        assert_eq!(relative_offset(utc, tokyo), "+9h");
        assert_eq!(relative_offset(utc, new_york), "-4h");
        assert_eq!(relative_offset(utc, utc), "same");
        assert_eq!(relative_offset(utc, kolkata), "+5h30");
        // Relative to New York rather than to UTC.
        assert_eq!(relative_offset(new_york, tokyo), "+13h");
    }

    #[test]
    fn the_first_zone_becomes_the_large_clock() {
        let (panel, _guard) = panel_from_named(
            "the_first_zone_becomes_the_l",
            ClocksConfig {
                zones: vec![zone("Home", "UTC"), zone("Tokyo", "Asia/Tokyo")],
                ..Default::default()
            },
        );
        assert_eq!(panel.primary.label, "Home");
        assert_eq!(panel.secondary.len(), 1);
        assert_eq!(panel.secondary[0].label, "Tokyo");
    }

    #[test]
    fn an_empty_zone_list_still_shows_local_time() {
        let (panel, _guard) = panel_from_named(
            "an_empty_zone_list_still_sho",
            ClocksConfig {
                zones: Vec::new(),
                ..Default::default()
            },
        );
        assert!(panel.primary.zone.is_ok());
        assert!(panel.secondary.is_empty());
    }

    #[test]
    fn a_label_falls_back_to_the_zone_name() {
        let (panel, _guard) = panel_from_named(
            "a_label_falls_back_to_the_zo",
            ClocksConfig {
                zones: vec![zone("", "UTC"), zone("", "Asia/Tokyo")],
                ..Default::default()
            },
        );
        assert_eq!(panel.primary.label, "UTC");
        assert_eq!(panel.secondary[0].label, "Asia/Tokyo");
    }

    #[test]
    fn s_toggles_seconds_and_is_consumed() {
        let (mut panel, _guard) =
            panel_from_named("s_toggles_seconds_and_is_con", ClocksConfig::default());
        let before = panel.show_seconds;
        let outcome = panel.handle_key(KeyEvent::new(
            KeyCode::Char('s'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(outcome, KeyOutcome::Consumed);
        assert_ne!(panel.show_seconds, before);
    }

    #[test]
    fn other_keys_fall_through_to_the_application() {
        let (mut panel, _guard) =
            panel_from_named("other_keys_fall_through_to_t", ClocksConfig::default());
        let outcome = panel.handle_key(KeyEvent::new(
            KeyCode::Tab,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(outcome, KeyOutcome::Ignored);
    }

    /// The move keys have to reach the *border*, not just the help overlay.
    ///
    /// They shipped in 0.17.0 as `extra` bindings, which put them in `?` and
    /// nowhere else — so the border advertised `e edit` and said nothing about
    /// reordering, which is the thing #109 was filed for. The owner went looking
    /// for the feature in a released build and concluded it had not been
    /// implemented. A key nobody can find is a key that does not exist.
    ///
    /// Asserted against the width the default layout actually gives this panel,
    /// because `hint_line` fills the border in order and stops at the first
    /// binding that will not fit: this passes or fails on the *order* and the
    /// *wording* of `BINDINGS`, not merely on the primary flag. Lengthening any
    /// label above `move` pushes it off, which is how it was invisible before.
    #[test]
    fn the_move_keys_reach_the_border_at_the_default_width() {
        // The clock's frame measures 52 cells at 150x42 — checked by rendering
        // it — and `render_frame` spends `width - 8` of that on hints.
        const BUDGET: u16 = 52 - 8;
        let theme = crate::theme::Theme::default();
        let hint =
            crate::frame::hint_line(BINDINGS, &theme, BUDGET).expect("the clock has primaries");
        let drawn: String = hint.spans.iter().map(|s| s.content.as_ref()).collect();

        // The move key, not a substring of another hint. The first version of
        // this test asked whether the border contained "move" — and `remove`
        // contains `move`, so it passed with the binding demoted back to an
        // extra. Match the key itself.
        assert!(
            drawn.contains("Shift+↑↓"),
            "reordering is invisible on the border: {drawn:?}"
        );
        assert!(
            drawn.contains("a add") && drawn.contains("e edit"),
            "adding and editing must survive alongside it: {drawn:?}"
        );
    }

    /// And the order is the thing that decides it, independently of any width.
    /// `move` must be reached before `remove`, or a narrow panel spends its last
    /// slot on the key every other list panel already teaches.
    #[test]
    fn move_is_offered_to_the_border_before_remove() {
        let primaries: Vec<&str> = BINDINGS
            .iter()
            .filter(|b| b.primary)
            .map(|b| b.action.as_ref())
            .collect();
        // `expect`, not a bare comparison. `position` returns `Option`, and
        // `None < Some(_)` is true — so comparing them directly passed when the
        // binding was not a primary at all, which is the exact regression this
        // is here to catch.
        let move_at = primaries
            .iter()
            .position(|a| *a == "move")
            .expect("`move` must be a primary, or it never reaches the border");
        let remove_at = primaries
            .iter()
            .position(|a| *a == "remove")
            .expect("`remove` is still a primary");
        assert!(
            move_at < remove_at,
            "move must come first; got {primaries:?}"
        );
    }
}
