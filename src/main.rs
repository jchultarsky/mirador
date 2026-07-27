//! mirador — a personal information dashboard for your terminal.
//!
//! See <https://github.com/jchultarsky/mirador> for documentation.

mod app;
mod chart;
mod config;
mod dateinput;
mod frame;
mod glyphs;
mod grid;
mod ical;
mod layout_edit;
mod migrate;
mod note;
mod panel;
mod picker;
mod poll;
mod quote;
mod samples;
mod selection;
mod state;
mod store;
mod task;
mod textarea;
mod textfield;
mod theme;
mod update;
mod widgets;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;

use crate::app::App;
use crate::config::Config;

const HELP: &str = "\
mirador — a personal information dashboard for your terminal

USAGE:
    mirador [OPTIONS]

OPTIONS:
    -c, --config <PATH>    Use a specific config file
        --print-config     Print the default config to stdout and exit
        --config-path      Print the resolved config path and exit
        --migrate-config   Update a config written by an older version
    -h, --help             Print this help and exit
    -V, --version          Print the version and exit

KEYS:
    Tab / Shift+Tab        Move focus between panels
    1 - 9                  Jump straight to a panel
    ?                      Show all key bindings
    q / Ctrl+C             Quit

On first run mirador writes a commented config file you can edit. Run
`mirador --config-path` to find it.
";

/// Parsed command line.
// A flags struct is exactly the case where several bools are the clearest
// representation; there is no state machine hiding here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default)]
struct Args {
    config: Option<PathBuf>,
    print_config: bool,
    show_config_path: bool,
    migrate_config: bool,
    help: bool,
    version: bool,
}

/// Parse arguments without pulling in a CLI framework for four flags.
fn parse_args(raw: impl Iterator<Item = String>) -> Result<Args> {
    let mut args = Args::default();
    let mut iter = raw.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => args.help = true,
            "-V" | "--version" => args.version = true,
            "--print-config" => args.print_config = true,
            "--config-path" => args.show_config_path = true,
            "--migrate-config" => args.migrate_config = true,
            "-c" | "--config" => {
                let value = iter.next().ok_or_else(|| {
                    anyhow::anyhow!("`{arg}` needs a path, e.g. `{arg} ~/mirador.toml`")
                })?;
                args.config = Some(PathBuf::from(value));
            }
            other if other.starts_with("--config=") => {
                args.config = Some(PathBuf::from(&other["--config=".len()..]));
            }
            other => {
                anyhow::bail!("unrecognised argument `{other}`. Run `mirador --help` for usage.")
            }
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // `{:#}` renders the whole anyhow context chain on one line.
            eprintln!("mirador: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args = parse_args(std::env::args().skip(1))?;

    if args.help {
        print!("{HELP}");
        return Ok(());
    }
    if args.version {
        println!("mirador {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.print_config {
        print!("{}", config::DEFAULT_CONFIG);
        return Ok(());
    }
    if args.show_config_path {
        let path = match args.config {
            Some(p) => p,
            None => Config::default_path()?,
        };
        println!("{}", path.display());
        return Ok(());
    }

    if args.migrate_config {
        let path = match args.config {
            Some(p) => p,
            None => Config::default_path()?,
        };
        let report = migrate::migrate_file(&path)?;
        if report.is_empty() {
            println!("{} is already current; nothing to do.", path.display());
        } else {
            println!("Updated {}:", path.display());
            for change in &report.changes {
                println!("  - {change}");
            }
            if let Some(backup) = &report.backup {
                println!("\nYour original is at {}.", backup.display());
            }
        }
        return Ok(());
    }

    let (mut config, config_path) = Config::load(args.config)?;

    // Preferences changed from the keyboard on a previous run are applied over
    // the config *before* any panel exists, so every panel is constructed with
    // the values the user last chose and none of them needs restoring code.
    // The config still seeds; this only records where things moved since.
    let state_path = Config::state_path().ok();
    let saved = state_path
        .as_deref()
        .map(crate::state::UiState::load)
        .unwrap_or_default();
    // Taken before `apply_state`, so it is what the *file* says rather than what
    // the file plus last session's changes say. Everything written later is the
    // difference from this, which is what makes a preference retractable.
    let baseline = crate::state::UiState::from_config(&config);
    config.apply_state(&saved);

    let mouse = config.general.mouse;
    // Started before the terminal is taken over, and off unless the config says
    // otherwise. Returns immediately either way — the request, if there is one,
    // is on its own thread and the dashboard never waits for it.
    let updates = crate::update::spawn(
        config.general.check_for_updates,
        Config::update_cache_path().ok(),
    );
    let mut app = App::new(config)?;
    app.watch_for_updates(updates);
    app.write_layout_to(config_path);
    if let Some(path) = state_path {
        app.remember_preferences_at(path, saved, baseline);
    }

    // `ratatui::init` installs a panic hook that restores the terminal, so a
    // panic leaves the user with a working shell rather than a broken one.
    let mut terminal = ratatui::init();

    if mouse {
        // ratatui's hook knows nothing about mouse capture, and a terminal
        // left holding the mouse turns every later click in the user's shell
        // into escape-sequence garbage. Chain a hook that releases it first.
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(std::io::stdout(), DisableMouseCapture);
            previous(info);
        }));
        execute!(std::io::stdout(), EnableMouseCapture).context("enabling mouse reporting")?;
    }

    let result = app.run(&mut terminal);

    if mouse {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
    }
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args> {
        parse_args(args.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn no_arguments_is_valid() {
        let args = parse(&[]).unwrap();
        assert!(args.config.is_none());
        assert!(!args.help);
    }

    #[test]
    fn flags_parse_in_both_short_and_long_form() {
        assert!(parse(&["-h"]).unwrap().help);
        assert!(parse(&["--help"]).unwrap().help);
        assert!(parse(&["-V"]).unwrap().version);
        assert!(parse(&["--version"]).unwrap().version);
        assert!(parse(&["--print-config"]).unwrap().print_config);
        assert!(parse(&["--config-path"]).unwrap().show_config_path);
        assert!(parse(&["--migrate-config"]).unwrap().migrate_config);
    }

    #[test]
    fn config_accepts_separate_and_joined_values() {
        assert_eq!(
            parse(&["-c", "/tmp/a.toml"]).unwrap().config,
            Some(PathBuf::from("/tmp/a.toml"))
        );
        assert_eq!(
            parse(&["--config", "/tmp/b.toml"]).unwrap().config,
            Some(PathBuf::from("/tmp/b.toml"))
        );
        assert_eq!(
            parse(&["--config=/tmp/c.toml"]).unwrap().config,
            Some(PathBuf::from("/tmp/c.toml"))
        );
    }

    #[test]
    fn a_config_flag_without_a_value_explains_itself() {
        let err = parse(&["--config"]).expect_err("must fail");
        assert!(err.to_string().contains("needs a path"), "got: {err}");
    }

    #[test]
    fn unknown_arguments_point_at_help() {
        let err = parse(&["--wat"]).expect_err("must fail");
        assert!(err.to_string().contains("--help"), "got: {err}");
    }

    #[test]
    fn the_default_config_is_valid_toml_and_parses_into_config() {
        let parsed: Config =
            toml::from_str(config::DEFAULT_CONFIG).expect("bundled config must parse");
        assert!(!parsed.layout.rows.is_empty());
    }

    #[test]
    fn help_text_documents_every_flag_the_parser_accepts() {
        for flag in [
            "--config",
            "--print-config",
            "--config-path",
            "--migrate-config",
            "--help",
            "--version",
        ] {
            assert!(HELP.contains(flag), "{flag} is undocumented");
        }
    }
}
