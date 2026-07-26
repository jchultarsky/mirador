//! CPU utilisation, as a live readout plus a scrolling history chart.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use sysinfo::{CpuRefreshKind, RefreshKind, System};

use crate::chart::{BrailleGraph, meter_spans};
use crate::config::CpuConfig;
use crate::frame::Binding;
use crate::panel::{Panel, RenderContext};

/// The CPU panel.
pub struct CpuPanel {
    config: CpuConfig,
    system: System,
    /// Recent average-utilisation samples, oldest first.
    history: VecDeque<u64>,
    /// Per-core utilisation from the most recent sample.
    per_core: Vec<f32>,
    current: f32,
    /// `None` until the first sample, so it fires immediately. See the note on
    /// `Slot::last_tick` for why this is not "now minus an hour".
    last_sample: Option<Instant>,
    core_count: usize,
    /// Cells the graph was last drawn into, so the history can grow to fill it.
    graph_cells: usize,
}

// `sysinfo::System` is deliberately opaque, so derive(Debug) is unavailable.
impl std::fmt::Debug for CpuPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpuPanel")
            .field("current", &self.current)
            .field("samples", &self.history.len())
            .field("cores", &self.core_count)
            .finish_non_exhaustive()
    }
}

impl CpuPanel {
    /// Build the panel and take a first sample.
    pub fn new(config: CpuConfig) -> Self {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing().with_cpu_usage()),
        );
        system.refresh_cpu_usage();
        let core_count = system.cpus().len();

        Self {
            history: VecDeque::with_capacity(config.history.max(1)),
            config,
            system,
            per_core: Vec::new(),
            current: 0.0,
            graph_cells: 0,
            // Back-date so the first tick samples immediately.
            last_sample: None,
            core_count,
        }
    }

    /// Take a sample if enough time has passed since the last one.
    fn sample(&mut self) {
        let interval = Duration::from_secs(self.config.sample_secs.max(1))
            // sysinfo needs a minimum gap between refreshes for its deltas to
            // be meaningful; sampling faster than this yields zeros.
            .max(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

        if self.last_sample.is_some_and(|at| at.elapsed() < interval) {
            return;
        }

        self.system.refresh_cpu_usage();
        self.current = self.system.global_cpu_usage();
        self.per_core = self
            .system
            .cpus()
            .iter()
            .map(sysinfo::Cpu::cpu_usage)
            .collect();
        self.core_count = self.per_core.len();
        self.last_sample = Some(Instant::now());

        let capacity = self.capacity();
        // A loop rather than a single pop: the capacity shrinks when the panel
        // is narrowed, and dropping one sample per tick would leave the buffer
        // oversized for as many ticks as the window lost cells.
        while self.history.len() >= capacity {
            self.history.pop_front();
        }
        self.history
            .push_back(self.current.clamp(0.0, 100.0).round() as u64);
    }

    /// How many samples to retain.
    ///
    /// `[cpu].history` is a floor, not a ceiling. The graph packs two samples
    /// into every cell and fills from the right, so a buffer of N samples can
    /// only ever cover N/2 cells — on a panel wider than that the graph stops
    /// short of its own left edge and the rest is dead space. Letting the
    /// buffer grow to the width fixes that, and the span readout beside the
    /// figure is computed from the live sample count, so it stays honest
    /// rather than continuing to claim the configured number.
    fn capacity(&self) -> usize {
        self.config.history.max(1).max(self.graph_cells * 2)
    }
}

/// Keys this panel responds to.
const BINDINGS: &[Binding] = &[Binding::primary("c", "per-core")];

impl Panel for CpuPanel {
    fn title(&self) -> String {
        "CPU".to_string()
    }

    fn counter(&self) -> Option<String> {
        (self.core_count > 0).then(|| format!("{} cores", self.core_count))
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn refresh_interval(&self) -> Duration {
        Duration::from_millis(500)
    }

    fn tick(&mut self) {
        self.sample();
    }

    fn handle_key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> crate::panel::KeyOutcome {
        use ratatui::crossterm::event::KeyCode;
        if matches!(key.code, KeyCode::Char('c')) {
            self.config.show_per_core = !self.config.show_per_core;
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
        let show_cores = self.config.show_per_core && area.height >= 5 && !self.per_core.is_empty();

        let rows = Layout::vertical([
            Constraint::Length(1),                         // readout
            Constraint::Min(1),                            // graph
            Constraint::Length(u16::from(show_cores) * 2), // per-core meters
        ])
        .split(area);

        // The number takes its colour from the same ramp as the graph, so the
        // whole panel changes temperature together.
        let colour = gradient.at(self.current.round() as i64);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{:>5.1}", self.current),
                    Style::default().fg(colour).add_modifier(Modifier::BOLD),
                ),
                Span::styled("% ", Style::default().fg(theme.muted)),
                Span::styled(
                    crate::glyphs::utility("load"),
                    Style::default()
                        .fg(theme.label)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "   {}s",
                        self.history.len() as u64 * self.config.sample_secs.max(1)
                    ),
                    Style::default().fg(theme.muted),
                ),
            ])),
            rows[0],
        );

        // Recorded so the next tick can size the buffer to the panel; see
        // `capacity`.
        self.graph_cells = rows[1].width as usize;

        if rows[1].height > 0 {
            let data: Vec<u64> = self.history.iter().copied().collect();
            BrailleGraph::new(&data, 100, gradient)
                .track_style(track)
                .render(rows[1], frame.buffer_mut());
        }

        if show_cores && rows[2].height >= 2 {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    crate::glyphs::utility("per core"),
                    Style::default()
                        .fg(theme.label)
                        .add_modifier(Modifier::BOLD),
                )),
                Rect::new(rows[2].x, rows[2].y, rows[2].width, 1),
            );

            // One meter per core, sharing the row. Below three columns each a
            // meter is unreadable, so fall back to a single-cell level mark.
            let cores = u16::try_from(self.per_core.len()).unwrap_or(1).max(1);
            let per = rows[2].width / cores;
            let y = rows[2].y + 1;
            let mut x = rows[2].x;
            for pct in &self.per_core {
                if x >= rows[2].x + rows[2].width {
                    break;
                }
                let value = pct.clamp(0.0, 100.0).round() as u64;
                if per >= 3 {
                    let cells = meter_spans(value, 100, per.saturating_sub(1), gradient, track);
                    for (index, (glyph, style)) in cells.iter().enumerate() {
                        let cx = x + u16::try_from(index).unwrap_or(0);
                        if cx >= rows[2].x + rows[2].width {
                            break;
                        }
                        frame.buffer_mut()[(cx, y)]
                            .set_char(*glyph)
                            .set_style(*style);
                    }
                    x += per;
                } else {
                    frame.buffer_mut()[(x, y)]
                        .set_char(level_mark(*pct))
                        .set_style(
                            Style::default().fg(gradient.at(i64::try_from(value).unwrap_or(100))),
                        );
                    x += 1;
                }
            }
        }
    }
}

/// A single-cell vertical level mark, used when a core has no room for a
/// full meter. NaN clamps to the floor rather than panicking on the cast.
fn level_mark(pct: f32) -> char {
    const BLOCKS: [char; 9] = ['▁', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let normalised = if pct.is_nan() {
        0.0
    } else {
        pct.clamp(0.0, 100.0)
    };
    let idx = ((normalised / 100.0) * (BLOCKS.len() - 1) as f32).round() as usize;
    BLOCKS[idx.min(BLOCKS.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_covers_the_full_range_without_panicking() {
        assert_eq!(level_mark(0.0), '▁');
        assert_eq!(level_mark(100.0), '█');
        // Out-of-range input must clamp rather than index out of bounds.
        assert_eq!(level_mark(-50.0), '▁');
        assert_eq!(level_mark(1000.0), '█');
        assert_eq!(level_mark(f32::NAN), '▁');
        for i in 0..=100 {
            let _ = level_mark(i as f32);
        }
    }

    #[test]
    fn meter_is_monotonic() {
        let mut previous = level_mark(0.0);
        for i in 0..=100 {
            let current = level_mark(i as f32);
            assert!(
                block_rank(current) >= block_rank(previous),
                "meter went backwards at {i}%"
            );
            previous = current;
        }
    }

    fn block_rank(block: char) -> usize {
        ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']
            .iter()
            .position(|b| *b == block)
            .unwrap_or(0)
    }

    #[test]
    fn history_is_bounded_by_the_configured_capacity() {
        let config = CpuConfig {
            history: 5,
            ..Default::default()
        };
        let mut panel = CpuPanel::new(config);
        for i in 0..50 {
            if panel.history.len() >= 5 {
                panel.history.pop_front();
            }
            panel.history.push_back(i);
        }
        assert_eq!(panel.history.len(), 5);
        assert_eq!(panel.history.front(), Some(&45));
    }

    #[test]
    #[ignore = "superseded: the graph now takes the whole history"]
    fn recent_never_over_reads_the_buffer() {
        let mut panel = CpuPanel::new(CpuConfig::default());
        panel.history.clear();
    }
}
