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
        // Read both before the change, in the error that sends people here,
        // and after it, in the report: so no tense that is true of only one.
        why: "the forecast is hourly now, and a day count does not convert to hours, so the default of 8 is used",
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

impl Rule {
    /// The section the key has to be in for the rule to touch it.
    fn section(&self) -> &'static str {
        match self {
            Self::Replace { section, .. } | Self::Retire { section, .. } => section,
        }
    }

    /// The key the rule is about.
    fn key(&self) -> &'static str {
        match self {
            Self::Replace { from, .. } => from,
            Self::Retire { key, .. } => key,
        }
    }

    /// The change once it is made, in the words a migration reports it in.
    ///
    /// A retired key is reported "commented out", the verb
    /// [`Rule::proposal`] promised and what the file actually gets: it said
    /// "removed" of a value still there to read.
    fn change(&self) -> String {
        match self {
            Self::Replace {
                section,
                from,
                to,
                why,
                ..
            } => format!("[{section}] {from} -> {to}: {why}"),
            Self::Retire { section, key, why } => {
                format!("[{section}] {key} commented out: {why}")
            }
        }
    }

    /// The change before it is made, for the error that sends a reader here.
    /// [`Rule::change`] is the report afterwards; quoted before the migration
    /// had run, it told people their value had already been replaced.
    fn proposal(&self) -> String {
        match self {
            Self::Replace {
                section,
                from,
                to,
                value,
                why,
            } => format!("[{section}] {from} will become {to} = {value}: {why}"),
            Self::Retire { section, key, why } => {
                format!("[{section}] {key} will be commented out: {why}")
            }
        }
    }
}

/// The rule that rewrites `line`, if one does, given the table it sits in.
///
/// A header falls through here as well and matches nothing: whatever stands
/// before an `=` in it starts with `[`, and no rule's key does.
fn rule_for(section: &str, line: &str) -> Option<&'static Rule> {
    let key = key_of(line)?;
    RULES
        .iter()
        .find(|rule| rule.section() == section && rule.key() == key)
}

/// What `--migrate-config` will do to the line that byte `at` of `contents`
/// falls in, or `None` if it will leave that line alone.
///
/// For the parse error that sends a reader to the migration, which has to
/// promise only what the migration does. The parser names the key and a name
/// is not enough, because every rule is scoped to a table: `forecast_days`
/// under `[clocks]` was sent here by its name, and then refused. The line is
/// found by the same walk [`migrate_text`] makes, so the two cannot disagree
/// about which table it sits in, or about a spelling such as
/// `weather = { forecast_days = 4 }` that a line-by-line rewrite cannot reach.
pub fn change_at(contents: &str, at: usize) -> Option<String> {
    let (_, section, line) = lines_in_sections(contents).find(|&(end, ..)| at < end)?;
    rule_for(section, line).map(Rule::proposal)
}

/// Every rule's table and key, so the hint's tests can sweep them all.
#[cfg(test)]
pub fn stale_keys() -> impl Iterator<Item = (&'static str, &'static str)> {
    RULES.iter().map(|rule| (rule.section(), rule.key()))
}

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

/// Each line of `contents` without its ending, beside the top-level table it
/// sits in and the byte just past its ending. A header is reported in the
/// table it opens.
///
/// Split as `str::lines` splits — at `\n`, taking a `\r` before it with it —
/// but by hand, because `lines` does not say where a line ends and
/// [`change_at`] has to find the line a byte falls in.
fn lines_in_sections(contents: &str) -> impl Iterator<Item = (usize, &str, &str)> {
    let mut section = "";
    let mut end = 0;
    contents.split_inclusive('\n').map(move |raw| {
        end += raw.len();
        let line = raw
            .strip_suffix('\n')
            .map_or(raw, |line| line.strip_suffix('\r').unwrap_or(line));
        if let Some(name) = section_of(line) {
            section = name;
        }
        (end, section, line)
    })
}

/// Rewrite `contents`, returning the new text and a description of each change.
pub fn migrate_text(contents: &str) -> (String, Vec<String>) {
    // Whatever the file already used. A line comes without its `\r`, so
    // pushing `\n` would convert a CRLF config to LF and report every line as
    // changed.
    let newline = crate::store::line_ending(contents);
    let mut out = String::with_capacity(contents.len());
    let mut changes = Vec::new();

    for (_, section, line) in lines_in_sections(contents) {
        let Some(rule) = rule_for(section, line) else {
            out.push_str(line);
            out.push_str(newline);
            continue;
        };

        // Writing into a String is infallible. `write!` and an explicit
        // ending, not `writeln!`, which hard-codes `\n` and would put an LF
        // line into a CRLF file.
        match rule {
            Rule::Replace {
                from, to, value, ..
            } => {
                // Land `=` in the column the file already used, so a renamed
                // key does not break the alignment of the block around it.
                // Byte length is the column here: TOML bare keys are ASCII
                // and the only thing before them is indent whitespace.
                let indent = &line[..line.len() - line.trim_start().len()];
                let eq_col = line.find('=').unwrap_or_default();
                let pad = eq_col.saturating_sub(indent.len() + to.len()).max(1);
                let _ = write!(
                    out,
                    "{indent}{to}{:pad$}= {value}  # migrated from {from}",
                    ""
                );
            }
            Rule::Retire { why, .. } => {
                // Commented rather than deleted: the old value stays
                // visible so the user can see what their colour was.
                let _ = write!(out, "# {}  # removed: {why}", line.trim());
            }
        }
        out.push_str(newline);
        changes.push(rule.change());
    }

    (out, changes)
}

/// Why `--migrate-config` writes nothing to a config that does not load.
#[derive(Debug)]
pub enum Refusal {
    /// No line in it is one a rule rewrites.
    NothingKnown,
    /// Lines were rewritten and the result still does not load.
    StillBroken {
        /// Why, as the parser said it of the rewritten text.
        error: toml::de::Error,
        /// The line, counted from 1, that `error` points at. It is the
        /// reader's line too, because the migration keeps every line where it
        /// was; the error's own span is not, being a byte in text nobody has
        /// seen.
        line: Option<usize>,
    },
}

/// What `--migrate-config` makes of a config that does not load: the text it
/// would write and each change in it, or why it writes nothing.
///
/// The one place the migration decides whether it can finish, so the error
/// that sends a reader to it and the migration itself cannot disagree. A line
/// a rule rewrites is not enough: a `forecast_hours` already beside the stale
/// `forecast_days`, or an unknown key two lines down, and what it would write
/// still does not load, so it writes nothing — and the error must not have
/// promised otherwise.
pub fn migrate(contents: &str) -> std::result::Result<(String, Vec<String>), Refusal> {
    let (migrated, changes) = migrate_text(contents);
    if changes.is_empty() {
        return Err(Refusal::NothingKnown);
    }
    // Refuse to write something that still will not load. Better to leave the
    // user's file untouched and say so than to half-fix it.
    if let Err(error) = toml::from_str::<crate::config::Config>(&migrated) {
        let line = error.span().and_then(|span| {
            let before = migrated.get(..span.start)?;
            Some(before.matches('\n').count() + 1)
        });
        return Err(Refusal::StillBroken { error, line });
    }
    Ok((migrated, changes))
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

    let (migrated, changes) = migrate(&contents).map_err(|refusal| match refusal {
        Refusal::NothingKnown => anyhow::anyhow!(
            "the config at {} does not parse, and none of the problems are ones \
             this version knows how to migrate. Run `mirador --print-config` to \
             see the current format.",
            path.display()
        ),
        Refusal::StillBroken { error, .. } => anyhow::anyhow!(
            "migrating {} did not produce a usable config: {error}\n\nThe original \
             file has been left untouched.",
            path.display()
        ),
    })?;

    // A free name, not a fixed one: `config.toml.bak` may already hold the
    // config a reset set aside, and copying over it lost that for good.
    let backup = crate::store::free_backup_path(path);
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
    use crate::store::testing::TempDir;

    /// Same hazard as `layout_edit`: `str::lines()` strips the `\r`, so a
    /// migration of a Windows config rewrote every line in it.
    ///
    /// Asserted as the LF migration with its endings swapped, and through the
    /// check `migrate_file` makes before writing. Counting `\r\n` against `\n`
    /// could not fail: a line that kept its `\r` would come out ending
    /// `\r\r\n`, which holds one of each and which TOML refuses, so a change
    /// that had the migration refuse every Windows config passed it.
    #[test]
    fn a_crlf_config_stays_crlf() {
        let lf = "[weather]\nlocation = \"Oslo\"\nforecast_days = 4\n\n[theme]\nrx = \"green\"\n";
        let (lf_out, lf_changes) = migrate(lf).expect("the LF config migrates");
        let (out, changes) = migrate(&lf.replace('\n', "\r\n"))
            .unwrap_or_else(|refusal| panic!("the CRLF config was refused: {refusal:?}"));
        assert_eq!(changes, lf_changes);
        assert_eq!(changes.len(), 2, "both rules must act: {changes:?}");
        assert_eq!(out, lf_out.replace('\n', "\r\n"));
    }

    /// The other half of the CRLF walk: a line's end offset has to count the
    /// `\r` that the line itself drops, or the hint reads a later line than the
    /// parser stopped on. A byte per line of drift never leaves a three-line
    /// file, so this is the shipped config, stale where it sets the forecast,
    /// with the offset the parser itself reports.
    #[test]
    fn a_stale_key_deep_in_a_crlf_config_is_found_on_its_own_line() {
        let shipped = crate::config::DEFAULT_CONFIG.replace("\r\n", "\n");
        let lf = shipped.replacen("\nforecast_hours ", "\nforecast_days  ", 1);
        assert_ne!(
            lf, shipped,
            "the shipped config sets no forecast_hours to make stale"
        );
        let crlf = lf.replace('\n', "\r\n");

        let error = toml::from_str::<crate::config::Config>(&crlf).expect_err("a stale key");
        let at = error
            .span()
            .expect("the parser says where it stopped")
            .start;
        let line = crlf[..at].matches('\n').count() + 1;
        assert!(
            line > 100,
            "line {line} is too shallow for a drift of a byte a line to leave it"
        );
        let change = change_at(&crlf, at);
        assert!(
            change
                .as_deref()
                .is_some_and(|c| c.starts_with("[weather] forecast_days will become")),
            "line {line} of a CRLF config was read as {change:?}"
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
        assert!(changes.is_empty(), "{changes:?}");
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
        assert!(changes.is_empty(), "{changes:?}");
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
        let dir = TempDir::new("migrate");
        let path = dir.join("config.toml");
        std::fs::write(&path, crate::config::DEFAULT_CONFIG).unwrap();

        let report = migrate_file(&path).expect("the shipped default must be healthy");
        assert!(report.is_empty());
        assert!(report.backup.is_none());
        assert!(!path.with_extension("toml.bak").exists());
    }

    #[test]
    fn migrating_a_stale_file_backs_it_up_and_rewrites_it() {
        let dir = TempDir::new("migrate2");
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
    }

    /// A backup already beside the config — the one `--reset-config` leaves,
    /// or an earlier migration's — is somebody's original, and a migration
    /// copied over it under the same fixed name. Numbered like a reset's now.
    #[test]
    fn a_migration_does_not_overwrite_an_earlier_backup() {
        let dir = TempDir::new("migrate4");
        let path = dir.join("config.toml");
        let earlier = dir.join("config.toml.bak");
        std::fs::write(&earlier, "# the config a reset set aside\n").unwrap();
        std::fs::write(&path, "[weather]\nlocation = \"Oslo\"\nforecast_days = 4\n").unwrap();

        let report = migrate_file(&path).expect("must migrate");
        let kept = std::fs::read_to_string(&earlier).unwrap();
        let backup = report.backup.expect("a backup must be written");
        let original = std::fs::read_to_string(&backup).unwrap();

        assert_eq!(
            kept, "# the config a reset set aside\n",
            "the earlier backup was overwritten"
        );
        assert_ne!(backup, earlier, "the new backup needed a name of its own");
        assert!(
            original.contains("forecast_days"),
            "the new backup must be the original: {original:?}"
        );
    }

    #[test]
    fn an_unrecognisable_failure_leaves_the_file_alone() {
        let dir = TempDir::new("migrate3");
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
    }
}
