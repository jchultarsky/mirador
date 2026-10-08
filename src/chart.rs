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

use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

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
    ///
    /// Only a ramp with a true-colour stop is interpolated. A named or indexed
    /// colour is whatever the terminal says it is, so there is nothing to
    /// blend, and a ramp with one among its stops *steps* between them
    /// instead: halves for two stops, thirds for three. That keeps a theme
    /// written in the sixteen ANSI colours inside them, which is the only
    /// reason such a theme exists. The one exception is `black` and `white`
    /// beside a hex stop, which blend as `#000000` and `#ffffff` because they
    /// always have; a ramp of names alone steps, those two included.
    ///
    /// The steps are placed for the graphs, which colour each row by the top of
    /// its band, rounded: a two-row graph's rows top out at 50 and 100, a
    /// three-row graph's at 33, 67 and 100. So a step starts just above each
    /// boundary, and a short graph still shows every colour the ramp has.
    pub fn new(start: Color, mid: Option<Color>, end: Option<Color>) -> Self {
        let mut colors = Box::new([start; STEPS]);

        let Some(end) = end else {
            return Self { colors };
        };

        let true_colour = [Some(start), mid, Some(end)]
            .into_iter()
            .any(|stop| matches!(stop, Some(Color::Rgb(..))));
        let rgb = |stop| if true_colour { rgb(stop) } else { None };
        match (rgb(start), mid.map(rgb), rgb(end)) {
            (Some(a), Some(Some(m)), Some(b)) => {
                for (i, slot) in colors.iter_mut().enumerate().take(51) {
                    *slot = lerp(a, m, i, 50);
                }
                for i in 51..STEPS {
                    colors[i] = lerp(m, b, i - 50, 50);
                }
            }
            (Some(a), None, Some(b)) => {
                for (i, slot) in colors.iter_mut().enumerate() {
                    *slot = lerp(a, b, i, STEPS - 1);
                }
            }
            _ => match mid {
                Some(mid) => {
                    colors[34..=67].fill(mid);
                    colors[68..].fill(end);
                }
                None => colors[51..].fill(end),
            },
        }

        Self { colors }
    }

    /// A flat ramp; every level is the same colour. Tests use it to make a
    /// graph's shape checkable without a gradient in the way — nothing else
    /// does, which is why it is compiled only for them.
    #[cfg(test)]
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
    pub fn scaled(&self, value: u64, max: u64) -> Color {
        if max == 0 {
            return self.at(0);
        }
        let pct = (u128::from(value.min(max)) * 100 / u128::from(max)) as i64;
        self.at(pct)
    }
}

/// The RGB value of a stop in a ramp that has a true-colour stop, or `None`
/// when it has none to rely on.
///
/// Black and white are given `#000000` and `#ffffff` so that a ramp from
/// `black` to a hex colour still blends, as it always has — not because the
/// terminal agrees: both are palette entries, and a Solarized terminal's black
/// is `#073642`. So `Gradient::new` asks only when a stop is already true
/// colour, and a ramp written wholly in names steps, those two included.
/// Other named and indexed colours used to collapse to mid grey here so the
/// arithmetic stayed total, and every ramp in the bundled `ansi` and
/// `high-contrast` themes baked to that one grey, sent as a 24-bit escape by
/// the theme that exists to avoid them.
fn rgb(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Black => Some((0, 0, 0)),
        Color::White => Some((255, 255, 255)),
        _ => None,
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
    /// The samples, oldest first, as two runs read one after the other —
    /// the two halves a `VecDeque` keeps its ring in. A plain slice is the
    /// first run with an empty second.
    data: (&'a [u64], &'a [u64]),
    max: u64,
    gradient: &'a Gradient,
    track_style: Style,
}

impl<'a> BrailleGraph<'a> {
    /// A graph of `data` scaled to `max`, coloured by `gradient`.
    pub fn new(data: &'a [u64], max: u64, gradient: &'a Gradient) -> Self {
        Self {
            data: (data, &[]),
            max,
            gradient,
            track_style: Style::default(),
        }
    }

    /// A graph of a panel's history, read where it lies.
    ///
    /// Every history in the program is a `VecDeque` (`samples::push_bounded`),
    /// and each panel used to copy the whole of it into a `Vec` on every frame
    /// to hand `new` a slice — a copy sized by `[<panel>].history`, which
    /// nothing bounds above, where the graph reads at most two samples a
    /// cell. This borrows the deque's two halves instead, so a frame costs
    /// what is on screen and nothing for what is behind it.
    pub fn of_history(history: &'a VecDeque<u64>, max: u64, gradient: &'a Gradient) -> Self {
        Self {
            data: history.as_slices(),
            max,
            gradient,
            track_style: Style::default(),
        }
    }

    /// How many samples there are, across both runs.
    fn len(&self) -> usize {
        self.data.0.len() + self.data.1.len()
    }

    /// The sample at `index`, counting from the oldest across both runs.
    fn sample(&self, index: usize) -> Option<u64> {
        let (head, tail) = self.data;
        match index.checked_sub(head.len()) {
            None => head.get(index).copied(),
            Some(index) => tail.get(index).copied(),
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
        // Clip to the buffer before touching it. Indexing a `Buffer` out of
        // bounds panics, and every ratatui widget clamps for exactly this
        // reason — a graph handed an area larger than the screen should draw
        // what fits, not bring the dashboard down. No caller does that today;
        // the panels take their rects from the layout, which is derived from
        // the frame. This is here so that a future one cannot.
        let area = area.intersection(buf.area);
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

        let len = self.len();
        if len == 0 || self.max == 0 {
            return;
        }

        // Two samples per cell, right-aligned to the newest data. Only the
        // last `capacity` are ever read, by index, so nothing is copied.
        let capacity = width * 2;
        let start = len.saturating_sub(capacity);
        let visible = len - start;

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
                let Some(end) = visible.checked_sub(cell_from_right * 2) else {
                    // No data this far back; leave the track showing.
                    continue;
                };

                // The ratio in `u128`, as `Gradient::scaled` and `meter_spans`
                // take it. Each side used to be clamped to `u32::MAX` on its
                // own, so a scale above that — a disk past two gigabytes a
                // second — drew every sample too high. Hundredths of a per
                // cent, so the result is below 10,001 and converts exactly.
                let pct = |index: Option<usize>| -> Option<f64> {
                    let raw = self.sample(start + index?)?;
                    let basis_points =
                        u128::from(raw.min(self.max)) * 10_000 / u128::from(self.max);
                    Some(f64::from(u32::try_from(basis_points).unwrap_or(10_000)) / 100.0)
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

/// `part` as a whole percentage of `whole`, saturating at 100 and reading `0`
/// for a whole of zero — a machine reporting no memory or a volume reporting
/// no size reads as nothing used rather than dividing by zero. The product is
/// taken in `u128`, so the widest inputs cannot overflow it.
pub fn percent(part: u64, whole: u64) -> u16 {
    if whole == 0 {
        return 0;
    }
    // At most 100 by the `min`, so the conversion cannot fail.
    u16::try_from(u128::from(part.min(whole)) * 100 / u128::from(whole)).unwrap_or(100)
}

/// Lines of muted prose centred in `area`, for a panel with nothing else to
/// draw — no disks, no battery, no sensors. Each line is ellipsised to the
/// width, so a narrow panel shows a marked cut rather than the terminal's
/// silent one (invariant 19), and an empty line keeps its row without
/// drawing anything. Lines past the bottom of the area are not drawn.
pub fn draw_notice<S: AsRef<str>>(
    frame: &mut Frame,
    area: Rect,
    theme: &crate::theme::Theme,
    lines: &[S],
) {
    let count = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let top = area.y + area.height.saturating_sub(count) / 2;
    let style = Style::default().fg(theme.muted);
    for (y, text) in (top..area.bottom()).zip(lines) {
        let text = text.as_ref();
        if text.is_empty() {
            continue;
        }
        frame.render_widget(
            Paragraph::new(Span::styled(
                crate::grid::truncate(text, usize::from(area.width)),
                style,
            ))
            .centered(),
            Rect::new(area.x, y, area.width, 1),
        );
    }
}

/// A flat meter: `percent` of `width` cells filled in one colour, the rest in
/// the track colour. The pomodoro's progress bar and the battery's charge bar
/// are both this; the graded version, coloured by level, is [`meter_spans`].
pub fn meter_line(percent: u16, width: u16, fill: Color, track: Color) -> Vec<Span<'static>> {
    let width = usize::from(width);
    let filled = usize::from(percent.min(100)) * width / 100;
    (0..width)
        .map(|i| {
            Span::styled(
                "■",
                Style::default().fg(if i < filled { fill } else { track }),
            )
        })
        .collect()
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

    /// Indexing a `Buffer` out of bounds panics. Nothing passes an oversized
    /// area today — the panels take their rects from the layout — but a graph
    /// asked to draw outside the screen should draw what fits.
    #[test]
    fn an_area_larger_than_the_buffer_is_clipped_rather_than_panicking() {
        let gradient = Gradient::new(c(0, 0, 0), None, Some(c(255, 0, 0)));
        let data: Vec<u64> = (0..200).collect();

        for (bw, bh) in [(1u16, 1u16), (10, 4), (40, 12)] {
            for (aw, ah) in [(1u16, 1u16), (40, 12), (500, 200)] {
                let mut buf = Buffer::empty(Rect::new(0, 0, bw, bh));
                BrailleGraph::new(&data, 199, &gradient).render(Rect::new(0, 0, aw, ah), &mut buf);
            }
        }

        // And an area that starts outside the buffer entirely.
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        BrailleGraph::new(&data, 199, &gradient).render(Rect::new(50, 50, 10, 4), &mut buf);
    }

    /// Values and a scale at the top of `u64` must not wrap or divide badly.
    #[test]
    fn saturated_values_still_draw() {
        let gradient = Gradient::new(c(0, 0, 0), None, Some(c(255, 0, 0)));
        for (data, max) in [
            (vec![u64::MAX; 60], u64::MAX),
            (vec![u64::MAX; 60], 1),
            (vec![0u64; 60], u64::MAX),
            (vec![0u64; 60], 0),
        ] {
            let mut buf = Buffer::empty(Rect::new(0, 0, 20, 4));
            BrailleGraph::new(&data, max, &gradient).render(Rect::new(0, 0, 20, 4), &mut buf);
            // "Still draw" has to mean something: a track is always painted,
            // so an all-blank buffer would be the graph giving up quietly.
            let painted = buf.content.iter().any(|cell| cell.symbol() != " ");
            assert!(painted, "nothing drawn for data={:?} max={max}", &data[..1]);
        }
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
            let now = rgb(g.at(level)).expect("hex stops bake to true colour");
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
        // A band with no height has no inside: everything is either at the top
        // or on the floor. The old version of this test called `level` once and
        // discarded the result, on the theory that low == high would divide by
        // zero — it cannot, because both early returns fire before the
        // division, and a float division would not panic anyway. So it passed
        // whatever the function did.
        assert_eq!(level(10.0, 10.0, 10.0, 0.1, 0), 4, "at the band is the top");
        assert_eq!(level(11.0, 10.0, 10.0, 0.1, 0), 4, "above it is the top");
        assert_eq!(level(9.0, 10.0, 10.0, 0.1, 0), 0, "below it is the floor");
        assert_eq!(
            level(9.0, 10.0, 10.0, 0.1, 1),
            1,
            "and the floor is honoured"
        );
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

        let mut buf = Buffer::empty(area);
        BrailleGraph::of_history(&VecDeque::new(), 100, &gradient).render(area, &mut buf);
    }

    /// A history drawn where it lies in its deque is the same picture as the
    /// same samples copied out into one slice, wherever the ring happens to
    /// break — which is the case the copy existed to hide. Every rotation of
    /// a full ring is tried, at widths that show part of it, exactly all of
    /// it and more than there is, and the ring is checked to be broken in
    /// two so the second run is really read.
    #[test]
    fn a_history_is_drawn_from_its_deque_as_it_would_be_from_a_copy() {
        let gradient = Gradient::new(c(0, 255, 0), Some(c(255, 255, 0)), Some(c(255, 0, 0)));
        let mut ring: VecDeque<u64> = VecDeque::with_capacity(48);
        let full = ring.capacity();
        ring.extend((0..full as u64).map(|i| i * 37 % 101));
        let mut broken = 0;
        for step in 0..full as u64 {
            ring.pop_front();
            ring.push_back(step * 53 % 101);
            assert_eq!(ring.capacity(), full, "the ring must not reallocate");
            if !ring.as_slices().1.is_empty() && !ring.as_slices().0.is_empty() {
                broken += 1;
            }
            let copy: Vec<u64> = ring.iter().copied().collect();
            for width in [1u16, 5, 20, (full / 2) as u16, 40, 70] {
                let area = Rect::new(0, 0, width, 3);
                let mut from_ring = Buffer::empty(area);
                BrailleGraph::of_history(&ring, 100, &gradient).render(area, &mut from_ring);
                let mut from_copy = Buffer::empty(area);
                BrailleGraph::new(&copy, 100, &gradient).render(area, &mut from_copy);
                assert_eq!(from_ring, from_copy, "rotated {step}, {width} wide");
            }
        }
        assert!(
            broken > full / 2,
            "the ring broke in two only {broken} times"
        );
    }

    /// One routine, where the memory and disk panels each had a copy that
    /// differed only in the type it returned.
    #[test]
    fn a_percentage_saturates_and_never_divides_by_zero() {
        assert_eq!(percent(0, 0), 0, "nothing at all reads as nothing used");
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
        assert_eq!(percent(u64::MAX - 1, u64::MAX), 99, "and round down");
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
        let spans = meter_spans(5, 10, 0, &g, Style::default());
        assert!(spans.is_empty(), "{spans:?}");
        assert_eq!(meter_spans(5, 0, 4, &g, Style::default()).len(), 4);
    }

    /// A scale above `u32::MAX` — a disk reading five gigabytes a second, whose
    /// combined graph is scaled to twice that — must draw each sample at its
    /// own height. Thirty per cent of a four-row graph is the bottom row and a
    /// little of the next; clamping the scale to `u32::MAX` while leaving the
    /// sample alone drew it at seventy.
    #[test]
    fn a_scale_beyond_u32_draws_samples_at_their_true_height() {
        let gradient = Gradient::flat(c(1, 1, 1));
        let area = Rect::new(0, 0, 10, 4);
        let mut buf = Buffer::empty(area);
        BrailleGraph::new(&[3_000_000_000; 40], 10_000_000_000, &gradient).render(area, &mut buf);

        let row = |y: u16| -> String { (0..10).map(|x| buf[(x, y)].symbol()).collect() };
        for y in [0, 1] {
            assert!(
                row(y).chars().all(|c| c == ' '),
                "30% reached row {y} of 4: `{}`",
                row(y)
            );
        }
        assert!(
            row(2).chars().any(|c| c != ' '),
            "30% must rise past the bottom quarter, got `{}`",
            row(2)
        );
    }

    /// Named stops have no RGB value mirador can rely on — the terminal
    /// decides — so interpolating them meant collapsing them to a grey first.
    /// A ramp of names steps between the names instead.
    #[test]
    fn named_colour_stops_keep_their_colours_rather_than_baking_to_grey() {
        let three = Gradient::new(Color::Green, Some(Color::Yellow), Some(Color::Red));
        assert_eq!(three.at(0), Color::Green);
        assert_eq!(three.at(33), Color::Green);
        assert_eq!(three.at(34), Color::Yellow);
        assert_eq!(three.at(67), Color::Yellow);
        assert_eq!(three.at(68), Color::Red);
        assert_eq!(three.at(100), Color::Red);

        let two = Gradient::new(Color::Indexed(28), None, Some(Color::LightRed));
        assert_eq!(two.at(0), Color::Indexed(28));
        assert_eq!(two.at(50), Color::Indexed(28));
        assert_eq!(two.at(51), Color::LightRed);
        assert_eq!(two.at(100), Color::LightRed);
    }

    /// `black` and `white` are palette entries like any other name — a
    /// Solarized terminal's black is `#073642` — so a ramp written wholly in
    /// names stays in names even when those are the two it uses. Beside a hex
    /// stop they still blend as `#000000` and `#ffffff`, as they always did.
    #[test]
    fn black_and_white_blend_only_beside_a_true_colour() {
        for (ramp, gradient) in [
            ("two", Gradient::new(Color::Black, None, Some(Color::White))),
            (
                "three",
                Gradient::new(Color::White, Some(Color::Black), Some(Color::White)),
            ),
        ] {
            for level in 0..=100 {
                assert!(
                    !matches!(gradient.at(level), Color::Rgb(..)),
                    "the {ramp} ramp at {level} is {:?}, a true-colour escape",
                    gradient.at(level)
                );
            }
        }
        let two = Gradient::new(Color::Black, None, Some(Color::White));
        assert_eq!(two.at(50), Color::Black);
        assert_eq!(two.at(51), Color::White);

        let mixed = Gradient::new(Color::Black, None, Some(Color::Rgb(200, 0, 0)));
        assert_eq!(mixed.at(50), Color::Rgb(100, 0, 0));
        let mixed = Gradient::new(
            Color::Rgb(0, 0, 200),
            Some(Color::White),
            Some(Color::Black),
        );
        assert_eq!(mixed.at(25), Color::Rgb(127, 127, 227));
        assert_eq!(mixed.at(100), Color::Rgb(0, 0, 0));
    }

    /// A graph colours each row by the top of its band, rounded, so a stepped
    /// ramp whose steps sat on those tops showed a three-row graph only two of
    /// its three colours, and a two-row graph only one.
    #[test]
    fn a_short_graph_shows_every_colour_a_stepped_ramp_has() {
        let three = Gradient::new(Color::Green, Some(Color::Yellow), Some(Color::Red));
        let two = Gradient::new(Color::Green, None, Some(Color::Red));
        for (gradient, rows, want) in [
            (&three, 3u16, vec![Color::Red, Color::Yellow, Color::Green]),
            (&two, 2, vec![Color::Red, Color::Green]),
        ] {
            let data = [100u64; 20];
            let area = Rect::new(0, 0, 10, rows);
            let mut buf = Buffer::empty(area);
            BrailleGraph::new(&data, 100, gradient).render(area, &mut buf);
            let colours: Vec<Color> = (0..rows).map(|y| buf[(9, y)].fg).collect();
            assert_eq!(colours, want, "{rows} rows");
        }
    }

    /// `ansi` exists to work on a terminal with no true colour, and
    /// `high-contrast` inherits it for the same reason. A single baked level
    /// that came out as `Color::Rgb` is a 24-bit escape that theme promised
    /// never to send — and every level did, in one grey, through 1.19.2.
    #[test]
    fn the_ansi_themes_draw_their_graphs_in_the_terminals_own_colours() {
        let ansi = crate::themes::resolve("ansi", None).unwrap().gradients();
        assert_eq!(ansi.cpu.at(0), Color::Green);
        assert_eq!(ansi.cpu.at(100), Color::Red);

        for name in ["ansi", "high-contrast"] {
            let g = crate::themes::resolve(name, None).unwrap().gradients();
            for (ramp, gradient) in [
                ("cpu", &g.cpu),
                ("rx", &g.rx),
                ("tx", &g.tx),
                ("gain", &g.gain),
                ("loss", &g.loss),
            ] {
                for level in 0..=100 {
                    assert!(
                        !matches!(gradient.at(level), Color::Rgb(..)),
                        "`{name}` {ramp} at {level} is {:?}, a true-colour escape",
                        gradient.at(level)
                    );
                }
            }
        }
    }

    /// The cpu ramp is how a busy machine looks different from an idle one,
    /// and every bundled theme names a different colour for each end of it.
    #[test]
    fn every_bundled_theme_has_a_cpu_ramp_that_changes_colour() {
        for name in crate::themes::bundled_names() {
            let g = crate::themes::resolve(name, None).unwrap().gradients();
            assert_ne!(
                g.cpu.at(0),
                g.cpu.at(100),
                "`{name}` draws an idle and a saturated cpu in one colour"
            );
        }
    }
}
