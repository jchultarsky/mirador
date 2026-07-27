//! Built-in widgets and the registry that turns a config name into a panel.
//!
//! Adding a widget means implementing [`crate::panel::Panel`], adding its name
//! to [`WIDGET_NAMES`] and adding one arm to [`build`]. Nothing else in the
//! codebase needs to know it exists.

pub mod agenda;
pub mod calendar;
pub mod clocks;
pub mod cpu;
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
    "clocks", "weather", "todo", "notes", "stocks", "calendar", "agenda", "pomodoro", "watchlog",
    "news", "cpu", "network",
];

/// Whether `name` refers to a widget mirador knows how to build.
pub fn is_known_widget(name: &str) -> bool {
    WIDGET_NAMES.contains(&name)
}

/// Widgets mirador can build that this layout never places, in declaration
/// order.
///
/// A widget missing from a layout is a legitimate choice, not an error, so this
/// can only ever drive a hint — never a warning and never a default. It exists
/// because a config written by an earlier version silently lacks every widget
/// added since, and there is otherwise no way to find out except reading the
/// release notes.
pub fn unused_widgets(config: &Config) -> Vec<&'static str> {
    let placed: Vec<&str> = config
        .layout
        .rows
        .iter()
        .flat_map(|row| row.panels.iter())
        .map(|panel| panel.widget.as_str())
        .collect();

    WIDGET_NAMES
        .iter()
        .copied()
        .filter(|name| !placed.contains(name))
        .collect()
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
        "network" => Box::new(network::NetworkPanel::new(config.network.clone())),
        _ => return Ok(None),
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
