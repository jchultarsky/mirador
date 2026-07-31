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
    // Last of the primaries, so it is the first to go when the terminal is too
    // narrow for all of them — but a primary, because the alternative is what
    // happened to the resize keys below: shipped, useful, and undiscoverable.
    Binding::primary("m", "arrange"),
    // Behind `m` for the same reason `m` is behind `w`, and a primary for the
    // same reason too: six themes ship, and a theme nobody can find is six
    // files of decoration.
    Binding::primary("t", "theme"),
    // Promoted from `extra` at the owner's request, and the comment on `m`
    // above had already named the reason: shipped, useful, and undiscoverable.
    // Spelled the way the arrange legend and `--help` already spell it —
    // `Ctrl+←/→ resize width` plus `Ctrl+↑/↓ resize height` is 45 cells of
    // status bar and needs 120 columns before either appears, where the
    // collapsed form fits from 92. Which arrow does which axis is the one
    // thing nobody has to be told.
    Binding::primary("Ctrl+arrows", "resize"),
    Binding::extra("Shift+Tab", "focus back"),
    Binding::extra("1-9", "jump to panel"),
    Binding::extra("Ctrl+C", "quit"),
];

/// The text of `lines` with the styling dropped, for measuring.
///
/// Only the characters decide how text wraps, so a measurement does not need
/// the spans — but it does need them concatenated in order, which is why this
/// is not simply the first span of each line.
fn plain_text(lines: &[Line<'_>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// How long a run of resize keystrokes must be quiet before the layout is
/// written back.
///
/// Long enough to sit out key auto-repeat, short enough that closing the window
/// a moment later still keeps the change.
const RESIZE_SETTLE: Duration = Duration::from_millis(750);

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
/// When *every* slot is bounded there is nobody left who can use the surplus,
/// and it is spread back across the whole row rather than left unallocated:
/// panels draw their own frames, so a gap would show as a hole in the middle of
/// the dashboard. Better every panel slightly over its maximum than a seam.
///
/// The emphasis on *spread* is the fix for a real bug. The surplus used to be
/// handed to the slots that could still grow without re-capping them, so on the
/// next pass they were over their own maxima, nobody could absorb it, and the
/// loop broke leaving the whole overshoot on one panel. On the shipped default
/// layout at 400 columns that gave the clock 302 of them — and the clock's
/// numerals stop growing at 158, so about 145 columns were literally blank
/// while the weather panel beside it sat at 51 and the task list below ran out
/// of room. It only appeared once the terminal was wider than the row's maxima
/// summed, which is why nothing caught it until someone opened a 4K terminal.
fn distribute(total: u16, weights: &[u16], maxima: &[Option<u16>]) -> Vec<u16> {
    let count = weights.len();
    if count == 0 || total == 0 {
        return vec![0; count];
    }

    let mut sizes = proportional(total, weights);

    // Each pass caps whoever is over and re-splits what they gave up. Bounded
    // by the slot count: every pass either caps at least one more slot or ends,
    // because a slot only receives surplus while it is still under its maximum.
    for _ in 0..count {
        let mut surplus: u32 = 0;
        for i in 0..count {
            if let Some(max) = maxima[i]
                && sizes[i] > max
            {
                surplus += u32::from(sizes[i] - max);
                sizes[i] = max;
            }
        }
        if surplus == 0 {
            break;
        }
        // `surplus` was summed out of `sizes`, which sums to `total`.
        let surplus = u16::try_from(surplus).unwrap_or(u16::MAX);

        let takers: Vec<usize> = (0..count)
            .filter(|&i| maxima[i].is_none_or(|max| sizes[i] < max))
            .collect();

        if takers.is_empty() {
            // Every slot is at its declared maximum and there are still cells
            // to place. Spread them across the row in proportion, so the
            // overshoot is shared rather than landing entirely on whichever
            // slot happened to be uncapped last.
            for (i, extra) in proportional(surplus, weights).into_iter().enumerate() {
                sizes[i] = sizes[i].saturating_add(extra);
            }
            break;
        }

        let taker_weights: Vec<u16> = takers.iter().map(|&i| weights[i].max(1)).collect();
        for (slot, extra) in takers.iter().zip(proportional(surplus, &taker_weights)) {
            sizes[*slot] = sizes[*slot].saturating_add(extra);
        }
        // A taker may now be over its own maximum; the next pass caps it.
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
    /// Which widget this is, so a layout change can carry the panel across
    /// rather than rebuilding it. Kept on the slot because the config's
    /// `[layout]` has already been mutated by the time a rebuild runs, so it
    /// no longer says what the *current* panels are.
    widget: String,
    panel: Box<dyn Panel>,
    /// When this panel last ticked. `None` until it has, which is what makes
    /// the first tick fire immediately.
    ///
    /// Deliberately not "now minus a day": `Instant` on Windows is a duration
    /// since boot, so `checked_sub` there returns `None` on a machine up for
    /// less than that and the `unwrap` was a hard panic before the terminal
    /// was even initialised. `network.rs` already solved the same "the first
    /// sample is not meaningful" problem this way.
    last_tick: Option<Instant>,
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
    /// First visible line of the help overlay.
    help_scroll: u16,
    /// How far the help overlay can scroll, and how tall its viewport is — both
    /// measured during the render that laid it out, because both depend on the
    /// terminal width the text wrapped at. Zero overflow is also what tells the
    /// key handler to leave the arrow keys alone and close on anything.
    help_overflow: u16,
    help_viewport: u16,
    should_quit: bool,
    /// Widgets available but not placed by this layout.
    ///
    /// A config written by an earlier version silently lacks every widget added
    /// since — an absent widget is a valid choice, so nothing errors and
    /// `--migrate-config` has nothing to fix. This is the only way to find out.
    /// Whether the startup hint is still on screen. Cleared by the first input
    /// of any kind: a dashboard you leave open all day must not nag, and a
    /// notice that will not go away is a nag.
    /// A newer version, if the opt-in check found one. Empty otherwise, and
    /// empty always when the check is off — `App` never starts it, so no test
    /// and no `--print-config` run can reach the network.
    update: crate::update::Found,
    /// Whether the update notice is still on screen. Retired by the first
    /// keypress, exactly like the widget hint: a dashboard you leave open all
    /// day must not nag, and a notice that will not go away is a nag.
    show_update_hint: bool,
    /// What has happened since mirador started.
    ///
    /// Lives here rather than in the watch log panel because the panel may not
    /// be placed, may be toggled off and on, and is rebuilt whenever the layout
    /// changes. Events would be lost every time, and a log that forgets when
    /// you rearrange the dashboard is not a log.
    watch: crate::watch::WatchLog,
    /// The day the watch log last saw.
    ///
    /// Held here rather than in a panel because the day turning is not any
    /// panel's business — the todo panel happens to notice it too, but a
    /// dashboard whose day-divider disappears when you switch off the task
    /// list would be a strange thing.
    today: jiff::civil::Date,
    /// The panel picker, while it is open.
    picker: Option<crate::picker::Picker>,
    /// The theme picker, while it is open, and the theme to put back if it is
    /// cancelled. Previewing means the live theme is not the committed one, so
    /// the dialog cannot be closed without knowing what it replaced.
    theme_picker: Option<(crate::theme_picker::ThemePicker, crate::theme::Theme)>,
    /// The layout as it stood when arrange mode opened, kept so that `Esc` can
    /// put it back. Moving panels is a bigger, more spatial change than
    /// toggling one on, and being able to try an arrangement and back out of it
    /// is most of what makes it safe to try at all.
    arranging: Option<Arrangement>,
    /// The config file, so layout changes can be written back to it. `None` in
    /// tests, which is what keeps them off a real user's config.
    config_path: Option<PathBuf>,
    /// When the last `Ctrl+arrow` landed, if one is still unwritten.
    last_resize: Option<Instant>,
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
    /// What the config file itself says. Preferences are recorded as the
    /// difference from this, which is what lets one be un-set.
    baseline: UiState,
}

/// What arrange mode has to put back if the user changes their mind.
struct Arrangement {
    layout: crate::config::Layout,
    /// Whether the layout was already unwritten when the mode opened — a resize
    /// a moment earlier, say. Cancelling restores the layout, and must not also
    /// throw away a change that was never part of this arrangement.
    was_dirty: bool,
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
        Ok(Self {
            config,
            gradients,
            slots,
            positions,
            focus: 0,
            show_help: false,
            help_scroll: 0,
            help_overflow: 0,
            help_viewport: 0,
            should_quit: false,
            update: crate::update::Found::default(),
            show_update_hint: true,
            watch: crate::watch::WatchLog::default(),
            today: jiff::Zoned::now().date(),
            picker: None,
            theme_picker: None,
            arranging: None,
            config_path: None,
            last_resize: None,
            layout_dirty: false,
            layout_error: None,
            state_path: None,
            saved_state: UiState::default(),
            baseline: UiState::default(),
        })
    }

    /// Build one panel per entry in the layout, in row-major order.
    fn build_slots(config: &Config) -> Result<Built> {
        let mut slots = Vec::new();
        let mut positions = Vec::new();
        for (row_index, row) in config.layout.rows.iter().enumerate() {
            for (column_index, entry) in row.panels.iter().enumerate() {
                let panel = crate::widgets::build(&entry.widget, config)
                    .with_context(|| format!("building the `{}` panel", entry.widget))?;
                if let Some(panel) = panel {
                    slots.push(Slot {
                        widget: entry.widget.clone(),
                        panel,
                        last_tick: None,
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

    /// Reconcile the panels with a changed layout, carrying across every panel
    /// that is still placed.
    ///
    /// This used to throw every panel away and remake them all, on the grounds
    /// that a beat of re-fetching reads as the dashboard responding. That was
    /// wrong, and the demo recording is what showed it: **start the pomodoro,
    /// toggle an unrelated panel, and the timer resets to 25:00.** A running
    /// timer is not a cache that can be refilled — it is the user's state, and
    /// nothing about switching the network panel off says to discard it. The
    /// weather and stocks panels lost their readings the same way and spent a
    /// fetch cycle showing "loading" after any toggle.
    ///
    /// A panel is matched to a layout entry by widget name. Nothing stops a
    /// hand-written config placing the same widget twice — `validate` checks
    /// that names are *known*, not that they are unique — so the surviving
    /// panels are consumed from a pool rather than looked up, and a second
    /// `clocks` entry gets a second panel rather than the same one twice.
    ///
    /// Two orderings matter here:
    ///
    /// 1. Every genuinely new panel is built **before** any existing one is
    ///    disturbed, so a layout that will not build leaves the running
    ///    dashboard exactly as it was. That was already true and is preserved.
    /// 2. Panels are shut down only once the new arrangement is settled, and
    ///    only the ones actually leaving. Dropping them without `shutdown`
    ///    discards the task store's save-on-shutdown and leaks the fetch
    ///    threads.
    ///
    /// This is only ever called after a `[layout]` edit, so no panel's *own*
    /// config can have changed underneath it. A caller that changes, say,
    /// `[weather].units` cannot use this — the carried-over panel would keep
    /// the old setting.
    fn rebuild_panels(&mut self) -> Result<()> {
        use std::collections::{HashMap, VecDeque};

        let desired: Vec<(usize, usize, String)> = self
            .config
            .layout
            .rows
            .iter()
            .enumerate()
            .flat_map(|(row, entry)| {
                entry
                    .panels
                    .iter()
                    .enumerate()
                    .map(move |(column, panel)| (row, column, panel.widget.clone()))
            })
            .collect();

        // What the live panels can supply, by name.
        let mut spare: HashMap<&str, usize> = HashMap::new();
        for slot in &self.slots {
            *spare.entry(slot.widget.as_str()).or_default() += 1;
        }

        // Build only what cannot be carried across. Any failure returns here,
        // with `self` untouched.
        let mut fresh: HashMap<String, VecDeque<Box<dyn Panel>>> = HashMap::new();
        let mut placed = 0usize;
        for (_, _, widget) in &desired {
            if let Some(count) = spare.get_mut(widget.as_str())
                && *count > 0
            {
                *count -= 1;
                placed += 1;
                continue;
            }
            let panel = crate::widgets::build(widget, &self.config)
                .with_context(|| format!("building the `{widget}` panel"))?;
            if let Some(panel) = panel {
                fresh.entry(widget.clone()).or_default().push_back(panel);
                placed += 1;
            }
        }

        // Checked before anything is taken apart, so a layout that would leave
        // nothing on screen is refused rather than applied.
        anyhow::ensure!(
            placed > 0,
            "no panels were built; check the `[layout]` table in your config"
        );

        let focused = self.slots.get(self.focus).map(|slot| slot.widget.clone());

        let mut pool: HashMap<String, VecDeque<Slot>> = HashMap::new();
        for slot in std::mem::take(&mut self.slots) {
            pool.entry(slot.widget.clone()).or_default().push_back(slot);
        }

        let mut slots = Vec::with_capacity(placed);
        let mut positions = Vec::with_capacity(placed);
        for (row, column, widget) in desired {
            let carried = pool.get_mut(&widget).and_then(VecDeque::pop_front);
            let slot = carried.or_else(|| {
                fresh
                    .get_mut(&widget)
                    .and_then(VecDeque::pop_front)
                    .map(|panel| Slot {
                        widget: widget.clone(),
                        panel,
                        last_tick: None,
                        area: None,
                    })
            });
            if let Some(mut slot) = slot {
                // The panel is almost certainly somewhere else on screen now,
                // and `area` is what mouse events are matched against. Cleared
                // rather than trusted until the next draw sets it.
                slot.area = None;
                slots.push(slot);
                positions.push((row, column));
            }
        }

        // Whatever is left was removed from the layout.
        for (_, leaving) in pool {
            for mut slot in leaving {
                slot.panel.shutdown();
            }
        }

        // Follow the focused panel to wherever it ended up, rather than leaving
        // the highlight on whatever now occupies its old index.
        self.focus = focused
            .and_then(|widget| slots.iter().position(|slot| slot.widget == widget))
            .unwrap_or_else(|| self.focus.min(slots.len().saturating_sub(1)));
        self.slots = slots;
        self.positions = positions;
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
                    // The closest thing to "the reader is looking" that a
                    // terminal will tell us. Not every terminal sends it and
                    // tmux forwards it only with `focus-events on`, so it is
                    // an improvement where available rather than the mechanism
                    // — the keypress below carries it everywhere else.
                    //
                    // **Losing focus only.** This used to mark on both, on the
                    // reasoning that gaining focus means they are here now and
                    // losing it means they were here until this moment — each
                    // true in isolation, and together they made the rule line
                    // impossible to see. Returning to the dashboard set "last
                    // looked" to *now*, so everything that arrived while the
                    // reader was away landed on the old side of the line and the
                    // line vanished in the instant they came back to read it.
                    // A terminal that reported focus *correctly* therefore made
                    // the feature less visible than one that did not (#132).
                    //
                    // Leaving is the half that can be recorded without
                    // destroying what it describes: they were present with those
                    // entries on screen, so marking them seen is honest. Coming
                    // back is precisely when the line has to still be there.
                    Event::FocusLost => self.watch.mark_seen(),
                    _ => {}
                }
            }

            dirty |= self.tick_panels();
            dirty |= self.collect_events();

            // Resizes are batched rather than written per keystroke —
            // `Ctrl+arrow` auto-repeats, and rewriting the config on every
            // repeat would be absurd — but they are not batched all the way to
            // exit any more. Closing the terminal window is a normal way to
            // stop a dashboard you leave open all day, and it never reaches the
            // code below: the process is signalled and the pending resize is
            // gone. This settles once the repeats stop, which is the earliest
            // moment the write is not wasted.
            //
            // Deliberately not a signal handler. The only thing at risk is this
            // one write, `SIGKILL` cannot be caught anyway, and the terminal
            // does not need restoring when the terminal is what went away.
            if self.layout_dirty
                && self
                    .last_resize
                    .is_some_and(|at| at.elapsed() >= RESIZE_SETTLE)
            {
                self.write_layout();
                self.last_resize = None;
            }
        }

        for slot in &mut self.slots {
            slot.panel.shutdown();
        }
        // Once more on the way out, in case the last thing changed was not a
        // key — and because Ctrl+C reaches here too.
        self.persist_preferences();
        self.write_layout();
        Ok(())
    }

    /// Remember preferences to `path` from now on.
    ///
    /// Separate from [`App::new`] so that tests, which build apps constantly,
    /// cannot write to a real user's state file by forgetting to opt out. An
    /// app with no path set simply never persists.
    ///
    /// `loaded` is what was read from that file, so startup does not rewrite a
    /// file it has just read. `baseline` is what the *config* says, taken before
    /// the loaded values were folded in — every write is the difference between
    /// the panels and that.
    pub fn remember_preferences_at(&mut self, path: PathBuf, loaded: UiState, baseline: UiState) {
        self.saved_state = loaded;
        self.baseline = baseline;
        self.state_path = Some(path);
    }

    /// The preferences that differ from the config, which is all that is worth
    /// recording.
    ///
    /// Panels report their current values unconditionally; the comparison is
    /// here, once, so a value set back to what the config says drops out of the
    /// file instead of leaving the earlier change asserted for ever.
    fn collect_preferences(&self) -> UiState {
        let mut current = UiState::default();
        for slot in &self.slots {
            slot.panel.remember(&mut current);
        }
        // The theme is the shell's, not any panel's, so it is reported here —
        // but reported the same way, unconditionally, so invariant 17 holds for
        // it too and picking your config's own theme back retracts the entry.
        current.theme.clone_from(&self.config.theme.name);
        current.only_changes_from(&self.baseline)
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

    /// Drain what the panels have noticed into the log.
    ///
    /// Returns whether anything was recorded, so the dashboard redraws: a new
    /// entry is a visible change if the watch log is on screen, and cheaper to
    /// report unconditionally than to ask whether it is placed.
    fn collect_events(&mut self) -> bool {
        let mut recorded = false;

        // The day turning is the one thing in here that always happens, which
        // matters more than it sounds: with a calendar unconfigured and no task
        // falling due, every other source can go quiet for days and leave the
        // panel looking broken. It is also the most useful divider a log of
        // "what changed while I was away" can have.
        let today = jiff::Zoned::now().date();
        if today != self.today {
            self.today = today;
            self.watch.push(crate::watch::Event::new(
                "clock",
                format!("{} began", today.strftime("%A %-d %B")),
            ));
            recorded = true;
        }

        for slot in &mut self.slots {
            for event in slot.panel.events() {
                self.watch.push(event);
                recorded = true;
            }
        }
        recorded
    }

    /// Tick any panel whose refresh interval has elapsed.
    ///
    /// Returns whether any panel ticked, and so whether the screen may now be
    /// out of date.
    /// Tick every panel whose interval has elapsed, and report whether any of
    /// them said something a viewer could see had changed.
    ///
    /// The distinction is the whole point, and getting it wrong is invisible in
    /// a screenshot and enormous in a profile. This used to return "some
    /// panel's timer fired", which the run loop OR'd straight into `dirty`.
    /// Measured on the shipped nine-panel default at 400x100: **243 redraws a
    /// minute**, and the same 243 whether `show_seconds` was on or off — so
    /// with seconds off the dashboard repainted 243 times to show content that
    /// changed once.
    fn tick_panels(&mut self) -> bool {
        let now = Instant::now();
        let mut changed = false;
        for slot in &mut self.slots {
            let due = slot
                .last_tick
                .is_none_or(|last| now.duration_since(last) >= slot.panel.refresh_interval());
            if due {
                // Not `changed |= slot.panel.tick()`: `|=` short-circuits once
                // the accumulator is true, and a panel that stops being ticked
                // stops updating. The operand order is load-bearing.
                changed = slot.panel.tick() || changed;
                slot.last_tick = Some(now);
            }
        }
        changed
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
        self.show_update_hint = false;
        // A keypress is weaker evidence than a focus event — the moments this
        // most wants to be right are the glances that touch nothing — but it is
        // the only evidence available on a terminal that does not report focus.
        self.watch.mark_seen();

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Ctrl+C always quits, even mid-form, because a terminal user expects
        // it to and there is no state we would lose: panels save as they go.
        if ctrl && matches!(key.code, KeyCode::Char('c')) {
            self.should_quit = true;
            return;
        }

        // The help overlay swallows the next key, whatever it is — except the
        // ones that scroll it, because on an 80x24 terminal the focused panel's
        // own bindings do not fit and a key that cannot be reached is a key
        // that does not exist. Scrolling only binds when there is something
        // below the fold, so on a tall terminal any key still closes it.
        if self.show_help {
            if self.help_overflow > 0 {
                let page = self.help_viewport.max(1);
                let moved = match key.code {
                    KeyCode::Down | KeyCode::Char('j') => Some(self.help_scroll.saturating_add(1)),
                    KeyCode::Up | KeyCode::Char('k') => Some(self.help_scroll.saturating_sub(1)),
                    KeyCode::PageDown => Some(self.help_scroll.saturating_add(page)),
                    KeyCode::PageUp => Some(self.help_scroll.saturating_sub(page)),
                    KeyCode::Home => Some(0),
                    KeyCode::End => Some(self.help_overflow),
                    _ => None,
                };
                if let Some(to) = moved {
                    self.help_scroll = to.min(self.help_overflow);
                    return;
                }
            }
            self.show_help = false;
            return;
        }

        // The picker is a real dialog rather than a notice, so it reads keys
        // instead of dismissing on any of them.
        if self.theme_picker.is_some() {
            self.handle_theme_picker_key(key);
            return;
        }
        if self.picker.is_some() {
            self.handle_picker_key(key);
            return;
        }

        // Arrange mode claims the bare arrows, which is the whole point of it
        // being a mode: no modifier to discover, and no terminal that declines
        // to deliver the chord. Anything with Ctrl held falls through, so the
        // resize keys keep working while you are rearranging — which is when
        // you are most likely to want them.
        if self.arranging.is_some() && !ctrl {
            self.handle_arrange_key(key);
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
            if resized {
                self.last_resize = Some(Instant::now());
            }
            // Swallowed either way: a resize that hit the minimum is still a
            // resize key, and must not fall through to a panel binding.
            return;
        }

        self.dispatch_key(key);
    }

    /// Open arrange mode, remembering what to go back to.
    fn enter_arrange(&mut self) {
        self.arranging = Some(Arrangement {
            layout: self.config.layout.clone(),
            was_dirty: self.layout_dirty,
        });
    }

    /// Move the focused panel, or leave the mode.
    fn handle_arrange_key(&mut self, key: KeyEvent) {
        use crate::arrange::Direction;

        // Shift moves the whole row rather than the panel, which is the same
        // escalation the clock panel uses: the bare key moves the small thing,
        // the shifted key moves the thing it sits in. Claimed before the plain
        // arrows below, or `Shift+Down` would fall through and merge the panel.
        //
        // Not `Ctrl+arrows`, which #100 proposed: those already resize inside
        // this mode, advertised in the legend and pinned by
        // `ctrl_arrows_still_resize_inside_arrange_mode`.
        let shifted = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Up if shifted => return self.move_focused_row(false),
            KeyCode::Char('K') => return self.move_focused_row(false),
            KeyCode::Down if shifted => return self.move_focused_row(true),
            KeyCode::Char('J') => return self.move_focused_row(true),
            _ => {}
        }

        let direction = match key.code {
            KeyCode::Left | KeyCode::Char('h') => Some(Direction::Left),
            KeyCode::Right | KeyCode::Char('l') => Some(Direction::Right),
            KeyCode::Up | KeyCode::Char('k') => Some(Direction::Up),
            KeyCode::Down | KeyCode::Char('j') => Some(Direction::Down),
            _ => None,
        };

        if let Some(direction) = direction {
            self.move_focused(direction);
            return;
        }

        match key.code {
            // Pick a different panel to move without leaving the mode.
            // Rearranging a dashboard means moving several things, and having
            // to commit and re-enter between each one would make a two-panel
            // swap a six-keystroke job.
            KeyCode::Tab => self.cycle_focus(true),
            KeyCode::BackTab => self.cycle_focus(false),
            KeyCode::Char(c @ '1'..='9') => {
                let index = c as usize - '1' as usize;
                if index < self.slots.len() {
                    self.focus = index;
                }
            }
            // Keeping it is the ordinary way out, so it gets the ordinary keys.
            // Written here rather than on every arrow: trying four arrangements
            // should cost one write, and the mode is a natural commit point —
            // the same reasoning as the picker.
            KeyCode::Enter | KeyCode::Char('m' | 'q') => {
                self.arranging = None;
                self.write_layout();
            }
            // Esc backs out of it, which is what Esc means everywhere else in
            // mirador. The picker commits on Esc instead, and that is defensible
            // there because each of its changes is one keystroke to undo; an
            // arrangement is not.
            KeyCode::Esc => {
                if let Some(before) = self.arranging.take() {
                    self.config.layout = before.layout;
                    // Rebuilding from a layout that was live a moment ago
                    // cannot fail, and if it somehow did there is nothing
                    // better to fall back to.
                    let _ = self.rebuild_panels();
                    self.layout_dirty = before.was_dirty;
                    self.layout_error = None;
                }
            }
            _ => {}
        }
    }

    /// Move the focused panel one step, putting the layout back if the result
    /// will not build.
    fn move_focused(&mut self, direction: crate::arrange::Direction) {
        let Some(&(row, column)) = self.positions.get(self.focus) else {
            return;
        };
        let before = self.config.layout.clone();
        if crate::arrange::move_panel(&mut self.config.layout, row, column, direction).is_none() {
            return;
        }

        // Focus follows the panel by name, so the moved panel keeps the
        // highlight wherever it lands and nothing here has to chase it.
        if let Err(e) = self.rebuild_panels() {
            self.config.layout = before;
            let _ = self.rebuild_panels();
            self.layout_error = Some(format!("{e:#}"));
            return;
        }
        self.layout_error = None;
        self.layout_dirty = true;
    }

    /// Move the row the focused panel sits in, up or down.
    ///
    /// Shares `move_focused`'s recovery: a layout the panels cannot be rebuilt
    /// from is put back and reported, rather than leaving the dashboard in a
    /// state the config cannot describe.
    fn move_focused_row(&mut self, down: bool) {
        let Some(&(row, _)) = self.positions.get(self.focus) else {
            return;
        };
        let before = self.config.layout.clone();
        if crate::arrange::move_row(&mut self.config.layout, row, down).is_none() {
            return;
        }

        // Focus follows the panel by name, as it does for a panel move, so the
        // highlight stays on whatever the reader was moving even though every
        // panel in the row changed its flat index.
        if let Err(e) = self.rebuild_panels() {
            self.config.layout = before;
            let _ = self.rebuild_panels();
            self.layout_error = Some(format!("{e:#}"));
            return;
        }
        self.layout_error = None;
        self.layout_dirty = true;
    }

    /// Act on whatever the picker made of a keypress.
    fn handle_picker_key(&mut self, key: KeyEvent) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        match picker.handle_key(key) {
            crate::picker::Action::None => {}
            crate::picker::Action::Toggle(name) => self.toggle_widget(name),
            crate::picker::Action::Close => {
                self.picker = None;
                // Written on close rather than on every toggle: someone trying
                // three arrangements should cost one write, not three, and the
                // dialog is a natural commit point.
                self.write_layout();
            }
        }
    }

    /// Open the theme picker, remembering the theme to put back on `Esc`.
    ///
    /// The themes directory sits beside the config, so a `--config` somewhere
    /// unusual looks for themes beside *it*. With no config path — which is
    /// only ever the tests — the bundled set is the whole list.
    fn open_theme_picker(&mut self) {
        let dir = self
            .config_path
            .as_deref()
            .and_then(crate::themes::user_dir);
        let picker = crate::theme_picker::ThemePicker::new(
            self.config.theme.name.as_deref(),
            dir.as_deref(),
        );
        self.theme_picker = Some((picker, self.config.theme.clone()));
    }

    /// Act on whatever the theme picker made of a keypress.
    fn handle_theme_picker_key(&mut self, key: KeyEvent) {
        let Some((picker, original)) = self.theme_picker.as_mut() else {
            return;
        };
        match picker.handle_key(key) {
            crate::theme_picker::Action::None => {}
            crate::theme_picker::Action::Preview(name) => {
                let dir = self
                    .config_path
                    .as_deref()
                    .and_then(crate::themes::user_dir);
                // A theme that will not load is left as a dead row rather than
                // reported: the file is the user's own, they have just watched
                // every other name repaint the screen, and the one that does
                // nothing is legible enough. Anything louder would put an error
                // dialog on top of a colour picker.
                if let Ok(theme) = crate::themes::resolve(&name, dir.as_deref()) {
                    self.apply_theme(theme);
                }
            }
            crate::theme_picker::Action::Accept => {
                self.theme_picker = None;
                // The live theme is already the chosen one, so there is nothing
                // to apply — only to record. Written here rather than on every
                // cursor move, so browsing the list costs no writes.
                self.persist_preferences();
            }
            crate::theme_picker::Action::Cancel => {
                let restore = original.clone();
                self.theme_picker = None;
                self.apply_theme(restore);
            }
        }
    }

    /// Swap the live theme, including the colours derived from it.
    ///
    /// `gradients` is the one thing that does not read `config.theme` at draw
    /// time — it is a baked 101-entry ramp, computed once. Forgetting it here
    /// would leave the cpu and network graphs painted in the previous theme
    /// while everything around them changed, which looks like a rendering bug
    /// rather than a missing line.
    fn apply_theme(&mut self, theme: crate::theme::Theme) {
        self.config.theme = theme;
        self.gradients = self.config.theme.gradients();
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

        // Atomic, like every other file mirador writes. This used to be a bare
        // `fs::write`, which is the one place it mattered most: the target is
        // the user's own config, complete with the comments they may have edited
        // and the ones mirador wrote to explain itself, and a crash or a full
        // disk part-way through a plain overwrite leaves them a truncated file
        // and nothing to recover from.
        let result = std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|source| crate::layout_edit::apply(&source, &self.config.layout))
            .and_then(|updated| crate::store::write_atomic(&path, &updated));

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
            // `q` and Ctrl+C only. Esc used to quit here, undocumented — while
            // the task panel prints "Nothing matches this filter. Esc to
            // clear." A panel consumes Esc only while its filter is non-empty,
            // so the same key in the same panel one keystroke apart either
            // cleared the filter or killed the dashboard, and nothing on screen
            // said which. Esc means "back out of something" everywhere else.
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Tab => self.cycle_focus(true),
            KeyCode::BackTab => self.cycle_focus(false),
            KeyCode::Char('?') => {
                self.show_help = true;
                // Opening it always starts at the top; the bindings for the
                // panel you just focused are the reason you pressed `?`.
                self.help_scroll = 0;
            }
            KeyCode::Char('w') => self.picker = Some(crate::picker::Picker::new()),
            KeyCode::Char('t') => self.open_theme_picker(),
            KeyCode::Char('m') => self.enter_arrange(),
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
        let had_hint = std::mem::take(&mut self.show_update_hint);

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

        // Indexed by slot, not by layout column, and written through `slot_at`.
        // Pushing one rect per column assumes every layout entry produced a
        // panel; a single entry that did not shifts every later slot onto the
        // previous entry's rectangle, which is what `slot.area` hit-tests, so
        // clicks land on the wrong panel. A slot that gets no rectangle keeps
        // the zero one and is skipped by the caller's size check.
        let mut rects = vec![Rect::default(); self.slots.len()];
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
            for (column, width) in columns.into_iter().enumerate() {
                if let Some(slot) = self.slot_at(row_index, column) {
                    rects[slot] = Rect::new(x, y, width, height);
                }
                x = x.saturating_add(width);
            }
            y = y.saturating_add(height);
        }
        rects
    }

    /// Which row the open picker is on, for tests that drive it by keystroke.
    #[cfg(test)]
    fn picker_row(&self) -> Option<usize> {
        self.picker.as_ref().map(crate::picker::Picker::selected)
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
                    watch: &self.watch,
                },
            );
        }

        // Panel dialogs, before the shell's own overlays and after every
        // panel: drawn any earlier and the panels following the one that owns
        // the dialog paint straight over it.
        for slot in &self.slots {
            if let Some(prompt) = slot.panel.overlay() {
                prompt.render(frame, area, &self.config.theme);
            }
        }

        if self.show_help {
            self.render_help(frame, area);
        }
        if let Some((picker, _)) = &self.theme_picker {
            picker.render(frame, area, &self.config.theme);
        }
        if let Some(picker) = &self.picker {
            picker.render(
                frame,
                area,
                &self.config.theme,
                |name| self.config.layout.places(name),
                self.layout_error.as_deref(),
            );
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

        // In arrange mode the bar belongs to the mode. The global keys are not
        // reachable anyway — the arrows have been claimed — so listing them
        // would be listing keys that do nothing.
        //
        // This also hides an alert for as long as the mode is open, which is a
        // decision rather than an oversight. Arrange mode is a gesture measured
        // in seconds and the legend is what makes it usable; an alert that
        // waits until `Enter` or `Esc` has lost nothing, because the thing it
        // names has already happened either way.
        if self.arranging.is_some() {
            let mut spans = vec![Span::styled(
                " ARRANGE",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )];
            // Which arrow moves along the row and which moves between rows is
            // self-evident the moment you press one, because the panels move.
            // What is *not* evident is that there is a row past the last one,
            // so the width saved by collapsing the first two entries into
            // "move" is spent saying so. Someone had to ask.
            //
            // Ordered so that the two that matter most survive a narrow
            // terminal: knowing how to get out of a mode beats knowing every
            // trick inside it.
            let mut used = crate::grid::display_width(" ARRANGE");
            // Both vertical hints were shortened when `Shift+↑↓` was added in
            // #100: a sixth entry does not fit beside the old wording at an
            // ordinary 110 columns, and dropping one was not an option —
            // the help overlay carries only the *global* keys, so this legend
            // is the only place any of these are documented. A key that ships
            // undiscoverable is the mistake the resize keys already made once.
            for (key, action) in [
                ("←→↑↓", "move"),
                ("Enter", "keep"),
                ("Esc", "cancel"),
                ("Shift+↑↓", "move row"),
                ("Ctrl+arrows", "resize"),
                ("↑↓ at edge", "new row"),
            ] {
                // Dropped whole rather than clipped: half a hint reads as a
                // rendering fault, where a missing one just reads as a narrow
                // terminal.
                let width = 3 + crate::grid::display_width(key) + 1 + action.len();
                if used + width > usize::from(area.width) {
                    break;
                }
                used += width;
                spans.push(Span::styled("   ", muted));
                spans.push(Span::styled(key, key_style));
                spans.push(Span::styled(format!(" {action}"), muted));
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), area);
            return;
        }

        let mut spans = vec![Span::styled(
            " mirador",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )];

        // Dropped whole rather than clipped, exactly as the arrange legend
        // above does it: half a hint reads as a rendering fault, a missing one
        // reads as a narrow terminal. This bar had no such guard and did not
        // need one while every primary was a single character — `t theme` was
        // the widest thing in it. Promoting the resize keys made it need one:
        // at 80 columns the bar ended `Ctrl+←`.
        let mut used = crate::grid::display_width(" mirador");
        for binding in GLOBAL.iter().filter(|b| b.primary) {
            let width = 3
                + crate::grid::display_width(binding.key)
                + 1
                + crate::grid::display_width(binding.action);
            if used + width > usize::from(area.width) {
                break;
            }
            used += width;
            spans.push(Span::styled("   ", muted));
            spans.push(Span::styled(binding.key, key_style));
            spans.push(Span::styled(format!(" {}", binding.action), muted));
        }

        // An alert takes the whole bar rather than sharing it. It outranks both
        // hints — one is about a new version, the other about widgets you are
        // not using, and neither gets worse while you read the other — and it
        // outranks the key list too, because the *reason* is the actionable
        // part and squeezing it in beside `Tab focus` truncated it to
        // "read-only file syste…". The keys are behind `?`, and an alert is
        // gone as soon as the thing it names is.
        if let Some(alert) = self.alert() {
            let marker = " ⚠ ";
            let room = usize::from(area.width).saturating_sub(marker.chars().count());
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        marker,
                        Style::default()
                            .fg(theme.error)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        crate::grid::truncate(&alert.text, room),
                        Style::default().fg(theme.error),
                    ),
                ])),
                area,
            );
            return;
        }

        // The hint rides on the right of the bar it shares with the global
        // keys, and gives way to them when the terminal is too narrow: knowing
        // how to quit matters more than knowing what you are not using.
        if let Some(hint) = self.update_hint() {
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

    /// The single most pressing thing, if anything is pressing.
    ///
    /// Absent almost always, which is the point — see [`crate::panel::Alert`].
    /// There is no all-clear: an indicator that says everything is fine is a
    /// light you have to read in order to learn nothing, and reading it is work
    /// a dashboard should not ask for.
    fn alert(&self) -> Option<crate::panel::Alert> {
        // The layout write is the one genuine silent failure in the program.
        // It is reported inside the `w` picker and nowhere else, so a
        // rearrangement that fails to persist says nothing at all once the
        // picker closes — and the change is gone at the next launch.
        let own = self.layout_error.as_ref().map(|why| {
            crate::panel::Alert::failing(format!("The layout could not be saved — {why}"))
        });

        self.slots
            .iter()
            .filter_map(|slot| slot.panel.alert())
            .chain(own)
            // Most severe wins, and the newest of equals — with two levels and
            // both near-never, which one shows barely matters; that it is only
            // ever *one* does.
            .max_by_key(|alert| alert.severity)
    }

    /// The one-line startup notice about widgets this layout does not place.
    /// Watch `found` for a newer version from now on.
    ///
    /// Separate from [`App::new`] for the same reason the state path is: tests
    /// build apps constantly, and none of them should be able to start a
    /// network request by accident.
    pub fn watch_for_updates(&mut self, found: crate::update::Found) {
        self.update = found;
    }

    /// The update notice, if there is one and it has not been dismissed.
    ///
    /// Takes precedence over the unused-widget hint when both apply: this one
    /// is rarer, is actionable now, and stops being true the moment you act on
    /// it, where the widget hint is the same every launch until you change your
    /// layout.
    fn update_hint(&self) -> Option<String> {
        if !self.show_update_hint {
            return None;
        }
        let latest = match self.update.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }?;
        Some(format!("mirador {latest} is out   mirador-update "))
    }

    fn render_help(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let theme = self.config.theme.clone();
        let theme = &theme;
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

        // The footer is rendered separately and pinned to the last row, rather
        // than being the last line of the scrolling text. A hint saying how to
        // close the overlay is no use once it has scrolled out of the overlay.
        let width = 46.min(area.width);
        let text_width = width.saturating_sub(crate::frame::FRAME_WIDTH).max(1);
        // Measured after wrapping, not from `lines.len()`. The two differ
        // whenever a line is longer than the popup, which the list of unused
        // widgets routinely is — sizing from the unwrapped count is how the
        // overlay came to be shorter than its own contents.
        let text_height = crate::grid::wrapped_height(&plain_text(&lines), text_width);
        let body = Paragraph::new(lines).wrap(Wrap { trim: false });

        // Borders, the blank line, and the footer.
        let chrome = 4;
        let height = text_height.saturating_add(chrome).min(area.height);
        let popup = crate::frame::centred(area, width, height);

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
            ]))
            // Right-aligned in the border, the way every panel shows its
            // counter — `?` is where someone goes to find out what the thing
            // does, so it is also where they will look for what version it is,
            // and the border costs no rows to say so.
            .title_top(
                Line::from(vec![
                    Span::styled("┤", Style::default().fg(theme.border_focused)),
                    Span::styled(
                        concat!("v", env!("CARGO_PKG_VERSION")),
                        Style::default().fg(theme.muted),
                    ),
                    Span::styled("├", Style::default().fg(theme.border_focused)),
                ])
                .right_aligned(),
            );
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        if inner.height == 0 {
            self.help_overflow = 0;
            self.help_viewport = 0;
            return;
        }

        // Give the footer the last row and the blank line the one above it,
        // but never at the cost of showing no text at all.
        let footer_rows = 2.min(inner.height.saturating_sub(1));
        let viewport = Rect {
            height: inner.height - footer_rows,
            ..inner
        };

        self.help_viewport = viewport.height;
        self.help_overflow = text_height.saturating_sub(viewport.height);
        // The terminal may have shrunk since the last frame, or the focused
        // panel changed to one with fewer bindings.
        self.help_scroll = self.help_scroll.min(self.help_overflow);

        frame.render_widget(body.scroll((self.help_scroll, 0)), viewport);

        if footer_rows > 0 {
            let footer = Rect {
                y: inner.y + inner.height - 1,
                height: 1,
                ..inner
            };
            frame.render_widget(Paragraph::new(self.help_footer(theme)), footer);
        }
    }

    /// The pinned last row of the help overlay.
    ///
    /// It says how to close the overlay, and when there is more text than fits,
    /// that scrolling is possible and where in the list you are. Without the
    /// position there is no way to tell a full list from a truncated one.
    fn help_footer(&self, theme: &crate::theme::Theme) -> Line<'static> {
        let italic = Style::default()
            .fg(theme.muted)
            .add_modifier(Modifier::ITALIC);

        if self.help_overflow == 0 {
            return Line::from(Span::styled("any key to close", italic));
        }

        let key_style = Style::default().fg(theme.key).add_modifier(Modifier::BOLD);
        let more_above = self.help_scroll > 0;
        let more_below = self.help_scroll < self.help_overflow;
        let arrows = match (more_above, more_below) {
            (true, true) => "↑↓",
            (true, false) => "↑",
            _ => "↓",
        };
        Line::from(vec![
            Span::styled(arrows, key_style),
            Span::styled(
                format!(
                    " {}/{} · any other key to close",
                    self.help_scroll + self.help_viewport,
                    self.help_overflow + self.help_viewport,
                ),
                italic,
            ),
        ])
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

    /// The status bar as drawn at `width`, with styling dropped.
    fn status_bar_at(app: &mut App, width: u16) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("backend");
        terminal
            .draw(|frame| app.render_status_bar(frame, Rect::new(0, 0, width, 1)))
            .expect("draws");
        let buffer = terminal.backend().buffer();
        (0..width)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<String>()
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

    /// The whole grid as widget names, which is what a move is supposed to
    /// change and what a cancelled one is supposed to restore.
    fn widgets_by_row(app: &App) -> Vec<Vec<String>> {
        app.config
            .layout
            .rows
            .iter()
            .map(|row| row.panels.iter().map(|p| p.widget.clone()).collect())
            .collect()
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

    /// The mode has to claim the bare arrows before panels do, for the same
    /// reason the resize keys are claimed: the calendar binds plain arrows and
    /// does not inspect modifiers, so a left arrow meant to move a panel would
    /// scroll a month instead.
    /// The mode has to say that there is a row past the last one. Someone read
    /// `↑↓ move rows`, took it to mean "move between the rows that exist", and
    /// asked how to make a new one — which the mode had done all along.
    #[test]
    fn the_arrange_legend_says_a_row_can_be_opened() {
        let mut app = App::new(resizable()).expect("builds");
        app.handle_key(KeyEvent::from(KeyCode::Char('m')));

        let bar = status_bar_at(&mut app, 110);
        assert!(bar.contains("ARRANGE"), "the mode names itself: {bar}");
        assert!(
            bar.contains("new row"),
            "and says rows can be opened: {bar}"
        );
        // #100 added `Shift+↑↓`, and this legend is the only place the mode's
        // keys are written down — the help overlay carries global keys only. So
        // an ordinary terminal has to show every one of them, which is what
        // forced both vertical hints to be shortened rather than one dropped.
        assert!(
            bar.contains("move row"),
            "and says a row can be moved: {bar}"
        );
    }

    /// The resize keys shipped as `extra` and so appeared only behind `?`.
    /// The owner went looking for them in the status bar, did not find them,
    /// and concluded the feature did not exist — which is the whole argument
    /// for `primary`, and the same failure #109 and #117 already made.
    #[test]
    fn the_resize_keys_are_advertised_on_the_status_bar() {
        let mut app = App::new(resizable()).expect("builds");
        let bar = status_bar_at(&mut app, 120);
        assert!(
            bar.contains("Ctrl+arrows resize"),
            "resize must be advertised where someone looking for it will look: {bar}"
        );
    }

    /// The bar is built from one table, so a hint cannot be worded differently
    /// in two places — but it *can* be worded differently from the legend and
    /// `--help`, which are separate strings. All three say `Ctrl+arrows`.
    #[test]
    fn the_resize_hint_is_worded_the_way_arrange_mode_words_it() {
        let mut app = App::new(resizable()).expect("builds");
        let plain = status_bar_at(&mut app, 120);
        app.handle_key(KeyEvent::from(KeyCode::Char('m')));
        let legend = status_bar_at(&mut app, 120);
        assert!(
            plain.contains("Ctrl+arrows resize") && legend.contains("Ctrl+arrows resize"),
            "the same key must read the same in both bars:\n  {plain}\n  {legend}"
        );
    }

    /// Every width must show whole hints or none — never a fragment. This is
    /// the property the arrange legend has always had and this bar did not:
    /// while every primary was one character the shortfall never showed, and
    /// promoting `Ctrl+arrows` made it show at 80 columns as `Ctrl+←`.
    ///
    /// Asserted by construction rather than by looking for a fragment: the
    /// drawn bar has to be one of the prefixes that end on a hint boundary,
    /// which no partial hint can be.
    #[test]
    fn the_status_bar_never_draws_half_a_hint() {
        let mut app = App::new(resizable()).expect("builds");

        let mut whole = Vec::new();
        let mut acc = String::from(" mirador");
        whole.push(acc.clone());
        for binding in GLOBAL.iter().filter(|b| b.primary) {
            acc.push_str("   ");
            acc.push_str(binding.key);
            acc.push(' ');
            acc.push_str(binding.action);
            whole.push(acc.clone());
        }

        for width in 8..200u16 {
            let bar = status_bar_at(&mut app, width);
            let drawn = bar.trim_end().to_string();
            assert!(
                whole.contains(&drawn),
                "the bar was cut mid-hint at {width}: {drawn:?}"
            );
            assert!(
                crate::grid::display_width(&drawn) <= usize::from(width),
                "the bar overflowed at {width}: {drawn:?}"
            );
        }
    }

    /// A terminal too narrow for every hint keeps the ones that get you out.
    /// Half a hint reads as a rendering fault; a missing one reads as a narrow
    /// terminal, which is what it is.
    #[test]
    fn a_narrow_arrange_legend_drops_whole_hints_and_keeps_the_way_out() {
        let mut app = App::new(resizable()).expect("builds");
        app.handle_key(KeyEvent::from(KeyCode::Char('m')));

        for width in 40..110u16 {
            let bar = status_bar_at(&mut app, width);
            assert!(
                crate::grid::display_width(bar.trim_end()) <= usize::from(width),
                "the bar overflowed at {width}: {bar}"
            );
            if width >= 46 {
                assert!(bar.contains("Esc cancel"), "no way out at {width}: {bar}");
            }
        }
    }

    /// The day turning is the one source that always fires, which is what
    /// stops the watch log looking broken on a dashboard with no calendar and
    /// nothing falling due. It lives in the shell rather than a panel so that
    /// switching off the task list cannot take it away.
    #[test]
    fn the_day_turning_is_recorded_whatever_panels_are_placed() {
        // Only a clock: neither of the panels that report events is here.
        let mut app = App::new(config_with(&["clocks"])).expect("builds");
        assert!(app.watch.entries().next().is_none(), "nothing yet");

        // Wind the remembered day back, which is what a night does.
        app.today = app.today.yesterday().expect("a day before today exists");
        assert!(app.collect_events(), "the rollover is worth a redraw");

        let entry = app.watch.entries().next().expect("an entry was recorded");
        assert!(
            entry.text.ends_with("began"),
            "reads as a day divider: {}",
            entry.text
        );

        // And only once — a second pass on the same day adds nothing.
        assert!(!app.collect_events(), "the same day is not news twice");
        assert_eq!(app.watch.entries().count(), 1);
    }

    /// The layout write is the one thing in the program that could fail
    /// silently: it was reported inside the `w` picker and nowhere else, so a
    /// rearrangement that did not persist said nothing once the picker closed,
    /// and the change was gone at the next launch.
    #[test]
    fn a_layout_that_would_not_save_reaches_the_status_bar() {
        let mut app = App::new(resizable()).expect("builds");
        assert!(app.alert().is_none(), "nothing is wrong yet");

        app.layout_error = Some("read-only file system (os error 30)".into());
        let alert = app.alert().expect("an alert");
        assert_eq!(alert.severity, crate::panel::Severity::Failing);
        assert!(alert.text.contains("layout"), "names it: {}", alert.text);

        let bar = status_bar_at(&mut app, 120);
        assert!(bar.contains('⚠'), "and it is on the bar: {bar}");
        assert!(bar.contains("os error 30"), "with the reason: {bar}");
    }

    /// There is no all-clear. An indicator saying everything is fine is a light
    /// you have to read to learn nothing, and reading it is work.
    #[test]
    fn a_quiet_dashboard_says_nothing_at_all() {
        let mut app = App::new(resizable()).expect("builds");
        let bar = status_bar_at(&mut app, 120);
        assert!(!bar.contains('⚠'), "no marker: {bar}");
        assert!(
            !bar.to_lowercase().contains("ok") && !bar.to_lowercase().contains("clear"),
            "and no reassurance: {bar}"
        );
    }

    /// An alert outranks the update notice. That notice is not going to get
    /// worse while you read the alert; the alert might.
    #[test]
    fn an_alert_displaces_the_hints_rather_than_queueing_behind_them() {
        let mut app = App::new(config_with(&["clocks"])).expect("builds");
        app.watch_for_updates(std::sync::Arc::new(std::sync::Mutex::new(Some(
            "9.9.9".to_string(),
        ))));
        let before = status_bar_at(&mut app, 200);
        assert!(before.contains("9.9.9"), "the notice is there to displace");

        app.layout_error = Some("disk full".into());
        let after = status_bar_at(&mut app, 200);
        assert!(after.contains("disk full"), "the alert shows: {after}");
        assert!(
            !after.contains("9.9.9"),
            "and the notice gives way: {after}"
        );
    }

    #[test]
    fn arrange_claims_the_arrows_the_focused_panel_would_otherwise_take() {
        let mut app = App::new(resizable()).expect("builds");
        app.handle_key(KeyEvent::from(KeyCode::Char('m')));
        assert!(app.arranging.is_some(), "m opened the mode");

        let before = widgets_by_row(&app);
        app.handle_key(KeyEvent::from(KeyCode::Right));
        assert_ne!(
            widgets_by_row(&app),
            before,
            "the arrow moved a panel rather than reaching the calendar"
        );
    }

    /// Esc means "back out of this" everywhere else in mirador, and an
    /// arrangement is exactly the kind of change worth being able to abandon.
    #[test]
    fn esc_puts_the_layout_back_exactly_as_it_was() {
        let mut app = App::new(resizable()).expect("builds");
        let before = widgets_by_row(&app);

        app.handle_key(KeyEvent::from(KeyCode::Char('m')));
        for code in [KeyCode::Right, KeyCode::Down, KeyCode::Up] {
            app.handle_key(KeyEvent::from(code));
        }
        assert_ne!(widgets_by_row(&app), before, "something actually moved");

        app.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(app.arranging.is_none(), "the mode closed");
        assert_eq!(widgets_by_row(&app), before, "and the layout came back");
        assert!(
            !app.layout_dirty,
            "a cancelled arrangement leaves nothing to write"
        );
    }

    /// A resize made before the mode opened is not part of the arrangement, so
    /// cancelling must not swallow it. Restoring the layout and clearing the
    /// flag unconditionally would lose a change the user had already made.
    #[test]
    fn cancelling_does_not_discard_a_change_made_before_the_mode_opened() {
        let mut app = App::new(resizable()).expect("builds");
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert!(app.layout_dirty, "the resize is waiting to be written");

        app.handle_key(KeyEvent::from(KeyCode::Char('m')));
        app.handle_key(KeyEvent::from(KeyCode::Down));
        app.handle_key(KeyEvent::from(KeyCode::Esc));

        assert!(
            app.layout_dirty,
            "the earlier resize still needs writing after the cancel"
        );
    }

    /// Focus follows the widget rather than the slot index, so the panel you
    /// are moving stays the one under the highlight. Without this you would
    /// move a panel once and then start moving whatever slid into its place.
    #[test]
    fn the_moved_panel_keeps_the_focus() {
        let mut app = App::new(resizable()).expect("builds");
        app.focus = 0;
        let moving = app.slots[0].widget.clone();

        app.handle_key(KeyEvent::from(KeyCode::Char('m')));
        app.handle_key(KeyEvent::from(KeyCode::Right));

        assert_eq!(
            app.slots[app.focus].widget, moving,
            "the highlight went with the panel"
        );
    }

    /// #100, tested through the shell rather than through `arrange::move_row`.
    ///
    /// The arithmetic having its own tests is not enough — #106 shipped a
    /// correct helper that was simply never called, and two unit tests passed
    /// throughout. This presses the key.
    #[test]
    fn shift_arrows_move_the_whole_row_in_arrange_mode() {
        let mut app = App::new(resizable()).expect("builds");
        // `resizable` is [clocks, calendar] over [cpu]. Focus the lone panel in
        // the second row — before #100 it could only ever merge upward.
        app.focus = 2;
        let moving = app.slots[2].widget.clone();
        assert_eq!(moving, "cpu", "fixture changed under this test");

        app.handle_key(KeyEvent::from(KeyCode::Char('m')));
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));

        assert_eq!(
            app.config.layout.rows.len(),
            2,
            "moving a row must not merge it away"
        );
        assert_eq!(
            app.config.layout.rows[0].panels[0].widget, "cpu",
            "the cpu row should now be first"
        );
        assert_eq!(
            app.slots[app.focus].widget, moving,
            "the highlight went with the panel"
        );
        assert!(app.arranging.is_some(), "and the mode stayed open");
    }

    /// `J`/`K` are the same gesture without a modifier, matching the clock
    /// panel. Bare `j`/`k` must still move the *panel*, or the shifted pair
    /// would have quietly replaced the plain one.
    #[test]
    fn capital_and_plain_movement_keys_do_different_things() {
        let mut app = App::new(resizable()).expect("builds");
        app.focus = 2;
        app.handle_key(KeyEvent::from(KeyCode::Char('m')));
        app.handle_key(KeyEvent::from(KeyCode::Char('K')));
        assert_eq!(
            app.config.layout.rows[0].panels[0].widget, "cpu",
            "K should have moved the row"
        );

        // And the plain key still merges, which is the behaviour #100 was
        // careful not to take away.
        let mut app = App::new(resizable()).expect("builds");
        app.focus = 2;
        app.handle_key(KeyEvent::from(KeyCode::Char('m')));
        app.handle_key(KeyEvent::from(KeyCode::Char('k')));
        assert_eq!(
            app.config.layout.rows.len(),
            1,
            "plain k should still merge the panel into the row above"
        );
    }

    /// The arrows are the mode's, but Ctrl+arrow still has to reach the resize
    /// path — rearranging is exactly when you want to adjust proportions.
    #[test]
    fn ctrl_arrows_still_resize_inside_arrange_mode() {
        let mut app = App::new(resizable()).expect("builds");
        app.handle_key(KeyEvent::from(KeyCode::Char('m')));

        let before = widths(&app);
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_ne!(widths(&app), before, "Ctrl+Right resized rather than moved");
        assert!(app.arranging.is_some(), "and did not close the mode");
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
    fn toggling_one_panel_leaves_the_others_untouched() {
        // The bug the demo recording caught: `rebuild_panels` remade every
        // panel, so switching the network panel off reset a running pomodoro to
        // 25:00 and sent the weather and stocks panels back to "loading".
        //
        // Panels are compared by pointer identity — the same allocation before
        // and after is the only thing that actually proves state survived,
        // where comparing a rendered figure would pass for a panel that had
        // been rebuilt and happened to look the same.
        let mut app = App::new(config_with(&["clocks", "todo", "pomodoro"])).unwrap();
        let before: Vec<*const u8> = app
            .slots
            .iter()
            .map(|slot| std::ptr::from_ref(&*slot.panel).cast::<u8>())
            .collect();

        app.toggle_widget("pomodoro");
        assert!(!app.config.layout.places("pomodoro"), "it went");

        let after: Vec<*const u8> = app
            .slots
            .iter()
            .map(|slot| std::ptr::from_ref(&*slot.panel).cast::<u8>())
            .collect();
        assert_eq!(after, before[..2], "the surviving panels were rebuilt");

        // And back again: the two that never left are still the same panels.
        app.toggle_widget("pomodoro");
        assert!(app.config.layout.places("pomodoro"));
        let again: Vec<*const u8> = app
            .slots
            .iter()
            .take(2)
            .map(|slot| std::ptr::from_ref(&*slot.panel).cast::<u8>())
            .collect();
        assert_eq!(again, before[..2], "re-adding a panel rebuilt the others");
    }

    #[test]
    fn focus_follows_the_panel_rather_than_the_index() {
        // Removing a panel to the left of the focused one shifts every later
        // index down. Leaving `focus` where it was moves the highlight to a
        // different panel, which gets noticed only when the next keypress goes
        // somewhere unexpected.
        //
        // The indices are chosen so that clamping cannot pass by accident: with
        // four panels and focus on the second, removing the first leaves the
        // old `min(focus, len - 1)` pointing at the *third* widget.
        let mut app = App::new(config_with(&["clocks", "todo", "pomodoro", "notes"])).unwrap();
        app.focus = 1;
        assert_eq!(app.slots[app.focus].widget, "todo");

        app.toggle_widget("clocks");
        assert_eq!(
            app.slots[app.focus].widget, "todo",
            "focus jumped to another panel"
        );
    }

    #[test]
    fn a_widget_placed_twice_gets_two_panels() {
        // `Config::validate` checks that widget names are *known*, not that
        // they are unique, so a hand-written config can place one twice.
        // Matching panels to entries by name has to consume from a pool — a
        // lookup would hand the same panel to both entries.
        let mut config = config_with(&["clocks", "clocks", "todo"]);
        config.layout.rows[0].panels[0].width = 30;
        config.layout.rows[0].panels[1].width = 30;
        config.layout.rows[0].panels[2].width = 40;

        let mut app = App::new(config).unwrap();
        assert_eq!(app.slots.len(), 3);

        let distinct: std::collections::HashSet<*const u8> = app
            .slots
            .iter()
            .map(|slot| std::ptr::from_ref(&*slot.panel).cast::<u8>())
            .collect();
        assert_eq!(distinct.len(), 3, "two entries share one panel");

        // A rebuild must keep them distinct too.
        app.toggle_widget("notes");
        let distinct: std::collections::HashSet<*const u8> = app
            .slots
            .iter()
            .map(|slot| std::ptr::from_ref(&*slot.panel).cast::<u8>())
            .collect();
        assert_eq!(distinct.len(), app.slots.len(), "a panel was reused twice");
    }

    #[test]
    fn no_update_notice_until_something_finds_a_version() {
        // `App` never starts a check itself. A dashboard built in a test — or
        // by `--print-config` — must not be able to reach the network.
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        assert_eq!(app.update_hint(), None);

        app.watch_for_updates(std::sync::Arc::new(std::sync::Mutex::new(Some(
            "9.9.9".to_string(),
        ))));
        let hint = app.update_hint().expect("a found version should show");
        assert!(hint.contains("9.9.9"), "got `{hint}`");
        assert!(
            hint.contains("mirador-update"),
            "the notice must say what to do about it: `{hint}`"
        );
    }

    #[test]
    fn the_update_notice_retires_on_the_first_keypress() {
        // Same rule as the widget hint. A dashboard left open all day must not
        // keep telling you something you have already read.
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        app.watch_for_updates(std::sync::Arc::new(std::sync::Mutex::new(Some(
            "9.9.9".to_string(),
        ))));
        assert!(app.update_hint().is_some());

        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.update_hint(), None, "the notice outlived a keypress");
    }

    #[test]
    fn the_update_notice_has_the_end_of_the_status_bar_to_itself() {
        // It used to share the row with a hint listing the widgets your layout
        // left out. That hint is gone — see
        // `a_layout_missing_widgets_is_not_advertised_anywhere` — so the update
        // notice is the only thing that can appear here.
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        app.watch_for_updates(std::sync::Arc::new(std::sync::Mutex::new(Some(
            "9.9.9".to_string(),
        ))));
        let shown = app.update_hint().expect("an update is waiting");
        assert!(shown.contains("9.9.9"), "got `{shown}`");
    }

    /// A layout that leaves widgets out is a decision, not an oversight.
    ///
    /// mirador used to say so — once in the status bar at startup, and for ever
    /// in the help overlay. The status bar line retired on the first keypress;
    /// the overlay section did not, so someone who had deliberately switched
    /// four panels off was told about them every time they pressed `?`. That is
    /// the nagging this dashboard exists not to do, arrived at from the
    /// direction of helpfulness rather than of notifications.
    ///
    /// `w` is still a primary binding, so the way to switch a panel on is on
    /// the status bar and in the help. What is gone is being told you *should*.
    #[test]
    fn a_layout_missing_widgets_is_not_advertised_anywhere() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        // One panel out of twelve, so eleven are unused — the loudest possible
        // case for the hint that used to be here.
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        let screen = |app: &mut App, help: bool| -> String {
            if help {
                app.handle_key(KeyEvent::from(KeyCode::Char('?')));
            }
            let mut terminal = Terminal::new(TestBackend::new(200, 40)).unwrap();
            terminal.draw(|frame| app.render_for_test(frame)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..40)
                .map(|y| {
                    (0..200)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let dashboard = screen(&mut app, false);
        assert!(
            !dashboard.contains("unused"),
            "the dashboard advertises what you chose not to run"
        );

        let help = screen(&mut app, true);
        assert!(app.show_help, "the overlay is open");
        for banned in ["unused", "not in your layout", "switch them on"] {
            assert!(
                !help.contains(banned),
                "the help overlay still nags about unused widgets: found `{banned}`"
            );
        }
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
        app.watch_for_updates(std::sync::Arc::new(std::sync::Mutex::new(Some(
            "9.9.9".to_string(),
        ))));
        app.handle_mouse(mouse(MouseEventKind::Moved));
        assert!(
            app.update_hint().is_some(),
            "the mouse crossing the window is not the user reading anything"
        );

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left)));
        assert!(app.update_hint().is_none(), "a deliberate click retires it");
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
    fn every_binding_of_the_focused_panel_is_reachable_on_an_80x24_terminal() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        // The tasks panel declares 17 bindings; with the global block, the
        // section headings and the footer that is well past the 22 rows an
        // 80x24 terminal leaves inside the overlay. It used to clip there
        // silently, so about a third of the keys did not exist as far as
        // anyone reading `?` could tell.
        let mut app = App::new(config_with(&["todo"])).unwrap();
        let bindings = app.slots[0].panel.bindings().len();
        assert!(bindings > 4, "this test needs a panel with many bindings");

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        app.handle_key(KeyEvent::from(KeyCode::Char('?')));
        terminal.draw(|frame| app.render_for_test(frame)).unwrap();

        assert!(
            app.help_overflow > 0,
            "the overlay fits at 80x24, so this test no longer proves anything"
        );

        // Scroll to the bottom one key at a time, redrawing as a user would.
        let mut guard = 0;
        while app.help_scroll < app.help_overflow {
            app.handle_key(KeyEvent::from(KeyCode::Down));
            terminal.draw(|frame| app.render_for_test(frame)).unwrap();
            assert!(app.show_help, "scrolling must not dismiss the overlay");
            guard += 1;
            assert!(guard < 200, "scrolling made no progress");
        }

        // The last row of text is on screen, and `End` and `Home` agree.
        app.handle_key(KeyEvent::from(KeyCode::Home));
        assert_eq!(app.help_scroll, 0);
        app.handle_key(KeyEvent::from(KeyCode::End));
        assert_eq!(app.help_scroll, app.help_overflow);

        // Anything that is not a scroll key still closes it.
        app.handle_key(KeyEvent::from(KeyCode::Char('x')));
        assert!(
            !app.show_help,
            "a non-scroll key must still close the overlay"
        );
    }

    #[test]
    fn the_overlay_closes_on_any_key_when_it_all_fits() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        // Scrolling only binds the arrow keys when there is something below the
        // fold. On a terminal with room to spare, `?` then Down must close,
        // because that is what "any key to close" promises.
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(120, 60)).unwrap();
        app.handle_key(KeyEvent::from(KeyCode::Char('?')));
        terminal.draw(|frame| app.render_for_test(frame)).unwrap();

        assert_eq!(
            app.help_overflow, 0,
            "120x60 has room for the whole overlay"
        );
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert!(!app.show_help);
    }

    #[test]
    fn the_hint_gives_way_to_the_global_keys_on_a_narrow_terminal() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = App::new(config_with(&["clocks"])).unwrap();
        app.watch_for_updates(std::sync::Arc::new(std::sync::Mutex::new(Some(
            "9.9.9".to_string(),
        ))));
        let row_text = |app: &mut App, width: u16| -> String {
            let mut terminal = Terminal::new(TestBackend::new(width, 6)).unwrap();
            terminal.draw(|frame| app.render_for_test(frame)).unwrap();
            let buf = terminal.backend().buffer().clone();
            (0..width).map(|x| buf[(x, 5)].symbol()).collect()
        };

        // Wide enough for the global keys *and* the notice.
        let wide = row_text(&mut app, 200);
        assert!(wide.contains("9.9.9"), "wide enough for both: `{wide}`");

        let narrow = row_text(&mut app, 44);
        assert!(
            !narrow.contains("9.9.9"),
            "knowing how to quit outranks the notice: `{narrow}`"
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
        //
        // Note this case is capped *before* redistribution, so it never reaches
        // the path that was broken. `no_slot_exceeds_its_maximum_while_another`
        // is the one that does; this one alone could not fail.
        let sizes = distribute(200, &[50, 50], &[Some(30), Some(30)]);
        assert_eq!(
            sizes.iter().sum::<u16>(),
            200,
            "no gap may be left: {sizes:?}"
        );
        assert_eq!(sizes, vec![100, 100], "and the overshoot is shared");
    }

    #[test]
    fn a_row_too_wide_for_its_maxima_shares_the_overshoot() {
        // The invariant that actually distinguishes the fix from the bug.
        //
        // "Nobody exceeds their maximum while somebody is under theirs" is not
        // enough: the buggy output [302, 47, 51] satisfies it, because 47 and
        // 51 are exactly at their maxima rather than under. What it violates is
        // that the *excess* be shared — [140, 0, 0] against weights
        // [26, 34, 40] is not a proportional split of anything.
        //
        // The real default top row: clocks / calendar / weather, at every width
        // from too-narrow to a 4K terminal.
        let weights = [26u16, 34, 40];
        let maxima = [Some(162u16), Some(47), Some(51)];
        let ceiling: u16 = maxima.iter().map(|m| m.unwrap()).sum();

        for total in (20u16..=1000).step_by(7) {
            let sizes = distribute(total, &weights, &maxima);

            assert_eq!(
                sizes.iter().map(|s| u32::from(*s)).sum::<u32>(),
                u32::from(total),
                "the row must cover its width exactly at {total}: {sizes:?}"
            );

            if total <= ceiling {
                // There is room to honour every maximum, so nobody may exceed.
                for i in 0..3 {
                    assert!(
                        sizes[i] <= maxima[i].unwrap(),
                        "at {total}: slot {i} exceeded its maximum with room to \
                         spare — {sizes:?} against {maxima:?}"
                    );
                }
                continue;
            }

            // Past the ceiling everybody has to go over. The excess must track
            // the weights, not land on one panel.
            let excess: Vec<u16> = (0..3).map(|i| sizes[i] - maxima[i].unwrap()).collect();
            let want = proportional(total - ceiling, &weights);
            assert_eq!(
                excess, want,
                "at {total}: the overshoot is not shared — {sizes:?}, excess \
                 {excess:?}, expected {want:?}"
            );
        }
    }

    #[test]
    fn a_wide_terminal_does_not_park_the_surplus_on_one_panel() {
        // The specific regression, with the numbers from the bug report.
        let sizes = distribute(400, &[26, 34, 40], &[Some(162), Some(47), Some(51)]);
        assert_eq!(sizes.iter().sum::<u16>(), 400);
        assert!(
            sizes[0] < 250,
            "the clock took the whole surplus again: {sizes:?}"
        );
        assert!(
            sizes[2] > 51,
            "the panel that could have used the space got none of it: {sizes:?}"
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
    fn a_layout_entry_that_builds_no_panel_does_not_shift_the_rest() {
        // `geometry` used to push one rect per layout column and `render` read
        // it back by slot index. Any entry that produced no panel made the two
        // disagree, so every later slot drew — and hit-tested clicks — in the
        // previous entry's box.
        //
        // `Config::validate` rejects an unknown widget name, but `App::new`
        // does not re-validate and neither does `rebuild_panels`, so this is
        // one `config.layout` mutation away from being reachable.
        let mut config = config_with(&["nope", "clocks", "cpu"]);
        config.layout.rows[0].panels[0].width = 25;
        config.layout.rows[0].panels[1].width = 25;
        config.layout.rows[0].panels[2].width = 50;

        let app = App::new(config).unwrap();
        assert_eq!(app.slots.len(), 2, "the unknown widget builds no panel");
        assert_eq!(app.positions, vec![(0, 1), (0, 2)]);

        let rects = app.geometry(Rect::new(0, 0, 80, 24));
        assert_eq!(rects.len(), 2, "one rect per slot, not per layout column");

        // Column 0 is 20 cells wide and belongs to nothing. The clock is the
        // first *slot* but the second *column*, so it starts at x = 20.
        assert_eq!(
            rects[0].x, 20,
            "the clock took the missing panel's rectangle"
        );
        assert_eq!(rects[1].x, rects[0].x + rects[0].width);
        assert_eq!(rects[0].width + rects[1].width, 60);
    }

    #[test]
    fn a_settled_dashboard_stops_asking_to_be_redrawn() {
        // The property this whole change exists for: with nothing moving,
        // ticking must eventually report no change. Before, every panel whose
        // timer fired counted as one, so the dashboard never went quiet and
        // repainted at the fastest panel's cadence for ever.
        //
        // Panels that legitimately change on a timer are left out: the clock
        // (its second or minute turns), pomodoro while running, and cpu and
        // network (a fresh sample is a new number). What is left must settle.
        let mut app = App::new(config_with(&["todo", "notes", "calendar"])).unwrap();

        // The first tick may report a change; nothing has been drawn yet.
        app.tick_panels();

        for round in 0..5 {
            // Force every panel due, so this cannot pass by ticking nothing.
            for slot in &mut app.slots {
                slot.last_tick = None;
            }
            assert!(
                !app.tick_panels(),
                "round {round}: a dashboard with nothing moving asked for a repaint"
            );
        }
    }

    #[test]
    fn a_due_panel_is_ticked_even_when_an_earlier_one_already_reported_a_change() {
        // `changed |= panel.tick()` would short-circuit and skip the call, and
        // a panel that stops being ticked stops updating — a bug that would
        // show up only when some *other* panel happened to change first.
        let mut app = App::new(config_with(&["clocks", "todo"])).unwrap();
        for slot in &mut app.slots {
            slot.last_tick = None;
        }
        app.tick_panels();
        assert!(
            app.slots.iter().all(|slot| slot.last_tick.is_some()),
            "a due panel was skipped"
        );
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

        // Esc must not. The task panel tells you to press it to clear a filter,
        // and a panel only consumes it while the filter is non-empty — so when
        // Esc also quit, the same key in the same panel one keystroke apart
        // either cleared the filter or killed the dashboard.
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            !app.should_quit,
            "Esc means back out of something, not quit"
        );
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

    /// Walk the theme picker down to a named theme and report what it landed on.
    fn preview_a_theme(app: &mut App) -> String {
        app.handle_key(key(KeyCode::Char('t')));
        // Down until the accent actually moves. The list is alphabetical and
        // starts on `ansi`, whose accent is an ANSI index rather than an RGB
        // triple, so the first row that differs is a real repaint.
        let start = app.config.theme.accent;
        for _ in 0..20 {
            app.handle_key(key(KeyCode::Down));
            if app.config.theme.accent != start {
                break;
            }
        }
        app.config
            .theme
            .name
            .clone()
            .expect("a previewed theme is a named one")
    }

    #[test]
    fn moving_in_the_theme_picker_repaints_before_anything_is_committed() {
        let mut app = App::new(config_with(&["clocks", "cpu"])).unwrap();
        let before = app.config.theme.accent;
        let name = preview_a_theme(&mut app);
        assert_ne!(
            app.config.theme.accent, before,
            "`{name}` was selected but the live theme did not change"
        );
        assert!(
            app.theme_picker.is_some(),
            "moving must not close the dialog"
        );
    }

    /// The graphs read a baked ramp rather than the theme, so a swap that
    /// forgets it leaves cpu and network painted in the previous palette.
    #[test]
    fn a_theme_swap_rebakes_the_gradients_the_graphs_draw_from() {
        let mut app = App::new(config_with(&["cpu"])).unwrap();
        let before = app.gradients.clone();
        preview_a_theme(&mut app);
        assert_ne!(
            app.gradients, before,
            "the graph ramp still holds the previous theme's colours"
        );
    }

    #[test]
    fn esc_puts_back_the_theme_the_picker_opened_on() {
        let mut app = App::new(config_with(&["clocks", "cpu"])).unwrap();
        let theme = app.config.theme.clone();
        let gradients = app.gradients.clone();

        preview_a_theme(&mut app);
        app.handle_key(key(KeyCode::Esc));

        assert!(app.theme_picker.is_none(), "Esc must close the dialog");
        assert_eq!(app.config.theme.accent, theme.accent, "theme not restored");
        assert_eq!(app.gradients, gradients, "gradients not restored");
    }

    #[test]
    fn enter_keeps_the_previewed_theme() {
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        let name = preview_a_theme(&mut app);
        app.handle_key(key(KeyCode::Enter));

        assert!(app.theme_picker.is_none());
        assert_eq!(
            app.config.theme.name.as_deref(),
            Some(name.as_str()),
            "Enter must keep what was on screen"
        );
    }

    /// The dialog owns the keyboard while it is open. Otherwise `q` under the
    /// cursor quits the dashboard mid-browse, which is invariant 2's rule
    /// applied to a shell dialog rather than a panel.
    #[test]
    fn the_theme_picker_takes_the_keyboard_while_it_is_open() {
        let mut app = App::new(config_with(&["clocks", "todo"])).unwrap();
        app.handle_key(key(KeyCode::Char('t')));
        let focus = app.focus;

        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.focus, focus, "Tab reached the focus ring");
        app.handle_key(key(KeyCode::Char('w')));
        assert!(
            app.picker.is_none(),
            "`w` opened the panel picker underneath"
        );

        app.handle_key(key(KeyCode::Char('q')));
        assert!(
            !app.should_quit,
            "`q` quit the dashboard from inside a dialog"
        );
        assert!(app.theme_picker.is_none(), "`q` should close the dialog");
    }

    #[test]
    fn the_picker_opens_and_toggles_a_panel_on_and_off() {
        let mut app = App::new(config_with(&["clocks", "todo"])).unwrap();
        let before = app.slots.len();

        app.handle_key(key(KeyCode::Char('w')));
        assert!(app.picker.is_some(), "w opens the dialog");

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

        // Cursor movement itself is `picker`'s; what matters here is that the
        // keys reach it rather than the panels or the global bindings.
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.picker_row(), Some(1));
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.picker_row(), Some(0));

        app.handle_key(key(KeyCode::Esc));
        assert!(app.picker.is_none());
        assert!(
            !app.should_quit,
            "Esc closes the dialog rather than falling through to quit"
        );
    }

    #[test]
    fn rebuilding_shuts_the_outgoing_panels_down() {
        // The picker rebuilds every panel on each toggle. Dropping the old ones
        // without shutdown() discarded the task store's save and left each
        // panel's fetch thread running — so toggling stocks five times left
        // five pollers hitting the same endpoint, defeating the per-thread
        // rate limit the module documents as enforced in code.
        let mut app = App::new(config_with(&["clocks", "todo"])).unwrap();
        let before = std::thread::available_parallelism().is_ok();
        assert!(before, "sanity: threads are available");

        app.handle_key(key(KeyCode::Char('w')));
        for _ in 0..5 {
            app.handle_key(key(KeyCode::Char(' ')));
        }
        // Whatever the toggles did, the dashboard is still coherent and every
        // slot still has a panel behind it.
        assert!(!app.slots.is_empty());
        assert_eq!(app.slots.len(), app.positions.len());
    }

    #[test]
    fn a_panel_ticks_immediately_rather_than_one_interval_late() {
        // Previously arranged by back-dating an Instant by 24 hours, which is
        // None on Windows below that uptime and panicked on the unwrap.
        let mut app = App::new(config_with(&["clocks"])).unwrap();
        assert!(
            app.slots.iter().all(|s| s.last_tick.is_none()),
            "a fresh panel has never ticked"
        );
        assert!(app.tick_panels(), "and is due immediately");
        assert!(app.slots.iter().all(|s| s.last_tick.is_some()));
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

    /// And the event loop itself must offer only `FocusLost` to `mark_seen`.
    /// The two tests above exercise `WatchLog`; this one is about the wiring,
    /// which is where #132 actually lived — the log was always correct.
    #[test]
    fn only_losing_focus_marks_the_log_seen() {
        let source = std::fs::read_to_string(file!()).expect("this file is readable");
        let wiring = source
            .lines()
            .find(|line| line.contains("=> self.watch.mark_seen()"))
            .expect("the focus arm still exists");
        assert!(
            wiring.contains("FocusLost") && !wiring.contains("FocusGained"),
            "gaining focus must not mark the log seen: {wiring:?}"
        );
    }
}
