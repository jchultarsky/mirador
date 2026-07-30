//! Configuration loading.
//!
//! Split three ways: this file resolves the path, reads the file and
//! validates the result; [`layout`] holds the grid; [`widgets`] holds one
//! settings struct per panel. They were one 916-line module, which is where a
//! misspelled `[theme]` key hid long enough to make its own migration hint
//! unreachable.
//!
//! Mirador reads a single TOML file. On first run, if no file exists, a fully
//! commented default is written to disk so there is always something to edit.
//!
//! Resolution order for the config path:
//! 1. `--config <PATH>` on the command line
//! 2. `$MIRADOR_CONFIG`
//! 3. `$XDG_CONFIG_HOME/mirador/config.toml` (or the platform equivalent)

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::theme::Theme;

/// The longest poll interval any panel will accept, in minutes.
///
/// Past any legitimate setting and comfortably inside the `u64` arithmetic that
/// turns it into a `Duration`.
const A_YEAR_IN_MINUTES: u64 = 365 * 24 * 60;

/// The default config written on first run.
pub const DEFAULT_CONFIG: &str = include_str!("../../assets/default_config.toml");

mod layout;
mod widgets;

// One flat namespace: the split into files is for readability, not a claim
// that `[layout]` and `[weather]` are different concepts. Everything a reader
// or a widget names lives at `crate::config::`.
//
// `LayoutPanel`, `LayoutRow` and `ClockZone` are named only by tests today —
// production code builds them through serde and reaches them through their
// parents — so the non-test build sees the re-export as unused. That is a fact
// about who *names* the type, not about whether it is part of the surface.
#[allow(unused_imports)]
pub use layout::{Layout, LayoutPanel, LayoutRow};
#[allow(unused_imports)]
pub use widgets::{
    AgendaConfig, CalendarConfig, ClockZone, ClocksConfig, CpuConfig, NetworkConfig, NewsConfig,
    NewsFeed, NotesConfig, PomodoroConfig, StocksConfig, TodoConfig, WeatherConfig,
};

/// Top-level configuration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    /// The resolved theme.
    ///
    /// Holds whatever `[theme]` said, or — when the config named one with
    /// `theme = "nord"` — a placeholder carrying only that name until
    /// [`Config::load`] resolves it. Nothing but `load` should ever see the
    /// placeholder; `--print-config` and the tests go through it too.
    #[serde(deserialize_with = "crate::theme::de_theme")]
    pub theme: Theme,
    pub layout: Layout,
    pub clocks: ClocksConfig,
    pub weather: WeatherConfig,
    pub todo: TodoConfig,
    pub notes: NotesConfig,
    pub stocks: StocksConfig,
    pub agenda: AgendaConfig,
    pub calendar: CalendarConfig,
    pub news: NewsConfig,
    pub pomodoro: PomodoroConfig,
    pub cpu: CpuConfig,
    pub network: NetworkConfig,
}

/// Global behaviour.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
// Four independent on/off settings with nothing in common but their type. A
// flags struct is the clearest representation of that; there is no state
// machine hiding here, and grouping them into an enum would invent a
// relationship the settings do not have.
#[allow(clippy::struct_excessive_bools)]
pub struct General {
    /// Frame budget in milliseconds. Lower is smoother and burns more CPU.
    pub tick_rate_ms: u64,
    /// Draw a border around each panel.
    pub show_borders: bool,
    /// Show the key-hint line at the bottom of the screen.
    pub show_status_bar: bool,
    /// Report mouse clicks and scrolling to the dashboard.
    ///
    /// This is a genuine trade: while mirador holds the mouse, the terminal's
    /// own click-to-select-text stops working, and copying a value off the
    /// dashboard needs the terminal's override modifier (Shift in most, Option
    /// in macOS Terminal and iTerm2). Set to `false` to keep selection.
    pub mouse: bool,
    /// Ask crates.io, once a day, whether a newer mirador exists.
    ///
    /// **Off, and staying off unless you turn it on.** The README promises that
    /// mirador does not phone home; an update check is a request that tells a
    /// third party your IP address and that you run this program, on a schedule
    /// you did not pick. Small, and still the thing that promise was about.
    ///
    /// `NO_UPDATE_CHECK` or `DO_NOT_TRACK` in the environment override this
    /// even when it is true — on a managed machine the person who set those may
    /// not be the person who wrote the config.
    pub check_for_updates: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            tick_rate_ms: 250,
            check_for_updates: false,
            show_borders: true,
            show_status_bar: true,
            mouse: true,
        }
    }
}

impl Config {
    /// Load the config, creating a commented default if none exists.
    pub fn load(explicit: Option<PathBuf>) -> Result<(Self, PathBuf)> {
        let path = match explicit {
            Some(p) => p,
            None => Self::default_path()?,
        };

        if !path.exists() {
            crate::store::write_atomic(&path, DEFAULT_CONFIG)
                .with_context(|| format!("writing default config to {}", path.display()))?;
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let mut config: Self = toml::from_str(&raw).map_err(|e| stale_config_hint(&e, &path))?;
        config.resolve_theme(&path)?;
        config.validate()?;
        Ok((config, path))
    }

    /// Turn `theme = "name"` into the colours it stands for.
    ///
    /// Separate from deserializing because that must not touch the filesystem,
    /// and because the themes directory is not known until the config's own
    /// path is — a `--config` somewhere unusual looks for its themes beside it.
    fn resolve_theme(&mut self, config_path: &Path) -> Result<()> {
        let Some(name) = self.theme.name.clone() else {
            return Ok(());
        };
        let dir = crate::themes::user_dir(config_path);
        self.theme = crate::themes::resolve(&name, dir.as_deref())?;
        Ok(())
    }

    /// Platform-appropriate config location.
    pub fn default_path() -> Result<PathBuf> {
        if let Ok(from_env) = std::env::var("MIRADOR_CONFIG") {
            return Ok(PathBuf::from(from_env));
        }
        let dir = dirs::config_dir()
            .context("could not determine a config directory for this platform")?;
        Ok(dir.join("mirador").join("config.toml"))
    }

    /// Where task data lives when `[todo].file` is unset.
    pub fn default_data_path() -> Result<PathBuf> {
        Ok(Self::default_data_dir()?.join("todos.toml"))
    }

    /// Replace the config at `path` with the shipped defaults, keeping the old
    /// one. Returns where the old one went, or `None` if there was no file to
    /// keep.
    ///
    /// **This destroys work**, which is why it takes a backup rather than
    /// overwriting and why the caller is expected to have asked first. The
    /// reader reaching for it is usually stuck rather than finished: `[layout]`
    /// is the part people curate by hand, and a config broken by one typo still
    /// holds an evening's arrangement.
    ///
    /// Backups follow `migrate`'s convention — `config.toml.bak` beside the
    /// original — with one deliberate difference: an existing backup is never
    /// clobbered. Resetting twice is exactly what a stuck user does, and with a
    /// fixed name the second run would replace their real config with the
    /// defaults written by the first, which is the one outcome this whole
    /// function exists to prevent.
    pub fn reset(path: &Path) -> Result<Option<PathBuf>> {
        let backup = match path.try_exists() {
            Ok(true) => {
                let to = free_backup_path(path);
                std::fs::copy(path, &to).with_context(|| {
                    format!("backing up {} to {}", path.display(), to.display())
                })?;
                Some(to)
            }
            // Nothing to keep. Writing the defaults is still the right outcome:
            // the reader asked for a config they can edit, and not having one is
            // a reason to write it rather than to refuse.
            Ok(false) => None,
            Err(e) => anyhow::bail!("could not check whether {} exists: {e}", path.display()),
        };

        crate::store::write_atomic(path, DEFAULT_CONFIG)
            .with_context(|| format!("writing default config to {}", path.display()))?;
        Ok(backup)
    }

    /// Platform data directory for mirador's own files.
    fn default_data_dir() -> Result<PathBuf> {
        let dir =
            dirs::data_dir().context("could not determine a data directory for this platform")?;
        Ok(dir.join("mirador"))
    }

    /// Reject configs that would produce an unusable dashboard, with a message
    /// that says how to fix it rather than just what is wrong.
    /// `pub(crate)` so the state tests can assert that no remembered preference,
    /// however mangled, can produce a config that would have been rejected had
    /// it come from the file.
    pub(crate) fn validate(&self) -> Result<()> {
        if self.layout.rows.is_empty() {
            anyhow::bail!(
                "`[layout]` has no rows, so there is nothing to draw. \
                 Add at least one `{{ height = 100, panels = [...] }}` entry to `rows`."
            );
        }
        for row in &self.layout.rows {
            if row.panels.is_empty() {
                anyhow::bail!(
                    "a layout row has an empty `panels` list. \
                     Remove the row, or give it a panel such as \
                     `{{ widget = \"todo\", width = 100 }}`."
                );
            }
            for panel in &row.panels {
                if !crate::widgets::is_known_widget(&panel.widget) {
                    anyhow::bail!(
                        "unknown widget `{}`. Available widgets: {}.",
                        panel.widget,
                        crate::widgets::WIDGET_NAMES.join(", ")
                    );
                }
            }
        }
        if !matches!(self.weather.units.as_str(), "metric" | "imperial") {
            anyhow::bail!(
                "`[weather].units` is `{}`; expected `metric` or `imperial`.",
                self.weather.units
            );
        }
        // A zero-length phase would end on the tick it started and spin the
        // timer through the cycle; a zero-round set would divide by zero
        // deciding when the long break falls. Both are caught here rather than
        // clamped silently, because a `0` in a config is someone's intent, not
        // a typo to guess at.
        for (key, minutes) in [
            ("focus_minutes", self.pomodoro.focus_minutes),
            ("short_break_minutes", self.pomodoro.short_break_minutes),
            ("long_break_minutes", self.pomodoro.long_break_minutes),
        ] {
            if minutes == 0 {
                anyhow::bail!("`[pomodoro].{key}` is 0; a phase needs at least one minute.");
            }
        }
        if self.pomodoro.rounds_before_long_break == 0 {
            anyhow::bail!(
                "`[pomodoro].rounds_before_long_break` is 0; a set needs at least one focus \
                 interval before the long break."
            );
        }

        // Poll intervals are multiplied out to seconds and then to a `Duration`,
        // and both `.max(1)` guards only the low end. An absurd value from a
        // hand-edited config overflows the multiply in release, wraps to a tiny
        // interval, and turns a polite once-every-thirty-minutes fetch into a
        // tight loop against somebody else's free API — the one failure mode
        // that costs a stranger rather than the user.
        //
        // A year is well past any legitimate setting and comfortably inside the
        // arithmetic.
        if self.weather.refresh_minutes > A_YEAR_IN_MINUTES {
            anyhow::bail!(
                "`[weather].refresh_minutes` is {}; the maximum is {A_YEAR_IN_MINUTES} \
                 (one year). Leave it out to use the default of 30.",
                self.weather.refresh_minutes
            );
        }
        if self.stocks.refresh_secs > A_YEAR_IN_MINUTES * 60 {
            anyhow::bail!(
                "`[stocks].refresh_secs` is {}; the maximum is {} (one year). \
                 Leave it out to use the default of 120.",
                self.stocks.refresh_secs,
                A_YEAR_IN_MINUTES * 60
            );
        }

        Ok(())
    }

    /// Resolve the task file path, expanding a leading `~`.
    pub fn todo_path(&self) -> Result<PathBuf> {
        match &self.todo.file {
            Some(p) => Ok(expand_tilde(p)),
            None => Self::default_data_path(),
        }
    }

    /// Resolve the notes file path, expanding a leading `~`.
    pub fn notes_path(&self) -> Result<PathBuf> {
        match &self.notes.file {
            Some(p) => Ok(expand_tilde(p)),
            None => Ok(Self::default_data_dir()?.join("notes.toml")),
        }
    }

    /// Resolve the agenda's `.ics` path, expanding a leading `~`.
    ///
    /// Falls back to `calendar.ics` beside the task and note files. Nothing
    /// creates it — unlike those two, an empty calendar seeded by mirador would
    /// be a lie about your day. The panel says which path it looked at, so an
    /// unset `file` reads as a thing to configure rather than as a fault.
    pub fn agenda_path(&self) -> Result<PathBuf> {
        match &self.agenda.file {
            Some(p) => Ok(expand_tilde(p)),
            None => Ok(Self::default_data_dir()?.join("calendar.ics")),
        }
    }

    /// Resolve the watchlist file path, expanding a leading `~`.
    pub fn stocks_path(&self) -> Result<PathBuf> {
        match &self.stocks.file {
            Some(p) => Ok(expand_tilde(p)),
            None => Ok(Self::default_data_dir()?.join("watchlist.toml")),
        }
    }

    /// Where the world clocks live. Like the watchlist, this is a data file
    /// rather than config, because the panel edits it; `[clocks].zones` seeds
    /// it on a first run and is not read again.
    pub fn zones_path() -> Result<PathBuf> {
        Ok(Self::default_data_dir()?.join("zones.toml"))
    }

    /// Where remembered UI preferences live. Not configurable: it is mirador's
    /// own bookkeeping rather than something you curate, and a config key
    /// pointing at it would invite exactly the confusion this file avoids.
    /// Where the update check caches its answer, beside the state file.
    pub fn update_cache_path() -> Result<PathBuf> {
        Ok(crate::update::default_path(&Self::default_data_dir()?))
    }

    pub fn state_path() -> Result<PathBuf> {
        Ok(crate::state::default_path(&Self::default_data_dir()?))
    }

    /// Apply remembered preferences over the config.
    ///
    /// Runs before any panel is built, so panels see a config that already
    /// reflects where the user left things and need no loading code of their
    /// own. An absent field means the config keeps its say.
    ///
    /// Values are *validated* rather than trusted: a sort mode or unit string
    /// that no longer parses is dropped and the config's value stands. The file
    /// outlives the version that wrote it, and a preference from a build where
    /// `smart` meant something else should not take a dashboard down.
    pub fn apply_state(&mut self, state: &crate::state::UiState) {
        if let Some(units) = &state.weather_units
            && matches!(units.as_str(), "metric" | "imperial")
        {
            self.weather.units.clone_from(units);
        }
        if let Some(sort) = &state.todo_sort
            && sort.parse::<crate::task::SortMode>().is_ok()
        {
            self.todo.sort.clone_from(sort);
        }
        if let Some(show) = state.todo_show_completed {
            self.todo.show_completed = show;
        }
        if let Some(show) = state.clocks_show_seconds {
            self.clocks.show_seconds = show;
        }
        // Free text, so there is nothing to validate against here — the panel
        // that wrote it checked that the file could be read, and a file that
        // has since gone is the panel's "no agenda file" case rather than a
        // reason to discard the setting.
        if let Some(file) = &state.agenda_file {
            self.agenda.file = Some(std::path::PathBuf::from(file));
        }
        if let Some(location) = &state.weather_location {
            self.weather.location.clone_from(location);
        }
        // Durations are clamped rather than dropped: the panel already bounds
        // them, so an out-of-range figure means a hand-edited file and the
        // nearest legal value is what was meant.
        for (slot, saved) in [
            (
                &mut self.pomodoro.focus_minutes,
                state.pomodoro_focus_minutes,
            ),
            (
                &mut self.pomodoro.short_break_minutes,
                state.pomodoro_short_break_minutes,
            ),
            (
                &mut self.pomodoro.long_break_minutes,
                state.pomodoro_long_break_minutes,
            ),
        ] {
            if let Some(minutes) = saved {
                *slot = minutes.clamp(1, crate::widgets::pomodoro::MAX_MINUTES);
            }
        }
    }

    /// Apply a remembered theme name, if there is one and it still loads.
    ///
    /// Separate from [`Config::apply_state`] for the same reason
    /// [`Config::resolve_theme`] is separate from deserializing: it touches the
    /// filesystem, and it needs the config's own path to know where to look.
    ///
    /// Failure is not an error. A theme file the user has since deleted or
    /// broken must not stop the dashboard starting — the state file is written
    /// by mirador and a person who has just made their config unloadable
    /// deserves a message, but a person whose *remembered* theme has gone
    /// deserves the config's theme and no drama. The picker will show them the
    /// list again the next time they press `t`.
    pub fn apply_state_theme(&mut self, state: &crate::state::UiState, config_path: &Path) {
        let Some(name) = state.theme.as_deref() else {
            return;
        };
        let dir = crate::themes::user_dir(config_path);
        if let Ok(theme) = crate::themes::resolve(name, dir.as_deref()) {
            self.theme = theme;
        }
    }
}

/// A backup path beside `path` that nothing is using yet.
///
/// `config.toml.bak` first, matching `migrate`, then `.bak.2`, `.bak.3` and so
/// on. The counter is not tidiness: without it a second reset would overwrite
/// the first backup with the defaults the first reset had just written, and the
/// user's real config would be gone with no way back.
///
/// Racy in principle — the name is checked and then written — but the loser of
/// that race is a person running two resets at the same instant, and the bound
/// stops a directory of stale backups from making this loop for ever.
fn free_backup_path(path: &Path) -> PathBuf {
    let first = path.with_extension("toml.bak");
    if !first.exists() {
        return first;
    }
    for n in 2..1000 {
        let candidate = path.with_extension(format!("toml.bak.{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

/// Turn a parse failure into an error that says how to fix it.
///
/// The common case by far is a config written by an older version: mirador
/// creates the file once and never rewrites it, so a key that has since been
/// renamed sits there looking correct. Silently ignoring such a key is worse
/// than failing on it — it makes a stale config look like stale code, and
/// sends people hunting through git for a build that was never the problem.
fn stale_config_hint(error: &toml::de::Error, path: &Path) -> anyhow::Error {
    // Keys renamed since 0.1.0, and what replaced them.
    const RENAMED: &[(&str, &str)] = &[
        (
            "forecast_days",
            "`forecast_hours` — the forecast is hourly now",
        ),
        ("rx", "the `[theme.rx_gradient]` table"),
        ("tx", "the `[theme.tx_gradient]` table"),
    ];

    let message = error.to_string();

    for (old, replacement) in RENAMED {
        if message.contains(&format!("`{old}`")) {
            return anyhow::anyhow!(
                "{message}\n\nThe config at {} was written by an older version \
                 of mirador: `{old}` was replaced by {replacement}.\n\nRun \
                 `mirador --migrate-config` to update it in place; your original \
                 is kept as a .bak file.",
                path.display(),
            );
        }
    }

    anyhow::anyhow!(
        "{message}\n\nin {}. Run `mirador --print-config` to see the current format.",
        path.display()
    )
}

/// Expand a leading `~` to the user's home directory.
fn expand_tilde(path: &Path) -> PathBuf {
    let Ok(stripped) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    dirs::home_dir().map_or_else(|| path.to_path_buf(), |home| home.join(stripped))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory named for the calling test, so the reset tests do
    /// not share files with each other or with a parallel run.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mirador-reset-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resetting_writes_the_defaults_and_keeps_the_old_config() {
        let dir = scratch("keeps");
        let path = dir.join("config.toml");
        std::fs::write(&path, "theme = \"nord\"\n# an evening's work\n").unwrap();

        let backup = Config::reset(&path)
            .unwrap()
            .expect("a backup must be kept");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "theme = \"nord\"\n# an evening's work\n",
            "the backup must be the config that was replaced, byte for byte"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The failure this guards is the one a stuck user actually walks into:
    /// reset once, still stuck, reset again. With a fixed `.bak` name the
    /// second run would copy the *defaults* over the backup and the real config
    /// would be gone for good.
    #[test]
    fn a_second_reset_does_not_destroy_the_first_backup() {
        let dir = scratch("twice");
        let path = dir.join("config.toml");
        let original = "theme = \"nord\"\n# irreplaceable\n";
        std::fs::write(&path, original).unwrap();

        let first = Config::reset(&path).unwrap().expect("first backup");
        let second = Config::reset(&path).unwrap().expect("second backup");

        assert_ne!(first, second, "the second reset must not reuse the name");
        assert_eq!(
            std::fs::read_to_string(&first).unwrap(),
            original,
            "the user's real config must still be recoverable after two resets"
        );
        assert_eq!(
            std::fs::read_to_string(&second).unwrap(),
            DEFAULT_CONFIG,
            "the second backup is what the first reset wrote"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Asking for a config where there is none is a reason to write one, not to
    /// fail: the reader wants something to edit either way.
    #[test]
    fn resetting_with_no_config_yet_writes_one_and_reports_no_backup() {
        let dir = scratch("absent");
        let path = dir.join("config.toml");

        assert!(Config::reset(&path).unwrap().is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Whatever it writes has to be a config mirador can actually load —
    /// otherwise the recovery command leaves the user exactly as stuck.
    #[test]
    fn what_a_reset_writes_is_a_loadable_config() {
        let dir = scratch("loadable");
        let path = dir.join("config.toml");
        std::fs::write(&path, "this is not = = valid toml\n").unwrap();

        Config::reset(&path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        toml::from_str::<Config>(&text).expect("a reset config must load");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shipped_default_config_parses() {
        let config: Config =
            toml::from_str(DEFAULT_CONFIG).expect("the bundled default config must always parse");
        config
            .validate()
            .expect("the bundled default config must always validate");
    }

    /// Every commented-out default in the shipped config is a real key.
    ///
    /// `shipped_default_config_parses` proves the *live* keys are real, because
    /// `deny_unknown_fields` refuses anything that is not a field. It says
    /// nothing about the commented ones — and those are the ones a user
    /// uncomments, which makes them the half of the file most likely to be
    /// acted on and the half nothing was checking. A key renamed in the code
    /// with its commented example left behind would be found by whoever
    /// uncommented it, and the error would say their config was wrong.
    ///
    /// Each is uncommented on its own rather than all at once, so a failure
    /// names the line rather than the file, and because some of them are
    /// alternatives that are not meant to be set together.
    ///
    /// This is the mechanical half of Phase 4's "every key reachable and
    /// documented": a promise that a config keeps working is not worth making
    /// about a file whose documentation nothing verifies.
    #[test]
    fn every_commented_out_default_is_a_key_that_still_exists() {
        // `# key = value`, with at most one space. An example inside a prose
        // block is indented further — `#   chime_command = [...]` — and is
        // illustration rather than a default to uncomment.
        let is_commented_default = |line: &str| -> Option<String> {
            let rest = line.strip_prefix('#')?;
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            if rest.starts_with(' ') {
                return None;
            }
            let key = rest.split('=').next()?.trim();
            let looks_like_a_key = !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            (looks_like_a_key && rest.contains('=')).then(|| rest.to_string())
        };

        let lines: Vec<&str> = DEFAULT_CONFIG.lines().collect();
        let mut checked = 0;
        for (index, line) in lines.iter().enumerate() {
            let Some(uncommented) = is_commented_default(line) else {
                continue;
            };
            checked += 1;

            let mut edited = lines.clone();
            edited[index] = &uncommented;
            let text = edited.join("\n");

            let parsed = toml::from_str::<Config>(&text).unwrap_or_else(|e| {
                panic!(
                    "line {}: uncommenting `{}` does not parse, so the shipped \
                     config documents an option that no longer exists: {e}",
                    index + 1,
                    line.trim()
                )
            });
            parsed.validate().unwrap_or_else(|e| {
                panic!(
                    "line {}: uncommenting `{}` parses but does not validate: {e}",
                    index + 1,
                    line.trim()
                )
            });
        }

        // Coverage asserted, not assumed: if the comment style ever changes,
        // this test would quietly check nothing and still pass.
        assert!(
            checked >= 6,
            "only {checked} commented-out defaults were found; the shipped \
             config had six, so either they have gone or this no longer \
             recognises them"
        );
    }

    /// The README's compatibility section makes three claims about configs.
    /// A promise is only worth making if it is checked, so they are checked
    /// here rather than believed.
    #[test]
    fn the_compatibility_promise_about_configs_holds() {
        // 1. Every option the shipped config documents is accepted. (The live
        //    keys; the commented ones are
        //    `every_commented_out_default_is_a_key_that_still_exists`.)
        toml::from_str::<Config>(DEFAULT_CONFIG).expect("the shipped config parses");

        // 2. An unknown key is refused rather than ignored, and the message
        //    names the key — "it names the key and the line" is in the README.
        let err = toml::from_str::<Config>("[weather]\nunits = \"metric\"\nunts = \"metric\"\n")
            .expect_err("an unknown key must be refused, not ignored");
        let message = err.to_string();
        assert!(
            message.contains("unts"),
            "the error must name the key the user got wrong: `{message}`"
        );

        // 3. A config that sets nothing at all is valid, which is what makes
        //    "delete anything you do not want to override" true.
        let bare: Config = toml::from_str("").expect("an empty config is valid");
        bare.validate()
            .expect("an empty config must validate, since every key has a default");
    }

    #[test]
    fn the_rust_default_layout_matches_the_shipped_one() {
        let shipped: Config = toml::from_str(DEFAULT_CONFIG).expect("must parse");

        let shape = |layout: &Layout| -> Vec<(u16, Vec<(String, u16)>)> {
            layout
                .rows
                .iter()
                .map(|r| {
                    let panels = r
                        .panels
                        .iter()
                        .map(|p| (p.widget.clone(), p.width))
                        .collect();
                    (r.height, panels)
                })
                .collect()
        };

        assert_eq!(
            shape(&shipped.layout),
            shape(&Layout::default()),
            "the shipped config and the Rust default describe different \
             dashboards. Both are first impressions — the file on a true first \
             run, the Rust default for any config that omits [layout] — so a \
             gap here means deleting one section silently removes panels."
        );
    }

    #[test]
    fn the_default_layout_places_every_widget() {
        // A widget nobody can see is a widget nobody knows exists. The startup
        // hint names what is missing, but the default should have nothing to
        // name: shipping a dashboard that hides a third of itself is a poor
        // first run, and this is exactly how notes and stocks went unseen.
        let layout = Layout::default();
        let placed: Vec<&str> = layout
            .rows
            .iter()
            .flat_map(|r| r.panels.iter().map(|p| p.widget.as_str()))
            .collect();

        for widget in crate::widgets::WIDGET_NAMES {
            assert!(
                placed.contains(widget),
                "the default layout does not place `{widget}`"
            );
        }
    }

    #[test]
    fn empty_config_falls_back_to_defaults() {
        let config: Config = toml::from_str("").expect("an empty config is valid");
        // Four since the news and watch log panels took a row of their own;
        // the figure is here to catch a default that silently loses rows, not
        // because three was ever special.
        assert_eq!(config.layout.rows.len(), 4);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn unknown_widget_is_rejected_with_a_helpful_message() {
        let config: Config =
            toml::from_str("[layout]\nrows = [{ height = 1, panels = [{ widget = \"nope\" }] }]")
                .expect("parses");
        let err = config.validate().expect_err("must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("unknown widget `nope`"), "got: {msg}");
        assert!(
            msg.contains("todo"),
            "should list valid widgets, got: {msg}"
        );
    }

    #[test]
    fn a_key_from_an_older_version_is_rejected_with_a_migration_hint() {
        // The exact failure that made a current build look like an old one.
        let err = toml::from_str::<Config>("[weather]\nforecast_days = 4")
            .map_err(|e| stale_config_hint(&e, Path::new("/tmp/config.toml")))
            .expect_err("a removed key must not be silently ignored");
        let message = format!("{err:#}");
        assert!(message.contains("forecast_days"), "got: {message}");
        assert!(message.contains("forecast_hours"), "got: {message}");
    }

    #[test]
    fn an_unrecognised_key_names_itself_rather_than_being_ignored() {
        let err = toml::from_str::<Config>("[weather]\nwibble = 4")
            .map_err(|e| stale_config_hint(&e, Path::new("/tmp/config.toml")))
            .expect_err("typos must be reported");
        assert!(format!("{err:#}").contains("wibble"));
    }

    #[test]
    fn a_misspelled_theme_key_is_reported_rather_than_ignored() {
        // `deny_unknown_fields` on `Config` only guards the top level, so
        // `[theme]` was handed to a struct that accepted anything and dropped
        // what it did not know. A one-letter slip meant a colour that never
        // changed and nothing on screen to say why.
        for source in [
            "[theme]\nacent = \"#ff0000\"",
            "[theme.rx_gradient]\nstrat = \"green\"",
        ] {
            let err = toml::from_str::<Config>(source)
                .map_err(|e| stale_config_hint(&e, Path::new("/tmp/config.toml")))
                .unwrap_err();
            let message = format!("{err:#}");
            assert!(
                message.contains("acent") || message.contains("strat"),
                "{source} was accepted, or the error did not name the key: {message}"
            );
        }
    }

    #[test]
    fn the_pre_0_1_0_theme_keys_reach_their_migration_hint() {
        // These two entries sat in `RENAMED` unreachable: `[theme] rx = ...`
        // parsed clean, so the hint telling the user to run
        // `--migrate-config` could not fire for the very keys it names.
        for key in ["rx", "tx"] {
            let err = toml::from_str::<Config>(&format!("[theme]\n{key} = \"green\""))
                .map_err(|e| stale_config_hint(&e, Path::new("/tmp/config.toml")))
                .expect_err("an old theme key must be rejected");
            let message = format!("{err:#}");
            assert!(
                message.contains("--migrate-config"),
                "`{key}` did not reach the migration hint: {message}"
            );
            assert!(
                message.contains(&format!("[theme.{key}_gradient]")),
                "`{key}` did not name its replacement: {message}"
            );
        }
    }

    #[test]
    fn an_absurd_poll_interval_is_rejected_rather_than_wrapping() {
        // `refresh_minutes * 60` and `* 60 * 2` are unchecked `u64` multiplies.
        // A value near `u64::MAX` wraps in release into a tiny interval, which
        // is a tight loop against a free API someone else pays for.
        let config: Config =
            toml::from_str(&format!("[weather]\nrefresh_minutes = {}", u64::MAX)).expect("parses");
        let err = config.validate().expect_err("must be rejected");
        assert!(
            format!("{err:#}").contains("refresh_minutes"),
            "the error must name the key: {err:#}"
        );

        let config: Config =
            toml::from_str(&format!("[stocks]\nrefresh_secs = {}", u64::MAX)).expect("parses");
        assert!(config.validate().is_err());

        // The defaults, and a generous manual setting, still pass.
        assert!(Config::default().validate().is_ok());
        let config: Config = toml::from_str("[weather]\nrefresh_minutes = 1440").expect("parses");
        assert!(config.validate().is_ok(), "a day is a legitimate setting");
    }

    #[test]
    fn bad_units_are_rejected() {
        let config: Config = toml::from_str("[weather]\nunits = \"kelvin\"").expect("parses");
        assert!(config.validate().is_err());
    }

    #[test]
    fn bad_colour_names_are_rejected_at_parse_time() {
        let err = toml::from_str::<Config>("[theme]\naccent = \"chartreuse\"")
            .expect_err("must be rejected");
        assert!(err.to_string().contains("not a colour"), "got: {err}");
    }

    #[test]
    fn tilde_expands_to_home() {
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expand_tilde(Path::new("~/x.toml")), home.join("x.toml"));
        }
        assert_eq!(expand_tilde(Path::new("/abs/x")), PathBuf::from("/abs/x"));
    }
}
