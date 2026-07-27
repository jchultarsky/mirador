//! Colour theme.
//!
//! Two kinds of value live here. Flat colours name a role — `border`,
//! `label`, `error` — and are used directly. Gradient *stops* name the ends of
//! a ramp; [`Theme::gradients`] bakes them into lookup tables at startup.
//!
//! Gradients follow btop's three-stop model: `start` is a dark, desaturated
//! form of the hue and `end` is the bright one, so a quiet graph recedes into
//! the background on its own and a busy one comes forward without any
//! per-frame decision. The same ramp colours the graph, the meter and the
//! number, which is what makes a panel change temperature all at once.
//!
//! Colours accept a name (`red`, `light-blue`), a hex string (`#d7af87`), a
//! 256-colour index (`179`), or `reset` for the terminal's own default.

use ratatui::style::Color;
use serde::{Deserialize, Deserializer};

use crate::chart::Gradient;

/// Parse a `Color` from its string form, reporting a useful error on failure.
fn parse_color<E: serde::de::Error>(raw: &str) -> Result<Color, E> {
    raw.parse::<Color>().map_err(|_| {
        serde::de::Error::custom(format!(
            "`{raw}` is not a colour; use a name (`red`, `light-blue`), \
             a hex string (`#ff8800`), a 256-colour index (`12`), or `reset`"
        ))
    })
}

fn de_color<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
    let raw = String::deserialize(deserializer)?;
    parse_color(&raw)
}

fn de_opt_color<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Color>, D::Error> {
    let raw = Option::<String>::deserialize(deserializer)?;
    match raw {
        None => Ok(None),
        Some(s) if s.trim().is_empty() => Ok(None),
        Some(s) => parse_color(&s).map(Some),
    }
}

/// The three stops of a colour ramp.
///
/// `mid` and `end` are optional: with neither, the ramp is flat; with `end`
/// only, it is a single linear segment.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GradientStops {
    #[serde(deserialize_with = "de_color")]
    pub start: Color,
    #[serde(deserialize_with = "de_opt_color")]
    pub mid: Option<Color>,
    #[serde(deserialize_with = "de_opt_color")]
    pub end: Option<Color>,
}

impl Default for GradientStops {
    fn default() -> Self {
        Self {
            start: Color::Reset,
            mid: None,
            end: None,
        }
    }
}

impl GradientStops {
    fn of(start: (u8, u8, u8), mid: (u8, u8, u8), end: (u8, u8, u8)) -> Self {
        Self {
            start: Color::Rgb(start.0, start.1, start.2),
            mid: Some(Color::Rgb(mid.0, mid.1, mid.2)),
            end: Some(Color::Rgb(end.0, end.1, end.2)),
        }
    }

    /// Bake the stops into a lookup table.
    pub fn bake(&self) -> Gradient {
        Gradient::new(self.start, self.mid, self.end)
    }
}

/// Every colour the dashboard draws with.
///
/// `deny_unknown_fields` matters more here than it looks. Without it a
/// misspelled colour is accepted and silently ignored — and because the
/// pre-0.1.0 theme keys `rx` and `tx` also parsed clean, the
/// `--migrate-config` hint that names them could never fire.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    /// Frame of an unfocused panel.
    #[serde(deserialize_with = "de_color")]
    pub border: Color,
    /// Frame of the focused panel.
    #[serde(deserialize_with = "de_color")]
    pub border_focused: Color,
    /// Interior rules, dimmer than any frame so they read as subordinate.
    #[serde(deserialize_with = "de_color")]
    pub rule: Color,
    /// Panel titles.
    #[serde(deserialize_with = "de_color")]
    pub title: Color,
    /// Primary body text.
    #[serde(deserialize_with = "de_color")]
    pub text: Color,
    /// De-emphasised text: units, hints, secondary values.
    #[serde(deserialize_with = "de_color")]
    pub muted: Color,
    /// Engraved labels and tags.
    #[serde(deserialize_with = "de_color")]
    pub label: Color,
    /// Selection, focus and the chronometer.
    #[serde(deserialize_with = "de_color")]
    pub accent: Color,
    /// The highlighted letter of a key hint.
    #[serde(deserialize_with = "de_color")]
    pub key: Color,
    /// Success, nominal, gains.
    #[serde(deserialize_with = "de_color")]
    pub success: Color,
    /// Due today, elevated load.
    #[serde(deserialize_with = "de_color")]
    pub warning: Color,
    /// Overdue, critical load, losses.
    #[serde(deserialize_with = "de_color")]
    pub error: Color,
    /// The unfilled part of a meter or the baseline of a graph.
    #[serde(deserialize_with = "de_color")]
    pub track: Color,

    /// Ramp for CPU load.
    pub cpu_gradient: GradientStops,
    /// Ramp for bytes received.
    pub rx_gradient: GradientStops,
    /// Ramp for bytes transmitted.
    pub tx_gradient: GradientStops,
    /// Ramp for a rising position.
    pub gain_gradient: GradientStops,
    /// Ramp for a falling position.
    pub loss_gradient: GradientStops,
}

impl Default for Theme {
    /// The shipped palette: an aged instrument panel.
    ///
    /// Brass for the instruments, verdigris for engraved labels, slate for
    /// chrome. Body text is `reset`, so it inherits whatever foreground the
    /// user has already tuned their terminal to.
    fn default() -> Self {
        Self {
            border: Color::Rgb(0x3a, 0x3a, 0x3a),
            border_focused: Color::Rgb(0xd7, 0xaf, 0x87),
            rule: Color::Rgb(0x30, 0x30, 0x30),
            title: Color::Rgb(0xd7, 0xaf, 0x87),
            text: Color::Reset,
            muted: Color::Rgb(0x70, 0x70, 0x70),
            label: Color::Rgb(0x5f, 0x87, 0x87),
            accent: Color::Rgb(0xd7, 0xaf, 0x87),
            key: Color::Rgb(0xd7, 0xaf, 0x87),
            success: Color::Rgb(0x87, 0xaf, 0x5f),
            warning: Color::Rgb(0xd7, 0xaf, 0x5f),
            error: Color::Rgb(0xd7, 0x5f, 0x5f),
            track: Color::Rgb(0x30, 0x30, 0x30),

            // Dark and desaturated at the cool end, bright at the hot end.
            cpu_gradient: GradientStops::of(
                (0x3f, 0x5f, 0x4f),
                (0xd7, 0xaf, 0x5f),
                (0xd7, 0x5f, 0x5f),
            ),
            rx_gradient: GradientStops::of(
                (0x2f, 0x4f, 0x3f),
                (0x6f, 0x9f, 0x5f),
                (0xaf, 0xd7, 0x87),
            ),
            tx_gradient: GradientStops::of(
                (0x4a, 0x3a, 0x5a),
                (0x8a, 0x6f, 0x9f),
                (0xc7, 0xaf, 0xd7),
            ),
            gain_gradient: GradientStops::of(
                (0x3f, 0x5f, 0x3f),
                (0x6f, 0x9f, 0x5f),
                (0xaf, 0xd7, 0x87),
            ),
            loss_gradient: GradientStops::of(
                (0x5f, 0x3a, 0x3a),
                (0xaf, 0x5f, 0x5f),
                (0xd7, 0x87, 0x87),
            ),
        }
    }
}

/// Gradients baked once at startup, so drawing a frame is array lookups.
#[derive(Debug)]
pub struct Gradients {
    pub cpu: Gradient,
    pub rx: Gradient,
    pub tx: Gradient,
    /// For a rising position. Unused until the watchlist panel lands.
    #[allow(dead_code)]
    pub gain: Gradient,
    /// For a falling position. Unused until the watchlist panel lands.
    #[allow(dead_code)]
    pub loss: Gradient,
}

impl Theme {
    /// Bake every configured ramp.
    pub fn gradients(&self) -> Gradients {
        Gradients {
            cpu: self.cpu_gradient.bake(),
            rx: self.rx_gradient.bake(),
            tx: self.tx_gradient.bake(),
            gain: self.gain_gradient.bake(),
            loss: self.loss_gradient.bake(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_theme_bakes_without_panicking() {
        let theme = Theme::default();
        let gradients = theme.gradients();
        // Every ramp must span its full range.
        assert_ne!(gradients.cpu.at(0), gradients.cpu.at(100));
        assert_ne!(gradients.rx.at(0), gradients.rx.at(100));
    }

    #[test]
    fn body_text_defaults_to_the_terminal_foreground() {
        // Hard-coding a text colour would fight the user's own theme.
        assert_eq!(Theme::default().text, Color::Reset);
    }

    #[test]
    fn colours_parse_in_every_documented_form() {
        let toml = r##"
            border = "red"
            border_focused = "#d7af87"
            title = "179"
            text = "reset"
            muted = "light-blue"
        "##;
        let theme: Theme = toml::from_str(toml).expect("all forms must parse");
        assert_eq!(theme.border, Color::Red);
        assert_eq!(theme.border_focused, Color::Rgb(0xd7, 0xaf, 0x87));
        assert_eq!(theme.text, Color::Reset);
    }

    #[test]
    fn a_bad_colour_names_itself_and_shows_valid_forms() {
        let err = toml::from_str::<Theme>(r#"accent = "chartreuse""#).expect_err("must fail");
        let message = err.to_string();
        assert!(message.contains("chartreuse"), "got: {message}");
        assert!(message.contains("#ff8800"), "got: {message}");
    }

    #[test]
    fn omitted_keys_fall_back_to_defaults() {
        let theme: Theme = toml::from_str(r#"accent = "red""#).expect("partial themes are valid");
        assert_eq!(theme.accent, Color::Red);
        assert_eq!(theme.border, Theme::default().border);
    }

    #[test]
    fn a_gradient_with_only_a_start_is_flat() {
        let stops: GradientStops =
            toml::from_str(r##"start = "#102030""##).expect("start alone is valid");
        let baked = stops.bake();
        assert_eq!(baked.at(0), baked.at(100));
    }

    #[test]
    fn an_empty_gradient_stop_is_treated_as_absent() {
        // Writing `end = ""` to disable a stop should not be an error.
        let stops: GradientStops =
            toml::from_str("start = \"#102030\"\nend = \"\"").expect("empty means unset");
        assert!(stops.end.is_none());
        assert_eq!(stops.bake().at(0), stops.bake().at(100));
    }

    #[test]
    fn a_two_stop_gradient_needs_no_midpoint() {
        let stops: GradientStops =
            toml::from_str("start = \"#000000\"\nend = \"#ffffff\"").expect("two stops are valid");
        let baked = stops.bake();
        assert_ne!(baked.at(0), baked.at(100));
    }

    #[test]
    fn gradients_can_be_overridden_from_config() {
        #[derive(Deserialize)]
        struct Wrapper {
            theme: Theme,
        }
        let parsed: Wrapper = toml::from_str(
            r##"
            [theme]
            [theme.cpu_gradient]
            start = "#000000"
            end = "#ffffff"
            "##,
        )
        .expect("nested gradient tables must parse");
        let baked = parsed.theme.cpu_gradient.bake();
        assert_eq!(baked.at(0), Color::Rgb(0, 0, 0));
        assert_eq!(baked.at(100), Color::Rgb(255, 255, 255));
    }
}
