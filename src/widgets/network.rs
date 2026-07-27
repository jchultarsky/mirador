//! Network throughput, as two scrolling charts: received and transmitted.
//!
//! Rates are computed from byte deltas divided by the real elapsed time between
//! samples, not by the configured interval, so a stalled or slow tick reports
//! the correct rate instead of an inflated one.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use sysinfo::Networks;

use crate::chart::BrailleGraph;
use crate::config::NetworkConfig;
use crate::frame::Binding;
use crate::panel::{Panel, RenderContext};
use crate::samples::push_bounded;

/// The network panel.
pub struct NetworkPanel {
    config: NetworkConfig,
    networks: Networks,
    /// Bytes per second received, oldest first.
    rx_history: VecDeque<u64>,
    /// Bytes per second transmitted, oldest first.
    tx_history: VecDeque<u64>,
    rx_rate: u64,
    tx_rate: u64,
    rx_total: u64,
    tx_total: u64,
    last_sample: Instant,
    /// False until the second sample, since the first has no delta to compare.
    primed: bool,
    /// Cells each graph was last drawn into, so the buffers can grow to fill.
    graph_cells: usize,
}

impl std::fmt::Debug for NetworkPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkPanel")
            .field("rx_rate", &self.rx_rate)
            .field("tx_rate", &self.tx_rate)
            .field("samples", &self.rx_history.len())
            .finish_non_exhaustive()
    }
}

impl NetworkPanel {
    /// Build the panel and prime the interface list.
    pub fn new(config: NetworkConfig) -> Self {
        let networks = Networks::new_with_refreshed_list();
        Self {
            rx_history: VecDeque::with_capacity(config.history.max(1)),
            tx_history: VecDeque::with_capacity(config.history.max(1)),
            config,
            networks,
            rx_rate: 0,
            tx_rate: 0,
            graph_cells: 0,
            rx_total: 0,
            tx_total: 0,
            last_sample: Instant::now(),
            primed: false,
        }
    }

    /// Whether an interface should be counted.
    fn includes(&self, name: &str) -> bool {
        if self.config.interfaces.is_empty() {
            !is_loopback(name)
        } else {
            self.config.interfaces.iter().any(|want| want == name)
        }
    }

    /// Sample byte counters and convert them to a rate.
    fn sample(&mut self) {
        let interval = Duration::from_secs(self.config.sample_secs.max(1));
        let elapsed = self.last_sample.elapsed();
        if elapsed < interval {
            return;
        }

        self.networks.refresh(true);

        let mut rx_delta = 0u64;
        let mut tx_delta = 0u64;
        let mut rx_total = 0u64;
        let mut tx_total = 0u64;

        for (name, data) in &self.networks {
            if !self.includes(name) {
                continue;
            }
            rx_delta = rx_delta.saturating_add(data.received());
            tx_delta = tx_delta.saturating_add(data.transmitted());
            rx_total = rx_total.saturating_add(data.total_received());
            tx_total = tx_total.saturating_add(data.total_transmitted());
        }

        // Divide by the time that actually elapsed, not the nominal interval.
        let seconds = elapsed.as_secs_f64().max(0.001);
        self.rx_rate = (rx_delta as f64 / seconds) as u64;
        self.tx_rate = (tx_delta as f64 / seconds) as u64;
        self.rx_total = rx_total;
        self.tx_total = tx_total;
        self.last_sample = Instant::now();

        // The first refresh after construction reports the counters since the
        // list was built, which is not a meaningful rate. Drop it.
        if !self.primed {
            self.primed = true;
            self.rx_rate = 0;
            self.tx_rate = 0;
            return;
        }

        let capacity = self.capacity();
        push_bounded(&mut self.rx_history, self.rx_rate, capacity);
        push_bounded(&mut self.tx_history, self.tx_rate, capacity);
    }

    /// How many samples to retain; see [`crate::samples::capacity`].
    fn capacity(&self) -> usize {
        crate::samples::capacity(self.config.history, self.graph_cells)
    }

    /// A shared ceiling for both charts so rx and tx stay visually comparable.
    fn scale(&self) -> u64 {
        self.rx_history
            .iter()
            .chain(self.tx_history.iter())
            .copied()
            .max()
            .unwrap_or(1)
            .max(1)
    }
}

/// Recognise loopback interfaces across platforms.
fn is_loopback(name: &str) -> bool {
    name == "lo" || name == "lo0" || name.starts_with("Loopback")
}

/// Format a byte count as a human-readable rate, e.g. `1.2 MB/s`.
pub fn format_rate(bytes_per_sec: u64) -> String {
    format!("{}/s", format_bytes(bytes_per_sec))
}

/// Format a byte count with a binary-ish scale and one decimal place.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Keys this panel responds to.
const BINDINGS: &[Binding] = &[];

impl Panel for NetworkPanel {
    fn title(&self) -> String {
        "Network".to_string()
    }

    fn counter(&self) -> Option<String> {
        if self.config.interfaces.is_empty() {
            Some("all".to_string())
        } else {
            Some(self.config.interfaces.join(" "))
        }
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

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.height == 0 || area.width == 0 {
            return;
        }

        let rx_color = ctx.gradients.rx.at(100);
        let tx_color = ctx.gradients.tx.at(100);
        let track = Style::default().fg(theme.track);

        // Two stacked graphs when there is room; one combined readout when not.
        let rows = Layout::vertical([
            Constraint::Length(1),                           // readout
            Constraint::Min(1),                              // rx graph
            Constraint::Min(1),                              // tx graph
            Constraint::Length(u16::from(area.height >= 6)), // totals
        ])
        .split(area);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("\u{2193} ", Style::default().fg(rx_color)),
                Span::styled(
                    format!("{:>10}", format_rate(self.rx_rate)),
                    Style::default().fg(rx_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled("   \u{2191} ", Style::default().fg(tx_color)),
                Span::styled(
                    format!("{:>10}", format_rate(self.tx_rate)),
                    Style::default().fg(tx_color).add_modifier(Modifier::BOLD),
                ),
            ])),
            rows[0],
        );

        // A shared ceiling keeps the two graphs visually comparable: a tall
        // upload spike should not look like a tall download spike at 1% the
        // rate.
        let scale = self.scale();

        // Recorded so the next tick sizes the buffers to the panel.
        self.graph_cells = rows[1].width as usize;

        let rx: Vec<u64> = self.rx_history.iter().copied().collect();
        let tx: Vec<u64> = self.tx_history.iter().copied().collect();
        for (index, (data, gradient)) in [(&rx, &ctx.gradients.rx), (&tx, &ctx.gradients.tx)]
            .into_iter()
            .enumerate()
        {
            let row = rows[index + 1];
            if row.height == 0 {
                continue;
            }
            BrailleGraph::new(data, scale, gradient)
                .track_style(track)
                .render(row, frame.buffer_mut());
        }

        if rows[3].height > 0 {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        crate::glyphs::utility("session"),
                        Style::default()
                            .fg(theme.label)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "  \u{2193} {}  \u{2191} {}   peak {}",
                            format_bytes(self.rx_total),
                            format_bytes(self.tx_total),
                            format_rate(scale)
                        ),
                        Style::default().fg(theme.muted),
                    ),
                ])),
                rows[3],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_format_across_every_unit() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_bytes(1024u64.pow(4)), "1.0 TB");
    }

    #[test]
    fn very_large_values_stay_in_the_top_unit_instead_of_overflowing() {
        let huge = format_bytes(u64::MAX);
        assert!(huge.ends_with("TB"), "got: {huge}");
    }

    #[test]
    fn rates_are_suffixed() {
        assert_eq!(format_rate(0), "0 B/s");
        assert_eq!(format_rate(2048), "2.0 KB/s");
    }

    #[test]
    fn loopback_is_recognised_on_every_platform() {
        assert!(is_loopback("lo"));
        assert!(is_loopback("lo0"));
        assert!(is_loopback("Loopback Pseudo-Interface 1"));
        assert!(!is_loopback("en0"));
        assert!(!is_loopback("eth0"));
        assert!(!is_loopback("wlan0"));
    }

    #[test]
    fn an_empty_interface_list_means_everything_but_loopback() {
        let panel = NetworkPanel::new(NetworkConfig::default());
        assert!(panel.includes("en0"));
        assert!(!panel.includes("lo0"));
    }

    #[test]
    fn an_explicit_interface_list_is_exclusive() {
        let panel = NetworkPanel::new(NetworkConfig {
            interfaces: vec!["en0".into()],
            ..Default::default()
        });
        assert!(panel.includes("en0"));
        assert!(!panel.includes("en1"));
        // An explicit list wins even for loopback.
        let lo = NetworkPanel::new(NetworkConfig {
            interfaces: vec!["lo0".into()],
            ..Default::default()
        });
        assert!(lo.includes("lo0"));
    }

    #[test]
    fn push_bounded_evicts_the_oldest_sample() {
        let mut buffer = VecDeque::new();
        for i in 0..10 {
            push_bounded(&mut buffer, i, 3);
        }
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer, VecDeque::from(vec![7, 8, 9]));
    }

    #[test]
    fn scale_is_never_zero_so_the_chart_cannot_divide_by_it() {
        let panel = NetworkPanel::new(NetworkConfig::default());
        assert_eq!(panel.scale(), 1, "an empty history must still yield 1");
    }

    #[test]
    fn scale_spans_both_series() {
        let mut panel = NetworkPanel::new(NetworkConfig::default());
        panel.rx_history.extend([10, 20]);
        panel.tx_history.extend([5, 99]);
        assert_eq!(panel.scale(), 99);
    }

    #[test]
    #[ignore = "superseded: the graph now takes the whole history"]
    fn recent_never_over_reads() {
        let history = VecDeque::from(vec![1, 2, 3]);
        let _ = history;
    }
}
