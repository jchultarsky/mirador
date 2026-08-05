# External panel protocol v1

Mirador can adapt an explicitly configured child process into one dashboard
tile. The process sends text and controls; Mirador remains the only renderer
and the only owner of the real terminal.

This is a language-neutral process protocol, not a public Rust API. Mirador
owns the host and this specification. Plugin authors own everything on the
other side of the pipe: language, runtime, dependencies, packaging, network
access, plugin-specific configuration and support.

## The contract

When Mirador hosts an external panel, it keeps these permanent obligations:

- `Ctrl+C` exits Mirador in every plugin state. It is never sent to a plugin.
- Mirador fits, wraps and clips plugin text to the panel rectangle in terminal
  cells. A wide line cannot overwrite a neighbouring panel.
- Mirador enforces startup, message, rate, queue and retention bounds. A child
  that blocks, floods, crashes or sends malformed data becomes a failed panel,
  not a blocked dashboard or damaged terminal session.
- Keyboard input follows the same focus and capture arbitration as an in-tree
  panel. Mouse input goes to the panel under the pointer.
- Every external tile is labelled `EXTERNAL` in its frame. Configuration names
  the executable explicitly; Mirador never scans or discovers plugins.
- The protocol remains compatible on Mirador's side of its version.
- Mirador has no credential store or credential configuration channel. Input
  sent to a focused plugin is not persisted, logged or forwarded elsewhere by
  the host.

With no external panel both declared and placed, Mirador starts no child,
searches for no runtime and loads no additional code. Python, Lua and every
other SDK remain optional projects outside the Mirador binary.

## Scope and non-goals

An external panel is still a widget: a bounded view with bounded controls. The
protocol does not provide a PTY, raw terminal access, signal forwarding,
scrollback, terminal copy mode, arbitrary cursor addressing or drawing outside
the panel rectangle. It is not a terminal-multiplexer API, and Mirador will not
grow those capabilities to support a shell-hosting plugin.

A plugin may run ordinary background work, but it owns that work and any child
processes it creates. Mirador sends shutdown to and, when necessary, terminates
the configured plugin process; descendants are the plugin's responsibility.

Plugin commands are arbitrary code running with the user's permissions. Pipes
provide failure and rendering isolation, not a security sandbox. Configure
only commands you trust.

## Configuration and credentials

Commands are argv arrays launched directly, without a platform shell. Paths
and quoting therefore have the same meaning on Windows, Linux and macOS.

```toml
[[plugins]]
id = "example"
command = ["example-mirador-plugin", "--optional-argument"]

[plugins.config]
answer = 42

[layout]
rows = [
  { height = 100, panels = [{ widget = "example", width = 100 }] },
]
```

The id is a lowercase ASCII letter followed by lowercase letters, digits,
hyphens or underscores. It cannot duplicate another plugin or a built-in
widget id. `plugins.config` is plugin-owned TOML and reaches the child as JSON;
Mirador does not interpret its schema.

That table is configuration, not secret storage. Passwords, access tokens and
other credentials must not appear in Mirador's config, command arguments or
protocol diagnostics. A plugin that needs a login collects it through its own
panel UI, masks it itself, and owns any persistence it offers. Mirador forwards
focused key or paste input only to that plugin process and never writes it to a
Mirador file. A plugin must likewise never echo a credential in a frame, error,
Watch Log event or stderr diagnostic.

Each placement owns one child process. Stdin and stdout carry UTF-8 JSON Lines;
stderr is a diagnostic stream. On removal or normal application exit, Mirador
sends `shutdown`, allows 300 ms for cleanup, then terminates a child that
remains. A failed panel can be restarted with `r`.

## Bounds

The host applies these v1 limits independently to every panel:

| Resource | Limit |
| --- | ---: |
| One stdin or stdout JSON-line message, including newline | 8 MiB |
| Child stdout rate | 120 messages and 16 MiB per rolling second |
| Time from process spawn to `ready` | 5 seconds |
| Retained frame text | 1 MiB |
| Lines / spans in one retained frame | 4,096 / 16,384 |
| Bindings / passive input keys in one frame | 64 / 128 |
| Ready or frame title / counter | 256 / 128 UTF-8 bytes |
| Binding key / action | 64 / 256 UTF-8 bytes |
| Non-fatal or fatal error text | 1,024 UTF-8 bytes |
| Undrained Watch Log events | 64, each at most 1,024 UTF-8 bytes |
| Host-to-child input queue | 256 messages |
| Child stderr accepted / retained | 64 KiB per second / latest 8 KiB |

Only the newest complete frame is retained. A newer frame replaces the prior
one rather than entering a render queue. `refresh_ms` is clamped to 16 through
60,000 ms, so a focused interactive panel cannot turn the shell into a busy
loop.

Crossing a protocol, startup, output-rate or retained-data limit fails that
panel and terminates its child. A full host-to-child queue drops new input and
shows a panel warning; input already claimed by a capturing panel never falls
through as a Mirador global action.

## Framing and version negotiation

Each protocol message is one JSON object followed by `\n`; an unterminated
final object is malformed rather than an implicit last line. The first host
message is:

```json
{
  "type": "hello",
  "protocol": 1,
  "host_version": "1.5.0",
  "plugin": "example",
  "config": {"answer": 42},
  "cwd": "/current/working/directory"
}
```

The first child message must arrive within five seconds and be:

```json
{
  "type": "ready",
  "protocol": 1,
  "title": "Example",
  "refresh_ms": 100
}
```

The version must match exactly and `ready` is sent once. A frame or Watch Log
event before `ready`, a second `ready`, an unknown field or message type, or
malformed JSON ends the panel session.

Version 1 is a compatibility commitment. A release that advertises v1 will
not remove or redefine its fields, weaken host-owned keys, or lower its bounds.
A future incompatible shape uses a new protocol number rather than silently
changing this one.

## Frames and host-owned rendering

Frames are complete immutable snapshots, not patches. Revisions increase;
late or duplicate revisions are ignored. Omitting a frame title uses the title
negotiated by `ready` (or the configured plugin id when `ready` had none); it
does not retain an override from an older frame.

```json
{
  "type": "frame",
  "revision": 7,
  "title": "Example",
  "counter": "ready",
  "lines": [
    {"spans": [
      {"text": "hello ", "fg": "theme:text"},
      {"text": "world", "fg": "ansi:10", "bold": true}
    ]}
  ],
  "bindings": [
    {"key": "Enter", "action": "open", "primary": true}
  ],
  "input": {
    "capture": false,
    "keys": ["Enter"],
    "paste": false,
    "mouse": false
  },
  "cursor": {"column": 3, "row": 0, "visible": true}
}
```

Each `WireLine` is one logical line; line breaks are represented by another
entry. Text, titles, counters, bindings and errors may not contain terminal
control characters. Mirador rejects them rather than allowing an escape
sequence to reach the real terminal. The host word-wraps logical lines by
terminal display width, preserves span styles, clips to the available height,
and replaces a glyph that cannot fit even one whole cell-width with an
ellipsis. Ratatui is never asked to wrap plugin text.

Cursor coordinates are zero-based viewport coordinates relative to the panel
interior. They are clipped to that rectangle and are not remapped when the
host wraps an over-wide logical line.

Every span requires `text`. Optional `fg` and `bg` values accept:

- `default` or `reset`;
- `theme:border`, `theme:border_focused`, `theme:rule`, `theme:title`,
  `theme:text`, `theme:muted`, `theme:label`, `theme:accent`, `theme:key`,
  `theme:success`, `theme:warning`, `theme:error` or `theme:track`;
- `ansi:0` through `ansi:255`;
- a ratatui colour name or `#rrggbb`.

The optional style flags are `bold`, `dim`, `italic`, `underlined` and
`reversed`. Unknown colours fail the panel rather than changing meaning with a
fallback.

A child may report a visible error:

```json
{"type":"error","message":"connection lost","fatal":false}
```

A fatal error ends the process. A non-fatal error remains as a bounded panel
notice until a later frame replaces it.

## Watch Log events

A plugin can hand a notable, completed event to Mirador's native Watch Log:

```json
{"type":"watch","text":"tests finished successfully in 42 seconds"}
```

Mirador supplies the timestamp and configured plugin id as the source. Text is
one non-empty line. When the bounded queue is full, the oldest undrained event
is discarded and the panel says so. This hook carries outcomes, not progress
updates; the Watch Log's existing high bar for what counts still applies.

## Host-to-child messages

After negotiation the host may send the following messages. `shutdown` is the
one exception to that ordering: it may follow `hello` immediately when Mirador
exits or removes a panel during startup.

```json
{"type":"resize","columns":80,"rows":24}
{"type":"focus","focused":true}
{"type":"tick"}
{"type":"paste","text":"one\ntwo"}
{"type":"shutdown"}
```

Keys include both a stable code and the canonical chord used by `input.keys`:

```json
{
  "type":"key",
  "key":"Ctrl+x",
  "code":"char",
  "text":"x",
  "modifiers":["Ctrl"]
}
```

Named canonical keys include `Enter`, `Backspace`, `Tab`, `BackTab`, `Esc`,
`Left`, `Right`, `Up`, `Down`, `Home`, `End`, `PageUp`, `PageDown`, `Delete`,
`Insert`, and `F1` through `F12`. Modifier order is `Ctrl`, `Alt`, `Shift`,
`Super`, `Hyper`, `Meta`. Character case is preserved.

Mouse coordinates are panel-relative:

```json
{
  "type":"mouse",
  "kind":"down",
  "button":"left",
  "column":4,
  "row":2,
  "modifiers":[]
}
```

Version 1 sends `down`, `scroll_up` and `scroll_down`. A `down` message names
its `left`, `middle` or `right` button; wheel messages have no button. Motion,
drag, release and horizontal-wheel traffic is deliberately not part of v1.

## Input ownership

Input policy is evaluated synchronously from the newest frame. Mirador never
waits for a child to decide whether a global key is safe.

- `capture: true` is the same explicit text-entry or modal state used by an
  in-tree panel. While focused, it receives ordinary keys and suspends ordinary
  global bindings.
- `keys` names the canonical chords a passive panel consumes. Other keys fall
  through to the shell.
- `paste` and `mouse` opt into those event classes.
- `Ctrl+C` is always consumed by the shell and exits Mirador. It is not a
  protocol capability. A plugin uses an ordinary declared chord such as
  `Ctrl+x` for its own cancel or interrupt action.

A passive plugin cannot claim or advertise the shell's `q`, `Tab`, `BackTab`,
`?`, `w`, `m`, `t`, `1` through `9`, `Ctrl+arrows`, or `Ctrl+C` bindings. In an
explicit capture state the ordinary keys are suspended just as they are for a
built-in editor; `Ctrl+C` remains reserved without exception.

After Mirador accepts a named key from a passive panel, it conservatively
captures subsequent input until a newer frame acknowledges the action. This
closes the asynchronous transition where, for example, `Enter` opens a login
form but a rapidly typed `q` arrives before the form's capture frame. Plugins
publish a new frame after handling a named passive key.
