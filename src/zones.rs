//! The world clocks you are watching, as a data file.
//!
//! Same reasoning as [`crate::quote::Watchlist`], and the same shape: mirador
//! deliberately never rewrites the config, so a list that lived in `[clocks]`
//! could only ever be changed in an editor. The config seeds the first run and
//! nothing after it.
//!
//! `[layout]` is the exception that proves the rule — it goes back to the
//! config because people read and curate it. A list of cities is not that.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::ClockZone;

/// Timezone names offered for completion.
///
/// Deliberately a short list of the ones people actually type rather than the
/// six hundred in the IANA database. Completion exists so you do not have to
/// remember whether it is `Asia/Calcutta` or `Asia/Kolkata`; it is not a
/// browser, and a list this size fits in the binary for nothing.
///
/// Anything else still works — the name is handed to jiff, which knows the
/// whole database. This only decides what `Tab` can finish for you.
pub const COMMON: &[&str] = &[
    "Africa/Cairo",
    "Africa/Johannesburg",
    "Africa/Lagos",
    "Africa/Nairobi",
    "America/Argentina/Buenos_Aires",
    "America/Bogota",
    "America/Chicago",
    "America/Denver",
    "America/Halifax",
    "America/Los_Angeles",
    "America/Mexico_City",
    "America/New_York",
    "America/Phoenix",
    "America/Sao_Paulo",
    "America/Toronto",
    "America/Vancouver",
    "Asia/Bangkok",
    "Asia/Dubai",
    "Asia/Hong_Kong",
    "Asia/Jakarta",
    "Asia/Jerusalem",
    "Asia/Kolkata",
    "Asia/Manila",
    "Asia/Seoul",
    "Asia/Shanghai",
    "Asia/Singapore",
    "Asia/Taipei",
    "Asia/Tokyo",
    "Australia/Brisbane",
    "Australia/Melbourne",
    "Australia/Perth",
    "Australia/Sydney",
    "Europe/Amsterdam",
    "Europe/Athens",
    "Europe/Berlin",
    "Europe/Brussels",
    "Europe/Bucharest",
    "Europe/Copenhagen",
    "Europe/Dublin",
    "Europe/Helsinki",
    "Europe/Istanbul",
    "Europe/Lisbon",
    "Europe/London",
    "Europe/Madrid",
    "Europe/Moscow",
    "Europe/Oslo",
    "Europe/Paris",
    "Europe/Prague",
    "Europe/Rome",
    "Europe/Stockholm",
    "Europe/Vienna",
    "Europe/Warsaw",
    "Europe/Zurich",
    "Pacific/Auckland",
    "Pacific/Honolulu",
    "UTC",
    "local",
];

/// The clocks on screen, in order.
#[derive(Debug)]
pub struct Zones {
    path: PathBuf,
    clocks: Vec<ClockZone>,
    dirty: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ZonesFile {
    #[serde(default)]
    zones: Vec<ClockZone>,
}

impl Zones {
    /// Load from `path`, falling back to `seed` when the file does not exist.
    pub fn load(path: impl Into<PathBuf>, seed: &[ClockZone]) -> Result<Self> {
        let path = path.into();
        let (zones, dirty) = if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("reading the clock zones from {}", path.display()))?;
            let parsed: ZonesFile = toml::from_str(&raw)
                .with_context(|| format!("parsing the clock zones in {}", path.display()))?;
            (parsed.zones, false)
        } else {
            // Seeded and marked dirty, so the file exists to be hand-edited
            // after the first run.
            (seed.to_vec(), !seed.is_empty())
        };

        Ok(Self {
            path,
            clocks: zones,
            dirty,
            last_error: None,
        })
    }

    pub fn zones(&self) -> &[ClockZone] {
        &self.clocks
    }

    /// Add a zone. Returns false when the timezone is blank or already shown.
    pub fn add(&mut self, label: &str, timezone: &str) -> bool {
        let timezone = timezone.trim();
        if timezone.is_empty() || self.clocks.iter().any(|z| z.timezone == timezone) {
            return false;
        }
        // An unlabelled clock takes the city out of the timezone name, which is
        // what someone typing `Europe/Lisbon` meant by it.
        let label = match label.trim() {
            "" => timezone
                .rsplit('/')
                .next()
                .unwrap_or(timezone)
                .replace('_', " "),
            given => given.to_string(),
        };
        self.clocks.push(ClockZone {
            label,
            timezone: timezone.to_string(),
        });
        self.dirty = true;
        true
    }

    /// Remove the zone at `index`.
    ///
    /// Refuses to remove the first, which is the panel's big clock: a clock
    /// panel with no primary has nothing to draw large, and the config cannot
    /// express one either.
    pub fn remove(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.clocks.len() {
            return false;
        }
        self.clocks.remove(index);
        self.dirty = true;
        true
    }

    /// Write atomically if there are pending changes.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let body = toml::to_string_pretty(&ZonesFile {
            zones: self.clocks.clone(),
        })
        .context("serialising the clock zones")?;
        let contents = format!(
            "# mirador world clocks. Safe to edit by hand.\n\
             # The first is the one drawn large. `[clocks].zones` in your config\n\
             # seeds this on a first run and is not read again.\n\n{body}"
        );

        crate::store::write_atomic(&self.path, &contents)?;
        self.dirty = false;
        Ok(())
    }

    pub fn save_reporting(&mut self) {
        crate::store::report(self.save(), &mut self.last_error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(label: &str, timezone: &str) -> ClockZone {
        ClockZone {
            label: label.into(),
            timezone: timezone.into(),
        }
    }

    fn seeded() -> Zones {
        Zones {
            path: PathBuf::from("/nonexistent/zones.toml"),
            clocks: vec![zone("Local", "local"), zone("Tokyo", "Asia/Tokyo")],
            dirty: false,
            last_error: None,
        }
    }

    #[test]
    fn a_zone_already_shown_is_not_added_twice() {
        let mut zones = seeded();
        assert!(!zones.add("Japan", "Asia/Tokyo"), "already there");
        assert_eq!(zones.zones().len(), 2);
        assert!(!zones.dirty, "and nothing needs writing");
    }

    #[test]
    fn an_unlabelled_zone_is_named_after_its_city() {
        let mut zones = seeded();
        assert!(zones.add("", "America/New_York"));
        assert_eq!(zones.zones()[2].label, "New York");
    }

    /// The first zone is the big clock. Removing it would leave the panel with
    /// nothing to draw large, and the config has no way to describe that.
    #[test]
    fn the_primary_clock_cannot_be_removed() {
        let mut zones = seeded();
        assert!(!zones.remove(0), "the big clock stays");
        assert!(!zones.remove(9), "and so does an index that is not there");
        assert!(zones.remove(1));
        assert_eq!(zones.zones().len(), 1);
    }

    #[test]
    fn a_blank_timezone_is_refused() {
        let mut zones = seeded();
        assert!(!zones.add("Nowhere", "   "));
        assert_eq!(zones.zones().len(), 2);
    }

    /// Every name offered for completion has to be one jiff will actually
    /// accept, or `Tab` would help you type something that is then rejected.
    #[test]
    fn every_offered_timezone_resolves() {
        for name in COMMON {
            if *name == "local" {
                continue;
            }
            assert!(
                jiff::tz::TimeZone::get(name).is_ok(),
                "`{name}` is offered for completion but does not resolve"
            );
        }
    }
}
