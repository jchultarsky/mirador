//! mirador — a personal information dashboard for your terminal.
//!
//! See <https://github.com/jchultarsky/mirador> for documentation.

mod app;
mod arrange;
mod calc;
mod chart;
mod clipboard;
mod config;
mod dateinput;
/// Working-notes checks; test builds only, so nothing ships with it.
#[cfg(test)]
mod docs;
mod feed;
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
mod prompt;
mod quote;
mod samples;
mod selection;
mod state;
mod store;
mod task;
mod textarea;
mod textfield;
mod theme;
mod theme_picker;
mod themes;
mod update;
mod upgrade;
mod watch;
mod widgets;
mod zones;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{
    DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
};
use ratatui::crossterm::execute;

use crate::app::App;
use crate::config::Config;

const HELP: &str = concat!(
    "mirador ",
    env!("CARGO_PKG_VERSION"),
    " — a personal information dashboard for your terminal

USAGE:
    mirador [OPTIONS]

OPTIONS:
    -c, --config <PATH>    Use a specific config file
        --print-config     Print the default config to stdout and exit
        --config-path      Print the resolved config path and exit
        --migrate-config   Update a config written by an older version
        --reset-config     Replace the config with the defaults, keeping a copy
        --factory-reset    Start over: config, preferences, tasks, notes and
                           watchlist all set aside, nothing deleted
        --update           Update through the installer or Cargo and exit
    -y, --yes              Do not ask for confirmation
    -h, --help             Print this help and exit
    -V, --version          Print the version and exit

KEYS:
    Tab / Shift+Tab        Move focus between panels
    1 - 9                  Jump straight to a panel
    w                      Choose which panels are shown
    m                      Rearrange the panels
    t                      Choose a theme
    Ctrl+arrows            Resize the focused panel
    ?                      Show all key bindings, and the version
    q / Ctrl+C             Quit

On first run mirador writes a commented config file you can edit. Run
`mirador --config-path` to find it.
"
);

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
    reset_config: bool,
    factory_reset: bool,
    update: bool,
    /// Skip the confirmation `--reset-config` would otherwise ask for.
    assume_yes: bool,
    help: bool,
    version: bool,
}

/// Parse arguments without pulling in a CLI framework for this small flag set.
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
            "--reset-config" => args.reset_config = true,
            "--factory-reset" => args.factory_reset = true,
            "--update" => args.update = true,
            "-y" | "--yes" => args.assume_yes = true,
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
    upgrade::cleanup_stale();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // `{:#}` renders the whole anyhow context chain on one line.
            eprintln!("mirador: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// `--reset-config`: replace the config with the defaults, keeping the old one.
///
/// The confirmation is the point of this function rather than an afterthought.
/// The name sounds harmless and the effect is not — `[layout]` is the part
/// people curate, and someone reaching for this because mirador will not start
/// may have one salvageable typo and an evening's arrangement.
///
/// It refuses rather than assuming when it cannot ask: piped into a script with
/// no `--yes`, "no answer" must not read as "go ahead". A prompt written to a
/// terminal nobody is watching is the same silent overwrite with extra steps.
fn reset_config(path: &Path, assume_yes: bool) -> Result<()> {
    let exists = path.try_exists().unwrap_or(false);

    if !assume_yes {
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "refusing to reset {} without a confirmation.\n\nThere is no \
                 terminal to ask on, so re-run with `--yes` if you mean it.",
                path.display()
            );
        }
        if exists {
            println!("This replaces {} with the defaults.", path.display());
            println!("Your current config will be copied alongside it first.");
        } else {
            println!("There is no config at {}.", path.display());
            println!("This writes the defaults there.");
        }
        // Say the second half out loud. Resetting the config without this
        // leaves the dashboard looking untouched (#153), so a reader who is
        // told only about the config is being told half of what will happen.
        println!("Preferences you changed from the keyboard are put aside too.");
        println!("Your tasks, notes and watchlist are left alone.");
        print!("Go ahead? [y/N] ");
        std::io::stdout().flush().context("writing the prompt")?;

        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .context("reading your answer")?;
        // Anything but an explicit yes is a no, including an empty line. The
        // default has to be the outcome that loses nothing.
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Left {} alone.", path.display());
            return Ok(());
        }
    }

    let backup = Config::reset(path)?;
    println!("Wrote the default config to {}.", path.display());
    if let Some(backup) = backup {
        println!("Your previous config is at {}.", backup.display());
    }

    // The config is only half of what decides the dashboard. `state.toml`
    // records what was changed from the keyboard and *outranks* the config, so
    // leaving it would apply the old preferences straight back over the file
    // just restored — the reason #153 looked like the command doing nothing.
    match Config::state_path() {
        Ok(state) => match state::clear(&state)? {
            Some(moved) => println!(
                "Put your remembered preferences aside; they are at {}.",
                moved.display()
            ),
            None => println!("There were no remembered preferences to clear."),
        },
        // Worth a word rather than a silent skip: the config really was reset,
        // and the reader should know which half did not happen.
        Err(e) => println!("Could not find the preferences file to clear it: {e}"),
    }

    println!("Left your tasks, notes and watchlist alone.");
    Ok(())
}

/// Put mirador back where a fresh install would leave it.
///
/// The reset `--reset-config` deliberately is not. That one is about
/// configuration; this one is about everything mirador has written, because
/// the owner's expectation of a reset was "as if I just installed the app" and
/// `--reset-config` cannot honestly promise that — the watchlist, tasks and
/// notes all outlive it (#153).
///
/// **Nothing is deleted.** Every file is renamed to a numbered `.bak` beside
/// itself, so the dashboard starts over and the reader can still get their task
/// list back. That is what makes a single `y` an acceptable confirmation for a
/// command with this name: the prompt names every file, and the worst outcome
/// is an afternoon of renaming rather than lost work.
fn factory_reset(config_path: &Path, assume_yes: bool) -> Result<()> {
    let data_files = Config::owned_data_files().unwrap_or_default();
    let present: Vec<&PathBuf> = data_files
        .iter()
        .filter(|p| p.try_exists().unwrap_or(false))
        .collect();
    let config_exists = config_path.try_exists().unwrap_or(false);

    if !assume_yes {
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "refusing to reset everything without a confirmation.\n\nThere is \
                 no terminal to ask on, so re-run with `--yes` if you mean it."
            );
        }
        println!("This starts mirador over from scratch.");
        println!();
        if config_exists {
            println!("  Replaced with the defaults:");
            println!("    {}", config_path.display());
            println!();
        }
        if present.is_empty() {
            println!("  There is nothing else to set aside.");
        } else {
            println!("  Set aside, each kept as a `.bak` beside itself:");
            for path in &present {
                println!("    {}", path.display());
            }
        }
        println!();
        println!("Nothing is deleted. Your tasks and notes stay on disk under their");
        println!("backup names, and mirador starts again with fresh ones.");
        print!("Go ahead? [y/N] ");
        std::io::stdout().flush().context("writing the prompt")?;

        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .context("reading your answer")?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Left everything alone.");
            return Ok(());
        }
    }

    let backup = Config::reset(config_path)?;
    println!("Wrote the default config to {}.", config_path.display());
    if let Some(backup) = backup {
        println!("Your previous config is at {}.", backup.display());
    }

    let mut moved = 0usize;
    for path in data_files {
        if let Some(to) = store::move_aside(&path)? {
            println!("Set aside {} -> {}", path.display(), to.display());
            moved += 1;
        }
    }
    if moved == 0 {
        println!("There was nothing else to set aside.");
    }

    println!("Next launch starts fresh: example tasks and notes, and the");
    println!("watchlist seeded from `[stocks].symbols` in the new config.");
    Ok(())
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
    } else if args.update {
        return upgrade::run();
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

    if args.reset_config {
        let path = match args.config.clone() {
            Some(p) => p,
            None => Config::default_path()?,
        };
        return reset_config(&path, args.assume_yes);
    }

    if args.factory_reset {
        let path = match args.config.clone() {
            Some(p) => p,
            None => Config::default_path()?,
        };
        return factory_reset(&path, args.assume_yes);
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
    config.apply_state_theme(&saved, &config_path);

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

    // Asks the terminal to say when its window gains or loses focus, which is
    // the only signal there is for "the reader is actually here" — everything
    // else measures interaction, and a dashboard you glance at is precisely one
    // you do not touch. Failure is ignored on purpose: plenty of terminals do
    // not implement it, and the watch log falls back to the last keypress.
    let _ = execute!(std::io::stdout(), EnableFocusChange);

    // Released on the way out, and on a panic, for the same reason mouse
    // capture is: a terminal left reporting focus writes escape sequences into
    // the user's shell every time they switch windows.
    {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(std::io::stdout(), DisableFocusChange);
            previous(info);
        }));
    }

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
    let _ = execute!(std::io::stdout(), DisableFocusChange);
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
        assert!(parse(&["--reset-config"]).unwrap().reset_config);
        assert!(parse(&["--factory-reset"]).unwrap().factory_reset);
        assert!(parse(&["--update"]).unwrap().update);
        // The two are separate commands, not degrees of one: a config reset
        // must never quietly take the tasks with it.
        assert!(!parse(&["--reset-config"]).unwrap().factory_reset);
        assert!(!parse(&["--factory-reset"]).unwrap().reset_config);
        assert!(parse(&["-y"]).unwrap().assume_yes);
        assert!(parse(&["--yes"]).unwrap().assume_yes);
    }

    /// `--yes` only ever *removes* a question. On its own it must not be
    /// mistaken for a request to reset anything.
    #[test]
    fn yes_on_its_own_resets_nothing() {
        let args = parse(&["--yes"]).unwrap();
        assert!(args.assume_yes);
        assert!(!args.reset_config);
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
            "--reset-config",
            "--factory-reset",
            "--update",
            "--yes",
            "--help",
            "--version",
        ] {
            assert!(HELP.contains(flag), "{flag} is undocumented");
        }
    }
}
