//! Migrating a config file written by an older version.
//!
//! mirador writes its config once and never rewrites it, so a key renamed in a
//! later release sits on disk looking correct. The parser now rejects such a
//! key loudly (see [`crate::config`]), but rejecting is only half a fix — the
//! other half is doing something about it without making the user hand-edit
//! TOML.
//!
//! The migration is deliberately **textual** rather than a parse-and-reserialise
//! round trip. Round-tripping through `toml::Table` would silently discard every
//! comment in the file, including the ones mirador itself wrote to explain the
//! options. Rewriting the offending lines in place preserves the user's
//! formatting, their comments, and any ordering they chose.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// What a migration did, so it can be reported rather than done silently.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Human-readable description of each change, in file order.
    pub changes: Vec<String>,
    /// Where the original was saved.
    pub backup: Option<PathBuf>,
}

impl Report {
    /// Whether anything needed changing.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// A key that no longer exists, and what to do about it.
enum Rule {
    /// Rename the key, replacing its value with a fixed one because the old
    /// value does not carry over meaningfully.
    Replace {
        section: &'static str,
        from: &'static str,
        to: &'static str,
        value: &'static str,
        why: &'static str,
    },
    /// Comment the line out; the setting moved somewhere a rename cannot reach.
    Retire {
        section: &'static str,
        key: &'static str,
        why: &'static str,
    },
}

/// Every key removed since 0.1.0.
const RULES: &[Rule] = &[
    Rule::Replace {
        section: "weather",
        from: "forecast_days",
        to: "forecast_hours",
        // A day count does not convert to an hour count — four days of
        // forecast is not four hours of it — so the new default is used and
        // the change is reported rather than guessed at.
        value: "8",
        why: "the forecast is hourly now, so the day count was replaced with the default of 8 hours",
    },
    Rule::Replace {
        section: "notes",
        from: "side_by_side_min_width",
        to: "preview",
        // A width threshold does not convert to a placement: the setting now
        // says where the body goes, not how wide the panel must be to earn a
        // side-by-side split.
        value: "\"below\"",
        why: "the note body is placed by name now, not by a width threshold",
    },
    Rule::Retire {
        section: "theme",
        key: "rx",
        why: "replaced by the [theme.rx_gradient] table",
    },
    Rule::Retire {
        section: "theme",
        key: "tx",
        why: "replaced by the [theme.tx_gradient] table",
    },
];

/// The `[section]` a line opens, if it opens one.
///
/// Only top-level tables matter here; `[theme.rx_gradient]` is reported as
/// `theme.rx_gradient` and so will not match a rule scoped to `theme`, which
/// is what we want.
fn section_of(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    // Array-of-table headers wrap in a second pair of brackets, and both have
    // to come off together — stripping only the leading one leaves a stray
    // bracket that stops every section from matching.
    let inner = match inner.strip_prefix('[') {
        Some(rest) => rest.strip_suffix(']')?,
        None => inner,
    };
    Some(inner.trim())
}

/// The bare key a line assigns to, if it assigns to one.
fn key_of(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let (key, _) = trimmed.split_once('=')?;
    Some(key.trim().trim_matches('"'))
}

/// Rewrite `contents`, returning the new text and a description of each change.
pub fn migrate_text(contents: &str) -> (String, Vec<String>) {
    // Whatever the file already used. `str::lines()` strips `\r`, so pushing
    // `\n` would convert a CRLF config to LF and report every line as changed.
    let newline = crate::store::line_ending(contents);
    let mut out = String::with_capacity(contents.len());
    let mut changes = Vec::new();
    let mut section = String::new();

    for line in contents.lines() {
        if let Some(name) = section_of(line) {
            section = name.to_string();
            out.push_str(line);
            out.push_str(newline);
            continue;
        }

        let Some(key) = key_of(line) else {
            out.push_str(line);
            out.push_str(newline);
            continue;
        };

        let mut handled = false;
        for rule in RULES {
            match rule {
                Rule::Replace {
                    section: want,
                    from,
                    to,
                    value,
                    why,
                } if section == *want && key == *from => {
                    // Land `=` in the column the file already used, so a renamed
                    // key does not break the alignment of the block around it.
                    // Byte length is the column here: TOML bare keys are ASCII
                    // and the only thing before them is indent whitespace.
                    let indent = &line[..line.len() - line.trim_start().len()];
                    let eq_col = line.find('=').unwrap_or_default();
                    let pad = eq_col.saturating_sub(indent.len() + to.len()).max(1);
                    // Writing into a String is infallible.
                    // `write!` and an explicit ending, not `writeln!`, which
                    // hard-codes `\n` and would put an LF line into a CRLF file.
                    let _ = write!(
                        out,
                        "{indent}{to}{:pad$}= {value}  # migrated from {from}",
                        ""
                    );
                    out.push_str(newline);
                    changes.push(format!("[{want}] {from} -> {to}: {why}"));
                    handled = true;
                }
                Rule::Retire {
                    section: want,
                    key: k,
                    why,
                } if section == *want && key == *k => {
                    // Commented rather than deleted: the old value stays
                    // visible so the user can see what their colour was.
                    let _ = write!(out, "# {}  # removed: {why}", line.trim());
                    out.push_str(newline);
                    changes.push(format!("[{want}] {k} removed: {why}"));
                    handled = true;
                }
                _ => {}
            }
            if handled {
                break;
            }
        }

        if !handled {
            out.push_str(line);
            out.push_str(newline);
        }
    }

    (out, changes)
}

/// Migrate the config at `path` in place, backing up the original first.
///
/// Returns an empty report when the file already parses, so running this on a
/// current config is a no-op rather than a spurious rewrite.
pub fn migrate_file(path: &Path) -> Result<Report> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("reading config {}", path.display()))?;

    // Already valid: nothing to do. Checked first so a healthy config is never
    // rewritten and never gains a stray backup file.
    if toml::from_str::<crate::config::Config>(&contents).is_ok() {
        return Ok(Report::default());
    }

    let (migrated, changes) = migrate_text(&contents);
    if changes.is_empty() {
        anyhow::bail!(
            "the config at {} does not parse, and none of the problems are ones \
             this version knows how to migrate. Run `mirador --print-config` to \
             see the current format.",
            path.display()
        );
    }

    // Refuse to write something that still will not load. Better to leave the
    // user's file untouched and say so than to half-fix it.
    toml::from_str::<crate::config::Config>(&migrated).map_err(|e| {
        anyhow::anyhow!(
            "migrating {} did not produce a usable config: {e}\n\nThe original \
             file has been left untouched.",
            path.display()
        )
    })?;

    let backup = path.with_extension("toml.bak");
    std::fs::copy(path, &backup)
        .with_context(|| format!("backing up {} to {}", path.display(), backup.display()))?;
    // Atomic even though the backup exists: a truncated config with a `.bak`
    // beside it still means an editor session lost, and the user has to know to
    // look for the backup.
    crate::store::write_atomic(path, &migrated)
        .with_context(|| format!("writing migrated config to {}", path.display()))?;

    Ok(Report {
        changes,
        backup: Some(backup),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same hazard as `layout_edit`: `str::lines()` strips the `\r`, so a
    /// migration of a Windows config rewrote every line in it.
    #[test]
    fn a_crlf_config_stays_crlf() {
        let lf = "[theme]\nrx = \"green\"\n";
        let (out, changes) = migrate_text(&lf.replace('\n', "\r\n"));
        assert!(!changes.is_empty(), "something was migrated");
        assert_eq!(
            out.matches("\r\n").count(),
            out.matches('\n').count(),
            "every newline is still a CRLF: {out:?}"
        );
    }

    #[test]
    fn an_lf_config_stays_lf() {
        let (out, _) = migrate_text("[theme]\nrx = \"green\"\n");
        assert_eq!(out.matches('\r').count(), 0);
    }

    #[test]
    fn a_renamed_key_keeps_the_alignment_of_its_block() {
        // The default config aligns `=` within a section. A migrated line that
        // used a single space stood out as ragged in an otherwise tidy block,
        // which undercuts the whole point of migrating textually.
        let before = "[weather]\n\
                      location        = \"Boston\"\n\
                      forecast_days   = 4\n\
                      refresh_minutes = 30\n";
        let (after, _) = migrate_text(before);
        let migrated = after
            .lines()
            .find(|l| l.starts_with("forecast_hours"))
            .expect("the key was renamed");
        assert_eq!(
            migrated.find('='),
            Some(16),
            "`=` moved out of the column its neighbours use: {migrated:?}"
        );
    }

    #[test]
    fn a_renamed_key_keeps_its_indentation_and_always_spaces_the_equals() {
        let (after, _) = migrate_text("[weather]\n    forecast_days=4\n");
        let migrated = after
            .lines()
            .find(|l| l.trim_start().starts_with("forecast_hours"))
            .expect("the key was renamed");
        // Indent survives, and a file with no padding still gets one space
        // rather than `forecast_hours= 8`.
        assert!(
            migrated.starts_with("    forecast_hours = "),
            "{migrated:?}"
        );
    }

    #[test]
    fn section_headers_are_recognised() {
        assert_eq!(section_of("[weather]"), Some("weather"));
        assert_eq!(section_of("  [theme]  "), Some("theme"));
        assert_eq!(section_of("[[layout.rows]]"), Some("layout.rows"));
        assert_eq!(section_of("[theme.rx_gradient]"), Some("theme.rx_gradient"));
        assert_eq!(section_of("key = 1"), None);
        assert_eq!(section_of("# [weather]"), None);
    }

    #[test]
    fn assignments_are_recognised_and_comments_are_not() {
        assert_eq!(key_of("forecast_days = 4"), Some("forecast_days"));
        assert_eq!(key_of("  units   =  \"imperial\""), Some("units"));
        assert_eq!(key_of("# forecast_days = 4"), None);
        assert_eq!(key_of(""), None);
        assert_eq!(key_of("   "), None);
    }

    #[test]
    fn a_renamed_key_is_rewritten_in_place() {
        let (out, changes) = migrate_text("[weather]\nlocation = \"Boston\"\nforecast_days = 4\n");
        assert!(out.contains("forecast_hours = 8"), "got:\n{out}");
        assert!(
            !out.contains("forecast_days = 4"),
            "old key survived:\n{out}"
        );
        assert!(
            out.contains("location = \"Boston\""),
            "settings must survive"
        );
        assert_eq!(changes.len(), 1);
        assert!(changes[0].contains("forecast_hours"));
    }

    #[test]
    fn a_retired_key_is_commented_out_with_its_value_visible() {
        let (out, changes) = migrate_text("[theme]\nrx = \"green\"\n");
        assert!(out.contains("# rx = \"green\""), "got:\n{out}");
        assert_eq!(changes.len(), 1);
        assert!(changes[0].contains("rx_gradient"));
    }

    #[test]
    fn rules_are_scoped_to_their_section() {
        // A key called `rx` outside [theme] is somebody else's business.
        let (out, changes) = migrate_text("[network]\nrx = \"green\"\n");
        assert!(out.contains("rx = \"green\""));
        assert!(!out.contains("# rx"), "must not touch another section");
        assert!(changes.is_empty());
    }

    #[test]
    fn comments_and_blank_lines_survive_untouched() {
        let input =
            "# my notes\n\n[weather]\n# which city\nlocation = \"Boston\"\nforecast_days = 4\n";
        let (out, _) = migrate_text(input);
        assert!(out.contains("# my notes"));
        assert!(out.contains("# which city"));
        assert!(out.contains("\n\n"), "blank lines must survive");
    }

    #[test]
    fn a_file_with_nothing_to_migrate_is_returned_unchanged() {
        let input = "[weather]\nlocation = \"Boston\"\nforecast_hours = 8\n";
        let (out, changes) = migrate_text(input);
        assert_eq!(out, input);
        assert!(changes.is_empty());
    }

    #[test]
    fn a_migrated_v1_config_actually_parses() {
        // The real shape of the file shipped with 0.1.0, abridged.
        let v1 = r#"
[general]
tick_rate_ms = 250

[theme]
border         = "dark-gray"
border_focused = "cyan"
rx             = "green"
tx             = "magenta"

[clocks]
time_format = "%H:%M:%S"

[weather]
location        = "Boston, Massachusetts"
units           = "imperial"
forecast_days   = 4
refresh_minutes = 30
"#;
        assert!(
            toml::from_str::<crate::config::Config>(v1).is_err(),
            "the v1 config must be rejected, or there is nothing to migrate"
        );

        let (migrated, changes) = migrate_text(v1);
        assert_eq!(changes.len(), 3, "forecast_days, rx and tx");
        toml::from_str::<crate::config::Config>(&migrated).expect("a migrated config must load");
        // The user's real settings survive.
        assert!(migrated.contains("Boston, Massachusetts"));
        assert!(migrated.contains("refresh_minutes = 30"));
        assert!(migrated.contains("border_focused = \"cyan\""));
    }

    #[test]
    fn migrating_a_healthy_file_is_a_no_op_and_leaves_no_backup() {
        let dir = std::env::temp_dir().join(format!("mirador-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, crate::config::DEFAULT_CONFIG).unwrap();

        let report = migrate_file(&path).expect("the shipped default must be healthy");
        assert!(report.is_empty());
        assert!(report.backup.is_none());
        assert!(!path.with_extension("toml.bak").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrating_a_stale_file_backs_it_up_and_rewrites_it() {
        let dir = std::env::temp_dir().join(format!("mirador-migrate2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[weather]\nlocation = \"Oslo\"\nforecast_days = 4\n").unwrap();

        let report = migrate_file(&path).expect("must migrate");
        assert!(!report.is_empty());

        let backup = report.backup.expect("a backup must be written");
        assert!(backup.exists());
        assert!(
            std::fs::read_to_string(&backup)
                .unwrap()
                .contains("forecast_days"),
            "the backup must be the original"
        );

        let now = std::fs::read_to_string(&path).unwrap();
        assert!(now.contains("forecast_hours"));
        assert!(now.contains("Oslo"), "settings must survive");
        toml::from_str::<crate::config::Config>(&now).expect("the result must load");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unrecognisable_failure_leaves_the_file_alone() {
        let dir = std::env::temp_dir().join(format!("mirador-migrate3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let original = "[weather]\nthis is not toml =\n";
        std::fs::write(&path, original).unwrap();

        assert!(migrate_file(&path).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "a file we cannot fix must not be touched"
        );
        assert!(!path.with_extension("toml.bak").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
