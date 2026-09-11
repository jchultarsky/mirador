//! Temperatures: the hottest thing in the machine, its history, and the rest
//! of the sensors under it.
//!
//! Read through `sysinfo::Components`, which is already in the tree for the
//! cpu and memory panels. What it hands back is the platform's own list — on
//! Apple silicon that is fourteen `PMU tdie` readings for one die, a
//! calibration reference that is not a temperature at all, and eight `tdev`
//! entries reporting minus nine thousand degrees — so the panel's real work is
//! [`group`]: turning the platform's list into the one a person would write.
//!
//! The face is the cpu panel's: a readout, a braille history, and a table
//! where the per-core strip would be. Same ramp, because a temperature in
//! degrees Celsius and a load in percent both run 0–100 and both mean "how
//! hard is this machine working" — the panels change colour together.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use crate::chart::BrailleGraph;
use crate::config::TemperatureConfig;
use crate::frame::Binding;
use crate::grid::{Column, Grid};
use crate::panel::{Panel, RenderContext};

/// The table under the graph: every sensor group, hottest first.
pub const COLUMNS: &[Column] = &[
    Column::flex("sensor", 1),
    Column::fixed("now", 5).right(),
    Column::fixed("peak", 5).right().drops_below(26),
];

/// Readings outside this range are a sensor that is not connected or a
/// platform reporting a placeholder, not a temperature. Apple's `PMU tdev`
/// entries sit at −9201; a value like that in the table would be the panel
/// reporting a thermometer that does not exist.
const PLAUSIBLE_C: std::ops::RangeInclusive<f32> = -40.0..=150.0;

/// Platform labels renamed to what they are. Each entry is a prefix matched
/// case-insensitively against the label with its trailing digits stripped, so
/// `PMU tdie1` through `PMU tdie14` all become one row called `CPU die`.
/// Anything not here keeps its own name, minus the trailing digits — honest,
/// if less pretty, and the rows still group.
const RENAMES: &[(&str, &str)] = &[
    // Apple silicon, via IOKit. `tdie` is the die; `tcal` is a calibration
    // reference and is dropped below rather than renamed.
    ("PMU tdie", "CPU die"),
    ("NAND CH", "SSD"),
    ("gas gauge battery", "Battery"),
    // Linux hwmon, as `sysinfo` labels it: driver name, then the sensor.
    ("coretemp Package id", "CPU package"),
    ("coretemp Core", "CPU core"),
    ("k10temp Tctl", "CPU (Tctl)"),
    ("k10temp Tccd", "CPU die"),
    ("nvme Composite", "SSD"),
    ("amdgpu edge", "GPU"),
    ("acpitz", "Motherboard"),
];

/// Labels that are not temperatures of anything, whatever they read.
const DROPPED: &[&str] = &["PMU tcal"];

/// One row of the table: a group of sensors under one name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sensor {
    pub name: String,
    /// Degrees Celsius, the hottest in the group.
    pub now: f32,
    /// The hottest the group has reported since the panel started.
    pub peak: f32,
}

/// The temperature panel.
pub struct TemperaturePanel {
    config: TemperatureConfig,
    components: Option<sysinfo::Components>,
    /// Hottest first.
    sensors: Vec<Sensor>,
    /// The hottest reading at each sample, whole degrees Celsius, oldest first.
    history: VecDeque<u64>,
    /// Cells the graph was last drawn into, so the history can grow to fill it.
    graph_cells: usize,
    last_sample: Option<Instant>,
    /// `u` toggles this; the config seeds it.
    celsius: bool,
}

// `sysinfo::Components` has no `Debug`.
impl std::fmt::Debug for TemperaturePanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemperaturePanel")
            .field("sensors", &self.sensors)
            .field("samples", &self.history.len())
            .finish_non_exhaustive()
    }
}

impl TemperaturePanel {
    /// Build the panel and take a first reading.
    pub fn new(config: TemperatureConfig) -> Self {
        let mut panel = Self {
            celsius: config.units != "fahrenheit",
            history: VecDeque::with_capacity(config.history.max(1)),
            config,
            components: Some(sysinfo::Components::new_with_refreshed_list()),
            sensors: Vec::new(),
            graph_cells: 0,
            last_sample: None,
        };
        panel.sample();
        panel
    }

    /// A panel holding `sensors` and `history`, reading nothing, for tests in
    /// other modules and for the sweep.
    #[cfg(test)]
    pub(crate) fn with_sensors(
        config: TemperatureConfig,
        sensors: Vec<Sensor>,
        history: &[u64],
    ) -> Self {
        Self {
            celsius: config.units != "fahrenheit",
            history: history.iter().copied().collect(),
            config,
            components: None,
            sensors,
            graph_cells: 0,
            last_sample: Some(Instant::now()),
        }
    }

    /// An Apple silicon laptop at work, for the sweep and the dump.
    #[cfg(test)]
    pub(crate) fn canned(config: TemperatureConfig) -> Self {
        let sensor = |name: &str, now: f32, peak: f32| Sensor {
            name: name.into(),
            now,
            peak,
        };
        Self::with_sensors(
            config,
            vec![
                sensor("CPU die", 47.3, 61.0),
                sensor("SSD", 34.0, 36.0),
                sensor("Battery", 29.0, 31.0),
            ],
            &[41, 43, 47, 52, 49, 47],
        )
    }

    /// Read every sensor if enough time has passed since the last reading.
    /// Returns whether anything was read.
    fn sample(&mut self) -> bool {
        let interval = Duration::from_secs(self.config.sample_secs.max(1));
        if self.last_sample.is_some_and(|at| at.elapsed() < interval) {
            return false;
        }
        self.last_sample = Some(Instant::now());
        let Some(components) = self.components.as_mut() else {
            return false;
        };
        components.refresh(true);
        let raw = components
            .iter()
            .map(|c| (c.label().to_string(), c.temperature(), c.max()))
            .collect::<Vec<_>>();
        self.sensors = group(raw, &self.sensors);
        if let Some(hottest) = self.sensors.first() {
            let capacity = crate::samples::capacity(self.config.history, self.graph_cells);
            crate::samples::push_bounded(
                &mut self.history,
                hottest.now.clamp(0.0, 150.0).round() as u64,
                capacity,
            );
        }
        true
    }

    /// A reading in the display units.
    fn shown(&self, celsius: f32) -> f32 {
        if self.celsius {
            celsius
        } else {
            celsius * 9.0 / 5.0 + 32.0
        }
    }

    fn unit(&self) -> &'static str {
        if self.celsius { "°C" } else { "°F" }
    }

    /// Say nothing, centred, rather than draw an empty graph over an empty
    /// table: on Windows the sensors need elevation, and a VM or a container
    /// has none to report.
    fn draw_empty(frame: &mut Frame, area: Rect, theme: &crate::theme::Theme) {
        let hint = if cfg!(windows) {
            "Run as administrator to read them"
        } else {
            "This machine reports none"
        };
        let top = area.y + area.height.saturating_sub(2) / 2;
        for (i, text) in ["No temperature sensors", hint].into_iter().enumerate() {
            let y = top + u16::try_from(i).unwrap_or(0);
            if y < area.y + area.height {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        crate::grid::truncate(text, usize::from(area.width)),
                        Style::default().fg(theme.muted),
                    ))
                    .centered(),
                    Rect::new(area.x, y, area.width, 1),
                );
            }
        }
    }
}

/// The platform's sensor list as a person would write it.
///
/// Each raw entry is a label, a reading and the platform's own running
/// maximum. Readings that are not plausible temperatures are dropped, so are
/// labels that are not temperatures of anything, trailing digits come off
/// the label so `Core 0` and `Core 7` share a row, known labels are renamed,
/// and each name keeps the hottest reading in its group. The peak is the
/// highest the group has shown since the panel started — `previous` carries
/// it across samples, because the platform's own maximum is not always kept.
pub(crate) fn group(
    raw: Vec<(String, Option<f32>, Option<f32>)>,
    previous: &[Sensor],
) -> Vec<Sensor> {
    let mut groups: Vec<Sensor> = Vec::new();
    for (label, now, max) in raw {
        let Some(now) = now.filter(|t| PLAUSIBLE_C.contains(t)) else {
            continue;
        };
        let Some(name) = display_name(&label) else {
            continue;
        };
        let peak = max
            .filter(|t| PLAUSIBLE_C.contains(t))
            .unwrap_or(now)
            .max(now);
        match groups.iter_mut().find(|g| g.name == name) {
            Some(g) => {
                g.now = g.now.max(now);
                g.peak = g.peak.max(peak);
            }
            None => groups.push(Sensor { name, now, peak }),
        }
    }
    for g in &mut groups {
        if let Some(p) = previous.iter().find(|p| p.name == g.name) {
            g.peak = g.peak.max(p.peak);
        }
    }
    groups.sort_by(|a, b| {
        b.now
            .partial_cmp(&a.now)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });
    groups
}

/// The row name for a platform label, or `None` for a label that is not a
/// temperature of anything.
fn display_name(label: &str) -> Option<String> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    if DROPPED.iter().any(|d| lower.starts_with(&d.to_lowercase())) {
        return None;
    }
    // Peel the label back to the thing it names: trailing digits, the
    // separators before them, and a trailing `temp`, which says nothing a
    // temperature panel has not already said — repeated, because
    // `iwlwifi_1 temp1` has one layer of each.
    let mut stem = trimmed;
    loop {
        let peeled = stem
            .trim_end_matches(|c: char| c.is_ascii_digit())
            .trim_end_matches([' ', '_', '-']);
        let peeled = strip_suffix_ci(peeled, "temp").trim_end_matches([' ', '_', '-']);
        if peeled == stem {
            break;
        }
        stem = peeled;
    }
    if stem.is_empty() {
        return Some(trimmed.to_string());
    }
    let stem_lower = stem.to_lowercase();
    for (prefix, name) in RENAMES {
        if stem_lower.starts_with(&prefix.to_lowercase()) {
            return Some((*name).to_string());
        }
    }
    Some(stem.to_string())
}

/// `text` without a trailing `suffix`, compared without case; unchanged when
/// it does not end with one.
fn strip_suffix_ci<'a>(text: &'a str, suffix: &str) -> &'a str {
    if text.len() >= suffix.len() && text.is_char_boundary(text.len() - suffix.len()) {
        let (head, tail) = text.split_at(text.len() - suffix.len());
        if tail.eq_ignore_ascii_case(suffix) {
            return head;
        }
    }
    text
}

/// Keys this panel responds to.
const BINDINGS: &[Binding] = &[Binding::primary("u", "units")];

impl Panel for TemperaturePanel {
    fn title(&self) -> String {
        "Temperature".to_string()
    }

    fn counter(&self) -> Option<String> {
        match self.sensors.len() {
            0 => None,
            1 => Some("1 sensor".to_string()),
            n => Some(format!("{n} sensors")),
        }
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn refresh_interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn tick(&mut self) -> bool {
        self.sample()
    }

    fn handle_key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> crate::panel::KeyOutcome {
        use ratatui::crossterm::event::KeyCode;
        if matches!(key.code, KeyCode::Char('u')) {
            self.celsius = !self.celsius;
            return crate::panel::KeyOutcome::Consumed;
        }
        crate::panel::KeyOutcome::Ignored
    }

    fn remember(&self, state: &mut crate::state::UiState) {
        state.temperature_units = Some(
            if self.celsius {
                "celsius"
            } else {
                "fahrenheit"
            }
            .into(),
        );
    }

    fn alert(&self) -> Option<crate::panel::Alert> {
        // A die past its limit is the one reading here that gets worse if
        // nobody acts — a throttled machine is the mild outcome, a shutdown
        // the other. Zero in the config switches it off.
        let limit = self.config.alert_above_c;
        let hottest = self.sensors.first()?;
        (limit > 0 && hottest.now > f32::from(limit)).then(|| {
            crate::panel::Alert::soon(format!(
                "{} at {:.0}{}",
                hottest.name,
                self.shown(hottest.now),
                self.unit()
            ))
        })
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.height == 0 || area.width == 0 {
            return;
        }
        let Some(hottest) = self.sensors.first().cloned() else {
            Self::draw_empty(frame, area, theme);
            return;
        };

        let gradient = &ctx.gradients.cpu;
        let track = Style::default().fg(theme.track);
        let muted = Style::default().fg(theme.muted);

        // The table takes at most half the panel and only when it can show a
        // header and a row; the graph keeps the rest.
        let wanted = u16::try_from(self.sensors.len() + 1).unwrap_or(u16::MAX);
        let table_rows = if area.height >= 5 {
            wanted.min(area.height / 2)
        } else {
            0
        };
        let table_rows = if table_rows >= 2 { table_rows } else { 0 };

        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(table_rows),
        ])
        .split(area);

        // The readout: the hottest thing, in the ramp's colour at its Celsius
        // value, then what it is, then how much history the graph holds. The
        // figure and its unit are one part; the name and the age drop whole.
        let colour = gradient.at(hottest.now.clamp(0.0, 100.0).round() as i64);
        frame.render_widget(
            Paragraph::new(crate::grid::assemble(
                vec![
                    vec![
                        Span::styled(
                            format!("{:.1}", self.shown(hottest.now)),
                            Style::default().fg(colour).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(self.unit(), muted),
                    ],
                    vec![Span::styled(
                        format!(" {}", crate::glyphs::utility(&hottest.name)),
                        Style::default()
                            .fg(theme.label)
                            .add_modifier(Modifier::BOLD),
                    )],
                    vec![Span::styled(
                        format!(
                            "   {}s",
                            self.history.len() as u64 * self.config.sample_secs.max(1)
                        ),
                        muted,
                    )],
                ],
                rows[0].width,
            )),
            rows[0],
        );

        self.graph_cells = rows[1].width as usize;
        if rows[1].height > 0 {
            let data: Vec<u64> = self.history.iter().copied().collect();
            BrailleGraph::new(&data, 100, gradient)
                .track_style(track)
                .render(rows[1], frame.buffer_mut());
        }

        if table_rows >= 2 {
            let grid = Grid::new(COLUMNS, rows[2].width);
            let mut y = rows[2].y;
            frame.render_widget(
                Paragraph::new(grid.header(theme)),
                Rect::new(rows[2].x, y, rows[2].width, 1),
            );
            y += 1;
            for sensor in self.sensors.iter().take(usize::from(table_rows) - 1) {
                let tone = gradient.at(sensor.now.clamp(0.0, 100.0).round() as i64);
                let line = grid.row(&[
                    Span::styled(sensor.name.clone(), Style::default().fg(theme.text)),
                    Span::styled(
                        format!("{:.0}°", self.shown(sensor.now)),
                        Style::default().fg(tone),
                    ),
                    Span::styled(format!("{:.0}°", self.shown(sensor.peak)), muted),
                ]);
                frame.render_widget(
                    Paragraph::new(line),
                    Rect::new(rows[2].x, y, rows[2].width, 1),
                );
                y += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn apple() -> Vec<(String, Option<f32>, Option<f32>)> {
        let mut raw = vec![
            ("PMU tcal".to_string(), Some(51.8), Some(51.8)),
            ("NAND CH0 temp".to_string(), Some(32.0), Some(32.0)),
            ("gas gauge battery".to_string(), Some(29.0), Some(29.0)),
        ];
        for i in 1..=14 {
            raw.push((
                format!("PMU tdie{i}"),
                Some(37.0 + i as f32 * 0.2),
                Some(40.0),
            ));
        }
        for i in 1..=8 {
            raw.push((format!("PMU tdev{i}"), Some(-9201.1), Some(0.0)));
        }
        raw
    }

    fn sensors() -> Vec<Sensor> {
        group(apple(), &[])
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    fn panel(sensors: Vec<Sensor>) -> TemperaturePanel {
        TemperaturePanel::with_sensors(TemperatureConfig::default(), sensors, &[38, 39, 40, 41, 40])
    }

    fn screen(panel: &mut TemperaturePanel, width: u16, height: u16) -> Vec<String> {
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

    /// Twenty-five platform entries become three rows a person would write:
    /// fourteen die sensors as one, the calibration reference gone, the
    /// eight impossible readings gone.
    #[test]
    fn apple_silicons_sensor_list_becomes_three_rows() {
        let rows = sensors();
        let names: Vec<&str> = rows.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["CPU die", "SSD", "Battery"], "{rows:?}");
        let die = &rows[0];
        assert!(
            (die.now - 39.8).abs() < 0.01,
            "the hottest of the fourteen: {die:?}"
        );
        assert!(
            (die.peak - 40.0).abs() < 0.01,
            "and the platform's own maximum: {die:?}"
        );
        assert!(
            rows.iter().all(|s| PLAUSIBLE_C.contains(&s.now)),
            "{rows:?}"
        );
    }

    #[test]
    fn linux_hwmon_labels_group_and_rename() {
        let raw = vec![
            ("coretemp Package id 0".to_string(), Some(52.0), Some(70.0)),
            ("coretemp Core 0".to_string(), Some(48.0), Some(66.0)),
            ("coretemp Core 1".to_string(), Some(51.0), Some(69.0)),
            ("nvme Composite".to_string(), Some(41.0), None),
            ("acpitz temp1".to_string(), Some(27.8), None),
            ("iwlwifi_1 temp1".to_string(), Some(45.0), None),
        ];
        let rows = group(raw, &[]);
        let names: Vec<&str> = rows.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["CPU package", "CPU core", "iwlwifi", "SSD", "Motherboard"],
            "hottest first, unknown labels kept minus their digits: {rows:?}"
        );
        assert!(close(rows[1].now, 51.0), "the hotter core");
        assert!(close(rows[1].peak, 69.0));
        assert!(
            close(rows[3].peak, 41.0),
            "no platform maximum: the peak is the reading"
        );
    }

    /// The peak is the panel's memory, not the platform's: a group that
    /// cooled since the last sample keeps the higher figure.
    #[test]
    fn the_peak_survives_a_sample_that_cooled() {
        let hot = group(vec![("PMU tdie1".to_string(), Some(80.0), None)], &[]);
        let cooler = group(vec![("PMU tdie1".to_string(), Some(50.0), None)], &hot);
        assert!(close(cooler[0].now, 50.0));
        assert!(close(cooler[0].peak, 80.0));
    }

    #[test]
    fn u_switches_units_and_is_remembered() {
        let mut p = panel(sensors());
        assert_eq!(p.unit(), "°C");
        let out = p.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        assert_eq!(out, crate::panel::KeyOutcome::Consumed);
        assert_eq!(p.unit(), "°F");
        assert!((p.shown(100.0) - 212.0).abs() < 0.01);
        let mut state = crate::state::UiState::default();
        p.remember(&mut state);
        assert_eq!(state.temperature_units.as_deref(), Some("fahrenheit"));
        assert!(
            BINDINGS.iter().any(|b| b.key == "u"),
            "the key is documented in the frame"
        );
    }

    #[test]
    fn the_alert_names_the_hottest_thing_past_the_limit_and_can_be_switched_off() {
        let hot = vec![Sensor {
            name: "CPU die".into(),
            now: 97.4,
            peak: 98.0,
        }];
        assert_eq!(
            panel(hot.clone()).alert().map(|a| a.text),
            Some("CPU die at 97°C".to_string())
        );
        assert!(
            panel(sensors()).alert().is_none(),
            "forty degrees is not news"
        );
        let off = TemperatureConfig {
            alert_above_c: 0,
            ..TemperatureConfig::default()
        };
        assert!(
            TemperaturePanel::with_sensors(off, hot, &[])
                .alert()
                .is_none()
        );
        assert!(panel(Vec::new()).alert().is_none());
    }

    #[test]
    fn the_border_counts_sensor_groups_not_platform_entries() {
        assert_eq!(panel(sensors()).counter().as_deref(), Some("3 sensors"));
        assert_eq!(
            panel(vec![Sensor {
                name: "SSD".into(),
                now: 40.0,
                peak: 40.0
            }])
            .counter()
            .as_deref(),
            Some("1 sensor")
        );
        assert_eq!(panel(Vec::new()).counter(), None);
    }

    /// Readout, graph, then the table with a header and every group under
    /// it — and no cell in the table left blank.
    #[test]
    fn the_face_reads_readout_graph_table() {
        let mut p = panel(sensors());
        let rows = screen(&mut p, 40, 10);
        assert!(rows[0].starts_with("39.8°C CPU DIE"), "{rows:?}");
        assert!(
            rows[0].ends_with("25s"),
            "five samples at five seconds: {rows:?}"
        );
        assert!(
            rows.iter()
                .any(|r| r.contains('⣀') || r.contains('⣿') || r.contains('⡀')),
            "a graph: {rows:?}"
        );
        let header = rows
            .iter()
            .position(|r| r.starts_with("SENSOR"))
            .expect("a table header");
        assert!(
            rows[header].contains("NOW") && rows[header].ends_with("PEAK"),
            "{rows:?}"
        );
        assert!(
            rows[header + 1].starts_with("CPU die") && rows[header + 1].ends_with("40°"),
            "{rows:?}"
        );
        assert!(rows[header + 2].starts_with("SSD"), "{rows:?}");
        assert!(rows[header + 3].starts_with("Battery"), "{rows:?}");
        for row in &rows[header + 1..header + 4] {
            assert!(
                row.matches('°').count() == 2,
                "now and peak both filled: {row:?}"
            );
        }
    }

    #[test]
    fn a_short_panel_keeps_the_graph_and_drops_the_table() {
        let mut p = panel(sensors());
        let rows = screen(&mut p, 40, 4);
        assert!(rows.iter().all(|r| !r.starts_with("SENSOR")), "{rows:?}");
        assert!(rows[0].starts_with("39.8°C"), "{rows:?}");
    }

    #[test]
    fn a_machine_with_no_sensors_says_so() {
        let mut p = panel(Vec::new());
        let rows = screen(&mut p, 40, 6);
        let text = rows.join("\n");
        assert!(text.contains("No temperature sensors"), "{rows:?}");
        assert!(!text.contains('⣀'), "no empty graph over nothing: {rows:?}");
    }
}
