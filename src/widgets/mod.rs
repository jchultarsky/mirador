//! Built-in widgets and the registry that turns a config name into a panel.
//!
//! Built-ins are compiled here. Explicit external declarations fall through
//! to the process-protocol adapter; they do not add a runtime dependency to
//! Mirador and are started only when the layout actually places them.

pub mod agenda;
pub mod calculator;
pub mod calendar;
pub mod clocks;
pub mod cpu;
pub mod memory;
pub mod network;
pub mod news;
pub mod notes;
pub mod pomodoro;
pub mod stocks;
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
    "network",
    "calculator",
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
    let panel: Box<dyn Panel> = match name {
        "clocks" => Box::new(clocks::ClocksPanel::new(
            config.clocks.clone(),
            crate::config::Config::zones_path()?,
        )?),
        "weather" => Box::new(weather::WeatherPanel::new(config.weather.clone())),
        "todo" => Box::new(todo::TodoPanel::new(
            config.todo.clone(),
            config.todo_path()?,
        )?),
        "notes" => Box::new(notes::NotesPanel::new(
            config.notes.clone(),
            config.notes_path()?,
        )?),
        "stocks" => Box::new(stocks::StocksPanel::new(
            config.stocks.clone(),
            config.stocks_path()?,
        )?),
        "calendar" => Box::new(calendar::CalendarPanel::new(config.calendar.clone())),
        "watchlog" => Box::new(watchlog::WatchLogPanel::new()),
        "news" => Box::new(news::NewsPanel::new(&config.news)),
        "agenda" => Box::new(agenda::AgendaPanel::new(
            &config.agenda,
            config.agenda_path()?,
        )),
        "pomodoro" => Box::new(pomodoro::PomodoroPanel::new(config.pomodoro.clone())),
        "cpu" => Box::new(cpu::CpuPanel::new(config.cpu.clone())),
        "memory" => Box::new(memory::MemoryPanel::new(config.memory.clone())),
        "network" => Box::new(network::NetworkPanel::new(config.network.clone())),
        "calculator" => Box::new(calculator::CalculatorPanel::new(config.calculator)),
        _ => {
            let Some(plugin) = config.plugin(name) else {
                return Ok(None);
            };
            Box::new(crate::plugin::PluginPanel::new(plugin.clone()))
        }
    };
    Ok(Some(panel))
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

    /// Render every widget at a range of sizes, including degenerate ones.
    ///
    /// Layout code is the usual source of index-out-of-bounds panics in a TUI,
    /// and those only show up at sizes nobody tries by hand. A terminal one
    /// column wide is not a supported way to use mirador, but it must not
    /// crash: users resize windows, and tiling window managers do it for them.
    #[test]
    fn every_widget_renders_at_any_size_without_panicking() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let dir = std::env::temp_dir().join(format!("mirador-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut config = Config::default();
        config.todo.file = Some(dir.join("todos.toml"));
        // Skip the network round trip: an explicit coordinate pair means the
        // weather panel never geocodes, so this test needs no network at all.
        config.weather.latitude = Some(42.36);
        config.weather.longitude = Some(-71.06);

        for name in WIDGET_NAMES {
            let mut panel = build(name, &config)
                .unwrap_or_else(|e| panic!("building `{name}` failed: {e:#}"))
                .unwrap_or_else(|| panic!("`{name}` is advertised but did not build"));

            panel.tick();
            let gradients = config.theme.gradients();

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
    #[test]
    fn the_full_dashboard_renders_at_any_size_without_panicking() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let dir = std::env::temp_dir().join(format!("mirador-full-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut config = Config::default();
        config.todo.file = Some(dir.join("todos.toml"));
        config.weather.latitude = Some(42.36);
        config.weather.longitude = Some(-71.06);

        let mut app = crate::app::App::new(config).unwrap();

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
                "network",
                Box::new(network::NetworkPanel::new(config.network.clone())),
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
    /// seen rather than reasoned about.
    /// Run with `cargo test dump_width_sweep -- --ignored --nocapture`.
    #[test]
    #[ignore = "renders every panel at many widths for eyeballing, not an assertion"]
    fn dump_width_sweep() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let dir = std::env::temp_dir().join(format!("mirador-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut config = Config::default();
        config.todo.file = Some(dir.join("todos.toml"));
        config.notes.file = Some(dir.join("notes.toml"));
        config.stocks.file = Some(dir.join("watchlist.toml"));
        config.weather.latitude = Some(42.36);
        config.weather.longitude = Some(-71.06);

        for name in WIDGET_NAMES {
            let mut panel = build(name, &config).unwrap().unwrap();
            panel.tick();
            let gradients = config.theme.gradients();

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

        let mut config = Config::default();
        config.todo.file = Some(dir.join("todos.toml"));
        config.notes.file = Some(dir.join("notes.toml"));
        config.stocks.file = Some(dir.join("watchlist.toml"));
        config.weather.latitude = Some(42.36);
        config.weather.longitude = Some(-71.06);

        let mut app = crate::app::App::new(config).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(3));

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

        let mut config = Config::default();
        config.todo.file = Some(todo_path);
        config.weather.latitude = Some(42.36);
        config.weather.longitude = Some(-71.06);

        let mut app = crate::app::App::new(config).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(3));

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
