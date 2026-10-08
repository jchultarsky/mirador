//! Panel frames, and the key hints drawn into them.
//!
//! The frame is treated as a widget bus rather than as decoration — the idea
//! bpytop uses to keep its interior rows entirely for data. Titles, counters
//! and key hints are punched into the border line with bracket glyphs, so they
//! cost zero interior rows.
//!
//! Key discoverability follows the layered scheme that modern TUIs converge
//! on. The *focused* panel carries its own two or three verbs in its bottom
//! border, physically attached to the thing they act on. Global bindings live
//! in the status bar. Everything else is behind `?`. Crucially, hints are only
//! ever shown for the focused panel: an undifferentiated list of every
//! binding teaches users to try keys on the wrong panel.
//!
//! Focus is signalled by *recession* rather than by brightening — unfocused
//! frames dim, so exactly one thing on screen is at normal brightness.

use std::borrow::Cow;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding};

use crate::theme::Theme;

/// Columns a frame costs a panel: a border and a padding column on each side.
///
/// This belongs here rather than in each widget, because it is a property of
/// how [`draw`] builds its `Block` — see the `Borders::ALL` and
/// `Padding::horizontal(1)` below — not of any panel. Five widgets each defined
/// their own copy of this and the next constant to work out `max_width` and
/// `max_height`, which meant changing the padding in one place here would have
/// silently desynchronised ten figures elsewhere.
pub const FRAME_WIDTH: u16 = 4;

/// Rows a frame costs a panel: the two borders.
///
/// The interior padding is horizontal only, so it does not enter a height.
pub const FRAME_HEIGHT: u16 = 2;

/// A rectangle of the given size, centred inside `area`.
///
/// Clamped to `area` first, so a dialog larger than the terminal is trimmed
/// rather than being placed outside it — a `Rect` that starts past the right
/// edge is not drawn at all, which reads as a dialog that failed to open.
pub fn centred(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

/// Esc, as every dialog spells it: the key no dialog's table can move.
///
/// The form [`crate::keymap::Key`] displays, which is what the key map lists
/// and what a key table's errors say. The dialogs once had three spellings
/// of it between them — `esc`, `ESC` and `Esc`.
pub const ESC: &str = "Esc";

/// The frame a dialog is drawn in: rounded, padded, in the focused border
/// colour, with `title` punched into the top border as `┤TITLE├` — the way
/// [`draw`] punches a panel's.
///
/// One builder for the four dialogs, which built it by hand and had drifted:
/// two drew the brackets and two laid a bare title on the border. `width` is
/// the dialog's own. A title too long for it is cut and says so with `…`,
/// rather than losing its closing `├` to the corner, and one cut down to the
/// ellipsis alone is not drawn — the rule `draw` keeps for a panel.
pub fn dialog_block(theme: &Theme, title: &str, width: u16) -> Block<'static> {
    let border = Style::default().fg(theme.border_focused);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .padding(Padding::horizontal(1));
    // Inside the two corners, less the two brackets.
    let room = usize::from(width).saturating_sub(4);
    let title = crate::grid::truncate(&crate::glyphs::utility(title), room);
    if title.is_empty() || title == "…" {
        return block;
    }
    block.title_top(Line::from(vec![
        Span::styled("┤", border),
        Span::styled(
            title,
            Style::default()
                .fg(theme.title)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("├", border),
    ]))
}

/// A dialog's key row: each primary binding as `key action`, the key bold in
/// the key colour, `gap` between them, fitted to `width`.
///
/// The bindings are the ones a dialog's key table derives, so a moved key, a
/// relabelled action or a hint made primary reaches the row with nothing
/// else to edit — invariant 3, which the two pickers' footers broke by
/// spelling their labels out a second time.
///
/// The last binding is the way out — each dialog appends Esc's with
/// `with_fixed` — and it is the last to go: hints drop whole, the ones
/// before it first, and Esc alone and still too wide is cut with `…`. The
/// row was built at its natural width and the terminal cut it wherever the
/// edge fell, `Esc put` with no `back`, and narrower still the keep hint
/// whole and Esc gone with nothing to say so (invariant 19).
pub fn key_row(bindings: &[Binding], gap: &str, theme: &Theme, width: u16) -> Line<'static> {
    let key_style = Style::default().fg(theme.key).add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(theme.muted);
    let hints = bindings
        .iter()
        .filter(|binding| binding.primary)
        .map(|binding| {
            vec![
                Span::styled(binding.key.clone(), key_style),
                Span::styled(format!(" {}", binding.action), muted),
            ]
        })
        .collect();
    crate::grid::assemble(
        crate::grid::way_out_last(hints, &Span::styled(gap.to_string(), muted), width.into()),
        width,
    )
}

/// A key binding, as declared by a panel.
///
/// One declaration feeds the border hint, the status bar and the help overlay,
/// so a binding can never drift out of sync with the text describing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The key as typed, e.g. `a`, `space`, `Tab`.
    pub key: Cow<'static, str>,
    /// What it does, in the imperative.
    pub action: Cow<'static, str>,
    /// Whether it is important enough for the border hint. Bindings that are
    /// aliases (`j` for `down`) or expert-only set this false so they do not
    /// spend the small hint budget.
    pub primary: bool,
}

impl Binding {
    /// A binding shown in the border hint.
    pub const fn primary(key: &'static str, action: &'static str) -> Self {
        Self {
            key: Cow::Borrowed(key),
            action: Cow::Borrowed(action),
            primary: true,
        }
    }

    /// A binding that appears only in the help overlay.
    pub const fn extra(key: &'static str, action: &'static str) -> Self {
        Self {
            key: Cow::Borrowed(key),
            action: Cow::Borrowed(action),
            primary: false,
        }
    }

    /// A binding supplied at runtime, for example by an external plugin.
    pub fn owned(key: impl Into<String>, action: impl Into<String>, primary: bool) -> Self {
        Self {
            key: Cow::Owned(key.into()),
            action: Cow::Owned(action.into()),
            primary,
        }
    }
}

/// Render `bindings` as `key action · key action`, keys highlighted.
///
/// Returns `None` when nothing fits, so the caller can leave the border clean
/// rather than drawing a truncated fragment.
pub fn hint_line(bindings: &[Binding], theme: &Theme, budget: u16) -> Option<Line<'static>> {
    if bindings.is_empty() || budget < 8 {
        return None;
    }

    let key_style = Style::default().fg(theme.key).add_modifier(Modifier::BOLD);
    let action_style = Style::default().fg(theme.muted);

    let mut spans = vec![Span::styled(" ", action_style)];
    let mut used = 2u16; // the leading and trailing space

    for binding in bindings.iter().filter(|b| b.primary) {
        let separator = if spans.len() > 1 { " · " } else { "" };
        let width = crate::grid::display_width(separator)
            + crate::grid::display_width(&binding.key)
            + 1
            + crate::grid::display_width(&binding.action);
        let Ok(width) = u16::try_from(width) else {
            continue;
        };
        if used + width > budget {
            break;
        }
        if !separator.is_empty() {
            spans.push(Span::styled(separator, action_style));
        }
        spans.push(Span::styled(binding.key.clone(), key_style));
        spans.push(Span::styled(format!(" {}", binding.action), action_style));
        used += width;
    }

    if spans.len() == 1 {
        return None;
    }
    spans.push(Span::styled(" ", action_style));
    Some(Line::from(spans))
}

/// How a panel's frame should be drawn.
pub struct FrameSpec<'a> {
    /// Panel name, shown top-left.
    pub title: &'a str,
    /// Optional status shown top-right, e.g. `3 open` or `delayed`.
    pub counter: Option<String>,
    /// Whether this panel has keyboard focus.
    pub focused: bool,
    /// Bindings for the bottom border. Only drawn when focused.
    pub bindings: &'a [Binding],
    /// One-based index used for the jump key shown beside the title.
    pub index: usize,
}

/// Draw the frame and return the interior area.
///
/// The returned rect already accounts for one column of horizontal padding.
/// Content sitting flush against a border is the single biggest reason a
/// terminal layout reads as mush, and padding is cheaper than any amount of
/// colour work.
pub fn draw(frame: &mut ratatui::Frame, area: Rect, theme: &Theme, spec: &FrameSpec<'_>) -> Rect {
    if area.width < 2 || area.height < 2 {
        return Rect::new(area.x, area.y, 0, 0);
    }

    let border_style = Style::default().fg(if spec.focused {
        theme.border_focused
    } else {
        theme.border
    });

    // The title and the counter are two separate `title_top` calls, one
    // left-aligned and one right. When they collide ratatui clips the
    // left-aligned one, and what it clips off is the closing `├` — so
    // `╭┤9 CPU├───┤18 cores├╮` degrades to `╭┤9 CPU┤18 cores├╮`, which reads as
    // a broken frame rather than a narrow one. Budget for the counter and
    // shorten the title's *text* instead, which is the part that can afford it.
    let available = usize::from(area.width).saturating_sub(2);
    let wanted = spec
        .counter
        .as_ref()
        .map_or(0, |counter| crate::grid::display_width(counter) + 2);
    // A counter with no room even on its own is dropped rather than drawn
    // across the corner.
    let counter_width = if wanted <= available { wanted } else { 0 };

    let index_width = usize::from(spec.index <= 9) * 2;
    // The title's own `┤` and `├`, plus one `─` so the two segments never sit
    // flush against each other.
    let title_budget = available.saturating_sub(counter_width + index_width + 3);
    let title = crate::grid::truncate(spec.title, title_budget);
    // `truncate` at one cell has nothing to spend on the name and returns the
    // ellipsis alone. `┤…├` is three cells that say a title was cut without
    // saying anything about which panel this is — the reader learns less than
    // from a plain border, and pays for it. The jump key beside it is a
    // different matter and stays: `┤4├` is still an answer to a question
    // somebody has.
    let title_is_gone = title == "…";
    let title = if title_is_gone { String::new() } else { title };

    // The jump key rides in the title, so panel switching is discoverable
    // without spending a legend row on it.
    let mut title_spans = vec![Span::styled("┤", border_style)];
    if spec.index <= 9 {
        title_spans.push(Span::styled(
            format!("{}", spec.index),
            Style::default()
                .fg(if spec.focused { theme.key } else { theme.muted })
                .add_modifier(Modifier::BOLD),
        ));
        if !title_is_gone {
            title_spans.push(Span::styled(" ", border_style));
        }
    }
    title_spans.push(Span::styled(
        title,
        if spec.focused {
            Style::default()
                .fg(theme.title)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.muted)
        },
    ));
    title_spans.push(Span::styled("├", border_style));

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .padding(Padding::horizontal(1));

    // A panel too narrow for even one character of its name goes without one.
    // An empty `┤├` is not a smaller title, it is a mark on the border that
    // means nothing — and neither is a lone `┤…├`, which is why a title cut
    // down to nothing but its ellipsis leaves the segment to the jump key or
    // drops it altogether.
    if title_budget > 0 && !(title_is_gone && spec.index > 9) {
        block = block.title_top(Line::from(title_spans));
    }

    if let Some(counter) = spec.counter.as_ref().filter(|_| counter_width > 0) {
        // Right-aligned in the top border, the way a list's "N of M" sits in
        // btop and lazygit — status without a row of its own.
        block = block.title_top(
            Line::from(vec![
                Span::styled("┤", border_style),
                Span::styled(
                    counter.clone(),
                    Style::default().fg(if spec.focused {
                        theme.label
                    } else {
                        theme.muted
                    }),
                ),
                Span::styled("├", border_style),
            ])
            .right_aligned(),
        );
    }

    // Hints only on the focused panel, and only when the frame is wide enough
    // that they will not be clipped mid-word.
    if spec.focused {
        let budget = area.width.saturating_sub(8);
        if let Some(hints) = hint_line(spec.bindings, theme, budget) {
            block = block.title_bottom(hints.centered());
        }
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Draw a labelled hairline rule across `area`.
///
/// Used only where content genuinely splits — "now" from "next", a summary
/// from its rows. A rule that merely decorates makes a panel busier, not
/// clearer.
pub fn rule(frame: &mut ratatui::Frame, area: Rect, theme: &Theme, label: &str) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let label = crate::glyphs::utility(label);
    let label_width = u16::try_from(label.chars().count()).unwrap_or(0);
    let rule_style = Style::default().fg(theme.rule);

    let mut spans = Vec::new();
    if label.is_empty() || label_width + 4 > area.width {
        spans.push(Span::styled("─".repeat(area.width as usize), rule_style));
    } else {
        spans.push(Span::styled(
            label,
            Style::default()
                .fg(theme.label)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" ", rule_style));
        let remaining = area.width.saturating_sub(label_width + 1);
        spans.push(Span::styled("─".repeat(remaining as usize), rule_style));
    }

    frame.render_widget(ratatui::widgets::Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// The top border of a panel, as drawn.
    fn top_border(width: u16, title: &str, counter: Option<&str>) -> String {
        top_border_indexed(width, title, counter, 9)
    }

    /// The same, for a panel past the ninth — which has no jump key, so its
    /// title segment has nothing else in it to fall back on.
    fn top_border_indexed(width: u16, title: &str, counter: Option<&str>, index: usize) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 3)).expect("backend");
        terminal
            .draw(|frame| {
                draw(
                    frame,
                    Rect::new(0, 0, width, 3),
                    &Theme::default(),
                    &FrameSpec {
                        title,
                        counter: counter.map(str::to_string),
                        focused: false,
                        bindings: &[],
                        index,
                    },
                );
            })
            .expect("draws");
        let buffer = terminal.backend().buffer();
        (0..width)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<String>()
    }

    /// A narrow panel used to render `╭┤9 CPU┤18 cores├╮`: the title and the
    /// counter are separate `title_top` calls, and ratatui clipped the
    /// left-aligned one, eating the `├` that closes it. Every other segment on
    /// every other border has both brackets, so one that does not reads as a
    /// broken frame rather than a narrow one.
    #[test]
    fn a_title_too_wide_for_its_border_is_shortened_not_left_unclosed() {
        for width in 12..40u16 {
            let border = top_border(width, "Weather — Boston, Massachusetts", Some("at 03:45"));
            assert_eq!(
                border.matches('├').count(),
                border.matches('┤').count(),
                "every segment opens and closes, at width {width}: {border}"
            );
        }
    }

    /// `╭┤…├─┤18 cores├╮` is what the CPU panel drew at 80 columns: three cells
    /// spent saying a title was cut, and nothing at all about which panel this
    /// is. A bare border says the same for free, and the counter beside it was
    /// already doing the identifying.
    ///
    /// The jump key is the exception and keeps its segment, because `┤4├` still
    /// answers a question somebody has.
    #[test]
    fn a_title_cut_down_to_its_ellipsis_is_not_drawn() {
        let mut ever_tight = false;
        for width in 6..40u16 {
            let unnumbered = top_border_indexed(width, "CPU", Some("18 cores"), 11);
            assert!(
                !unnumbered.contains("┤…├"),
                "a lone ellipsis is not a title, at width {width}: {unnumbered}"
            );

            let numbered = top_border_indexed(width, "CPU", Some("18 cores"), 4);
            assert!(
                !numbered.contains("┤…├"),
                "nor when there is a jump key beside it, at width {width}: {numbered}"
            );
            if numbered.contains("┤4├") {
                ever_tight = true;
                assert!(
                    !numbered.contains('…'),
                    "the key is kept and the ellipsis dropped, at width {width}: {numbered}"
                );
            }
        }
        assert!(
            ever_tight,
            "no width in the sweep was tight enough to reach the case under test"
        );
    }

    /// The whole border still has to fit, and a title long enough to push the
    /// counter off the end would be the same bug wearing a different hat.
    #[test]
    fn the_counter_survives_a_title_that_wants_the_whole_border() {
        let border = top_border(30, "An Extremely Long Panel Name Indeed", Some("12 open"));
        assert!(
            border.contains("12 open"),
            "the counter is not squeezed out: {border}"
        );
        assert!(border.contains('…'), "the title says it was cut: {border}");
    }

    fn bindings() -> Vec<Binding> {
        vec![
            Binding::primary("a", "add"),
            Binding::primary("e", "edit"),
            Binding::primary("d", "delete"),
            Binding::extra("j", "down"),
        ]
    }

    #[test]
    fn hints_include_only_primary_bindings() {
        let line = hint_line(&bindings(), &Theme::default(), 200).expect("should fit");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("add"));
        assert!(text.contains("edit"));
        assert!(!text.contains("down"), "extras belong in the help overlay");
    }

    #[test]
    fn hints_stop_at_the_budget_rather_than_overflowing() {
        let theme = Theme::default();
        for budget in 0..80u16 {
            if let Some(line) = hint_line(&bindings(), &theme, budget) {
                let width: usize = line
                    .spans
                    .iter()
                    .map(|span| crate::grid::display_width(&span.content))
                    .sum();
                assert!(
                    width <= budget as usize,
                    "hint of {width} exceeded budget {budget}"
                );
            }
        }
    }

    #[test]
    fn a_tiny_budget_produces_no_hint_at_all() {
        // Better a clean border than a clipped fragment.
        assert!(hint_line(&bindings(), &Theme::default(), 4).is_none());
        assert!(hint_line(&bindings(), &Theme::default(), 0).is_none());
    }

    #[test]
    fn runtime_binding_hints_budget_terminal_cells() {
        let binding = Binding::owned("界", "open", true);
        let line = hint_line(&[binding], &Theme::default(), 9).expect("exactly fits");
        let width: usize = line
            .spans
            .iter()
            .map(|span| crate::grid::display_width(&span.content))
            .sum();
        assert_eq!(width, 9);
    }

    #[test]
    fn no_bindings_means_no_hint() {
        assert!(hint_line(&[], &Theme::default(), 100).is_none());
        let extras_only = [Binding::extra("j", "down")];
        assert!(hint_line(&extras_only, &Theme::default(), 100).is_none());
    }

    #[test]
    fn the_frame_returns_a_padded_interior() {
        let theme = Theme::default();
        let area = Rect::new(0, 0, 40, 10);
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let mut inner = Rect::default();
        terminal
            .draw(|f| {
                inner = draw(
                    f,
                    area,
                    &theme,
                    &FrameSpec {
                        title: "Tasks",
                        counter: None,
                        focused: true,
                        bindings: &bindings(),
                        index: 1,
                    },
                );
            })
            .unwrap();

        // One border column plus one padding column on each side.
        assert_eq!(inner.width, area.width - 4);
        assert_eq!(inner.height, area.height - 2);
        assert_eq!(inner.x, area.x + 2);
    }

    #[test]
    fn the_frame_degrades_instead_of_panicking_when_tiny() {
        let theme = Theme::default();
        for (w, h) in [(0, 0), (1, 1), (2, 2), (3, 3), (5, 2)] {
            let mut terminal = Terminal::new(TestBackend::new(w.max(1), h.max(1))).unwrap();
            terminal
                .draw(|f| {
                    let inner = draw(
                        f,
                        Rect::new(0, 0, w, h),
                        &theme,
                        &FrameSpec {
                            title: "Tasks",
                            counter: Some("3 open".into()),
                            focused: true,
                            bindings: &bindings(),
                            index: 1,
                        },
                    );
                    assert!(inner.width <= w);
                    assert!(inner.height <= h);
                })
                .unwrap();
        }
    }

    #[test]
    fn the_title_and_counter_appear_in_the_top_border() {
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(44, 6)).unwrap();
        terminal
            .draw(|f| {
                draw(
                    f,
                    Rect::new(0, 0, 44, 6),
                    &theme,
                    &FrameSpec {
                        title: "Tasks",
                        counter: Some("3 open".into()),
                        focused: true,
                        bindings: &bindings(),
                        index: 2,
                    },
                );
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let top: String = (0..44).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(top.contains("Tasks"), "got: {top}");
        assert!(
            top.contains("3 open"),
            "counter belongs in the frame: {top}"
        );
        assert!(
            top.contains('2'),
            "the jump key belongs in the title: {top}"
        );
    }

    #[test]
    fn hints_appear_only_when_the_panel_is_focused() {
        let theme = Theme::default();
        let render = |focused: bool| {
            let mut terminal = Terminal::new(TestBackend::new(50, 6)).unwrap();
            terminal
                .draw(|f| {
                    draw(
                        f,
                        Rect::new(0, 0, 50, 6),
                        &theme,
                        &FrameSpec {
                            title: "Tasks",
                            counter: None,
                            focused,
                            bindings: &bindings(),
                            index: 1,
                        },
                    );
                })
                .unwrap();
            let buf = terminal.backend().buffer();
            (0..50).map(|x| buf[(x, 5)].symbol()).collect::<String>()
        };

        assert!(render(true).contains("add"), "focused panels show hints");
        assert!(
            !render(false).contains("add"),
            "unfocused hints teach users to press keys on the wrong panel"
        );
    }

    #[test]
    fn a_rule_fills_its_width_exactly() {
        let theme = Theme::default();
        for width in [0u16, 1, 5, 12, 40] {
            let mut terminal = Terminal::new(TestBackend::new(width.max(1), 1)).unwrap();
            terminal
                .draw(|f| rule(f, Rect::new(0, 0, width, 1), &theme, "next hours"))
                .unwrap();
        }
    }

    #[test]
    fn a_rule_drops_its_label_when_there_is_no_room() {
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(6, 1)).unwrap();
        terminal
            .draw(|f| rule(f, Rect::new(0, 0, 6, 1), &theme, "next hours"))
            .unwrap();
        let buf = terminal.backend().buffer();
        let line: String = (0..6).map(|x| buf[(x, 0)].symbol()).collect();
        assert_eq!(line, "──────", "a clipped label is worse than none");
    }

    /// A dialog narrower than its title cuts the title and says so, rather
    /// than letting the corner take its closing `├` and leave `┤KEY M` on the
    /// border; one with no room for a letter of it draws no title at all.
    #[test]
    fn a_dialog_title_is_cut_with_its_brackets_or_not_drawn() {
        let theme = Theme::default();
        for width in 1..=16u16 {
            let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
            terminal
                .draw(|f| f.render_widget(dialog_block(&theme, "key map", width), f.area()))
                .unwrap();
            let buf = terminal.backend().buffer();
            let top: String = (0..width).map(|x| buf[(x, 0)].symbol()).collect();
            if width >= 11 {
                assert!(top.contains("┤KEY MAP├"), "{width}: {top}");
            } else if width >= 6 {
                assert!(top.contains("…├"), "{width}: cut and marked: {top}");
            } else {
                assert!(!top.contains('┤'), "{width}: no room, no title: {top}");
            }
            assert_eq!(
                top.contains('┤'),
                top.contains('├'),
                "{width}: a bracket without its partner: {top}"
            );
        }
    }

    /// The first row of an 80x30 screen with a dialog's top border in it.
    fn dialog_top(draw: impl FnOnce(&mut ratatui::Frame)) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal.draw(draw).unwrap();
        let buf = terminal.backend().buffer();
        (0..30)
            .map(|y| (0..80).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .find(|row| row.contains('╭'))
            .expect("a dialog")
            .trim()
            .to_string()
    }

    /// The four dialogs built their frames by hand and drew their titles two
    /// ways: `┤THEME├` and `┤KEY MAP├` punched into the border the way every
    /// panel's is, `PANELS` and a prompt's label laid on it with no brackets.
    /// One builder now, and this reads each dialog's own top border.
    #[test]
    fn every_dialog_punches_its_title_into_the_border() {
        let theme = Theme::default();
        let picker = crate::picker::Picker::new(vec!["clocks".into()]);
        let themes = crate::theme_picker::ThemePicker::new(None, None);
        let prompt = crate::prompt::Prompt::new(
            "ADD A CLOCK",
            "Esc cancels",
            "",
            crate::prompt::Completion::None,
        );
        let mut keys = crate::keymap_dialog::KeymapDialog::new();
        let keymap = crate::keymap::Keymap::default();
        let tops = [
            (
                dialog_top(|f| picker.render(f, f.area(), &theme, |_| false, None)),
                "┤PANELS├",
            ),
            (
                dialog_top(|f| themes.render(f, f.area(), &theme)),
                "┤THEME├",
            ),
            (
                dialog_top(|f| prompt.render(f, f.area(), &theme)),
                "┤ADD A CLOCK├",
            ),
            (
                dialog_top(|f| keys.render(f, f.area(), &keymap, &[], None, &theme)),
                "┤KEY MAP├",
            ),
        ];
        for (top, title) in tops {
            assert!(top.starts_with(&format!("╭{title}─")), "{top}");
        }
    }

    /// The rows inside a dialog drawn on a `width`x30 screen, between its side
    /// borders and trimmed — the last is its footer. Empty where the dialog
    /// is too narrow to have an inside.
    fn dialog_rows(width: u16, draw: impl FnOnce(&mut ratatui::Frame)) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
        terminal.draw(draw).unwrap();
        let buf = terminal.backend().buffer();
        let rows: Vec<String> = (0..30)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        let (Some(top), Some(bottom)) = (
            rows.iter().position(|row| row.contains('╭')),
            rows.iter().position(|row| row.contains('╰')),
        ) else {
            return Vec::new();
        };
        rows[top + 1..bottom]
            .iter()
            .filter_map(|row| {
                let inside = row.trim().strip_prefix('│')?.strip_suffix('│')?;
                Some(inside.trim().to_string())
            })
            .collect()
    }

    /// What a dialog's key row has to say: its hints but the way out, the
    /// gap between them, the way out, and the whole row where there is room.
    struct Footer {
        dialog: &'static str,
        hints: &'static [&'static str],
        gap: &'static str,
        esc: &'static str,
        full: &'static str,
    }

    impl Footer {
        /// Invariant 19 for a dialog's key row, and the way out last: at
        /// every width from 1 to 100 the row is whole hints ending in `esc`,
        /// or `esc` alone cut with an `…`, and it says nothing only where the
        /// dialog has no room inside its border and padding.
        fn sweep(&self, mut draw: impl FnMut(&mut ratatui::Frame)) {
            let Self { dialog, esc, .. } = self;
            let (mut cut, mut dropped) = (false, false);
            for width in 1..=100u16 {
                let rows = dialog_rows(width, &mut draw);
                let Some(footer) = rows.last() else {
                    continue;
                };
                if footer.is_empty() {
                    assert!(width < 5, "{dialog} at {width}: room, and nothing said");
                    continue;
                }
                if let Some(head) = footer.strip_suffix('…') {
                    assert!(
                        esc.starts_with(head.trim_end()),
                        "{dialog} at {width}: cut, and not down to {esc:?} alone: {footer:?}"
                    );
                    cut = true;
                } else {
                    self.assert_whole(width, footer);
                    dropped |= footer == esc;
                }
                if width == 100 {
                    assert_eq!(
                        footer, self.full,
                        "{dialog}: every hint where there is room"
                    );
                }
            }
            // A sweep that never reached the cut tests nothing.
            assert!(cut && dropped, "{dialog}: cut {cut}, dropped {dropped}");
        }

        /// `footer` is whole hints, `gap` between them, ending in `esc`.
        fn assert_whole(&self, width: u16, footer: &str) {
            let Self {
                dialog,
                hints,
                gap,
                esc,
                ..
            } = self;
            let Some(rest) = footer.strip_suffix(esc) else {
                panic!("{dialog} at {width}: {footer:?} does not end in {esc:?}");
            };
            if rest.is_empty() {
                return;
            }
            let rest = rest.strip_suffix(gap).unwrap_or_else(|| {
                panic!("{dialog} at {width}: {footer:?} has no gap before {esc:?}")
            });
            for hint in rest.split(gap) {
                assert!(
                    hints.contains(&hint),
                    "{dialog} at {width}: {hint:?} is not a whole hint in {footer:?}"
                );
            }
        }
    }

    /// The terminal cut the pickers' key rows wherever the edge fell —
    /// `↵ keep  Esc put` with no `back` and no mark, and narrower still the
    /// keep hint whole and Esc, the way out, gone without a trace. The key
    /// map's row dropped from the end, so Esc went first; and a prompt on a
    /// screen under twenty columns measured its help row for the twenty it
    /// asked for, and the terminal cut that to `Esc can`.
    #[test]
    fn every_dialog_footer_drops_whole_hints_and_keeps_esc_longest() {
        let theme = Theme::default();
        let picker = crate::picker::Picker::new(
            crate::widgets::WIDGET_NAMES
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        );
        Footer {
            dialog: "the panel picker",
            hints: &["space toggle"],
            gap: "   ",
            esc: "Esc close",
            full: "space toggle   Esc close",
        }
        .sweep(|f| picker.render(f, f.area(), &theme, |_| false, None));

        let themes = crate::theme_picker::ThemePicker::new(None, None);
        Footer {
            dialog: "the theme picker",
            hints: &["↵ keep"],
            gap: "  ",
            esc: "Esc put back",
            full: "↵ keep  Esc put back",
        }
        .sweep(|f| themes.render(f, f.area(), &theme));

        let prompt = crate::prompt::Prompt::new(
            "ADD A CLOCK",
            "Tab completes · Esc cancels",
            "",
            crate::prompt::Completion::None,
        );
        Footer {
            dialog: "a prompt",
            hints: &["Tab completes"],
            gap: " · ",
            esc: "Esc cancels",
            full: "Tab completes · Esc cancels",
        }
        .sweep(|f| prompt.render(f, f.area(), &theme));

        let mut keys = crate::keymap_dialog::KeymapDialog::new();
        let keymap = crate::keymap::Keymap::default();
        Footer {
            dialog: "the key map",
            hints: &["↓ more", "↑↓ more", "↑ more", "r reload", "d defaults"],
            gap: "  ",
            esc: "Esc close",
            full: "r reload  d defaults  Esc close",
        }
        .sweep(|f| keys.render(f, f.area(), &keymap, &[], None, &theme));
    }

    /// The panel picker's status row is prose — its usual line, or what was
    /// just refused, or why the config could not be written — and is whole
    /// or cut with an `…`. The terminal cut the usual line to `written to
    /// your config on cl` on a screen under 37 columns, and an error longer
    /// than the dialog's 36 inside columns at every size.
    #[test]
    fn the_panel_pickers_status_row_is_whole_or_says_it_was_cut() {
        let theme = Theme::default();
        let picker = crate::picker::Picker::new(vec!["clocks".into(), "cpu".into()]);
        for error in [
            None,
            Some("config not saved: /home/someone/.config/mirador/config.toml: permission denied"),
        ] {
            let status = error.unwrap_or("written to your config on close");
            let mut cut = false;
            for width in 1..=100u16 {
                let rows = dialog_rows(width, |f| {
                    picker.render(f, f.area(), &theme, |_| false, error);
                });
                let Some(row) = rows.len().checked_sub(2).map(|row| &rows[row]) else {
                    continue;
                };
                if let Some(head) = row.strip_suffix('…') {
                    assert!(status.starts_with(head), "{width}: {row:?}");
                    cut = true;
                } else {
                    assert!(row.is_empty() || row == status, "{width}: {row:?}");
                }
            }
            assert!(cut, "a sweep that never cut the status tests nothing");
        }
    }
}
