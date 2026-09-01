//! OSC 8 hyperlinks, punched into cells the widgets already drew.
//!
//! `linkify` runs after a panel has rendered, so the text underneath was
//! measured, wrapped and clipped by the same code as everything else —
//! invariant 19 is settled before a link exists. Each cell in the range is
//! made *self-contained*: its symbol becomes open sequence + glyph + close
//! sequence, declared at the glyph's real width via
//! [`CellDiffOption::ForcedWidth`] so the escape bytes cost no columns. The
//! shared `id=` parameter is what makes the run one logical link to the
//! terminal, and it is also the whole answer for a link that wraps: call
//! `linkify` once per row with the same id and URL.
//!
//! Self-containment is not a style choice. OSC 8 is *modal* — an open
//! sequence stays in force until a close arrives — and ratatui's diff
//! re-emits individual cells. A link spanning cells (open on the first,
//! close on the last) breaks the moment the diff rewrites a subset: a
//! middle-only change rewrites cells with no link active, and a re-emitted
//! opener without its closer leaks link state onto everything printed after
//! it. `examples/osc8_probe.rs` demonstrates both on a captured byte
//! stream. With every cell self-contained, any subset the diff picks is
//! consistent by construction.
//!
//! The URL rides *inside* an escape sequence, and the URLs mirador links to
//! come from RSS feeds — the one input somebody else writes. An entity like
//! `&#27;` puts a real escape byte in a parsed link, and a string terminator
//! smuggled there would end the sequence early and feed the rest to the
//! terminal as input. So `linkable` is a strict allowlist, the same shape as
//! the quote module's symbol check: `http`/`https` only, RFC 3986 characters
//! only, and anything else simply gets no link — the headline still renders,
//! `o` and `y` still work.

use std::num::NonZeroU16;

use ratatui::buffer::{Buffer, CellDiffOption};
use unicode_width::UnicodeWidthStr;

/// Whether a URL is safe to embed in an OSC 8 sequence.
///
/// `http://` or `https://` and RFC 3986 characters only — unreserved,
/// reserved, and `%`. Everything else, most importantly every control byte,
/// refuses the whole link rather than escaping it: there is no encoding a
/// terminal is guaranteed to undo, and a headline without a link loses
/// nothing but the shortcut.
pub fn linkable(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && url.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'-' | b'.'
                        | b'_'
                        | b'~'
                        | b':'
                        | b'/'
                        | b'?'
                        | b'#'
                        | b'['
                        | b']'
                        | b'@'
                        | b'!'
                        | b'$'
                        | b'&'
                        | b'\''
                        | b'('
                        | b')'
                        | b'*'
                        | b'+'
                        | b','
                        | b';'
                        | b'='
                        | b'%'
                )
        })
}

/// Turns the cells of row `y` from `x0` to `x1` (exclusive) into an OSC 8
/// hyperlink, leaving the glyphs and styles the widgets drew untouched.
///
/// A URL that fails [`linkable`] links nothing, silently — see the module
/// docs for why refusal beats escaping. The `id` is truncated to the
/// characters that are safe in a parameter (letters, digits, dash), which
/// every caller-provided id already consists of; the guard exists so no
/// future caller has to think about it.
///
/// The walk advances by each glyph's width, so the continuation cell behind
/// a double-width glyph is passed over rather than wrapped: the buffer
/// resets it to an empty cell indistinguishable from a real space, and the
/// only way to know it is covered is to have measured its neighbour. `x0`
/// must sit on a glyph boundary, which ranges taken from a `Line`'s own
/// width are by construction. A range reaching past the buffer stops at the
/// edge.
pub fn linkify(buf: &mut Buffer, y: u16, x0: u16, x1: u16, url: &str, id: &str) {
    if !linkable(url) {
        return;
    }
    let id: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let open = format!("\x1b]8;id={id};{url}\x1b\\");
    let mut x = x0;
    while x < x1 {
        let Some(cell) = buf.cell_mut((x, y)) else {
            return;
        };
        let glyph = cell.symbol().to_string();
        // A symbol measuring 0 would stall the walk; treating it as 1 keeps
        // the loop moving, the same guard `samples::push_bounded` carries.
        let width = glyph.width().max(1) as u16;
        cell.set_symbol(&format!("{open}{glyph}\x1b]8;;\x1b\\"));
        cell.set_diff_option(CellDiffOption::ForcedWidth(
            NonZeroU16::new(width).expect("width floored at 1"),
        ));
        x += width;
    }
}

/// The text of `symbol` with any OSC 8 sequences removed.
///
/// For tests that read a rendered buffer back as a string: a linked cell's
/// symbol carries the URL in escape bytes, so a substring assertion against
/// the raw concatenation would see text nobody can see on screen.
#[cfg(test)]
pub fn without_links(symbol: &str) -> String {
    let mut out = String::with_capacity(symbol.len());
    let mut rest = symbol;
    while let Some(start) = rest.find("\x1b]8;") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 4..];
        match after.find("\x1b\\") {
            Some(end) => rest = &after[end + 2..],
            None => return out, // torn sequence: drop the tail, as a terminal would
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;
    use ratatui::style::{Modifier, Style};

    use super::*;

    fn buffer_with(text: &str) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 2));
        buf.set_string(0, 0, text, Style::default().add_modifier(Modifier::BOLD));
        buf
    }

    #[test]
    fn a_linked_cell_is_self_contained_and_keeps_its_width() {
        let mut buf = buffer_with("news");
        linkify(&mut buf, 0, 0, 4, "https://example.com/a", "story-1");

        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(
            cell.symbol(),
            "\x1b]8;id=story-1;https://example.com/a\x1b\\n\x1b]8;;\x1b\\",
            "open sequence, the glyph, close sequence — nothing shared \
             between cells, so any subset the diff re-emits is consistent"
        );
        assert_eq!(
            cell.diff_option,
            CellDiffOption::ForcedWidth(NonZeroU16::new(1).unwrap()),
            "the declared width is the glyph's, or every column after \
             this cell drifts"
        );
        assert!(
            cell.modifier.contains(Modifier::BOLD),
            "linkify must not disturb the style the widget chose"
        );
    }

    #[test]
    fn a_wide_glyph_keeps_its_width_and_its_continuation_cell() {
        let mut buf = buffer_with("日x");
        linkify(&mut buf, 0, 0, 3, "https://example.com/w", "story-2");

        let wide = buf.cell((0, 0)).unwrap();
        assert!(wide.symbol().contains('日'));
        assert_eq!(
            wide.diff_option,
            CellDiffOption::ForcedWidth(NonZeroU16::new(2).unwrap()),
            "a double-width glyph must declare both its columns"
        );
        let continuation = buf.cell((1, 0)).unwrap();
        assert!(
            !continuation.symbol().contains('\x1b'),
            "the continuation cell is the covered half of its neighbour and \
             must not grow sequences of its own; it holds {:?}",
            continuation.symbol()
        );
        assert_eq!(continuation.diff_option, CellDiffOption::None);
        let after = buf.cell((2, 0)).unwrap();
        assert!(
            after.symbol().contains('x') && after.symbol().contains("]8;"),
            "the glyph after the wide one is back on a boundary and linked"
        );
    }

    #[test]
    fn an_unsafe_url_links_nothing() {
        // Each of these reached a Story::link once upon parsing somebody
        // else's feed: a smuggled escape byte (an `&#27;` entity), a BEL
        // that terminates an OSC early, a scheme that is not the web, and
        // a link the feed simply did not have.
        for url in [
            "https://example.com/\x1b]8;;evil",
            "https://example.com/\x07",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "",
        ] {
            let mut buf = buffer_with("headline");
            linkify(&mut buf, 0, 0, 8, url, "story-3");
            for x in 0..8 {
                let cell = buf.cell((x, 0)).unwrap();
                assert!(
                    !cell.symbol().contains('\x1b'),
                    "url {url:?} must not reach the terminal, in a link or \
                     otherwise; cell {x} holds {:?}",
                    cell.symbol()
                );
                assert_eq!(cell.diff_option, CellDiffOption::None);
            }
        }
    }

    #[test]
    fn a_range_past_the_buffer_edge_is_clipped_not_fatal() {
        let mut buf = buffer_with("ok");
        linkify(&mut buf, 0, 0, 200, "https://example.com/e", "story-4");
        linkify(&mut buf, 9, 0, 4, "https://example.com/e", "story-4");
        assert!(buf.cell((0, 0)).unwrap().symbol().contains("]8;"));
    }

    #[test]
    fn stripping_links_recovers_exactly_the_visible_text() {
        let mut buf = buffer_with("ab 日");
        linkify(&mut buf, 0, 0, 5, "https://example.com/s", "story-5");
        let text: String = (0..5)
            .filter_map(|x| buf.cell((x, 0)).map(|c| without_links(c.symbol())))
            .collect();
        // The trailing space is the covered cell behind 日 — concatenating
        // symbols has always shown it, links or no links.
        assert_eq!(text, "ab 日 ");
        assert_eq!(without_links("plain"), "plain");
    }
}
