//! The world clocks you are watching, as a data file.
//!
//! Same reasoning as [`crate::quote::Watchlist`], and the same shape: mirador
//! deliberately never rewrites the config, so a list that lived in `[clocks]`
//! could only ever be changed in an editor. The config seeds the first run and
//! nothing after it.
//!
//! `[layout]` is the exception that proves the rule — it goes back to the
//! config because people read and curate it. A list of cities is not that.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::ClockZone;

/// Cities offered when adding a clock, and the zone each one is in.
///
/// Many cities to one zone, deliberately — that is the entire point. The IANA
/// identifier names *a* city in the zone, and it is very often not the one the
/// reader has in mind: Seattle is `America/Los_Angeles`, Bengaluru is
/// `Asia/Kolkata`, Boston is `America/New_York`. Asking someone to know that
/// before they can add a clock is asking the wrong question.
///
/// Curated rather than generated. The tz database has around six hundred zones
/// and no city labels beyond the identifiers themselves, so generating this
/// would reproduce exactly the problem it exists to solve.
///
/// Anything not here still works: the prompt falls back to whatever was typed,
/// and jiff knows the whole database.
pub const PLACES: &[Place] = &[
    Place {
        city: "Auckland",
        tz: "Pacific/Auckland",
    },
    Place {
        city: "Wellington",
        tz: "Pacific/Auckland",
    },
    Place {
        city: "Sydney",
        tz: "Australia/Sydney",
    },
    Place {
        city: "Canberra",
        tz: "Australia/Sydney",
    },
    Place {
        city: "Melbourne",
        tz: "Australia/Melbourne",
    },
    Place {
        city: "Brisbane",
        tz: "Australia/Brisbane",
    },
    Place {
        city: "Perth",
        tz: "Australia/Perth",
    },
    Place {
        city: "Adelaide",
        tz: "Australia/Adelaide",
    },
    Place {
        city: "Tokyo",
        tz: "Asia/Tokyo",
    },
    Place {
        city: "Osaka",
        tz: "Asia/Tokyo",
    },
    Place {
        city: "Seoul",
        tz: "Asia/Seoul",
    },
    Place {
        city: "Taipei",
        tz: "Asia/Taipei",
    },
    Place {
        city: "Shanghai",
        tz: "Asia/Shanghai",
    },
    Place {
        city: "Beijing",
        tz: "Asia/Shanghai",
    },
    Place {
        city: "Shenzhen",
        tz: "Asia/Shanghai",
    },
    Place {
        city: "Hong Kong",
        tz: "Asia/Hong_Kong",
    },
    Place {
        city: "Singapore",
        tz: "Asia/Singapore",
    },
    Place {
        city: "Manila",
        tz: "Asia/Manila",
    },
    Place {
        city: "Jakarta",
        tz: "Asia/Jakarta",
    },
    Place {
        city: "Bangkok",
        tz: "Asia/Bangkok",
    },
    Place {
        city: "Hanoi",
        tz: "Asia/Bangkok",
    },
    Place {
        city: "Ho Chi Minh City",
        tz: "Asia/Ho_Chi_Minh",
    },
    Place {
        city: "Kuala Lumpur",
        tz: "Asia/Kuala_Lumpur",
    },
    Place {
        city: "Yangon",
        tz: "Asia/Yangon",
    },
    Place {
        city: "Dhaka",
        tz: "Asia/Dhaka",
    },
    Place {
        city: "Kathmandu",
        tz: "Asia/Kathmandu",
    },
    Place {
        city: "Mumbai",
        tz: "Asia/Kolkata",
    },
    Place {
        city: "Delhi",
        tz: "Asia/Kolkata",
    },
    Place {
        city: "Bengaluru",
        tz: "Asia/Kolkata",
    },
    Place {
        city: "Chennai",
        tz: "Asia/Kolkata",
    },
    Place {
        city: "Hyderabad",
        tz: "Asia/Kolkata",
    },
    Place {
        city: "Kolkata",
        tz: "Asia/Kolkata",
    },
    Place {
        city: "Karachi",
        tz: "Asia/Karachi",
    },
    Place {
        city: "Lahore",
        tz: "Asia/Karachi",
    },
    Place {
        city: "Islamabad",
        tz: "Asia/Karachi",
    },
    Place {
        city: "Tashkent",
        tz: "Asia/Tashkent",
    },
    Place {
        city: "Almaty",
        tz: "Asia/Almaty",
    },
    Place {
        city: "Dubai",
        tz: "Asia/Dubai",
    },
    Place {
        city: "Abu Dhabi",
        tz: "Asia/Dubai",
    },
    Place {
        city: "Muscat",
        tz: "Asia/Dubai",
    },
    Place {
        city: "Tehran",
        tz: "Asia/Tehran",
    },
    Place {
        city: "Baku",
        tz: "Asia/Baku",
    },
    Place {
        city: "Tbilisi",
        tz: "Asia/Tbilisi",
    },
    Place {
        city: "Yerevan",
        tz: "Asia/Yerevan",
    },
    Place {
        city: "Riyadh",
        tz: "Asia/Riyadh",
    },
    Place {
        city: "Doha",
        tz: "Asia/Qatar",
    },
    Place {
        city: "Kuwait City",
        tz: "Asia/Kuwait",
    },
    Place {
        city: "Baghdad",
        tz: "Asia/Baghdad",
    },
    Place {
        city: "Jerusalem",
        tz: "Asia/Jerusalem",
    },
    Place {
        city: "Tel Aviv",
        tz: "Asia/Jerusalem",
    },
    Place {
        city: "Amman",
        tz: "Asia/Amman",
    },
    Place {
        city: "Beirut",
        tz: "Asia/Beirut",
    },
    Place {
        city: "Nicosia",
        tz: "Asia/Nicosia",
    },
    Place {
        city: "Istanbul",
        tz: "Europe/Istanbul",
    },
    Place {
        city: "Ankara",
        tz: "Europe/Istanbul",
    },
    Place {
        city: "Moscow",
        tz: "Europe/Moscow",
    },
    Place {
        city: "Saint Petersburg",
        tz: "Europe/Moscow",
    },
    Place {
        city: "Kyiv",
        tz: "Europe/Kyiv",
    },
    Place {
        city: "Minsk",
        tz: "Europe/Minsk",
    },
    Place {
        city: "Athens",
        tz: "Europe/Athens",
    },
    Place {
        city: "Bucharest",
        tz: "Europe/Bucharest",
    },
    Place {
        city: "Sofia",
        tz: "Europe/Sofia",
    },
    Place {
        city: "Helsinki",
        tz: "Europe/Helsinki",
    },
    Place {
        city: "Tallinn",
        tz: "Europe/Tallinn",
    },
    Place {
        city: "Riga",
        tz: "Europe/Riga",
    },
    Place {
        city: "Vilnius",
        tz: "Europe/Vilnius",
    },
    Place {
        city: "Cairo",
        tz: "Africa/Cairo",
    },
    Place {
        city: "Johannesburg",
        tz: "Africa/Johannesburg",
    },
    Place {
        city: "Cape Town",
        tz: "Africa/Johannesburg",
    },
    Place {
        city: "Nairobi",
        tz: "Africa/Nairobi",
    },
    Place {
        city: "Addis Ababa",
        tz: "Africa/Addis_Ababa",
    },
    Place {
        city: "Lagos",
        tz: "Africa/Lagos",
    },
    Place {
        city: "Accra",
        tz: "Africa/Accra",
    },
    Place {
        city: "Casablanca",
        tz: "Africa/Casablanca",
    },
    Place {
        city: "Berlin",
        tz: "Europe/Berlin",
    },
    Place {
        city: "Munich",
        tz: "Europe/Berlin",
    },
    Place {
        city: "Frankfurt",
        tz: "Europe/Berlin",
    },
    Place {
        city: "Hamburg",
        tz: "Europe/Berlin",
    },
    Place {
        city: "Paris",
        tz: "Europe/Paris",
    },
    Place {
        city: "Madrid",
        tz: "Europe/Madrid",
    },
    Place {
        city: "Barcelona",
        tz: "Europe/Madrid",
    },
    Place {
        city: "Rome",
        tz: "Europe/Rome",
    },
    Place {
        city: "Milan",
        tz: "Europe/Rome",
    },
    Place {
        city: "Amsterdam",
        tz: "Europe/Amsterdam",
    },
    Place {
        city: "Brussels",
        tz: "Europe/Brussels",
    },
    Place {
        city: "Vienna",
        tz: "Europe/Vienna",
    },
    Place {
        city: "Zurich",
        tz: "Europe/Zurich",
    },
    Place {
        city: "Geneva",
        tz: "Europe/Zurich",
    },
    Place {
        city: "Prague",
        tz: "Europe/Prague",
    },
    Place {
        city: "Warsaw",
        tz: "Europe/Warsaw",
    },
    Place {
        city: "Budapest",
        tz: "Europe/Budapest",
    },
    Place {
        city: "Stockholm",
        tz: "Europe/Stockholm",
    },
    Place {
        city: "Oslo",
        tz: "Europe/Oslo",
    },
    Place {
        city: "Copenhagen",
        tz: "Europe/Copenhagen",
    },
    Place {
        city: "London",
        tz: "Europe/London",
    },
    Place {
        city: "Edinburgh",
        tz: "Europe/London",
    },
    Place {
        city: "Manchester",
        tz: "Europe/London",
    },
    Place {
        city: "Dublin",
        tz: "Europe/Dublin",
    },
    Place {
        city: "Lisbon",
        tz: "Europe/Lisbon",
    },
    Place {
        city: "Porto",
        tz: "Europe/Lisbon",
    },
    Place {
        city: "Reykjavik",
        tz: "Atlantic/Reykjavik",
    },
    Place {
        city: "Sao Paulo",
        tz: "America/Sao_Paulo",
    },
    Place {
        city: "Rio de Janeiro",
        tz: "America/Sao_Paulo",
    },
    Place {
        city: "Buenos Aires",
        tz: "America/Argentina/Buenos_Aires",
    },
    Place {
        city: "Montevideo",
        tz: "America/Montevideo",
    },
    Place {
        city: "Santiago",
        tz: "America/Santiago",
    },
    Place {
        city: "Lima",
        tz: "America/Lima",
    },
    Place {
        city: "Bogota",
        tz: "America/Bogota",
    },
    Place {
        city: "Caracas",
        tz: "America/Caracas",
    },
    Place {
        city: "Halifax",
        tz: "America/Halifax",
    },
    Place {
        city: "New York",
        tz: "America/New_York",
    },
    Place {
        city: "Boston",
        tz: "America/New_York",
    },
    Place {
        city: "Washington DC",
        tz: "America/New_York",
    },
    Place {
        city: "Atlanta",
        tz: "America/New_York",
    },
    Place {
        city: "Miami",
        tz: "America/New_York",
    },
    Place {
        city: "Philadelphia",
        tz: "America/New_York",
    },
    Place {
        city: "Toronto",
        tz: "America/Toronto",
    },
    Place {
        city: "Montreal",
        tz: "America/Toronto",
    },
    Place {
        city: "Ottawa",
        tz: "America/Toronto",
    },
    Place {
        city: "Detroit",
        tz: "America/Detroit",
    },
    Place {
        city: "Chicago",
        tz: "America/Chicago",
    },
    Place {
        city: "Dallas",
        tz: "America/Chicago",
    },
    Place {
        city: "Houston",
        tz: "America/Chicago",
    },
    Place {
        city: "Austin",
        tz: "America/Chicago",
    },
    Place {
        city: "Minneapolis",
        tz: "America/Chicago",
    },
    Place {
        city: "Mexico City",
        tz: "America/Mexico_City",
    },
    Place {
        city: "Winnipeg",
        tz: "America/Winnipeg",
    },
    Place {
        city: "Denver",
        tz: "America/Denver",
    },
    Place {
        city: "Salt Lake City",
        tz: "America/Denver",
    },
    Place {
        city: "Calgary",
        tz: "America/Edmonton",
    },
    Place {
        city: "Edmonton",
        tz: "America/Edmonton",
    },
    Place {
        city: "Phoenix",
        tz: "America/Phoenix",
    },
    Place {
        city: "Los Angeles",
        tz: "America/Los_Angeles",
    },
    Place {
        city: "San Francisco",
        tz: "America/Los_Angeles",
    },
    Place {
        city: "Seattle",
        tz: "America/Los_Angeles",
    },
    Place {
        city: "Portland",
        tz: "America/Los_Angeles",
    },
    Place {
        city: "San Diego",
        tz: "America/Los_Angeles",
    },
    Place {
        city: "Las Vegas",
        tz: "America/Los_Angeles",
    },
    Place {
        city: "Vancouver",
        tz: "America/Vancouver",
    },
    Place {
        city: "Anchorage",
        tz: "America/Anchorage",
    },
    Place {
        city: "Honolulu",
        tz: "Pacific/Honolulu",
    },
    Place {
        city: "UTC",
        tz: "UTC",
    },
    Place {
        city: "Local time",
        tz: "local",
    },
];

/// A city and the zone it keeps time in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub city: &'static str,
    pub tz: &'static str,
}

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

    /// Move the zone at `index` one place towards the top of the list.
    ///
    /// Refuses to move into the first slot, for the same reason [`Self::remove`]
    /// refuses to empty it: that entry is the big clock, and promoting a
    /// secondary would silently demote the one the reader chose to see from
    /// across the room. Reordering the *table* is what was asked for; choosing
    /// the primary is a different decision and would want its own key.
    pub fn move_up(&mut self, index: usize) -> bool {
        if index <= 1 || index >= self.clocks.len() {
            return false;
        }
        self.clocks.swap(index, index - 1);
        self.dirty = true;
        true
    }

    /// Move the zone at `index` one place towards the bottom of the list.
    pub fn move_down(&mut self, index: usize) -> bool {
        if index == 0 || index + 1 >= self.clocks.len() {
            return false;
        }
        self.clocks.swap(index, index + 1);
        self.dirty = true;
        true
    }

    /// Replace the label and timezone of the zone at `index`.
    ///
    /// Returns false when the timezone is blank, or when it names a zone some
    /// *other* entry already shows. Comparing against other entries rather than
    /// all of them is what lets an edit change only the label — the common case,
    /// and the one the reporter wanted — without the entry colliding with
    /// itself.
    pub fn edit(&mut self, index: usize, label: &str, timezone: &str) -> bool {
        let timezone = timezone.trim();
        if timezone.is_empty() || index >= self.clocks.len() {
            return false;
        }
        if self
            .clocks
            .iter()
            .enumerate()
            .any(|(i, z)| i != index && z.timezone == timezone)
        {
            return false;
        }
        // Same rule as `add`: an emptied label falls back to the city in the
        // timezone, so clearing the field is a way to get the default back
        // rather than a way to end up with a nameless clock.
        let label = match label.trim() {
            "" => timezone
                .rsplit('/')
                .next()
                .unwrap_or(timezone)
                .replace('_', " "),
            given => given.to_string(),
        };
        self.clocks[index] = ClockZone {
            label,
            timezone: timezone.to_string(),
        };
        self.dirty = true;
        true
    }

    /// Where the zones are stored, for the panel's `o`.
    pub fn path(&self) -> &Path {
        &self.path
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

    fn three() -> Zones {
        Zones {
            path: PathBuf::from("/nonexistent/zones.toml"),
            clocks: vec![
                zone("Local", "local"),
                zone("Tokyo", "Asia/Tokyo"),
                zone("London", "Europe/London"),
            ],
            dirty: false,
            last_error: None,
        }
    }

    fn labels(zones: &Zones) -> Vec<&str> {
        zones.zones().iter().map(|z| z.label.as_str()).collect()
    }

    #[test]
    fn a_clock_can_be_moved_through_the_list() {
        let mut zones = three();
        assert!(zones.move_down(1));
        assert_eq!(labels(&zones), ["Local", "London", "Tokyo"]);
        assert!(zones.move_up(2));
        assert_eq!(labels(&zones), ["Local", "Tokyo", "London"]);
    }

    /// The first entry is the big clock. `remove` refuses to take it and this
    /// refuses to displace it, for the same reason: promoting a secondary would
    /// silently demote the clock the reader chose to see from across the room.
    #[test]
    fn nothing_can_be_moved_into_the_big_clocks_slot() {
        let mut zones = three();
        assert!(!zones.move_up(1), "moving the top secondary up must refuse");
        assert!(!zones.move_up(0), "the primary itself cannot move either");
        assert!(!zones.move_down(0));
        assert_eq!(labels(&zones), ["Local", "Tokyo", "London"]);
    }

    #[test]
    fn moving_off_either_end_is_refused_rather_than_wrapping() {
        let mut zones = three();
        assert!(!zones.move_down(2), "the last entry has nowhere to go");
        assert!(!zones.move_down(9), "and neither has one off the end");
        assert!(!zones.move_up(9));
        assert_eq!(labels(&zones), ["Local", "Tokyo", "London"]);
    }

    /// Relabelling without touching the timezone is the case the reporter
    /// actually wanted, and the one a naive duplicate check breaks: the entry
    /// collides with itself and the edit is refused.
    #[test]
    fn an_entry_can_be_relabelled_without_changing_its_zone() {
        let mut zones = three();
        assert!(zones.edit(1, "Japan", "Asia/Tokyo"));
        assert_eq!(labels(&zones), ["Local", "Japan", "London"]);
        assert_eq!(zones.zones()[1].timezone, "Asia/Tokyo");
    }

    #[test]
    fn an_edit_onto_a_zone_another_clock_shows_is_refused() {
        let mut zones = three();
        assert!(
            !zones.edit(1, "Britain", "Europe/London"),
            "that zone is already on the panel"
        );
        assert_eq!(labels(&zones), ["Local", "Tokyo", "London"]);
    }

    /// Same rule as `add`: an emptied label falls back to the city in the
    /// timezone, so clearing the field restores the default rather than
    /// leaving a nameless clock.
    #[test]
    fn clearing_the_label_falls_back_to_the_city_in_the_zone() {
        let mut zones = three();
        assert!(zones.edit(1, "   ", "America/New_York"));
        assert_eq!(labels(&zones), ["Local", "New York", "London"]);
    }

    #[test]
    fn an_edit_out_of_range_or_with_a_blank_zone_changes_nothing() {
        let mut zones = three();
        assert!(!zones.edit(9, "Nowhere", "Asia/Tokyo"));
        assert!(!zones.edit(1, "Tokyo", "   "));
        assert_eq!(labels(&zones), ["Local", "Tokyo", "London"]);
    }

    /// Every mutation has to mark the list dirty, or the change is on screen
    /// and never reaches the file — the failure is invisible until a restart.
    #[test]
    fn every_change_marks_the_list_for_saving() {
        for (name, mutate) in [
            (
                "move_down",
                &(|z: &mut Zones| z.move_down(1)) as &dyn Fn(&mut Zones) -> bool,
            ),
            ("move_up", &|z: &mut Zones| z.move_up(2)),
            ("edit", &|z: &mut Zones| z.edit(1, "Japan", "Asia/Tokyo")),
        ] {
            let mut zones = three();
            assert!(!zones.dirty, "starts clean");
            assert!(mutate(&mut zones), "{name} should have applied");
            assert!(zones.dirty, "{name} left the list unsaved");
        }
    }

    /// Every zone in the picker has to be one jiff will actually accept.
    /// Choosing a city from a list and being told the zone is unknown would be
    /// the list's fault, and there is nothing the reader could do about it.
    #[test]
    fn every_offered_timezone_resolves() {
        for place in PLACES {
            if place.tz == "local" {
                continue;
            }
            assert!(
                jiff::tz::TimeZone::get(place.tz).is_ok(),
                "`{}` is offered for {} but does not resolve",
                place.tz,
                place.city
            );
        }
    }
}
