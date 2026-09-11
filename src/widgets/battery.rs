//! The battery: how much is left, which way it is going, and how long that
//! gives you.
//!
//! Asked for by name, which is the filter's own rule for what ships, and the
//! first panel that reads something `sysinfo` does not: `starship-battery`
//! does, on every shipped target including NetBSD, through pure-Rust bindings.
//! The alternative was a polled process per platform (`pmset -g batt`, sysfs,
//! a PowerShell query), which is three code paths to keep honest for one fact.
//!
//! The face is the pomodoro's: a label, the figure in block numerals, a meter,
//! a line of detail. Calm by default — brass while there is plenty, and the
//! signal colours only when the charge is low and the machine is running on
//! it. Charging is a state, not an alarm, and is told by the label rather than
//! by flooding the panel green.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::chart::meter_line;
use crate::config::BatteryConfig;
use crate::frame::Binding;
use crate::glyphs::{self, BigText};
use crate::panel::{Panel, RenderContext};

/// The tallest the numerals are drawn.
const MAX_SCALE: u16 = 2;

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

        // The face, top to bottom: label, numerals, meter, detail — centred as a
        // block the way the pomodoro centres its clock, so a tall panel does not
        // pin the whole thing to its top edge. Two cells are reserved beside the
        // numerals for the small `%`, as the clock reserves them for its small
        // seconds.
        let digits = reading.charge_pct.to_string();
        let scale = glyphs::fitting_scale(
            &digits,
            area.width.saturating_sub(2),
            area.height.saturating_sub(3).max(1),
            MAX_SCALE,
        );
        let numeral_rows = scale.map_or(1, |s| BigText::new(&digits, s).height);
        let mut cursor = area.y + area.height.saturating_sub(3 + numeral_rows) / 2;

        // 1. The label.
        if cursor < bottom {
            let label = glyphs::utility(label_for(reading.flow));
            draw_centred(frame, area, cursor, &label, colour);
            cursor += 1;
        }

        // 2. The figure.
        cursor += match scale {
            Some(scale) => draw_figure(frame, area, cursor, &digits, scale, colour, muted),
            None if cursor < bottom => {
                draw_centred(
                    frame,
                    area,
                    cursor,
                    &format!("{}%", reading.charge_pct),
                    colour,
                );
                1
            }
            None => 0,
        };

        // 3. The meter: the charge, as a bar. This *is* the battery, drawn.
        if cursor < bottom && area.width > 4 {
            let meter = meter_line(reading.charge_pct, area.width, colour, theme.track);
            frame.render_widget(
                Paragraph::new(Line::from(meter)),
                Rect::new(area.x, cursor, area.width, 1),
            );
            cursor += 1;
        }

        // 4. The detail.
        if cursor < bottom {
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

/// The charge in block numerals with a small `%` on their baseline — a
/// subscript rather than another glyph, since the face has no `%`. Returns
/// the rows used.
fn draw_figure(
    frame: &mut Frame,
    area: Rect,
    top: u16,
    digits: &str,
    scale: u16,
    colour: Color,
    muted: Style,
) -> u16 {
    let bottom = area.y + area.height;
    let big = BigText::new(digits, scale);
    let x = area.x + area.width.saturating_sub(big.width + 2) / 2;
    for (i, row) in big.rows.iter().enumerate() {
        let y = top + u16::try_from(i).unwrap_or(0);
        if y >= bottom {
            break;
        }
        frame.render_widget(
            Paragraph::new(Span::styled(row.clone(), Style::default().fg(colour))),
            Rect::new(x, y, big.width.min(area.width), 1),
        );
    }
    let y = top + big.height.saturating_sub(1);
    let sx = x + big.width + 1;
    if sx < area.x + area.width && y < bottom {
        frame.render_widget(
            Paragraph::new(Span::styled("%", muted)),
            Rect::new(sx, y, 1, 1),
        );
    }
    big.height
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
    /// eye reads it, and nothing is drawn as a fragment at any width.
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
        assert!(
            text.iter().any(|r| r.contains('█')),
            "the figure is drawn in numerals: {rows:?}"
        );
        assert!(
            text.iter().any(|r| r.contains('█') && r.ends_with('%')),
            "with the small percent beside the numerals: {rows:?}"
        );
        assert!(
            text.iter().any(|r| r.starts_with('■')),
            "then the meter: {rows:?}"
        );
        assert_eq!(
            *text.last().unwrap(),
            "health 100%   3 cycles   12.4 W",
            "then the detail — and not the time, which the border has: {rows:?}"
        );
        assert!(
            !rows.iter().any(|r| r.contains("left")),
            "the time is said once, by the border: {rows:?}"
        );

        for width in 1..=40u16 {
            for height in 1..=9u16 {
                let rows = screen(&mut p, width, height);
                for row in &rows {
                    let t = row.trim();
                    // No fragment of any detail value: whole or absent.
                    for bad in ["healt", "cycle", "12."] {
                        assert!(!t.ends_with(bad), "fragment {t:?} at {width}x{height}");
                    }
                }
            }
        }
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
