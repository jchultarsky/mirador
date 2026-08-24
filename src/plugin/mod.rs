//! Out-of-process panels over a small, versioned JSON-lines protocol.
//!
//! Mirador does not discover plugins, embed an interpreter, or load dynamic
//! libraries. A process exists only when the config explicitly declares its
//! command *and* the layout places its id. The process publishes complete
//! render snapshots; Mirador remains the only owner of the real terminal and
//! the only renderer touching ratatui.

use std::sync::mpsc::TrySendError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use serde::{Deserialize, Serialize};

use crate::config::PluginConfig;
use crate::frame::Binding;
use crate::panel::{KeyOutcome, Panel, RenderContext};

mod process;

use process::{Phase, Runtime, Shared, spawn_process};

pub const PROTOCOL_VERSION: u16 = 1;
const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_WATCH_TEXT_BYTES: usize = 1024;
const MAX_RETAINED_FRAME_TEXT_BYTES: usize = 1024 * 1024;
const MAX_FRAME_LINES: usize = 4096;
const MAX_FRAME_SPANS: usize = 16_384;
const MAX_BINDINGS: usize = 64;
const MAX_INPUT_KEYS: usize = 128;
const MAX_TITLE_BYTES: usize = 256;
const MAX_COUNTER_BYTES: usize = 128;
const MAX_BINDING_KEY_BYTES: usize = 64;
const MAX_BINDING_ACTION_BYTES: usize = 256;
const MAX_ERROR_BYTES: usize = 1024;
const MAX_COLOR_BYTES: usize = 64;
/// A visible cell may carry combining marks, but an unbounded run of them must
/// not make drawing one row proportional to an entire retained frame.
const RENDER_BYTES_PER_CELL: usize = 16;
const DEFAULT_REFRESH: Duration = Duration::from_millis(33);
const INPUT_BARRIER_REFRESHES: u32 = 3;
const MIN_INPUT_BARRIER: Duration = Duration::from_millis(100);
const MAX_INPUT_BARRIER: Duration = Duration::from_secs(1);
const INPUT_BARRIER_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostMessage {
    Hello {
        protocol: u16,
        host_version: &'static str,
        plugin: String,
        config: serde_json::Value,
        cwd: String,
    },
    Resize {
        columns: u16,
        rows: u16,
    },
    Focus {
        focused: bool,
    },
    Key {
        key: String,
        code: String,
        text: Option<String>,
        modifiers: Vec<String>,
    },
    Paste {
        text: String,
    },
    Mouse {
        kind: String,
        button: Option<String>,
        column: u16,
        row: u16,
        modifiers: Vec<String>,
    },
    Tick,
    Shutdown,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PluginMessage {
    Ready {
        protocol: u16,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        refresh_ms: Option<u64>,
    },
    Frame {
        revision: u64,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        counter: Option<String>,
        #[serde(default)]
        lines: Vec<WireLine>,
        #[serde(default)]
        bindings: Vec<WireBinding>,
        #[serde(default)]
        input: InputPolicy,
        #[serde(default)]
        cursor: Option<WireCursor>,
    },
    Error {
        message: String,
        #[serde(default)]
        fatal: bool,
    },
    Watch {
        text: String,
    },
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct WireLine {
    #[serde(default)]
    spans: Vec<WireSpan>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
// These are independent terminal style attributes on the wire, not states of
// one machine; combining them would make the protocol less direct.
#[allow(clippy::struct_excessive_bools)]
struct WireSpan {
    text: String,
    #[serde(default)]
    fg: Option<String>,
    #[serde(default)]
    bg: Option<String>,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    dim: bool,
    #[serde(default)]
    italic: bool,
    #[serde(default)]
    underlined: bool,
    #[serde(default)]
    reversed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBinding {
    key: String,
    action: String,
    #[serde(default)]
    primary: bool,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
// Each flag grants a distinct input class. A bitset would leak a Rust encoding
// into a language-neutral JSON protocol.
#[allow(clippy::struct_excessive_bools)]
struct InputPolicy {
    /// Consume every key while focused.
    capture: bool,
    /// Canonical keys to consume while not capturing, e.g. `Enter` or `r`.
    keys: Vec<String>,
    paste: bool,
    mouse: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCursor {
    column: u16,
    row: u16,
    #[serde(default = "yes")]
    visible: bool,
}

const fn yes() -> bool {
    true
}

#[derive(Debug, Clone)]
struct WireFrame {
    revision: u64,
    title: Option<String>,
    counter: Option<String>,
    lines: Vec<WireLine>,
    bindings: Vec<WireBinding>,
    input: InputPolicy,
    cursor: Option<WireCursor>,
}

#[derive(Debug, Clone, Copy)]
struct InputBarrier {
    frame_sequence: u64,
    expires_at: Instant,
}

/// Adapter from one explicitly configured process to Mirador's private panel
/// trait. Process failures are panel state, never application failures.
pub struct PluginPanel {
    spec: PluginConfig,
    runtime: Option<Runtime>,
    shared: Arc<Mutex<Shared>>,
    seen_generation: u64,
    seen_frame_sequence: u64,
    phase: Phase,
    title: String,
    refresh: Duration,
    frame: Option<Arc<WireFrame>>,
    bindings: Vec<Binding>,
    policy: InputPolicy,
    notice: Option<String>,
    /// A passive key action awaiting any accepted frame. The short deadline is
    /// what makes this a race barrier rather than an unbounded modal state.
    input_barrier: Option<InputBarrier>,
    last_size: Option<(u16, u16)>,
    last_focus: Option<bool>,
}

impl PluginPanel {
    pub fn new(spec: PluginConfig) -> Self {
        let id = spec.id.clone();
        let mut panel = Self {
            spec,
            runtime: None,
            shared: Arc::new(Mutex::new(Shared::starting())),
            seen_generation: u64::MAX,
            seen_frame_sequence: 0,
            phase: Phase::Starting,
            title: id,
            refresh: DEFAULT_REFRESH,
            frame: None,
            bindings: Vec::new(),
            policy: InputPolicy::default(),
            notice: None,
            input_barrier: None,
            last_size: None,
            last_focus: None,
        };
        panel.start();
        panel.sync();
        panel
    }

    fn start(&mut self) {
        self.stop();
        self.shared = Arc::new(Mutex::new(Shared::starting()));
        self.seen_generation = u64::MAX;
        self.seen_frame_sequence = 0;
        self.phase = Phase::Starting;
        self.frame = None;
        self.bindings.clear();
        self.policy = InputPolicy::default();
        self.notice = None;
        self.input_barrier = None;
        self.last_size = None;
        self.last_focus = None;

        match spawn_process(&self.spec, &self.shared) {
            Ok(runtime) => self.runtime = Some(runtime),
            Err(error) => self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .fail(format!("could not start plugin: {error}")),
        }
    }

    fn stop(&mut self) {
        if let Some(mut runtime) = self.runtime.take() {
            let _ = runtime.shutdown();
        }
    }

    fn sync(&mut self) -> bool {
        let shared = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let frame_sequence = shared.frame_sequence;
        if shared.generation == self.seen_generation && frame_sequence == self.seen_frame_sequence {
            return false;
        }

        if shared.generation == self.seen_generation {
            drop(shared);
            if self
                .input_barrier
                .is_some_and(|barrier| barrier.frame_sequence != frame_sequence)
            {
                self.input_barrier = None;
            }
            self.seen_frame_sequence = frame_sequence;
            return false;
        }

        self.seen_generation = shared.generation;
        self.phase = shared.phase.clone();
        self.refresh = shared.refresh;
        self.notice.clone_from(&shared.notice);
        let negotiated_title = shared.title.clone().unwrap_or_else(|| self.spec.id.clone());
        let next = if matches!(self.phase, Phase::Exited(_) | Phase::Failed(_)) {
            None
        } else {
            shared.frame.clone()
        };
        drop(shared);

        if self
            .input_barrier
            .is_some_and(|barrier| barrier.frame_sequence != frame_sequence)
        {
            self.input_barrier = None;
        }
        self.seen_frame_sequence = frame_sequence;

        self.title = next
            .as_ref()
            .and_then(|frame| frame.title.clone())
            .unwrap_or(negotiated_title);

        if let Some(frame) = &next {
            self.policy = frame.input.clone();
            if matches!(self.phase, Phase::Exited(_) | Phase::Failed(_)) {
                self.input_barrier = None;
            }
            self.bindings = frame
                .bindings
                .iter()
                .map(|binding| {
                    Binding::owned(binding.key.clone(), binding.action.clone(), binding.primary)
                })
                .collect();
        } else {
            self.policy = InputPolicy::default();
            self.input_barrier = None;
            self.bindings.clear();
        }
        if matches!(self.phase, Phase::Exited(_) | Phase::Failed(_)) {
            self.bindings.push(Binding::primary("r", "restart"));
            // Closing the last host sender lets the stdin worker leave too.
            // A completed child must not leave one parked thread behind until
            // the panel is restarted or the whole application exits.
            self.runtime = None;
        }
        self.frame = next;
        true
    }

    fn send(&mut self, message: HostMessage) -> bool {
        if !matches!(self.phase, Phase::Running) {
            return false;
        }
        let Some(runtime) = &self.runtime else {
            return false;
        };
        match runtime.send(message) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                let mut shared = self
                    .shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if shared.notice.as_deref() != Some("plugin input queue is full") {
                    shared.notice = Some("plugin input queue is full".into());
                    shared.changed();
                }
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    fn begin_input_barrier(&mut self) {
        if self.policy.capture || self.input_barrier.is_some() {
            return;
        }
        let timeout = input_barrier_timeout(self.refresh);
        self.input_barrier = Some(InputBarrier {
            frame_sequence: self.seen_frame_sequence,
            expires_at: Instant::now() + timeout,
        });
    }

    fn input_barrier_active(&self) -> bool {
        self.input_barrier
            .is_some_and(|barrier| Instant::now() < barrier.expires_at)
    }

    fn expire_input_barrier(&mut self) -> bool {
        let Some(barrier) = self.input_barrier else {
            return false;
        };
        if Instant::now() < barrier.expires_at {
            return false;
        }

        self.input_barrier = None;
        let timeout = input_barrier_timeout(self.refresh);
        let message = format!(
            "plugin did not acknowledge input with a frame within {} ms",
            timeout.as_millis()
        );
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .fail(message);
        if let Some(runtime) = &self.runtime {
            runtime.abort();
        }
        self.sync()
    }

    fn status_lines(
        &self,
        theme: &crate::theme::Theme,
        width: u16,
        height: u16,
    ) -> Vec<Line<'static>> {
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let (headline, detail) = match &self.phase {
            Phase::Starting => ("starting plugin".to_string(), None),
            Phase::Running => ("waiting for first frame".to_string(), None),
            Phase::Stopping => ("stopping plugin".to_string(), None),
            Phase::Exited(status) => (
                "plugin exited".to_string(),
                Some(format!("{status}; press r to restart")),
            ),
            Phase::Failed(error) => (
                "plugin failed".to_string(),
                Some(format!("{error}; press r to retry")),
            ),
        };
        let mut lines = Vec::new();
        push_status_text(
            &mut lines,
            &headline,
            Style::default().fg(theme.muted),
            width,
            height,
        );
        if let Some(detail) = detail {
            push_status_text(
                &mut lines,
                &detail,
                Style::default().fg(theme.error),
                width,
                height,
            );
        }
        let shared = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(notice) = &shared.notice {
            push_status_text(
                &mut lines,
                notice,
                Style::default().fg(theme.warning),
                width,
                height,
            );
        }
        if !shared.stderr.trim().is_empty() && lines.len() < usize::from(height) {
            lines.push(Line::from(""));
            for line in shared.stderr.lines().take(3) {
                push_status_text(
                    &mut lines,
                    line,
                    Style::default().fg(theme.muted),
                    width,
                    height,
                );
            }
        }
        lines.truncate(usize::from(height));
        lines
    }
}

impl Panel for PluginPanel {
    fn title(&self) -> String {
        format!("{} · {}", crate::glyphs::utility("external"), self.title)
    }

    fn counter(&self) -> Option<String> {
        match &self.phase {
            Phase::Starting => Some("starting".into()),
            Phase::Exited(_) => Some("exited".into()),
            Phase::Failed(_) => Some("failed".into()),
            Phase::Running | Phase::Stopping => {
                self.frame.as_ref().and_then(|frame| frame.counter.clone())
            }
        }
    }

    fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    fn refresh_interval(&self) -> Duration {
        self.input_barrier.map_or(self.refresh, |barrier| {
            let until_deadline = barrier
                .expires_at
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1));
            self.refresh.min(INPUT_BARRIER_POLL).min(until_deadline)
        })
    }

    fn tick(&mut self) -> bool {
        let changed = self.expire_input_barrier() || self.sync();
        let _ = self.send(HostMessage::Tick);
        changed
    }

    fn events(&mut self) -> Vec<crate::watch::Event> {
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        shared
            .watch
            .drain(..)
            .map(|text| crate::watch::Event::new(self.spec.id.clone(), text))
            .collect()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        self.expire_input_barrier();
        self.sync();
        let size = (area.width.max(1), area.height.max(1));
        if self.last_size != Some(size)
            && self.send(HostMessage::Resize {
                columns: size.0,
                rows: size.1,
            })
        {
            self.last_size = Some(size);
        }
        if self.last_focus != Some(ctx.focused)
            && self.send(HostMessage::Focus {
                focused: ctx.focused,
            })
        {
            self.last_focus = Some(ctx.focused);
        }

        let Some(snapshot) = &self.frame else {
            frame.render_widget(
                Paragraph::new(self.status_lines(ctx.theme, area.width, area.height)),
                area,
            );
            return;
        };

        let lines = wrapped_wire_lines(&snapshot.lines, area.width, area.height, ctx.theme);
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().fg(ctx.theme.text)),
            area,
        );

        let (notice, notice_color) = if let Some(notice) = self.notice.as_deref() {
            (Some(notice), ctx.theme.warning)
        } else {
            (
                match &self.phase {
                    Phase::Exited(status) => Some(status.as_str()),
                    Phase::Failed(error) => Some(error.as_str()),
                    Phase::Starting | Phase::Running | Phase::Stopping => None,
                },
                ctx.theme.error,
            )
        };
        if let Some(notice) = notice
            && area.height > 0
        {
            let status = Rect::new(
                area.x,
                area.y + area.height.saturating_sub(1),
                area.width,
                1,
            );
            frame.render_widget(Clear, status);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    crate::grid::truncate(notice, usize::from(area.width)),
                    Style::default().fg(notice_color),
                )),
                status,
            );
        }

        if ctx.focused
            && let Some(cursor) = snapshot.cursor
            && cursor.visible
            && cursor.column < area.width
            && cursor.row < area.height
        {
            frame.set_cursor_position((area.x + cursor.column, area.y + cursor.row));
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        self.expire_input_barrier();
        let canonical = canonical_key(key);
        if matches!(self.phase, Phase::Exited(_) | Phase::Failed(_)) && canonical == "r" {
            self.start();
            return KeyOutcome::Consumed;
        }
        // A passive panel can add local actions, but it cannot redefine the
        // shell's navigation. Capturing is the same explicit modal state used
        // by in-tree editors, where ordinary globals are suspended; Ctrl+C is
        // intercepted by App before either state reaches this method.
        if !self.policy.capture && self.input_barrier.is_none() && host_owns_key_while_passive(key)
        {
            return KeyOutcome::Ignored;
        }
        if !self.policy.capture
            && self.input_barrier.is_none()
            && !self.policy.keys.iter().any(|item| item == &canonical)
        {
            return KeyOutcome::Ignored;
        }

        let accepted = self.send(HostMessage::Key {
            key: canonical,
            code: key_code(key.code),
            text: match key.code {
                KeyCode::Char(character) => Some(character.to_string()),
                _ => None,
            },
            modifiers: key_modifiers(key.modifiers),
        });
        if accepted {
            self.begin_input_barrier();
        }
        // Once a plugin declares capture, a stalled process must not turn its
        // queued `q` into Mirador's global quit key.
        KeyOutcome::Consumed
    }

    fn handle_paste(&mut self, text: &str) -> KeyOutcome {
        self.expire_input_barrier();
        if !self.policy.paste {
            // Capture and the race barrier own the event, but paste is a
            // separate capability. Consume it without converting its contents
            // into key messages or forwarding it to a process that opted out.
            return if self.policy.capture || self.input_barrier_active() {
                KeyOutcome::Consumed
            } else {
                KeyOutcome::Ignored
            };
        }
        let message = HostMessage::Paste {
            text: text.to_string(),
        };
        if !host_message_fits(&message) {
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            shared.notice =
                Some("paste is too large for external panel input (8 MiB message limit)".into());
            shared.changed();
            drop(shared);
            self.sync();
            return KeyOutcome::Consumed;
        }
        let _ = self.send(message);
        KeyOutcome::Consumed
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> KeyOutcome {
        self.expire_input_barrier();
        if !self.policy.mouse {
            return if self.policy.capture || self.input_barrier_active() {
                KeyOutcome::Consumed
            } else {
                KeyOutcome::Ignored
            };
        }
        let Some((kind, button)) = mouse_kind(event.kind) else {
            return KeyOutcome::Ignored;
        };
        let _ = self.send(HostMessage::Mouse {
            kind,
            button,
            column: event.column.saturating_sub(area.x),
            row: event.row.saturating_sub(area.y),
            modifiers: key_modifiers(event.modifiers),
        });
        KeyOutcome::Consumed
    }

    fn captures_input(&self) -> bool {
        self.policy.capture || self.input_barrier_active()
    }

    fn shutdown(&mut self) {
        self.stop();
        self.phase = Phase::Stopping;
    }
}

impl Drop for PluginPanel {
    fn drop(&mut self) {
        self.stop();
    }
}

fn push_status_text(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    style: Style,
    width: u16,
    height: u16,
) {
    let remaining = usize::from(height).saturating_sub(lines.len());
    if remaining == 0 {
        return;
    }
    for row in crate::grid::wrap(text, usize::from(width))
        .into_iter()
        .take(remaining)
    {
        lines.push(Line::from(Span::styled(
            crate::grid::truncate(&row, usize::from(width)),
            style,
        )));
    }
}

/// Wrap a plugin snapshot using Mirador's own cell-aware rules while keeping
/// the styles attached to the bytes that supplied each row.
///
/// Only enough source for the visible rows plus one row of word look-ahead is
/// inspected. A legal 1 MiB snapshot therefore cannot make a ten-row panel
/// reprocess 1 MiB on every draw, and pathological runs of zero-width marks are
/// bounded by a per-cell byte budget as well.
fn wrapped_wire_lines(
    source: &[WireLine],
    width: u16,
    height: u16,
    theme: &crate::theme::Theme,
) -> Vec<Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let mut output = Vec::with_capacity(usize::from(height));
    for line in source {
        let remaining = usize::from(height).saturating_sub(output.len());
        if remaining == 0 {
            break;
        }
        let prefix = visible_wire_line(line, usize::from(width), remaining, theme);
        for row in crate::grid::wrap_line(&prefix, usize::from(width))
            .into_iter()
            .take(remaining)
        {
            output.push(row);
        }
    }
    output
}

fn visible_wire_line(
    line: &WireLine,
    width: usize,
    rows: usize,
    theme: &crate::theme::Theme,
) -> Line<'static> {
    // A two-cell glyph in a one-cell panel is replaced by one ellipsis. Count
    // enough source width to produce every requested replacement row.
    let cell_budget = width.max(2).saturating_mul(rows.saturating_add(1));
    let byte_budget = cell_budget
        .saturating_mul(RENDER_BYTES_PER_CELL)
        .clamp(256, MAX_RETAINED_FRAME_TEXT_BYTES);
    let mut output = Vec::new();
    let mut bytes = 0usize;
    let mut cells = 0usize;
    let mut complete = true;

    for span in &line.spans {
        let mut text = String::new();
        for character in span.text.chars() {
            let character_bytes = character.len_utf8();
            let character_width = crate::grid::char_width(character);
            if bytes.saturating_add(character_bytes) > byte_budget
                || (character_width > 0 && cells.saturating_add(character_width) > cell_budget)
            {
                complete = false;
                break;
            }
            text.push(character);
            bytes = bytes.saturating_add(character_bytes);
            cells = cells.saturating_add(character_width);
        }
        if !text.is_empty() {
            output.push(Span::styled(text, span_style(span, theme)));
        }
        if !complete {
            break;
        }
    }
    Line::from(output)
}

fn input_barrier_timeout(refresh: Duration) -> Duration {
    refresh
        .saturating_mul(INPUT_BARRIER_REFRESHES)
        .clamp(MIN_INPUT_BARRIER, MAX_INPUT_BARRIER)
}

fn host_message_fits(message: &HostMessage) -> bool {
    serde_json::to_vec(message)
        .is_ok_and(|encoded| encoded.len().saturating_add(1) <= MAX_MESSAGE_BYTES)
}

fn canonical_key(key: KeyEvent) -> String {
    let base = match key.code {
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char(character) => character.to_string(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "BackTab".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::F(number) => format!("F{number}"),
        other => format!("{other:?}"),
    };
    let mut prefixes = key_modifiers(key.modifiers);
    if prefixes.is_empty() {
        base
    } else {
        prefixes.push(base);
        prefixes.join("+")
    }
}

/// Keys whose passive meaning belongs to the dashboard shell.
///
/// This mirrors `app::dispatch_key` rather than the text printed in a help
/// overlay. Matching the event code is important: Mirador's existing globals
/// act on `Alt+q` as well as bare `q`, so a plugin must not be able to claim the
/// modified spelling and silently get ahead of the shell.
fn host_owns_key_while_passive(key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(
            key.code,
            KeyCode::Char('c') | KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
        )
    {
        return true;
    }
    matches!(
        key.code,
        KeyCode::Char('q' | '?' | 'w' | 'm' | 't' | '1'..='9') | KeyCode::Tab | KeyCode::BackTab
    )
}

/// String-side equivalent used to reject a frame that advertises a passive
/// claim the host will never honour.
fn host_owned_chord(raw: &str) -> bool {
    let (control, base) = chord_parts(raw);
    let jump = base.len() == 1 && matches!(base.as_bytes()[0], b'1'..=b'9');
    (control
        && matches!(
            base.to_ascii_lowercase().as_str(),
            "c" | "left" | "right" | "up" | "down"
        ))
        || matches!(base, "q" | "?" | "w" | "m" | "t" | "Tab" | "BackTab")
        || jump
}

fn is_ctrl_c_chord(raw: &str) -> bool {
    let (control, base) = chord_parts(raw);
    control && base.eq_ignore_ascii_case("c")
}

fn binding_mentions_host_key(raw: &str, capture: bool) -> bool {
    raw.split(|character: char| {
        character.is_whitespace() || matches!(character, '/' | ',' | '·' | '(' | ')')
    })
    .filter(|part| !part.is_empty())
    .any(|part| {
        is_ctrl_c_chord(part)
            || part.eq_ignore_ascii_case("ctrl+c")
            || (!capture
                && (host_owned_chord(part)
                    || part == "1-9"
                    || part.eq_ignore_ascii_case("ctrl+arrows")))
    })
}

fn chord_parts(raw: &str) -> (bool, &str) {
    let mut pieces = raw.split('+').peekable();
    let mut control = false;
    let mut base = "";
    while let Some(piece) = pieces.next() {
        if pieces.peek().is_none() {
            base = piece;
        } else if piece.eq_ignore_ascii_case("ctrl") {
            control = true;
        }
    }
    (control, base)
}

fn key_code(code: KeyCode) -> String {
    match code {
        KeyCode::Char(_) => "char".into(),
        KeyCode::F(number) => format!("f{number}"),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}

fn key_modifiers(modifiers: KeyModifiers) -> Vec<String> {
    [
        (KeyModifiers::CONTROL, "Ctrl"),
        (KeyModifiers::ALT, "Alt"),
        (KeyModifiers::SHIFT, "Shift"),
        (KeyModifiers::SUPER, "Super"),
        (KeyModifiers::HYPER, "Hyper"),
        (KeyModifiers::META, "Meta"),
    ]
    .into_iter()
    .filter(|(flag, _)| modifiers.contains(*flag))
    .map(|(_, name)| name.to_string())
    .collect()
}

fn mouse_kind(kind: MouseEventKind) -> Option<(String, Option<String>)> {
    match kind {
        MouseEventKind::Down(button) => Some(("down".into(), Some(mouse_button(button)))),
        MouseEventKind::ScrollDown => Some(("scroll_down".into(), None)),
        MouseEventKind::ScrollUp => Some(("scroll_up".into(), None)),
        MouseEventKind::Up(_)
        | MouseEventKind::Drag(_)
        | MouseEventKind::Moved
        | MouseEventKind::ScrollLeft
        | MouseEventKind::ScrollRight => None,
    }
}

fn mouse_button(button: MouseButton) -> String {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
    .into()
}

fn span_style(span: &WireSpan, theme: &crate::theme::Theme) -> Style {
    let mut style = Style::default();
    if let Some(color) = span.fg.as_deref().and_then(|raw| wire_color(raw, theme)) {
        style = style.fg(color);
    }
    if let Some(color) = span.bg.as_deref().and_then(|raw| wire_color(raw, theme)) {
        style = style.bg(color);
    }
    for (enabled, modifier) in [
        (span.bold, Modifier::BOLD),
        (span.dim, Modifier::DIM),
        (span.italic, Modifier::ITALIC),
        (span.underlined, Modifier::UNDERLINED),
        (span.reversed, Modifier::REVERSED),
    ] {
        if enabled {
            style = style.add_modifier(modifier);
        }
    }
    style
}

fn wire_color(raw: &str, theme: &crate::theme::Theme) -> Option<Color> {
    match raw {
        "default" | "reset" => Some(Color::Reset),
        "theme:border" => Some(theme.border),
        "theme:border_focused" => Some(theme.border_focused),
        "theme:rule" => Some(theme.rule),
        "theme:title" => Some(theme.title),
        "theme:text" => Some(theme.text),
        "theme:muted" => Some(theme.muted),
        "theme:label" => Some(theme.label),
        "theme:accent" => Some(theme.accent),
        "theme:key" => Some(theme.key),
        "theme:success" => Some(theme.success),
        "theme:warning" => Some(theme.warning),
        "theme:error" => Some(theme.error),
        "theme:track" => Some(theme.track),
        _ => raw
            .strip_prefix("ansi:")
            .and_then(|index| index.parse::<u8>().ok())
            .map(Color::Indexed)
            .or_else(|| raw.parse().ok()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, BufReader};

    use super::process::{apply_message, read_limited_line};
    use super::*;

    fn detached_panel(policy: InputPolicy) -> PluginPanel {
        PluginPanel {
            spec: PluginConfig {
                id: "test".into(),
                command: vec!["unused".into()],
                config: toml::Table::new(),
            },
            runtime: None,
            shared: Arc::new(Mutex::new(Shared::starting())),
            seen_generation: 0,
            seen_frame_sequence: 0,
            phase: Phase::Running,
            title: "test".into(),
            refresh: DEFAULT_REFRESH,
            frame: None,
            bindings: Vec::new(),
            policy,
            notice: None,
            input_barrier: None,
            last_size: None,
            last_focus: None,
        }
    }

    #[test]
    fn ready_must_match_the_host_protocol() {
        let shared = Arc::new(Mutex::new(Shared::starting()));
        assert!(!apply_message(
            PluginMessage::Ready {
                protocol: PROTOCOL_VERSION + 1,
                title: None,
                refresh_ms: None,
            },
            &shared,
        ));
        assert!(matches!(
            shared.lock().unwrap().phase,
            Phase::Failed(ref message) if message.contains("incompatible")
        ));
    }

    #[test]
    fn spawn_failure_is_panel_state_and_restart_retries_the_process() {
        let mut panel = PluginPanel::new(PluginConfig {
            id: "missing-test-plugin".into(),
            command: vec!["mirador-plugin-test-command-that-does-not-exist-7f09".into()],
            config: toml::Table::new(),
        });
        assert!(matches!(
            panel.phase,
            Phase::Failed(ref error) if error.contains("could not start plugin")
        ));
        let first_attempt = Arc::clone(&panel.shared);

        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Char('r'))),
            KeyOutcome::Consumed
        );
        assert!(
            !Arc::ptr_eq(&first_attempt, &panel.shared),
            "restart did not create a new process session"
        );
        assert!(panel.sync());
        assert!(matches!(
            panel.phase,
            Phase::Failed(ref error) if error.contains("could not start plugin")
        ));
    }

    #[test]
    fn frames_are_full_snapshots_and_old_revisions_are_ignored() {
        let shared = Arc::new(Mutex::new(Shared::starting()));
        assert!(apply_message(
            PluginMessage::Ready {
                protocol: PROTOCOL_VERSION,
                title: Some("test".into()),
                refresh_ms: Some(1),
            },
            &shared,
        ));
        let frame = |revision, text: &str| PluginMessage::Frame {
            revision,
            title: None,
            counter: None,
            lines: vec![WireLine {
                spans: vec![WireSpan {
                    text: text.into(),
                    fg: None,
                    bg: None,
                    bold: false,
                    dim: false,
                    italic: false,
                    underlined: false,
                    reversed: false,
                }],
            }],
            bindings: Vec::new(),
            input: InputPolicy::default(),
            cursor: None,
        };
        assert!(apply_message(frame(2, "new"), &shared));
        assert!(apply_message(frame(1, "old"), &shared));
        let shared = shared.lock().unwrap();
        assert_eq!(shared.refresh, Duration::from_millis(16));
        assert_eq!(shared.frame.as_ref().unwrap().lines[0].spans[0].text, "new");
    }

    #[test]
    fn an_omitted_frame_title_restores_the_negotiated_title() {
        let mut panel = detached_panel(InputPolicy::default());
        assert!(apply_message(
            PluginMessage::Ready {
                protocol: PROTOCOL_VERSION,
                title: Some("negotiated".into()),
                refresh_ms: None,
            },
            &panel.shared,
        ));
        let frame = |revision, title| PluginMessage::Frame {
            revision,
            title,
            counter: None,
            lines: Vec::new(),
            bindings: Vec::new(),
            input: InputPolicy::default(),
            cursor: None,
        };

        assert!(apply_message(
            frame(1, Some("temporary".into())),
            &panel.shared
        ));
        assert!(panel.sync());
        assert_eq!(panel.title, "temporary");

        assert!(apply_message(frame(2, None), &panel.shared));
        assert!(panel.sync());
        assert_eq!(panel.title, "negotiated");
    }

    #[test]
    fn canonical_keys_are_stable_across_platforms() {
        assert_eq!(
            canonical_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            "Ctrl+c"
        );
        assert_eq!(
            canonical_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::SHIFT)),
            "Shift+PageUp"
        );
        assert_eq!(canonical_key(KeyEvent::from(KeyCode::Enter)), "Enter");
    }

    #[test]
    fn semantic_and_terminal_colours_share_the_wire_format() {
        let theme = crate::theme::Theme::default();
        assert_eq!(wire_color("theme:accent", &theme), Some(theme.accent));
        assert_eq!(wire_color("ansi:203", &theme), Some(Color::Indexed(203)));
        assert_eq!(wire_color("#112233", &theme), Some(Color::Rgb(17, 34, 51)));
    }

    #[test]
    fn host_wraps_styled_output_to_cells_and_visible_rows() {
        let theme = crate::theme::Theme::default();
        let source = [WireLine {
            spans: vec![
                WireSpan {
                    text: "one ".into(),
                    fg: Some("theme:accent".into()),
                    bg: None,
                    bold: true,
                    dim: false,
                    italic: false,
                    underlined: false,
                    reversed: false,
                },
                WireSpan {
                    text: "two three".into(),
                    fg: None,
                    bg: None,
                    bold: false,
                    dim: false,
                    italic: true,
                    underlined: false,
                    reversed: false,
                },
            ],
        }];
        let lines = wrapped_wire_lines(&source, 7, 2, &theme);
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let text: String = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            assert!(crate::grid::display_width(&text) <= 7, "{text:?}");
        }
        assert!(
            lines[0]
                .spans
                .first()
                .unwrap()
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn a_glyph_wider_than_the_panel_is_replaced_not_spilled() {
        let theme = crate::theme::Theme::default();
        let source = [WireLine {
            spans: vec![WireSpan {
                text: "界界".into(),
                fg: None,
                bg: None,
                bold: false,
                dim: false,
                italic: false,
                underlined: false,
                reversed: false,
            }],
        }];
        let lines = wrapped_wire_lines(&source, 1, 2, &theme);
        assert_eq!(lines.len(), 2);
        for line in lines {
            let text: String = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            assert_eq!(text, "…");
            assert_eq!(crate::grid::display_width(&text), 1);
        }
    }

    #[test]
    fn a_style_boundary_cannot_make_a_joined_glyph_overrun() {
        let theme = crate::theme::Theme::default();
        let source = [WireLine {
            spans: vec![
                WireSpan {
                    text: "👩".into(),
                    fg: Some("theme:accent".into()),
                    bg: None,
                    bold: false,
                    dim: false,
                    italic: false,
                    underlined: false,
                    reversed: false,
                },
                WireSpan {
                    text: "\u{200d}💻".into(),
                    fg: Some("theme:text".into()),
                    bg: None,
                    bold: false,
                    dim: false,
                    italic: false,
                    underlined: false,
                    reversed: false,
                },
            ],
        }];
        let lines = wrapped_wire_lines(&source, 2, 1, &theme);
        let rendered_width: usize = lines[0]
            .spans
            .iter()
            .map(|span| crate::grid::display_width(&span.content))
            .sum();
        assert!(rendered_width <= 2, "rendered width was {rendered_width}");
    }

    #[test]
    fn external_panels_disclose_their_origin_in_the_frame_title() {
        let panel = detached_panel(InputPolicy::default());
        assert_eq!(panel.title(), "EXTERNAL · test");
    }

    #[test]
    fn passive_policy_claims_only_named_keys_and_capture_never_falls_through() {
        let mut passive = detached_panel(InputPolicy {
            keys: vec!["Enter".into()],
            ..InputPolicy::default()
        });
        assert_eq!(
            passive.handle_key(KeyEvent::from(KeyCode::Enter)),
            KeyOutcome::Consumed
        );
        assert_eq!(
            passive.handle_key(KeyEvent::from(KeyCode::Char('q'))),
            KeyOutcome::Ignored
        );

        let mut capturing = detached_panel(InputPolicy {
            capture: true,
            ..InputPolicy::default()
        });
        assert_eq!(
            capturing.handle_key(KeyEvent::from(KeyCode::Char('q'))),
            KeyOutcome::Consumed,
            "a stalled plugin must not turn its q into a global quit"
        );
    }

    #[test]
    fn passive_plugins_cannot_claim_shell_keys_but_capture_can_suspend_them() {
        let shell_keys = [
            KeyEvent::from(KeyCode::Char('q')),
            KeyEvent::from(KeyCode::Tab),
            KeyEvent::from(KeyCode::Char('?')),
            KeyEvent::from(KeyCode::Char('w')),
            KeyEvent::from(KeyCode::Char('m')),
            KeyEvent::from(KeyCode::Char('t')),
            KeyEvent::from(KeyCode::Char('3')),
            KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
        ];
        let mut passive = detached_panel(InputPolicy {
            keys: shell_keys.iter().copied().map(canonical_key).collect(),
            ..InputPolicy::default()
        });
        for key in shell_keys {
            assert_eq!(passive.handle_key(key), KeyOutcome::Ignored, "{key:?}");
        }

        let mut capturing = detached_panel(InputPolicy {
            capture: true,
            ..InputPolicy::default()
        });
        assert_eq!(
            capturing.handle_key(KeyEvent::from(KeyCode::Char('q'))),
            KeyOutcome::Consumed
        );
    }

    #[test]
    fn protocol_spellings_identify_every_reserved_chord() {
        for chord in [
            "q",
            "Alt+q",
            "?",
            "w",
            "m",
            "t",
            "Tab",
            "BackTab",
            "Shift+BackTab",
            "1",
            "9",
            "Ctrl+c",
            "Ctrl+Shift+c",
            "Ctrl+Left",
        ] {
            assert!(host_owned_chord(chord), "{chord}");
        }
        for chord in ["Q", "Enter", "r", "Alt+Left", "Ctrl+x"] {
            assert!(!host_owned_chord(chord), "{chord}");
        }
    }

    #[test]
    fn any_accepted_frame_releases_a_passive_input_barrier() {
        let passive = InputPolicy {
            keys: vec!["Enter".into()],
            ..InputPolicy::default()
        };
        let mut panel = detached_panel(passive.clone());
        assert!(apply_message(
            PluginMessage::Ready {
                protocol: PROTOCOL_VERSION,
                title: None,
                refresh_ms: None,
            },
            &panel.shared,
        ));
        assert!(apply_message(
            PluginMessage::Frame {
                revision: u64::MAX,
                title: None,
                counter: None,
                lines: Vec::new(),
                bindings: Vec::new(),
                input: passive,
                cursor: None,
            },
            &panel.shared,
        ));
        assert!(panel.sync());

        // The maximum revision cannot be exceeded. A second valid frame with
        // that revision is still an acknowledgement even though it cannot
        // replace the retained snapshot.
        panel.begin_input_barrier();
        assert!(panel.captures_input());
        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Char('q'))),
            KeyOutcome::Consumed
        );
        assert!(apply_message(
            PluginMessage::Frame {
                revision: u64::MAX,
                title: None,
                counter: None,
                lines: Vec::new(),
                bindings: Vec::new(),
                input: InputPolicy::default(),
                cursor: None,
            },
            &panel.shared,
        ));
        assert!(
            !panel.sync(),
            "an acknowledgement without a new snapshot must not force a redraw"
        );
        assert!(!panel.captures_input());
        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Char('q'))),
            KeyOutcome::Ignored
        );
    }

    #[test]
    fn an_unacknowledged_input_barrier_expires_as_a_failed_panel() {
        let mut panel = detached_panel(InputPolicy {
            keys: vec!["Enter".into()],
            ..InputPolicy::default()
        });
        panel.input_barrier = Some(InputBarrier {
            frame_sequence: 0,
            expires_at: Instant::now()
                .checked_sub(Duration::from_millis(1))
                .expect("one millisecond fits before now"),
        });

        assert_eq!(panel.refresh_interval(), Duration::from_millis(1));
        assert!(panel.expire_input_barrier());
        assert!(!panel.captures_input());
        assert!(matches!(
            panel.phase,
            Phase::Failed(ref error) if error.contains("acknowledge input")
        ));
        assert_eq!(
            panel.handle_key(KeyEvent::from(KeyCode::Char('q'))),
            KeyOutcome::Ignored,
            "a failed plugin must return ordinary shell keys to Mirador"
        );
    }

    #[test]
    fn a_barrier_never_grants_undeclared_paste_or_mouse_capabilities() {
        let mut panel = detached_panel(InputPolicy::default());
        let (runtime, received) = Runtime::stub();
        panel.runtime = Some(runtime);
        panel.input_barrier = Some(InputBarrier {
            frame_sequence: 0,
            expires_at: Instant::now() + Duration::from_secs(1),
        });
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };

        assert_eq!(panel.handle_paste("private"), KeyOutcome::Consumed);
        assert_eq!(
            panel.handle_mouse(mouse, Rect::new(0, 0, 10, 10)),
            KeyOutcome::Consumed
        );
        assert!(
            matches!(
                received.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "undeclared input reached the plugin process"
        );
        panel.shutdown();
    }

    #[test]
    fn oversized_user_paste_is_dropped_without_blame_or_process_failure() {
        let mut panel = detached_panel(InputPolicy {
            paste: true,
            ..InputPolicy::default()
        });
        let (runtime, received) = Runtime::stub();
        panel.runtime = Some(runtime);

        assert_eq!(
            panel.handle_paste(&"x".repeat(MAX_MESSAGE_BYTES)),
            KeyOutcome::Consumed
        );
        let shared = panel.shared.lock().unwrap();
        assert!(
            !matches!(shared.phase, Phase::Failed(_)),
            "host input size must not be reported as a plugin protocol failure"
        );
        assert!(
            shared
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("paste is too large"))
        );
        drop(shared);
        assert!(matches!(
            received.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        panel.shutdown();
    }

    #[test]
    fn watch_messages_enter_the_native_panel_event_drain() {
        let mut panel = detached_panel(InputPolicy::default());
        assert!(apply_message(
            PluginMessage::Ready {
                protocol: PROTOCOL_VERSION,
                title: None,
                refresh_ms: None,
            },
            &panel.shared,
        ));
        assert!(apply_message(
            PluginMessage::Watch {
                text: "the build finished".into(),
            },
            &panel.shared,
        ));

        let events = panel.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source, "test");
        assert_eq!(events[0].text, "the build finished");
        assert!(panel.events().is_empty(), "the native hook is a drain");
    }

    #[test]
    fn watch_messages_are_single_lines_and_their_queue_is_bounded() {
        let shared = Arc::new(Mutex::new(Shared::starting()));
        assert!(apply_message(
            PluginMessage::Ready {
                protocol: PROTOCOL_VERSION,
                title: None,
                refresh_ms: None,
            },
            &shared,
        ));
        for index in 0..65 {
            assert!(apply_message(
                PluginMessage::Watch {
                    text: format!("event {index}"),
                },
                &shared,
            ));
        }
        let guard = shared.lock().unwrap();
        assert_eq!(guard.watch.len(), 64);
        assert_eq!(guard.watch.front().map(String::as_str), Some("event 1"));
        drop(guard);

        assert!(!apply_message(
            PluginMessage::Watch {
                text: "not\none line".into(),
            },
            &shared,
        ));
        assert!(matches!(
            shared.lock().unwrap().phase,
            Phase::Failed(ref error) if error.contains("one non-empty line")
        ));
    }

    #[test]
    fn oversized_messages_are_rejected_before_unbounded_allocation() {
        let input = vec![b'x'; MAX_MESSAGE_BYTES + 1];
        let mut reader = BufReader::new(input.as_slice());
        let error = read_limited_line(&mut reader, &mut Vec::new()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn the_last_json_object_still_requires_a_newline() {
        let mut reader = BufReader::new(br#"{"type":"ready","protocol":1}"#.as_slice());
        let error = read_limited_line(&mut reader, &mut Vec::new()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("newline"));
    }

    /// Cross-repository smoke test. Point `MIRADOR_TEST_PLUGIN` at any protocol
    /// command; language SDKs can use this in their own release checks.
    #[test]
    #[ignore = "requires an explicitly installed external plugin command"]
    fn an_external_process_negotiates_and_publishes_a_frame() {
        use std::time::Instant;

        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let command = std::env::var("MIRADOR_TEST_PLUGIN")
            .expect("set MIRADOR_TEST_PLUGIN to an external plugin executable");
        let mut panel = PluginPanel::new(PluginConfig {
            id: "integration".into(),
            command: vec![command],
            config: toml::Table::new(),
        });
        let theme = crate::theme::Theme::default();
        let gradients = theme.gradients();
        let watch = crate::watch::WatchLog::default();
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);

        while Instant::now() < deadline {
            panel.tick();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    panel.render(
                        frame,
                        area,
                        RenderContext {
                            theme: &theme,
                            gradients: &gradients,
                            focused: true,
                            watch: &watch,
                        },
                    );
                })
                .unwrap();
            if matches!(panel.phase, Phase::Running) && panel.frame.is_some() {
                panel.shutdown();
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        let phase = panel.phase.clone();
        panel.shutdown();
        panic!("plugin did not publish a frame within five seconds: {phase:?}");
    }

    /// Full proof for the dashboard-native example: Rust host -> Python SDK ->
    /// process watcher -> child process -> protocol watch message -> native
    /// `Panel::events` drain.
    #[test]
    #[ignore = "requires an explicitly installed mirador-process-watch command"]
    fn an_external_process_watcher_reports_a_native_watch_event() {
        use std::time::Instant;

        let watcher = std::env::var("MIRADOR_TEST_WATCH_PLUGIN")
            .expect("set MIRADOR_TEST_WATCH_PLUGIN to mirador-process-watch");
        let child = std::env::current_exe()
            .expect("test executable has a path")
            .to_string_lossy()
            .into_owned();
        let mut config = toml::Table::new();
        config.insert("name".into(), toml::Value::String("Host tests".into()));
        config.insert(
            "command".into(),
            toml::Value::Array(
                [child, "--list".into()]
                    .into_iter()
                    .map(toml::Value::String)
                    .collect(),
            ),
        );
        config.insert("autostart".into(), toml::Value::Boolean(true));
        config.insert("output_lines".into(), toml::Value::Integer(50));
        let mut panel = PluginPanel::new(PluginConfig {
            id: "process-watch-test".into(),
            command: vec![watcher],
            config,
        });
        let deadline = Instant::now() + Duration::from_secs(15);

        while Instant::now() < deadline {
            panel.tick();
            if let Some(event) = panel.events().into_iter().next() {
                panel.shutdown();
                assert_eq!(event.source, "process-watch-test");
                assert!(
                    event.text.contains("finished successfully"),
                    "unexpected Watch Log event: {}",
                    event.text
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        let phase = panel.phase.clone();
        panel.shutdown();
        panic!("process watcher did not report completion within fifteen seconds: {phase:?}");
    }
}
