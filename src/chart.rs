//! Braille history graphs and the colour gradients that fill them.
//!
//! Both ideas are taken from bpytop/btop++, which solved two problems worth
//! copying exactly.
//!
//! **Resolution.** A braille cell is a 2x4 dot matrix, so one character column
//! holds two time samples and one character row resolves four levels. A graph
//! `w` cells wide by `h` tall shows `2w` samples at `4h` vertical steps.
//!
//! **Colour.** On a multi-row graph the gradient runs *vertically, by
//! magnitude* — one colour per row, hot at the top. It is deliberately not
//! per-sample: a static colour profile means the picture does not shimmer as
//! data scrolls past, which is what makes a graph tolerable to leave on screen
//! all day. On a single-row graph there is no vertical axis to encode
//! magnitude, so the gradient runs horizontally by value instead.
//!
//! The same gradient also colours the numeric readout, so a hot CPU turns red
//! in the number and the graph at the same instant.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

/// Number of entries in a baked gradient: one per percentage point.
const STEPS: usize = 101;

/// A pre-computed colour ramp indexed 0..=100.
///
/// Baking the ramp once means drawing a frame is array lookups rather than
/// per-cell interpolation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gradient {
    colors: Box<[Color; STEPS]>,
}

impl Gradient {
    /// A ramp from `start` through an optional `mid` to an optional `end`.
    ///
    /// With no `end` the ramp is flat. With `start` and `end` it is one linear
    /// segment. With all three, 0..=50 runs start to mid and 50..=100 runs mid
    /// to end, matching btop's two-segment model.
    pub fn new(start: Color, mid: Option<Color>, end: Option<Color>) -> Self {
        let mut colors = Box::new([start; STEPS]);

        let Some(end) = end else {
            return Self { colors };
        };

        let (a, b) = (rgb(start), rgb(end));
        match mid.map(rgb) {
            Some(m) => {
                for (i, slot) in colors.iter_mut().enumerate().take(51) {
                    *slot = lerp(a, m, i, 50);
                }
                for i in 51..STEPS {
                    colors[i] = lerp(m, b, i - 50, 50);
                }
            }
            None => {
                for (i, slot) in colors.iter_mut().enumerate() {
                    *slot = lerp(a, b, i, STEPS - 1);
                }
            }
        }

        Self { colors }
    }

    /// A flat ramp; every level is the same colour.
    #[allow(dead_code)] // Used by tests and by themes that disable a gradient.
    pub fn flat(color: Color) -> Self {
        Self {
            colors: Box::new([color; STEPS]),
        }
    }

    /// The colour at `level`, clamped to 0..=100.
    pub fn at(&self, level: i64) -> Color {
        let index = level.clamp(0, 100) as usize;
        self.colors[index]
    }

    /// The colour for `value` on a 0..=`max` scale. A zero `max` yields the
    /// bottom of the ramp rather than dividing by zero.
    #[allow(dead_code)] // Wanted by the watchlist panel, which is not yet built.
    pub fn scaled(&self, value: u64, max: u64) -> Color {
        if max == 0 {
            return self.at(0);
        }
        let pct = (u128::from(value.min(max)) * 100 / u128::from(max)) as i64;
        self.at(pct)
    }
}

/// Resolve a `Color` to RGB so it can be interpolated.
///
/// Named and indexed colours have no true RGB value we can rely on — the
/// terminal decides what they look like — so they collapse to mid grey. In
/// practice gradients are configured with hex values; this only keeps the
/// arithmetic total.
fn rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::White => (255, 255, 255),
        _ => (128, 128, 128),
    }
}

/// Linear per-channel interpolation in raw sRGB.
fn lerp(a: (u8, u8, u8), b: (u8, u8, u8), step: usize, span: usize) -> Color {
    if span == 0 {
        return Color::Rgb(a.0, a.1, a.2);
    }
    // Spans here are always the gradient's 100 steps, so these conversions
    // cannot realistically fail; saturating keeps the function total anyway.
    let step = i32::try_from(step.min(span)).unwrap_or(i32::MAX);
    let span = i32::try_from(span).unwrap_or(i32::MAX);
    let channel = |from: u8, to: u8| -> u8 {
        let from = i32::from(from);
        let to = i32::from(to);
        u8::try_from(from + (to - from) * step / span).unwrap_or(u8::MAX)
    };
    Color::Rgb(channel(a.0, b.0), channel(a.1, b.1), channel(a.2, b.2))
}

// ---------------------------------------------------------------------------
// Braille glyph tables
// ---------------------------------------------------------------------------

/// Dot bitmasks for the left column of a cell, filling upward from the bottom,
/// indexed by level 0..=4.
const UP_LEFT: [u8; 5] = [0x00, 0x40, 0x44, 0x46, 0x47];
/// Dot bitmasks for the right column of a cell, filling upward.
const UP_RIGHT: [u8; 5] = [0x00, 0x80, 0xA0, 0xB0, 0xB8];

/// The braille character for a left and right fill level.
fn braille(left: usize, right: usize) -> char {
    let mask = UP_LEFT[left.min(4)] | UP_RIGHT[right.min(4)];
    char::from_u32(0x2800 | u32::from(mask)).unwrap_or(' ')
}

/// The faint dotted baseline drawn under an empty graph, so an idle panel
/// reads as "present and working" rather than as broken. U+28C0, two low dots.
const TRACK: char = '⣀';

/// Quantise `value` to 0..=4 within the band a given row covers.
///
/// `bias` nudges small values up so that a 1% sample still lights a dot rather
/// than rounding away to nothing.
fn level(value: f64, low: f64, high: f64, bias: f64, floor: usize) -> usize {
    if value >= high {
        return 4;
    }
    if value <= low {
        return floor;
    }
    let span = (high - low).max(f64::EPSILON);
    let scaled = ((value - low) * 4.0 / span + bias).round();
    (scaled.max(0.0) as usize).clamp(floor, 4)
}

/// A history graph rendered with braille cells.
#[derive(Debug)]
pub struct BrailleGraph<'a> {
    data: &'a [u64],
    max: u64,
    gradient: &'a Gradient,
    track_style: Style,
}

impl<'a> BrailleGraph<'a> {
    /// A graph of `data` scaled to `max`, coloured by `gradient`.
    pub fn new(data: &'a [u64], max: u64, gradient: &'a Gradient) -> Self {
        Self {
            data,
            max,
            gradient,
            track_style: Style::default(),
        }
    }

    /// Style for the dotted baseline behind the data.
    pub fn track_style(mut self, style: Style) -> Self {
        self.track_style = style;
        self
    }

    /// Draw into `area`.
    ///
    /// The most recent samples are kept: a graph narrower than the history
    /// shows the right-hand end of it, which is the part anyone cares about.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let width = area.width as usize;
        let height = area.height as usize;

        // Lay down the track first; data overdraws it. Only the bottom row
        // carries dots: a full field of them would compete with the data.
        for row in 0..height {
            let y = area.y + row as u16;
            let symbol = if row == height - 1 { TRACK } else { ' ' };
            for col in 0..width {
                buf[(area.x + col as u16, y)]
                    .set_char(symbol)
                    .set_style(self.track_style);
            }
        }

        if self.data.is_empty() || self.max == 0 {
            return;
        }

        // Two samples per cell, right-aligned to the newest data.
        let capacity = width * 2;
        let start = self.data.len().saturating_sub(capacity);
        let visible = &self.data[start..];

        // A single row cannot encode magnitude vertically, so widen the
        // rounding bias to keep small values visible.
        let bias = if height == 1 { 0.3 } else { 0.1 };

        for row in 0..height {
            // Row 0 is the top of the graph and the hot end of the ramp.
            let band_high = 100.0 * (height - row) as f64 / height as f64;
            let band_low = 100.0 * (height - row - 1) as f64 / height as f64;
            let color = self.gradient.at((band_high.round() as i64).min(100));
            let style = Style::default().fg(color);
            let y = area.y + row as u16;
            // Keep a baseline dot on the bottom row so a flat-zero graph still
            // draws a line rather than vanishing.
            let floor = usize::from(row == height - 1);

            for col in 0..width {
                // Fill from the right so the newest sample sits at the edge,
                // which is where the eye goes on a scrolling graph.
                let cell_from_right = width - 1 - col;
                let Some(end) = visible.len().checked_sub(cell_from_right * 2) else {
                    // No data this far back; leave the track showing.
                    continue;
                };

                let pct = |index: Option<usize>| -> Option<f64> {
                    let raw = *visible.get(index?)?;
                    Some(
                        f64::from(u32::try_from(raw.min(self.max)).unwrap_or(u32::MAX)) * 100.0
                            / f64::from(u32::try_from(self.max).unwrap_or(u32::MAX)),
                    )
                };

                let (Some(left), Some(right)) = (pct(end.checked_sub(2)), pct(end.checked_sub(1)))
                else {
                    continue;
                };

                let l = level(left, band_low, band_high, bias, floor);
                let r = level(right, band_low, band_high, bias, floor);
                if l == 0 && r == 0 {
                    continue;
                }
                buf[(area.x + col as u16, y)]
                    .set_char(braille(l, r))
                    .set_style(style);
            }
        }
    }
}

/// A horizontal bar meter.
///
/// The gradient is indexed by each cell's *position*, not by the value, so a
/// bar at 40% shows the cool end of the ramp and a bar at 95% runs the whole
/// way to hot. The unfilled tail keeps the same glyph in the track colour, so
/// the meter's footprint never changes as the value moves.
pub fn meter_spans(
    value: u64,
    max: u64,
    width: u16,
    gradient: &Gradient,
    track: Style,
) -> Vec<(char, Style)> {
    let width = width as usize;
    if width == 0 {
        return Vec::new();
    }
    let filled = if max == 0 {
        0
    } else {
        ((u128::from(value.min(max)) * width as u128) / u128::from(max)) as usize
    };

    (0..width)
        .map(|i| {
            if i < filled {
                let pct = i64::try_from((i + 1) * 100 / width).unwrap_or(100);
                ('■', Style::default().fg(gradient.at(pct)))
            } else {
                ('■', track)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(r: u8, g: u8, b: u8) -> Color {
        Color::Rgb(r, g, b)
    }

    #[test]
    fn a_flat_gradient_is_the_same_everywhere() {
        let g = Gradient::flat(c(10, 20, 30));
        for level in [0, 1, 50, 99, 100] {
            assert_eq!(g.at(level), c(10, 20, 30));
        }
    }

    #[test]
    fn a_two_stop_gradient_hits_both_ends_exactly() {
        let g = Gradient::new(c(0, 0, 0), None, Some(c(100, 200, 250)));
        assert_eq!(g.at(0), c(0, 0, 0));
        assert_eq!(g.at(100), c(100, 200, 250));
    }

    #[test]
    fn a_three_stop_gradient_passes_through_its_midpoint() {
        let g = Gradient::new(c(0, 0, 0), Some(c(50, 50, 50)), Some(c(255, 255, 255)));
        assert_eq!(g.at(0), c(0, 0, 0));
        assert_eq!(g.at(50), c(50, 50, 50));
        assert_eq!(g.at(100), c(255, 255, 255));
    }

    #[test]
    fn a_gradient_is_monotonic_per_channel() {
        let g = Gradient::new(c(0, 0, 0), Some(c(120, 60, 30)), Some(c(255, 255, 255)));
        let mut previous = (0u8, 0u8, 0u8);
        for level in 0..=100 {
            let now = rgb(g.at(level));
            assert!(now.0 >= previous.0, "red went backwards at {level}");
            previous = now;
        }
    }

    #[test]
    fn gradient_levels_clamp_rather_than_panic() {
        let g = Gradient::new(c(0, 0, 0), None, Some(c(255, 255, 255)));
        assert_eq!(g.at(-50), g.at(0));
        assert_eq!(g.at(9_999), g.at(100));
    }

    #[test]
    fn scaled_handles_a_zero_maximum() {
        let g = Gradient::new(c(0, 0, 0), None, Some(c(255, 255, 255)));
        assert_eq!(g.scaled(5, 0), g.at(0), "must not divide by zero");
        assert_eq!(g.scaled(50, 100), g.at(50));
        assert_eq!(g.scaled(500, 100), g.at(100), "over-max must clamp");
    }

    #[test]
    fn scaled_does_not_overflow_on_huge_values() {
        let g = Gradient::flat(c(1, 2, 3));
        // Network byte counters get very large; the maths must not wrap.
        assert_eq!(g.scaled(u64::MAX, u64::MAX), g.at(100));
    }

    #[test]
    fn braille_masks_match_the_expected_characters() {
        assert_eq!(braille(0, 0), '\u{2800}');
        assert_eq!(braille(4, 4), '⣿');
        assert_eq!(braille(1, 0), '⡀');
        assert_eq!(braille(0, 1), '⢀');
        assert_eq!(braille(2, 2), '⣤');
    }

    #[test]
    fn braille_levels_clamp_out_of_range_input() {
        assert_eq!(braille(99, 99), braille(4, 4));
    }

    #[test]
    fn braille_output_is_always_in_the_braille_block() {
        for l in 0..=4 {
            for r in 0..=4 {
                let ch = braille(l, r) as u32;
                assert!((0x2800..=0x28FF).contains(&ch), "{l},{r} escaped the block");
            }
        }
    }

    #[test]
    fn level_saturates_and_floors() {
        assert_eq!(level(100.0, 0.0, 100.0, 0.1, 0), 4);
        assert_eq!(level(0.0, 0.0, 100.0, 0.1, 0), 0);
        // The bottom-row floor keeps a baseline visible at zero.
        assert_eq!(level(0.0, 0.0, 100.0, 0.1, 1), 1);
    }

    #[test]
    fn the_rounding_bias_rescues_values_just_above_a_band_floor() {
        // Within a band, an eighth of the way up would round to zero dots on a
        // bare `.round()`; the bias is what lifts it to one.
        // 11% of a full-height band is 0.44 dots: bare rounding loses it.
        assert_eq!(level(11.0, 0.0, 100.0, 0.0, 0), 0);
        assert_eq!(level(11.0, 0.0, 100.0, 0.1, 0), 1);
    }

    #[test]
    fn a_value_below_a_bands_floor_draws_nothing_in_that_band() {
        // 1% belongs in the bottom band of a tall graph, not in the top one.
        // The upper rows must stay empty or the graph would read as solid.
        assert_eq!(level(1.0, 75.0, 100.0, 0.1, 0), 0);
    }

    #[test]
    fn level_handles_a_degenerate_band() {
        // low == high would divide by zero without the epsilon guard.
        let _ = level(5.0, 10.0, 10.0, 0.1, 0);
    }

    #[test]
    fn graph_renders_without_panicking_at_any_size() {
        let gradient = Gradient::new(c(0, 255, 0), Some(c(255, 255, 0)), Some(c(255, 0, 0)));
        let data: Vec<u64> = (0..200u64).map(|i| i % 101).collect();

        for (w, h) in [(0, 0), (1, 1), (1, 8), (40, 1), (80, 6), (200, 20)] {
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            BrailleGraph::new(&data, 100, &gradient).render(area, &mut buf);
        }
    }

    #[test]
    fn graph_survives_empty_data_and_a_zero_maximum() {
        let gradient = Gradient::flat(c(1, 1, 1));
        let area = Rect::new(0, 0, 20, 4);

        let mut buf = Buffer::empty(area);
        BrailleGraph::new(&[], 100, &gradient).render(area, &mut buf);

        let mut buf = Buffer::empty(area);
        BrailleGraph::new(&[1, 2, 3], 0, &gradient).render(area, &mut buf);
    }

    #[test]
    fn an_idle_graph_still_draws_a_baseline() {
        let gradient = Gradient::flat(c(1, 1, 1));
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        BrailleGraph::new(&[0; 20], 100, &gradient).render(area, &mut buf);

        let bottom: String = (0..10).map(|x| buf[(x, 2)].symbol()).collect();
        assert!(
            bottom.chars().any(|c| c != ' '),
            "the bottom row must show a baseline, got `{bottom}`"
        );
    }

    #[test]
    fn a_full_graph_reaches_the_top_row() {
        let gradient = Gradient::flat(c(1, 1, 1));
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        BrailleGraph::new(&[100; 40], 100, &gradient).render(area, &mut buf);

        let top: String = (0..10).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(
            top.contains('⣿'),
            "a saturated graph must fill the top row, got `{top}`"
        );
    }

    #[test]
    fn the_newest_sample_lands_at_the_right_edge() {
        let gradient = Gradient::flat(c(1, 1, 1));
        let area = Rect::new(0, 0, 4, 1);
        let mut buf = Buffer::empty(area);
        // All zeros except the final pair, which is full scale.
        let mut data = vec![0u64; 20];
        data[18] = 100;
        data[19] = 100;
        BrailleGraph::new(&data, 100, &gradient).render(area, &mut buf);

        assert_eq!(
            buf[(3, 0)].symbol(),
            "⣿",
            "newest data belongs on the right"
        );
    }

    #[test]
    fn meter_fills_proportionally_and_keeps_its_width() {
        let g = Gradient::flat(c(1, 1, 1));
        let track = Style::default();
        for value in [0u64, 25, 50, 100] {
            let cells = meter_spans(value, 100, 10, &g, track);
            assert_eq!(cells.len(), 10, "the meter footprint must be constant");
        }
        assert_eq!(meter_spans(0, 100, 10, &g, track).len(), 10);
        assert!(
            meter_spans(100, 100, 10, &g, track)
                .iter()
                .all(|(c, _)| *c == '■')
        );
    }

    #[test]
    fn meter_handles_zero_width_and_zero_maximum() {
        let g = Gradient::flat(c(1, 1, 1));
        assert!(meter_spans(5, 10, 0, &g, Style::default()).is_empty());
        assert_eq!(meter_spans(5, 0, 4, &g, Style::default()).len(), 4);
    }
}
