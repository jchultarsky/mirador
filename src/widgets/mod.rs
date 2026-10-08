//! Built-in widgets and the registry that turns a config name into a panel.
//!
//! Built-ins are compiled here. Explicit external declarations fall through
//! to the process-protocol adapter; they do not add a runtime dependency to
//! Mirador and are started only when the layout actually places them.

pub mod agenda;
pub mod battery;
pub mod calculator;
pub mod calendar;
pub mod clocks;
pub mod cpu;
pub mod disk;
pub mod memory;
pub mod network;
pub mod news;
pub mod notes;
pub mod pomodoro;
pub mod stocks;
pub mod temperature;
pub mod todo;
pub mod watchlog;
pub mod weather;

use anyhow::Result;

use crate::config::Config;
use crate::panel::Panel;

/// Every widget id accepted in the `[layout]` table.
pub const WIDGET_NAMES: &[&str] = &[
    "clocks",
    "weather",
    "todo",
    "notes",
    "stocks",
    "calendar",
    "agenda",
    "pomodoro",
    "watchlog",
    "news",
    "cpu",
    "memory",
    "disk",
    "network",
    "battery",
    "temperature",
    "calculator",
];

/// A panel whose keys can be moved, under `[<widget>.keys]` in the config.
///
/// The shell reaches every such table through this list — to check it at
/// startup, reload or reset it from the key map, and list it there — so a
/// panel that moves its keys to a [`crate::keymap::PanelKeymap`] is added
/// here and nowhere else in the shell.
pub struct KeyScope {
    pub widget: &'static str,
    pub keys: fn(&Config) -> &crate::keymap::KeysConfig,
    pub keys_mut: fn(&mut Config) -> &mut crate::keymap::KeysConfig,
    /// Build the panel's keymap from a table and list it, or say why not.
    pub listing: fn(&crate::keymap::KeysConfig) -> Result<Vec<crate::keymap::Listed>, String>,
}

/// Every panel whose keys can be moved, in the order the key map lists them
/// — the order they appear in the shipped config. The calculator, battery and
/// network panels are not here: the calculator's keys are the digits and
/// operators you type, and the other two have no keys.
pub const KEY_SCOPES: &[KeyScope] = &[
    KeyScope {
        widget: "clocks",
        keys: |config| &config.clocks.keys,
        keys_mut: |config| &mut config.clocks.keys,
        listing: |keys| clocks::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "weather",
        keys: |config| &config.weather.keys,
        keys_mut: |config| &mut config.weather.keys,
        listing: |keys| weather::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "todo",
        keys: |config| &config.todo.keys,
        keys_mut: |config| &mut config.todo.keys,
        listing: |keys| todo::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "notes",
        keys: |config| &config.notes.keys,
        keys_mut: |config| &mut config.notes.keys,
        listing: |keys| notes::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "stocks",
        keys: |config| &config.stocks.keys,
        keys_mut: |config| &mut config.stocks.keys,
        listing: |keys| stocks::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "calendar",
        keys: |config| &config.calendar.keys,
        keys_mut: |config| &mut config.calendar.keys,
        listing: |keys| calendar::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "agenda",
        keys: |config| &config.agenda.keys,
        keys_mut: |config| &mut config.agenda.keys,
        listing: |keys| agenda::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "news",
        keys: |config| &config.news.keys,
        keys_mut: |config| &mut config.news.keys,
        listing: |keys| news::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "watchlog",
        keys: |config| &config.watchlog.keys,
        keys_mut: |config| &mut config.watchlog.keys,
        listing: |keys| watchlog::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "pomodoro",
        keys: |config| &config.pomodoro.keys,
        keys_mut: |config| &mut config.pomodoro.keys,
        listing: |keys| pomodoro::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "cpu",
        keys: |config| &config.cpu.keys,
        keys_mut: |config| &mut config.cpu.keys,
        listing: |keys| cpu::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "memory",
        keys: |config| &config.memory.keys,
        keys_mut: |config| &mut config.memory.keys,
        listing: |keys| memory::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "disk",
        keys: |config| &config.disk.keys,
        keys_mut: |config| &mut config.disk.keys,
        listing: |keys| disk::keymap(keys).map(|map| map.listing()),
    },
    KeyScope {
        widget: "temperature",
        keys: |config| &config.temperature.keys,
        keys_mut: |config| &mut config.temperature.keys,
        listing: |keys| temperature::keymap(keys).map(|map| map.listing()),
    },
];

/// Whether `name` refers to a widget mirador knows how to build.
pub fn is_known_widget(name: &str) -> bool {
    WIDGET_NAMES.contains(&name)
}

/// Construct a panel by widget id.
///
/// Returns `Ok(None)` for an unknown name; the config validator rejects those
/// earlier with a better message, so this is only a defensive fallback.
pub fn build(name: &str, config: &Config) -> Result<Option<Box<dyn Panel>>> {
    let mut panel: Box<dyn Panel> = match name {
        "clocks" => Box::new(clocks::ClocksPanel::new(
            config.clocks.clone(),
            crate::config::Config::zones_path()?,
        )?),
        "weather" => {
            #[cfg(test)]
            refuse_a_fetch_thread(name)?;
            Box::new(weather::WeatherPanel::new(config.weather.clone()))
        }
        "todo" => Box::new(todo::TodoPanel::new(
            config.todo.clone(),
            config.todo_path()?,
        )?),
        "notes" => Box::new(notes::NotesPanel::new(
            config.notes.clone(),
            config.notes_path()?,
        )?),
        "stocks" => {
            #[cfg(test)]
            refuse_a_fetch_thread(name)?;
            Box::new(stocks::StocksPanel::new(
                config.stocks.clone(),
                config.stocks_path()?,
            )?)
        }
        "calendar" => Box::new(calendar::CalendarPanel::new(config.calendar.clone())),
        "watchlog" => Box::new(watchlog::WatchLogPanel::new()),
        "news" => {
            #[cfg(test)]
            refuse_a_fetch_thread(name)?;
            Box::new(news::NewsPanel::new(&config.news))
        }
        "agenda" => Box::new(agenda::AgendaPanel::new(
            &config.agenda,
            config.agenda_path()?,
        )),
        "pomodoro" => Box::new(pomodoro::PomodoroPanel::new(config.pomodoro.clone())),
        "cpu" => Box::new(cpu::CpuPanel::new(config.cpu.clone())),
        "memory" => Box::new(memory::MemoryPanel::new(config.memory.clone())),
        "disk" => Box::new(disk::DiskPanel::new(config.disk.clone())),
        "network" => Box::new(network::NetworkPanel::new(config.network.clone())),
        "battery" => Box::new(battery::BatteryPanel::new(config.battery.clone())),
        "temperature" => Box::new(temperature::TemperaturePanel::new(
            config.temperature.clone(),
        )),
        "calculator" => Box::new(calculator::CalculatorPanel::new(config.calculator)),
        _ => {
            let Some(plugin) = config.plugin(name) else {
                return Ok(None);
            };
            Box::new(crate::plugin::PluginPanel::new(plugin.clone()))
        }
    };
    // Every panel starts on its default keys; this is where the config's
    // `[<widget>.keys]` reaches it, the same way a reload does.
    panel.set_keys(config);
    Ok(Some(panel))
}

/// Under test, the panels whose constructor starts a fetch thread are refused
/// rather than built, because the thread's first act is a request and no test
/// touches the network.
///
/// The refusal sits inside each panel's own arm of `build`, after the name has
/// matched, so a test that asks for one still proves the arm is there. A test
/// that wants these panels builds them offline, as the widget tests'
/// `offline_panels` does, and hands a whole dashboard of them to
/// `App::with_panels`.
#[cfg(test)]
fn refuse_a_fetch_thread(name: &str) -> Result<()> {
    anyhow::bail!(
        "`{name}` starts a fetch thread as it is built, and no test may reach the network; \
         build it offline instead"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_advertised_widget_is_recognised() {
        for name in WIDGET_NAMES {
            assert!(is_known_widget(name), "{name} is advertised but unknown");
        }
    }

    #[test]
    fn unknown_widgets_are_rejected() {
        assert!(!is_known_widget("nope"));
        assert!(!is_known_widget(""));
        assert!(!is_known_widget("CLOCKS"), "matching must be exact");
    }

    /// The widgets whose constructor starts a fetch thread, and that `build`
    /// therefore refuses under test: the thread's first act is a request.
    const FETCHING: [&str; 3] = ["weather", "stocks", "news"];

    /// No test reads or writes the reader's own data directory.
    ///
    /// Every data file a config leaves unset — the task list, notes,
    /// watchlist and calendar — falls back to the platform data directory,
    /// and the zone list and the state file live there whatever the config
    /// says. The suite built dashboards from `Config::default()`, so it read
    /// the reader's own task list, and on a machine without the others it
    /// seeded their notes, watchlist and zones: from the default config
    /// rather than theirs, and a seed is read only while its file is
    /// missing, so their own `[clocks].zones` would never have been.
    #[test]
    fn no_test_reaches_the_readers_data_directory() {
        let Some(theirs) = dirs::data_dir() else {
            // No data directory on this platform, so none to reach.
            return;
        };
        let config = Config::default();
        let mut paths = vec![
            config.todo_path().unwrap(),
            config.notes_path().unwrap(),
            config.stocks_path().unwrap(),
            config.agenda_path().unwrap(),
            Config::zones_path().unwrap(),
            Config::state_path().unwrap(),
            Config::update_cache_path().unwrap(),
        ];
        paths.extend(Config::owned_data_files().unwrap());
        for path in &paths {
            assert!(
                !path.starts_with(&theirs),
                "a test would reach {}, in the reader's own data directory",
                path.display()
            );
        }
    }

    /// Render every widget at a range of sizes, including degenerate ones.
    ///
    /// Layout code is the usual source of index-out-of-bounds panics in a TUI,
    /// and those only show up at sizes nobody tries by hand. A terminal one
    /// column wide is not a supported way to use mirador, but it must not
    /// crash: users resize windows, and tiling window managers do it for them.
    ///
    /// `build` is still asked for every advertised widget, so a name it has
    /// no arm for fails here. The three in `FETCHING` must be refused rather
    /// than built — this test used to build them, and the weather and news
    /// panels' threads went straight out to Open-Meteo and three RSS feeds on
    /// every run — so they are swept as `offline_panels` builds them, along
    /// with every other widget in that form.
    #[test]
    fn every_widget_renders_at_any_size_without_panicking() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let dir = std::env::temp_dir().join(format!("mirador-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut config = Config::default();
        config.todo.file = Some(dir.join("todos.toml"));

        let mut panels = Vec::new();
        for name in WIDGET_NAMES {
            match (FETCHING.contains(name), build(name, &config)) {
                (_, Ok(None)) => panic!("`{name}` is advertised but did not build"),
                (true, Ok(Some(_))) => panic!(
                    "`{name}` was built in a test, and the fetch thread it starts goes \
                     straight to the network"
                ),
                (true, Err(e)) => assert!(
                    e.to_string().contains("no test may reach the network"),
                    "building `{name}` failed: {e:#}"
                ),
                (false, Ok(Some(panel))) => panels.push((*name, panel)),
                (false, Err(e)) => panic!("building `{name}` failed: {e:#}"),
            }
        }
        panels.extend(offline_panels(&dir, &config));

        let gradients = config.theme.gradients();
        for (name, mut panel) in panels {
            panel.tick();

            for (width, height) in [(1, 1), (2, 3), (10, 4), (40, 12), (200, 60)] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        panel.render(
                            frame,
                            area,
                            crate::panel::RenderContext {
                                theme: &config.theme,
                                gradients: &gradients,
                                focused: true,
                                watch: &crate::watch::WatchLog::default(),
                            },
                        );
                    })
                    .unwrap_or_else(|e| panic!("`{name}` failed to draw at {width}x{height}: {e}"));
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same check for the whole dashboard, so the grid maths is covered too.
    ///
    /// The default layout places all three panels `build` refuses under test,
    /// so the dashboard is handed the offline ones, and fails to build if the
    /// layout names a widget the list does not supply.
    #[test]
    fn the_full_dashboard_renders_at_any_size_without_panicking() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let dir = std::env::temp_dir().join(format!("mirador-full-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let config = Config::default();
        let panels = offline_panels(&dir, &config);
        let mut app = crate::app::App::with_panels(config, panels).unwrap();

        for (width, height) in [(1, 1), (3, 2), (20, 5), (80, 24), (250, 80)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| app.render_for_test(frame))
                .unwrap_or_else(|e| panic!("dashboard failed to draw at {width}x{height}: {e}"));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A calendar with events tomorrow and three headlines, written and built
    /// for the sweep so the agenda and news panels have something to measure.
    fn sample_data(dir: &std::path::Path) -> (std::path::PathBuf, Vec<crate::feed::Story>) {
        let today = jiff::Zoned::now().date();
        let tomorrow = today.tomorrow().unwrap_or(today);
        let ics = dir.join("sample.ics");
        std::fs::write(
            &ics,
            format!(
                "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nDTSTART:{d}T091500\nDTEND:{d}T093000\n\
                 SUMMARY:Standup\nLOCATION:Zoom\nEND:VEVENT\nBEGIN:VEVENT\nDTSTART;VALUE=DATE:{d}\n\
                 SUMMARY:Quarterly planning day\nEND:VEVENT\nEND:VCALENDAR\n",
                d = tomorrow.strftime("%Y%m%d")
            ),
        )
        .unwrap();
        let story = |source: &str, title: &str| crate::feed::Story {
            source: source.to_string(),
            title: title.to_string(),
            link: "https://example.test/story".to_string(),
            published: None,
        };
        let stories = vec![
            story(
                "NASA",
                "Anak Krakatau rumbles again as satellites watch the plume",
            ),
            story(
                "PHYS.ORG",
                "Some black holes grow much faster than their galaxies",
            ),
            story(
                "ARS TECHNICA",
                "A very long headline that has to wrap on any panel narrower than it",
            ),
        ];

        (ics, stories)
    }

    /// Every panel, built with nothing behind it that leaves the process, and
    /// with enough on screen to be worth measuring: seeded tasks and notes, a
    /// canned watchlist and reading, three headlines, a calendar with events
    /// tomorrow.
    ///
    /// Named alongside `WIDGET_NAMES` and checked against it, so a new widget
    /// cannot be left out of the sweep without this failing to compile the
    /// list it expects.
    fn offline_panels(
        dir: &std::path::Path,
        config: &Config,
    ) -> Vec<(&'static str, Box<dyn Panel>)> {
        let (ics, stories) = sample_data(dir);

        let panels: Vec<(&'static str, Box<dyn Panel>)> = vec![
            (
                "clocks",
                Box::new(
                    clocks::ClocksPanel::new(config.clocks.clone(), dir.join("zones.toml"))
                        .unwrap(),
                ),
            ),
            (
                "weather",
                Box::new(weather::WeatherPanel::offline(config.weather.clone())),
            ),
            (
                "todo",
                Box::new(
                    todo::TodoPanel::new(config.todo.clone(), dir.join("todos.toml")).unwrap(),
                ),
            ),
            (
                "notes",
                Box::new(
                    notes::NotesPanel::new(config.notes.clone(), dir.join("notes.toml")).unwrap(),
                ),
            ),
            (
                "stocks",
                Box::new(
                    stocks::StocksPanel::offline(config.stocks.clone(), dir.join("watchlist.toml"))
                        .unwrap(),
                ),
            ),
            (
                "calendar",
                Box::new(calendar::CalendarPanel::new(config.calendar.clone())),
            ),
            (
                "agenda",
                Box::new(agenda::AgendaPanel::new(&config.agenda, ics)),
            ),
            (
                "pomodoro",
                Box::new(pomodoro::PomodoroPanel::new(config.pomodoro.clone())),
            ),
            ("watchlog", Box::new(watchlog::WatchLogPanel::new())),
            (
                "news",
                Box::new(news::NewsPanel::offline(&config.news, stories)),
            ),
            ("cpu", Box::new(cpu::CpuPanel::new(config.cpu.clone()))),
            (
                "memory",
                Box::new(memory::MemoryPanel::with_reading(
                    config.memory.clone(),
                    6_657_199_308,
                    17_179_869_184,
                )),
            ),
            (
                "disk",
                Box::new(disk::DiskPanel::canned(config.disk.clone())),
            ),
            (
                "network",
                Box::new(network::NetworkPanel::new(config.network.clone())),
            ),
            (
                "battery",
                Box::new(battery::BatteryPanel::canned(config.battery.clone())),
            ),
            (
                "temperature",
                Box::new(temperature::TemperaturePanel::canned(
                    config.temperature.clone(),
                )),
            ),
            (
                "calculator",
                Box::new(calculator::CalculatorPanel::new(config.calculator)),
            ),
        ];
        let listed: Vec<&str> = panels.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            listed, WIDGET_NAMES,
            "the offline sweep must build every widget, in the same order"
        );
        panels
    }

    /// Give the offline panels' threads — the canned quote source and the
    /// calendar read — a moment to answer, and let `tick` collect it.
    fn settle(panels: &mut [(&'static str, Box<dyn Panel>)]) {
        for _ in 0..4 {
            std::thread::sleep(std::time::Duration::from_millis(25));
            for (_, panel) in panels.iter_mut() {
                panel.tick();
            }
        }
    }

    /// The one rendering fault a single-width render cannot see, found by
    /// rendering two.
    ///
    /// A `Paragraph` handed a rect narrower than its text is cut by the
    /// terminal, which leaves no mark — invariant 19's failure, and the clock's
    /// date line had it for months at any width under 22. A buffer records
    /// what the terminal kept, so the cut is invisible at that width alone.
    /// It is not invisible across two: if a row at width W is exactly the first
    /// W cells of the same row at W+1, and W+1 has something in cell W, then
    /// content that would have extended past the edge was dropped without an
    /// ellipsis. Two things that look the same are not cuts and are excused —
    /// a whole value dropped at a space, which the grid does on purpose, and a
    /// word that reappears at the head of the next row, which was wrapped.
    ///
    /// Pointed at the clock before its date was fixed, this reported
    /// `THURSDAY 10 SEPTEMBE` at width 20; the day it was written it found the
    /// small seconds cut to one digit at 40. Widths start where a panel can
    /// hold a word at all.
    #[test]
    fn no_panel_cuts_a_value_silently_at_any_width() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let dir = std::env::temp_dir().join(format!("mirador-clip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = Config::default();
        let gradients = config.theme.gradients();
        let height = 14u16;

        let cells = |panel: &mut Box<dyn Panel>, width: u16| -> Vec<Vec<String>> {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    panel.render(
                        frame,
                        area,
                        crate::panel::RenderContext {
                            theme: &config.theme,
                            gradients: &gradients,
                            focused: true,
                            watch: &crate::watch::WatchLog::default(),
                        },
                    );
                })
                .unwrap();
            let buf = terminal.backend().buffer().clone();
            (0..height)
                .map(|y| {
                    (0..width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect()
                })
                .collect()
        };

        let mut findings = Vec::new();
        for (name, mut panel) in offline_panels(&dir, &config) {
            // The canned quote source and the calendar read both answer on a
            // thread; give them a moment and let `tick` collect the result.
            for _ in 0..4 {
                std::thread::sleep(std::time::Duration::from_millis(25));
                panel.tick();
            }
            let mut prev = cells(&mut panel, 6);
            for width in 7..=100u16 {
                let cur = cells(&mut panel, width);
                let w = usize::from(width) - 1;
                for (y, (narrow, wide)) in prev.iter().zip(cur.iter()).enumerate() {
                    let narrow_text: String = narrow.concat();
                    if narrow_text.trim().is_empty() {
                        continue;
                    }
                    let overflowed = wide[w].trim() != "";
                    let same_prefix = narrow[..] == wide[..w];
                    let marked = narrow_text.trim_end().ends_with('\u{2026}');
                    // A rule, a track or a meter fills whatever width it gets;
                    // being a prefix of itself is not a cut.
                    let distinct = narrow
                        .iter()
                        .filter(|c| c.trim() != "")
                        .collect::<std::collections::BTreeSet<_>>()
                        .len();
                    // A cut through a word or a number, as opposed to a whole
                    // value dropped at a space.
                    let mid_token = narrow[w - 1].trim() != "";
                    // A labelled rule — `NEXT HOURS ────` — grows by one more
                    // of the same glyph at each width. The same glyph carrying
                    // on past the edge is a fill, not a lost character; a
                    // letter or digit repeating (`SEPTEMBE` into `M`) is not.
                    let fill_run =
                        wide[w] == narrow[w - 1] && !wide[w].chars().all(char::is_alphanumeric);
                    // The cut character turning up at the head of the next row
                    // is a wrap, not a loss.
                    let wrapped = prev.get(y + 1).is_some_and(|next| {
                        next.concat().trim_start().starts_with(wide[w].as_str())
                    });
                    if overflowed
                        && same_prefix
                        && !marked
                        && distinct > 1
                        && mid_token
                        && !wrapped
                        && !fill_run
                    {
                        findings.push(format!(
                            "{name} at width {w} row {y}: {:?} continues as {:?}",
                            narrow_text.trim_end(),
                            wide[w]
                        ));
                    }
                }
                prev = cur;
            }
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            findings.is_empty(),
            "{} silent cut(s) — text lost past the edge with no ellipsis:\n  {}",
            findings.len(),
            findings.join("\n  ")
        );
    }

    /// Not a test: renders every panel across a width sweep so clipping can be
    /// seen rather than reasoned about, each built offline as the silent-cut
    /// sweep builds them.
    /// Run with `cargo test dump_width_sweep -- --ignored --nocapture`.
    #[test]
    #[ignore = "renders every panel at many widths for eyeballing, not an assertion"]
    fn dump_width_sweep() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let dir = std::env::temp_dir().join(format!("mirador-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let config = Config::default();
        let gradients = config.theme.gradients();
        let mut panels = offline_panels(&dir, &config);
        settle(&mut panels);

        for (name, mut panel) in panels {
            for width in [12u16, 16, 20, 24, 26, 30, 40] {
                let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
                terminal
                    .draw(|frame| {
                        let area = frame.area();
                        panel.render(
                            frame,
                            area,
                            crate::panel::RenderContext {
                                theme: &config.theme,
                                gradients: &gradients,
                                focused: true,
                                watch: &crate::watch::WatchLog::default(),
                            },
                        );
                    })
                    .unwrap();
                let buf = terminal.backend().buffer().clone();
                println!("\n=== {name} @ {width} ===");
                for y in 0..buf.area.height {
                    let mut line = String::new();
                    for x in 0..buf.area.width {
                        line.push_str(buf[(x, y)].symbol());
                    }
                    if !line.trim().is_empty() {
                        println!("|{line}|");
                    }
                }
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Not a test: renders what a brand-new user sees, with nothing on disk,
    /// so the seeded examples can be eyeballed the way they will be met.
    /// Run with `cargo test dump_first_run -- --ignored --nocapture`.
    ///
    /// Worth looking at after touching the seeds: they are the first and for
    /// some users the only impression of the task and notes panels, and the
    /// wording has to fit the columns at an ordinary terminal size.
    #[test]
    #[ignore = "renders the first-run dashboard to stdout for eyeballing, not an assertion"]
    fn dump_first_run() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        // An empty directory is the whole point: the stores seed themselves.
        let dir = std::env::temp_dir().join("mirador-dump-first-run");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let config = Config::default();
        let mut panels = offline_panels(&dir, &config);
        // A first run has no calendar, where the offline list has a sample.
        for (name, panel) in &mut panels {
            if *name == "agenda" {
                *panel = Box::new(agenda::AgendaPanel::new(
                    &config.agenda,
                    dir.join("calendar.ics"),
                ));
            }
        }
        // The canned watchlist is staggered like a live one; let it fill.
        std::thread::sleep(std::time::Duration::from_secs(3));
        settle(&mut panels);
        let mut app = crate::app::App::with_panels(config, panels).unwrap();

        let mut terminal = Terminal::new(TestBackend::new(100, 34)).unwrap();
        terminal.draw(|f| app.render_for_test(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        println!("\n+{}+", "-".repeat(100));
        for y in 0..buf.area.height {
            let mut line = String::new();
            for x in 0..buf.area.width {
                line.push_str(buf[(x, y)].symbol());
            }
            println!("|{line}|");
        }
        println!("+{}+", "-".repeat(100));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Not a test: renders the dashboard to text so it can be eyeballed.
    /// Run with `cargo test dump_dashboard -- --ignored --nocapture`.
    #[test]
    #[ignore = "renders the dashboard to stdout for eyeballing, not an assertion"]
    fn dump_dashboard() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let dir = std::env::temp_dir().join("mirador-dump");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todos.toml");
        std::fs::write(
            &todo_path,
            r#"
[[task]]
id = 1
title = "Renew the domain"
notes = "Expires soon"
due = "2026-07-23"
priority = "high"
tags = ["admin"]
done = false
created = "2026-07-01"

[[task]]
id = 2
title = "Reply to the design review"
priority = "medium"
due = "2026-07-25"
tags = ["work"]
done = false
created = "2026-07-20"

[[task]]
id = 3
title = "Publish 0.0.0 placeholder"
notes = "Reserve the crates.io name before someone else does"
due = "2026-07-28"
priority = "low"
tags = ["mirador", "rust"]
done = false
created = "2026-07-25"
"#,
        )
        .unwrap();

        // `offline_panels` builds the task list from the file just written.
        let config = Config::default();
        let mut panels = offline_panels(&dir, &config);
        // The canned watchlist is staggered like a live one; let it fill.
        std::thread::sleep(std::time::Duration::from_secs(3));
        settle(&mut panels);
        let mut app = crate::app::App::with_panels(config, panels).unwrap();

        let mut terminal = Terminal::new(TestBackend::new(100, 34)).unwrap();
        terminal.draw(|f| app.render_for_test(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        println!("\n+{}+", "-".repeat(100));
        for y in 0..buf.area.height {
            let mut line = String::new();
            for x in 0..buf.area.width {
                line.push_str(buf[(x, y)].symbol());
            }
            println!("|{line}|");
        }
        println!("+{}+", "-".repeat(100));
    }

    #[test]
    fn the_default_layout_only_references_real_widgets() {
        let config = Config::default();
        for row in &config.layout.rows {
            for panel in &row.panels {
                assert!(
                    is_known_widget(&panel.widget),
                    "default layout references unknown widget `{}`",
                    panel.widget
                );
            }
        }
    }
}
