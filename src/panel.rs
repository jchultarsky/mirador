//! The [`Panel`] trait: the seam every dashboard widget goes through.
//!
//! **This is an in-tree seam, not a plugin API.** mirador is a binary crate
//! with no library target, so nothing outside it can name this trait; and even
//! if it could, `widgets::build` dispatches on a fixed `match` over
//! `WIDGET_NAMES` and each widget's settings are a field on `Config`. Adding a
//! widget is a pull request, and `CONTRIBUTING.md` lists the five places to
//! touch. A real plugin story needs three things this does not have: a runtime
//! registry instead of that `match`, per-widget config carried as an
//! unparsed `toml::Value` so a widget can own its own schema, and a library
//! target with a deliberate public surface. Adding only the last of those
//! would make the trait nameable without making it usable, which is worse than
//! the honest version.
//!
//! What the seam does buy, in-tree, is real: a panel owns its own state and
//! refresh cadence. The application shell is
//! responsible only for layout, focus, frames and dispatching ticks and key
//! events; it knows nothing about what any individual panel displays.
//!
//! # Why the description is six methods and not one
//!
//! `title`, `counter`, `bindings`, `max_width`, `max_height` and
//! `refresh_interval` are all "describe yourself", and folding them into one
//! `describe() -> PanelInfo` would take the trait from thirteen methods to
//! eight. It would also be slower and less useful, because the six are asked
//! for at different times and at very different rates: `max_width` and
//! `max_height` during layout, `refresh_interval` on the tick, `bindings` in
//! three separate places, and `title` and `counter` on **every frame** — the
//! last two from the shell's render loop, where nothing guards them.
//!
//! A single `describe()` would compute all six whenever any one was wanted,
//! and return an owned struct carrying a `String` title and an
//! `Option<String>` counter — so the cheapest question would allocate twice,
//! sixty times a second, per panel. The trait is wide because the shell's
//! questions are genuinely separate.

use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;

use crate::frame::Binding;
use crate::theme::{Gradients, Theme};

/// Whether a panel handled an input event, or wants it to fall through to the
/// application's global bindings.
///
/// Shared by [`Panel::handle_key`] and [`Panel::handle_mouse`]: the two differ
/// in what they receive, not in how the shell reacts to the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// The panel used the event; the app should not act on it.
    Consumed,
    /// The panel did not use the event; the app may apply a global binding.
    Ignored,
}

/// Everything a panel needs at draw time that it does not own itself.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext<'a> {
    pub theme: &'a Theme,
    /// Colour ramps, baked once at startup.
    pub gradients: &'a Gradients,
    /// Whether this panel currently has keyboard focus.
    pub focused: bool,
    /// What has happened since mirador started.
    ///
    /// Handed to every panel rather than only the watch log, because it is
    /// read-only from here and threading it to exactly one panel would mean a
    /// second context type. Nothing else reads it today.
    pub watch: &'a crate::watch::WatchLog,
}

/// Something that will get worse if it is ignored.
///
/// Deliberately not "something is wrong". Panels already show what is wrong in
/// their own frames — an overdue task is red, a stale reading says its age, a
/// failing fetch says so — and eight of the twelve do it today. Repeating all
/// of that in one line would produce a signal that is lit permanently, and a
/// permanently lit alarm is furniture: you stop seeing it inside a week, which
/// is the unread-badge failure reached from a different direction.
///
/// The bar is therefore **will this deteriorate if nobody acts in the next few
/// minutes**. A save that is failing loses work. An event about to start is
/// missed. A layout that did not persist is gone at the next launch. An overdue
/// task from three days ago is none of those: it is notable, it is already red,
/// and it will be exactly as overdue in an hour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub severity: Severity,
    /// One line, naming the thing rather than the panel, in the terms someone
    /// would use to describe the problem to somebody else.
    pub text: String,
}

/// How bad, for picking one alert out of several.
///
/// Two levels because two is all the distinction that is needed: something is
/// about to happen, or something has broken. More would be a taxonomy nobody
/// consults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// A deadline approaching. Missing it is bad and survivable.
    Soon,
    /// Something is failing now. Ranked above `Soon` because what it costs —
    /// usually work you cannot retype — does not come back, where a missed
    /// meeting at least leaves you the rest of the day.
    Failing,
}

impl Alert {
    pub fn soon(text: impl Into<String>) -> Self {
        Self {
            severity: Severity::Soon,
            text: text.into(),
        }
    }

    pub fn failing(text: impl Into<String>) -> Self {
        Self {
            severity: Severity::Failing,
            text: text.into(),
        }
    }
}

/// A single dashboard widget.
///
/// Implementors are stored as `Box<dyn Panel>`, so the trait is deliberately
/// object safe.
pub trait Panel {
    /// Short name shown in the panel's frame.
    fn title(&self) -> String;

    /// Optional status for the top-right of the frame, e.g. `3 open`.
    ///
    /// Keep it to a few characters: it shares the border with the title, and a
    /// long counter squeezes the frame rather than wrapping.
    fn counter(&self) -> Option<String> {
        None
    }

    /// Key bindings, declared once and reused for the border hint, the status
    /// bar and the help overlay. Order matters: the border shows as many
    /// `primary` bindings as fit, in order.
    fn bindings(&self) -> &'static [Binding] {
        &[]
    }

    /// The width past which this panel can do nothing with more space, if it
    /// has one. Includes the frame, so it is a whole-panel figure.
    ///
    /// A clock cannot use a hundred columns and a calendar cannot use more than
    /// its months need. Declaring the limit lets the shell give the surplus to
    /// a neighbour that will fill it, instead of leaving a panel sitting in
    /// space it cannot use while the list next door runs out.
    ///
    /// `None` means "whatever is going", which is right for anything that
    /// scrolls or scales — lists, graphs, prose. Only return a figure when more
    /// space genuinely buys the reader nothing.
    fn max_width(&self) -> Option<u16> {
        None
    }

    /// The height past which this panel can do nothing with more space.
    ///
    /// A row of the grid is only shortened when *every* panel in it is bounded,
    /// since they share the height between them.
    fn max_height(&self) -> Option<u16> {
        None
    }

    /// How often [`Panel::tick`] should be called.
    ///
    /// This is how often the panel wants to *look*, which is not the same as
    /// how often it expects to have something new to show — a clock with
    /// seconds ticks four times a second and changes once. Say how often you
    /// need to check, and answer the second question from [`Panel::tick`].
    fn refresh_interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    /// Advance any time-based state, and report whether anything **a viewer
    /// could see** changed.
    ///
    /// Must not block: panels needing network or disk I/O do it on a background
    /// thread and poll the result here.
    ///
    /// The return value drives the redraw, and the distinction it draws is the
    /// whole point. `true` means the next frame will differ from the last one.
    /// It does *not* mean "my timer elapsed", "I re-read the clock", or "I
    /// looked and there was nothing new" — a panel that says `true` on every
    /// tick makes the shell repaint the entire dashboard at that panel's
    /// cadence, whether or not a single cell would change.
    ///
    /// That was measurably the largest thing the program did. With the default
    /// layout the clock panel drove a 250ms tick and reported nothing at all,
    /// so the dashboard repainted four times a second, for ever — and with
    /// `show_seconds = false`, where the visible content changes once a minute,
    /// idle CPU was *identical*. 240 repaints per visible change.
    ///
    /// The cheap way to answer honestly is usually to compare what you are
    /// about to render against what you rendered last: the formatted string,
    /// the date, a generation counter bumped by your fetch thread. If a panel
    /// truly cannot tell, `true` is correct and costs a repaint — but it should
    /// then say why in a comment, because the next reader will assume it was
    /// laziness.
    fn tick(&mut self) -> bool {
        false
    }

    /// Notable things that have happened since this was last called.
    ///
    /// Drained every tick and handed to the watch log. Distinct from [`tick`]
    /// on purpose: `tick` answers "did the screen change", which is true
    /// constantly, and this answers "did something happen that a reader would
    /// want to know about", which is true almost never.
    ///
    /// The bar is deliberately high. An event is something that happened **to**
    /// the user rather than because of them, and that they would want to know
    /// even if they never looked at this panel. A price moving, a graph
    /// scrolling and a clock ticking are all changes and none of them qualify;
    /// nor does anything the user did themselves, because they were there.
    ///
    /// Returning something every tick would make the log unreadable, which
    /// costs the feature rather than the panel: a log nobody can scan is worse
    /// than no log.
    ///
    /// [`tick`]: Panel::tick
    fn events(&mut self) -> Vec<crate::watch::Event> {
        Vec::new()
    }

    /// A dialog this panel wants drawn over the whole screen.
    ///
    /// Returned rather than drawn in [`render`] because the shell draws panels
    /// in order, so anything a panel paints outside its own rectangle is
    /// immediately painted over by the panels after it. The first version of
    /// the zone picker did exactly that and came out interleaved with the task
    /// list. The shell draws these last, alongside the help overlay and the
    /// panel picker, which are screen-wide for the same reason.
    ///
    /// A panel with a dialog open also returns `true` from
    /// [`captures_input`](Panel::captures_input), so at most one is ever open.
    ///
    /// [`render`]: Panel::render
    fn overlay(&self) -> Option<&crate::prompt::Prompt> {
        None
    }

    /// Something about this panel that will get worse if it is ignored.
    ///
    /// The present-tense sibling of [`events`]: that one reports a moment that
    /// has passed, this one a condition that is still true. Both ask what is
    /// notable; they differ in tense, which is why they are separate hooks
    /// rather than one.
    ///
    /// Called every frame, so it must be cheap and must not allocate when there
    /// is nothing to say — which is nearly always. Returning `Some` for an
    /// ordinary state is how this feature stops working: see [`Alert`].
    ///
    /// [`events`]: Panel::events
    fn alert(&self) -> Option<crate::panel::Alert> {
        None
    }

    /// Draw the panel's contents. `area` is already inside the frame and its
    /// padding.
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>);

    /// Handle a key event while focused.
    fn handle_key(&mut self, _key: KeyEvent) -> KeyOutcome {
        KeyOutcome::Ignored
    }

    /// Handle a mouse event landing inside this panel.
    ///
    /// `area` is the panel's interior — the same rectangle [`Panel::render`]
    /// was last given — so a panel can hit-test its own rows without knowing
    /// where the shell put it. `event.column` and `event.row` are absolute
    /// terminal coordinates, so subtract `area.x`/`area.y` to get a local
    /// offset.
    ///
    /// Unlike [`Panel::handle_key`], this is called for the panel under the
    /// *cursor*, which is not necessarily the focused one: a scroll wheel
    /// should move the list it is pointing at without stealing focus from the
    /// panel the keyboard is talking to.
    fn handle_mouse(&mut self, _event: MouseEvent, _area: Rect) -> KeyOutcome {
        KeyOutcome::Ignored
    }

    /// When true, the panel is in a text-entry or modal state and the app
    /// suppresses *all* global bindings, including quit, so that typing a `q`
    /// into a form does not exit the dashboard.
    ///
    /// This looks like it should be a variant of [`KeyOutcome`] — a panel that
    /// swallowed the key could say so on the way out, and the trait would lose
    /// a method. It cannot be, because two of the three places the shell asks
    /// have no key outcome to read:
    ///
    /// - The resize keys are claimed by the shell *before* the panel is
    ///   offered anything, so `Ctrl+Left` does not scroll the calendar. A panel
    ///   mid-form still has to veto that, and it has not been called yet.
    /// - A mouse click must not pull focus out of a half-typed task. There is
    ///   no key event anywhere in that path.
    ///
    /// The third is the one that looks foldable and is not: the veto applies
    /// when the panel returned [`KeyOutcome::Ignored`]. A form ignores `q`
    /// because it is not one of its keys, and `q` must still not quit.
    fn captures_input(&self) -> bool {
        false
    }

    /// Contribute any preference this panel wants remembered across restarts.
    ///
    /// Report the panel's *current* values, unconditionally. Deciding whether a
    /// value is worth writing is not a panel's job: the shell compares the whole
    /// set against the config as it was read, before any remembered preference
    /// was folded in, and writes only the difference.
    ///
    /// Do not filter here by comparing against the value the panel was built
    /// with. That was the previous design and it made a preference impossible to
    /// retract: panels are built *after* remembered preferences are applied, so
    /// once a setting had been recorded the panel was constructed from the
    /// recorded value and could never see it as equal to the config's again.
    /// Setting it back to the config's value then wrote nothing, which left the
    /// old entry standing for ever. See `CLAUDE.md` invariant 17.
    ///
    /// There is no matching load hook on purpose. Remembered preferences are
    /// applied to the config before any panel is built, so a panel sees the
    /// right values in its constructor and needs no restoring code at all.
    fn remember(&self, _state: &mut crate::state::UiState) {}

    /// Called once before the terminal is torn down, so panels can flush state.
    fn shutdown(&mut self) {}
}

/// Render a duration the way a person would say it.
///
/// Used wherever a panel shows data it fetched rather than computed, which is
/// the only honest way to present a reading that may have stopped updating.
pub fn describe_age(age: Duration) -> String {
    let minutes = age.as_secs() / 60;
    if minutes < 60 {
        return format!("{minutes}m old");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h old");
    }
    format!("{}d old", hours / 24)
}
