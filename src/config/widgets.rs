//! One settings struct per widget.
//!
//! These live together rather than beside their panels because they are the
//! *schema of the config file*: `serde` reads them all before any panel exists,
//! and a reader working out what a key does should find every key in one place.
//! They carry no behaviour beyond `Default`.
//!
//! Every one of them sets `deny_unknown_fields`. A silently ignored key is the
//! worst outcome a config can have — it makes a stale config look like stale
//! code — and the two theme structs that lacked it went unnoticed for four
//! releases. See `theme.rs`.

use std::path::PathBuf;

use serde::Deserialize;

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
/// and those changes are remembered in the state file beside your tasks.
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
    ///
    /// Two, not one. Since the redraw follows visible change rather than the
    /// tick, a fresh sample is a new number and every new number costs a
    /// repaint of the whole grid — so this panel and `network` set the floor on
    /// what an idle dashboard does.
    ///
    /// Measured at 400x100 on the default layout, redraws per idle minute:
    ///
    /// ```text
    ///                        sample_secs = 1    = 2
    ///   show_seconds = true          95          81
    ///   show_seconds = false         66          36
    /// ```
    ///
    /// The second row is the change; the first is what the clock costs. With
    /// seconds on, the clock alone asks for a repaint every second and swamps
    /// this, which is worth knowing before attributing an idle dashboard's cost
    /// to the graphs.
    ///
    /// It also buys a chart covering twice the wall-clock time for the same
    /// buffer — the span beside the figure is computed from the live sample
    /// count, so it says so.
    ///
    /// The cost is that the figure is a two-second average, so a brief spike
    /// reads slightly lower. Set it to 1 if you would rather watch it closely
    /// than leave it open all day; the floor is 1 either way.
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
            sample_secs: 2,
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
    /// Seconds between samples. See [`CpuConfig::sample_secs`] for why it is 2.
    ///
    /// This one is also the more expensive of the pair: enumerating interfaces
    /// costs the better part of a millisecond a sample on macOS, against 60
    /// microseconds for reading CPU usage.
    pub sample_secs: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            interfaces: Vec::new(),
            history: 120,
            sample_secs: 2,
        }
    }
}
