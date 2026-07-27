//! A small column grid, so tabular data reads as a table.
//!
//! Every panel that lists things uses this. The point is not decoration: a
//! column that starts at a different place on every row cannot be scanned
//! vertically, and a column with no name leaves the reader guessing what the
//! number means. Headers are set in the utility face — bold uppercase — so they
//! read as labels rather than as another row of data.
//!
//! Widths are resolved once per draw against the available space. Fixed columns
//! take what they ask for; flexible columns share what is left in proportion to
//! their weight. When space runs short, columns are dropped from the *end* of
//! the optional list rather than every column being squeezed into illegibility.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// Display width of `text` in terminal cells.
///
/// Not the same as the character count, and the difference is not academic: a
/// weather glyph like `☀` or an emoji occupies two cells in most terminals, so
/// counting characters silently shifts every column after it and the values
/// end up under the wrong headers.
pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Cut `text` down to `width` terminal cells, ending in `…` if anything was
/// dropped.
///
/// Measured in cells for the same reason as [`display_width`]: budgeting by
/// `chars().count()` means one CJK glyph or emoji per two cells of overflow,
/// which is enough to push the text through the panel's own border.
///
/// The result never exceeds `width`, but may fall one cell short of it when a
/// double-width character would not fit — there is no half a cell to give
/// back. Callers that need an exact width should pad, as [`fit`] does.
pub fn truncate(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if display_width(text) <= width {
        return text.to_string();
    }

    // Take characters while they fit, keeping one cell back for the ellipsis.
    let budget = width - 1;
    let mut out = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let w = char_width(c);
        if used + w > budget {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Display width of a single character, treating unprintables as zero-width.
fn char_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Rows that `text` occupies once word-wrapped to `width` cells.
///
/// Needed wherever something is sized to fit its own contents — a scroll
/// clamp, or a dialog that must not be shorter than the text inside it. The
/// unwrapped line count is not a substitute: it is right until a line is
/// longer than the box, and then it is short by exactly the amount that
/// matters.
///
/// Matches `Paragraph::new(text).wrap(Wrap { trim: false })`, whose own
/// `line_count` is private. An empty line still occupies a row.
/// Break `text` into lines no wider than `width` display cells.
///
/// The companion to [`wrapped_height`], which counts the same rows without
/// producing them. `wrap(t, w).len()` and `wrapped_height(t, w)` are held equal
/// by a test — they were written from the same rules and would otherwise drift,
/// which for a panel that sizes itself from one and draws with the other means
/// content sitting outside the box it was measured for.
///
/// Measured in cells rather than characters, per the rule the whole module
/// exists for: a headline with an em dash or an accent breaks in the wrong
/// place otherwise.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    for line in text.lines() {
        let mut current = String::new();
        let mut used = 0usize;

        for word in line.split_inclusive(' ') {
            let w = display_width(word);
            if used > 0 && used + w > width {
                out.push(std::mem::take(&mut current));
                used = 0;
            }
            current.push_str(word);
            used += w;

            // A single word longer than the line breaks inside itself, rather
            // than running off the edge.
            while used > width {
                let keep: String = current
                    .chars()
                    .scan(0usize, |taken, c| {
                        *taken += display_width(&c.to_string());
                        (*taken <= width).then_some(c)
                    })
                    .collect();
                let rest = current[keep.len()..].to_string();
                out.push(keep);
                current = rest;
                used = display_width(&current);
            }
        }
        out.push(current);
    }
    // `wrapped_height` floors at one row, on the grounds that an empty line
    // still occupies one. These two are held equal by a test, so this has to
    // agree — and `"".lines()` yields nothing at all.
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

pub fn wrapped_height(text: &str, width: u16) -> u16 {
    if width == 0 {
        return 0;
    }
    let width = usize::from(width);
    let mut rows: usize = 0;

    for line in text.lines() {
        let mut used = 0usize;
        let mut rows_here = 1usize;
        for word in line.split_inclusive(' ') {
            let w = display_width(word);
            if used > 0 && used + w > width {
                rows_here += 1;
                used = w;
            } else {
                used += w;
            }
            // A single word longer than the line wraps within itself.
            while used > width {
                rows_here += 1;
                used -= width;
            }
        }
        rows += rows_here;
    }

    u16::try_from(rows.max(1)).unwrap_or(u16::MAX)
}

/// How a column is sized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    /// Exactly this many columns.
    Fixed(u16),
    /// A share of the leftover space, proportional to this weight.
    Flex(u16),
}

/// Which edge a cell's content is aligned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// One column of a grid.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    /// Header text. Rendered uppercased and bold.
    pub label: &'static str,
    pub width: Width,
    pub align: Align,
    /// The narrowest total grid width at which this column is worth showing.
    /// Zero means it is never dropped.
    pub min_total: u16,
}

impl Column {
    /// A left-aligned column that flexes to fill space.
    pub const fn flex(label: &'static str, weight: u16) -> Self {
        Self {
            label,
            width: Width::Flex(weight),
            align: Align::Left,
            min_total: 0,
        }
    }

    /// A fixed-width, left-aligned column.
    pub const fn fixed(label: &'static str, width: u16) -> Self {
        Self {
            label,
            width: Width::Fixed(width),
            align: Align::Left,
            min_total: 0,
        }
    }

    /// Right-align this column. Numbers and dates belong on the right so their
    /// last digits line up.
    pub const fn right(mut self) -> Self {
        self.align = Align::Right;
        self
    }

    /// Drop this column when the grid is narrower than `total`.
    pub const fn drops_below(mut self, total: u16) -> Self {
        self.min_total = total;
        self
    }
}

/// Columns resolved against a concrete width.
#[derive(Debug, Clone)]
pub struct Grid {
    /// The columns that survived the width budget, each with its resolved
    /// width and the index it held in the caller's declaration.
    ///
    /// The declared index is what keeps a row's cells under the right headers.
    /// Indexing the caller's cells by *surviving* position instead means that
    /// dropping one column slides every value after it one header to the left
    /// — a task's tags appearing under DUE, a forecast's feels-like under RAIN.
    resolved: Vec<(Column, u16, usize)>,
}

/// Space between adjacent columns.
const GUTTER: u16 = 1;

impl Grid {
    /// Resolve `columns` against a total width.
    pub fn new(columns: &[Column], total: u16) -> Self {
        // Drop optional columns that this width cannot justify, narrowest
        // threshold last, so the most valuable columns survive.
        let kept: Vec<(usize, Column)> = columns
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, c)| c.min_total == 0 || total >= c.min_total)
            .collect();

        if kept.is_empty() {
            return Self {
                resolved: Vec::new(),
            };
        }

        let gutters = GUTTER * u16::try_from(kept.len().saturating_sub(1)).unwrap_or(0);
        let fixed: u16 = kept
            .iter()
            .filter_map(|(_, c)| match c.width {
                Width::Fixed(w) => Some(w),
                Width::Flex(_) => None,
            })
            .sum();

        let flexible: u16 = kept
            .iter()
            .filter_map(|(_, c)| match c.width {
                Width::Flex(w) => Some(w.max(1)),
                Width::Fixed(_) => None,
            })
            .sum();

        let spare = total.saturating_sub(fixed).saturating_sub(gutters);

        let mut resolved = Vec::with_capacity(kept.len());
        let mut handed_out = 0u16;
        let flex_count = kept
            .iter()
            .filter(|(_, c)| matches!(c.width, Width::Flex(_)))
            .count();
        let mut flex_seen = 0usize;

        for (declared, column) in kept {
            let width = match column.width {
                Width::Fixed(w) => w,
                Width::Flex(weight) => {
                    flex_seen += 1;
                    if flex_seen == flex_count {
                        // The last flexible column absorbs the rounding
                        // remainder, so the grid always fills its width exactly.
                        spare.saturating_sub(handed_out)
                    } else {
                        let portion = spare
                            .checked_mul(weight.max(1))
                            .and_then(|scaled| scaled.checked_div(flexible))
                            .unwrap_or(0);
                        handed_out += portion;
                        portion
                    }
                }
            };
            resolved.push((column, width, declared));
        }

        Self { resolved }
    }

    /// Whether any column survived.
    pub fn is_empty(&self) -> bool {
        self.resolved.is_empty()
    }

    /// Whether the column with this label is being drawn.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn has(&self, label: &str) -> bool {
        self.resolved
            .iter()
            .any(|(c, w, _)| c.label == label && *w > 0)
    }

    /// The resolved width of a column, or zero if it was dropped.
    ///
    /// For panels that draw something sized to a column — a sparkline, a bar —
    /// and would otherwise have to re-derive the width the grid already
    /// computed. Two copies of that arithmetic drift, and the symptom is a
    /// column that silently renders empty.
    pub fn column_width(&self, label: &str) -> u16 {
        self.resolved
            .iter()
            .find(|(c, _, _)| c.label == label)
            .map_or(0, |(_, w, _)| *w)
    }

    /// The header row.
    ///
    /// Every column gets the same treatment, which used to require deciding
    /// tracking once for the whole row: per column it produced `DONE PRI
    /// T A S K`, where the mixed treatment reads as an accident. Now that the
    /// utility face is bold rather than letterspaced it costs no extra width,
    /// so there is no longer a fit to fail and nothing to decide.
    pub fn header(&self, theme: &Theme) -> Line<'static> {
        let style = Style::default()
            .fg(theme.label)
            .add_modifier(Modifier::BOLD);

        let mut spans = Vec::new();
        for (index, (column, width, _)) in self.resolved.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(" ".repeat(GUTTER as usize)));
            }
            if *width == 0 {
                continue;
            }
            let text = crate::glyphs::utility(column.label);
            spans.push(Span::styled(fit(&text, *width, column.align), style));
        }
        Line::from(spans)
    }

    /// A data row. Extra cells are ignored; missing cells render blank.
    pub fn row(&self, cells: &[Span<'_>]) -> Line<'static> {
        let mut spans = Vec::new();
        for (index, (column, width, declared)) in self.resolved.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(" ".repeat(GUTTER as usize)));
            }
            if *width == 0 {
                continue;
            }
            // Indexed by the column's declared position, not its surviving
            // one, so a dropped column takes its own value with it rather than
            // shifting every later value under the wrong header.
            let (content, style) = match cells.get(*declared) {
                Some(span) => (span.content.as_ref(), span.style),
                None => ("", Style::default()),
            };
            spans.push(Span::styled(fit(content, *width, column.align), style));
        }
        Line::from(spans)
    }
}

/// Pad or truncate `text` to exactly `width` terminal cells.
///
/// Measured in display width rather than characters, so a cell containing a
/// double-width glyph still occupies exactly its column. Truncation appends an
/// ellipsis, so a long value degrades visibly instead of silently pushing its
/// neighbours sideways.
fn fit(text: &str, width: u16, align: Align) -> String {
    let width = width as usize;
    if width == 0 {
        return String::new();
    }

    let actual = display_width(text);
    if actual > width {
        let mut out = truncate(text, width);
        // A double-width character can leave the result a cell short.
        out.push_str(&" ".repeat(width.saturating_sub(display_width(&out))));
        return out;
    }

    let pad = " ".repeat(width - actual);
    match align {
        Align::Left => format!("{text}{pad}"),
        Align::Right => format!("{pad}{text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two must agree: a panel sizes itself with one and draws with the
    /// other, so a disagreement puts content outside the box measured for it.
    #[test]
    fn wrapping_produces_exactly_as_many_rows_as_it_measures() {
        let samples = [
            "Wildfire now nine miles away from the French city of Bordeaux, mayor warns",
            "short",
            "",
            "a supercalifragilisticexpialidociousandthensomeword in a line",
            "Europa Clipper returns its first images — and they are extraordinary",
            "line one\nline two is a good deal longer than the first one is",
        ];
        for text in samples {
            for width in 8..40u16 {
                assert_eq!(
                    u16::try_from(wrap(text, usize::from(width)).len()).unwrap(),
                    wrapped_height(text, width),
                    "`{text}` at {width}"
                );
            }
        }
    }

    #[test]
    fn no_wrapped_line_is_wider_than_asked_for() {
        let text = "Wildfire now nine miles away from the French city of Bordeaux";
        for width in 8..40usize {
            for line in wrap(text, width) {
                assert!(
                    display_width(&line) <= width,
                    "`{line}` is wider than {width}"
                );
            }
        }
    }

    fn columns() -> Vec<Column> {
        vec![
            Column::fixed("hour", 5),
            Column::flex("sky", 1),
            Column::fixed("temp", 6).right(),
            Column::fixed("rain", 5).right().drops_below(40),
        ]
    }

    fn width_of(line: &Line<'_>) -> usize {
        line.spans.iter().map(|s| display_width(&s.content)).sum()
    }

    #[test]
    fn a_grid_fills_its_width_exactly() {
        for total in [20u16, 33, 40, 80, 200] {
            let grid = Grid::new(&columns(), total);
            let header = grid.header(&Theme::default());
            assert_eq!(
                width_of(&header),
                total as usize,
                "header did not fill width {total}"
            );
        }
    }

    #[test]
    fn every_row_is_the_same_width_as_the_header() {
        let grid = Grid::new(&columns(), 60);
        let header = width_of(&grid.header(&Theme::default()));
        for cells in [
            vec![],
            vec![Span::raw("14:00")],
            vec![
                Span::raw("14:00"),
                Span::raw("partly cloudy"),
                Span::raw("72°"),
                Span::raw("20%"),
            ],
            // More cells than columns must not widen the row.
            vec![
                Span::raw("a"),
                Span::raw("b"),
                Span::raw("c"),
                Span::raw("d"),
                Span::raw("e"),
                Span::raw("f"),
            ],
        ] {
            assert_eq!(width_of(&grid.row(&cells)), header);
        }
    }

    #[test]
    fn optional_columns_drop_when_the_grid_is_narrow() {
        let wide = Grid::new(&columns(), 80);
        assert!(wide.has("rain"));

        let narrow = Grid::new(&columns(), 30);
        assert!(!narrow.has("rain"), "rain should drop below its threshold");
        assert!(narrow.has("hour"), "required columns must survive");
    }

    #[test]
    fn a_grid_with_no_room_still_produces_a_full_width_row() {
        // Fixed columns can exceed a tiny total; the row must still not
        // overflow the area it is drawn into by more than its declared width.
        let grid = Grid::new(&columns(), 4);
        let row = grid.row(&[Span::raw("14:00")]);
        assert!(width_of(&row) >= 4);
    }

    #[test]
    fn an_empty_column_list_yields_an_empty_grid() {
        let grid = Grid::new(&[], 40);
        assert!(grid.is_empty());
        assert_eq!(width_of(&grid.row(&[Span::raw("x")])), 0);
    }

    #[test]
    fn flex_columns_split_the_leftover_space_by_weight() {
        let cols = [
            Column::fixed("a", 10),
            Column::flex("b", 1),
            Column::flex("c", 3),
        ];
        let grid = Grid::new(&cols, 50);
        let widths: Vec<u16> = grid.resolved.iter().map(|(_, w, _)| *w).collect();
        assert_eq!(widths[0], 10);
        // 50 - 10 fixed - 2 gutters = 38 spare, split 1:3.
        assert_eq!(widths[1] + widths[2], 38);
        assert!(
            widths[2] > widths[1] * 2,
            "weight 3 should dominate weight 1"
        );
    }

    #[test]
    fn fit_pads_and_aligns() {
        assert_eq!(fit("ab", 5, Align::Left), "ab   ");
        assert_eq!(fit("ab", 5, Align::Right), "   ab");
        assert_eq!(fit("", 3, Align::Left), "   ");
        assert_eq!(fit("abc", 3, Align::Left), "abc");
    }

    #[test]
    fn fit_truncates_with_an_ellipsis_on_char_boundaries() {
        assert_eq!(fit("abcdef", 4, Align::Left), "abc…");
        // Each CJK character is two cells, so only one fits before the
        // ellipsis in a three-cell column.
        assert_eq!(display_width(&fit("日本語テスト", 3, Align::Left)), 3);
        assert_eq!(fit("abc", 1, Align::Left), "…");
        assert_eq!(fit("abc", 0, Align::Left), "");
    }

    #[test]
    fn fit_output_is_always_exactly_the_requested_width() {
        // Includes double-width glyphs, which are the whole reason this is
        // measured in cells rather than characters.
        for text in [
            "",
            "a",
            "hello",
            "a much longer value",
            "日本語テスト",
            "☀ clear",
            "⛈ thunderstorm",
            "🦀 rust",
        ] {
            for width in 0..14u16 {
                let out = fit(text, width, Align::Left);
                assert_eq!(
                    display_width(&out),
                    width as usize,
                    "`{text}` at width {width} produced `{out}`"
                );
            }
        }
    }

    #[test]
    fn a_double_width_glyph_does_not_shift_the_columns_after_it() {
        let cols = [Column::fixed("sky", 12), Column::fixed("temp", 6).right()];
        let grid = Grid::new(&cols, 19);
        let plain = grid.row(&[Span::raw("clear"), Span::raw("63°F")]);
        let wide = grid.row(&[Span::raw("☀ clear"), Span::raw("63°F")]);
        assert_eq!(
            width_of(&plain),
            width_of(&wide),
            "a wide glyph must not change the row width"
        );
    }

    #[test]
    fn a_dropped_column_takes_its_own_value_with_it() {
        // The bug this pins: row cells were indexed by surviving position, so
        // dropping a middle column slid every later value one header to the
        // left — a forecast's feels-like appearing under RAIN, a task's tags
        // under DUE. Silent, and exactly what this module exists to prevent.
        let cols = [
            Column::fixed("a", 4),
            Column::fixed("b", 4).drops_below(100),
            Column::fixed("c", 4),
        ];
        let cells = [Span::raw("AAA"), Span::raw("BBB"), Span::raw("CCC")];

        // Wide: everything shows, in order.
        let wide = Grid::new(&cols, 100);
        let text: String = wide
            .row(&cells)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("AAA") && text.contains("BBB") && text.contains("CCC"));

        // Narrow: `b` is dropped. `c` must stay under its own header, and
        // `BBB` must not appear anywhere.
        let narrow = Grid::new(&cols, 20);
        assert!(!narrow.has("b"), "the test needs `b` dropped");
        let text: String = narrow
            .row(&cells)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            !text.contains("BBB"),
            "a dropped column's value must not be rendered: `{text}`"
        );
        assert!(text.contains("AAA") && text.contains("CCC"), "got `{text}`");

        // And the value sits under the right heading: compare cell positions
        // in the header and the row.
        let header: String = narrow
            .header(&Theme::default())
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(
            header.find('C'),
            text.find("CCC"),
            "`CCC` is not under the `C` header: header `{header}` row `{text}`"
        );
    }

    #[test]
    fn headers_are_uppercased_and_bold_rather_than_letterspaced() {
        let grid = Grid::new(&[Column::fixed("hour", 12)], 12);
        let header = grid.header(&Theme::default());
        let text: String = header.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("HOUR"), "got `{text}`");
        assert!(
            header
                .spans
                .iter()
                .all(|s| s.style.add_modifier.contains(Modifier::BOLD)),
            "weight is what separates the header from the data now"
        );
    }

    #[test]
    fn every_column_in_a_header_gets_the_same_treatment() {
        // Mixed treatment within a row reads as a bug. This used to be a real
        // risk, because a column too narrow for its letterspaced label demoted
        // the whole row; the bold face costs no extra width, so a narrow column
        // can no longer force the question.
        let cols = [Column::fixed("done", 4), Column::fixed("sky", 20)];
        let grid = Grid::new(&cols, 25);
        let text: String = grid
            .header(&Theme::default())
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.starts_with("DONE"), "got `{text}`");
        assert!(text.contains("SKY"), "got `{text}`");
        assert!(!text.contains("S K Y"), "no letterspacing: `{text}`");
        assert!(!text.contains("D O N E"), "no letterspacing: `{text}`");
    }

    #[test]
    fn a_header_too_long_for_its_column_truncates_rather_than_shifting() {
        // "TEMPERATURE" is 11 columns; the column is 6.
        let grid = Grid::new(
            &[Column::fixed("temperature", 6), Column::fixed("b", 4)],
            11,
        );
        let header = grid.header(&Theme::default());
        assert_eq!(width_of(&header), 11);
    }
}
