//! Keys by name, and the map from a keypress to what it does.
//!
//! Every key in mirador was a hard-coded `match` arm until #284, and the text
//! describing it was a separate hand-written [`Binding`] — two layers joined
//! only by care. This module is the bridge for the shell's own keys: a
//! [`Key`] that can be read from a config and printed in a hint, a named
//! [`Action`] for each thing the shell does, and a [`Keymap`] holding the
//! defaults with the reader's `[keys]` laid over them. The hints are *derived*
//! from the map, so a rebound key cannot be advertised under its old name —
//! invariant 3 made structural rather than kept by hand.
//!
//! Only the shell's keys go through it so far. Panels still match their own,
//! and arrange mode, the pickers and the help overlay keep theirs; each is a
//! scope of its own to move later, the way this one moved.
//!
//! Two keys are not in the map at all, and cannot be put there: Ctrl+C, which
//! always quits (invariant 2), and Esc, which always backs out of whatever is
//! open. They are handled before anything consults the map, and a config that
//! tries to bind either is refused by name rather than half-obeyed — Zellij
//! calls some keys reserved and then lets a config shadow them anyway, which
//! is the version of this rule not to copy.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use serde::de::{self, Deserializer, SeqAccess, Visitor};

use crate::frame::Binding;

/// One key, with the modifiers held while it was pressed.
///
/// Normalised on construction, so the same keystroke compares equal however a
/// terminal chose to report it. Shift is folded into the character for
/// printable keys — a terminal sends `?` or `K`, with or without a Shift flag
/// beside it depending on the terminal — and into `BackTab` for Tab. It is
/// kept only where it is the sole thing distinguishing two keys, as on the
/// arrows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl Key {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        // Super, Hyper and Meta are dropped: a terminal that is not speaking
        // the kitty protocol never reports them, and mirador does not ask for
        // it, so a binding that needed one could never fire.
        let mut modifiers =
            modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let code = match code {
            KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => {
                modifiers.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            KeyCode::BackTab => {
                modifiers.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            KeyCode::Char(c) => {
                let shifted = modifiers.contains(KeyModifiers::SHIFT);
                modifiers.remove(KeyModifiers::SHIFT);
                if shifted && c.is_lowercase() {
                    KeyCode::Char(c.to_uppercase().next().unwrap_or(c))
                } else {
                    KeyCode::Char(c)
                }
            }
            other => other,
        };
        Self { code, modifiers }
    }

    /// Ctrl+C, the one way out that no panel and no config can take.
    fn is_ctrl_c(self) -> bool {
        self.modifiers == KeyModifiers::CONTROL && self.code == KeyCode::Char('c')
    }

    /// The modifiers as a prefix, `Ctrl+Alt+` — empty when none are held.
    fn modifier_prefix(modifiers: KeyModifiers) -> String {
        let mut prefix = String::new();
        for (flag, name) in [
            (KeyModifiers::CONTROL, "Ctrl+"),
            (KeyModifiers::ALT, "Alt+"),
            (KeyModifiers::SHIFT, "Shift+"),
        ] {
            if modifiers.contains(flag) {
                prefix.push_str(name);
            }
        }
        prefix
    }

    /// The key without its modifiers, as a hint spells it.
    fn code_name(code: KeyCode) -> String {
        match code {
            KeyCode::Char(' ') => "Space".into(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::BackTab => "Shift+Tab".into(),
            KeyCode::Enter => "Enter".into(),
            KeyCode::Esc => "Esc".into(),
            KeyCode::Backspace => "Backspace".into(),
            KeyCode::Delete => "Del".into(),
            KeyCode::Insert => "Ins".into(),
            KeyCode::Home => "Home".into(),
            KeyCode::End => "End".into(),
            KeyCode::PageUp => "PgUp".into(),
            KeyCode::PageDown => "PgDn".into(),
            KeyCode::Left => "←".into(),
            KeyCode::Right => "→".into(),
            KeyCode::Up => "↑".into(),
            KeyCode::Down => "↓".into(),
            KeyCode::F(n) => format!("F{n}"),
            // Nothing `FromStr` accepts produces any other code, so nothing
            // in a keymap can be one.
            other => format!("{other:?}"),
        }
    }
}

impl From<KeyEvent> for Key {
    fn from(event: KeyEvent) -> Self {
        Self::new(event.code, event.modifiers)
    }
}

/// Spelled the way the hints spell keys — `Ctrl+←`, `Shift+Tab`, `?` — which
/// `FromStr` also reads, so a key printed in an error can be pasted back.
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            Self::modifier_prefix(self.modifiers),
            Self::code_name(self.code)
        )
    }
}

/// Reads `q`, `?`, `tab`, `shift+tab`, `ctrl+left`, `alt+h`, `f5`, `space`.
///
/// Words rather than Helix's `C-`/`A-`/`S-`, because a config is read far more
/// often than it is written and `ctrl+left` needs no legend. Modifier and key
/// names are case-insensitive; a single character is not, since `K` and `k`
/// are different keys.
impl FromStr for Key {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        if text.is_empty() {
            return Err("an empty key name; to unbind an action, write `[]`".into());
        }
        // `+` joins modifiers to the key, and is also a key: `+` alone, or
        // `ctrl++` with a modifier.
        let (modifier_part, key_part) = if text == "+" {
            ("", "+")
        } else if let Some(modifiers) = text.strip_suffix("++") {
            (modifiers, "+")
        } else {
            text.rsplit_once('+').unwrap_or(("", text))
        };

        let mut modifiers = KeyModifiers::NONE;
        if !modifier_part.is_empty() {
            for word in modifier_part.split('+') {
                modifiers |= match word.to_ascii_lowercase().as_str() {
                    "ctrl" | "control" => KeyModifiers::CONTROL,
                    // Option is what the key is called on a Mac, and with
                    // "Use Option as Meta" set it is what a terminal sends
                    // as Alt.
                    "alt" | "option" | "opt" => KeyModifiers::ALT,
                    "shift" => KeyModifiers::SHIFT,
                    "cmd" | "command" | "super" | "win" | "meta" => {
                        return Err(format!(
                            "`{text}`: a terminal does not pass `{word}` through to \
                             the programs running in it; use ctrl or alt"
                        ));
                    }
                    _ => return Err(format!("`{text}`: `{word}` is not a modifier")),
                };
            }
        }

        let mut chars = key_part.chars();
        let code = match (chars.next(), chars.next()) {
            (Some(c), None) => match c {
                '←' => KeyCode::Left,
                '→' => KeyCode::Right,
                '↑' => KeyCode::Up,
                '↓' => KeyCode::Down,
                c if c.is_control() => {
                    return Err(format!("`{text}` is a control character, not a key"));
                }
                c => KeyCode::Char(c),
            },
            _ => named_key(key_part).ok_or_else(|| format!("`{key_part}` is not a key name"))?,
        };

        // Shift only turns a letter into a capital. On anything else it
        // turns the key into whatever the keyboard layout says — `shift+1`
        // is `!` on one keyboard and `+` on another — and the terminal sends
        // that character, not the digit with a flag on it.
        if modifiers.contains(KeyModifiers::SHIFT)
            && let KeyCode::Char(c) = code
            && !c.is_alphabetic()
        {
            return Err(format!(
                "`{text}`: write the character Shift makes rather than `shift+{c}`"
            ));
        }

        Ok(Self::new(code, modifiers))
    }
}

/// A key longer than one character, by name.
fn named_key(name: &str) -> Option<KeyCode> {
    let lower = name.to_ascii_lowercase();
    Some(match lower.as_str() {
        "tab" => KeyCode::Tab,
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        _ => {
            let n: u8 = lower.strip_prefix('f')?.parse().ok()?;
            if !(1..=12).contains(&n) {
                return None;
            }
            KeyCode::F(n)
        }
    })
}

/// What one of the shell's keys does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    FocusNext,
    FocusPrevious,
    Help,
    Quit,
    Panels,
    Arrange,
    Theme,
    ResizeWider,
    ResizeNarrower,
    ResizeTaller,
    ResizeShorter,
}

/// An action's name in `[keys]`, its default keys, and how its hint reads.
struct Meta {
    action: Action,
    name: &'static str,
    defaults: &'static [(KeyCode, KeyModifiers)],
    label: &'static str,
    primary: bool,
    /// What it does, for the key map dialog, in the imperative.
    about: &'static str,
}

/// Every action, in the order their hints appear.
///
/// The order is the status bar's, and it is deliberate: the bar shows as many
/// primary hints as fit, left to right, so what comes first is what survives a
/// narrow terminal.
const ACTIONS: &[Meta] = &[
    Meta {
        action: Action::FocusNext,
        name: "focus_next",
        defaults: &[(KeyCode::Tab, KeyModifiers::NONE)],
        label: "focus",
        primary: true,
        about: "move focus to the next panel",
    },
    Meta {
        action: Action::Help,
        name: "help",
        defaults: &[(KeyCode::Char('?'), KeyModifiers::NONE)],
        label: "keys",
        primary: true,
        about: "show the keys, then the key map",
    },
    Meta {
        action: Action::Quit,
        name: "quit",
        defaults: &[(KeyCode::Char('q'), KeyModifiers::NONE)],
        label: "quit",
        primary: true,
        about: "quit mirador",
    },
    // After `quit` deliberately. On a narrow terminal knowing how to get out
    // beats knowing how to add a panel. A notice naming this key for anyone
    // with unplaced widgets used to sit on the bar as well; it was retired,
    // and `w` being a primary is all that remains of it.
    Meta {
        action: Action::Panels,
        name: "panels",
        defaults: &[(KeyCode::Char('w'), KeyModifiers::NONE)],
        label: "panels",
        primary: true,
        about: "choose which panels are shown",
    },
    // Last of the single-key primaries, so it is the first to go when the
    // terminal is too narrow for all of them — but a primary, because the
    // alternative is what happened to the resize keys: shipped, useful, and
    // undiscoverable.
    Meta {
        action: Action::Arrange,
        name: "arrange",
        defaults: &[(KeyCode::Char('m'), KeyModifiers::NONE)],
        label: "arrange",
        primary: true,
        about: "rearrange the panels",
    },
    // Behind `m` for the same reason `m` is behind `w`, and a primary for the
    // same reason too: nineteen themes ship, and a theme nobody can find is
    // nineteen files of decoration.
    Meta {
        action: Action::Theme,
        name: "theme",
        defaults: &[(KeyCode::Char('t'), KeyModifiers::NONE)],
        label: "theme",
        primary: true,
        about: "choose a theme",
    },
    // The four resize actions are advertised as one hint when they can be —
    // see `Keymap::resize_bindings` — and one each when they cannot.
    Meta {
        action: Action::ResizeWider,
        name: "resize_wider",
        defaults: &[(KeyCode::Right, KeyModifiers::CONTROL)],
        label: "wider",
        primary: true,
        about: "widen the focused panel",
    },
    Meta {
        action: Action::ResizeNarrower,
        name: "resize_narrower",
        defaults: &[(KeyCode::Left, KeyModifiers::CONTROL)],
        label: "narrower",
        primary: true,
        about: "narrow the focused panel",
    },
    Meta {
        action: Action::ResizeTaller,
        name: "resize_taller",
        defaults: &[(KeyCode::Down, KeyModifiers::CONTROL)],
        label: "taller",
        primary: true,
        about: "make the focused panel taller",
    },
    Meta {
        action: Action::ResizeShorter,
        name: "resize_shorter",
        defaults: &[(KeyCode::Up, KeyModifiers::CONTROL)],
        label: "shorter",
        primary: true,
        about: "make the focused panel shorter",
    },
    Meta {
        action: Action::FocusPrevious,
        name: "focus_previous",
        defaults: &[(KeyCode::BackTab, KeyModifiers::NONE)],
        label: "focus back",
        primary: false,
        about: "move focus to the previous panel",
    },
];

impl Action {
    /// Whether the action resizes the focused panel.
    ///
    /// Resize keys are read before the focused panel sees the key, the way
    /// tmux reads its own, which is why they are held to a stricter rule than
    /// the others — see [`Keymap::new`].
    pub fn is_resize(self) -> bool {
        matches!(
            self,
            Self::ResizeWider | Self::ResizeNarrower | Self::ResizeTaller | Self::ResizeShorter
        )
    }

    fn meta(self) -> &'static Meta {
        ACTIONS
            .iter()
            .find(|meta| meta.action == self)
            .expect("every action has an entry in ACTIONS")
    }

    pub fn name(self) -> &'static str {
        self.meta().name
    }

    /// What the action does, for the key map dialog.
    pub fn about(self) -> &'static str {
        self.meta().about
    }

    /// The keys the action has when `[keys]` says nothing about it.
    pub fn defaults(self) -> Vec<Key> {
        self.meta()
            .defaults
            .iter()
            .map(|&(code, modifiers)| Key::new(code, modifiers))
            .collect()
    }

    /// Every action, in the order the key map dialog lists them: the order
    /// they are written in `[keys]`, which pairs each with its opposite rather
    /// than following the status bar.
    pub const LISTED: [Self; 11] = [
        Self::FocusNext,
        Self::FocusPrevious,
        Self::Help,
        Self::Quit,
        Self::Panels,
        Self::Arrange,
        Self::Theme,
        Self::ResizeWider,
        Self::ResizeNarrower,
        Self::ResizeTaller,
        Self::ResizeShorter,
    ];
}

/// One or more keys, written in a config as a string or a list of strings.
///
/// A hand-written visitor rather than `#[serde(untagged)]`, for the reason
/// the theme shim is one: an untagged enum reports "data did not match any
/// variant" for every failure, where this says which key name it could not
/// read, on the line it was written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyList(pub Vec<Key>);

impl<'de> Deserialize<'de> for KeyList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyListVisitor;

        impl<'de> Visitor<'de> for KeyListVisitor {
            type Value = KeyList;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a key such as \"ctrl+left\", or a list of them")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<KeyList, E> {
                text.parse()
                    .map(|key| KeyList(vec![key]))
                    .map_err(|e| E::custom(format!("in `[keys]`, {e}")))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<KeyList, A::Error> {
                let mut keys = Vec::new();
                while let Some(text) = seq.next_element::<String>()? {
                    keys.push(
                        text.parse()
                            .map_err(|e| de::Error::custom(format!("in `[keys]`, {e}")))?,
                    );
                }
                Ok(KeyList(keys))
            }
        }

        deserializer.deserialize_any(KeyListVisitor)
    }
}

/// `[keys]` as written: action name to keys, for the actions the reader
/// changed. Anything left out keeps its default.
///
/// A map rather than a struct with a field per action, so that an unknown
/// name can be answered with the list of real ones — and so that the panel
/// scopes, when they come, are the same shape.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct KeysConfig(pub BTreeMap<String, KeyList>);

/// The shell's keys: the defaults, with `[keys]` laid over them.
#[derive(Debug, Clone)]
pub struct Keymap {
    /// Each action's keys, in [`ACTIONS`] order. An empty list is an action
    /// the reader unbound.
    keys: Vec<(Action, Vec<Key>)>,
    /// The hints, built once, since the map cannot change while running.
    bindings: Vec<Binding>,
}

impl Keymap {
    /// Lay `config` over the defaults and check the result.
    ///
    /// Refused, with a sentence saying which line to change:
    ///
    /// - an action name that does not exist;
    /// - Ctrl+C or Esc, which are always the way out and never in the map;
    /// - `1`–`9`, which still jump to panels and are not in the map yet;
    /// - one key doing two things — including a new key colliding with a
    ///   default the reader did not touch, since silently taking the key from
    ///   the other action would make a working key stop working;
    /// - a resize key without Ctrl or Alt. Resize is read before the focused
    ///   panel, so a bare `h` would be taken from every panel that uses it,
    ///   and Shift alone is what arrange mode moves rows with.
    pub fn new(config: &KeysConfig) -> Result<Self, String> {
        for name in config.0.keys() {
            if !ACTIONS.iter().any(|meta| meta.name == name) {
                let names: Vec<&str> = ACTIONS.iter().map(|meta| meta.name).collect();
                return Err(format!(
                    "`[keys]` has no action `{name}`. The actions are: {}.",
                    names.join(", ")
                ));
            }
        }

        let mut keys: Vec<(Action, Vec<Key>)> = Vec::with_capacity(ACTIONS.len());
        for meta in ACTIONS {
            let mut list: Vec<Key> = match config.0.get(meta.name) {
                Some(KeyList(written)) => written.clone(),
                None => meta.action.defaults(),
            };
            let mut seen = Vec::new();
            list.retain(|key| {
                let first = !seen.contains(key);
                seen.push(*key);
                first
            });

            for key in &list {
                check_key(meta, *key)?;
            }
            for (other, taken) in &keys {
                if let Some(key) = list.iter().find(|key| taken.contains(key)) {
                    return Err(format!(
                        "`{key}` is bound to both `{}` and `{}` in `[keys]`. Give one \
                         of them another key, or unbind it with `{} = []`.",
                        other.name(),
                        meta.name,
                        if config.0.contains_key(meta.name) {
                            other.name()
                        } else {
                            meta.name
                        },
                    ));
                }
            }
            keys.push((meta.action, list));
        }

        let mut keymap = Self {
            keys,
            bindings: Vec::new(),
        };
        keymap.bindings = keymap.derive_bindings();
        Ok(keymap)
    }

    /// The action `key` is bound to, if any.
    pub fn action(&self, key: impl Into<Key>) -> Option<Action> {
        let key = key.into();
        self.keys
            .iter()
            .find(|(_, keys)| keys.contains(&key))
            .map(|(action, _)| *action)
    }

    /// The keys bound to `action`, first the one its hint shows.
    pub fn keys(&self, action: Action) -> &[Key] {
        self.keys
            .iter()
            .find(|(candidate, _)| *candidate == action)
            .map_or(&[], |(_, keys)| keys.as_slice())
    }

    /// Whether `action` has exactly its default keys.
    pub fn is_default(&self, action: Action) -> bool {
        self.keys(action) == action.defaults().as_slice()
    }

    /// The shell's hints, for the status bar and the help overlay.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// The resize hints on their own, for the arrange legend.
    ///
    /// One hint — `Ctrl+←→↑↓ resize` — when all four actions use the arrows
    /// under the same modifiers, which is the shape the defaults have and the
    /// shape anyone moving them off Ctrl will most likely keep. Otherwise one
    /// hint per action, since a collapsed form would be a guess at what the
    /// reader meant.
    ///
    /// Drawn rather than spelled: `Ctrl+←/→ resize width` plus `Ctrl+↑/↓
    /// resize height` is 45 cells of status bar, and the arrange legend draws
    /// `←→↑↓ move` two hints away, so the pair reads as one idea — the same
    /// arrows, plain to move and with a modifier to resize. The four arrows
    /// are East Asian Ambiguous: `unicode-width` calls them one cell and a
    /// terminal set to wide ambiguous draws two, which is invariant 10's trap
    /// from the other side. The bar drops hints whole, so the worst such a
    /// terminal costs is a hint dropped one column early.
    pub fn resize_bindings(&self) -> Vec<Binding> {
        let first = |action| self.keys(action).first().copied();
        let order = [
            (Action::ResizeNarrower, KeyCode::Left),
            (Action::ResizeWider, KeyCode::Right),
            (Action::ResizeShorter, KeyCode::Up),
            (Action::ResizeTaller, KeyCode::Down),
        ];
        let firsts: Vec<Option<Key>> = order.iter().map(|(action, _)| first(*action)).collect();
        if let Some(Some(lead)) = firsts.first()
            && firsts.iter().zip(order).all(|(key, (_, arrow))| {
                key.is_some_and(|key| key.modifiers == lead.modifiers && key.code == arrow)
            })
        {
            return vec![Binding::owned(
                format!("{}←→↑↓", Key::modifier_prefix(lead.modifiers)),
                "resize",
                true,
            )];
        }
        [
            Action::ResizeWider,
            Action::ResizeNarrower,
            Action::ResizeTaller,
            Action::ResizeShorter,
        ]
        .into_iter()
        .filter_map(|action| {
            let meta = action.meta();
            first(action).map(|key| Binding::owned(key.to_string(), meta.label, meta.primary))
        })
        .collect()
    }

    fn derive_bindings(&self) -> Vec<Binding> {
        let mut bindings = Vec::new();
        let mut aliases = Vec::new();
        let mut resize_done = false;
        for (action, keys) in &self.keys {
            let meta = action.meta();
            if action.is_resize() {
                if !resize_done {
                    bindings.extend(self.resize_bindings());
                    resize_done = true;
                }
                // A second key for a resize is an alias like any other.
                aliases.extend(
                    keys.iter()
                        .skip(1)
                        .map(|key| Binding::owned(key.to_string(), meta.label, false)),
                );
                continue;
            }
            let mut keys = keys.iter();
            if let Some(key) = keys.next() {
                bindings.push(Binding::owned(key.to_string(), meta.label, meta.primary));
            }
            aliases.extend(keys.map(|key| Binding::owned(key.to_string(), meta.label, false)));
        }
        bindings.extend(aliases);
        // Not in the map, and so not derived from it: the jump keys have no
        // action yet, and Ctrl+C is never an action at all.
        bindings.push(Binding::extra("1-9", "jump to panel"));
        bindings.push(Binding::extra("Ctrl+C", "quit"));
        bindings
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new(&KeysConfig::default()).expect("the default keymap is valid")
    }
}

/// Read `[keys]` from the config at `path` and check it, without touching
/// anything else in the file.
///
/// What the key map dialog's reload runs. Only `[keys]` is read, so a config
/// that is fine apart from a half-finished edit elsewhere still reloads its
/// keys — but a file that is not TOML at all is reported, since nothing in it
/// can be trusted to mean what it appears to.
pub fn read_keys(path: &Path) -> Result<(KeysConfig, Keymap), String> {
    #[derive(Deserialize)]
    struct KeysOnly {
        #[serde(default)]
        keys: KeysConfig,
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let only: KeysOnly = toml::from_str(&text).map_err(|e| e.to_string())?;
    let keymap = Keymap::new(&only.keys)?;
    Ok((only.keys, keymap))
}

/// Put every key back to its default by commenting out the live lines under
/// `[keys]`, with a line above them saying when and why.
///
/// `None` when there is nothing to comment out. An edit, never a rewrite, for
/// invariant 16's reason: a round trip through `toml` would throw away every
/// comment in the file. And a comment rather than a deletion, for the reason
/// the reset flags keep a `.bak`: a reader who reset by mistake gets their
/// keys back by deleting a `# `, and nothing they wrote is lost.
///
/// Checked the way `layout_edit` checks itself. The result is parsed and
/// compared with the original: everything outside `[keys]` must come out the
/// same, and `[keys]` must come out empty. Keys set some other way — an
/// inline `keys = { … }`, a dotted `keys.quit` — fail the second half, and
/// the file is left alone rather than half reset.
pub fn reset_text(text: &str, note: &str) -> Result<Option<String>, String> {
    let ending = crate::store::line_ending(text);
    let mut lines: Vec<String> = Vec::new();
    let mut in_keys = false;
    let mut depth = 0i32;
    let mut marked = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if depth == 0 && trimmed.starts_with('[') {
            in_keys = is_keys_header(trimmed);
            lines.push(line.to_string());
            continue;
        }
        let live = depth > 0 || !(trimmed.is_empty() || trimmed.starts_with('#'));
        if in_keys && live {
            if !marked {
                lines.push(format!("# {note}"));
                marked = true;
            }
            depth += bracket_depth_change(line);
            lines.push(format!("# {line}"));
        } else {
            lines.push(line.to_string());
        }
    }
    let mut edited = lines.join(ending);
    if text.ends_with('\n') {
        edited.push_str(ending);
    }

    let refused = "the keys in this config are not all written under a `[keys]` \
                   heading, so they cannot be reset automatically; comment out \
                   the lines that set them by hand";
    let parse = |text: &str| text.parse::<toml::Table>().map_err(|e| e.to_string());
    let mut before = parse(text)?;
    let mut after = parse(&edited).map_err(|_| refused.to_string())?;
    before.remove("keys");
    let left = after.remove("keys");
    if before != after || left.is_some_and(|keys| keys.as_table().is_none_or(|t| !t.is_empty())) {
        return Err(refused.into());
    }
    Ok(marked.then_some(edited))
}

/// [`reset_text`] applied to the file at `path`, written atomically. `false`
/// when the file sets no keys and so was left alone.
pub fn reset_file(path: &Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let note = format!(
        "Keys reset to the defaults by mirador on {}. What was set:",
        jiff::Zoned::now().date()
    );
    match reset_text(&text, &note)? {
        Some(edited) => crate::store::write_atomic(path, &edited)
            .map(|()| true)
            .map_err(|e| format!("{e:#}")),
        None => Ok(false),
    }
}

/// Whether a table header line is `[keys]`, allowing the spaces and trailing
/// comment TOML allows.
fn is_keys_header(trimmed: &str) -> bool {
    trimmed
        .strip_prefix('[')
        .filter(|rest| !rest.starts_with('['))
        .and_then(|rest| rest.split_once(']'))
        .is_some_and(|(name, after)| {
            name.trim() == "keys" && {
                let after = after.trim_start();
                after.is_empty() || after.starts_with('#')
            }
        })
}

/// How far `line` opens (positive) or closes (negative) an array, ignoring
/// brackets inside strings — `"["` is a key — and anything after a comment.
fn bracket_depth_change(line: &str) -> i32 {
    let mut change = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in line.chars() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '#' => break,
                '[' => change += 1,
                ']' => change -= 1,
                _ => {}
            },
        }
    }
    change
}

/// Refuse a key that `meta`'s action may not have, whoever asked for it.
fn check_key(meta: &Meta, key: Key) -> Result<(), String> {
    let name = meta.name;
    if key.is_ctrl_c() {
        return Err(format!(
            "`{name}` cannot be Ctrl+C in `[keys]`: Ctrl+C always quits, whatever \
             else is bound, so that there is always a way out."
        ));
    }
    if key.code == KeyCode::Esc {
        return Err(format!(
            "`{name}` cannot use Esc in `[keys]`: Esc always backs out of whatever \
             is open, and is not in the keymap."
        ));
    }
    if key.modifiers.is_empty() && matches!(key.code, KeyCode::Char('1'..='9')) {
        return Err(format!(
            "`{name}` cannot be `{key}` in `[keys]`: 1-9 jump straight to a panel."
        ));
    }
    if meta.action.is_resize()
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return Err(format!(
            "`{name}` is `{key}` in `[keys]`, and a resize key needs Ctrl or Alt \
             held, as in \"alt+right\": resizing is read before the focused panel sees the \
             key, so without one it would take `{key}` from every panel."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Key {
        text.parse().unwrap_or_else(|e| panic!("{text:?}: {e}"))
    }

    fn keys_config(toml_text: &str) -> KeysConfig {
        let mut tables: BTreeMap<String, KeysConfig> =
            toml::from_str(toml_text).unwrap_or_else(|e| panic!("{e}"));
        tables.remove("keys").expect("a [keys] table")
    }

    fn event(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn key_names_read_the_way_a_config_writes_them() {
        assert_eq!(key("q"), Key::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert_eq!(
            key("ctrl+left"),
            Key::new(KeyCode::Left, KeyModifiers::CONTROL)
        );
        assert_eq!(key("Ctrl+Left"), key("ctrl+left"));
        assert_eq!(key("option+right"), key("alt+right"));
        assert_eq!(
            key("shift+tab"),
            Key::new(KeyCode::BackTab, KeyModifiers::NONE)
        );
        assert_eq!(
            key("space"),
            Key::new(KeyCode::Char(' '), KeyModifiers::NONE)
        );
        assert_eq!(key("+"), Key::new(KeyCode::Char('+'), KeyModifiers::NONE));
        assert_eq!(
            key("ctrl++"),
            Key::new(KeyCode::Char('+'), KeyModifiers::CONTROL)
        );
        assert_eq!(key("f5"), Key::new(KeyCode::F(5), KeyModifiers::NONE));
        assert_eq!(key("shift+k"), key("K"));
        assert_ne!(key("K"), key("k"), "a letter's case is the key");
    }

    #[test]
    fn keys_that_cannot_arrive_are_refused_with_a_reason() {
        for (text, says) in [
            ("", "empty"),
            ("cmd+left", "does not pass"),
            ("hyper+x", "not a modifier"),
            ("ctrl+", "not a key name"),
            ("banana", "not a key name"),
            ("f13", "not a key name"),
            ("shift+1", "the character Shift makes"),
        ] {
            let error = text.parse::<Key>().expect_err(text);
            assert!(error.contains(says), "{text:?} said {error:?}");
        }
    }

    /// Every key is printed the way a hint prints it, and that spelling must
    /// read back as the same key — an error message names a key, and the
    /// reader will paste it.
    #[test]
    fn a_printed_key_reads_back_as_itself() {
        let codes = [
            KeyCode::Char('q'),
            KeyCode::Char('Q'),
            KeyCode::Char('?'),
            KeyCode::Char('+'),
            KeyCode::Char(' '),
            KeyCode::Char('é'),
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Enter,
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::F(1),
            KeyCode::F(12),
        ];
        let modifiers = [
            KeyModifiers::NONE,
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SHIFT,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ];
        for code in codes {
            for held in modifiers {
                let original = Key::new(code, held);
                let printed = original.to_string();
                assert_eq!(
                    printed.parse::<Key>(),
                    Ok(original),
                    "{printed:?} did not read back"
                );
            }
        }
    }

    /// Terminals disagree about whether a shifted character carries a Shift
    /// flag. Both reports are the same keystroke.
    #[test]
    fn a_shifted_character_matches_however_the_terminal_reports_it() {
        let map = Keymap::default();
        for held in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
            assert_eq!(
                map.action(event(KeyCode::Char('?'), held)),
                Some(Action::Help)
            );
        }
        assert_eq!(
            map.action(event(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::FocusPrevious)
        );
    }

    /// The golden test: the default map answers every key the shell's `match`
    /// arms answered before the keymap existed, and nothing it did not.
    #[test]
    fn the_default_map_is_the_keys_mirador_always_had() {
        let map = Keymap::default();
        let expected = [
            (KeyCode::Tab, KeyModifiers::NONE, Action::FocusNext),
            (KeyCode::BackTab, KeyModifiers::NONE, Action::FocusPrevious),
            (KeyCode::Char('?'), KeyModifiers::NONE, Action::Help),
            (KeyCode::Char('q'), KeyModifiers::NONE, Action::Quit),
            (KeyCode::Char('w'), KeyModifiers::NONE, Action::Panels),
            (KeyCode::Char('m'), KeyModifiers::NONE, Action::Arrange),
            (KeyCode::Char('t'), KeyModifiers::NONE, Action::Theme),
            (KeyCode::Right, KeyModifiers::CONTROL, Action::ResizeWider),
            (KeyCode::Left, KeyModifiers::CONTROL, Action::ResizeNarrower),
            (KeyCode::Down, KeyModifiers::CONTROL, Action::ResizeTaller),
            (KeyCode::Up, KeyModifiers::CONTROL, Action::ResizeShorter),
        ];
        for (code, held, action) in expected {
            assert_eq!(map.action(event(code, held)), Some(action), "{code:?}");
        }
        assert_eq!(
            ACTIONS.len(),
            expected.len(),
            "an action was added without a default in this table"
        );
        for unbound in [
            event(KeyCode::Esc, KeyModifiers::NONE),
            event(KeyCode::Char('c'), KeyModifiers::CONTROL),
            event(KeyCode::Char('1'), KeyModifiers::NONE),
            event(KeyCode::Right, KeyModifiers::NONE),
        ] {
            assert_eq!(map.action(unbound), None, "{unbound:?}");
        }
    }

    /// The hints the shell drew from a hand-written table before the keymap
    /// existed, character for character. The README's key table and three
    /// tests about the status bar were written against these.
    #[test]
    fn the_default_hints_are_the_ones_the_shell_always_drew() {
        let drawn: Vec<(String, String, bool)> = Keymap::default()
            .bindings()
            .iter()
            .map(|b| (b.key.to_string(), b.action.to_string(), b.primary))
            .collect();
        let expected = [
            ("Tab", "focus", true),
            ("?", "keys", true),
            ("q", "quit", true),
            ("w", "panels", true),
            ("m", "arrange", true),
            ("t", "theme", true),
            ("Ctrl+←→↑↓", "resize", true),
            ("Shift+Tab", "focus back", false),
            ("1-9", "jump to panel", false),
            ("Ctrl+C", "quit", false),
        ]
        .map(|(k, a, p)| (k.to_string(), a.to_string(), p));
        assert_eq!(drawn, expected);
    }

    /// #284: Ctrl+arrows switch desktops on a Mac, so the reader moves resize
    /// to Alt — one line per action — and the one hint moves with it.
    #[test]
    fn resize_moves_off_ctrl_and_its_hint_follows() {
        let map = Keymap::new(&keys_config(
            r#"
            [keys]
            resize_wider = "alt+right"
            resize_narrower = "alt+left"
            resize_taller = "alt+down"
            resize_shorter = "alt+up"
            "#,
        ))
        .expect("valid");
        assert_eq!(
            map.action(event(KeyCode::Right, KeyModifiers::ALT)),
            Some(Action::ResizeWider)
        );
        assert_eq!(
            map.action(event(KeyCode::Right, KeyModifiers::CONTROL)),
            None,
            "the old key is released, not kept as a second binding"
        );
        let resize = map.resize_bindings();
        assert_eq!(resize.len(), 1);
        assert_eq!(resize[0].key, "Alt+←→↑↓");
        assert!(map.bindings().iter().any(|b| b.key == "Alt+←→↑↓"));
        assert!(!map.bindings().iter().any(|b| b.key.contains("Ctrl+←")));
    }

    /// Resize keys that are not four arrows under one modifier are not
    /// squeezed into a collapsed hint that would misdescribe them.
    #[test]
    fn resize_keys_of_mixed_shapes_are_hinted_one_by_one() {
        let map = Keymap::new(&keys_config(
            r#"
            [keys]
            resize_wider = "alt+l"
            resize_narrower = "alt+h"
            "#,
        ))
        .expect("valid");
        let hints: Vec<String> = map
            .resize_bindings()
            .iter()
            .map(|b| format!("{} {}", b.key, b.action))
            .collect();
        assert_eq!(
            hints,
            [
                "Alt+l wider",
                "Alt+h narrower",
                "Ctrl+↓ taller",
                "Ctrl+↑ shorter"
            ]
        );
    }

    #[test]
    fn one_key_changed_leaves_the_rest_as_they_were() {
        let map = Keymap::new(&keys_config("[keys]\ntheme = \"T\"")).expect("valid");
        assert_eq!(
            map.action(event(KeyCode::Char('T'), KeyModifiers::SHIFT)),
            Some(Action::Theme)
        );
        assert_eq!(
            map.action(event(KeyCode::Char('t'), KeyModifiers::NONE)),
            None
        );
        for (action, keys) in &Keymap::default().keys {
            if *action != Action::Theme {
                assert_eq!(map.keys(*action), keys.as_slice(), "{action:?} moved");
            }
        }
    }

    #[test]
    fn an_action_can_have_several_keys_or_none() {
        let map =
            Keymap::new(&keys_config("[keys]\nquit = [\"q\", \"x\"]\ntheme = []")).expect("valid");
        for c in ['q', 'x'] {
            assert_eq!(
                map.action(event(KeyCode::Char(c), KeyModifiers::NONE)),
                Some(Action::Quit)
            );
        }
        assert!(map.keys(Action::Theme).is_empty());
        let hints = map.bindings();
        assert!(hints.iter().any(|b| b.key == "q" && b.primary));
        assert!(hints.iter().any(|b| b.key == "x" && !b.primary));
        assert!(
            !hints.iter().any(|b| b.action == "theme"),
            "an unbound action is not advertised"
        );
    }

    #[test]
    fn the_way_out_cannot_be_bound() {
        for (line, says) in [
            ("quit = \"ctrl+c\"", "Ctrl+C always quits"),
            ("help = \"esc\"", "Esc always backs out"),
            ("panels = \"3\"", "1-9 jump"),
        ] {
            let error = Keymap::new(&keys_config(&format!("[keys]\n{line}"))).expect_err(line);
            assert!(error.contains(says), "{line}: {error}");
        }
    }

    #[test]
    fn a_resize_key_must_hold_ctrl_or_alt() {
        for line in ["resize_wider = \"l\"", "resize_taller = \"shift+down\""] {
            let error = Keymap::new(&keys_config(&format!("[keys]\n{line}"))).expect_err(line);
            assert!(error.contains("needs Ctrl or Alt"), "{line}: {error}");
        }
        Keymap::new(&keys_config("[keys]\nresize_wider = \"ctrl+shift+right\""))
            .expect("Ctrl with Shift is fine");
    }

    /// Taking `q` for the theme picker while `quit` still has it would make
    /// one of the two stop working with nothing to say which. Refused, and the
    /// message names the action the reader did *not* write as the one to
    /// unbind, since that is the default they are colliding with.
    #[test]
    fn one_key_cannot_do_two_things() {
        let error = Keymap::new(&keys_config("[keys]\ntheme = \"q\"")).expect_err("clash");
        assert!(
            error.contains("`q` is bound to both `quit` and `theme`")
                && error.contains("`quit = []`"),
            "{error}"
        );
        Keymap::new(&keys_config("[keys]\ntheme = \"q\"\nquit = \"x\""))
            .expect("moving the other one out of the way resolves it");
    }

    #[test]
    fn a_misspelled_action_is_answered_with_the_real_ones() {
        let error =
            Keymap::new(&keys_config("[keys]\nresize_widder = \"alt+right\"")).expect_err("typo");
        assert!(
            error.contains("no action `resize_widder`") && error.contains("resize_wider"),
            "{error}"
        );
    }

    /// The shipped config lists every action with its default, commented
    /// out. Uncommented together they must describe exactly the default map —
    /// invariant 18's rule that the two routes to a default agree, and the
    /// test that notices an action added here and not documented there.
    #[test]
    fn the_shipped_config_documents_every_default_exactly() {
        let block = crate::config::DEFAULT_CONFIG
            .split_once("\n[keys]\n")
            .expect("the shipped config has a [keys] section")
            .1;
        let uncommented = block
            .lines()
            .filter_map(|line| line.strip_prefix("# "))
            .filter(|line| !line.starts_with(' ') && line.contains('='))
            .collect::<Vec<_>>()
            .join("\n");
        let written = keys_config(&format!("[keys]\n{uncommented}"));
        assert_eq!(
            written.0.len(),
            ACTIONS.len(),
            "every action is listed once: {:?}",
            written.0.keys().collect::<Vec<_>>()
        );
        let documented = Keymap::new(&written).expect("the documented defaults are valid");
        for (action, keys) in &Keymap::default().keys {
            assert_eq!(documented.keys(*action), keys.as_slice(), "{action:?}");
        }
    }

    const NOTE: &str = "Keys reset to the defaults by mirador on 2026-09-24. What was set:";

    /// The reset comments out what `[keys]` set, and only that: every other
    /// line of the file, comments and spacing included, comes out identical.
    #[test]
    fn a_reset_comments_out_the_keys_and_touches_nothing_else() {
        let text = "[general]\nmouse = true   # keep this\n\n[keys]\n# quit = \"q\"\n\
                    quit = [\n  \"x\",\n  \"[\",  # a key that is a bracket\n]\n\
                    theme = \"T\"\n\n[network]\ninterfaces = []\n";
        let edited = reset_text(text, NOTE)
            .expect("resets")
            .expect("something to do");
        assert_eq!(
            edited,
            "[general]\nmouse = true   # keep this\n\n[keys]\n# quit = \"q\"\n\
             # Keys reset to the defaults by mirador on 2026-09-24. What was set:\n\
             # quit = [\n#   \"x\",\n#   \"[\",  # a key that is a bracket\n# ]\n\
             # theme = \"T\"\n\n[network]\ninterfaces = []\n"
        );
    }

    #[test]
    fn a_reset_keeps_the_files_own_line_endings() {
        let text = "[keys]\r\nquit = \"x\"\r\n";
        let edited = reset_text(text, NOTE).expect("resets").expect("changed");
        assert!(!edited.replace("\r\n", "").contains('\n'), "{edited:?}");
    }

    #[test]
    fn a_config_that_sets_no_keys_has_nothing_to_reset() {
        for text in ["[general]\nmouse = true\n", crate::config::DEFAULT_CONFIG] {
            assert_eq!(reset_text(text, NOTE), Ok(None));
        }
    }

    /// Keys written somewhere other than under a `[keys]` heading cannot be
    /// found line by line, and the check that follows the edit is what says
    /// so — the file is refused rather than half reset.
    #[test]
    fn keys_the_reset_cannot_find_are_refused_rather_than_half_reset() {
        for text in [
            "keys = { quit = \"x\" }\n",
            "keys.quit = \"x\"\n[general]\nmouse = true\n",
            "[[keys]]\nquit = \"x\"\n",
        ] {
            let error = reset_text(text, NOTE).expect_err(text);
            assert!(error.contains("by hand"), "{text}: {error}");
        }
    }

    #[test]
    fn a_reset_file_reads_back_as_the_default_keymap() {
        let dir = std::env::temp_dir().join(format!("mirador-reset-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[keys]\nquit = \"x\"\n").expect("write");

        let (_, before) = read_keys(&path).expect("reads");
        assert!(!before.is_default(Action::Quit));
        assert_eq!(reset_file(&path), Ok(true));
        let (keys, after) = read_keys(&path).expect("reads");
        assert!(keys.0.is_empty(), "{keys:?}");
        assert!(Action::LISTED.iter().all(|a| after.is_default(*a)));
        assert_eq!(
            reset_file(&path),
            Ok(false),
            "a second reset has nothing to do"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reload_reports_a_bad_keymap_instead_of_applying_it() {
        let dir = std::env::temp_dir().join(format!("mirador-read-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[keys]\ntheme = \"q\"\n").expect("write");
        let error = read_keys(&path).expect_err("clash");
        assert!(error.contains("bound to both"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_action_is_listed_once_in_the_dialog_order() {
        for meta in ACTIONS {
            assert_eq!(
                Action::LISTED.iter().filter(|a| **a == meta.action).count(),
                1,
                "{}",
                meta.name
            );
        }
    }

    #[test]
    fn a_bad_key_name_is_reported_where_it_was_written() {
        let error = toml::from_str::<BTreeMap<String, KeysConfig>>("[keys]\n\nquit = \"cmd+q\"\n")
            .expect_err("cmd is not deliverable")
            .to_string();
        assert!(
            error.contains("line 3") && error.contains("does not pass"),
            "{error}"
        );
        // `main` appends `--reset-keys` to any startup error naming `[keys]`,
        // so every way a keymap can fail has to name it — a single key and a
        // list alike.
        assert!(error.contains("[keys]"), "{error}");
        let listed = toml::from_str::<BTreeMap<String, KeysConfig>>("[keys]\nquit = [\"cmd+q\"]\n")
            .expect_err("in a list too")
            .to_string();
        assert!(listed.contains("[keys]"), "{listed}");
    }
}
