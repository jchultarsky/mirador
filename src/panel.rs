//! The [`Panel`] trait: the extension seam every dashboard widget implements.
//!
//! A panel owns its own state and refresh cadence. The application shell is
//! responsible only for layout, focus, frames and dispatching ticks and key
//! events; it knows nothing about what any individual panel displays.

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
    fn refresh_interval(&self) -> Duration {
        Duration::from_secs(1)
    }

    /// Advance any time-based state. Must not block: panels needing network or
    /// disk I/O do it on a background thread and poll the result here.
    fn tick(&mut self) {}

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
    fn captures_input(&self) -> bool {
        false
    }

    /// Contribute any preference this panel wants remembered across restarts.
    ///
    /// Write a setting only when it differs from the value the panel was built
    /// with. A panel that reports everything pins the config's own values into
    /// the state file the first time any *other* preference moves, and editing
    /// the config then silently stops working — which is the failure this whole
    /// mechanism exists to avoid. Comparing against the seed is what keeps
    /// "changed by the user" and "merely configured" apart.
    ///
    /// There is no matching load hook on purpose. Remembered preferences are
    /// applied to the config before any panel is built, so a panel sees the
    /// right values in its constructor and needs no restoring code at all.
    fn remember(&self, _state: &mut crate::state::UiState) {}

    /// Called once before the terminal is torn down, so panels can flush state.
    fn shutdown(&mut self) {}
}
