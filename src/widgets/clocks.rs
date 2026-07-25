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
use crate::frame::Binding;
use crate::glyphs::{self, BigText};
use crate::grid::{Column, Grid};
use crate::panel::{KeyOutcome, Panel, RenderContext};

/// Keys this panel responds to.
const BINDINGS: &[Binding] = &[Binding::primary("s", "seconds")];

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
}

impl ClocksPanel {
    /// Resolve every configured zone once, at construction.
    pub fn new(config: ClocksConfig) -> Self {
        let mut clocks: Vec<Clock> = config
            .zones
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
        Self {
            config,
            primary,
            secondary: clocks,
            show_seconds,
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

    fn refresh_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(250)
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        if matches!(key.code, KeyCode::Char('s')) {
            self.show_seconds = !self.show_seconds;
            return KeyOutcome::Consumed;
        }
        KeyOutcome::Ignored
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

        let fits = |text: &str| {
            glyphs::fitting_scale(text, area.width, 3)
                .filter(|scale| BigText::new(text, *scale).height <= clock_budget)
        };

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

        for clock in &self.secondary {
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
                        Span::styled(clock.label.clone(), Style::default().fg(theme.text)),
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
                    Span::styled(clock.label.clone(), Style::default().fg(theme.muted)),
                    Span::styled(message.clone(), Style::default().fg(theme.error)),
                ])),
            }
        }

        frame.render_widget(
            Paragraph::new(lines),
            Rect::new(area.x, cursor, area.width, remaining),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClockZone;
    use jiff::tz::Offset;

    fn zone(label: &str, tz: &str) -> ClockZone {
        ClockZone {
            label: label.into(),
            timezone: tz.into(),
        }
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
        let panel = ClocksPanel::new(ClocksConfig {
            zones: vec![zone("Home", "UTC"), zone("Tokyo", "Asia/Tokyo")],
            ..Default::default()
        });
        assert_eq!(panel.primary.label, "Home");
        assert_eq!(panel.secondary.len(), 1);
        assert_eq!(panel.secondary[0].label, "Tokyo");
    }

    #[test]
    fn an_empty_zone_list_still_shows_local_time() {
        let panel = ClocksPanel::new(ClocksConfig {
            zones: Vec::new(),
            ..Default::default()
        });
        assert!(panel.primary.zone.is_ok());
        assert!(panel.secondary.is_empty());
    }

    #[test]
    fn a_label_falls_back_to_the_zone_name() {
        let panel = ClocksPanel::new(ClocksConfig {
            zones: vec![zone("", "UTC"), zone("", "Asia/Tokyo")],
            ..Default::default()
        });
        assert_eq!(panel.primary.label, "UTC");
        assert_eq!(panel.secondary[0].label, "Asia/Tokyo");
    }

    #[test]
    fn s_toggles_seconds_and_is_consumed() {
        let mut panel = ClocksPanel::new(ClocksConfig::default());
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
        let mut panel = ClocksPanel::new(ClocksConfig::default());
        let outcome = panel.handle_key(KeyEvent::new(
            KeyCode::Tab,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(outcome, KeyOutcome::Ignored);
    }
}
