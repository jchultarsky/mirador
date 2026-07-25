//! Weather: current conditions plus an hour-by-hour forecast.
//!
//! Data comes from Open-Meteo, which needs no API key and no account — that is
//! what keeps mirador's "no registration" promise credible. All network I/O
//! happens on a background thread; the panel only ever reads a mutex-guarded
//! snapshot, so a slow or hung request can never stall the render loop.
//!
//! The forecast is hourly rather than daily because the question a dashboard
//! answers is "what is the rest of my day like", not "what is the week like".
//! Each row is labelled with the hour it applies to, which the previous daily
//! layout left the reader to infer.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use serde::Deserialize;

use crate::config::WeatherConfig;
use crate::frame::Binding;
use crate::glyphs;
use crate::grid::{Column, Grid};
use crate::panel::{Panel, RenderContext};

/// How long to wait on any single HTTP request.
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Columns of the hourly forecast.
///
/// Ordered by how often they are wanted, because the narrow ones drop first.
const COLUMNS: &[Column] = &[
    Column::fixed("hour", 5),
    // Wide enough for the mark plus the longest label, "thunderstorm".
    Column::flex("sky", 1),
    Column::fixed("temp", 5).right(),
    Column::fixed("feels", 5).right().drops_below(52),
    Column::fixed("rain", 5).right().drops_below(38),
    Column::fixed("wind", 7).right().drops_below(62),
];

const BINDINGS: &[Binding] = &[
    Binding::primary("r", "refresh"),
    Binding::primary("u", "units"),
];

/// Border and interior padding on both sides, since `max_*` describes the whole
/// panel rather than its interior.
const FRAME_AND_PADDING: u16 = 4;

/// One slot of the hourly forecast.
#[derive(Debug, Clone)]
pub struct Slot {
    /// Local wall-clock hour, 0-23.
    pub hour: u8,
    pub temperature: f64,
    pub feels_like: f64,
    pub code: u8,
    pub precipitation_chance: Option<u8>,
    pub wind: f64,
}

/// A complete weather snapshot.
#[derive(Debug, Clone)]
pub struct WeatherData {
    pub place: String,
    pub temperature: f64,
    pub feels_like: f64,
    pub code: u8,
    pub wind: f64,
    pub humidity: Option<u8>,
    pub hours: Vec<Slot>,
    pub temperature_unit: &'static str,
    pub wind_unit: &'static str,
    /// Local time the observation was taken, for the frame counter.
    pub observed: String,
}

/// What the background thread has produced so far.
#[derive(Debug, Clone, Default)]
enum Status {
    #[default]
    Loading,
    Ready(Box<WeatherData>),
    Failed(String),
}

/// The weather panel.
#[derive(Debug)]
pub struct WeatherPanel {
    status: Arc<Mutex<Status>>,
    /// Set to true to ask the fetch thread for an immediate refresh.
    refresh: Arc<Mutex<bool>>,
    /// Kept for `max_height`; the rest of the config moves to the fetch thread.
    forecast_hours: u8,
    /// Display units, toggled with `u`.
    ///
    /// Held separately from the fetched data and applied at render, so the
    /// switch is instant. Re-requesting in the other unit would put a network
    /// round trip behind a keypress, and leave the panel showing the old unit
    /// — or nothing — until it came back.
    imperial: bool,
}

/// Convert a temperature between the scales.
fn convert_temperature(value: f64, to_imperial: bool) -> f64 {
    if to_imperial {
        value * 9.0 / 5.0 + 32.0
    } else {
        (value - 32.0) * 5.0 / 9.0
    }
}

/// Convert a wind speed between mph and km/h.
fn convert_wind(value: f64, to_imperial: bool) -> f64 {
    const KM_PER_MILE: f64 = 1.609_344;
    if to_imperial {
        value / KM_PER_MILE
    } else {
        value * KM_PER_MILE
    }
}

impl WeatherPanel {
    /// Start the background fetch loop and return immediately.
    pub fn new(config: WeatherConfig) -> Self {
        let forecast_hours = config.forecast_hours;
        let imperial = config.units != "metric";
        let status = Arc::new(Mutex::new(Status::Loading));
        let refresh = Arc::new(Mutex::new(false));
        let shared = Arc::clone(&status);
        let shared_refresh = Arc::clone(&refresh);

        std::thread::Builder::new()
            .name("mirador-weather".into())
            .spawn(move || fetch_loop(&config, &shared, &shared_refresh))
            .expect("spawning the weather thread");

        Self {
            status,
            refresh,
            forecast_hours,
            imperial,
        }
    }

    /// Restate `data` in the display units, if it was not fetched in them.
    ///
    /// The source values arrive rounded to one decimal and everything is shown
    /// to zero, so a conversion cannot shift a displayed figure by more than
    /// the rounding already applied.
    fn in_display_units(&self, mut data: WeatherData) -> WeatherData {
        let fetched_imperial = data.temperature_unit == "°F";
        if fetched_imperial == self.imperial {
            return data;
        }

        let to = self.imperial;
        data.temperature = convert_temperature(data.temperature, to);
        data.feels_like = convert_temperature(data.feels_like, to);
        data.wind = convert_wind(data.wind, to);
        for hour in &mut data.hours {
            hour.temperature = convert_temperature(hour.temperature, to);
            hour.feels_like = convert_temperature(hour.feels_like, to);
            hour.wind = convert_wind(hour.wind, to);
        }
        data.temperature_unit = if to { "°F" } else { "°C" };
        data.wind_unit = if to { "mph" } else { "km/h" };
        data
    }

    fn snapshot(&self) -> Status {
        // A poisoned lock means the fetch thread panicked. Recover the value
        // rather than propagating the panic into the render loop: one dead
        // panel should not take the dashboard with it.
        match self.status.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

/// Fetch now, then every `refresh_minutes`, until the process exits.
fn fetch_loop(config: &WeatherConfig, status: &Arc<Mutex<Status>>, refresh: &Arc<Mutex<bool>>) {
    // Resolve coordinates once; a geocoding failure is fatal for this panel and
    // there is no point retrying it every cycle.
    let located = match resolve_location(config) {
        Ok(v) => v,
        Err(e) => {
            store(status, Status::Failed(format!("{e:#}")));
            return;
        }
    };

    let interval = Duration::from_secs(config.refresh_minutes.max(1) * 60);
    loop {
        match fetch_weather(config, &located) {
            Ok(data) => store(status, Status::Ready(Box::new(data))),
            Err(e) => store(status, Status::Failed(format!("{e:#}"))),
        }

        // Sleep in short slices so a manual refresh does not wait out the full
        // interval, without needing a channel or a condvar.
        let mut waited = Duration::ZERO;
        while waited < interval {
            std::thread::sleep(Duration::from_millis(500));
            waited += Duration::from_millis(500);
            let asked = match refresh.lock() {
                Ok(mut flag) => std::mem::replace(&mut *flag, false),
                Err(poisoned) => std::mem::replace(&mut *poisoned.into_inner(), false),
            };
            if asked {
                break;
            }
        }
    }
}

fn store(status: &Arc<Mutex<Status>>, value: Status) {
    match status.lock() {
        Ok(mut guard) => *guard = value,
        Err(poisoned) => *poisoned.into_inner() = value,
    }
}

/// A place with coordinates.
#[derive(Debug, Clone)]
struct Located {
    name: String,
    latitude: f64,
    longitude: f64,
}

/// Use explicit coordinates when given, otherwise geocode the location name.
fn resolve_location(config: &WeatherConfig) -> Result<Located> {
    if let (Some(latitude), Some(longitude)) = (config.latitude, config.longitude) {
        return Ok(Located {
            name: if config.location.is_empty() {
                format!("{latitude:.2}, {longitude:.2}")
            } else {
                config.location.clone()
            },
            latitude,
            longitude,
        });
    }

    if config.location.trim().is_empty() {
        anyhow::bail!(
            "no location set. Add `location = \"City, Region\"` or explicit \
             `latitude`/`longitude` under [weather]."
        );
    }

    geocode(&config.location)
}

#[derive(Debug, Deserialize)]
struct GeocodeResponse {
    #[serde(default)]
    results: Vec<GeocodeResult>,
}

#[derive(Debug, Deserialize)]
struct GeocodeResult {
    name: String,
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    admin1: Option<String>,
    #[serde(default)]
    country_code: Option<String>,
}

/// Turn a place name into coordinates.
fn geocode(query: &str) -> Result<Located> {
    // Open-Meteo's geocoder matches on the city alone, so drop any region
    // suffix the user wrote and use it to disambiguate the results instead.
    let city = query.split(',').next().unwrap_or(query).trim();
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=10&language=en&format=json",
        urlencode(city)
    );

    let body = http_get(&url).context("geocoding the configured location")?;
    let parsed: GeocodeResponse =
        serde_json::from_str(&body).context("parsing the geocoding response")?;

    if parsed.results.is_empty() {
        anyhow::bail!(
            "could not find `{query}`. Try a different spelling, or set \
             `latitude` and `longitude` under [weather]."
        );
    }

    let hint = query
        .split_once(',')
        .map(|(_, rest)| rest.trim().to_ascii_lowercase())
        .unwrap_or_default();

    let best = parsed
        .results
        .iter()
        .find(|r| {
            !hint.is_empty()
                && (r
                    .admin1
                    .as_ref()
                    .is_some_and(|a| a.to_ascii_lowercase() == hint)
                    || r.country_code
                        .as_ref()
                        .is_some_and(|c| c.to_ascii_lowercase() == hint))
        })
        .unwrap_or(&parsed.results[0]);

    let label = match &best.admin1 {
        Some(region) if !region.is_empty() => format!("{}, {region}", best.name),
        _ => best.name.clone(),
    };

    Ok(Located {
        name: label,
        latitude: best.latitude,
        longitude: best.longitude,
    })
}

#[derive(Debug, Deserialize)]
struct ForecastResponse {
    current: Current,
    hourly: Hourly,
}

#[derive(Debug, Deserialize)]
struct Current {
    time: String,
    temperature_2m: f64,
    apparent_temperature: f64,
    weather_code: u8,
    wind_speed_10m: f64,
    #[serde(default)]
    relative_humidity_2m: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct Hourly {
    time: Vec<String>,
    temperature_2m: Vec<f64>,
    apparent_temperature: Vec<f64>,
    weather_code: Vec<u8>,
    wind_speed_10m: Vec<f64>,
    #[serde(default)]
    precipitation_probability: Vec<Option<u8>>,
}

/// Fetch current conditions plus an hourly forecast.
fn fetch_weather(config: &WeatherConfig, located: &Located) -> Result<WeatherData> {
    let imperial = config.units == "imperial";
    let wanted = usize::from(config.forecast_hours.clamp(1, 24));

    let url = format!(
        "https://api.open-meteo.com/v1/forecast\
         ?latitude={lat}&longitude={lon}\
         &current=temperature_2m,apparent_temperature,weather_code,wind_speed_10m,relative_humidity_2m\
         &hourly=temperature_2m,apparent_temperature,weather_code,wind_speed_10m,precipitation_probability\
         &forecast_days=2&timezone=auto{units}",
        lat = located.latitude,
        lon = located.longitude,
        units = if imperial {
            "&temperature_unit=fahrenheit&wind_speed_unit=mph&precipitation_unit=inch"
        } else {
            ""
        },
    );

    let body = http_get(&url).context("fetching the forecast")?;
    let parsed: ForecastResponse =
        serde_json::from_str(&body).context("parsing the forecast response")?;

    let hours = upcoming_hours(&parsed, wanted);

    Ok(WeatherData {
        place: located.name.clone(),
        temperature: parsed.current.temperature_2m,
        feels_like: parsed.current.apparent_temperature,
        code: parsed.current.weather_code,
        wind: parsed.current.wind_speed_10m,
        humidity: parsed.current.relative_humidity_2m,
        hours,
        temperature_unit: if imperial { "°F" } else { "°C" },
        wind_unit: if imperial { "mph" } else { "km/h" },
        observed: hour_label(&parsed.current.time).unwrap_or_default(),
    })
}

/// Select the next `wanted` hourly entries at or after the current hour.
///
/// Open-Meteo returns the whole requested span starting at midnight local, so
/// the first half of it is usually in the past. The `current.time` field is the
/// server's own idea of "now" in the same local zone, which makes it the right
/// thing to compare against — using the machine's clock would break for anyone
/// forecasting a location in another timezone.
fn upcoming_hours(parsed: &ForecastResponse, wanted: usize) -> Vec<Slot> {
    let now = parsed.current.time.as_str();
    let start = parsed
        .hourly
        .time
        .iter()
        .position(|t| t.as_str() >= now)
        .unwrap_or(0);

    parsed
        .hourly
        .time
        .iter()
        .enumerate()
        .skip(start)
        .take(wanted)
        .filter_map(|(index, time)| {
            Some(Slot {
                hour: parse_hour(time)?,
                temperature: *parsed.hourly.temperature_2m.get(index)?,
                feels_like: parsed
                    .hourly
                    .apparent_temperature
                    .get(index)
                    .copied()
                    .unwrap_or_default(),
                code: parsed.hourly.weather_code.get(index).copied().unwrap_or(0),
                precipitation_chance: parsed
                    .hourly
                    .precipitation_probability
                    .get(index)
                    .copied()
                    .flatten(),
                wind: parsed
                    .hourly
                    .wind_speed_10m
                    .get(index)
                    .copied()
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Extract the hour from an ISO local timestamp like `2026-07-25T14:00`.
fn parse_hour(timestamp: &str) -> Option<u8> {
    let time = timestamp.split('T').nth(1)?;
    time.split(':').next()?.parse().ok()
}

/// The `HH:MM` portion of an ISO local timestamp.
fn hour_label(timestamp: &str) -> Option<String> {
    let time = timestamp.split('T').nth(1)?;
    let mut parts = time.split(':');
    let hour = parts.next()?;
    let minute = parts.next().unwrap_or("00");
    Some(format!("{hour}:{minute}"))
}

/// A blocking GET with a timeout, returning the body as a string.
fn http_get(url: &str) -> Result<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .user_agent(concat!("mirador/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();

    let mut response = agent.get(url).call().map_err(|e| match e {
        ureq::Error::StatusCode(code) => {
            anyhow::anyhow!("the weather service returned HTTP {code}")
        }
        other => anyhow::anyhow!("network request failed: {other}"),
    })?;

    response
        .body_mut()
        .read_to_string()
        .context("reading the response body")
}

/// Percent-encode the characters that matter for a query string value.
fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push_str("%20"),
            other => {
                use std::fmt::Write as _;
                // Writing into a String is infallible.
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

impl Panel for WeatherPanel {
    fn title(&self) -> String {
        match self.snapshot() {
            Status::Ready(data) => format!("Weather — {}", data.place),
            _ => "Weather".to_string(),
        }
    }

    fn counter(&self) -> Option<String> {
        match self.snapshot() {
            // The observation time matters: a dashboard left running all day
            // should never let you mistake stale data for live data.
            Status::Ready(data) if !data.observed.is_empty() => {
                Some(format!("at {}", data.observed))
            }
            Status::Loading => Some("loading".to_string()),
            _ => None,
        }
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn max_height(&self) -> Option<u16> {
        // Current conditions (the sky art is the tall part), the rule, the
        // column header, and one row per forecast hour. The forecast is a
        // fixed number of rows, so past this the panel is a table with a
        // growing blank field under it.
        let hours = u16::from(self.forecast_hours.clamp(1, 24));
        Some(
            u16::try_from(glyphs::ART_HEIGHT).unwrap_or(4)
                + 2  // the "next hours" rule and the column header
                + hours
                + FRAME_AND_PADDING,
        )
    }

    fn refresh_interval(&self) -> Duration {
        // The background thread owns the real cadence; this only controls how
        // quickly a completed fetch appears on screen.
        Duration::from_secs(2)
    }

    fn handle_key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> crate::panel::KeyOutcome {
        use ratatui::crossterm::event::KeyCode;
        match key.code {
            KeyCode::Char('r') => {
                if let Ok(mut flag) = self.refresh.lock() {
                    *flag = true;
                }
                crate::panel::KeyOutcome::Consumed
            }
            // Converted at render rather than re-requested, so the switch is
            // immediate instead of putting a network round trip behind a key.
            KeyCode::Char('u') => {
                self.imperial = !self.imperial;
                crate::panel::KeyOutcome::Consumed
            }
            _ => crate::panel::KeyOutcome::Ignored,
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.width == 0 || area.height == 0 {
            return;
        }

        let data = match self.snapshot() {
            Status::Loading => {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        "Fetching weather…",
                        Style::default().fg(theme.muted),
                    )),
                    area,
                );
                return;
            }
            Status::Failed(message) => {
                frame.render_widget(
                    Paragraph::new(vec![
                        Line::from(Span::styled(
                            "Weather unavailable",
                            Style::default()
                                .fg(theme.error)
                                .add_modifier(Modifier::BOLD),
                        )),
                        Line::from(Span::styled(message, Style::default().fg(theme.muted))),
                        Line::from(Span::styled("r to retry", Style::default().fg(theme.muted))),
                    ])
                    .wrap(Wrap { trim: true }),
                    area,
                );
                return;
            }
            Status::Ready(data) => Box::new(self.in_display_units(*data)),
        };

        // The art is the one indulgence in this panel, so it is the first thing
        // dropped when the panel gets short.
        let show_art = area.height >= 7 && area.width >= 34;
        let now_height = if show_art {
            u16::try_from(glyphs::ART_HEIGHT).unwrap_or(4)
        } else {
            2
        };

        let rows = Layout::vertical([
            Constraint::Length(now_height.min(area.height)),
            Constraint::Length(u16::from(area.height > now_height + 2)), // rule
            Constraint::Min(0),                                          // forecast
        ])
        .split(area);

        Self::render_now(frame, rows[0], theme, &data, show_art);

        if rows[1].height > 0 {
            crate::frame::rule(frame, rows[1], theme, "next hours");
        }

        if rows[2].height > 0 {
            render_forecast(frame, rows[2], theme, &data);
        }
    }
}

impl WeatherPanel {
    /// Current conditions: art on the left, readings on the right.
    fn render_now(
        frame: &mut Frame,
        area: Rect,
        theme: &crate::theme::Theme,
        data: &WeatherData,
        show_art: bool,
    ) {
        if area.height == 0 {
            return;
        }
        let sky = glyphs::sky(data.code);

        let readings_area = if show_art {
            let art_width = u16::try_from(glyphs::ART_WIDTH).unwrap_or(12);
            let split = Layout::horizontal([Constraint::Length(art_width + 2), Constraint::Min(0)])
                .split(area);
            for (index, line) in glyphs::art(sky).iter().enumerate() {
                let y = area.y + u16::try_from(index).unwrap_or(0);
                if y >= area.y + area.height {
                    break;
                }
                frame.render_widget(
                    Paragraph::new(Span::styled(*line, Style::default().fg(theme.accent))),
                    Rect::new(split[0].x, y, split[0].width, 1),
                );
            }
            split[1]
        } else {
            area
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("{:.0}{}", data.temperature, data.temperature_unit),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", glyphs::describe(sky)),
                    Style::default().fg(theme.text),
                ),
            ]),
            Line::from(Span::styled(
                format!("feels {:.0}{}", data.feels_like, data.temperature_unit),
                Style::default().fg(theme.muted),
            )),
        ];

        let mut extras = vec![format!("wind {:.0} {}", data.wind, data.wind_unit)];
        if let Some(humidity) = data.humidity {
            extras.push(format!("humidity {humidity}%"));
        }
        lines.push(Line::from(Span::styled(
            extras.join("   "),
            Style::default().fg(theme.muted),
        )));

        frame.render_widget(Paragraph::new(lines), readings_area);
    }
}

/// The hourly table.
fn render_forecast(frame: &mut Frame, area: Rect, theme: &crate::theme::Theme, data: &WeatherData) {
    if data.hours.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No hourly data.",
                Style::default().fg(theme.muted),
            )),
            area,
        );
        return;
    }

    let grid = Grid::new(COLUMNS, area.width);
    if grid.is_empty() {
        return;
    }

    let mut lines = vec![grid.header(theme)];
    let room = usize::from(area.height.saturating_sub(1));

    for hour in data.hours.iter().take(room) {
        let sky = glyphs::sky(hour.code);
        // Always render a value. A blank cell reads as "this column is
        // broken", where "0%" reads as "it is not going to rain" — which is
        // the fact the reader wanted.
        let rain = hour
            .precipitation_chance
            .map_or_else(|| "–".to_string(), |c| format!("{c}%"));

        // Rain colour tracks likelihood: a 10% chance should not shout.
        let rain_style = match hour.precipitation_chance.unwrap_or(0) {
            0..=19 => Style::default().fg(theme.muted),
            20..=59 => Style::default().fg(theme.label),
            _ => Style::default().fg(theme.warning),
        };

        lines.push(grid.row(&[
            Span::styled(
                format!("{:02}:00", hour.hour),
                Style::default().fg(theme.muted),
            ),
            Span::styled(
                format!("{} {}", glyphs::mark(sky), glyphs::describe(sky)),
                Style::default().fg(theme.text),
            ),
            Span::styled(
                format!("{:.0}{}", hour.temperature, data.temperature_unit),
                Style::default().fg(theme.text),
            ),
            Span::styled(
                format!("{:.0}°", hour.feels_like),
                Style::default().fg(theme.muted),
            ),
            Span::styled(rain, rain_style),
            Span::styled(
                format!("{:.0} {}", hour.wind, data.wind_unit),
                Style::default().fg(theme.muted),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data(imperial: bool) -> WeatherData {
        WeatherData {
            place: "Cincinnati".into(),
            temperature: if imperial { 82.0 } else { 27.777_78 },
            feels_like: if imperial { 86.0 } else { 30.0 },
            code: 0,
            wind: if imperial { 6.0 } else { 9.656_064 },
            humidity: Some(44),
            hours: vec![Slot {
                hour: 13,
                temperature: if imperial { 212.0 } else { 100.0 },
                feels_like: if imperial { 32.0 } else { 0.0 },
                code: 0,
                precipitation_chance: Some(0),
                wind: if imperial { 10.0 } else { 16.093_44 },
            }],
            temperature_unit: if imperial { "\u{b0}F" } else { "\u{b0}C" },
            wind_unit: if imperial { "mph" } else { "km/h" },
            observed: "13:00".into(),
        }
    }

    fn panel_showing(imperial: bool) -> WeatherPanel {
        WeatherPanel {
            status: Arc::new(Mutex::new(Status::Loading)),
            refresh: Arc::new(Mutex::new(false)),
            forecast_hours: 8,
            imperial,
        }
    }

    #[test]
    fn data_already_in_the_display_units_is_left_alone() {
        let panel = panel_showing(true);
        let before = sample_data(true);
        let after = panel.in_display_units(before.clone());
        assert_eq!(after.temperature.to_bits(), before.temperature.to_bits());
        assert_eq!(after.temperature_unit, "\u{b0}F");
    }

    #[test]
    fn fahrenheit_data_is_restated_in_celsius_including_the_forecast() {
        let panel = panel_showing(false);
        let converted = panel.in_display_units(sample_data(true));

        assert!((converted.temperature - 27.777_78).abs() < 0.001);
        assert!((converted.feels_like - 30.0).abs() < 0.001);
        assert_eq!(converted.temperature_unit, "\u{b0}C");
        assert_eq!(converted.wind_unit, "km/h");

        // The forecast rows have to convert too, or the table disagrees with
        // the readout above it.
        assert!(
            (converted.hours[0].temperature - 100.0).abs() < 0.001,
            "212F is 100C, got {}",
            converted.hours[0].temperature
        );
        assert!((converted.hours[0].feels_like - 0.0).abs() < 0.001);
        assert!((converted.hours[0].wind - 16.093_44).abs() < 0.001);
    }

    #[test]
    fn celsius_data_is_restated_in_fahrenheit() {
        let panel = panel_showing(true);
        let converted = panel.in_display_units(sample_data(false));
        assert!((converted.temperature - 82.0).abs() < 0.01);
        assert!((converted.hours[0].temperature - 212.0).abs() < 0.01);
        assert!((converted.wind - 6.0).abs() < 0.01);
        assert_eq!(converted.temperature_unit, "\u{b0}F");
    }

    #[test]
    fn converting_there_and_back_returns_the_original_reading() {
        let out = panel_showing(false).in_display_units(sample_data(true));
        let back = panel_showing(true).in_display_units(out);
        let original = sample_data(true);
        assert!((back.temperature - original.temperature).abs() < 0.001);
        assert!((back.wind - original.wind).abs() < 0.001);
    }

    #[test]
    fn u_switches_units_and_is_documented() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut panel = panel_showing(true);
        assert!(
            BINDINGS.iter().any(|b| b.key == "u"),
            "a key nobody is told about might as well not exist"
        );

        let outcome = panel.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        assert_eq!(outcome, crate::panel::KeyOutcome::Consumed);
        assert!(!panel.imperial, "the first press switches to metric");
        panel.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        assert!(panel.imperial, "and the second switches back");
    }

    fn sample() -> ForecastResponse {
        serde_json::from_str(
            r#"{
            "current":{"time":"2026-07-25T14:00","temperature_2m":71.2,
                       "apparent_temperature":70.0,"weather_code":3,
                       "wind_speed_10m":8.1,"relative_humidity_2m":58},
            "hourly":{
              "time":["2026-07-25T12:00","2026-07-25T13:00","2026-07-25T14:00",
                      "2026-07-25T15:00","2026-07-25T16:00"],
              "temperature_2m":[68.0,70.0,71.2,72.5,73.0],
              "apparent_temperature":[67.0,69.0,70.0,71.0,72.0],
              "weather_code":[0,1,3,61,80],
              "wind_speed_10m":[5.0,6.0,8.1,9.0,10.0],
              "precipitation_probability":[0,5,10,60,80]
            }}"#,
        )
        .expect("sample must parse")
    }

    #[test]
    fn urlencode_escapes_spaces_and_punctuation() {
        assert_eq!(urlencode("Boston"), "Boston");
        assert_eq!(urlencode("New York"), "New%20York");
        assert_eq!(urlencode("a,b"), "a%2Cb");
        assert_eq!(urlencode("Zürich"), "Z%C3%BCrich");
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn the_forecast_starts_at_the_current_hour_not_at_midnight() {
        let hours = upcoming_hours(&sample(), 10);
        assert_eq!(hours[0].hour, 14, "past hours must be dropped");
        assert_eq!(hours.len(), 3);
    }

    #[test]
    fn the_forecast_respects_the_requested_length() {
        let hours = upcoming_hours(&sample(), 2);
        assert_eq!(hours.len(), 2);
        assert_eq!(hours[0].hour, 14);
        assert_eq!(hours[1].hour, 15);
    }

    #[test]
    fn hourly_values_line_up_with_their_timestamps() {
        let hours = upcoming_hours(&sample(), 10);
        // Index 2 in the source arrays is 14:00.
        assert!((hours[0].temperature - 71.2).abs() < f64::EPSILON);
        assert_eq!(hours[0].code, 3);
        assert_eq!(hours[0].precipitation_chance, Some(10));
        assert_eq!(hours[1].precipitation_chance, Some(60));
    }

    #[test]
    fn a_current_time_past_every_hour_falls_back_to_the_whole_span() {
        let mut parsed = sample();
        parsed.current.time = "2026-07-26T23:00".into();
        let hours = upcoming_hours(&parsed, 3);
        assert_eq!(hours.len(), 3, "must not return an empty forecast");
        assert_eq!(hours[0].hour, 12);
    }

    #[test]
    fn hours_parse_out_of_iso_timestamps() {
        assert_eq!(parse_hour("2026-07-25T14:00"), Some(14));
        assert_eq!(parse_hour("2026-07-25T00:00"), Some(0));
        assert_eq!(parse_hour("2026-07-25T23:00"), Some(23));
        assert_eq!(parse_hour("nonsense"), None);
        assert_eq!(parse_hour(""), None);
    }

    #[test]
    fn observation_labels_keep_hours_and_minutes() {
        assert_eq!(hour_label("2026-07-25T14:30"), Some("14:30".to_string()));
        assert_eq!(hour_label("2026-07-25T09:00"), Some("09:00".to_string()));
        assert_eq!(hour_label("broken"), None);
    }

    #[test]
    fn missing_optional_hourly_fields_do_not_drop_the_hour() {
        let parsed: ForecastResponse = serde_json::from_str(
            r#"{
            "current":{"time":"2026-07-25T14:00","temperature_2m":71.2,
                       "apparent_temperature":70.0,"weather_code":3,"wind_speed_10m":8.1},
            "hourly":{"time":["2026-07-25T14:00"],"temperature_2m":[71.2],
                      "apparent_temperature":[70.0],"weather_code":[3],
                      "wind_speed_10m":[8.1]}
        }"#,
        )
        .expect("precipitation is optional");
        let hours = upcoming_hours(&parsed, 5);
        assert_eq!(hours.len(), 1);
        assert_eq!(hours[0].precipitation_chance, None);
    }

    #[test]
    fn geocode_responses_parse() {
        let json = r#"{"results":[{"name":"Boston","latitude":42.36,"longitude":-71.06,
                        "admin1":"Massachusetts","country_code":"US"}]}"#;
        let parsed: GeocodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.results[0].name, "Boston");
    }

    #[test]
    fn an_empty_geocode_result_set_parses_as_empty() {
        let parsed: GeocodeResponse = serde_json::from_str("{}").unwrap();
        assert!(parsed.results.is_empty());
    }

    #[test]
    fn explicit_coordinates_skip_geocoding() {
        let config = WeatherConfig {
            location: "Anywhere".into(),
            latitude: Some(42.36),
            longitude: Some(-71.06),
            ..Default::default()
        };
        let located = resolve_location(&config).expect("no network needed");
        assert!((located.latitude - 42.36).abs() < f64::EPSILON);
        assert_eq!(located.name, "Anywhere");
    }

    #[test]
    fn a_blank_location_with_no_coordinates_is_an_error() {
        let config = WeatherConfig {
            location: "  ".into(),
            latitude: None,
            longitude: None,
            ..Default::default()
        };
        let err = resolve_location(&config).expect_err("must fail");
        assert!(err.to_string().contains("no location set"));
    }

    #[test]
    fn every_forecast_column_fits_a_reasonable_panel() {
        // The default layout gives weather roughly 60 columns; the header must
        // fill exactly that with no column collapsing to zero.
        let grid = Grid::new(COLUMNS, 60);
        assert!(grid.has("hour"));
        assert!(grid.has("temp"));
        assert!(grid.has("rain"));
    }

    #[test]
    fn narrow_panels_drop_optional_columns_rather_than_squeezing() {
        let grid = Grid::new(COLUMNS, 30);
        assert!(grid.has("hour"), "the hour is the whole point of the row");
        assert!(grid.has("temp"));
        assert!(!grid.has("wind"));
        assert!(!grid.has("feels"));
    }
}
