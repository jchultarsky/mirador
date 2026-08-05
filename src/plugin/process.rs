//! Child lifecycle and bounded stdio workers for one external panel.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::PluginConfig;

use super::{
    DEFAULT_REFRESH, HostMessage, InputPolicy, MAX_BINDING_ACTION_BYTES, MAX_BINDING_KEY_BYTES,
    MAX_BINDINGS, MAX_COLOR_BYTES, MAX_COUNTER_BYTES, MAX_ERROR_BYTES, MAX_FRAME_LINES,
    MAX_FRAME_SPANS, MAX_INPUT_KEYS, MAX_MESSAGE_BYTES, MAX_RETAINED_FRAME_TEXT_BYTES,
    MAX_TITLE_BYTES, MAX_WATCH_TEXT_BYTES, PROTOCOL_VERSION, PluginMessage, WireBinding, WireFrame,
    WireLine, binding_mentions_host_key, host_owned_chord, is_ctrl_c_chord, wire_color,
};

const MAX_STDERR_BYTES: usize = 8 * 1024;
const EVENT_QUEUE: usize = 256;
const WATCH_QUEUE: usize = 64;
const SHUTDOWN_GRACE: Duration = Duration::from_millis(300);
const OUTPUT_CLOSE_GRACE: Duration = Duration::from_millis(100);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const MESSAGE_RATE_WINDOW: Duration = Duration::from_secs(1);
const MAX_MESSAGES_PER_WINDOW: usize = 120;
const MAX_STDOUT_BYTES_PER_WINDOW: usize = 16 * 1024 * 1024;
const MAX_STDERR_BYTES_PER_WINDOW: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Phase {
    Starting,
    Running,
    Exited(String),
    Failed(String),
    Stopping,
}

pub(super) struct Shared {
    pub(super) generation: u64,
    pub(super) ready: bool,
    pub(super) phase: Phase,
    pub(super) title: Option<String>,
    pub(super) refresh: Duration,
    pub(super) frame: Option<Arc<WireFrame>>,
    pub(super) notice: Option<String>,
    pub(super) stderr: String,
    pub(super) watch: VecDeque<String>,
}

impl Shared {
    pub(super) fn starting() -> Self {
        Self {
            generation: 0,
            ready: false,
            phase: Phase::Starting,
            title: None,
            refresh: DEFAULT_REFRESH,
            frame: None,
            notice: None,
            stderr: String::new(),
            watch: VecDeque::new(),
        }
    }

    pub(super) fn changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub(super) fn fail(&mut self, message: impl Into<String>) {
        // Stdio workers and the supervisor can discover the same failure in a
        // different order. Preserve the first terminal explanation instead of
        // replacing a useful protocol error with the broken pipe caused by
        // terminating that process.
        if matches!(
            self.phase,
            Phase::Exited(_) | Phase::Failed(_) | Phase::Stopping
        ) {
            return;
        }
        self.phase = Phase::Failed(message.into());
        self.changed();
    }
}

enum SupervisorCommand {
    Shutdown,
    Abort,
    OutputClosed,
}

pub(super) struct Runtime {
    pub(super) events: SyncSender<HostMessage>,
    supervisor: mpsc::Sender<SupervisorCommand>,
}

impl Runtime {
    pub(super) fn shutdown(&self) {
        let _ = self.events.try_send(HostMessage::Shutdown);
        let _ = self.supervisor.send(SupervisorCommand::Shutdown);
    }
}

pub(super) fn spawn_process(
    spec: &PluginConfig,
    shared: &Arc<Mutex<Shared>>,
) -> io::Result<Runtime> {
    let mut command = Command::new(&spec.command[0]);
    command
        .args(&spec.command[1..])
        .env("MIRADOR_PLUGIN_PROTOCOL", PROTOCOL_VERSION.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let startup_deadline = Instant::now() + STARTUP_TIMEOUT;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("missing stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing stderr"))?;

    let (event_tx, event_rx) = mpsc::sync_channel(EVENT_QUEUE);
    let (supervisor_tx, supervisor_rx) = mpsc::channel();

    let writer_shared = Arc::clone(shared);
    let writer_supervisor = supervisor_tx.clone();
    thread::spawn(move || writer_loop(stdin, event_rx, &writer_shared, &writer_supervisor));

    let reader_shared = Arc::clone(shared);
    let reader_supervisor = supervisor_tx.clone();
    thread::spawn(move || reader_loop(stdout, &reader_shared, &reader_supervisor));

    let stderr_shared = Arc::clone(shared);
    let stderr_supervisor = supervisor_tx.clone();
    thread::spawn(move || stderr_loop(stderr, &stderr_shared, &stderr_supervisor));

    let supervisor_shared = Arc::clone(shared);
    thread::spawn(move || {
        supervisor_loop(
            &mut child,
            &supervisor_rx,
            &supervisor_shared,
            startup_deadline,
        );
    });

    let cwd = std::env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let config = serde_json::to_value(&spec.config).unwrap_or(serde_json::Value::Null);
    let hello = HostMessage::Hello {
        protocol: PROTOCOL_VERSION,
        host_version: env!("CARGO_PKG_VERSION"),
        plugin: spec.id.clone(),
        config,
        cwd,
    };
    event_tx
        .try_send(hello)
        .map_err(|error| io::Error::other(format!("sending hello: {error}")))?;

    Ok(Runtime {
        events: event_tx,
        supervisor: supervisor_tx,
    })
}

fn writer_loop(
    mut stdin: impl Write,
    events: Receiver<HostMessage>,
    shared: &Arc<Mutex<Shared>>,
    supervisor: &mpsc::Sender<SupervisorCommand>,
) {
    for event in events {
        let result = write_host_message(&mut stdin, &event);
        if let Err(error) = result {
            shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .fail(format!("writing to plugin: {error}"));
            let _ = supervisor.send(SupervisorCommand::Abort);
            break;
        }
        if matches!(event, HostMessage::Shutdown) {
            break;
        }
    }
}

fn write_host_message(mut target: impl Write, event: &HostMessage) -> io::Result<()> {
    let encoded = serde_json::to_vec(event).map_err(io::Error::other)?;
    if encoded.len().saturating_add(1) > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("host message exceeds {MAX_MESSAGE_BYTES} bytes"),
        ));
    }
    target.write_all(&encoded)?;
    target.write_all(b"\n")?;
    target.flush()
}

#[derive(Default)]
struct MessageRate {
    recent: VecDeque<(Instant, usize)>,
    bytes: usize,
}

impl MessageRate {
    fn accepts(&mut self, now: Instant, bytes: usize) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|(then, _)| now.saturating_duration_since(*then) >= MESSAGE_RATE_WINDOW)
        {
            if let Some((_, expired)) = self.recent.pop_front() {
                self.bytes = self.bytes.saturating_sub(expired);
            }
        }
        if self.recent.len() >= MAX_MESSAGES_PER_WINDOW
            || self.bytes.saturating_add(bytes) > MAX_STDOUT_BYTES_PER_WINDOW
        {
            return false;
        }
        self.recent.push_back((now, bytes));
        self.bytes += bytes;
        true
    }
}

fn reader_loop(
    stdout: impl io::Read,
    shared: &Arc<Mutex<Shared>>,
    supervisor: &mpsc::Sender<SupervisorCommand>,
) {
    let mut reader = BufReader::new(stdout);
    let mut rate = MessageRate::default();
    loop {
        let mut bytes = Vec::new();
        match read_limited_line(&mut reader, &mut bytes) {
            Ok(0) => {
                let _ = supervisor.send(SupervisorCommand::OutputClosed);
                break;
            }
            Ok(_) => {}
            Err(error) => {
                shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .fail(format!("reading plugin output: {error}"));
                let _ = supervisor.send(SupervisorCommand::Abort);
                break;
            }
        }
        if !rate.accepts(Instant::now(), bytes.len()) {
            shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .fail(format!(
                    "plugin exceeded {MAX_MESSAGES_PER_WINDOW} messages or {MAX_STDOUT_BYTES_PER_WINDOW} output bytes in one second"
                ));
            let _ = supervisor.send(SupervisorCommand::Abort);
            break;
        }
        while matches!(bytes.last(), Some(b'\n' | b'\r')) {
            bytes.pop();
        }
        let message: PluginMessage = match serde_json::from_slice(&bytes) {
            Ok(message) => message,
            Err(error) => {
                shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .fail(format!("invalid plugin message: {error}"));
                let _ = supervisor.send(SupervisorCommand::Abort);
                break;
            }
        };
        if !apply_message(message, shared) {
            let _ = supervisor.send(SupervisorCommand::Abort);
            break;
        }
    }
}

pub(super) fn read_limited_line(
    reader: &mut impl BufRead,
    target: &mut Vec<u8>,
) -> io::Result<usize> {
    let mut limited = io::Read::take(
        reader,
        u64::try_from(MAX_MESSAGE_BYTES + 1).unwrap_or(u64::MAX),
    );
    let read = limited.read_until(b'\n', target)?;
    if read > MAX_MESSAGE_BYTES || (read == MAX_MESSAGE_BYTES && !target.ends_with(b"\n")) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message exceeds {MAX_MESSAGE_BYTES} bytes"),
        ));
    }
    if read > 0 && !target.ends_with(b"\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plugin message is not terminated by a newline",
        ));
    }
    Ok(read)
}

pub(super) fn apply_message(message: PluginMessage, shared: &Arc<Mutex<Shared>>) -> bool {
    let mut shared = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match message {
        PluginMessage::Ready {
            protocol,
            title,
            refresh_ms,
        } => return apply_ready(protocol, title, refresh_ms, &mut shared),
        PluginMessage::Frame {
            revision,
            title,
            counter,
            lines,
            bindings,
            input,
            cursor,
        } => {
            return apply_frame(
                WireFrame {
                    revision,
                    title,
                    counter,
                    lines,
                    bindings,
                    input,
                    cursor,
                },
                &mut shared,
            );
        }
        PluginMessage::Error { message, fatal } => {
            if !shared.ready {
                shared.fail("plugin sent an error before `ready`");
                return false;
            }
            if let Err(error) = validate_text(&message, "error message", MAX_ERROR_BYTES) {
                shared.fail(error);
                return false;
            }
            if fatal {
                shared.fail(message);
                return false;
            }
            shared.notice = Some(message);
            shared.changed();
        }
        PluginMessage::Watch { text } => {
            if !shared.ready {
                shared.fail("plugin sent a watch event before `ready`");
                return false;
            }
            if text.trim().is_empty()
                || text.len() > MAX_WATCH_TEXT_BYTES
                || text.chars().any(char::is_control)
            {
                shared.fail(format!(
                    "plugin watch text must be one non-empty line of at most {MAX_WATCH_TEXT_BYTES} UTF-8 bytes"
                ));
                return false;
            }
            if shared.watch.len() == WATCH_QUEUE {
                shared.watch.pop_front();
                shared.notice = Some("plugin watch queue overflowed; oldest event dropped".into());
            }
            shared.watch.push_back(text);
            shared.changed();
        }
    }
    true
}

fn apply_ready(
    protocol: u16,
    title: Option<String>,
    refresh_ms: Option<u64>,
    shared: &mut Shared,
) -> bool {
    if protocol != PROTOCOL_VERSION {
        shared.fail(format!(
            "protocol {protocol} is incompatible with host protocol {PROTOCOL_VERSION}"
        ));
        return false;
    }
    if shared.ready {
        shared.fail("plugin sent `ready` more than once");
        return false;
    }
    if let Some(title) = title.as_deref()
        && let Err(error) = validate_text(title, "ready title", MAX_TITLE_BYTES)
    {
        shared.fail(error);
        return false;
    }
    shared.ready = true;
    shared.phase = Phase::Running;
    shared.title = title;
    shared.refresh = refresh_ms.map_or(DEFAULT_REFRESH, |milliseconds| {
        Duration::from_millis(milliseconds.clamp(16, 60_000))
    });
    shared.changed();
    true
}

fn apply_frame(frame: WireFrame, shared: &mut Shared) -> bool {
    if !shared.ready {
        shared.fail("plugin sent a frame before `ready`");
        return false;
    }
    if let Err(error) = validate_frame(
        frame.title.as_deref(),
        frame.counter.as_deref(),
        &frame.lines,
        &frame.bindings,
        &frame.input,
    ) {
        shared.fail(error);
        return false;
    }
    if shared
        .frame
        .as_ref()
        .is_some_and(|previous| frame.revision <= previous.revision)
    {
        return true;
    }
    shared.frame = Some(Arc::new(frame));
    shared.notice = None;
    shared.changed();
    true
}

fn validate_frame(
    title: Option<&str>,
    counter: Option<&str>,
    lines: &[WireLine],
    bindings: &[WireBinding],
    input: &InputPolicy,
) -> Result<(), String> {
    if lines.len() > MAX_FRAME_LINES {
        return Err(format!("plugin frame exceeds {MAX_FRAME_LINES} lines"));
    }
    if bindings.len() > MAX_BINDINGS {
        return Err(format!("plugin frame exceeds {MAX_BINDINGS} bindings"));
    }
    if input.keys.len() > MAX_INPUT_KEYS {
        return Err(format!("plugin frame exceeds {MAX_INPUT_KEYS} input keys"));
    }

    let mut retained = 0usize;
    if let Some(title) = title {
        validate_text(title, "frame title", MAX_TITLE_BYTES)?;
        add_retained(&mut retained, title.len())?;
    }
    if let Some(counter) = counter {
        validate_text(counter, "frame counter", MAX_COUNTER_BYTES)?;
        add_retained(&mut retained, counter.len())?;
    }

    let mut spans = 0usize;
    let theme = crate::theme::Theme::default();
    for line in lines {
        spans = spans
            .checked_add(line.spans.len())
            .ok_or_else(|| "plugin frame span count overflowed".to_string())?;
        if spans > MAX_FRAME_SPANS {
            return Err(format!("plugin frame exceeds {MAX_FRAME_SPANS} spans"));
        }
        for span in &line.spans {
            validate_display_text(&span.text, "span text")?;
            add_retained(&mut retained, span.text.len())?;
            for (field, color) in [("foreground", &span.fg), ("background", &span.bg)] {
                if let Some(color) = color {
                    validate_text(color, field, MAX_COLOR_BYTES)?;
                    if wire_color(color, &theme).is_none() {
                        return Err(format!(
                            "plugin span has an unknown {field} colour `{color}`"
                        ));
                    }
                    add_retained(&mut retained, color.len())?;
                }
            }
        }
    }

    for binding in bindings {
        validate_text(&binding.key, "binding key", MAX_BINDING_KEY_BYTES)?;
        validate_text(&binding.action, "binding action", MAX_BINDING_ACTION_BYTES)?;
        if binding_mentions_host_key(&binding.key, input.capture) {
            return Err(format!(
                "plugin binding `{}` is owned by the Mirador shell",
                binding.key
            ));
        }
        add_retained(&mut retained, binding.key.len())?;
        add_retained(&mut retained, binding.action.len())?;
    }

    for key in &input.keys {
        validate_text(key, "input key", MAX_BINDING_KEY_BYTES)?;
        if is_ctrl_c_chord(key) || (!input.capture && host_owned_chord(key)) {
            return Err(format!(
                "plugin input key `{key}` is owned by the Mirador shell"
            ));
        }
        add_retained(&mut retained, key.len())?;
    }
    Ok(())
}

fn validate_text(text: &str, field: &str, max_bytes: usize) -> Result<(), String> {
    if text.len() > max_bytes {
        return Err(format!("plugin {field} exceeds {max_bytes} UTF-8 bytes"));
    }
    validate_display_text(text, field)
}

fn validate_display_text(text: &str, field: &str) -> Result<(), String> {
    if text.chars().any(char::is_control) {
        return Err(format!(
            "plugin {field} contains a terminal control character"
        ));
    }
    Ok(())
}

fn add_retained(total: &mut usize, bytes: usize) -> Result<(), String> {
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| "plugin frame text size overflowed".to_string())?;
    if *total > MAX_RETAINED_FRAME_TEXT_BYTES {
        return Err(format!(
            "plugin frame retains more than {MAX_RETAINED_FRAME_TEXT_BYTES} UTF-8 text bytes"
        ));
    }
    Ok(())
}

fn stderr_loop(
    mut stderr: impl io::Read,
    shared: &Arc<Mutex<Shared>>,
    supervisor: &mpsc::Sender<SupervisorCommand>,
) {
    let mut buffer = [0u8; 1024];
    let mut window_started = Instant::now();
    let mut window_bytes = 0usize;
    loop {
        let read = match stderr.read(&mut buffer) {
            Ok(0) => return,
            Ok(read) => read,
            Err(error) => {
                shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .fail(format!("reading plugin stderr: {error}"));
                let _ = supervisor.send(SupervisorCommand::Abort);
                return;
            }
        };
        let now = Instant::now();
        if now.saturating_duration_since(window_started) >= MESSAGE_RATE_WINDOW {
            window_started = now;
            window_bytes = 0;
        }
        window_bytes = window_bytes.saturating_add(read);
        if window_bytes > MAX_STDERR_BYTES_PER_WINDOW {
            shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .fail(format!(
                    "plugin exceeded {MAX_STDERR_BYTES_PER_WINDOW} stderr bytes in one second"
                ));
            let _ = supervisor.send(SupervisorCommand::Abort);
            return;
        }

        let text: String = String::from_utf8_lossy(&buffer[..read])
            .chars()
            .map(|character| match character {
                '\n' | '\r' => '\n',
                '\t' => ' ',
                control if control.is_control() => '\u{fffd}',
                printable => printable,
            })
            .collect();
        let mut shared = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        shared.stderr.push_str(&text);
        if shared.stderr.len() > MAX_STDERR_BYTES {
            let excess = shared.stderr.len() - MAX_STDERR_BYTES;
            let mut boundary = excess;
            while boundary < shared.stderr.len() && !shared.stderr.is_char_boundary(boundary) {
                boundary += 1;
            }
            shared.stderr.drain(..boundary);
        }
        shared.changed();
    }
}

fn supervisor_loop(
    child: &mut Child,
    commands: &Receiver<SupervisorCommand>,
    shared: &Arc<Mutex<Shared>>,
    startup_deadline: Instant,
) {
    let mut deadline = None;
    let mut output_closed = None;
    loop {
        match commands.try_recv() {
            Ok(SupervisorCommand::Abort) => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            Ok(SupervisorCommand::Shutdown) => {
                deadline = Some(Instant::now() + SHUTDOWN_GRACE);
                let mut shared = shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !matches!(shared.phase, Phase::Exited(_) | Phase::Failed(_)) {
                    shared.phase = Phase::Stopping;
                    shared.changed();
                }
            }
            Ok(SupervisorCommand::OutputClosed) => {
                output_closed = Some(Instant::now() + OUTPUT_CLOSE_GRACE);
            }
            Err(TryRecvError::Disconnected) if deadline.is_none() => {
                deadline = Some(Instant::now() + SHUTDOWN_GRACE);
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                let mut shared = shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !matches!(shared.phase, Phase::Stopping | Phase::Failed(_)) {
                    if shared.ready {
                        shared.phase = Phase::Exited(status.to_string());
                    } else {
                        shared.phase =
                            Phase::Failed(format!("plugin exited before `ready`: {status}"));
                    }
                    shared.changed();
                }
                return;
            }
            Ok(None) => {}
            Err(error) => {
                shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .fail(format!("checking plugin process: {error}"));
                return;
            }
        }

        let startup_timed_out = expire_startup(shared, Instant::now(), startup_deadline);
        if startup_timed_out {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }

        if output_closed.is_some_and(|when| Instant::now() >= when) {
            let should_fail = {
                let mut shared = shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if matches!(shared.phase, Phase::Stopping | Phase::Failed(_)) {
                    false
                } else {
                    shared.fail("plugin closed stdout without exiting");
                    true
                }
            };
            if should_fail {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            output_closed = None;
        }

        if deadline.is_some_and(|when| Instant::now() >= when) {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn expire_startup(shared: &Arc<Mutex<Shared>>, now: Instant, deadline: Instant) -> bool {
    let mut shared = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if now >= deadline && !shared.ready && matches!(shared.phase, Phase::Starting) {
        shared.fail(format!(
            "plugin did not send `ready` within {} seconds",
            STARTUP_TIMEOUT.as_secs()
        ));
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{WireBinding, WireSpan};

    fn span(text: impl Into<String>) -> WireSpan {
        WireSpan {
            text: text.into(),
            fg: None,
            bg: None,
            bold: false,
            dim: false,
            italic: false,
            underlined: false,
            reversed: false,
        }
    }

    #[test]
    fn inbound_message_rate_has_a_rolling_bound() {
        let start = Instant::now();
        let mut rate = MessageRate::default();
        for _ in 0..MAX_MESSAGES_PER_WINDOW {
            assert!(rate.accepts(start, 1));
        }
        assert!(!rate.accepts(start, 1));
        assert!(rate.accepts(start + MESSAGE_RATE_WINDOW, 1));

        let mut bytes = MessageRate::default();
        assert!(bytes.accepts(start, MAX_STDOUT_BYTES_PER_WINDOW));
        assert!(!bytes.accepts(start, 1));
    }

    #[test]
    fn startup_deadline_fails_only_an_unready_process() {
        let now = Instant::now();
        let waiting = Arc::new(Mutex::new(Shared::starting()));
        assert!(expire_startup(&waiting, now, now));
        assert!(matches!(
            waiting.lock().unwrap().phase,
            Phase::Failed(ref error) if error.contains("within 5 seconds")
        ));

        let ready = Arc::new(Mutex::new(Shared::starting()));
        ready.lock().unwrap().ready = true;
        assert!(!expire_startup(&ready, now, now));
    }

    #[test]
    fn concurrent_workers_preserve_the_first_terminal_error() {
        let mut shared = Shared::starting();
        shared.fail("invalid plugin message");
        let generation = shared.generation;
        shared.fail("writing to plugin: broken pipe");
        assert_eq!(shared.generation, generation);
        assert_eq!(shared.phase, Phase::Failed("invalid plugin message".into()));
    }

    #[test]
    fn stderr_is_sanitised_retained_and_rate_bounded() {
        let shared = Arc::new(Mutex::new(Shared::starting()));
        let (supervisor, _commands) = mpsc::channel();
        stderr_loop("safe\u{1b}[2J".as_bytes(), &shared, &supervisor);
        assert_eq!(shared.lock().unwrap().stderr, "safe�[2J");

        let shared = Arc::new(Mutex::new(Shared::starting()));
        stderr_loop(
            &vec![b'x'; MAX_STDERR_BYTES_PER_WINDOW + 1][..],
            &shared,
            &supervisor,
        );
        assert!(matches!(
            shared.lock().unwrap().phase,
            Phase::Failed(ref error) if error.contains("stderr bytes")
        ));
    }

    #[test]
    fn outbound_messages_obey_the_same_byte_limit() {
        let event = HostMessage::Paste {
            text: "x".repeat(MAX_MESSAGE_BYTES),
        };
        let error = write_host_message(Vec::new(), &event).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn frames_cannot_smuggle_terminal_controls_or_retain_unbounded_text() {
        let controls = [WireLine {
            spans: vec![span("safe\u{1b}[2Jnot safe")],
        }];
        let error =
            validate_frame(None, None, &controls, &[], &InputPolicy::default()).unwrap_err();
        assert!(error.contains("control character"), "{error}");

        let oversized = [WireLine {
            spans: vec![span("x".repeat(MAX_RETAINED_FRAME_TEXT_BYTES + 1))],
        }];
        let error =
            validate_frame(None, None, &oversized, &[], &InputPolicy::default()).unwrap_err();
        assert!(error.contains("retains more"), "{error}");
    }

    #[test]
    fn passive_frames_cannot_advertise_shell_bindings() {
        let input = InputPolicy {
            keys: vec!["q".into()],
            ..InputPolicy::default()
        };
        let error = validate_frame(None, None, &[], &[], &input).unwrap_err();
        assert!(error.contains("owned by the Mirador shell"), "{error}");

        let bindings = [WireBinding {
            key: "Ctrl+C / Ctrl+X".into(),
            action: "interrupt".into(),
            primary: true,
        }];
        let capturing = InputPolicy {
            capture: true,
            ..InputPolicy::default()
        };
        let error = validate_frame(None, None, &[], &bindings, &capturing).unwrap_err();
        assert!(error.contains("owned by the Mirador shell"), "{error}");
    }
}
