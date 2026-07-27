//! Preferences changed from the UI, remembered across restarts.
//!
//! mirador never *reserialises* its config — a round trip through `toml` would
//! discard every comment in it — so a setting changed with a keystroke has
//! nowhere obvious to go unless it is worth the surgery [`crate::layout_edit`]
//! does for `[layout]`. The watchlist answered this first by
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
//! Deliberately *not* here: anything that belongs to `[layout]` — which panels
//! exist and how wide they are. Those are written back to the config itself by
//! [`crate::layout_edit`], because `[layout]` is the part of the config people
//! actually read and curate, and a state file shadowing it would leave the
//! config describing a dashboard nobody is looking at. Preferences have no such
//! problem: nobody keeps their preferred sort order under version control.

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
    /// The preferences a config expresses, used as the baseline to compare
    /// against.
    ///
    /// Must be taken from the config **as loaded from disk**, before
    /// [`crate::config::Config::apply_state`] has folded any remembered values
    /// into it —
    /// otherwise the baseline already contains last session's changes and
    /// nothing can ever be seen to differ from it.
    ///
    /// Values are normalised the way the panels normalise them, so a config
    /// holding a sort mode in some other spelling does not read as a change the
    /// user made.
    pub fn from_config(config: &crate::config::Config) -> Self {
        let sort = config
            .todo
            .sort
            .parse::<crate::task::SortMode>()
            .unwrap_or_default();
        Self {
            weather_units: Some(if config.weather.units == "metric" {
                "metric".into()
            } else {
                "imperial".into()
            }),
            todo_sort: Some(sort.label().to_string()),
            todo_show_completed: Some(config.todo.show_completed),
            clocks_show_seconds: Some(config.clocks.show_seconds),
            pomodoro_focus_minutes: Some(config.pomodoro.focus_minutes),
            pomodoro_short_break_minutes: Some(config.pomodoro.short_break_minutes),
            pomodoro_long_break_minutes: Some(config.pomodoro.long_break_minutes),
        }
    }

    /// Drop every field that matches `baseline`, keeping only real changes.
    ///
    /// This is what makes a preference retractable. Panels report their current
    /// values unconditionally and the comparison happens here, in one place, so
    /// setting a value back to what the config says *removes* it from the file
    /// rather than leaving the old change asserted forever.
    ///
    /// Panels used to do the comparison themselves against the value they were
    /// constructed with. That is subtly different and wrong: after
    /// `apply_state`, a panel is built from the *remembered* value, so once a
    /// preference had been changed the panel could never again see it as equal
    /// to the config's — and writing nothing left the previous entry standing.
    #[must_use]
    pub fn only_changes_from(mut self, baseline: &Self) -> Self {
        macro_rules! keep_if_changed {
            ($($field:ident),+ $(,)?) => {
                $(if self.$field == baseline.$field {
                    self.$field = None;
                })+
            };
        }
        keep_if_changed!(
            weather_units,
            todo_sort,
            todo_show_completed,
            clocks_show_seconds,
            pomodoro_focus_minutes,
            pomodoro_short_break_minutes,
            pomodoro_long_break_minutes,
        );
        self
    }
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

    /// Write atomically; see [`crate::store::write_atomic`].
    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self).context("serialising ui state")?;
        let contents = format!(
            "# mirador remembers preferences you changed from the keyboard here.\n\
             # Config seeds these; this file records where you moved since.\n\
             # Safe to delete: everything falls back to your config.\n\n{body}"
        );

        crate::store::write_atomic(path, &contents)
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

    /// A config with known preference values to diff against.
    fn baseline_config() -> crate::config::Config {
        let mut config = crate::config::Config::default();
        config.weather.units = "imperial".into();
        config.todo.sort = "smart".into();
        config.todo.show_completed = false;
        config.clocks.show_seconds = true;
        config.pomodoro.focus_minutes = 25;
        config
    }

    #[test]
    fn a_preference_toggled_back_is_forgotten() {
        // The bug this whole mechanism was rebuilt around. Panels used to
        // compare against the value they were *built* with — but after
        // `apply_state` that is the remembered value, so once a preference had
        // been changed the panel could never see it as equal to the config's
        // again. Writing nothing then left the old entry standing, and the
        // state file went on asserting a change the user had undone.
        let baseline = UiState::from_config(&baseline_config());

        // Changed: recorded.
        let changed = UiState {
            weather_units: Some("metric".into()),
            ..baseline.clone()
        }
        .only_changes_from(&baseline);
        assert_eq!(changed.weather_units.as_deref(), Some("metric"));

        // Changed back: not recorded, which is what lets the file shrink again.
        let reverted = baseline.clone().only_changes_from(&baseline);
        assert_eq!(reverted, UiState::default());
        assert_eq!(
            reverted.weather_units, None,
            "pressing `u` twice must leave nothing behind"
        );
    }

    #[test]
    fn the_baseline_ignores_how_the_config_spelled_a_sort_mode() {
        // A config holding an unparsable sort falls back to the default, and the
        // panel reports the default's label — so the baseline has to normalise
        // the same way or an untouched panel reads as a change on every start.
        let mut config = baseline_config();
        config.todo.sort = "nonsense".into();
        let baseline = UiState::from_config(&config);

        let reported = UiState {
            todo_sort: Some(crate::task::SortMode::default().label().to_string()),
            ..baseline.clone()
        };
        assert_eq!(
            reported.only_changes_from(&baseline).todo_sort,
            None,
            "the default sort must not look like a user choice"
        );
    }

    #[test]
    fn only_the_field_that_moved_is_kept() {
        let baseline = UiState::from_config(&baseline_config());
        let current = UiState {
            pomodoro_focus_minutes: Some(40),
            ..baseline.clone()
        };
        let kept = current.only_changes_from(&baseline);

        assert_eq!(kept.pomodoro_focus_minutes, Some(40));
        assert_eq!(
            kept.pomodoro_short_break_minutes, None,
            "an untouched field must not be pinned by a neighbour changing"
        );
        assert_eq!(kept.weather_units, None);
        assert_eq!(kept.todo_sort, None);
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
