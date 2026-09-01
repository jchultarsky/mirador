//! OSC 8 hyperlink probe — the prototype the parked item in CLAUDE.md asked
//! for before any promise gets made.
//!
//! The question it answers: can OSC 8 hyperlinks be pushed through ratatui's
//! cell-diffing renderer cleanly — sequences intact on the wire, no column
//! drift, no unpaired state, and a story for a link that wraps onto a second
//! row?
//!
//! The mechanism under test is ratatui 0.30's `CellDiffOption::ForcedWidth`:
//! a cell's symbol may carry zero-width control sequences around its glyph,
//! with the declared width keeping the buffer's column arithmetic honest.
//! The backend prints symbols verbatim, so the sequences reach the terminal
//! exactly as written. Links are applied by post-processing the frame buffer
//! after the widgets have drawn — the same order a mirador panel would use,
//! and the reason the text underneath stays subject to `grid`'s clipping
//! rules: the link is punched into cells that already obey invariant 19.
//!
//! **Every linked cell is self-contained** — open sequence, glyph, close
//! sequence — with a shared `id=` parameter so the terminal treats the run
//! as one link. The tempting alternative, opening on the first cell and
//! closing on the last, is unsound against the diff: OSC 8 is *modal*, and
//! the diff re-emits whichever cells changed. A middle-only change rewrites
//! cells with no link active (linkage silently lost), and a change that
//! re-emits the opener without the closer leaks link state onto every cell
//! printed after it. Case F below builds the span version on purpose so the
//! captured byte stream shows the failure; nothing real should use it.
//!
//! Run it interactively (`cargo run --example osc8_probe`) in a terminal
//! that renders hyperlinks (iTerm2, `WezTerm`, kitty, recent GNOME Terminal)
//! and the labels are clickable. Run it headlessly for the byte-level
//! evidence:
//!
//! ```sh
//! tmux -L osc8 new-session -d -x 80 -y 24
//! tmux -L osc8 pipe-pane -t 0 -o 'cat >> /tmp/osc8-raw.bin'
//! tmux -L osc8 send-keys -t 0 'cargo run --example osc8_probe -- --ticks 30' Enter
//! # wait, then inspect /tmp/osc8-raw.bin for \x1b]8; sequences
//! ```
//!
//! `--ticks N` renders N frames at ~100ms and exits without a keypress, so a
//! harness can count emissions deterministically. Without it, `q` or Ctrl+C
//! quits — the probe honours the exit guarantee even though no panel here
//! captures input.

use std::num::NonZeroU16;
use std::time::Duration;

use ratatui::Frame;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType};
use unicode_width::UnicodeWidthStr;

const BRASS: Color = Color::Rgb(0xd7, 0xaf, 0x87);
const VERDIGRIS: Color = Color::Rgb(0x5f, 0x87, 0x87);

fn main() {
    let ticks: Option<u32> = {
        let args: Vec<String> = std::env::args().collect();
        args.iter()
            .position(|a| a == "--ticks")
            .and_then(|i| args.get(i + 1))
            .and_then(|n| n.parse().ok())
    };

    let mut terminal = ratatui::init();
    let mut frame_no: u32 = 0;
    loop {
        terminal
            .draw(|frame| draw(frame, frame_no))
            .expect("drawing the probe frame");
        frame_no += 1;
        if let Some(limit) = ticks
            && frame_no >= limit
        {
            break;
        }
        if event::poll(Duration::from_millis(100)).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
        {
            let ctrl_c =
                key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
            if key.code == KeyCode::Char('q') || ctrl_c {
                break;
            }
        }
    }
    ratatui::restore();
}

#[allow(clippy::too_many_lines)] // a display of numbered cases, not logic
fn draw(frame: &mut Frame, frame_no: u32) {
    let area = Rect::new(
        0,
        0,
        74.min(frame.area().width),
        14.min(frame.area().height),
    );
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BRASS))
        .title("┤OSC 8 PROBE├");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 60 || inner.height < 11 {
        return;
    }

    let label = Style::default().fg(VERDIGRIS).add_modifier(Modifier::BOLD);
    let linkish = Style::default().add_modifier(Modifier::UNDERLINED);
    let buf = frame.buffer_mut();
    let x = inner.x;
    let y = inner.y;

    // Case A: a static link. With the ticker below forcing a fresh diff every
    // frame, its cells must reach the wire exactly once across the whole run.
    buf.set_string(x, y, "STATIC   Example Domain", label);
    buf.set_style(Rect::new(x + 9, y, 14, 1), linkish);
    linkify(
        buf,
        y,
        x + 9,
        x + 23,
        "https://example.com/static",
        "probe-static",
    );

    // Case B: no link at all — it exists to change every frame so the diff
    // machinery runs, which is what makes case A's count meaningful.
    buf.set_string(x, y + 1, format!("TICKER   frame {frame_no:06}"), label);

    // Case C: a link whose label flips every 10 frames, so re-emission
    // through the diff is exercised on purpose. Both labels are 11 cells and
    // share the "state " prefix, so a flip rewrites only the changed tail —
    // which self-contained cells survive and a spanning link would not.
    let toggled = if (frame_no / 10).is_multiple_of(2) {
        "state alpha"
    } else {
        "state beta "
    };
    buf.set_string(x, y + 2, format!("TOGGLE   {toggled}"), label);
    buf.set_style(Rect::new(x + 9, y + 2, 11, 1), linkish);
    linkify(
        buf,
        y + 2,
        x + 9,
        x + 20,
        "https://example.com/toggle",
        "probe-toggle",
    );

    // Case D: one logical link wrapped across two rows. The shared id= is
    // what tells the terminal every cell belongs to the same link, so
    // hovering either row highlights both.
    let row1 = "this headline is long enough that it";
    let row2 = "continues on the following row";
    buf.set_string(x, y + 3, format!("WRAPPED  {row1}"), label);
    buf.set_string(x + 9, y + 4, row2, label);
    buf.set_style(Rect::new(x + 9, y + 3, row1.width() as u16, 1), linkish);
    buf.set_style(Rect::new(x + 9, y + 4, row2.width() as u16, 1), linkish);
    let wrap_url = "https://example.com/wrapped";
    linkify(
        buf,
        y + 3,
        x + 9,
        x + 9 + row1.width() as u16,
        wrap_url,
        "probe-wrap",
    );
    linkify(
        buf,
        y + 4,
        x + 9,
        x + 9 + row2.width() as u16,
        wrap_url,
        "probe-wrap",
    );

    // Case E: wide glyphs inside a link, with a column marker to make drift
    // visible. The `|` on both rows must line up in a capture; if ForcedWidth
    // lied about a cell, the second one moves.
    let wide = "日本語 link";
    buf.set_string(
        x,
        y + 5,
        format!("         {:w$}|", "", w = wide.width()),
        label,
    );
    buf.set_string(x, y + 6, format!("WIDE     {wide}|"), label);
    buf.set_style(Rect::new(x + 9, y + 6, wide.width() as u16, 1), linkish);
    linkify(
        buf,
        y + 6,
        x + 9,
        x + 9 + wide.width() as u16,
        "https://example.com/wide",
        "probe-wide",
    );

    // Case F: the deliberately unsound spanning strategy — open on the first
    // cell, close on the last — around a label whose middle flips while both
    // ends hold still. The captured stream shows the flipped cells rewritten
    // with no open sequence anywhere near them: those cells have silently
    // left the link. This case exists to be read in the capture, not copied.
    let mid = if (frame_no / 10).is_multiple_of(2) {
        "aaaa"
    } else {
        "bbbb"
    };
    buf.set_string(x, y + 8, format!("SPANHAZ  fixed {mid} fixed"), label);
    buf.set_style(Rect::new(x + 9, y + 8, 16, 1), linkish);
    linkify_span(
        buf,
        y + 8,
        x + 9,
        x + 25,
        "https://example.com/span-hazard",
        "probe-span",
    );

    buf.set_string(
        x,
        y + 10,
        "q quits · --ticks N for headless runs",
        Style::default().fg(VERDIGRIS),
    );
}

/// Turns the cells of row `y` from `x0` to `x1` (exclusive) into an OSC 8
/// hyperlink, leaving the glyphs and styles the widgets drew untouched.
///
/// Every cell is made self-contained — open sequence, its own glyph, close
/// sequence — declared at the glyph's real width via
/// `CellDiffOption::ForcedWidth` so the escape bytes cost no columns. The
/// shared `id` makes the cells one logical link to the terminal, and it is
/// also what lets a wrapped link span rows: call this once per row with the
/// same `id` and URL. Self-containment is what makes the scheme safe against
/// partial redraws — see the module docs and case F for the alternative.
///
/// A continuation cell of a wide glyph (width 0) is left alone: it renders
/// as the skipped half of its neighbour, and the neighbour's own sequences
/// already cover both columns.
fn linkify(buf: &mut Buffer, y: u16, x0: u16, x1: u16, url: &str, id: &str) {
    let open = format!("\x1b]8;id={id};{url}\x1b\\");
    let close = "\x1b]8;;\x1b\\";
    for x in x0..x1 {
        if cell_width(buf, x, y) > 0 {
            wrap_cell(buf, x, y, &open, close);
        }
    }
}

/// The spanning variant — open rides in the first cell, close in the last —
/// kept only so case F can demonstrate on the wire why it is unsound. OSC 8
/// is modal, and the diff re-emits individual cells: a change confined to
/// the middle of the span rewrites those cells with no link active, and a
/// change that re-emits the opener without the closer would leak link state
/// onto everything printed after it.
fn linkify_span(buf: &mut Buffer, y: u16, x0: u16, x1: u16, url: &str, id: &str) {
    if x1 <= x0 {
        return;
    }
    let open = format!("\x1b]8;id={id};{url}\x1b\\");
    let close = "\x1b]8;;\x1b\\";
    if x1 == x0 + 1 {
        wrap_cell(buf, x0, y, &open, close);
        return;
    }
    wrap_cell(buf, x0, y, &open, "");
    let mut last = x1 - 1;
    if cell_width(buf, last, y) == 0 && last > x0 {
        last -= 1;
    }
    wrap_cell(buf, last, y, "", close);
}

fn cell_width(buf: &Buffer, x: u16, y: u16) -> u16 {
    buf.cell((x, y)).map_or(1, |c| c.symbol().width() as u16)
}

fn wrap_cell(buf: &mut Buffer, x: u16, y: u16, before: &str, after: &str) {
    let Some(cell) = buf.cell_mut((x, y)) else {
        return;
    };
    let glyph = cell.symbol().to_string();
    let width = glyph.width().max(1) as u16;
    cell.set_symbol(&format!("{before}{glyph}{after}"));
    cell.set_diff_option(CellDiffOption::ForcedWidth(
        NonZeroU16::new(width).expect("width floored at 1"),
    ));
}
