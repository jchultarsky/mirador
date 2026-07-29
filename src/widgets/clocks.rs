//! The chronometer: one large local clock, with secondary zones beneath it.
//!
//! The hierarchy is the point. A dashboard answers "what time is it" dozens of
//! times a day and "what time is it in Tokyo" rarely, so the local time is set
//! in block numerals and everything else is a labelled list. Reading the local
//! time should not require focusing on the panel at all.

use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
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
const BINDINGS: &[Binding] = &[
    Binding::primary("s", "seconds"),
    Binding::primary("a", "add zone"),
    Binding::primary("d", "remove"),
    Binding::extra("↑ / ↓", "select a clock"),
    Binding::extra("j / k", "select a clock"),
];

/// The largest scale the numerals are ever drawn at. Past this a clock stops
/// being readable-from-across-the-room and starts being a poster.
const MAX_CLOCK_SCALE: u16 = 3;

/// Rows the numerals occupy at that scale: glyphs are five rows tall at 1.
const BIG_CLOCK_ROWS: u16 = 5 * MAX_CLOCK_SCALE;

/// Columns of the secondary zone list.
const COLUMNS: &[Column] = &[
    Column::flex("zone", 1),
    Column::fixed("time", 9),
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
    /// The `a` dialog, while it is open.
    asking: Option<crate::prompt::Prompt>,
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

    /// Deal with a keypress while the add-zone prompt is open.
    fn handle_prompt_key(&mut self, key: KeyEvent) {
        let Some(prompt) = self.asking.as_mut() else {
            return;
        };
        match prompt.handle_key(key) {
            crate::prompt::Outcome::Editing => {}
            crate::prompt::Outcome::Cancelled => self.asking = None,
            // Picked from the list: the city the reader recognised becomes
            // the clock's label, which is the whole reason the outcome carries
            // both. Nothing needs validating — the list only holds zones that
            // resolve, and a test holds it to that.
            crate::prompt::Outcome::Chose { label, value } => {
                if self.zones.add(label, value) {
                    self.asking = None;
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
                if !self.zones.add(label, timezone) {
                    prompt.reject("that clock is already on the panel");
                    return;
                }
                self.asking = None;
                self.reload();
            }
        }
    }
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
                self.asking = Some(crate::prompt::Prompt::new(
                    "ADD A CLOCK",
                    "Type to narrow · ↑↓ to choose · Enter adds · Esc cancels",
                    "",
                    crate::prompt::Completion::Places(crate::zones::PLACES),
                ));
            }
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

        // Width and height together: filtering a width-only answer by height
        // rejects instead of stepping down a scale, and a *shorter* string earns
        // a bigger one. That is how hiding the seconds used to make the clock
        // smaller rather than larger.
        let fits =
            |text: &str| glyphs::fitting_scale(text, area.width, clock_budget, MAX_CLOCK_SCALE);

        let (time_text, small_seconds) = match (self.show_seconds, fits(&full)) {
            (true, Some(_)) => (full.clone(), None),
            (true, None) => (short.clone(), Some(seconds.clone())),
            (false, _) => (short.clone(), None),
        };

        let scale = fits(&time_text);
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
                if sx < area.x + area.width && y < area.y + area.height {
                    frame.render_widget(
                        Paragraph::new(Span::styled(
                            seconds.clone(),
                            Style::default().fg(theme.muted),
                        )),
                        Rect::new(sx, y, suffix.min(area.width), 1),
                    );
                }
            }
            cursor += big.height;
        } else {
            // Too small for block digits at any scale: fall back to plain text
            // rather than clipping.
            let text = if self.show_seconds { full } else { short };
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
        if cursor < area.y + area.height && !self.config.date_format.is_empty() {
            let date = glyphs::utility(&local.strftime(&self.config.date_format).to_string());
            let width = u16::try_from(date.chars().count()).unwrap_or(0);
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
                            format!("{}{day_marker}", zoned.strftime(&self.config.time_format)),
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
}
