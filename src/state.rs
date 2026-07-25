//! Preferences changed from the UI, remembered across restarts.
//!
//! mirador never rewrites its config. That is what makes the config safe to
//! hand-edit and keep in git, and it is the reason a setting you change with a
//! keystroke has nowhere obvious to go. The watchlist answered this first by
//! keeping symbols in a data file of their own; this is the same answer
//! generalised, and the config's role is unchanged — it *seeds*, and this file
//! records where you have moved since.
//!
//! Every field is optional and absent means "no opinion, use the config". So a
//! state file listing one preference overrides exactly one, an empty one
//! overrides nothing, and deleting it puts everything back to what the config
//! says. That last property is the point: there must be an obvious way to undo
//! a preference you cannot remember setting.
//!
//! Deliberately *not* here: panel sizes. `Ctrl+arrow` resizing is geometry
//! rather than preference — it belongs to the same vocabulary as `[layout]`,
//! and remembering it would mean a saved width silently overruling a layout you
//! had since edited by hand. Sizes stay session-only until that conflict has an
//! answer.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The persisted preferences, all optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiState {
    /// `u` in the weather panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weather_units: Option<String>,
    /// `s` in the task panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo_sort: Option<String>,
    /// `c` in the task panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo_show_completed: Option<bool>,
    /// `s` in the clock panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clocks_show_seconds: Option<bool>,
    /// `+` and `-` in the pomodoro panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pomodoro_focus_minutes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pomodoro_short_break_minutes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pomodoro_long_break_minutes: Option<u64>,
}

impl UiState {
    /// Read the state file, treating a missing one as no preferences.
    ///
    /// A *corrupt* one is also treated as no preferences rather than as a
    /// startup failure. This file is written by the program, not by a person,
    /// so a parse error here is mirador's fault or a half-written file — and
    /// refusing to start a dashboard over a remembered sort order would be a
    /// wildly disproportionate response. The config, which a person does edit,
    /// keeps its strict parsing precisely because that error *is* actionable.
    pub fn load(path: &Path) -> Self {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        toml::from_str(&raw).unwrap_or_default()
    }

    /// Write atomically, the same temp-file-and-rename the task store uses, so
    /// an interrupted save cannot leave a truncated file behind.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating data directory {}", parent.display()))?;
        }

        let body = toml::to_string_pretty(self).context("serialising ui state")?;
        let contents = format!(
            "# mirador remembers preferences you changed from the keyboard here.\n\
             # Config seeds these; this file records where you moved since.\n\
             # Safe to delete: everything falls back to your config.\n\n{body}"
        );

        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &contents).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("replacing {} with {}", path.display(), tmp.display()))?;
        Ok(())
    }
}

/// Where the state file lives, beside the tasks and notes.
pub fn default_path(data_dir: &Path) -> PathBuf {
    data_dir.join("state.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dir(name: &str) -> (PathBuf, TempDir) {
        let dir = std::env::temp_dir().join(format!("mirador-state-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (dir.join("state.toml"), TempDir(dir))
    }

    #[test]
    fn a_missing_file_means_no_preferences() {
        let (path, _g) = dir("missing");
        assert_eq!(UiState::load(&path), UiState::default());
    }

    #[test]
    fn preferences_survive_a_save_and_reload() {
        let (path, _g) = dir("roundtrip");
        let state = UiState {
            weather_units: Some("metric".into()),
            todo_sort: Some("due".into()),
            todo_show_completed: Some(true),
            clocks_show_seconds: Some(false),
            pomodoro_focus_minutes: Some(30),
            pomodoro_short_break_minutes: Some(7),
            pomodoro_long_break_minutes: Some(20),
        };
        state.save(&path).unwrap();
        assert_eq!(UiState::load(&path), state);
    }

    #[test]
    fn only_what_was_set_is_written() {
        let (path, _g) = dir("sparse");
        let state = UiState {
            weather_units: Some("metric".into()),
            ..UiState::default()
        };
        state.save(&path).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("weather_units"));
        assert!(
            !raw.contains("todo_sort"),
            "an untouched preference must not be written, or deleting the file \
             would be the only way to go back to the config:\n{raw}"
        );
    }

    #[test]
    fn a_corrupt_file_is_ignored_rather_than_refusing_to_start() {
        let (path, _g) = dir("corrupt");
        std::fs::write(&path, "this is not toml =").unwrap();
        assert_eq!(
            UiState::load(&path),
            UiState::default(),
            "mirador writes this file; a parse error is its own fault and no \
             reason to refuse to open a dashboard"
        );
    }

    #[test]
    fn a_file_from_a_newer_version_does_not_wedge_an_older_one() {
        let (path, _g) = dir("unknown-key");
        std::fs::write(&path, "weather_units = \"metric\"\nfuture_thing = 3\n").unwrap();
        // `deny_unknown_fields` means the whole file is rejected, which drops
        // the preferences but still starts. Recorded as a test because the
        // alternative — silently keeping half a file — is worse, and because
        // the next person to add a field should know the older binary's
        // behaviour rather than discover it.
        assert_eq!(UiState::load(&path), UiState::default());
    }

    #[test]
    fn saving_creates_the_directory_if_it_is_not_there_yet() {
        let (path, _g) = dir("mkdir");
        let nested = path.parent().unwrap().join("deeper").join("state.toml");
        UiState {
            todo_show_completed: Some(true),
            ..UiState::default()
        }
        .save(&nested)
        .unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn a_save_leaves_no_temp_file_behind() {
        let (path, _g) = dir("tmp");
        UiState {
            todo_sort: Some("title".into()),
            ..UiState::default()
        }
        .save(&path)
        .unwrap();
        assert!(!path.with_extension("toml.tmp").exists());
    }

    #[test]
    fn a_remembered_preference_reaches_the_config_a_panel_is_built_from() {
        // The whole point, end to end: what a panel wrote must come back as the
        // value the next run's panels are constructed with.
        let mut config = crate::config::Config::default();
        config.weather.units = "imperial".into();
        config.todo.sort = "smart".into();
        config.pomodoro.focus_minutes = 25;

        config.apply_state(&UiState {
            weather_units: Some("metric".into()),
            todo_sort: Some("due".into()),
            pomodoro_focus_minutes: Some(40),
            ..UiState::default()
        });

        assert_eq!(config.weather.units, "metric");
        assert_eq!(config.todo.sort, "due");
        assert_eq!(config.pomodoro.focus_minutes, 40);
    }

    #[test]
    fn an_absent_preference_leaves_the_config_alone() {
        let mut config = crate::config::Config::default();
        config.weather.units = "imperial".into();
        config.apply_state(&UiState::default());
        assert_eq!(
            config.weather.units, "imperial",
            "an empty state file must change nothing, or deleting it would not \
             be a way back to the config"
        );
    }

    #[test]
    fn a_preference_that_no_longer_parses_is_dropped_rather_than_applied() {
        let mut config = crate::config::Config::default();
        config.weather.units = "imperial".into();
        config.todo.sort = "smart".into();

        // Both plausible in a file written by a future or a broken build.
        config.apply_state(&UiState {
            weather_units: Some("kelvin".into()),
            todo_sort: Some("by-vibes".into()),
            ..UiState::default()
        });

        assert_eq!(config.weather.units, "imperial");
        assert_eq!(config.todo.sort, "smart");
        config
            .validate()
            .expect("a junk state file must not produce a config that fails validation");
    }

    #[test]
    fn a_hand_edited_duration_is_clamped_into_range() {
        let mut config = crate::config::Config::default();
        config.apply_state(&UiState {
            pomodoro_focus_minutes: Some(0),
            pomodoro_long_break_minutes: Some(99_999),
            ..UiState::default()
        });

        assert_eq!(
            config.pomodoro.focus_minutes, 1,
            "zero would spin the timer"
        );
        assert_eq!(
            config.pomodoro.long_break_minutes,
            crate::widgets::pomodoro::MAX_MINUTES
        );
        config
            .validate()
            .expect("clamped durations must still satisfy validation");
    }

    #[test]
    fn the_written_file_says_how_to_get_rid_of_it() {
        let (path, _g) = dir("comment");
        UiState {
            todo_sort: Some("due".into()),
            ..UiState::default()
        }
        .save(&path)
        .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.contains("Safe to delete"),
            "someone finding this file must be told how to undo it:\n{raw}"
        );
    }
}
