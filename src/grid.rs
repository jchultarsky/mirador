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
pub fn char_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Break `text` into rows of at most `width` cells, handing each to `emit`.
///
/// **The single source of the wrapping rules.** [`wrap`] collects what this
/// yields and [`wrapped_height`] counts it without building anything, so the
/// two cannot drift. They used to be two transcriptions of the same rules held
/// equal by a test, and they *had* drifted: `wrapped_height` measured an
/// over-long word by subtracting `width` a row at a time, which assumes the
/// overflow can be split at any cell. A double-width glyph cannot be split, so
/// the two disagreed in both directions the moment a headline was not ASCII —
/// `日本語` at width 3 wraps to three rows and measured two. A panel that sizes
/// itself with one and draws with the other then puts content outside the box
/// it was measured for. The test that was supposed to catch this only ever fed
/// it ASCII.
///
/// Rows are emitted as slices of `text`, so nothing here allocates. Measured in
/// cells rather than characters, per the rule the whole module exists for.
fn break_lines<'a>(text: &'a str, width: usize, mut emit: impl FnMut(&'a str)) {
    if width == 0 {
        return;
    }

    let mut any = false;
    for line in text.lines() {
        // Byte offsets into `line`: where the row being built starts, and how
        // far the words consumed so far reach.
        let mut start = 0usize;
        let mut cursor = 0usize;
        let mut used = 0usize;
        let mut emitted_here = false;

        for word in line.split_inclusive(' ') {
            let w = display_width(word);
            if used > 0 && used + w > width {
                emit(&line[start..cursor]);
                (any, emitted_here) = (true, true);
                start = cursor;
                used = 0;
            }
            cursor += word.len();
            used += w;

            // A single word longer than the row breaks inside itself, rather
            // than running off the edge.
            while used > width {
                let mut cut = start;
                let mut taken = 0usize;
                for c in line[start..cursor].chars() {
                    let cw = char_width(c);
                    if taken + cw > width {
                        break;
                    }
                    taken += cw;
                    cut += c.len_utf8();
                }
                // Always take at least one character. A glyph wider than the
                // whole row — any CJK character or emoji in a one-cell column —
                // otherwise fits nowhere, so nothing is taken, nothing is
                // consumed, and this loops for ever. That is a hang, not a slow
                // path: the dashboard freezes and has to be killed.
                //
                // The row then exceeds `width` by a cell, which is the honest
                // outcome. A terminal cannot draw half a wide glyph either.
                if cut == start {
                    match line[start..cursor].chars().next() {
                        Some(c) => cut += c.len_utf8(),
                        None => break,
                    }
                }
                emit(&line[start..cut]);
                (any, emitted_here) = (true, true);
                start = cut;
                used = display_width(&line[start..cursor]);
            }
        }

        // The remainder — but only if there is one. Emitting it unconditionally
        // appended a blank row whenever the break above consumed the line
        // exactly, which is what an over-long run of wide glyphs always does:
        // `wrap("🌞🌞", 1)` came out as three rows, the last one empty. A
        // genuinely empty source line still gets its row, hence `emitted_here`.
        if start < cursor || !emitted_here {
            emit(&line[start..cursor]);
            any = true;
        }
    }

    // An empty string has no lines at all, and still occupies one row.
    if !any {
        emit("");
    }
}

/// Break `text` into lines no wider than `width` display cells.
///
/// The companion to [`wrapped_height`], which counts the same rows without
/// producing them. Both are [`break_lines`] with a different consumer, so
/// `wrap(t, w).len()` and `wrapped_height(t, w)` agree by construction rather
/// than by a test that has to remember to try a wide glyph.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    break_lines(text, width, |line| out.push(line.to_string()));
    out
}

/// `text` wrapped to `width` and rejoined, ready for a `Paragraph` that must
/// **not** be given `Wrap`.
///
/// This exists because ratatui's own word wrapper panics on text mirador did
/// not write. `Paragraph::new("a🌞b").wrap(…)` rendered two cells wide indexes
/// past the end of the buffer and brings the whole dashboard down, and a
/// leading combining mark — a grapheme with no base character — does the same
/// at any width. Both are reachable from a note, a task's notes, or a fetch
/// error: emoji in prose is ordinary, and the panic is a crash rather than a
/// misdraw.
///
/// So the rule is that ratatui never wraps anything a user or a server wrote.
/// Wrapping here first means its wrapper never runs, and the wrapping is the
/// one in this module, which is measured in cells and tested against a corpus
/// of wide glyphs and combining marks.
pub fn wrapped(text: &str, width: u16) -> String {
    let width = usize::from(width);
    let mut out = String::with_capacity(text.len());
    break_lines(text, width, |line| {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    });
    out
}

/// Rows that `text` occupies once word-wrapped to `width` cells.
///
/// Needed wherever something is sized to fit its own contents — a scroll
/// clamp, or a dialog that must not be shorter than the text inside it. The
/// unwrapped line count is not a substitute: it is right until a line is
/// longer than the box, and then it is short by exactly the amount that
/// matters.
///
/// Counts the rows [`wrap`] would produce, without producing them — the whole
/// point, since a scroll clamp runs against a note's entire body and must not
/// allocate in proportion to it. An empty line still occupies a row.
///
/// This used to be documented as matching `Paragraph::wrap`, whose own
/// `line_count` is private. It no longer needs to: nothing is handed to
/// ratatui's wrapper any more (see [`wrapped`]), so the thing worth agreeing
/// with is mirador's own wrapping, which is what gets drawn.
pub fn wrapped_height(text: &str, width: u16) -> u16 {
    if width == 0 {
        return 0;
    }
    let mut rows: usize = 0;
    break_lines(text, usize::from(width), |_| rows += 1);
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
pub(crate) const GUTTER: u16 = 1;

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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::widgets::{Paragraph, Wrap};

    /// The two must agree: a panel sizes itself with one and draws with the
    /// other, so a disagreement puts content outside the box measured for it.
    ///
    /// They are one function with two consumers now, so this cannot fail by
    /// drift any more — but it could once, and for a long time it did not
    /// notice. Every sample was ASCII, and the bug was that `wrapped_height`
    /// measured an over-long word by subtracting `width` a row at a time, which
    /// is only right when every glyph is one cell. The wide glyphs and the
    /// widths below 8 are the part of this test that ever had a chance.
    #[test]
    fn wrapping_produces_exactly_as_many_rows_as_it_measures() {
        let samples = [
            "Wildfire now nine miles away from the French city of Bordeaux, mayor warns",
            "short",
            "",
            "a supercalifragilisticexpialidociousandthensomeword in a line",
            "Europa Clipper returns its first images — and they are extraordinary",
            "line one\nline two is a good deal longer than the first one is",
            // A headline is somebody else's text, and it is not always Latin.
            "\u{4E2D}\u{6587}\u{6807}\u{9898}",
            "\u{65E5}\u{672C}\u{8A9E}\u{306E}\u{30CB}\u{30E5}\u{30FC}\u{30B9}\u{3067}\u{3059}",
            "a\u{1F31E}b\u{1F31E}c",
            "\u{1F31E}\u{1F31E}\u{1F31E}\u{1F31E}",
            "mixed \u{4E2D}\u{6587} and english \u{1F31E} together",
            // A combining mark with no base character, which is a grapheme all
            // the same and measures zero cells.
            "\u{0301}\u{0301}leading marks",
            "blank\n\nline in the middle",
        ];
        for text in samples {
            for width in 1..40u16 {
                assert_eq!(
                    u16::try_from(wrap(text, usize::from(width)).len()).unwrap(),
                    wrapped_height(text, width),
                    "`{text}` at {width}"
                );
            }
        }
    }

    /// An over-long run of wide glyphs consumes the row exactly, and the
    /// remainder is then empty. Emitting it anyway appended a blank row that
    /// nothing asked for — `wrap("🌞🌞", 1)` came out as three rows, the last
    /// one empty, which in the news panel is a wasted line at the bottom of
    /// every headline that ends on a boundary.
    #[test]
    fn a_row_consumed_exactly_leaves_no_blank_behind() {
        assert_eq!(wrap("\u{1F31E}\u{1F31E}", 1), ["\u{1F31E}", "\u{1F31E}"]);
        assert_eq!(wrap("abcdef", 3), ["abc", "def"]);
        assert_eq!(wrap("\u{65E5}\u{672C}", 2), ["\u{65E5}", "\u{672C}"]);

        // But a blank line the author actually wrote still gets its row.
        assert_eq!(wrap("a\n\nb", 4), ["a", "", "b"]);
        assert_eq!(wrap("", 4), [""]);
        assert_eq!(wrap("\n", 4), [""]);
    }

    /// A glyph wider than the whole line fits nowhere, so "take as many
    /// characters as fit" takes none, consumes nothing, and loops for ever.
    /// That is a hang — the dashboard freezes and has to be killed — and it is
    /// reachable by any CJK or emoji headline in a narrow panel.
    #[test]
    fn a_glyph_wider_than_the_line_does_not_hang() {
        for text in [
            "\u{1F31E}\u{1F31E}",
            "\u{4E2D}\u{6587}\u{6807}\u{9898}",
            "a\u{1F31E}b",
        ] {
            for width in 1..4usize {
                let out = wrap(text, width);
                assert!(
                    out.len() <= text.chars().count() + 1,
                    "`{text}` at {width} produced {} lines",
                    out.len()
                );
                // Every character survives; the line may exceed the width by a
                // cell, because a terminal cannot draw half a wide glyph.
                let joined: String = out.concat();
                assert_eq!(joined, text, "at {width}");
            }
        }
    }

    /// The corpus that crashes ratatui: narrow and wide glyphs at every offset,
    /// spaces, newlines, and a combining mark with no base character.
    ///
    /// Deterministic and hand-rolled for the same reason as `ical`'s — what
    /// matters is that every fragment is in it because of something the
    /// wrapper does, not that the bytes are random.
    fn crashing_corpus() -> Vec<String> {
        let alphabet = ["a", "\u{1F31E}", "\u{65E5}", " ", "\n", "\u{0301}"];
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        (0..600)
            .map(|_| {
                let len = (next() % 9) as usize + 1;
                (0..len)
                    .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                    .collect()
            })
            .collect()
    }

    fn renders_without_panicking(paragraph: impl Fn() -> Paragraph<'static>, width: u16) -> bool {
        let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            terminal
                .draw(|frame| frame.render_widget(paragraph(), frame.area()))
                .unwrap();
        }))
        .is_ok()
    }

    /// The reason [`wrapped`] exists, pinned rather than described.
    ///
    /// ratatui's own word wrapper indexes past the end of the buffer and
    /// panics, which for a dashboard is a crash rather than a misdraw. Two ways
    /// in: a double-width glyph inside a word too long for a two-cell area, and
    /// a leading combining mark, which throws the accounting off at any width.
    /// Neither needs unusual input — emoji in a note is ordinary.
    ///
    /// Reported upstream as <https://github.com/ratatui/ratatui/issues/2679>.
    ///
    /// **If this test starts failing, ratatui has fixed it.** That is good
    /// news, and the thing to do is check whether `wrapped` can go, not to
    /// delete the assertion. The bound lives in a dependency and would leave
    /// with it.
    /// Nothing outside this module may hand text to ratatui's word wrapper.
    ///
    /// [`wrapped`] exists because that wrapper panics on text mirador did not
    /// write, and the fix was to route every such site through here. The panels
    /// that were fixed have their own tests — `a_note_full_of_wide_glyphs_draws_at_every_width`
    /// is the model — but those guard the sites that were *known* about. A new
    /// panel, or a new render site in an old one, would reintroduce the crash
    /// with nothing to catch it.
    ///
    /// This is that catch, and it is structural rather than behavioural on
    /// purpose. A geometry sweep was tried first and abandoned: to trigger the
    /// fault a specific string has to meet a specific pane width, panels
    /// sub-divide their own area, and only one note's body is ever drawn — so
    /// the sweep passed cheerfully with the defect reintroduced. Counting the
    /// call sites cannot miss.
    ///
    /// Two are allowed, and both are text this program wrote itself: the help
    /// overlay and the delete confirmation, whose one variable line is
    /// truncated to the width before it gets there.
    #[test]
    fn only_the_two_known_places_use_ratatuis_own_wrapper() {
        // Split so this test's own source is not a match.
        let needle = concat!(".wr", "ap(");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

        fn walk(dir: &std::path::Path, needle: &str, found: &mut Vec<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, needle, found);
                } else if path.extension().is_some_and(|e| e == "rs")
                    // This module's own tests use the wrapper deliberately, to
                    // prove it still panics. Testing the thing is not using it.
                    && path.file_name().is_some_and(|n| n != "grid.rs")
                    && let Ok(text) = std::fs::read_to_string(&path)
                {
                    for line in text.lines() {
                        let trimmed = line.trim_start();
                        // Prose about the rule is not a use of it.
                        if trimmed.starts_with("//") {
                            continue;
                        }
                        if line.contains(needle) {
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            found.push(format!("{name}: {}", trimmed.trim_end()));
                        }
                    }
                }
            }
        }

        let mut found = Vec::new();
        walk(&root, needle, &mut found);
        found.sort();

        assert_eq!(
            found.len(),
            2,
            "expected exactly two uses of ratatui's wrapper — the help overlay \
             and the delete confirmation — and found {}:\n  {}\n\n\
             If this is a new site rendering text mirador did not write, wrap it \
             with `grid::wrapped` instead: ratatui's wrapper panics on a leading \
             combining mark, and on a double-width glyph in a word too long for \
             a two-cell area, which is ordinary prose. If it really is text this \
             program wrote itself, widen this count and say why.",
            found.len(),
            found.join("\n  ")
        );
        assert!(
            found.iter().any(|f| f.starts_with("app.rs:")),
            "the help overlay's wrap went missing: {found:?}"
        );
        assert!(
            found.iter().any(|f| f.starts_with("todo.rs:")),
            "the delete confirmation's wrap went missing: {found:?}"
        );
    }

    #[test]
    fn ratatuis_own_wrapper_is_why_this_module_wraps_first() {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let straddles_a_two_cell_row = renders_without_panicking(
            || Paragraph::new("a\u{1F31E}b").wrap(Wrap { trim: false }),
            2,
        );
        let leading_combining_mark = renders_without_panicking(
            || {
                Paragraph::new("\u{0301} \u{1F31E}\u{65E5}\u{65E5}\u{65E5}\u{1F31E}\u{1F31E}a")
                    .wrap(Wrap { trim: false })
            },
            8,
        );
        std::panic::set_hook(hook);

        assert!(
            !straddles_a_two_cell_row && !leading_combining_mark,
            "ratatui no longer panics here (two-cell row: {}, combining mark: {}). \
             Check whether `grid::wrapped` is still needed before removing this.",
            !straddles_a_two_cell_row,
            !leading_combining_mark
        );
    }

    /// And the fix: the same corpus, wrapped here and handed to a `Paragraph`
    /// with no `Wrap`, draws at every width without bringing the dashboard
    /// down.
    #[test]
    fn pre_wrapping_survives_what_crashes_ratatui() {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let mut crashed = Vec::new();
        for text in crashing_corpus() {
            for width in 1..12u16 {
                let wrapped_text = wrapped(&text, width);
                if !renders_without_panicking(|| Paragraph::new(wrapped_text.clone()), width) {
                    crashed.push(format!("{text:?} at {width}"));
                }
            }
        }
        std::panic::set_hook(hook);
        assert!(crashed.is_empty(), "still panics: {crashed:?}");
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

    /// The one exception, and it is deliberate: a single glyph that cannot fit
    /// is emitted anyway rather than dropped or looped on. It overruns by a
    /// cell, which a terminal handles and an infinite loop does not.
    #[test]
    fn only_an_unfittable_glyph_may_overrun_and_only_by_itself() {
        let out = wrap("\u{1F31E}\u{1F31E}", 1);
        for line in &out {
            assert!(
                line.chars().count() <= 1,
                "an overrunning line holds one glyph and no more: `{line}`"
            );
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
