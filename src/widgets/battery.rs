//! The battery: how much is left, which way it is going, and how long that
//! gives you.
//!
//! Asked for by name, which is the filter's own rule for what ships, and the
//! first panel that reads something `sysinfo` does not: `starship-battery`
//! does, on every shipped target including NetBSD, through pure-Rust bindings.
//! The alternative was a polled process per platform (`pmset -g batt`, sysfs,
//! a PowerShell query), which is three code paths to keep honest for one fact.
//!
//! The face is a battery, drawn: a label, a rounded cell with its terminal
//! filled to the charge and the figure beside it, a line of detail. It shipped
//! first in the pomodoro's face, the charge in block numerals over a meter,
//! and the owner rejected that on sight — it read as a second clock. The
//! numerals are for one continuously changing value glanced at across a room,
//! and a charge is neither; a cell you can see filling is what a battery
//! looks like everywhere else, and it is nothing else on the dashboard. Calm
//! by default — brass while there is plenty, and the signal colours only when
//! the charge is low and the machine is running on it. Charging is a state,
//! not an alarm, and is told by the label rather than by flooding the panel
//! green.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::chart::meter_line;
use crate::config::BatteryConfig;
use crate::frame::Binding;
use crate::glyphs;
use crate::panel::{Panel, RenderContext};

/// The most interior rows the cell is drawn with. Taller than this and it
/// stops looking like a battery and starts looking like a box.
const MAX_INTERIOR_ROWS: u16 = 3;

/// Columns per row of the cell, outline included, which is what keeps its
/// shape as it grows: three times as wide as it is tall, on a screen whose
/// cells are twice as tall as they are wide.
const COLUMNS_PER_ROW: u16 = 6;

/// The fewest interior columns worth an outline. Narrower than this and a
/// bare meter says more.
const MIN_BODY: u16 = 4;

/// Which way the charge is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Running on the battery.
    Discharging,
    Charging,
    /// On mains and full.
    Full,
    /// On mains, not charging, not full — macOS holding at 80% is the common
    /// case. The library reports this as unknown with no energy moving, which
    /// is exactly what "plugged in" looks like from the outside.
    Holding,
    Unknown,
}

/// One reading of the battery.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Reading {
    pub charge_pct: u16,
    pub flow: Flow,
    /// To empty when discharging, to full when charging, otherwise `None`.
    pub remaining: Option<Duration>,
    pub health_pct: Option<u16>,
    pub cycles: Option<u32>,
    /// Power moving in or out, in watts.
    pub watts: f32,
}

/// The battery panel.
pub struct BatteryPanel {
    config: BatteryConfig,
    manager: Option<starship_battery::Manager>,
    /// `None` until the first sample, or when the machine has no battery.
    reading: Option<Reading>,
    /// Why the last read failed, if it did.
    error: Option<String>,
    last_sample: Option<Instant>,
}

impl std::fmt::Debug for BatteryPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatteryPanel")
            .field("reading", &self.reading)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl BatteryPanel {
    /// Build the panel and take a first reading.
    pub fn new(config: BatteryConfig) -> Self {
        let (manager, error) = match starship_battery::Manager::new() {
            Ok(manager) => (Some(manager), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let mut panel = Self {
            config,
            manager,
            reading: None,
            error,
            last_sample: None,
        };
        panel.sample();
        panel
    }

    /// A panel holding `reading`, with nothing behind it, for tests in other
    /// modules and for the sweep.
    #[cfg(test)]
    pub(crate) fn with_reading(config: BatteryConfig, reading: Option<Reading>) -> Self {
        Self {
            config,
            manager: None,
            reading,
            error: None,
            last_sample: Some(Instant::now()),
        }
    }

    /// A laptop three hours from empty, for the sweep and the dump.
    #[cfg(test)]
    pub(crate) fn canned(config: BatteryConfig) -> Self {
        Self::with_reading(
            config,
            Some(Reading {
                charge_pct: 80,
                flow: Flow::Discharging,
                remaining: Some(Duration::from_mins(192)),
                health_pct: Some(100),
                cycles: Some(3),
                watts: 12.4,
            }),
        )
    }

    /// Read the first battery the machine reports.
    fn sample(&mut self) -> bool {
        let interval = Duration::from_secs(self.config.sample_secs.max(1));
        if self.last_sample.is_some_and(|at| at.elapsed() < interval) {
            return false;
        }
        self.last_sample = Some(Instant::now());
        let Some(manager) = &self.manager else {
            return false;
        };
        let before = self.reading;
        match manager.batteries() {
            Err(e) => {
                self.error = Some(e.to_string());
                self.reading = None;
            }
            Ok(batteries) => {
                self.error = None;
                self.reading = batteries.flatten().next().map(|b| read(&b));
            }
        }
        before != self.reading
    }

    /// Brass while there is plenty; the signal colours only when the charge is
    /// low *and* the machine is running on it. A low battery on mains is
    /// nothing to be told about.
    fn colour(&self, reading: Reading, theme: &crate::theme::Theme) -> Color {
        match reading.flow {
            Flow::Charging | Flow::Full => theme.success,
            Flow::Discharging if reading.charge_pct < self.config.alert_below_pct => theme.error,
            Flow::Discharging if reading.charge_pct < self.config.warn_below_pct => theme.warning,
            _ => theme.accent,
        }
    }
}

fn read(b: &starship_battery::Battery) -> Reading {
    use starship_battery::State;
    use starship_battery::units::power::watt;
    use starship_battery::units::ratio::percent;
    use starship_battery::units::time::second;

    let pct = |ratio: starship_battery::units::Ratio| -> u16 {
        // `round` before the cast, and clamp: a ratio slightly over one is a
        // battery reporting 100.4%, not a panel reporting 101.
        ratio.get::<percent>().round().clamp(0.0, 100.0) as u16
    };
    let secs =
        |t: starship_battery::units::Time| Duration::from_secs(t.get::<second>().max(0.0) as u64);
    let watts = b.energy_rate().get::<watt>();
    let flow = match b.state() {
        State::Charging => Flow::Charging,
        State::Discharging | State::Empty => Flow::Discharging,
        State::Full => Flow::Full,
        State::Unknown if watts.abs() < 0.05 => Flow::Holding,
        State::Unknown => Flow::Unknown,
    };
    let remaining = match flow {
        Flow::Discharging => b.time_to_empty().map(secs),
        Flow::Charging => b.time_to_full().map(secs),
        _ => None,
    };
    Reading {
        charge_pct: pct(b.state_of_charge()),
        flow,
        remaining,
        health_pct: Some(pct(b.state_of_health())),
        cycles: b.cycle_count(),
        watts,
    }
}

/// `3h 12m`, `42m`, or `under a minute` — how someone says it, not `03:12:00`.
pub(crate) fn describe_remaining(left: Duration) -> String {
    let minutes = left.as_secs() / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "under a minute".to_string(),
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// The label over the numerals, in the utility face.
fn label_for(flow: Flow) -> &'static str {
    match flow {
        Flow::Discharging => "on battery",
        Flow::Charging => "charging",
        Flow::Full => "full",
        Flow::Holding => "plugged in",
        Flow::Unknown => "battery",
    }
}

/// Keys this panel responds to: none. It has nothing to set.
const BINDINGS: &[Binding] = &[];

impl Panel for BatteryPanel {
    fn title(&self) -> String {
        "Battery".to_string()
    }

    fn counter(&self) -> Option<String> {
        // The one fact worth a glance at the frame: how long. It lives here
        // and nowhere inside the panel, the way the task count lives in its
        // border — the label says which way, the numerals say how much, and
        // nothing is said twice.
        let reading = self.reading?;
        match (reading.flow, reading.remaining) {
            (Flow::Discharging, Some(left)) => Some(format!("{} left", describe_remaining(left))),
            (Flow::Charging, Some(left)) => Some(format!("{} to full", describe_remaining(left))),
            _ => None,
        }
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn refresh_interval(&self) -> Duration {
        Duration::from_secs(5)
    }

    fn tick(&mut self) -> bool {
        self.sample()
    }

    fn alert(&self) -> Option<crate::panel::Alert> {
        // The one battery fact that gets worse if nobody acts in the next few
        // minutes: it is running down and it is nearly gone. A low battery on
        // mains, or one that is low and charging, is not that.
        let reading = self.reading?;
        if reading.flow != Flow::Discharging || reading.charge_pct >= self.config.alert_below_pct {
            return None;
        }
        let left = reading
            .remaining
            .map(|left| format!(" — {} left", describe_remaining(left)))
            .unwrap_or_default();
        Some(crate::panel::Alert::soon(format!(
            "Battery at {}%{left}",
            reading.charge_pct
        )))
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.width == 0 || area.height == 0 {
            return;
        }
        let Some(reading) = self.reading else {
            self.draw_empty(frame, area, theme);
            return;
        };

        let colour = self.colour(reading, theme);
        let muted = Style::default().fg(theme.muted);
        let bottom = area.y + area.height;
        let figure = format!("{}%", reading.charge_pct);

        // The face, top to bottom: label, cell, detail — centred as a block
        // the way the pomodoro centres its clock, so a tall panel does not
        // pin the whole thing to its top edge. The cell takes the rows the
        // label and the detail leave, up to a shape that still reads as a
        // battery; below three rows there is no room for an outline and the
        // charge is a bare meter with the figure beside it.
        let face = Face::fit(area.width, area.height, &figure);
        let mut cursor = area.y + area.height.saturating_sub(face.rows()) / 2;

        // 1. The label.
        if face.label && cursor < bottom {
            let label = glyphs::utility(label_for(reading.flow));
            draw_centred(frame, area, cursor, &label, colour);
            cursor += 1;
        }

        // 2. The cell, or what there is room for.
        cursor += draw_cell(
            frame,
            area,
            cursor,
            face.cell_rows,
            reading.charge_pct,
            &figure,
            colour,
            theme,
        );

        // 3. The detail.
        if face.detail && cursor < bottom {
            let parts = detail_parts(reading, muted);
            if !parts.is_empty() {
                frame.render_widget(
                    Paragraph::new(crate::grid::assemble(parts, area.width)).centered(),
                    Rect::new(area.x, cursor, area.width, 1),
                );
            }
        }
    }
}

/// Which rows of the face a height has room for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Face {
    label: bool,
    /// Rows for the cell: one or two is a bare meter, three or more an
    /// outlined cell with `cell_rows - 2` interior rows.
    cell_rows: u16,
    detail: bool,
}

impl Face {
    /// The label and the detail give way before the cell does, in that
    /// order from the outside in: a panel one row high is a meter and a
    /// figure, which is the battery with everything else gone. A panel too
    /// narrow for an outline beside the figure gets the same bare meter at
    /// any height, so the rest of the face can close around it.
    fn fit(width: u16, height: u16, figure: &str) -> Self {
        let outlined = width >= chrome_width(figure) + MIN_BODY;
        let (label, cell_rows, detail) = match height {
            0 | 1 => (false, 1, false),
            2 => (true, 1, false),
            3 if outlined => (false, 3, false),
            4 if outlined => (true, 3, false),
            _ if outlined => (true, (height - 2).min(MAX_INTERIOR_ROWS + 2), true),
            _ => (true, 1, true),
        };
        Self {
            label,
            cell_rows,
            detail,
        }
    }

    fn rows(self) -> u16 {
        u16::from(self.label) + self.cell_rows + u16::from(self.detail)
    }
}

/// The battery cell at `top`: a rounded outline with its terminal on the
/// right, the interior filled to the charge, and the figure beside it. Where
/// the width has no room for an outline it is a bare meter and the figure;
/// where it has no room for that, the figure alone. Returns the rows used.
#[allow(clippy::too_many_arguments)] // one call site, and each is a distinct fact of the face
fn draw_cell(
    frame: &mut Frame,
    area: Rect,
    top: u16,
    rows: u16,
    charge_pct: u16,
    figure: &str,
    colour: Color,
    theme: &crate::theme::Theme,
) -> u16 {
    let bottom = area.y + area.height;
    if top >= bottom || area.width == 0 {
        return 0;
    }
    let figure_w = u16::try_from(crate::grid::display_width(figure)).unwrap_or(u16::MAX);
    let chrome = chrome_width(figure);
    let outline = Style::default().fg(theme.muted);
    let fill = Style::default().fg(colour);
    let track = Style::default().fg(theme.track);
    let figure_spans = |gap: &'static str| {
        let (digits, unit) = figure.split_at(figure.len().saturating_sub(1));
        vec![
            Span::raw(gap),
            Span::styled(digits.to_string(), fill.add_modifier(Modifier::BOLD)),
            Span::styled(unit.to_string(), outline),
        ]
    };

    let interior_rows = rows.saturating_sub(2);
    if rows >= 3 && area.width >= chrome + MIN_BODY {
        let body = (area.width - chrome).min(COLUMNS_PER_ROW * rows - 2);
        let filled = filled_cells(charge_pct, body);
        let x = area.x + area.width.saturating_sub(body + chrome) / 2;
        let cap = |left: &str, right: &str| {
            Line::from(vec![
                Span::styled(left.to_string(), outline),
                Span::styled("─".repeat(usize::from(body)), outline),
                Span::styled(right.to_string(), outline),
            ])
        };
        let mut lines = vec![cap("╭", "╮")];
        // The terminal is a third of the cell's height, and the figure sits
        // on its middle row.
        let nub_rows: std::ops::RangeInclusive<u16> = match interior_rows {
            2 => 0..=1,
            n => (n - 1) / 2..=(n - 1) / 2,
        };
        for row in 0..interior_rows {
            let mut spans = vec![
                Span::styled("│", outline),
                Span::styled("█".repeat(usize::from(filled)), fill),
                Span::styled("░".repeat(usize::from(body - filled)), track),
                Span::styled("│", outline),
            ];
            if nub_rows.contains(&row) {
                spans.push(Span::styled("▌", outline));
            }
            if row == (interior_rows - 1) / 2 {
                if !nub_rows.contains(&row) {
                    spans.push(Span::raw(" "));
                }
                spans.extend(figure_spans(" "));
            }
            lines.push(Line::from(spans));
        }
        lines.push(cap("╰", "╯"));
        for (i, line) in lines.into_iter().enumerate() {
            let y = top + u16::try_from(i).unwrap_or(u16::MAX);
            if y >= bottom {
                break;
            }
            frame.render_widget(
                Paragraph::new(line),
                Rect::new(x, y, (body + chrome).min(area.width), 1),
            );
        }
        return rows;
    }

    // No room for an outline: the meter and the figure on one row, the
    // figure alone if not even that fits, and `…` if not even the figure.
    let line = if area.width >= figure_w + 3 {
        let meter = area.width - figure_w - 1;
        let mut spans = meter_line(charge_pct, meter, colour, theme.track);
        spans.extend(figure_spans(" "));
        Line::from(spans)
    } else if area.width >= figure_w {
        Line::from(figure_spans("")).centered()
    } else {
        let cut = crate::grid::truncate(figure, usize::from(area.width));
        Line::from(Span::styled(cut, fill.add_modifier(Modifier::BOLD))).centered()
    };
    frame.render_widget(Paragraph::new(line), Rect::new(area.x, top, area.width, 1));
    1
}

/// What sits beside the cell's interior on its widest row: the outline
/// either side, the terminal, a space, and the figure.
fn chrome_width(figure: &str) -> u16 {
    4 + u16::try_from(crate::grid::display_width(figure)).unwrap_or(u16::MAX)
}

/// How many of `body` cells the charge fills. Whole cells, rounded down, so
/// only a full battery draws a full cell — and never none while there is any
/// charge at all, since an empty cell beside `1%` says two different things.
fn filled_cells(charge_pct: u16, body: u16) -> u16 {
    let filled = charge_pct.min(100) * body / 100;
    if charge_pct > 0 {
        filled.max(1)
    } else {
        filled
    }
}

impl BatteryPanel {
    /// A desktop, or a battery the platform will not show us. Say which,
    /// centred, and stop: an empty meter would read as a broken one.
    fn draw_empty(&self, frame: &mut Frame, area: Rect, theme: &crate::theme::Theme) {
        let muted = theme.muted;
        let (first, second) = match &self.error {
            Some(why) => ("No battery readable", why.as_str()),
            None => ("No battery", "Mains powered"),
        };
        let top = area.y + area.height.saturating_sub(2) / 2;
        for (i, text) in [first, second].into_iter().enumerate() {
            let y = top + u16::try_from(i).unwrap_or(0);
            if y < area.y + area.height {
                let text = crate::grid::truncate(text, usize::from(area.width));
                frame.render_widget(
                    Paragraph::new(Span::styled(text, Style::default().fg(muted))).centered(),
                    Rect::new(area.x, y, area.width, 1),
                );
            }
        }
    }
}

/// One bold line, centred, ellipsised to the width.
fn draw_centred(frame: &mut Frame, area: Rect, y: u16, text: &str, colour: Color) {
    frame.render_widget(
        Paragraph::new(Span::styled(
            crate::grid::truncate(text, usize::from(area.width)),
            Style::default().fg(colour).add_modifier(Modifier::BOLD),
        ))
        .centered(),
        Rect::new(area.x, y, area.width, 1),
    );
}

/// The detail line as parts for `grid::assemble`: the context under the
/// figure, in the order someone would ask for it, each dropping whole. The
/// time is not here — it is in the border — and neither is the charge, which
/// is the figure. The gap travels with the part it introduces, so a dropped
/// part takes its gap with it.
fn detail_parts(reading: Reading, muted: Style) -> Vec<Vec<Span<'static>>> {
    let mut texts: Vec<String> = Vec::new();
    if let Some(health) = reading.health_pct {
        texts.push(format!("health {health}%"));
    }
    if let Some(cycles) = reading.cycles {
        texts.push(format!("{cycles} cycles"));
    }
    if reading.watts.abs() >= 0.05 {
        texts.push(format!("{:.1} W", reading.watts.abs()));
    }
    texts
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let gap = if i == 0 { "" } else { "   " };
            vec![Span::styled(format!("{gap}{text}"), muted)]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(charge_pct: u16, flow: Flow, remaining: Option<u64>) -> Reading {
        Reading {
            charge_pct,
            flow,
            remaining: remaining.map(Duration::from_secs),
            health_pct: Some(100),
            cycles: Some(3),
            watts: 12.4,
        }
    }

    fn panel(reading: Option<Reading>) -> BatteryPanel {
        BatteryPanel::with_reading(BatteryConfig::default(), reading)
    }

    fn screen(panel: &mut BatteryPanel, width: u16, height: u16) -> Vec<String> {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();
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
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn remaining_time_reads_the_way_someone_says_it() {
        assert_eq!(describe_remaining(Duration::from_secs(0)), "under a minute");
        assert_eq!(
            describe_remaining(Duration::from_secs(59)),
            "under a minute"
        );
        assert_eq!(describe_remaining(Duration::from_mins(42)), "42m");
        assert_eq!(describe_remaining(Duration::from_hours(3)), "3h");
        assert_eq!(
            describe_remaining(Duration::from_mins(3 * 60 + 12)),
            "3h 12m"
        );
    }

    /// The counter carries the one fact worth a glance at the frame — how
    /// long — and nothing the panel already says: the label has the state
    /// and the numerals have the charge.
    #[test]
    fn the_border_says_how_long_and_nothing_the_panel_already_says() {
        assert_eq!(
            panel(Some(reading(80, Flow::Discharging, Some(11520))))
                .counter()
                .as_deref(),
            Some("3h 12m left")
        );
        assert_eq!(
            panel(Some(reading(40, Flow::Charging, Some(2520))))
                .counter()
                .as_deref(),
            Some("42m to full")
        );
        assert_eq!(
            panel(Some(reading(80, Flow::Holding, None))).counter(),
            None
        );
        assert_eq!(panel(Some(reading(100, Flow::Full, None))).counter(), None);
        assert_eq!(
            panel(Some(reading(40, Flow::Charging, None))).counter(),
            None
        );
        assert_eq!(
            panel(Some(reading(63, Flow::Discharging, None))).counter(),
            None,
            "no estimate, nothing to say — the charge is the figure"
        );
        assert_eq!(panel(None).counter(), None, "no battery, no counter");
    }

    /// Calm by default: brass while there is plenty, and the signal colours
    /// only when the charge is low *and* the machine is running on it.
    #[test]
    fn the_colour_is_calm_unless_the_battery_is_low_and_in_use() {
        let theme = crate::theme::Theme::default();
        let p = panel(None);
        assert_eq!(
            p.colour(reading(80, Flow::Discharging, None), &theme),
            theme.accent
        );
        assert_eq!(
            p.colour(reading(15, Flow::Discharging, None), &theme),
            theme.warning
        );
        assert_eq!(
            p.colour(reading(7, Flow::Discharging, None), &theme),
            theme.error
        );
        assert_eq!(
            p.colour(reading(7, Flow::Charging, None), &theme),
            theme.success,
            "low but charging is fine"
        );
        assert_eq!(
            p.colour(reading(7, Flow::Holding, None), &theme),
            theme.accent,
            "low on mains is nothing to shout about"
        );
        assert_eq!(
            p.colour(reading(100, Flow::Full, None), &theme),
            theme.success
        );
    }

    /// The bar for "will this get worse if nobody acts": running down and
    /// nearly gone. Charging at the same level is not that.
    #[test]
    fn the_alert_fires_only_when_running_down_and_nearly_gone() {
        assert_eq!(
            panel(Some(reading(8, Flow::Discharging, Some(14 * 60))))
                .alert()
                .map(|a| a.text),
            Some("Battery at 8% — 14m left".to_string())
        );
        assert_eq!(
            panel(Some(reading(8, Flow::Discharging, None)))
                .alert()
                .map(|a| a.text),
            Some("Battery at 8%".to_string())
        );
        assert!(
            panel(Some(reading(8, Flow::Charging, None)))
                .alert()
                .is_none()
        );
        assert!(
            panel(Some(reading(10, Flow::Discharging, None)))
                .alert()
                .is_none(),
            "the threshold is exclusive"
        );
        assert!(panel(None).alert().is_none());
    }

    /// Every row of the face is present at an ordinary size, in the order the
    /// eye reads it: the label, a cell drawn as a battery with the figure
    /// beside its terminal, the detail. Nothing is drawn in block numerals,
    /// which is what made the first face read as a second clock, and nothing
    /// is drawn as a fragment at any size.
    #[test]
    fn the_face_reads_label_figure_meter_detail_and_never_a_fragment() {
        let mut p = panel(Some(reading(80, Flow::Discharging, Some(11520))));
        let rows = screen(&mut p, 34, 9);
        let text: Vec<&str> = rows
            .iter()
            .map(|r| r.trim())
            .filter(|r| !r.is_empty())
            .collect();
        assert_eq!(text[0], "ON BATTERY", "{rows:?}");
        assert_eq!(
            text[1], "╭───────────────────────────╮",
            "a rounded cell, three columns to a row: {rows:?}"
        );
        assert_eq!(text[2], "│█████████████████████░░░░░░│", "{rows:?}");
        assert_eq!(
            text[3], "│█████████████████████░░░░░░│▌ 80%",
            "the terminal on the middle row, and the figure beside it: {rows:?}"
        );
        assert_eq!(text[4], "│█████████████████████░░░░░░│", "{rows:?}");
        assert_eq!(text[5], "╰───────────────────────────╯", "{rows:?}");
        assert_eq!(
            text[6], "health 100%   3 cycles   12.4 W",
            "then the detail — and not the time, which the border has: {rows:?}"
        );
        assert_eq!(text.len(), 7, "{rows:?}");
        assert!(
            !rows.iter().any(|r| r.contains("left")),
            "the time is said once, by the border: {rows:?}"
        );

        for width in 1..=40u16 {
            for height in 1..=12u16 {
                let rows = screen(&mut p, width, height);
                for row in &rows {
                    let t = row.trim();
                    // No fragment of any value: whole or absent. The figure
                    // is the one that matters most — `80` without its `%`
                    // beside a cell is a number nobody said.
                    for bad in ["healt", "cycle", "12.", "80"] {
                        assert!(!t.ends_with(bad), "fragment {t:?} at {width}x{height}");
                    }
                    assert!(
                        !t.contains("80") || t.contains("80%"),
                        "a figure without its unit: {t:?} at {width}x{height}"
                    );
                    assert!(
                        !t.contains('▌') || t.contains("│▌"),
                        "a terminal off its cell: {t:?} at {width}x{height}"
                    );
                }
            }
        }
    }

    /// The cell fills in proportion to the charge, in whole cells rounded
    /// down: only a full battery draws a full cell, and any charge at all
    /// draws at least one, because an empty cell beside `1%` says two things.
    #[test]
    fn the_cell_fills_in_proportion_to_the_charge() {
        let middle = |charge: u16| -> (usize, usize) {
            let mut p = panel(Some(reading(charge, Flow::Discharging, None)));
            let rows = screen(&mut p, 34, 3);
            let row = rows.iter().find(|r| r.contains('▌')).expect("the cell");
            (row.matches('█').count(), row.matches('░').count())
        };
        assert_eq!(middle(100), (16, 0), "full");
        assert_eq!(middle(99), (15, 1), "nearly full is not full");
        assert_eq!(middle(50), (8, 8), "half");
        assert_eq!(middle(7), (1, 15), "low");
        assert_eq!(middle(1), (1, 15), "any charge at all draws a cell");
        assert_eq!(middle(0), (0, 16), "empty");
        assert_eq!(filled_cells(1, 40), 1);
        assert_eq!(filled_cells(0, 40), 0);
        assert_eq!(filled_cells(100, 40), 40);
        assert_eq!(
            filled_cells(200, 40),
            40,
            "a reading over 100 is still full"
        );
    }

    /// The face gives up its rows from the outside in — detail, then label —
    /// and the cell gives up its outline last, becoming the bare meter and
    /// figure it always was underneath. Too narrow for an outline beside the
    /// figure, it is the bare meter at any height.
    #[test]
    fn the_face_gives_up_its_rows_from_the_outside_in() {
        let mut p = panel(Some(reading(80, Flow::Discharging, None)));
        let nonblank = |rows: Vec<String>| -> Vec<String> {
            rows.into_iter()
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty())
                .collect()
        };
        let mut at = |w, h| nonblank(screen(&mut p, w, h));

        assert_eq!(at(34, 1), ["■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■ 80%"]);
        assert_eq!(
            at(34, 2),
            ["ON BATTERY", "■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■ 80%"]
        );
        assert_eq!(
            at(34, 3),
            [
                "╭────────────────╮",
                "│████████████░░░░│▌ 80%",
                "╰────────────────╯"
            ],
            "three rows is the outline alone"
        );
        assert_eq!(
            at(34, 4),
            [
                "ON BATTERY",
                "╭────────────────╮",
                "│████████████░░░░│▌ 80%",
                "╰────────────────╯"
            ]
        );
        assert_eq!(
            at(34, 5).last().map(String::as_str),
            Some("health 100%   3 cycles   12.4 W"),
            "five rows brings the detail back"
        );
        assert_eq!(
            at(9, 7),
            ["ON BATTE…", "■■■■■ 80%", "health 1…"],
            "too narrow for an outline: a bare meter, at any height"
        );
        let mut q = panel(Some(reading(80, Flow::Discharging, None)));
        assert_eq!(
            screen(&mut q, 9, 7)[3],
            "■■■■■ 80%",
            "and the three rows are centred, not pinned to the top by the \
             rows an outline would have taken"
        );
        assert_eq!(at(3, 1), ["80%"], "the figure alone");
        assert_eq!(at(2, 1), ["8…"], "and the figure says when it is cut");
    }

    /// A taller panel draws a taller cell, wider in step so it keeps its
    /// shape, up to three interior rows — past that it stops looking like a
    /// battery and starts looking like a box, and the extra rows go to
    /// centring the face instead.
    #[test]
    fn the_cell_grows_with_the_panel_and_stops_looking_like_a_box() {
        let mut p = panel(Some(reading(80, Flow::Discharging, None)));
        let mut cell = |h: u16| -> (usize, usize) {
            let rows = screen(&mut p, 60, h);
            let top = rows.iter().find(|r| r.contains('╭')).expect("an outline");
            let interior = rows.iter().filter(|r| r.contains('│')).count();
            (interior, top.trim().chars().count())
        };
        assert_eq!(cell(5), (1, 18));
        assert_eq!(cell(6), (2, 24));
        assert_eq!(cell(7), (3, 30));
        assert_eq!(cell(12), (3, 30), "capped");
        assert_eq!(cell(40), (3, 30), "still capped");
    }

    #[test]
    fn a_machine_without_a_battery_says_so_rather_than_drawing_an_empty_meter() {
        let mut p = panel(None);
        let rows = screen(&mut p, 24, 5);
        let text = rows.join("\n");
        assert!(text.contains("No battery"), "{rows:?}");
        assert!(text.contains("Mains powered"), "{rows:?}");
        assert!(!text.contains('■'), "no meter for nothing: {rows:?}");
    }
}
