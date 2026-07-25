//! The application shell: layout, focus, the event loop and global bindings.
//!
//! The shell knows nothing about what any panel displays. It owns the grid, the
//! focus ring, the frames and the tick schedule, and forwards everything else
//! through the [`Panel`] trait.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap};

use crate::config::Config;
use crate::frame::{Binding, FrameSpec};
use crate::panel::{KeyOutcome, Panel, RenderContext};
use crate::theme::Gradients;

/// Global bindings, used for both the status bar and the help overlay.
const GLOBAL: &[Binding] = &[
    Binding::primary("Tab", "focus"),
    Binding::primary("?", "keys"),
    Binding::primary("q", "quit"),
    Binding::extra("Shift+Tab", "focus back"),
    Binding::extra("1-9", "jump to panel"),
    Binding::extra("Ctrl+←/→", "resize width"),
    Binding::extra("Ctrl+↑/↓", "resize height"),
    Binding::extra("Ctrl+C", "quit"),
];

/// Smallest weight a panel or row may be squeezed to.
const MIN_WEIGHT: u16 = 1;

/// The index to trade space with: the next one along, or the previous one when
/// `index` is last. `None` when there is nobody to trade with.
fn neighbour_of(index: usize, len: usize) -> Option<usize> {
    if len < 2 {
        return None;
    }
    if index + 1 < len {
        Some(index + 1)
    } else {
        index.checked_sub(1)
    }
}

/// A panel plus its tick bookkeeping.
struct Slot {
    panel: Box<dyn Panel>,
    last_tick: Instant,
    /// Interior rectangle the panel was last drawn into, used to route mouse
    /// events. `None` until the first draw, and while the panel is too small
    /// to render at all — in both cases there is nothing to click.
    area: Option<Rect>,
}

/// The running dashboard.
pub struct App {
    config: Config,
    gradients: Gradients,
    slots: Vec<Slot>,
    /// `(row, column)` in `config.layout` for each slot, so a resize knows
    /// which weights the focused panel is made of. Built alongside `slots`
    /// rather than recomputed, because a widget that fails to build leaves a
    /// hole and the two would drift apart.
    positions: Vec<(usize, usize)>,
    focus: usize,
    show_help: bool,
    should_quit: bool,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("panels", &self.slots.len())
            .field("focus", &self.focus)
            .finish_non_exhaustive()
    }
}

impl App {
    /// Build every panel named in the layout, in row-major order.
    pub fn new(config: Config) -> Result<Self> {
        let mut slots = Vec::new();
        // Start each panel far enough in the past that its first tick fires
        // immediately rather than one interval from now.
        let epoch = Instant::now()
            .checked_sub(Duration::from_secs(86_400))
            .unwrap();

        let mut positions = Vec::new();
        for (row_index, row) in config.layout.rows.iter().enumerate() {
            for (column_index, entry) in row.panels.iter().enumerate() {
                let panel = crate::widgets::build(&entry.widget, &config)
                    .with_context(|| format!("building the `{}` panel", entry.widget))?;
                if let Some(panel) = panel {
                    slots.push(Slot {
                        panel,
                        last_tick: epoch,
                        area: None,
                    });
                    positions.push((row_index, column_index));
                }
            }
        }

        anyhow::ensure!(
            !slots.is_empty(),
            "no panels were built; check the `[layout]` table in your config"
        );

        let gradients = config.theme.gradients();
        Ok(Self {
            config,
            gradients,
            slots,
            positions,
            focus: 0,
            show_help: false,
            should_quit: false,
        })
    }

    /// Run until the user quits.
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let tick_rate = Duration::from_millis(self.config.general.tick_rate_ms.clamp(16, 5_000));

        // Redraw only when something actually changed. Mouse reporting makes
        // this matter: the terminal sends an event for every cell the pointer
        // crosses, and drawing on each one would have a dashboard left open all
        // day burning CPU whenever the mouse passes over it.
        let mut dirty = true;

        while !self.should_quit {
            if dirty {
                terminal.draw(|frame| self.render(frame))?;
                dirty = false;
            }

            if event::poll(tick_rate)? {
                match event::read()? {
                    // Only react to presses; on Windows every key also emits a
                    // release event, which would otherwise double every action.
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.handle_key(key);
                        dirty = true;
                    }
                    Event::Mouse(mouse) => dirty |= self.handle_mouse(mouse),
                    // Resize re-runs layout against the new frame size, which
                    // is the next draw's job — but that draw has to happen.
                    Event::Resize(_, _) => dirty = true,
                    _ => {}
                }
            }

            dirty |= self.tick_panels();
        }

        for slot in &mut self.slots {
            slot.panel.shutdown();
        }
        Ok(())
    }

    /// Tick any panel whose refresh interval has elapsed.
    ///
    /// Returns whether any panel ticked, and so whether the screen may now be
    /// out of date.
    fn tick_panels(&mut self) -> bool {
        let now = Instant::now();
        let mut ticked = false;
        for slot in &mut self.slots {
            if now.duration_since(slot.last_tick) >= slot.panel.refresh_interval() {
                slot.panel.tick();
                slot.last_tick = now;
                ticked = true;
            }
        }
        ticked
    }

    /// True when the focused panel is in a text-entry or modal state and global
    /// bindings must not fire.
    fn focus_captures_input(&self) -> bool {
        self.slots
            .get(self.focus)
            .is_some_and(|slot| slot.panel.captures_input())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Ctrl+C always quits, even mid-form, because a terminal user expects
        // it to and there is no state we would lose: panels save as they go.
        if ctrl && matches!(key.code, KeyCode::Char('c')) {
            self.should_quit = true;
            return;
        }

        // The help overlay swallows the next key, whatever it is.
        if self.show_help {
            self.show_help = false;
            return;
        }

        // Resizing is a shell-level concern, the way it is in tmux, so it is
        // claimed before panels get a look. It has to be: the calendar binds
        // the bare arrow keys and does not inspect modifiers, so offering the
        // key onward first would have Ctrl+Left scroll the month instead.
        //
        // A panel in a text-entry state still vetoes it, under the same rule
        // that stops `q` quitting mid-form.
        if ctrl && !self.focus_captures_input() {
            let resized = match key.code {
                KeyCode::Right => self.resize_width(true),
                KeyCode::Left => self.resize_width(false),
                KeyCode::Down => self.resize_height(true),
                KeyCode::Up => self.resize_height(false),
                _ => return self.dispatch_key(key),
            };
            // Swallowed either way: a resize that hit the minimum is still a
            // resize key, and must not fall through to a panel binding.
            let _ = resized;
            return;
        }

        self.dispatch_key(key);
    }

    /// Offer a key to the focused panel, then to the global bindings.
    fn dispatch_key(&mut self, key: KeyEvent) {
        // Offer the key to the focused panel first.
        if let Some(slot) = self.slots.get_mut(self.focus) {
            if slot.panel.handle_key(key) == KeyOutcome::Consumed {
                return;
            }
        }

        // A panel in a modal state gets an absolute veto on global bindings, so
        // typing "q" into a task title cannot quit the dashboard.
        if self.focus_captures_input() {
            return;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Tab => self.cycle_focus(true),
            KeyCode::BackTab => self.cycle_focus(false),
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char(c @ '1'..='9') => {
                let index = c as usize - '1' as usize;
                if index < self.slots.len() {
                    self.focus = index;
                }
            }
            _ => {}
        }
    }

    /// Move `step` weight from `donor` to `taker` within a set of weights,
    /// leaving the total untouched.
    ///
    /// Keeping the total fixed is what makes this feel like tmux: widening one
    /// panel narrows its neighbour and nothing else on screen moves. Scaling a
    /// single weight instead would silently reflow every other panel in the row.
    fn transfer(weights: &mut [u16], taker: usize, donor: usize) -> bool {
        let total: u32 = weights.iter().map(|w| u32::from(*w)).sum();
        // Step with the scale of the config rather than a fixed number of
        // units: weights are relative, so `width = 60` and `width = 3` are both
        // legitimate ways to write the same layout.
        let step = u16::try_from(total / 50).unwrap_or(1).max(1);

        let (Some(&grows), Some(&shrinks)) = (weights.get(taker), weights.get(donor)) else {
            return false;
        };
        // Never squeeze a panel out of existence — a panel that vanished could
        // not be focused, and so could not be given its space back.
        let step = step.min(shrinks.saturating_sub(MIN_WEIGHT));
        if step == 0 {
            return false;
        }
        weights[taker] = grows.saturating_add(step);
        weights[donor] = shrinks - step;
        true
    }

    /// Widen or narrow the focused panel against its neighbour in the row.
    fn resize_width(&mut self, grow: bool) -> bool {
        let Some(&(row, column)) = self.positions.get(self.focus) else {
            return false;
        };
        let Some(entry) = self.config.layout.rows.get_mut(row) else {
            return false;
        };
        // Borrow from the panel to the right, or from the left when the focused
        // panel is last in its row.
        let Some(neighbour) = neighbour_of(column, entry.panels.len()) else {
            return false;
        };

        let mut weights: Vec<u16> = entry.panels.iter().map(|p| p.width).collect();
        let (taker, donor) = if grow {
            (column, neighbour)
        } else {
            (neighbour, column)
        };
        if !Self::transfer(&mut weights, taker, donor) {
            return false;
        }
        for (panel, weight) in entry.panels.iter_mut().zip(weights) {
            panel.width = weight;
        }
        true
    }

    /// Grow or shrink the focused panel's row against the neighbouring row.
    fn resize_height(&mut self, grow: bool) -> bool {
        let Some(&(row, _)) = self.positions.get(self.focus) else {
            return false;
        };
        let rows = &mut self.config.layout.rows;
        let Some(neighbour) = neighbour_of(row, rows.len()) else {
            return false;
        };

        let mut weights: Vec<u16> = rows.iter().map(|r| r.height).collect();
        let (taker, donor) = if grow {
            (row, neighbour)
        } else {
            (neighbour, row)
        };
        if !Self::transfer(&mut weights, taker, donor) {
            return false;
        }
        for (entry, weight) in rows.iter_mut().zip(weights) {
            entry.height = weight;
        }
        true
    }

    /// The panel whose interior contains this point, if any.
    fn panel_at(&self, column: u16, row: u16) -> Option<usize> {
        self.slots.iter().position(|slot| {
            slot.area
                .is_some_and(|area| area.contains(Position::new(column, row)))
        })
    }

    /// Route a mouse event, returning whether the screen needs redrawing.
    ///
    /// Click focuses the panel under the pointer and is then offered to it;
    /// scroll is offered to the panel under the pointer *without* moving focus,
    /// so running the wheel over a list does not yank the keyboard away from
    /// whatever the user was working in.
    fn handle_mouse(&mut self, event: MouseEvent) -> bool {
        let interesting = matches!(
            event.kind,
            MouseEventKind::Down(_) | MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        );
        if !interesting {
            // Motion and button-release arrive constantly and mean nothing
            // here. Returning false is what keeps the redraw loop quiet.
            return false;
        }

        // The help overlay swallows the next input, whatever it is — the same
        // rule keys follow.
        if self.show_help {
            self.show_help = false;
            return true;
        }

        // A panel in a text-entry or modal state gets the same absolute veto
        // over the mouse that it has over global keys: a stray click must not
        // pull focus out of a half-typed task and strand the form.
        if self.focus_captures_input() {
            let focus = self.focus;
            let Some(area) = self.slots.get(focus).and_then(|slot| slot.area) else {
                return false;
            };
            if !area.contains(Position::new(event.column, event.row)) {
                return false;
            }
            return self
                .slots
                .get_mut(focus)
                .is_some_and(|slot| slot.panel.handle_mouse(event, area) == KeyOutcome::Consumed);
        }

        let Some(index) = self.panel_at(event.column, event.row) else {
            return false;
        };

        let focus_moved = if matches!(event.kind, MouseEventKind::Down(_)) {
            let moved = self.focus != index;
            self.focus = index;
            moved
        } else {
            false
        };

        let Some(slot) = self.slots.get_mut(index) else {
            return focus_moved;
        };
        let Some(area) = slot.area else {
            return focus_moved;
        };
        let consumed = slot.panel.handle_mouse(event, area) == KeyOutcome::Consumed;
        consumed || focus_moved
    }

    /// Move focus one panel forward or backward, wrapping at both ends.
    fn cycle_focus(&mut self, forward: bool) {
        let len = self.slots.len();
        if len == 0 {
            return;
        }
        self.focus = if forward {
            (self.focus + 1) % len
        } else {
            (self.focus + len - 1) % len
        };
    }

    /// Compute one rectangle per panel, in the same row-major order as
    /// `self.slots`.
    fn geometry(&self, area: Rect) -> Vec<Rect> {
        let row_constraints: Vec<Constraint> = self
            .config
            .layout
            .rows
            .iter()
            .map(|row| Constraint::Fill(row.height.max(1)))
            .collect();

        let row_areas = Layout::vertical(row_constraints).split(area);

        let mut rects = Vec::with_capacity(self.slots.len());
        for (row, row_area) in self.config.layout.rows.iter().zip(row_areas.iter()) {
            let column_constraints: Vec<Constraint> = row
                .panels
                .iter()
                .map(|panel| Constraint::Fill(panel.width.max(1)))
                .collect();
            let column_areas = Layout::horizontal(column_constraints).split(*row_area);
            rects.extend(column_areas.iter().copied());
        }
        rects
    }

    /// Test-only access to the private render pass.
    #[cfg(test)]
    pub fn render_for_test(&mut self, frame: &mut ratatui::Frame) {
        self.render(frame);
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();

        let body = if self.config.general.show_status_bar && area.height > 1 {
            let parts = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
            self.render_status_bar(frame, parts[1]);
            parts[0]
        } else {
            area
        };

        let rects = self.geometry(body);
        let theme = self.config.theme.clone();
        let focus = self.focus;

        for (index, slot) in self.slots.iter_mut().enumerate() {
            // Cleared first so a panel that fails to draw this pass cannot keep
            // catching clicks at the place it used to be.
            slot.area = None;

            let Some(rect) = rects.get(index).copied() else {
                continue;
            };
            if rect.width == 0 || rect.height == 0 {
                continue;
            }

            let focused = index == focus;
            let title = slot.panel.title();
            let spec = FrameSpec {
                title: &title,
                counter: slot.panel.counter(),
                focused,
                bindings: slot.panel.bindings(),
                index: index + 1,
            };
            let inner = crate::frame::draw(frame, rect, &theme, &spec);

            if inner.width == 0 || inner.height == 0 {
                continue;
            }
            slot.area = Some(inner);

            slot.panel.render(
                frame,
                inner,
                RenderContext {
                    theme: &theme,
                    gradients: &self.gradients,
                    focused,
                },
            );
        }

        if self.show_help {
            self.render_help(frame, area);
        }
    }

    /// The status bar carries *global* bindings only.
    ///
    /// Panel bindings live in the focused panel's own border, which keeps the
    /// two scopes visually separate. A flat list of both teaches users to press
    /// panel keys while the wrong panel is focused.
    fn render_status_bar(&self, frame: &mut ratatui::Frame, area: Rect) {
        let theme = &self.config.theme;
        let key_style = Style::default().fg(theme.key).add_modifier(Modifier::BOLD);
        let muted = Style::default().fg(theme.muted);

        let mut spans = vec![Span::styled(
            " mirador",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )];

        for binding in GLOBAL.iter().filter(|b| b.primary) {
            spans.push(Span::styled("   ", muted));
            spans.push(Span::styled(binding.key, key_style));
            spans.push(Span::styled(format!(" {}", binding.action), muted));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_help(&self, frame: &mut ratatui::Frame, area: Rect) {
        let theme = &self.config.theme;
        let key_style = Style::default().fg(theme.key).add_modifier(Modifier::BOLD);
        let muted = Style::default().fg(theme.muted);

        let section = |title: &str| {
            Line::from(Span::styled(
                crate::glyphs::utility(title),
                Style::default()
                    .fg(theme.label)
                    .add_modifier(Modifier::BOLD),
            ))
        };
        let entry = |binding: &Binding| {
            Line::from(vec![
                Span::styled(format!("  {:<12}", binding.key), key_style),
                Span::styled(binding.action.to_string(), muted),
            ])
        };

        let mut lines = vec![section("global")];
        lines.extend(GLOBAL.iter().map(entry));

        // Bindings are grouped by the panel they belong to, so it is always
        // clear which panel a key acts on.
        if let Some(slot) = self.slots.get(self.focus) {
            let panel_keys = slot.panel.bindings();
            if !panel_keys.is_empty() {
                lines.push(Line::from(""));
                lines.push(section(&slot.panel.title()));
                lines.extend(panel_keys.iter().map(entry));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  any key to close",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        )));

        let width = 46.min(area.width);
        let height = (u16::try_from(lines.len()).unwrap_or(u16::MAX) + 2).min(area.height);
        let popup = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        };

        frame.render_widget(Clear, popup);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_focused))
            .padding(Padding::horizontal(1))
            .title_top(Line::from(vec![
                Span::styled("┤", Style::default().fg(theme.border_focused)),
                Span::styled(
                    "Keys",
                    Style::default()
                        .fg(theme.title)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("├", Style::default().fg(theme.border_focused)),
            ]));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Layout as LayoutConfig, LayoutPanel, LayoutRow};

    /// A config whose layout is only panels that need no I/O.
    fn config_with(widgets: &[&str]) -> Config {
        Config {
            layout: LayoutConfig {
                rows: vec![LayoutRow {
                    height: 1,
                    panels: widgets
                        .iter()
                        .map(|w| LayoutPanel {
                            widget: (*w).to_string(),
                            width: 1,
                        })
                        .collect(),
                }],
            },
            ..Config::default()
        }
    }

    /// A two-row layout with two panels in the first row, all on a 100 scale.
    fn resizable() -> Config {
        Config {
            layout: LayoutConfig {
                rows: vec![
                    LayoutRow {
                        height: 50,
                        panels: vec![
                            LayoutPanel {
                                widget: "clocks".into(),
                                width: 50,
                            },
                            LayoutPanel {
                                widget: "calendar".into(),
                                width: 50,
                            },
                        ],
                    },
                    LayoutRow {
                        height: 50,
                        panels: vec![LayoutPanel {
                            widget: "cpu".into(),
                            width: 100,
                        }],
                    },
                ],
            },
            ..Config::default()
        }
    }

    fn widths(app: &App) -> Vec<u16> {
        app.config.layout.rows[0]
            .panels
            .iter()
            .map(|p| p.width)
            .collect()
    }

    fn heights(app: &App) -> Vec<u16> {
        app.config.layout.rows.iter().map(|r| r.height).collect()
    }

    #[test]
    fn widening_a_panel_narrows_its_neighbour_and_holds_the_total() {
        let mut app = App::new(resizable()).unwrap();
        let before: u16 = widths(&app).iter().sum();

        assert!(app.resize_width(true));
        let after = widths(&app);
        assert!(after[0] > 50, "focused panel must grow: {after:?}");
        assert!(after[1] < 50, "its neighbour must give the space up");
        assert_eq!(
            after.iter().sum::<u16>(),
            before,
            "the row total must not drift, or every panel reflows"
        );
    }

    #[test]
    fn narrowing_is_the_exact_inverse_of_widening() {
        let mut app = App::new(resizable()).unwrap();
        let before = widths(&app);
        assert!(app.resize_width(true));
        assert!(app.resize_width(false));
        assert_eq!(widths(&app), before);
    }

    #[test]
    fn the_last_panel_in_a_row_borrows_from_the_one_before_it() {
        let mut app = App::new(resizable()).unwrap();
        app.focus = 1;
        assert!(app.resize_width(true));
        let after = widths(&app);
        assert!(after[1] > 50, "the last panel must still be able to grow");
        assert!(after[0] < 50);
    }

    #[test]
    fn a_panel_alone_in_its_row_cannot_be_resized_horizontally() {
        let mut app = App::new(resizable()).unwrap();
        // The cpu panel is the only one in row 1; there is nobody to take
        // space from, and stretching it alone would mean nothing.
        app.focus = 2;
        assert!(!app.resize_width(true));
        assert!(!app.resize_width(false));
    }

    #[test]
    fn resizing_height_trades_between_rows() {
        let mut app = App::new(resizable()).unwrap();
        let before: u16 = heights(&app).iter().sum();
        assert!(app.resize_height(true));
        let after = heights(&app);
        assert!(after[0] > 50 && after[1] < 50, "{after:?}");
        assert_eq!(after.iter().sum::<u16>(), before);
    }

    #[test]
    fn a_panel_can_never_be_squeezed_out_of_existence() {
        let mut app = App::new(resizable()).unwrap();
        // Far more presses than it takes to consume the neighbour entirely.
        for _ in 0..500 {
            app.resize_width(true);
        }
        let after = widths(&app);
        assert!(
            after[1] >= MIN_WEIGHT,
            "a panel squeezed to nothing can never be focused to get its space back: {after:?}"
        );
        assert_eq!(after.iter().sum::<u16>(), 100, "total still holds");
    }

    #[test]
    fn resize_keys_are_claimed_before_the_focused_panel_sees_them() {
        // The calendar binds bare Left/Right and ignores modifiers, so if the
        // shell offered the key onward first, Ctrl+Left would scroll the month
        // instead of resizing. Focus it and check the width actually moved.
        let mut app = App::new(resizable()).unwrap();
        app.focus = 1;
        let before = widths(&app);
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_ne!(widths(&app), before, "Ctrl+Left was swallowed by the panel");
    }

    #[test]
    fn focus_wraps_in_both_directions() {
        let mut app = App::new(config_with(&["clocks", "cpu", "network"])).unwrap();
        assert_eq!(app.focus, 0);

        app.cycle_focus(true);
        assert_eq!(app.focus, 1);
        app.cycle_focus(true);
        app.cycle_focus(true);
        assert_eq!(app.focus, 0, "forward focus must wrap");

        app.cycle_focus(false);
        assert_eq!(app.focus, 2, "backward focus must wrap");
    }

    #[test]
    fn geometry_returns_one_rect_per_panel_and_fills_the_area() {
        let config = Config {
            layout: LayoutConfig {
                rows: vec![
                    LayoutRow {
                        height: 50,
                        panels: vec![
                            LayoutPanel {
                                widget: "clocks".into(),
                                width: 50,
                            },
                            LayoutPanel {
                                widget: "cpu".into(),
                                width: 50,
                            },
                        ],
                    },
                    LayoutRow {
                        height: 50,
                        panels: vec![LayoutPanel {
                            widget: "network".into(),
                            width: 100,
                        }],
                    },
                ],
            },
            ..Config::default()
        };
        let app = App::new(config).unwrap();
        let area = Rect::new(0, 0, 80, 24);
        let rects = app.geometry(area);

        assert_eq!(rects.len(), 3);
        assert_eq!(rects[2].width, 80);
        assert_eq!(rects[0].width + rects[1].width, 80);
        assert_eq!(rects[0].x, 0);
        assert_eq!(rects[1].x, rects[0].width);
        for rect in &rects {
            assert!(rect.x + rect.width <= area.x + area.width);
            assert!(rect.y + rect.height <= area.y + area.height);
        }
    }

    #[test]
    fn geometry_survives_a_terminal_too_small_to_draw() {
        let app = App::new(config_with(&["clocks", "cpu"])).unwrap();
        for (w, h) in [(0, 0), (1, 1), (3, 2)] {
            let rects = app.geometry(Rect::new(0, 0, w, h));
            assert_eq!(rects.len(), 2, "must still return one rect per panel");
        }
    }

    #[test]
    fn zero_weights_are_treated_as_one_rather_than_dividing_by_zero() {
        let config = Config {
            layout: LayoutConfig {
                rows: vec![LayoutRow {
                    height: 0,
                    panels: vec![LayoutPanel {
                        widget: "cpu".into(),
                        width: 0,
                    }],
                }],
            },
            ..Config::default()
        };
        let app = App::new(config).unwrap();
        let rects = app.geometry(Rect::new(0, 0, 80, 24));
        assert_eq!(rects.len(), 1);
        assert!(rects[0].width > 0);
    }

    #[test]
    fn quit_keys_set_the_quit_flag() {
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        assert!(!app.should_quit);
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.should_quit);

        let mut app = App::new(config_with(&["clocks"])).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn number_keys_jump_to_a_panel_and_ignore_out_of_range_indices() {
        let mut app = App::new(config_with(&["clocks", "cpu"])).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(app.focus, 1);
        app.handle_key(KeyEvent::new(KeyCode::Char('9'), KeyModifiers::NONE));
        assert_eq!(app.focus, 1, "an out-of-range index must not move focus");
    }

    #[test]
    fn help_opens_and_the_next_key_closes_it() {
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(app.show_help);
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(!app.show_help);
        assert!(!app.should_quit, "closing help must not also quit");
    }

    #[test]
    fn an_empty_layout_is_rejected_at_construction() {
        let config = Config {
            layout: LayoutConfig { rows: Vec::new() },
            ..Config::default()
        };
        assert!(App::new(config).is_err());
    }

    #[test]
    fn every_global_binding_has_a_key_and_an_action() {
        for binding in GLOBAL {
            assert!(!binding.key.is_empty());
            assert!(!binding.action.is_empty());
        }
        assert!(
            GLOBAL.iter().any(|b| b.key == "?" && b.primary),
            "help must always be advertised"
        );
    }
}
