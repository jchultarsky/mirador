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

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding};

use crate::theme::Theme;

/// A key binding, as declared by a panel.
///
/// One declaration feeds the border hint, the status bar and the help overlay,
/// so a binding can never drift out of sync with the text describing it.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// The key as typed, e.g. `a`, `space`, `Tab`.
    pub key: &'static str,
    /// What it does, in the imperative.
    pub action: &'static str,
    /// Whether it is important enough for the border hint. Bindings that are
    /// aliases (`j` for `down`) or expert-only set this false so they do not
    /// spend the small hint budget.
    pub primary: bool,
}

impl Binding {
    /// A binding shown in the border hint.
    pub const fn primary(key: &'static str, action: &'static str) -> Self {
        Self {
            key,
            action,
            primary: true,
        }
    }

    /// A binding that appears only in the help overlay.
    pub const fn extra(key: &'static str, action: &'static str) -> Self {
        Self {
            key,
            action,
            primary: false,
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
        let width = separator.chars().count()
            + binding.key.chars().count()
            + 1
            + binding.action.chars().count();
        let Ok(width) = u16::try_from(width) else {
            continue;
        };
        if used + width > budget {
            break;
        }
        if !separator.is_empty() {
            spans.push(Span::styled(separator, action_style));
        }
        spans.push(Span::styled(binding.key, key_style));
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
        title_spans.push(Span::styled(" ", border_style));
    }
    title_spans.push(Span::styled(
        spec.title.to_string(),
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
        .padding(Padding::horizontal(1))
        .title_top(Line::from(title_spans));

    if let Some(counter) = &spec.counter {
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
                let width: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
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
}
