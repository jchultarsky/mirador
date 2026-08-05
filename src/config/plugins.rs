//! Explicit declarations for out-of-process panels.
//!
//! Nothing in this module discovers or starts a plugin. A declaration only
//! gives the layout registry a name and a command; the process is started if
//! and when that name is actually placed in the layout.

use serde::Deserialize;

/// One external panel command and the settings passed to it unchanged.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    /// Stable widget id used by `[layout]` and the panel picker.
    pub id: String,
    /// Executable followed by arguments. It is launched directly, never
    /// through a platform shell, so quoting behaves the same on every OS.
    pub command: Vec<String>,
    /// Plugin-owned, non-secret TOML. Mirador deliberately does not interpret
    /// its schema; credentials are collected by plugin UI, never stored here.
    #[serde(default)]
    pub config: toml::Table,
}

impl PluginConfig {
    pub(super) fn validate(&self) -> anyhow::Result<()> {
        let valid_id = self.id.chars().enumerate().all(|(index, character)| {
            if index == 0 {
                character.is_ascii_lowercase()
            } else {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '-' | '_')
            }
        });
        if self.id.is_empty() || !valid_id {
            anyhow::bail!(
                "plugin id `{}` is invalid; use a lowercase letter followed by lowercase letters, digits, `-` or `_`.",
                self.id
            );
        }
        if self.command.is_empty() || self.command[0].trim().is_empty() {
            anyhow::bail!(
                "plugin `{}` has no executable; `command` must be a non-empty array.",
                self.id
            );
        }
        Ok(())
    }
}
