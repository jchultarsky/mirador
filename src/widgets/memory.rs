//! Memory in use, as a live readout plus a scrolling history chart.
//!
//! The fourth quarter of "what compute is available": `cpu` and `network` had
//! answered the question for a year with the middle of every system monitor
//! missing. Built as `cpu` is — the same sampler cadence, the same braille
//! history, the same meter — because a reader who has learned one should not
//! have to learn the other. It draws from the cpu gradient rather than a ramp
//! of its own: the two are one instrument reading two gauges, and a fourth
//! gradient key would have been a line in nineteen theme files for a
//! distinction nobody would see.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use sysinfo::{MemoryRefreshKind, RefreshKind, System};

use crate::chart::{BrailleGraph, meter_spans};
use crate::config::MemoryConfig;
use crate::frame::Binding;
use crate::panel::{Panel, RenderContext};

/// The memory panel.
pub struct MemoryPanel {
    config: MemoryConfig,
    system: System,
    /// Recent used-percentage samples, oldest first.
    history: VecDeque<u64>,
    /// The most recent reading, in bytes.
    reading: Reading,
    /// `None` until the first sample, so it fires immediately.
    last_sample: Option<Instant>,
    /// Cells the graph was last drawn into, so the history can grow to fill it.
    graph_cells: usize,
}

/// One sample of the machine's memory, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Reading {
    used: u64,
    total: u64,
    swap_used: u64,
    swap_total: u64,
}

impl Reading {
    /// Used memory as a whole percentage of the total. A machine reporting no
    /// memory at all — a sandbox, a container with the file unmounted — reads
    /// as `0` rather than dividing by zero.
    fn used_pct(self) -> u64 {
        percent(self.used, self.total)
    }
}

/// `part` as a whole percentage of `whole`, saturating at 100 and reading `0`
/// for a whole of zero.
fn percent(part: u64, whole: u64) -> u64 {
    if whole == 0 {
        return 0;
    }
    ((u128::from(part.min(whole)) * 100) / u128::from(whole)) as u64
}

/// Bytes as gibibytes to one decimal, which is how memory is sold and how
/// every other monitor states it.
fn gib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_073_741_824.0)
}

/// Bytes as whole gibibytes, rounded, for the border counter.
fn whole_gib(bytes: u64) -> u64 {
    (bytes as f64 / 1_073_741_824.0).round() as u64
}

// `sysinfo::System` is deliberately opaque, so derive(Debug) is unavailable.
impl std::fmt::Debug for MemoryPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryPanel")
            .field("reading", &self.reading)
            .field("samples", &self.history.len())
            .finish_non_exhaustive()
    }
}

impl MemoryPanel {
    /// Build the panel and take a first sample.
    pub fn new(config: MemoryConfig) -> Self {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
        );
        system.refresh_memory();
        let reading = Self::read(&system);

        Self {
            history: VecDeque::with_capacity(config.history.max(1)),
            config,
            system,
            reading,
            last_sample: None,
            graph_cells: 0,
        }
    }

    /// A panel holding `reading`, with no `sysinfo` behind it, for tests in
    /// other modules — a reading is a reading, whichever machine took it.
    #[cfg(test)]
    pub(crate) fn with_reading(config: MemoryConfig, used: u64, total: u64) -> Self {
        let mut panel = Self::new(config);
        panel.reading = Reading {
            used,
            total,
            swap_used: 0,
            swap_total: 0,
        };
        panel.history.clear();
        panel.history.push_back(panel.reading.used_pct());
        panel
    }

    fn read(system: &System) -> Reading {
        Reading {
            used: system.used_memory(),
            total: system.total_memory(),
            swap_used: system.used_swap(),
            swap_total: system.total_swap(),
        }
    }

    /// Take a sample if enough time has passed since the last one. Returns
    /// whether it did, so a tick that changed nothing costs no repaint.
    fn sample(&mut self) -> bool {
        let interval = Duration::from_secs(self.config.sample_secs.max(1));
        if self.last_sample.is_some_and(|at| at.elapsed() < interval) {
            return false;
        }
        self.system.refresh_memory();
        self.reading = Self::read(&self.system);
        self.last_sample = Some(Instant::now());

        let capacity = crate::samples::capacity(self.config.history, self.graph_cells);
        crate::samples::push_bounded(&mut self.history, self.reading.used_pct(), capacity);
        true
    }
}

/// Keys this panel responds to.
const BINDINGS: &[Binding] = &[Binding::primary("s", "swap")];

impl Panel for MemoryPanel {
    fn title(&self) -> String {
        "Memory".to_string()
    }

    fn counter(&self) -> Option<String> {
        // Whole gigabytes: `64.0 GB` cost the title its last letters at 120
        // columns (`┤Memo…├`), and the decimal is in the readout below for
        // anyone who wants it.
        (self.reading.total > 0).then(|| format!("{} GB", whole_gib(self.reading.total)))
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn refresh_interval(&self) -> Duration {
        Duration::from_millis(500)
    }

    fn tick(&mut self) -> bool {
        self.sample()
    }

    fn handle_key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> crate::panel::KeyOutcome {
        use ratatui::crossterm::event::KeyCode;
        if matches!(key.code, KeyCode::Char('s')) {
            self.config.show_swap = !self.config.show_swap;
            return crate::panel::KeyOutcome::Consumed;
        }
        crate::panel::KeyOutcome::Ignored
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.height == 0 || area.width == 0 {
            return;
        }

        let gradient = &ctx.gradients.cpu;
        let track = Style::default().fg(theme.track);
        // Swap is worth a row only when the machine has some and the panel has
        // the height; a machine without swap has nothing to say about it.
        let show_swap = self.config.show_swap && self.reading.swap_total > 0 && area.height >= 5;

        let rows = Layout::vertical([
            Constraint::Length(1),                        // readout
            Constraint::Min(1),                           // graph
            Constraint::Length(u16::from(show_swap) * 2), // swap label + meter
        ])
        .split(area);

        let pct = self.reading.used_pct();
        let colour = gradient.at(i64::try_from(pct).unwrap_or(100));
        // Three parts, dropped from the end: the figure and its `%` are one
        // value; the word USED goes next; the absolute figures go first, since
        // a percentage in a panel titled Memory says the most in the least.
        frame.render_widget(
            Paragraph::new(crate::grid::assemble(
                vec![
                    vec![
                        Span::styled(
                            format!("{pct:>3}"),
                            Style::default().fg(colour).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("%", Style::default().fg(theme.muted)),
                    ],
                    vec![Span::styled(
                        format!(" {}", crate::glyphs::utility("used")),
                        Style::default()
                            .fg(theme.label)
                            .add_modifier(Modifier::BOLD),
                    )],
                    vec![Span::styled(
                        format!(
                            "   {} / {} GB",
                            gib(self.reading.used),
                            gib(self.reading.total)
                        ),
                        Style::default().fg(theme.muted),
                    )],
                ],
                rows[0].width,
            )),
            rows[0],
        );

        // Recorded so the next tick can size the buffer to the panel.
        self.graph_cells = rows[1].width as usize;

        if rows[1].height > 0 {
            let data: Vec<u64> = self.history.iter().copied().collect();
            BrailleGraph::new(&data, 100, gradient)
                .track_style(track)
                .render(rows[1], frame.buffer_mut());
        }

        if show_swap && rows[2].height >= 2 {
            let swap_pct = percent(self.reading.swap_used, self.reading.swap_total);
            frame.render_widget(
                Paragraph::new(crate::grid::assemble(
                    vec![
                        vec![Span::styled(
                            crate::glyphs::utility("swap"),
                            Style::default()
                                .fg(theme.label)
                                .add_modifier(Modifier::BOLD),
                        )],
                        vec![Span::styled(
                            format!(
                                "   {} / {} GB",
                                gib(self.reading.swap_used),
                                gib(self.reading.swap_total)
                            ),
                            Style::default().fg(theme.muted),
                        )],
                    ],
                    rows[2].width,
                )),
                Rect::new(rows[2].x, rows[2].y, rows[2].width, 1),
            );
            let y = rows[2].y + 1;
            for (index, (glyph, style)) in
                meter_spans(swap_pct, 100, rows[2].width, gradient, track)
                    .iter()
                    .enumerate()
            {
                let x = rows[2].x + u16::try_from(index).unwrap_or(u16::MAX);
                if x >= rows[2].x + rows[2].width {
                    break;
                }
                frame.buffer_mut()[(x, y)]
                    .set_char(*glyph)
                    .set_style(*style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percentage_saturates_and_never_divides_by_zero() {
        assert_eq!(percent(0, 0), 0, "no memory at all reads as nothing used");
        assert_eq!(percent(5, 0), 0);
        assert_eq!(percent(1, 4), 25);
        assert_eq!(percent(4, 4), 100);
        assert_eq!(
            percent(9, 4),
            100,
            "used past total saturates rather than overflowing"
        );
        assert_eq!(
            percent(u64::MAX, u64::MAX),
            100,
            "the widest inputs multiply without overflow"
        );
    }

    #[test]
    fn gibibytes_are_stated_to_one_decimal() {
        assert_eq!(gib(0), "0.0");
        assert_eq!(gib(1_073_741_824), "1.0");
        assert_eq!(gib(17_179_869_184), "16.0");
        assert_eq!(gib(6_657_199_308), "6.2");
    }

    /// Every line the panel builds by hand is assembled, so a narrow panel
    /// drops the absolute figures before the word and the word before the
    /// percentage — never a fragment of any of them. Swept against the sizes
    /// a narrow instrument row actually hands it.
    #[test]
    fn the_readout_drops_whole_values_and_never_a_fragment() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();
        let mut panel =
            MemoryPanel::with_reading(MemoryConfig::default(), 6_657_199_308, 17_179_869_184);

        for width in 1..=40u16 {
            let mut terminal = Terminal::new(TestBackend::new(width, 6)).unwrap();
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
            let readout: String = (0..width).map(|x| buffer[(x, 0)].symbol()).collect();
            let readout = readout.trim_end();
            // `assemble` abridges a first part that cannot fit at all with
            // `…`, which is the marked cut invariant 19 asks for; everything
            // else is whole values or nothing. What must never appear is an
            // unmarked fragment like `38` or `6.2 / 16`.
            let readout = readout.trim();
            assert!(
                readout.is_empty()
                    || readout.ends_with('\u{2026}')
                    || readout == "38%"
                    || readout == "38% USED"
                    || readout == "38% USED   6.2 / 16.0 GB",
                "at width {width} the readout was {readout:?}"
            );
        }
    }

    #[test]
    fn the_border_carries_the_total_and_nothing_when_there_is_none() {
        let panel = MemoryPanel::with_reading(MemoryConfig::default(), 1, 17_179_869_184);
        assert_eq!(panel.counter().as_deref(), Some("16 GB"));
        let odd = MemoryPanel::with_reading(MemoryConfig::default(), 1, 8_267_366_400);
        assert_eq!(
            odd.counter().as_deref(),
            Some("8 GB"),
            "7.7 GB rounds to the nearest whole"
        );
        let none = MemoryPanel::with_reading(MemoryConfig::default(), 0, 0);
        assert_eq!(none.counter(), None);
    }

    #[test]
    fn s_toggles_the_swap_row_and_is_documented() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut panel = MemoryPanel::with_reading(MemoryConfig::default(), 1, 2);
        let before = panel.config.show_swap;
        let outcome = panel.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert_eq!(outcome, crate::panel::KeyOutcome::Consumed);
        assert_ne!(panel.config.show_swap, before);
        assert!(
            BINDINGS.iter().any(|b| b.key == "s"),
            "the key the panel takes is the key it advertises"
        );
    }

    #[test]
    fn the_history_is_bounded_by_what_the_panel_can_draw() {
        let mut panel = MemoryPanel::with_reading(
            MemoryConfig {
                history: 4,
                ..MemoryConfig::default()
            },
            1,
            2,
        );
        panel.graph_cells = 0;
        for i in 0..50 {
            crate::samples::push_bounded(&mut panel.history, i, crate::samples::capacity(4, 0));
        }
        assert!(panel.history.len() <= 8, "bounded: {}", panel.history.len());
    }
}
