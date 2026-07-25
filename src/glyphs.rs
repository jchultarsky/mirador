//! The display and utility "typefaces", plus weather art.
//!
//! A terminal has exactly one font, so a dashboard that wants typographic
//! hierarchy has to manufacture it. Two faces live here:
//!
//! - [`BigText`] draws scalable block numerals for the chronometer.
//! - [`utility`] uppercases text for labels, which callers set bold. Weight
//!   and case separate a label from its value without relying on colour alone
//!   — which matters, because a user's terminal theme can render our colours
//!   as almost anything.
//!
//! The numerals use run-length rows rather than a table of pre-drawn strings,
//! an idea taken from `clock-tui`'s "bricks" font. Each glyph is five rows, and
//! each row is a list of alternating off/on runs over a six-unit grid. Thirteen
//! glyphs fit in a few lines, and integer scaling comes for free — which is the
//! whole reason to do it this way rather than hand-drawing 3x5 strings.

/// Glyph grid width in units, before scaling.
const CELL_W: u16 = 6;
/// Glyph grid height in units, before scaling.
const CELL_H: u16 = 5;
/// Columns between glyphs. Deliberately not scaled: at large sizes a scaled
/// gap makes the digits read as separate objects rather than as one number.
const GAP: u16 = 2;

/// Run-length rows for one glyph: five rows, each alternating off/on runs
/// starting with off. `[0, 6]` is a full bar; `[2, 2]` is a centred stub.
type Glyph = [&'static [u16]; 5];

fn glyph(c: char) -> Glyph {
    match c {
        '0' => [
            &[0, 6],
            &[0, 2, 2, 2],
            &[0, 2, 2, 2],
            &[0, 2, 2, 2],
            &[0, 6],
        ],
        '1' => [&[0, 4], &[2, 2], &[2, 2], &[2, 2], &[0, 6]],
        '2' => [&[0, 6], &[4, 2], &[0, 6], &[0, 2], &[0, 6]],
        '3' => [&[0, 6], &[4, 2], &[0, 6], &[4, 2], &[0, 6]],
        '4' => [&[0, 2, 2, 2], &[0, 2, 2, 2], &[0, 6], &[4, 2], &[4, 2]],
        '5' => [&[0, 6], &[0, 2], &[0, 6], &[4, 2], &[0, 6]],
        '6' => [&[0, 6], &[0, 2], &[0, 6], &[0, 2, 2, 2], &[0, 6]],
        '7' => [&[0, 6], &[4, 2], &[4, 2], &[4, 2], &[4, 2]],
        '8' => [&[0, 6], &[0, 2, 2, 2], &[0, 6], &[0, 2, 2, 2], &[0, 6]],
        '9' => [&[0, 6], &[0, 2, 2, 2], &[0, 6], &[4, 2], &[0, 6]],
        ':' => [&[], &[2, 2], &[], &[2, 2], &[]],
        '.' => [&[], &[], &[], &[], &[2, 2]],
        '-' => [&[], &[], &[0, 6], &[], &[]],
        // Unknown characters render blank but still occupy a cell, so a
        // malformed time string cannot break alignment.
        _ => [&[], &[], &[], &[], &[]],
    }
}

/// A block-numeral string rendered at an integer scale.
#[derive(Debug, Clone)]
pub struct BigText {
    /// One string per output row, all the same display width.
    pub rows: Vec<String>,
    pub width: u16,
    pub height: u16,
}

impl BigText {
    /// Render `text` at `scale`. A scale of 1 gives glyphs 6x5 cells.
    pub fn new(text: &str, scale: u16) -> Self {
        let scale = scale.max(1);
        let height = CELL_H * scale;
        let width = width_of(text, scale);
        let mut rows = vec![String::with_capacity(width as usize); height as usize];

        let mut pen = 0u16;
        for (index, c) in text.chars().enumerate() {
            if index > 0 {
                pen += GAP;
            }
            let g = glyph(c);
            for (unit_row, runs) in g.iter().enumerate() {
                // Each logical row is repeated `scale` times vertically.
                for repeat in 0..scale {
                    let row = usize::from(scale) * unit_row + usize::from(repeat);
                    let line = &mut rows[row];
                    pad_to(line, pen);

                    let mut cursor = pen;
                    let mut on = false;
                    for run in *runs {
                        let cells = run * scale;
                        if on {
                            pad_to(line, cursor);
                            for _ in 0..cells {
                                line.push('█');
                            }
                        }
                        cursor += cells;
                        on = !on;
                    }
                }
            }
            pen += CELL_W * scale;
        }

        for row in &mut rows {
            pad_to(row, width);
        }

        Self {
            rows,
            width,
            height,
        }
    }
}

/// Extend `line` with spaces until it is `target` display columns wide.
fn pad_to(line: &mut String, target: u16) {
    let current = line.chars().count();
    for _ in current..usize::from(target) {
        line.push(' ');
    }
}

/// Columns `text` will occupy at `scale`.
pub fn width_of(text: &str, scale: u16) -> u16 {
    let scale = scale.max(1);
    let count = u16::try_from(text.chars().count()).unwrap_or(u16::MAX);
    if count == 0 {
        return 0;
    }
    count * CELL_W * scale + (count - 1) * GAP
}

/// The largest scale at which `text` fits in `available` columns, or `None` if
/// even scale 1 will not fit.
pub fn fitting_scale(text: &str, available: u16, max_scale: u16) -> Option<u16> {
    (1..=max_scale.max(1))
        .rev()
        .find(|scale| width_of(text, *scale) <= available)
}

/// Render text in the utility face: uppercase, normally spaced.
///
/// `utility("next hours")` becomes `NEXT HOURS`.
///
/// This face used to be letterspaced — `N E X T  H O U R S` — on the theory
/// that tracking separates a label from data without leaning on colour. In
/// practice it read as stretched rather than as engraved, and it cost width
/// that narrow panels could not spare. Weight and colour carry the distinction
/// instead: callers pair this with `Modifier::BOLD` and `theme.label`.
pub fn utility(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.extend(word.chars().flat_map(char::to_uppercase));
    }
    out
}

/// Broad weather categories, one per piece of art.
///
/// Deliberately coarser than the WMO code list: art that distinguished
/// "moderate" from "dense" drizzle would be illegible at this size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sky {
    Clear,
    PartlyCloudy,
    Overcast,
    Fog,
    Drizzle,
    Rain,
    Showers,
    Snow,
    Thunder,
    Unknown,
}

/// Classify a WMO 4677 code, which is what Open-Meteo reports.
pub fn sky(code: u8) -> Sky {
    match code {
        0 | 1 => Sky::Clear,
        2 => Sky::PartlyCloudy,
        3 => Sky::Overcast,
        45 | 48 => Sky::Fog,
        51..=57 => Sky::Drizzle,
        61..=67 => Sky::Rain,
        80..=82 => Sky::Showers,
        71..=77 | 85 | 86 => Sky::Snow,
        95..=99 => Sky::Thunder,
        _ => Sky::Unknown,
    }
}

/// Rows in the weather art.
pub const ART_HEIGHT: usize = 4;
/// Columns in the weather art. Every row is padded to this.
pub const ART_WIDTH: usize = 12;

/// Four-row art for a sky condition.
///
/// Restricted to ASCII plus a handful of box-drawing characters, so it renders
/// in a plain terminal font with no Nerd Font or emoji fallback.
pub fn art(sky: Sky) -> [&'static str; ART_HEIGHT] {
    match sky {
        Sky::Clear => [
            r"   \   /    ",
            r"    .-.     ",
            r" --(   )--  ",
            r"   /   \    ",
        ],
        Sky::PartlyCloudy => [
            r"  \ /  .-.  ",
            r" - o -(   ) ",
            r"  / \ (___) ",
            r"            ",
        ],
        Sky::Overcast => [
            r"    .--.    ",
            r"  .(    ).  ",
            r" (___.__)_) ",
            r"            ",
        ],
        Sky::Fog => [
            r"  _ - _ - _ ",
            r"   _ - _ -  ",
            r"  _ - _ - _ ",
            r"            ",
        ],
        Sky::Drizzle => [
            r"    .-.     ",
            r"   (   ).   ",
            r"  (___(__)  ",
            r"   ' ' ' '  ",
        ],
        Sky::Rain => [
            r"    .-.     ",
            r"   (   ).   ",
            r"  (___(__)  ",
            r"  ' ' ' ' ' ",
        ],
        Sky::Showers => [
            r"  _ /''.-.  ",
            r"   \_(   ). ",
            r"   (___(__) ",
            r"    ' ' ' ' ",
        ],
        Sky::Snow => [
            r"    .-.     ",
            r"   (   ).   ",
            r"  (___(__)  ",
            r"   *  *  *  ",
        ],
        Sky::Thunder => [
            r"    .-.     ",
            r"   (   ).   ",
            r"  (___(__)  ",
            r"   /_  /_   ",
        ],
        Sky::Unknown => [
            r"            ",
            r"    .---.   ",
            r"   ( ? ? )  ",
            r"    `---'   ",
        ],
    }
}

/// An emoji mark for a sky condition, used in the hourly table.
///
/// Every glyph here is deliberately chosen so that `unicode-width` reports the
/// same cell count the terminal actually draws. That sounds pedantic and is
/// not: several of the obvious weather emoji — U+1F327 rain cloud, U+1F328
/// snow cloud, U+2601 cloud — report width 1 but render as two cells, which
/// silently shifts every column after them and puts values under the wrong
/// headers. The set below is restricted to glyphs that measure 2 and draw 2.
///
/// `every_sky_mark_has_a_predictable_display_width` asserts this. Plain code
/// rather than an intra-doc link: the test is behind `cfg(test)`, so rustdoc
/// cannot resolve a link to it and `-D warnings` turns that into a build
/// failure.
pub fn mark(sky: Sky) -> &'static str {
    match sky {
        Sky::Clear => "🌞",
        Sky::PartlyCloudy => "⛅",
        // Every cloud emoji in the obvious range measures 1 and draws 2, so
        // overcast and fog borrow neighbours that measure honestly: a dark
        // disc for a flat grey sky, and the "foggy" pictograph for fog.
        Sky::Overcast => "🌑",
        Sky::Fog => "🌁",
        Sky::Drizzle | Sky::Rain | Sky::Showers => "☔",
        Sky::Snow => "⛄",
        Sky::Thunder => "⚡",
        Sky::Unknown => "❓",
    }
}

/// A short human label for a sky condition.
pub fn describe(sky: Sky) -> &'static str {
    match sky {
        Sky::Clear => "clear",
        Sky::PartlyCloudy => "partly cloudy",
        Sky::Overcast => "overcast",
        Sky::Fog => "fog",
        Sky::Drizzle => "drizzle",
        Sky::Rain => "rain",
        Sky::Showers => "showers",
        Sky::Snow => "snow",
        Sky::Thunder => "thunderstorm",
        Sky::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_rows_are_rectangular_at_every_scale() {
        for scale in 1..=4 {
            let big = BigText::new("12:34", scale);
            assert_eq!(big.rows.len(), usize::from(big.height));
            for row in &big.rows {
                assert_eq!(
                    row.chars().count(),
                    usize::from(big.width),
                    "ragged row at scale {scale}"
                );
            }
        }
    }

    #[test]
    fn height_and_width_scale_linearly() {
        let one = BigText::new("00", 1);
        let two = BigText::new("00", 2);
        assert_eq!(two.height, one.height * 2);
        // Width doubles except for the unscaled inter-glyph gap.
        assert_eq!(two.width, one.width * 2 - GAP);
    }

    #[test]
    fn reported_width_matches_what_is_rendered() {
        for text in ["0", "12:34", "23:59:59", ""] {
            for scale in 1..=3 {
                let big = BigText::new(text, scale);
                assert_eq!(width_of(text, scale), big.width, "`{text}` at {scale}");
                if !text.is_empty() {
                    assert_eq!(big.rows[0].chars().count(), usize::from(big.width));
                }
            }
        }
    }

    #[test]
    fn a_scale_of_zero_is_treated_as_one() {
        assert_eq!(BigText::new("1", 0).height, BigText::new("1", 1).height);
        assert_eq!(width_of("1", 0), width_of("1", 1));
    }

    #[test]
    fn empty_input_produces_no_width() {
        let big = BigText::new("", 2);
        assert_eq!(big.width, 0);
        assert_eq!(width_of("", 2), 0);
    }

    #[test]
    fn digits_actually_draw_something() {
        for c in "0123456789".chars() {
            let big = BigText::new(&c.to_string(), 1);
            let filled: usize = big.rows.iter().map(|r| r.matches('█').count()).sum();
            assert!(filled > 0, "digit {c} rendered blank");
        }
        // The colon is sparse but non-empty; unknown glyphs are blank by design.
        assert!(BigText::new(":", 1).rows.join("").contains('█'));
        assert!(!BigText::new("x", 1).rows.join("").contains('█'));
    }

    #[test]
    fn fitting_scale_picks_the_largest_that_fits() {
        let text = "12:34";
        let w2 = width_of(text, 2);
        assert_eq!(fitting_scale(text, w2, 4), Some(2));
        assert_eq!(fitting_scale(text, w2 - 1, 4), Some(1));
        assert_eq!(fitting_scale(text, 0, 4), None);
        // Never exceeds the requested ceiling.
        assert_eq!(fitting_scale(text, 10_000, 3), Some(3));
    }

    #[test]
    fn fitting_scale_result_always_fits() {
        for available in 0..120u16 {
            if let Some(scale) = fitting_scale("12:34", available, 5) {
                assert!(width_of("12:34", scale) <= available);
            }
        }
    }

    #[test]
    fn the_utility_face_uppercases_without_letterspacing() {
        assert_eq!(utility("local"), "LOCAL");
        assert_eq!(utility("next hours"), "NEXT HOURS");
        assert_eq!(utility("a"), "A");
        assert_eq!(utility(""), "");
    }

    #[test]
    fn the_utility_face_collapses_runs_of_whitespace_and_trims() {
        assert_eq!(utility("  due   date  "), "DUE DATE");
    }

    /// The face must not cost width any more. It was letterspaced once, which
    /// more than doubled every label and pushed them out of narrow panels.
    #[test]
    fn the_utility_face_is_never_wider_than_the_text_it_renders() {
        for text in ["load", "per core", "next hours", "session", "due date"] {
            assert_eq!(
                utility(text).chars().count(),
                text.split_whitespace().collect::<Vec<_>>().join(" ").len(),
                "`{text}` changed width"
            );
        }
    }

    #[test]
    fn every_sky_has_rectangular_art_of_the_declared_size() {
        for s in [
            Sky::Clear,
            Sky::PartlyCloudy,
            Sky::Overcast,
            Sky::Fog,
            Sky::Drizzle,
            Sky::Rain,
            Sky::Showers,
            Sky::Snow,
            Sky::Thunder,
            Sky::Unknown,
        ] {
            let rows = art(s);
            assert_eq!(rows.len(), ART_HEIGHT, "{s:?} has the wrong height");
            for row in rows {
                assert_eq!(
                    row.chars().count(),
                    ART_WIDTH,
                    "{s:?} row `{row}` is not {ART_WIDTH} wide"
                );
            }
            assert!(!describe(s).is_empty(), "{s:?} needs a label");
        }
    }

    /// Every sky mark must measure the width it actually draws.
    ///
    /// This is the guard against the whole class of bug where a wide emoji
    /// measured as one cell shunts every later column sideways. If a future
    /// glyph swap breaks this, the table silently misaligns — so it is asserted
    /// rather than left to inspection.
    #[test]
    fn every_sky_mark_has_a_predictable_display_width() {
        use unicode_width::UnicodeWidthStr;
        for s in [
            Sky::Clear,
            Sky::PartlyCloudy,
            Sky::Overcast,
            Sky::Fog,
            Sky::Drizzle,
            Sky::Rain,
            Sky::Showers,
            Sky::Snow,
            Sky::Thunder,
            Sky::Unknown,
        ] {
            let mark = mark(s);
            let width = UnicodeWidthStr::width(mark);
            assert_eq!(
                width, 2,
                "{s:?} mark `{mark}` measures {width}; emoji that measure 1 but \
                 draw 2 break every column after them"
            );
        }
    }

    #[test]
    fn every_code_open_meteo_emits_maps_to_real_art() {
        let codes = [
            0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81, 82,
            85, 86, 95, 96, 99,
        ];
        for code in codes {
            assert_ne!(sky(code), Sky::Unknown, "code {code} is unclassified");
            assert!(!mark(sky(code)).is_empty(), "code {code} has no mark");
        }
    }

    #[test]
    fn unrecognised_codes_degrade_rather_than_panic() {
        assert_eq!(sky(200), Sky::Unknown);
        assert_eq!(describe(sky(200)), "unknown");
        assert_eq!(mark(sky(200)), "❓");
    }
}
