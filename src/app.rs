//! The application shell: layout, focus, the event loop and global bindings.
//!
//! The shell knows nothing about what any panel displays. It owns the grid, the
//! focus ring, the frames and the tick schedule, and forwards everything else
//! through the [`Panel`] trait.

use std::path::PathBuf;
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
use crate::state::UiState;
use crate::theme::Gradients;

/// Global bindings, used for both the status bar and the help overlay.
const GLOBAL: &[Binding] = &[
    Binding::primary("Tab", "focus"),
    Binding::primary("?", "keys"),
    Binding::primary("q", "quit"),
    // After `quit` deliberately. The status bar shows as many primary bindings
    // as fit, in order, and on a narrow terminal knowing how to get out beats
    // knowing how to add a panel. The unused-widget notice names this key
    // anyway, which is where someone actually needs to be told about it.
    Binding::primary("w", "panels"),
    Binding::extra("Shift+Tab", "focus back"),
    Binding::extra("1-9", "jump to panel"),
    Binding::extra("Ctrl+←/→", "resize width"),
    Binding::extra("Ctrl+↑/↓", "resize height"),
    Binding::extra("Ctrl+C", "quit"),
];

/// Smallest weight a panel or row may be squeezed to.
const MIN_WEIGHT: u16 = 1;

/// Split `total` cells across `weights`, giving no slot more than its maximum
/// and handing what it declines to the slots that can still use it.
///
/// A clock cannot use a hundred columns; a calendar cannot use more than its
/// months need. Pure proportional layout gives them the space anyway and they
/// sit in it, while the task list next door runs out of room. So a panel may
/// declare the point past which more space does nothing for it, and the surplus
/// moves sideways to a neighbour that will actually fill it.
///
/// When *every* slot is bounded there is nobody to hand the surplus to, and the
/// maxima are ignored rather than leaving a gap: panels draw their own frames,
/// so unallocated cells would show as a hole in the middle of the dashboard.
/// Better a slightly over-wide panel than a visible seam.
fn distribute(total: u16, weights: &[u16], maxima: &[Option<u16>]) -> Vec<u16> {
    let count = weights.len();
    if count == 0 || total == 0 {
        return vec![0; count];
    }

    let mut sizes = proportional(total, weights);

    // Each pass caps whoever is over and re-splits what they gave up. Bounded
    // by the slot count: every pass either caps at least one more slot or ends.
    for _ in 0..count {
        let capped = |i: usize, sizes: &[u16]| maxima[i].is_some_and(|m| sizes[i] >= m);

        let surplus: u16 = (0..count)
            .filter_map(|i| maxima[i].and_then(|m| sizes[i].checked_sub(m)))
            .sum();
        if surplus == 0 {
            break;
        }

        let takers: Vec<usize> = (0..count).filter(|&i| !capped(i, &sizes)).collect();
        if takers.is_empty() {
            // Nobody can absorb it. Leave the proportional split alone so the
            // row still covers its full width.
            break;
        }

        for i in 0..count {
            if let Some(max) = maxima[i] {
                sizes[i] = sizes[i].min(max);
            }
        }

        let taker_weights: Vec<u16> = takers.iter().map(|&i| weights[i].max(1)).collect();
        for (slot, extra) in takers.iter().zip(proportional(surplus, &taker_weights)) {
            sizes[*slot] = sizes[*slot].saturating_add(extra);
        }
    }

    sizes
}

/// Split `total` in proportion to `weights`, losing no cells to rounding.
///
/// Largest-remainder rather than plain division: dividing and truncating leaves
/// up to one cell per slot unallocated, which shows up as a ragged right edge.
fn proportional(total: u16, weights: &[u16]) -> Vec<u16> {
    let count = weights.len();
    if count == 0 {
        return Vec::new();
    }
    let sum: u32 = weights.iter().map(|w| u32::from((*w).max(1))).sum();
    if sum == 0 {
        return vec![0; count];
    }

    let mut sizes = Vec::with_capacity(count);
    let mut remainders: Vec<(u32, usize)> = Vec::with_capacity(count);
    let mut used: u32 = 0;

    for (index, weight) in weights.iter().enumerate() {
        let exact = u32::from(total) * u32::from((*weight).max(1));
        let whole = exact / sum;
        remainders.push((exact % sum, index));
        used += whole;
        sizes.push(u16::try_from(whole).unwrap_or(u16::MAX));
    }

    // Hand the leftover cells to the largest remainders, ties by position so
    // the result is stable frame to frame.
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut leftover = u32::from(total).saturating_sub(used);
    for (_, index) in remainders {
        if leftover == 0 {
            break;
        }
        sizes[index] = sizes[index].saturating_add(1);
        leftover -= 1;
    }

    sizes
}

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

/// The panels of a layout, with the `(row, column)` each came from.
type Built = (Vec<Slot>, Vec<(usize, usize)>);

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
///
/// The flags are genuinely independent — an overlay being open says nothing
/// about whether the layout needs writing — so grouping them into a struct to
/// satisfy the lint would add a name without adding a meaning.
#[allow(clippy::struct_excessive_bools)]
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
    /// Widgets available but not placed by this layout.
    ///
    /// A config written by an earlier version silently lacks every widget added
    /// since — an absent widget is a valid choice, so nothing errors and
    /// `--migrate-config` has nothing to fix. This is the only way to find out.
    unused_widgets: Vec<&'static str>,
    /// Whether the startup hint is still on screen. Cleared by the first input
    /// of any kind: a dashboard you leave open all day must not nag, and a
    /// notice that will not go away is a nag.
    show_widget_hint: bool,
    /// Open panel picker, and which row it is on.
    picker: Option<usize>,
    /// The config file, so layout changes can be written back to it. `None` in
    /// tests, which is what keeps them off a real user's config.
    config_path: Option<PathBuf>,
    /// Whether the layout has been changed since it was last written.
    layout_dirty: bool,
    /// Why the last layout write failed, if it did. Shown in the picker: a
    /// change you made that silently did not persist is the worst outcome here.
    layout_error: Option<String>,
    /// Where to write remembered preferences, once someone asks for that.
    /// `None` in tests, which is what keeps them off a real user's file.
    state_path: Option<PathBuf>,
    /// The last state written, so a keypress that changed nothing writes
    /// nothing.
    saved_state: UiState,
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
        let (slots, positions) = Self::build_slots(&config)?;

        let gradients = config.theme.gradients();
        let unused_widgets = crate::widgets::unused_widgets(&config);
        Ok(Self {
            config,
            gradients,
            slots,
            positions,
            focus: 0,
            show_help: false,
            should_quit: false,
            show_widget_hint: !unused_widgets.is_empty(),
            unused_widgets,
            picker: None,
            config_path: None,
            layout_dirty: false,
            layout_error: None,
            state_path: None,
            saved_state: UiState::default(),
        })
    }

    /// Build one panel per entry in the layout, in row-major order.
    fn build_slots(config: &Config) -> Result<Built> {
        // Start each panel far enough in the past that its first tick fires
        // immediately rather than one interval from now.
        let epoch = Instant::now()
            .checked_sub(Duration::from_hours(24))
            .unwrap();

        let mut slots = Vec::new();
        let mut positions = Vec::new();
        for (row_index, row) in config.layout.rows.iter().enumerate() {
            for (column_index, entry) in row.panels.iter().enumerate() {
                let panel = crate::widgets::build(&entry.widget, config)
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
        Ok((slots, positions))
    }

    /// Rebuild every panel after the layout changed.
    ///
    /// Panels are thrown away and remade rather than moved, which costs a fresh
    /// weather fetch and an empty CPU history for the panels that survived. The
    /// alternative is matching old panels to new positions by widget name and
    /// carrying them across, which is a good deal more machinery for something
    /// that happens when a person deliberately opens a dialog and toggles
    /// something — a moment where a beat of re-fetching reads as the dashboard
    /// responding rather than as a stall.
    ///
    /// A failure leaves the previous panels in place: a layout that will not
    /// build is a reason to refuse the change, not to end up with nothing.
    fn rebuild_panels(&mut self) -> Result<()> {
        let (slots, positions) = Self::build_slots(&self.config)?;
        self.slots = slots;
        self.positions = positions;
        self.focus = self.focus.min(self.slots.len().saturating_sub(1));
        self.unused_widgets = crate::widgets::unused_widgets(&self.config);
        Ok(())
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
                        // Only keys can move a preference, and only after one
                        // has been handled can it have moved. Cheap because it
                        // compares before writing: an arrow key costs a struct
                        // comparison, not a file.
                        self.persist_preferences();
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
        // Once more on the way out, in case the last thing changed was not a
        // key — and because Ctrl+C reaches here too.
        self.persist_preferences();
        // Resizes batch to here rather than writing per keystroke: Ctrl+arrow
        // repeats, and a config rewritten on every repeat would be absurd.
        self.write_layout();
        Ok(())
    }

    /// Remember preferences to `path` from now on.
    ///
    /// Separate from [`App::new`] so that tests, which build apps constantly,
    /// cannot write to a real user's state file by forgetting to opt out. An
    /// app with no path set simply never persists.
    ///
    /// `loaded` is what was read from that file, and becomes the base every
    /// later write merges into — so starting up does not rewrite a file it has
    /// just read, and a preference from an earlier session is not dropped by a
    /// panel that has nothing new to say about it.
    pub fn remember_preferences_at(&mut self, path: PathBuf, loaded: UiState) {
        self.saved_state = loaded;
        self.state_path = Some(path);
    }

    /// What every panel currently wants remembered.
    ///
    /// Starts from what is already on disk rather than from nothing, so a
    /// preference set last session and left alone this one survives instead of
    /// being dropped by the panel that is no longer calling it a change.
    fn collect_preferences(&self) -> UiState {
        let mut state = self.saved_state.clone();
        for slot in &self.slots {
            slot.panel.remember(&mut state);
        }
        state
    }

    /// Write preferences if any of them moved.
    ///
    /// A failed write is deliberately not surfaced. There is no panel that owns
    /// this to show an error in, and the failure costs a sort order that one
    /// keystroke restores — putting a warning on a dashboard designed to be
    /// left open, over that, would be the wrong trade. Task and note saves,
    /// which can lose something you cannot retype, do surface theirs.
    fn persist_preferences(&mut self) {
        let Some(path) = self.state_path.clone() else {
            return;
        };
        let current = self.collect_preferences();
        if current == self.saved_state {
            return;
        }
        if current.save(&path).is_ok() {
            self.saved_state = current;
        }
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
        // Any key at all retires the startup hint. It has been read or it has
        // been ignored; either way it has had its turn.
        self.show_widget_hint = false;

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

        // The picker is a real dialog rather than a notice, so it reads keys
        // instead of dismissing on any of them.
        if self.picker.is_some() {
            self.handle_picker_key(key);
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
            // Only a resize that actually moved something needs writing. Held
            // against a minimum, the key repeats without changing anything, and
            // marking those dirty would write the config on shutdown after a
            // session that changed nothing.
            self.layout_dirty |= resized;
            // Swallowed either way: a resize that hit the minimum is still a
            // resize key, and must not fall through to a panel binding.
            return;
        }

        self.dispatch_key(key);
    }

    /// Drive the panel picker.
    fn handle_picker_key(&mut self, key: KeyEvent) {
        let names = crate::widgets::WIDGET_NAMES;
        let Some(selected) = self.picker else { return };
        let last = names.len().saturating_sub(1);

        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'w') | KeyCode::Enter => {
                self.picker = None;
                // Written on close rather than on every toggle: someone trying
                // three arrangements should cost one write, not three, and the
                // dialog is a natural commit point.
                self.write_layout();
            }
            KeyCode::Down | KeyCode::Char('j') => self.picker = Some((selected + 1).min(last)),
            KeyCode::Up | KeyCode::Char('k') => self.picker = Some(selected.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => self.picker = Some(0),
            KeyCode::End | KeyCode::Char('G') => self.picker = Some(last),
            KeyCode::Char(' ') => {
                if let Some(name) = names.get(selected) {
                    self.toggle_widget(name);
                }
            }
            _ => {}
        }
    }

    /// Turn a widget on or off, rebuilding the dashboard around it.
    fn toggle_widget(&mut self, widget: &str) {
        let before = self.config.layout.clone();

        if self.config.layout.places(widget) {
            if !self.config.layout.remove_widget(widget) {
                // The last panel. An empty layout is rejected at startup, so
                // allowing this would write a config that cannot be opened.
                self.layout_error = Some("at least one panel has to stay".into());
                return;
            }
        } else {
            self.config.layout.add_widget(widget);
        }

        if let Err(e) = self.rebuild_panels() {
            // Put it back. A layout that will not build is a reason to refuse
            // the toggle, not to leave the dashboard in pieces.
            self.config.layout = before;
            let _ = self.rebuild_panels();
            self.layout_error = Some(format!("{e:#}"));
            return;
        }

        self.layout_error = None;
        self.layout_dirty = true;
    }

    /// Write the layout back into the config file, if it changed.
    ///
    /// Textual, so comments and formatting survive; see [`crate::layout_edit`].
    /// A failure is kept and shown rather than swallowed — a panel you switched
    /// on that quietly fails to persist is worse than one that never appeared,
    /// because you will not find out until the next start.
    fn write_layout(&mut self) {
        if !self.layout_dirty {
            return;
        }
        let Some(path) = self.config_path.clone() else {
            return;
        };

        let result = std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|source| crate::layout_edit::apply(&source, &self.config.layout))
            .and_then(|updated| std::fs::write(&path, updated).map_err(anyhow::Error::from));

        match result {
            Ok(()) => {
                self.layout_dirty = false;
                self.layout_error = None;
            }
            Err(e) => self.layout_error = Some(format!("{e:#}")),
        }
    }

    /// Write layout changes to `path` from now on.
    ///
    /// Separate from [`App::new`] for the same reason the state path is: tests
    /// build apps constantly and must not be able to touch a real config.
    pub fn write_layout_to(&mut self, path: PathBuf) {
        self.config_path = Some(path);
    }

    /// Offer a key to the focused panel, then to the global bindings.
    fn dispatch_key(&mut self, key: KeyEvent) {
        // Offer the key to the focused panel first.
        if let Some(slot) = self.slots.get_mut(self.focus)
            && slot.panel.handle_key(key) == KeyOutcome::Consumed
        {
            return;
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
            KeyCode::Char('w') => self.picker = Some(0),
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

        // A deliberate click or scroll retires the startup hint, as a key does.
        // Pointer motion deliberately does not: the mouse crossing the window
        // on its way somewhere else is not the user reading anything.
        let had_hint = std::mem::take(&mut self.show_widget_hint);

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
                return had_hint;
            };
            if !area.contains(Position::new(event.column, event.row)) {
                return had_hint;
            }
            let consumed = self
                .slots
                .get_mut(focus)
                .is_some_and(|slot| slot.panel.handle_mouse(event, area) == KeyOutcome::Consumed);
            return consumed || had_hint;
        }

        let Some(index) = self.panel_at(event.column, event.row) else {
            return had_hint;
        };

        let focus_moved = if matches!(event.kind, MouseEventKind::Down(_)) {
            let moved = self.focus != index;
            self.focus = index;
            moved
        } else {
            false
        };

        let Some(slot) = self.slots.get_mut(index) else {
            return focus_moved || had_hint;
        };
        let Some(area) = slot.area else {
            return focus_moved || had_hint;
        };
        let consumed = slot.panel.handle_mouse(event, area) == KeyOutcome::Consumed;
        consumed || focus_moved || had_hint
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

    /// The slot index of the panel at `(row, column)` of the layout.
    fn slot_at(&self, row: usize, column: usize) -> Option<usize> {
        self.positions.iter().position(|p| *p == (row, column))
    }

    /// Compute one rectangle per panel, in the same row-major order as
    /// `self.slots`.
    ///
    /// Two passes of [`distribute`]: heights down the rows, then widths across
    /// each row. Both honour the panels' declared maxima, so a panel that
    /// cannot use more space passes it to one that can.
    fn geometry(&self, area: Rect) -> Vec<Rect> {
        let rows = &self.config.layout.rows;

        let row_weights: Vec<u16> = rows.iter().map(|row| row.height.max(1)).collect();
        // A row is only bounded when every panel in it is: they share the
        // height, so one unbounded panel keeps the whole row unbounded.
        let row_maxima: Vec<Option<u16>> = rows
            .iter()
            .enumerate()
            .map(|(row_index, row)| {
                let mut tallest = 0u16;
                for column in 0..row.panels.len() {
                    let max = self
                        .slot_at(row_index, column)
                        .and_then(|slot| self.slots[slot].panel.max_height())?;
                    tallest = tallest.max(max);
                }
                (tallest > 0).then_some(tallest)
            })
            .collect();

        let heights = distribute(area.height, &row_weights, &row_maxima);

        let mut rects = Vec::with_capacity(self.slots.len());
        let mut y = area.y;
        for (row_index, row) in rows.iter().enumerate() {
            let height = heights.get(row_index).copied().unwrap_or(0);

            let widths: Vec<u16> = row.panels.iter().map(|p| p.width.max(1)).collect();
            let maxima: Vec<Option<u16>> = (0..row.panels.len())
                .map(|column| {
                    self.slot_at(row_index, column)
                        .and_then(|slot| self.slots[slot].panel.max_width())
                })
                .collect();
            let columns = distribute(area.width, &widths, &maxima);

            let mut x = area.x;
            for width in columns {
                rects.push(Rect::new(x, y, width, height));
                x = x.saturating_add(width);
            }
            y = y.saturating_add(height);
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
        if let Some(selected) = self.picker {
            self.render_picker(frame, area, selected);
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

        // The hint rides on the right of the bar it shares with the global
        // keys, and gives way to them when the terminal is too narrow: knowing
        // how to quit matters more than knowing what you are not using.
        if let Some(hint) = self.widget_hint() {
            let used: usize = spans
                .iter()
                .map(|s| crate::grid::display_width(&s.content))
                .sum();
            let hint_width = crate::grid::display_width(&hint);
            let total = usize::from(area.width);
            // One space of breathing room on each side of the gap.
            if used + hint_width + 3 <= total {
                spans.push(Span::styled(
                    " ".repeat(total - used - hint_width - 1),
                    muted,
                ));
                spans.push(Span::styled(hint, Style::default().fg(theme.label)));
            }
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// The one-line startup notice about widgets this layout does not place.
    fn widget_hint(&self) -> Option<String> {
        if !self.show_widget_hint || self.unused_widgets.is_empty() {
            return None;
        }
        // Names the key rather than only the widgets. Saying what is missing
        // without saying what to do about it is how someone ends up reading the
        // help, not finding the answer there either, and going to look for a
        // config file.
        Some(format!(
            "{} unused: {}   press w ",
            self.unused_widgets.len(),
            self.unused_widgets.join(", ")
        ))
    }

    /// The panel picker: every widget mirador has, and whether it is on.
    fn render_picker(&self, frame: &mut ratatui::Frame, area: Rect, selected: usize) {
        let theme = &self.config.theme;
        let names = crate::widgets::WIDGET_NAMES;

        let mut lines: Vec<Line> = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let on = self.config.layout.places(name);
            let here = index == selected;
            // A filled mark and an empty one in the track colour, the same
            // vocabulary the meters use, so "on" is legible without relying on
            // the word beside it.
            let mark = if on { "■" } else { "□" };
            let mark_style = Style::default().fg(if on { theme.accent } else { theme.track });
            let name_style = if here {
                Style::default()
                    .fg(theme.text)
                    .add_modifier(Modifier::REVERSED)
            } else if on {
                Style::default().fg(theme.text)
            } else {
                Style::default().fg(theme.muted)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if here { " ▸ " } else { "   " },
                    Style::default().fg(theme.accent),
                ),
                Span::styled(format!("{mark} "), mark_style),
                Span::styled(format!("{name:<10}"), name_style),
            ]));
        }

        lines.push(Line::from(""));
        if let Some(error) = &self.layout_error {
            lines.push(Line::from(Span::styled(
                format!("  {error}"),
                Style::default().fg(theme.error),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "  written to your config on close",
                Style::default().fg(theme.muted),
            )));
        }
        lines.push(Line::from(vec![
            Span::styled(
                "  space",
                Style::default().fg(theme.key).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" toggle   ", Style::default().fg(theme.muted)),
            Span::styled(
                "esc",
                Style::default().fg(theme.key).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" close", Style::default().fg(theme.muted)),
        ]));

        let width = 40.min(area.width);
        let height = (u16::try_from(lines.len()).unwrap_or(u16::MAX) + 2).min(area.height);
        let popup = Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        };

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.border_focused))
                    .title(Span::styled(
                        crate::glyphs::utility(" panels "),
                        Style::default()
                            .fg(theme.title)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .padding(Padding::horizontal(1)),
            ),
            popup,
        );
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

        // The durable half of the unused-widget hint. The status bar notice is
        // gone after one keypress; this stays, because `?` is where someone
        // goes when they wonder what else the thing does.
        if !self.unused_widgets.is_empty() {
            lines.push(Line::from(""));
            lines.push(section("widgets not in your layout"));
            // The overlay clips silently when the content outgrows the screen,
            // so the actionable line comes before the list: losing the names
            // still leaves you able to act, losing the instruction does not.
            // The names go on one wrapped line for the same reason — one line
            // per widget cost eight rows on a stale config.
            lines.push(Line::from(vec![
                Span::styled("  press ", muted),
                Span::styled("w", key_style),
                Span::styled(" to switch them on", muted),
            ]));
            lines.push(Line::from(Span::styled(
                format!("  {}", self.unused_widgets.join(", ")),
                Style::default().fg(theme.text),
            )));
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
    fn a_layout_missing_widgets_says_so_once_and_then_stops() {
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        assert!(
            app.unused_widgets.contains(&"stocks"),
            "a widget the layout never places must be reported: {:?}",
            app.unused_widgets
        );
        let hint = app.widget_hint().expect("the hint shows at startup");
        assert!(hint.contains("stocks"), "got `{hint}`");

        app.handle_key(KeyEvent::from(KeyCode::Char('?')));
        assert!(
            app.widget_hint().is_none(),
            "a dashboard left open all day must not keep nagging"
        );
    }

    #[test]
    fn a_layout_using_everything_gets_no_hint_at_all() {
        let config = Config {
            layout: LayoutConfig {
                rows: crate::widgets::WIDGET_NAMES
                    .iter()
                    .map(|name| LayoutRow {
                        height: 1,
                        panels: vec![LayoutPanel {
                            widget: (*name).to_string(),
                            width: 1,
                        }],
                    })
                    .collect(),
            },
            ..Config::default()
        };
        let unused = crate::widgets::unused_widgets(&config);
        assert!(unused.is_empty(), "nothing to suggest: {unused:?}");
    }

    #[test]
    fn a_click_retires_the_hint_but_the_pointer_merely_passing_over_does_not() {
        use ratatui::crossterm::event::MouseButton;

        let mouse = |kind| MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };

        let mut app = App::new(config_with(&["clocks"])).unwrap();
        app.handle_mouse(mouse(MouseEventKind::Moved));
        assert!(
            app.widget_hint().is_some(),
            "the mouse crossing the window is not the user reading anything"
        );

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left)));
        assert!(app.widget_hint().is_none(), "a deliberate click retires it");
    }

    #[test]
    fn retiring_the_hint_forces_a_redraw_so_it_actually_disappears() {
        use ratatui::crossterm::event::MouseButton;

        // A click landing on no panel at all still has to repaint, or the
        // retired hint stays on screen until something else happens to redraw.
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        let dirty = app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 9_000,
            row: 9_000,
            modifiers: KeyModifiers::NONE,
        });
        assert!(dirty, "the hint was cleared, so the screen is out of date");
    }

    #[test]
    fn the_help_overlay_renders_at_any_size_with_widgets_to_report() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        // The unused-widget section grows the overlay, and the overlay clips
        // silently rather than erroring — so a size that cannot fit it must
        // still draw something rather than panic on the arithmetic.
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        app.handle_key(KeyEvent::from(KeyCode::Char('?')));
        assert!(app.show_help);

        for (width, height) in [(1, 1), (4, 3), (30, 8), (80, 24), (200, 60)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| app.render_for_test(frame))
                .unwrap_or_else(|e| panic!("help failed to draw at {width}x{height}: {e}"));
        }
    }

    #[test]
    fn the_unused_widget_section_stays_a_fixed_size_however_many_are_unused() {
        // One line per widget put eight rows into an overlay that clips
        // silently; the names share a single wrapped line instead.
        let one = App::new(config_with(&["clocks"])).unwrap();
        let hint = one.widget_hint().expect("something is unused");
        assert!(
            hint.lines().count() == 1,
            "the status hint must stay one line: `{hint}`"
        );
    }

    #[test]
    fn the_hint_gives_way_to_the_global_keys_on_a_narrow_terminal() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = App::new(config_with(&["clocks"])).unwrap();
        let row_text = |app: &mut App, width: u16| -> String {
            let mut terminal = Terminal::new(TestBackend::new(width, 6)).unwrap();
            terminal.draw(|frame| app.render_for_test(frame)).unwrap();
            let buf = terminal.backend().buffer().clone();
            (0..width).map(|x| buf[(x, 5)].symbol()).collect()
        };

        let wide = row_text(&mut app, 160);
        assert!(wide.contains("unused"), "wide enough for both: `{wide}`");

        let narrow = row_text(&mut app, 44);
        assert!(
            !narrow.contains("unused"),
            "knowing how to quit outranks the hint: `{narrow}`"
        );
        assert!(
            narrow.contains("quit"),
            "the global keys survive: `{narrow}`"
        );
    }

    #[test]
    fn space_is_split_by_weight_when_nobody_declares_a_limit() {
        assert_eq!(distribute(100, &[50, 50], &[None, None]), vec![50, 50]);
        assert_eq!(distribute(100, &[25, 75], &[None, None]), vec![25, 75]);
    }

    #[test]
    fn every_cell_is_allocated_however_the_weights_divide() {
        // Truncating division would leave a ragged edge where the frames stop
        // short of the terminal.
        for total in [1u16, 7, 23, 80, 81, 199, 200] {
            for weights in [vec![1, 1, 1], vec![34, 33, 33], vec![1, 2, 7]] {
                let maxima = vec![None; weights.len()];
                let sizes = distribute(total, &weights, &maxima);
                assert_eq!(
                    sizes.iter().sum::<u16>(),
                    total,
                    "{total} across {weights:?} gave {sizes:?}"
                );
            }
        }
    }

    #[test]
    fn a_panel_that_cannot_use_more_space_hands_it_to_one_that_can() {
        // The calendar case: bounded neighbour, unbounded list.
        let sizes = distribute(100, &[50, 50], &[Some(30), None]);
        assert_eq!(sizes, vec![30, 70], "the surplus must move sideways");
        assert_eq!(sizes.iter().sum::<u16>(), 100);
    }

    #[test]
    fn surplus_from_several_bounded_panels_lands_on_the_one_that_can_grow() {
        let sizes = distribute(120, &[40, 40, 40], &[Some(20), Some(20), None]);
        assert_eq!(sizes, vec![20, 20, 80]);
    }

    #[test]
    fn surplus_is_shared_between_takers_in_proportion_to_their_weights() {
        // Two unbounded panels, one twice the weight of the other.
        let sizes = distribute(120, &[60, 20, 40], &[Some(30), None, None]);
        assert_eq!(sizes.iter().sum::<u16>(), 120);
        assert_eq!(sizes[0], 30, "the bounded one is capped");
        assert!(
            sizes[2] > sizes[1],
            "the heavier taker gets more of it: {sizes:?}"
        );
    }

    #[test]
    fn a_row_of_entirely_bounded_panels_still_covers_its_full_width() {
        // Nobody to hand the surplus to. Panels draw their own frames, so
        // leaving cells unallocated would show as a hole in the dashboard —
        // an over-wide panel is the lesser evil.
        let sizes = distribute(200, &[50, 50], &[Some(30), Some(30)]);
        assert_eq!(
            sizes.iter().sum::<u16>(),
            200,
            "no gap may be left: {sizes:?}"
        );
    }

    #[test]
    fn a_limit_larger_than_the_space_available_changes_nothing() {
        let sizes = distribute(40, &[50, 50], &[Some(500), None]);
        assert_eq!(sizes, vec![20, 20], "a limit nobody reaches is inert");
    }

    #[test]
    fn degenerate_distributions_do_not_panic() {
        assert_eq!(distribute(0, &[1, 1], &[None, None]), vec![0, 0]);
        assert!(distribute(10, &[], &[]).is_empty());
        assert_eq!(
            distribute(10, &[0, 0], &[None, None]).iter().sum::<u16>(),
            10
        );
        assert_eq!(distribute(1, &[1, 1, 1], &[None; 3]).iter().sum::<u16>(), 1);
    }

    #[test]
    fn a_bounded_row_gives_its_leftover_height_to_the_row_below() {
        // The clock is bounded on height — numerals, date, zone table, and
        // nothing that grows past that. The CPU graph is not. The whole point
        // of the mechanism: reclaim the void under the clock and give it to
        // something that fills it.
        //
        // The calendar is deliberately *not* used here: it stacks another row
        // of months when given height, so it is bounded on width only.
        let config = Config {
            layout: LayoutConfig {
                rows: vec![
                    LayoutRow {
                        height: 50,
                        panels: vec![LayoutPanel {
                            widget: "clocks".into(),
                            width: 100,
                        }],
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
        };

        let app = App::new(config).unwrap();
        let bound = app.slots[0]
            .panel
            .max_height()
            .expect("the clock is bounded");
        let rects = app.geometry(Rect::new(0, 0, 120, bound * 3));
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].height, bound, "the clock takes only what it uses");
        assert_eq!(
            rects[0].height + rects[1].height,
            bound * 3,
            "and the height it gave up must be used, not lost"
        );
        assert_eq!(rects[1].y, rects[0].height, "the rows stay flush");
    }

    #[test]
    fn a_calendar_keeps_its_height_because_it_stacks_more_months_into_it() {
        let config = Config {
            layout: LayoutConfig {
                rows: vec![
                    LayoutRow {
                        height: 50,
                        panels: vec![LayoutPanel {
                            widget: "calendar".into(),
                            width: 100,
                        }],
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
        };

        let app = App::new(config).unwrap();
        let rects = app.geometry(Rect::new(0, 0, 120, 60));
        assert_eq!(
            rects[0].height, 30,
            "extra height becomes another row of months, so none is handed back"
        );
    }

    #[test]
    fn a_bounded_panel_gives_its_leftover_width_to_its_neighbour() {
        let config = Config {
            layout: LayoutConfig {
                rows: vec![LayoutRow {
                    height: 100,
                    panels: vec![
                        LayoutPanel {
                            widget: "calendar".into(),
                            width: 50,
                        },
                        LayoutPanel {
                            widget: "cpu".into(),
                            width: 50,
                        },
                    ],
                }],
            },
            ..Config::default()
        };

        let app = App::new(config).unwrap();
        let rects = app.geometry(Rect::new(0, 0, 200, 40));
        assert!(
            rects[0].width < 100,
            "the calendar is bounded: {:?}",
            rects[0]
        );
        assert_eq!(
            rects[0].width + rects[1].width,
            200,
            "and the columns still cover the terminal"
        );
        assert_eq!(rects[1].x, rects[0].width, "the panels stay flush");
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

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn the_picker_opens_and_toggles_a_panel_on_and_off() {
        let mut app = App::new(config_with(&["clocks", "todo"])).unwrap();
        let before = app.slots.len();

        app.handle_key(key(KeyCode::Char('w')));
        assert_eq!(app.picker, Some(0), "opens on the first widget");

        // WIDGET_NAMES starts with clocks, which this layout already places.
        app.handle_key(key(KeyCode::Char(' ')));
        assert!(!app.config.layout.places("clocks"), "space turned it off");
        assert_eq!(app.slots.len(), before - 1, "and the panel actually went");
        assert!(app.layout_dirty);

        app.handle_key(key(KeyCode::Char(' ')));
        assert!(
            app.config.layout.places("clocks"),
            "space turned it back on"
        );
        assert_eq!(app.slots.len(), before);
    }

    #[test]
    fn the_picker_moves_and_closes_without_quitting() {
        let mut app = App::new(config_with(&["clocks", "todo"])).unwrap();
        app.handle_key(key(KeyCode::Char('w')));

        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.picker, Some(1));
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.picker, Some(0));
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.picker, Some(0), "clamps at the top");

        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.picker, None);
        assert!(
            !app.should_quit,
            "Esc closes the dialog rather than falling through to quit"
        );
    }

    #[test]
    fn the_last_panel_cannot_be_switched_off() {
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        app.handle_key(key(KeyCode::Char('w')));
        app.handle_key(key(KeyCode::Char(' ')));

        assert!(
            app.config.layout.places("clocks"),
            "an empty layout is rejected at startup, so this would write a \
             config that cannot be opened again"
        );
        assert_eq!(app.slots.len(), 1);
        assert!(app.layout_error.is_some(), "and it says why");
    }

    #[test]
    fn a_toggle_updates_the_unused_list_the_hint_reads_from() {
        let mut app = App::new(config_with(&["clocks", "todo"])).unwrap();
        assert!(app.unused_widgets.contains(&"pomodoro"));

        app.handle_key(key(KeyCode::Char('w')));
        let index = crate::widgets::WIDGET_NAMES
            .iter()
            .position(|n| *n == "pomodoro")
            .unwrap();
        app.picker = Some(index);
        app.handle_key(key(KeyCode::Char(' ')));

        assert!(
            !app.unused_widgets.contains(&"pomodoro"),
            "switching a widget on must stop it being advertised as missing"
        );
    }

    #[test]
    fn nothing_is_written_when_no_config_path_was_given() {
        // The guard that keeps the whole test suite off a real user's config.
        let mut app = App::new(config_with(&["clocks", "todo"])).unwrap();
        app.handle_key(key(KeyCode::Char('w')));
        app.handle_key(key(KeyCode::Char(' ')));
        assert!(app.layout_dirty);
        app.handle_key(key(KeyCode::Esc));
        assert!(
            app.layout_dirty,
            "still pending, because there was nowhere to write it"
        );
        assert_eq!(app.layout_error, None, "and that is not an error");
    }

    #[test]
    fn a_resize_that_changed_nothing_does_not_mark_the_layout_dirty() {
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        // One panel in its row: there is no neighbour to trade with, so the
        // key does nothing and must not queue a config write.
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert!(!app.layout_dirty);
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
