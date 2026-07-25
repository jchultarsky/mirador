//! Configuration loading.
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

/// The default config written on first run.
pub const DEFAULT_CONFIG: &str = include_str!("../assets/default_config.toml");

/// Top-level configuration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    pub theme: Theme,
    pub layout: Layout,
    pub clocks: ClocksConfig,
    pub weather: WeatherConfig,
    pub todo: TodoConfig,
    pub notes: NotesConfig,
    pub stocks: StocksConfig,
    pub calendar: CalendarConfig,
    pub pomodoro: PomodoroConfig,
    pub cpu: CpuConfig,
    pub network: NetworkConfig,
}

/// Global behaviour.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
}

impl Default for General {
    fn default() -> Self {
        Self {
            tick_rate_ms: 250,
            show_borders: true,
            show_status_bar: true,
            mouse: true,
        }
    }
}

/// A grid of rows, each holding one or more side-by-side panels.
///
/// `height` and `width` are relative weights, not absolute cells, so a layout
/// keeps its proportions at any terminal size.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layout {
    pub rows: Vec<LayoutRow>,
}

impl Default for Layout {
    fn default() -> Self {
        // Kept identical to the `[layout]` block in assets/default_config.toml,
        // and `the_rust_default_layout_matches_the_shipped_one` fails if they
        // drift. They are reached by different routes — the shipped file on a
        // true first run, this on any config that omits `[layout]` — and when
        // they disagreed, deleting the section silently cost three panels.
        let row = |height: u16, panels: &[(&str, u16)]| LayoutRow {
            height,
            panels: panels
                .iter()
                .map(|(widget, width)| LayoutPanel {
                    widget: (*widget).into(),
                    width: *width,
                })
                .collect(),
        };

        Self {
            rows: vec![
                row(34, &[("clocks", 26), ("calendar", 34), ("weather", 40)]),
                row(42, &[("todo", 44), ("notes", 30), ("pomodoro", 26)]),
                row(24, &[("stocks", 40), ("cpu", 30), ("network", 30)]),
            ],
        }
    }
}

/// One horizontal band of the dashboard.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutRow {
    /// Relative height weight.
    pub height: u16,
    pub panels: Vec<LayoutPanel>,
}

impl Default for LayoutRow {
    fn default() -> Self {
        Self {
            height: 1,
            panels: Vec::new(),
        }
    }
}

/// One panel within a row.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutPanel {
    /// Widget id: `clocks`, `weather`, `todo`, `cpu` or `network`.
    pub widget: String,
    /// Relative width weight.
    pub width: u16,
}

impl Default for LayoutPanel {
    fn default() -> Self {
        Self {
            widget: String::new(),
            width: 1,
        }
    }
}

/// World clock settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClocksConfig {
    /// Clocks to display, in order.
    pub zones: Vec<ClockZone>,
    /// `strftime`-style time format.
    pub time_format: String,
    /// `strftime`-style date format. Empty hides the date.
    pub date_format: String,
    /// Show each zone's offset relative to the primary clock.
    pub show_offset: bool,
    /// Include seconds in the large clock. Off by default: a ticking seconds
    /// field draws the eye every second, which is the opposite of what a
    /// leave-it-running dashboard wants.
    pub show_seconds: bool,
}

impl Default for ClocksConfig {
    fn default() -> Self {
        Self {
            zones: vec![
                ClockZone {
                    label: "Local".into(),
                    timezone: "local".into(),
                },
                ClockZone {
                    label: "UTC".into(),
                    timezone: "UTC".into(),
                },
                ClockZone {
                    label: "London".into(),
                    timezone: "Europe/London".into(),
                },
                ClockZone {
                    label: "Tokyo".into(),
                    timezone: "Asia/Tokyo".into(),
                },
            ],
            time_format: "%H:%M:%S".into(),
            date_format: "%A %d %B".into(),
            show_offset: true,
            show_seconds: true,
        }
    }
}

/// A single clock.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ClockZone {
    /// Display name.
    pub label: String,
    /// IANA timezone id, or the literal `local` for the system zone.
    pub timezone: String,
}

/// Weather settings. Data comes from Open-Meteo, which needs no API key.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WeatherConfig {
    /// Place name, geocoded on startup when `latitude`/`longitude` are unset.
    pub location: String,
    /// Explicit latitude; skips geocoding.
    pub latitude: Option<f64>,
    /// Explicit longitude; skips geocoding.
    pub longitude: Option<f64>,
    /// `metric` (C, km/h) or `imperial` (F, mph).
    pub units: String,
    /// Number of forecast hours to show, 1 to 24.
    pub forecast_hours: u8,
    /// Minutes between refreshes.
    pub refresh_minutes: u64,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            location: "Boston, Massachusetts".into(),
            latitude: None,
            longitude: None,
            units: "imperial".into(),
            forecast_hours: 8,
            refresh_minutes: 30,
        }
    }
}

/// To-do list settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TodoConfig {
    /// Path to the task file. `~` is expanded. Defaults to the data directory.
    pub file: Option<PathBuf>,
    /// Show completed tasks in the list.
    pub show_completed: bool,
    /// Initial sort: `smart`, `due`, `priority`, `created` or `title`.
    pub sort: String,
    /// Date format used in the list.
    pub date_format: String,
    /// Hide tasks whose due date is more than this many days out. 0 disables.
    pub horizon_days: u32,
}

impl Default for TodoConfig {
    fn default() -> Self {
        Self {
            file: None,
            show_completed: false,
            sort: "smart".into(),
            date_format: "%a %d %b".into(),
            horizon_days: 0,
        }
    }
}

/// Stock watchlist settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StocksConfig {
    /// Symbols used to seed the watchlist on first run only. After that the
    /// watchlist file is the truth, so symbols added or removed in the UI stick.
    pub symbols: Vec<String>,
    /// Path to the watchlist file. `~` is expanded. Defaults to the data
    /// directory. Only symbols are stored; prices are never written to disk.
    pub file: Option<PathBuf>,
    /// Where quotes come from. See `[stocks].source` in the default config.
    pub source: String,
    /// Seconds between polls. Clamped to a minimum of 60: the sources are free
    /// and unauthenticated, and hammering them gets the address blocked.
    pub refresh_secs: u64,
    /// Milliseconds between individual symbol requests, so a watchlist goes out
    /// as a trickle rather than a burst.
    pub stagger_ms: u64,
    /// Draw the intraday sparkline when the panel is wide enough for it.
    pub show_sparkline: bool,
}

impl Default for StocksConfig {
    fn default() -> Self {
        Self {
            symbols: vec!["AAPL".into(), "MSFT".into(), "^GSPC".into()],
            file: None,
            source: "yahoo".to_string(),
            refresh_secs: 120,
            stagger_ms: 400,
            show_sparkline: true,
        }
    }
}

/// Notes settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotesConfig {
    /// Path to the notes file. `~` is expanded. Defaults to the data directory.
    pub file: Option<PathBuf>,
    /// Date format used in the list.
    pub date_format: String,
    /// Where the note body sits: `below` the list, or `beside` it.
    ///
    /// Below by default. Beside splits a finite width between two things that
    /// both want it — the list loses room for titles and the body loses room
    /// for prose — where stacking gives each the full width and trades only
    /// height, which is the cheaper axis for both.
    pub preview: String,
}

impl Default for NotesConfig {
    fn default() -> Self {
        Self {
            file: None,
            date_format: "%d %b".to_string(),
            preview: "below".to_string(),
        }
    }
}

/// Calendar settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CalendarConfig {
    /// How many months to show, starting with the current one. The panel draws
    /// as many as its size allows, up to this number.
    pub months: u8,
    /// `sunday` or `monday`.
    pub week_starts: String,
}

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            months: 2,
            week_starts: "sunday".to_string(),
        }
    }
}

/// Pomodoro timer settings.
///
/// These are the *starting* values. `+` and `-` change the timer in the panel,
/// and those changes last for the session — mirador never rewrites this file.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PomodoroConfig {
    /// Length of a focus interval, in minutes.
    pub focus_minutes: u64,
    /// Length of the break after an ordinary focus interval.
    pub short_break_minutes: u64,
    /// Length of the break that closes a set.
    pub long_break_minutes: u64,
    /// Focus intervals per set, after which the long break falls due.
    pub rounds_before_long_break: u32,
    /// Begin the next phase the moment the current one ends, rather than
    /// waiting for a keypress.
    pub auto_start: bool,
    /// Sound a notification when a phase ends. Off by default: a dashboard you
    /// leave open all day has no business making noise you did not ask for.
    pub chime: bool,
    /// What to run for that notification, as a program and its arguments.
    ///
    /// Empty means the terminal bell, which costs nothing and lets your
    /// terminal and OS decide whether that is a sound, a flash, or nothing at
    /// all. Set it to play an actual file if you want a specific chime — see
    /// the commented examples in the default config.
    ///
    /// Run directly rather than through a shell, so there is no quoting to get
    /// wrong and no shell to inject into.
    pub chime_command: Vec<String>,
}

impl Default for PomodoroConfig {
    fn default() -> Self {
        Self {
            focus_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            rounds_before_long_break: 4,
            auto_start: false,
            chime: false,
            chime_command: Vec::new(),
        }
    }
}

/// CPU chart settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CpuConfig {
    /// Number of samples retained in the moving chart.
    pub history: usize,
    /// Seconds between samples.
    pub sample_secs: u64,
    /// Also draw a per-core breakdown when the panel is tall enough.
    pub show_per_core: bool,
    /// Percentage above which the readout turns the warning colour.
    pub warn_pct: f32,
    /// Percentage above which the readout turns the error colour.
    pub critical_pct: f32,
}

impl Default for CpuConfig {
    fn default() -> Self {
        Self {
            history: 120,
            sample_secs: 1,
            show_per_core: true,
            warn_pct: 70.0,
            critical_pct: 90.0,
        }
    }
}

/// Network chart settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Interfaces to include. Empty means every non-loopback interface.
    pub interfaces: Vec<String>,
    /// Number of samples retained in the moving chart.
    pub history: usize,
    /// Seconds between samples.
    pub sample_secs: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            interfaces: Vec::new(),
            history: 120,
            sample_secs: 1,
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
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating config directory {}", parent.display()))?;
            }
            std::fs::write(&path, DEFAULT_CONFIG)
                .with_context(|| format!("writing default config to {}", path.display()))?;
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let config: Self = toml::from_str(&raw).map_err(|e| stale_config_hint(&e, &path))?;
        config.validate()?;
        Ok((config, path))
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

    /// Resolve the watchlist file path, expanding a leading `~`.
    pub fn stocks_path(&self) -> Result<PathBuf> {
        match &self.stocks.file {
            Some(p) => Ok(expand_tilde(p)),
            None => Ok(Self::default_data_dir()?.join("watchlist.toml")),
        }
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

    #[test]
    fn shipped_default_config_parses() {
        let config: Config =
            toml::from_str(DEFAULT_CONFIG).expect("the bundled default config must always parse");
        config
            .validate()
            .expect("the bundled default config must always validate");
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
        assert_eq!(config.layout.rows.len(), 3);
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
