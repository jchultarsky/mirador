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
mod plugins;
mod widgets;

// One flat namespace: the split into files is for readability, not a claim
// that `[layout]` and `[weather]` are different concepts. Everything a reader
// or a widget names lives at `crate::config::`.
pub use layout::{Layout, LayoutPanel, LayoutRow};
pub use plugins::PluginConfig;
pub use widgets::{
    AgendaConfig, BatteryConfig, CalculatorConfig, CalendarConfig, ClockZone, ClocksConfig,
    CpuConfig, DiskConfig, MemoryConfig, NetworkConfig, NewsConfig, NotesConfig, PomodoroConfig,
    StocksConfig, TemperatureConfig, TodoConfig, WatchlogConfig, WeatherConfig,
};
// `NewsFeed` is named by nothing outside this module, in either build: serde
// builds it and the news panel reaches it through `NewsConfig::feeds`. That is
// a fact about who *names* the type, not about whether it is part of the
// surface, so it is re-exported anyway — alone, so the allow covers it and
// nothing else. The allow used to sit over the whole list, excusing
// `ClockZone` too, which `zones.rs` names; spread that wide it would have hidden
// any of the others going dead.
#[allow(unused_imports)]
pub use widgets::NewsFeed;

/// Top-level configuration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    /// External panels are opt-in and named explicitly. An empty list performs
    /// no discovery, imports no runtime and starts no process.
    pub plugins: Vec<PluginConfig>,
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
    pub watchlog: WatchlogConfig,
    pub pomodoro: PomodoroConfig,
    pub calculator: CalculatorConfig,
    pub cpu: CpuConfig,
    pub memory: MemoryConfig,
    pub disk: DiskConfig,
    pub network: NetworkConfig,
    pub battery: BatteryConfig,
    pub temperature: TemperatureConfig,
    /// The shell's keys, where the reader has moved them. See [`crate::keymap`].
    pub keys: crate::keymap::KeysConfig,
    /// Arrange mode, whose only settings are its keys.
    pub arrange: KeysSection,
    /// The `w` picker, whose only settings are its keys. Not `[panels]`,
    /// which reads as though it configured the panels themselves.
    pub panel_picker: KeysSection,
    /// The `t` picker, whose only settings are its keys. Not `[theme]`, which
    /// is the theme.
    pub theme_picker: KeysSection,
    /// The help overlay (`?`), whose only settings are its scroll keys. Not
    /// `[help]`, which reads as the key that opens it.
    pub help_overlay: KeysSection,
}

/// A shell mode with nothing to set but its keys: arrange mode (`m`), the
/// two pickers (`w`, `t`) and the help overlay (`?`), each under
/// `[<mode>.keys]`. A section of its own
/// keeps each table beside the others of its shape rather than inventing a
/// second one.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeysSection {
    pub keys: crate::keymap::KeysConfig,
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
    /// How long to wait for an event before looking around again, in
    /// milliseconds.
    ///
    /// Not a frame budget, despite how it reads: the redraw follows visible
    /// change rather than the tick, so lowering this wakes the process more
    /// often without making anything smoother.
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
    /// Built-in and explicitly declared widget ids, in picker order.
    pub fn widget_names(&self) -> Vec<String> {
        crate::widgets::WIDGET_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .chain(self.plugins.iter().map(|plugin| plugin.id.clone()))
            .collect()
    }

    /// The declaration for an external widget, if one was explicitly named.
    pub fn plugin(&self, id: &str) -> Option<&PluginConfig> {
        self.plugins.iter().find(|plugin| plugin.id == id)
    }

    /// Whether a layout name resolves to a built-in or declared plugin.
    pub fn knows_widget(&self, id: &str) -> bool {
        crate::widgets::is_known_widget(id) || self.plugin(id).is_some()
    }

    /// Load the config at `path`, creating a commented default if none exists.
    pub fn load(path: PathBuf) -> Result<(Self, PathBuf)> {
        if !path.exists() {
            crate::store::write_atomic(&path, DEFAULT_CONFIG)
                .with_context(|| format!("writing default config to {}", path.display()))?;
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let mut config: Self =
            toml::from_str(&raw).map_err(|e| stale_config_hint(&e, &raw, &path))?;
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

    /// The config `--config` named, else `MIRADOR_CONFIG`, else the platform's.
    pub fn path_or_default(explicit: Option<PathBuf>) -> Result<PathBuf> {
        explicit.map_or_else(Self::default_path, Ok)
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
    /// Backups go where `migrate`'s do — `config.toml.bak` beside the original
    /// — and an existing backup is never clobbered: the name is numbered
    /// instead. Resetting twice is exactly what a stuck user does, and with a
    /// fixed name the second run would replace their real config with the
    /// defaults written by the first, which is the one outcome this whole
    /// function exists to prevent.
    pub fn reset(path: &Path) -> Result<Option<PathBuf>> {
        // A config that is a link — into a dotfiles repository, say — is moved
        // aside as itself, the way a factory reset moves it: the backup is the
        // link, the file it points at is left exactly as it was, and the
        // defaults go into a new file of their own. Copying through the link
        // and writing the defaults back through it put shipped defaults into
        // the reader's repository, with their curated copy outside it.
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            // A link whose file is missing is moved too, so the defaults never
            // go through it.
            let to = crate::store::move_aside(path)?;
            crate::store::write_atomic(path, DEFAULT_CONFIG)
                .with_context(|| format!("writing default config to {}", path.display()))?;
            return Ok(to);
        }
        let backup = match path.try_exists() {
            Ok(true) => {
                let to = crate::store::free_backup_path(path);
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
    #[cfg(not(test))]
    fn default_data_dir() -> Result<PathBuf> {
        let dir =
            dirs::data_dir().context("could not determine a data directory for this platform")?;
        Ok(dir.join("mirador"))
    }

    /// Under test, a data directory of the test run's own, never the
    /// reader's.
    ///
    /// The zone list and the state file live in the data directory whatever
    /// the config says, and every data file a config leaves unset falls back
    /// to it, so a test that built a panel from `Config::default()` read the
    /// reader's task list and seeded their zones, notes and watchlist when
    /// those were missing. One directory per test process, emptied as it is
    /// first resolved in case an earlier process had the same id. Nothing
    /// runs when a test process exits, so those of earlier runs are swept
    /// here once they are an hour old — far longer than the suite takes.
    #[cfg(test)]
    // It stands in for the platform resolver above, which can fail, so every
    // caller is written for a `Result` and this has to return one.
    #[allow(clippy::unnecessary_wraps)]
    fn default_data_dir() -> Result<PathBuf> {
        static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        let dir = DIR.get_or_init(|| Self::test_run_dir(&std::env::temp_dir(), std::process::id()));
        Ok(dir.clone())
    }

    /// The data directory for test process `pid`, made in `temp`.
    ///
    /// Directly in `temp`, named the way the other tests name theirs, and
    /// not inside a parent shared by every run: on Linux `temp` is usually
    /// one `/tmp` for every user, and a parent made by whoever ran the suite
    /// first is one nobody else can write into. The directory is made here
    /// rather than left to the first save, so a run that cannot have it
    /// fails saying so, not in whichever test happened to save first.
    #[cfg(test)]
    fn test_run_dir(temp: &Path, pid: u32) -> PathBuf {
        const RUN: &str = "mirador-test-data-";
        for entry in std::fs::read_dir(temp).into_iter().flatten().flatten() {
            if !entry.file_name().to_string_lossy().starts_with(RUN) {
                continue;
            }
            let stale = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > std::time::Duration::from_hours(1));
            if stale {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        let dir = temp.join(format!("{RUN}{pid}"));
        let _ = std::fs::remove_dir_all(&dir);
        // `create_dir`, not `create_dir_all`: one left by another user's run
        // with the same id could not be emptied, and must not be adopted.
        if let Err(err) = std::fs::create_dir(&dir) {
            panic!("this test run's data directory {}: {err}", dir.display());
        }
        dir
    }

    /// Reject configs that would produce an unusable dashboard, with a message
    /// that says how to fix it rather than just what is wrong.
    /// `pub(crate)` so the state tests can assert that no remembered preference,
    /// however mangled, can produce a config that would have been rejected had
    /// it come from the file.
    pub(crate) fn validate(&self) -> Result<()> {
        crate::keymap::KeyTables::from_config(self)
            .check()
            .map_err(anyhow::Error::msg)?;

        let mut plugin_ids = std::collections::HashSet::new();
        for plugin in &self.plugins {
            plugin.validate()?;
            if crate::widgets::is_known_widget(&plugin.id) {
                anyhow::bail!(
                    "plugin id `{}` conflicts with a built-in widget.",
                    plugin.id
                );
            }
            if !plugin_ids.insert(plugin.id.as_str()) {
                anyhow::bail!("plugin id `{}` is declared more than once.", plugin.id);
            }
        }

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
                if !self.knows_widget(&panel.widget) {
                    anyhow::bail!(
                        "unknown widget `{}`. Available widgets: {}.",
                        panel.widget,
                        self.widget_names().join(", ")
                    );
                }
            }
        }
        self.validate_words()?;
        // A zero-length phase would end on the tick it started and spin the
        // timer through the cycle; a zero-round set would divide by zero
        // deciding when the long break falls. Both are caught here rather than
        // clamped silently, because a `0` in a config is someone's intent, not
        // a typo to guess at.
        //
        // The top end is bounded for the poll intervals' reason below: the
        // panel multiplies a length out to seconds unchecked, so an absurd one
        // panics in a debug build and wraps in release to a phase nobody
        // chose. A remembered length cannot get past this, because
        // `apply_state` clamps it to the config's own.
        let defaults = PomodoroConfig::default();
        for (key, minutes, default) in [
            (
                "focus_minutes",
                self.pomodoro.focus_minutes,
                defaults.focus_minutes,
            ),
            (
                "short_break_minutes",
                self.pomodoro.short_break_minutes,
                defaults.short_break_minutes,
            ),
            (
                "long_break_minutes",
                self.pomodoro.long_break_minutes,
                defaults.long_break_minutes,
            ),
        ] {
            if minutes == 0 {
                anyhow::bail!("`[pomodoro].{key}` is 0; a phase needs at least one minute.");
            }
            if minutes > A_YEAR_IN_MINUTES {
                anyhow::bail!(
                    "`[pomodoro].{key}` is {minutes}; the maximum is {A_YEAR_IN_MINUTES} \
                     (one year). Leave it out to use the default of {default}."
                );
            }
        }
        if self.pomodoro.rounds_before_long_break == 0 {
            anyhow::bail!(
                "`[pomodoro].rounds_before_long_break` is 0; a set needs at least one focus \
                 interval before the long break."
            );
        }

        // Poll intervals are multiplied out to seconds and then to a `Duration`,
        // and each one's `.max` floor guards only the low end. An absurd value from a
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
        if self.news.refresh_minutes > A_YEAR_IN_MINUTES {
            anyhow::bail!(
                "`[news].refresh_minutes` is {}; the maximum is {A_YEAR_IN_MINUTES} \
                 (one year). Leave it out to use the default of 60.",
                self.news.refresh_minutes
            );
        }

        Ok(())
    }

    /// The keys that take one of a few words.
    ///
    /// Each is checked the way its panel reads it — the units exactly, the
    /// rest with case folded, the sort trimmed as well by its own parser — so
    /// every value that worked still loads, and one the panel would have passed
    /// over for its default is refused here instead of starting the dashboard
    /// wrong without a word.
    fn validate_words(&self) -> Result<()> {
        if !WeatherConfig::UNITS.contains(&self.weather.units.as_str()) {
            return Err(not_one_of(
                "[weather].units",
                &self.weather.units,
                &WeatherConfig::UNITS,
            ));
        }
        if !TemperatureConfig::UNITS.contains(&self.temperature.units.as_str()) {
            return Err(not_one_of(
                "[temperature].units",
                &self.temperature.units,
                &TemperatureConfig::UNITS,
            ));
        }
        if self.todo.sort.parse::<crate::task::SortMode>().is_err() {
            let words = crate::task::SortMode::ALL.map(crate::task::SortMode::label);
            return Err(not_one_of("[todo].sort", &self.todo.sort, &words));
        }
        for (key, value, words) in [
            (
                "[notes].preview",
                &self.notes.preview,
                NotesConfig::PREVIEWS,
            ),
            (
                "[calendar].week_starts",
                &self.calendar.week_starts,
                CalendarConfig::WEEK_STARTS,
            ),
        ] {
            if !words.iter().any(|word| word.eq_ignore_ascii_case(value)) {
                return Err(not_one_of(key, value, &words));
            }
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
            None => Self::default_notes_path(),
        }
    }

    /// Where notes live when `[notes].file` is unset.
    fn default_notes_path() -> Result<PathBuf> {
        Ok(Self::default_data_dir()?.join("notes.toml"))
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
            None => Self::default_watchlist_path(),
        }
    }

    /// Where the watchlist lives when `[stocks].file` is unset.
    fn default_watchlist_path() -> Result<PathBuf> {
        Ok(Self::default_data_dir()?.join("watchlist.toml"))
    }

    /// Where the world clocks live. Like the watchlist, this is a data file
    /// rather than config, because the panel edits it; `[clocks].zones` seeds
    /// it on a first run and is not read again.
    pub fn zones_path() -> Result<PathBuf> {
        Ok(Self::default_data_dir()?.join("zones.toml"))
    }

    /// Where the update check caches its answer, beside the state file.
    pub fn update_cache_path() -> Result<PathBuf> {
        Ok(crate::update::default_path(&Self::default_data_dir()?))
    }

    /// Every file mirador writes into its own data directory.
    ///
    /// The list a factory reset works from, and deliberately not the same as
    /// "every file a panel reads". `calendar.ics` is absent because mirador
    /// only ever *reads* a calendar — it is the reader's file, sitting in
    /// mirador's directory by default, and a reset has no business moving it.
    /// The rule is ownership by authorship: if mirador wrote it, mirador may
    /// set it aside.
    ///
    /// Default locations only. A `[todo].file` pointing somewhere else is a
    /// path the reader chose, and resetting the config already stops mirador
    /// looking there — moving a file out of a directory the reader picked
    /// would be a surprise a reset cannot justify.
    ///
    /// Built from the same functions the panels find their files with, so
    /// each name is spelt once. Spelt again here, a file renamed there would
    /// leave a reset setting aside a name nothing uses any more and keeping
    /// the file it was meant to move.
    pub fn owned_data_files() -> Result<Vec<PathBuf>> {
        Ok(vec![
            Self::state_path()?,
            Self::default_data_path()?,
            Self::default_notes_path()?,
            Self::default_watchlist_path()?,
            Self::zones_path()?,
            Self::update_cache_path()?,
        ])
    }

    /// Where remembered UI preferences live. Not configurable: it is mirador's
    /// own bookkeeping rather than something you curate, and a config key
    /// pointing at it would invite exactly the confusion this file avoids.
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
            && WeatherConfig::UNITS.contains(&units.as_str())
        {
            self.weather.units.clone_from(units);
        }
        if let Some(units) = &state.temperature_units
            && TemperatureConfig::UNITS.contains(&units.as_str())
        {
            self.temperature.units.clone_from(units);
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
        if let Some(twelve) = state.clocks_twelve_hour {
            self.clocks.twelve_hour = twelve;
        }
        // Free text, so there is nothing to validate against here — the panel
        // that wrote it checked that the file could be read, and a file that
        // has since gone is the panel's "no agenda file" case rather than a
        // reason to discard the setting.
        if let Some(file) = &state.agenda_file {
            self.agenda.file = Some(std::path::PathBuf::from(file));
        }
        // Not over coordinates: they win, and the name only labels them, so a
        // place remembered before they were written would retitle the panel
        // over their weather. The panel then reports the config's name, and
        // the stale entry is retracted at the next save (invariant 17).
        if let Some(location) = &state.weather_location
            && (self.weather.latitude.is_none() || self.weather.longitude.is_none())
        {
            self.weather.location.clone_from(location);
        }
        // Durations are clamped rather than dropped: an out-of-range figure
        // means a hand-edited file, and the nearest legal value is what was
        // meant. Legal is what the panel's dial allows — up to `MAX_MINUTES`,
        // or to the config's own length where that is longer — so a
        // 240-minute phase shortened to 239 comes back as 239 rather than cut
        // to the cap. The config's lengths are kept first, so the panel built
        // from a remembered one still knows how far `+` may take it back.
        let pomodoro = &mut self.pomodoro;
        let configured = *pomodoro.as_configured.get_or_insert([
            pomodoro.focus_minutes,
            pomodoro.short_break_minutes,
            pomodoro.long_break_minutes,
        ]);
        let slots = [
            &mut pomodoro.focus_minutes,
            &mut pomodoro.short_break_minutes,
            &mut pomodoro.long_break_minutes,
        ];
        let saved = [
            state.pomodoro_focus_minutes,
            state.pomodoro_short_break_minutes,
            state.pomodoro_long_break_minutes,
        ];
        for ((slot, saved), configured) in slots.into_iter().zip(saved).zip(configured) {
            if let Some(minutes) = saved {
                *slot = minutes.clamp(1, crate::widgets::pomodoro::MAX_MINUTES.max(configured));
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

/// The refusal for a key that takes one of a few words: what was written, and
/// every word the key takes, so the message is also the fix.
fn not_one_of(key: &str, value: &str, words: &[&str]) -> anyhow::Error {
    let quoted: Vec<String> = words.iter().map(|word| format!("`{word}`")).collect();
    let expected = match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => quoted.concat(),
    };
    anyhow::anyhow!("`{key}` is `{value}`; expected {expected}.")
}

/// Turn a parse failure into an error that says how to fix it.
///
/// The common case by far is a config written by an older version: mirador
/// creates the file once and never rewrites it, so a key that has since been
/// renamed sits there looking correct. Silently ignoring such a key is worse
/// than failing on it — it makes a stale config look like stale code, and
/// sends people hunting through git for a build that was never the problem.
///
/// `raw` is the text that failed, because the migration has to be asked about
/// the line the parser stopped on rather than about a key's name.
fn stale_config_hint(error: &toml::de::Error, raw: &str, path: &Path) -> anyhow::Error {
    let message = error.to_string();

    // Asked of the migration itself, about the line the error points at: the
    // migration's rules are scoped to a table, so a key's name alone pointed a
    // `forecast_days` under `[clocks]` at a migration that then refused it.
    let change = error
        .span()
        .and_then(|span| crate::migrate::change_at(raw, span.start));
    // And asked whether it would finish, by the check the migration itself
    // makes: a rewritten line in a file that still does not load is written
    // nowhere, so it is not a reason to promise an update in place. What the
    // line becomes is still the one thing the parser cannot say, so the
    // reader keeps it, with the reason the migration would stop.
    match change.map(|change| (change, crate::migrate::migrate(raw))) {
        Some((change, Ok(_))) => {
            return anyhow::anyhow!(
                "{message}\n\nThe config at {} was written by an older version of \
                 mirador. Run `mirador --migrate-config` to update it in place, \
                 keeping your original as a .bak file. The line above is one it fixes:\n\n    \
                 {change}",
                path.display(),
            );
        }
        Some((change, Err(crate::migrate::Refusal::StillBroken { error, line }))) => {
            let at = line.map_or_else(String::new, |line| format!(" at line {line}"));
            return anyhow::anyhow!(
                "{message}\n\nThe line above in {} was written by an older version of \
                 mirador, and `mirador --migrate-config` rewrites it:\n\n    {change}\n\n\
                 but it cannot finish while the config has other problems, and would \
                 leave the file untouched. The first is{at}: {}",
                path.display(),
                error.message(),
            );
        }
        // A line the migration rewrites always leaves it something to do, so
        // this is unreachable; the ordinary advice is the safe answer anyway.
        Some((_, Err(crate::migrate::Refusal::NothingKnown))) | None => {}
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
    /// A config that is a link into a dotfiles repository is reset by moving
    /// the link aside, not by writing the defaults through it: the curated file
    /// stays as it was, the backup is the link to it, and the defaults land in a
    /// file of their own.
    #[cfg(unix)]
    #[test]
    fn a_reset_leaves_the_file_a_linked_config_points_at_alone() {
        let dir = TempDir::new("reset-link");
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        std::fs::create_dir_all(dir.join("cfg")).unwrap();
        std::fs::write(dir.join("dotfiles/mirador.toml"), "# my curated config\n").unwrap();
        let path = dir.join("cfg/config.toml");
        std::os::unix::fs::symlink("../dotfiles/mirador.toml", &path).unwrap();

        let backup = Config::reset(&path).expect("resets").expect("a backup");

        let curated = std::fs::read_to_string(dir.join("dotfiles/mirador.toml")).unwrap();
        assert_eq!(
            curated, "# my curated config\n",
            "the linked file was overwritten"
        );
        assert!(
            std::fs::symlink_metadata(&backup)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            !std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
    }

    /// The same, for a link whose file has gone: the repository is there but
    /// the curated file was moved or deleted. The link is set aside all the
    /// same, so the defaults land in a file of their own rather than being
    /// written through it into the repository.
    #[cfg(unix)]
    #[test]
    fn a_reset_sets_aside_a_linked_config_whose_file_is_missing() {
        let dir = TempDir::new("reset-dangling");
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        std::fs::create_dir_all(dir.join("cfg")).unwrap();
        let path = dir.join("cfg/config.toml");
        std::os::unix::fs::symlink("../dotfiles/mirador.toml", &path).unwrap();

        let backup = Config::reset(&path).expect("resets");

        assert!(
            !dir.join("dotfiles/mirador.toml").exists(),
            "the defaults were written through the link"
        );
        let backup = backup.expect("the link was set aside");
        assert_eq!(
            std::fs::read_link(&backup).unwrap(),
            Path::new("../dotfiles/mirador.toml")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
    }

    use super::*;
    use crate::store::testing::TempDir;

    /// The list a factory reset works from. What is *absent* is the load-bearing
    /// part: mirador reads a calendar and never writes one, so `calendar.ics`
    /// belongs to the reader even when it sits in mirador's own directory.
    /// Setting it aside would be destroying data mirador did not create.
    #[test]
    fn the_owned_files_are_the_ones_mirador_writes() {
        let Ok(files) = Config::owned_data_files() else {
            // No data directory on this platform; nothing to assert about.
            return;
        };
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into())
            .collect();

        for wanted in [
            "state.toml",
            "todos.toml",
            "notes.toml",
            "watchlist.toml",
            "zones.toml",
            "update-check.toml",
        ] {
            assert!(names.iter().any(|n| n == wanted), "{wanted} must be reset");
        }
        // And each is the very file the code that writes it uses. The names
        // above are the spec of what the directory holds; a file renamed where
        // a panel finds it, and not here, passes them and leaves a reset moving
        // a name nothing writes any more while the real file stays put.
        let config = Config::default();
        for (what, used) in [
            ("tasks", config.todo_path()),
            ("notes", config.notes_path()),
            ("watchlist", config.stocks_path()),
            ("zones", Config::zones_path()),
            ("state", Config::state_path()),
            ("update check", Config::update_cache_path()),
        ] {
            let used = used.expect("a default path");
            assert!(
                files.contains(&used),
                "the {what} file is written to {} and a reset would not move it: {files:?}",
                used.display()
            );
        }
        assert!(
            !names.iter().any(|n| n == "calendar.ics"),
            "a factory reset must not touch a calendar mirador only ever reads: {names:?}"
        );
        // All in one directory, so nothing here can reach outside it.
        let dir = files[0].parent().expect("a parent");
        assert!(
            files.iter().all(|p| p.parent() == Some(dir)),
            "every owned file must live in mirador's own data directory: {files:?}"
        );
    }

    /// A scratch directory named for the calling test, so the reset tests do
    /// not share files with each other or with a parallel run.
    fn scratch(name: &str) -> TempDir {
        TempDir::new(&format!("reset-{name}"))
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
    }

    /// Asking for a config where there is none is a reason to write one, not to
    /// fail: the reader wants something to edit either way.
    #[test]
    fn resetting_with_no_config_yet_writes_one_and_reports_no_backup() {
        let dir = scratch("absent");
        let path = dir.join("config.toml");

        assert!(Config::reset(&path).unwrap().is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
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
    }

    #[test]
    fn shipped_default_config_parses() {
        let config: Config =
            toml::from_str(DEFAULT_CONFIG).expect("the bundled default config must always parse");
        config
            .validate()
            .expect("the bundled default config must always validate");
    }

    /// The shipped config's list of widgets names every one there is.
    ///
    /// It is a comment written into every first-run config, so nothing parses
    /// it, and it stopped at thirteen while four more shipped — three of them
    /// the excused ones, which a reader can only find by name, since the
    /// default layout does not place them.
    #[test]
    fn the_shipped_config_lists_every_widget() {
        let list: String = DEFAULT_CONFIG
            .lines()
            .skip_while(|line| !line.contains("Available widgets"))
            .take_while(|line| line.starts_with('#'))
            .collect::<Vec<_>>()
            .join(" ");
        let named: Vec<&str> = list.split(|c: char| !c.is_ascii_alphanumeric()).collect();
        let missing: Vec<&&str> = crate::widgets::WIDGET_NAMES
            .iter()
            .filter(|widget| !named.contains(widget))
            .collect();
        assert!(
            missing.is_empty(),
            "the config's \"Available widgets\" comment leaves out {missing:?}"
        );
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

        assert_eq!(
            shipped.layout,
            Layout::default(),
            "the shipped config and the Rust default describe different \
             dashboards. Both are first impressions — the file on a true first \
             run, the Rust default for any config that omits [layout] — so a \
             gap here means deleting one section silently removes panels."
        );
    }

    #[test]
    fn the_default_layout_places_every_widget() {
        // A widget nobody can see is a widget nobody knows exists. Nothing
        // advertises an unplaced widget, so the default has to place every one
        // it can: shipping a dashboard that hides a third of itself is a poor
        // first run, and this is exactly how notes and stocks went unseen.
        //
        // Widgets deliberately left out of the default, each with its reason.
        // The default is a dashboard for any machine; a panel that is empty on
        // most of them would be a poor first run for everyone to save a
        // discovery step for some. `w` places them, and the README names them.
        const UNPLACED_BY_DEFAULT: &[(&str, &str)] = &[
            (
                "battery",
                "laptops only — a desktop would open on `No battery`",
            ),
            (
                "temperature",
                "reads what the platform reports, which on Windows is at most one \
                 ACPI zone many machines lack, on NetBSD nothing yet, and in most \
                 VMs and containers nothing",
            ),
            (
                "disk",
                "the instrument row at 120 columns has no room for a sixth panel \
                 without costing `stocks` the change column, and a fifth row is \
                 a gesture the reader makes, not one shipped for everyone",
            ),
        ];
        let layout = Layout::default();
        let placed: Vec<&str> = layout
            .rows
            .iter()
            .flat_map(|r| r.panels.iter().map(|p| p.widget.as_str()))
            .collect();

        for (widget, _) in UNPLACED_BY_DEFAULT {
            assert!(
                crate::widgets::WIDGET_NAMES.contains(widget),
                "`{widget}` is excused from the default layout but is not a widget"
            );
            assert!(
                !placed.contains(widget),
                "`{widget}` is placed after all; drop it from the excused list"
            );
        }
        for widget in crate::widgets::WIDGET_NAMES {
            if UNPLACED_BY_DEFAULT.iter().any(|(name, _)| name == widget) {
                continue;
            }
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
    fn an_explicit_plugin_can_supply_a_layout_name_and_opaque_settings() {
        let source = r#"
plugins = [{
    id = "example",
    command = ["mirador-example", "--compact"],
    config = { label = "local" }
}]

[layout]
rows = [{ height = 1, panels = [{ widget = "example" }] }]
"#;
        let config: Config = toml::from_str(source).expect("plugin declaration parses");
        config
            .validate()
            .expect("declared plugin is a known widget");
        assert_eq!(
            config.widget_names().last().map(String::as_str),
            Some("example")
        );
        assert_eq!(config.plugins[0].config["label"].as_str(), Some("local"));
    }

    #[test]
    fn plugin_ids_cannot_shadow_core_widgets_or_each_other() {
        let collision: Config =
            toml::from_str("plugins = [{ id = \"notes\", command = [\"anything\"] }]").unwrap();
        assert!(
            collision
                .validate()
                .unwrap_err()
                .to_string()
                .contains("conflicts with a built-in")
        );

        let duplicate: Config = toml::from_str(
            "plugins = [\n  { id = \"sample\", command = [\"one\"] },\n  { id = \"sample\", command = [\"two\"] }\n]",
        )
        .unwrap();
        assert!(
            duplicate
                .validate()
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );
    }

    #[test]
    fn plugin_commands_are_argv_not_platform_shell_strings() {
        let config: Config = toml::from_str(
            "plugins = [{ id = \"sample\", command = [\"python\", \"-m\", \"sample\"] }]",
        )
        .unwrap();
        assert_eq!(config.plugins[0].command, ["python", "-m", "sample"]);

        let missing: Config =
            toml::from_str("plugins = [{ id = \"sample\", command = [] }]").unwrap();
        assert!(
            missing
                .validate()
                .unwrap_err()
                .to_string()
                .contains("no executable")
        );
    }

    /// What `Config::load` says about `source`, which has to be refused.
    fn refusal(source: &str) -> String {
        let error = toml::from_str::<Config>(source).expect_err("the source must be refused");
        format!(
            "{:#}",
            stale_config_hint(&error, source, Path::new("/tmp/config.toml"))
        )
    }

    #[test]
    fn a_key_from_an_older_version_is_rejected_with_a_migration_hint() {
        // The exact failure that made a current build look like an old one.
        let message = refusal("[weather]\nforecast_days = 4");
        assert!(message.contains("forecast_days"), "got: {message}");
        assert!(message.contains("forecast_hours"), "got: {message}");
    }

    #[test]
    fn an_unrecognised_key_names_itself_rather_than_being_ignored() {
        assert!(refusal("[weather]\nwibble = 4").contains("wibble"));
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
            let message = refusal(source);
            assert!(
                message.contains("acent") || message.contains("strat"),
                "{source} was accepted, or the error did not name the key: {message}"
            );
        }
    }

    /// The same hint behind a header with a comment after it, which is legal
    /// TOML and which the migration's walk did not read as a header — so the
    /// reader got the unknown-key error and no word of the fix.
    #[test]
    fn a_commented_header_does_not_hide_the_migration_hint() {
        let message = refusal("[theme] # my colours\nrx = \"green\"");
        assert!(
            message.contains("--migrate-config"),
            "a comment after `[theme]` hid the migration hint: {message}"
        );
    }

    #[test]
    fn the_pre_0_1_0_theme_keys_reach_their_migration_hint() {
        // These two entries sat in `RENAMED` unreachable: `[theme] rx = ...`
        // parsed clean, so the hint telling the user to run
        // `--migrate-config` could not fire for the very keys it names.
        for key in ["rx", "tx"] {
            let message = refusal(&format!("[theme]\n{key} = \"green\""));
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

    /// Every key `--migrate-config` can fix has to be pointed at it. The hint
    /// kept its own list of those keys through 1.20.0, and the list had three
    /// of the four: `[notes] side_by_side_min_width` was refused with advice to
    /// read `--print-config`, by a program that could have fixed it.
    #[test]
    fn every_key_the_migration_fixes_reaches_the_migration_hint() {
        let mut swept = 0;
        for (section, key) in crate::migrate::stale_keys() {
            let message = refusal(&format!("[{section}]\n{key} = \"x\""));
            assert!(
                message.contains("--migrate-config"),
                "`[{section}] {key}` did not reach the migration hint: {message}"
            );
            swept += 1;
        }
        assert_eq!(
            swept, 4,
            "the sweep must see every rule, or it checks nothing"
        );
    }

    /// What `--migrate-config` really does to a config, found by running it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Migration {
        /// Rewrites the file, and what it wrote loads.
        Fixes,
        /// Rewrites a line, finds the result still refused, and writes nothing.
        RewritesButRefuses,
        /// Knows nothing in the file, and writes nothing.
        LeavesAlone,
    }

    /// Run `migrate_file` on a copy of `source` and say which of the three it
    /// did, so a case in a table below is labelled by the outcome and not by
    /// anybody's reading of the rules.
    fn migration_of(dir: &Path, n: usize, source: &str) -> Migration {
        let path = dir.join(format!("config-{n}.toml"));
        std::fs::write(&path, source).unwrap();
        match crate::migrate::migrate_file(&path) {
            Ok(report) if !report.is_empty() => Migration::Fixes,
            Ok(_) => panic!("{source:?} parses, so it is no case for the hint"),
            Err(e) if e.to_string().contains("did not produce a usable config") => {
                Migration::RewritesButRefuses
            }
            Err(e) if e.to_string().contains("none of the problems") => Migration::LeavesAlone,
            Err(e) => panic!("{source:?} failed some other way: {e:#}"),
        }
    }

    /// The hint is a promise about what `--migrate-config` will do, so it has
    /// to be made exactly where the migration keeps it. The rules are scoped to
    /// a table and the hint once went by the key's name alone: `forecast_days`
    /// under `[clocks]` was sent to a migration that then refused to touch it,
    /// with a line about `[weather]` that was nowhere in the reader's file.
    ///
    /// And a line the migration rewrites is not a migration that finishes:
    /// `migrate_file` refuses whenever what it would write still does not
    /// load. A reader who added `forecast_hours` beside a stale
    /// `forecast_days`, because the parser's own message lists it, was told to
    /// "update it in place" by a migration that then refused on the duplicate.
    #[test]
    fn the_migration_hint_is_given_exactly_where_the_migration_acts() {
        use Migration::{Fixes, LeavesAlone, RewritesButRefuses};
        let cases = [
            // Where the rules look: the hint, and a change to back it.
            ("[weather]\nforecast_days = 4", Fixes),
            ("[notes]\nside_by_side_min_width = 80", Fixes),
            ("[theme]\nrx = \"green\"", Fixes),
            (
                "[weather]\r\nlocation = \"x\"\r\n  forecast_days   = 4\r\n",
                Fixes,
            ),
            (
                "[clocks]\nzones = []\n\n[weather]\nforecast_days = 4",
                Fixes,
            ),
            // The line is rewritten, and the file still does not load: the
            // replacement is already there, or something else is wrong too.
            (
                "[weather]\nlocation = \"Oslo\"\nforecast_days = 4\nforecast_hours = 6\n",
                RewritesButRefuses,
            ),
            (
                "[weather]\nforecast_days = 4\nwibble = 1\n",
                RewritesButRefuses,
            ),
            // Somewhere else: neither.
            ("[clocks]\nforecast_days = 4", LeavesAlone),
            ("[todo]\nside_by_side_min_width = 80", LeavesAlone),
            (
                "[weather]\nlocation = \"x\"\n\n[clocks]\nforecast_days = 4",
                LeavesAlone,
            ),
            ("[theme.rx_gradient]\nrx = \"green\"", LeavesAlone),
            // A spelling of the right table the line-by-line migration cannot
            // rewrite, so it must not be promised either.
            ("weather = { forecast_days = 4 }", LeavesAlone),
            ("weather.forecast_days = 4", LeavesAlone),
        ];
        let dir = TempDir::new("migration-hint");
        for (n, (source, label)) in cases.into_iter().enumerate() {
            let outcome = migration_of(&dir, n, source);
            assert_eq!(
                outcome, label,
                "the case is mislabelled: {source:?} is one --migrate-config {outcome:?}"
            );
            let message = refusal(source);
            let says = |text: &str| message.contains(text);
            match label {
                Fixes => assert!(
                    says("update it in place") && says(" will "),
                    "{source:?} was not sent to the migration that fixes it: {message}"
                ),
                RewritesButRefuses => assert!(
                    says(" will ") && says("cannot finish") && !says("update it in place"),
                    "{source:?} was promised a migration that refuses it: {message}"
                ),
                LeavesAlone => assert!(
                    says("--print-config") && !says("--migrate-config"),
                    "{source:?} was told the wrong thing: {message}"
                ),
            }
        }
    }

    /// "Other problems" alone would send the reader hunting, so the hint names
    /// the first one, by the line in *their* file: the rewritten text the
    /// parser stopped in is one nobody has seen, and only the migration's
    /// keeping every line where it was makes its line number theirs.
    #[test]
    fn a_migration_that_cannot_finish_says_where_it_would_stop() {
        for (source, at, reason) in [
            (
                "[weather]\nlocation = \"Oslo\"\nforecast_days = 4\nforecast_hours = 6\n",
                "at line 4: ",
                "duplicate key",
            ),
            (
                "[weather]\nforecast_days = 4\n\n\nwibble = 1\n",
                "at line 5: ",
                "unknown field `wibble`",
            ),
        ] {
            let message = refusal(source);
            let tail = message
                .split_once("cannot finish")
                .map_or("", |(_, tail)| tail);
            assert!(
                tail.contains(at) && tail.contains(reason),
                "{source:?} did not say where the migration stops: {message}"
            );
        }
    }

    #[test]
    fn the_notes_width_threshold_is_pointed_at_its_replacement() {
        let message = refusal("[notes]\nside_by_side_min_width = 80");
        assert!(message.contains("--migrate-config"), "got: {message}");
        // Not merely `preview`: the parser's own "expected one of" names that
        // already, so an assertion on the bare word could not fail.
        assert!(
            message.contains("side_by_side_min_width will become preview"),
            "the hint must name the replacement: {message}"
        );
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

    /// The news feed's interval is the third poll multiplied out unchecked,
    /// and it was missed when the other two were bounded. 2^62 minutes is a
    /// legal TOML integer that passed validation, and times sixty it wraps
    /// to exactly zero: a debug build panicked building the panel, and a
    /// release build fetched every feed with no wait at all — the `.max(60)`
    /// floor runs before the multiply, so it guards nothing here.
    #[test]
    fn a_news_refresh_too_long_to_multiply_out_is_refused() {
        let parse = |minutes: u64| -> Config {
            toml::from_str(&format!("[news]\nrefresh_minutes = {minutes}")).expect("parses")
        };
        let err = parse(1 << 62).validate().expect_err("must be refused");
        assert!(
            format!("{err:#}").contains("`[news].refresh_minutes`"),
            "the error must name the key: {err:#}"
        );
        assert!(parse(A_YEAR_IN_MINUTES + 1).validate().is_err());
        assert!(
            parse(A_YEAR_IN_MINUTES).validate().is_ok(),
            "a year is the limit, not past it"
        );
    }

    /// The three `[pomodoro]` lengths were bounded only from below, and the
    /// panel multiplies each out to seconds unchecked: a hand-edited
    /// `u64::MAX` panicked in a debug build and wrapped in release to a phase
    /// of a length nobody wrote. Bounded where the poll intervals are, at a
    /// year, and refused by name.
    #[test]
    fn an_absurd_pomodoro_length_is_rejected_rather_than_wrapping() {
        let parse = |key: &str, minutes: u64| -> Config {
            toml::from_str(&format!("[pomodoro]\n{key} = {minutes}")).expect("parses")
        };
        for key in ["focus_minutes", "short_break_minutes", "long_break_minutes"] {
            let err = parse(key, u64::MAX)
                .validate()
                .expect_err("must be rejected");
            assert!(
                format!("{err:#}").contains(&format!("`[pomodoro].{key}`")),
                "the error must name the key: {err:#}"
            );
            assert!(
                parse(key, A_YEAR_IN_MINUTES + 1).validate().is_err(),
                "{key} one minute past a year"
            );
            // The edge itself is legal, and well inside the arithmetic.
            assert!(
                parse(key, A_YEAR_IN_MINUTES).validate().is_ok(),
                "{key} at a year"
            );
        }
    }

    #[test]
    fn bad_units_are_rejected() {
        let config: Config = toml::from_str("[weather]\nunits = \"kelvin\"").expect("parses");
        assert!(config.validate().is_err());
    }

    /// `[todo].sort`, `[notes].preview` and `[calendar].week_starts` each
    /// take a closed set of words, and nothing checked them: each panel read
    /// the words it knew and fell back to its default for anything else, so
    /// a misspelling started the dashboard and quietly did nothing. The two
    /// `units` keys beside them had been refused by name all along.
    #[test]
    fn a_word_a_setting_does_not_take_is_refused_by_name() {
        for (text, key, accepted) in [
            (
                "[todo]\nsort = \"dues\"",
                "[todo].sort",
                "`smart`, `due`, `priority`, `created` or `title`",
            ),
            (
                "[notes]\npreview = \"besides\"",
                "[notes].preview",
                "`below` or `beside`",
            ),
            // A trailing space: the calendar compares the whole word, so this
            // started every week on Sunday.
            (
                "[calendar]\nweek_starts = \"Monday \"",
                "[calendar].week_starts",
                "`sunday` or `monday`",
            ),
        ] {
            let config: Config = toml::from_str(text).expect("parses");
            let Err(err) = config.validate() else {
                panic!("{text:?} is a word the setting does not take and must be refused");
            };
            let err = err.to_string();
            assert!(
                err.contains(&format!("`{key}`")) && err.contains(accepted),
                "the refusal must name the key and say what to write instead: {err}"
            );
        }

        // What the panels already read keeps loading. Case is folded for all
        // three, as the panels fold it, and the sort is trimmed, as its parser
        // always has been — so no config that worked is refused.
        for text in [
            "[todo]\nsort = \" Due \"",
            "[notes]\npreview = \"Beside\"",
            "[notes]\npreview = \"below\"",
            "[calendar]\nweek_starts = \"MONDAY\"",
        ] {
            let config: Config = toml::from_str(text).expect("parses");
            config
                .validate()
                .unwrap_or_else(|e| panic!("{text:?} worked before and must still load: {e}"));
        }
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

    /// A fresh directory for one of the tests below to stand in for the
    /// system's temporary directory.
    #[cfg(unix)]
    fn stand_in_temp(name: &str) -> TempDir {
        TempDir::new(name)
    }

    /// On Linux the temporary directory is usually one `/tmp` for everyone,
    /// so a test run must not need anything in it that another user's run
    /// could have made first. A shared parent did: whoever ran the suite
    /// first owned it with their umask, and a second user's run could not
    /// make its own directory inside, so nine dashboard tests failed on an
    /// alert about the world clocks.
    #[cfg(unix)]
    #[test]
    fn a_test_run_needs_nothing_another_users_run_made() {
        use std::os::unix::fs::PermissionsExt;
        let temp = stand_in_temp("shared-temp");
        // What a second user finds after the first user's run.
        let theirs = temp.join("mirador-test-data");
        std::fs::create_dir(&theirs).unwrap();
        std::fs::set_permissions(&theirs, std::fs::Permissions::from_mode(0o555)).unwrap();

        let ours = Config::test_run_dir(&temp, 4242);
        let made = ours.is_dir() && std::fs::write(ours.join("probe"), "").is_ok();

        std::fs::set_permissions(&theirs, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(made, "the run directory {} was not made", ours.display());
        // Root can write into the other user's directory, so on a machine
        // where the suite runs as root only this catches a shared parent.
        assert_eq!(ours.parent(), Some(&*temp), "{}", ours.display());
    }

    /// Runs that are over are swept by name, and the name is what keeps the
    /// sweep to them: in a temporary directory it shares with every other
    /// test and program, it takes an hour-old `mirador-test-data-*` and
    /// leaves a newer one, which may be a run still going, and anything
    /// else, however old.
    #[cfg(unix)]
    #[test]
    fn the_sweep_takes_only_test_runs_an_hour_old() {
        let temp = stand_in_temp("sweep");
        let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_hours(2);
        let aged = |name: &str| {
            let dir = temp.join(name);
            std::fs::create_dir(&dir).unwrap();
            std::fs::File::open(&dir)
                .unwrap()
                .set_modified(two_hours_ago)
                .unwrap();
            dir
        };
        let over = aged("mirador-test-data-1");
        let not_ours = aged("mirador-layout-1");
        let going = temp.join("mirador-test-data-2");
        std::fs::create_dir(&going).unwrap();

        Config::test_run_dir(&temp, 3);
        let left = (over.exists(), not_ours.exists(), going.exists());

        assert_eq!(left, (false, true, true), "(over, not ours, still going)");
    }
}
