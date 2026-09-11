//! Checks that the working notes still describe the program.
//!
//! `CLAUDE.md` is the file the next session reads cold, and it has no compiler.
//! Six claims in it went stale in a single day — a released version six
//! releases behind, an issue described as open for a day after it closed
//! (twice), a soak result denied after it had happened, a terminal capability
//! called untestable after it had been tested. Every one was corrected by hand,
//! and correcting them by hand is what let the next one through.
//!
//! **This module does not check prose, and cannot.** "No soak has crossed a
//! real midnight" is a sentence about the world; nothing here can know. What it
//! checks is the part of the notes that *names things in the repository*, where
//! staleness is a mechanical fact:
//!
//! - a test cited by name that no longer exists,
//! - the released version disagreeing with the manifest,
//! - a source path that has been moved or deleted.
//!
//! That is a minority of what can rot, and saying so matters more than the
//! coverage: a guard that looks total and is not would make the untested
//! sentences feel checked. Treat a green run as "the names are real", nothing
//! further.

use std::path::{Path, PathBuf};

/// The notes, with line endings normalised.
///
/// `include_str!` does *not* normalise CRLF the way rustc does inside a string
/// literal, so a Windows checkout hands this file back with `\r\n` and a naive
/// comparison fails there and nowhere else. That asymmetry has already cost
/// this repository one confusing CI failure; see the `layout_edit` sweep.
fn notes() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("CLAUDE.md");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// The README, with line endings normalised for the same reason as `notes`.
fn readme() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// The individual keys a `Binding::key` label or a README key cell names.
///
/// Both sides spell a list of keys in prose — `j / k`, `+/-`, `0-9 . + - * /`,
/// `` `1` – `9` ``, `` `Enter` or `=` `` — and the comparison has to be between
/// the keys, not the prose. `/` is both a separator and a key, which is why
/// ` / ` with spaces is split first and a lone `/` afterwards is kept.
fn key_tokens(label: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for group in label.split(" / ") {
        for piece in group.split_whitespace() {
            let pieces: Vec<&str> = if piece.len() > 1 && piece.contains('/') {
                piece.split('/').collect()
            } else {
                vec![piece]
            };
            for raw in pieces {
                if raw.is_empty() || raw == "or" {
                    continue;
                }
                let token = match raw.replace('–', "-").as_str() {
                    "Enter" | "↵" => "↵".to_string(),
                    "Space" | "space" => "space".to_string(),
                    "PageUp" => "PgUp".to_string(),
                    "PageDown" => "PgDn".to_string(),
                    other => other.to_string(),
                };
                out.insert(token);
            }
        }
    }
    out
}

/// The keys named in one README table cell: every backtick span, with
/// `` `X`–`Y` `` ranges joined back into `X-Y` before splitting.
fn readme_cell_keys(cell: &str) -> std::collections::BTreeSet<String> {
    // Only an en dash joins a range: `` `0`–`9` `` is one key, `` `+` `-` ``
    // is two, and the minus key is spelled with the same character as the
    // ASCII range dash — so the ASCII form is never treated as a range here.
    let mut joined = cell.to_string();
    for dash in ["`–`", "` – `"] {
        joined = joined.replace(dash, "-");
    }
    let spans: Vec<&str> = joined.split('`').skip(1).step_by(2).collect();
    key_tokens(&spans.join(" "))
}

/// The README's key tables, keyed by the panel (or `global`) each describes.
///
/// Three panels and the global keys have tables; every other panel documents
/// its keys in prose, which this cannot check. The tasks table sits under the
/// arranging heading, introduced by a sentence rather than a heading of its
/// own, so it is found by that sentence.
fn readme_key_tables() -> std::collections::BTreeMap<&'static str, Vec<String>> {
    let text = readme();
    let mut tables: std::collections::BTreeMap<&'static str, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut heading = String::new();
    let mut intro = String::new();
    for line in text.lines() {
        if line.starts_with('#') {
            heading = line.trim_start_matches('#').trim().to_string();
            intro.clear();
            continue;
        }
        if line.starts_with("| `") {
            let cell = line
                .trim_start_matches('|')
                .split('|')
                .next()
                .unwrap_or("")
                .trim();
            let which = match (heading.as_str(), intro.as_str()) {
                ("Pomodoro", _) => "pomodoro",
                ("Calculator", _) => "calculator",
                ("Keys", _) => "global",
                (_, "In the task panel:") => "todo",
                _ => continue,
            };
            tables.entry(which).or_default().push(cell.to_string());
        } else if !line.trim().is_empty() && !line.starts_with('|') {
            intro = line.trim().to_string();
        }
    }
    tables
}

/// Every `.rs` file under `src/`, concatenated.
fn sources() -> String {
    fn walk(dir: &Path, into: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, into);
            } else if path.extension().is_some_and(|e| e == "rs")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                into.push_str(&text);
                into.push('\n');
            }
        }
    }

    let mut all = String::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut all);
    all
}

/// Backticked `snake_case` names in the notes long enough to be test names.
///
/// Four underscores is the discriminator, and it is empirical rather than
/// principled: test names in this repository are sentences
/// (`a_layout_missing_widgets_is_not_advertised_anywhere`) while config keys
/// and ordinary functions are not (`refresh_secs`, `write_atomic`,
/// `free_backup_path`). Measured over the whole file when this was written, it
/// selected twenty-one names, twenty of which were tests and one of which was
/// a test that had been renamed — which is the case this exists to catch.
fn cited_test_names(notes: &str) -> Vec<String> {
    let mut found = Vec::new();
    for chunk in notes.split('`').skip(1).step_by(2) {
        let looks_like_a_name = !chunk.is_empty()
            && chunk.starts_with(|c: char| c.is_ascii_lowercase())
            && chunk
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if looks_like_a_name && chunk.matches('_').count() >= 4 {
            found.push(chunk.to_string());
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Repository paths the notes cite in backticks.
fn cited_paths(notes: &str) -> Vec<PathBuf> {
    let roots = ["src/", "assets/", "docs/", ".github/"];
    let mut found = Vec::new();
    for chunk in notes.split('`').skip(1).step_by(2) {
        if roots.iter().any(|r| chunk.starts_with(r))
            && chunk
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
        {
            found.push(PathBuf::from(chunk));
        }
    }
    found.sort();
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test named in the notes has to exist.
    ///
    /// This is the failure with the longest history here. The news panel
    /// section claimed four commitments were tested when two were not, and said
    /// "there is a test whose failure message says so" about a test nobody had
    /// written. A citation is a claim, and this is the one kind of claim in
    /// that file which can be checked.
    #[test]
    fn every_test_the_notes_name_still_exists() {
        let sources = sources();
        let missing: Vec<String> = cited_test_names(&notes())
            .into_iter()
            .filter(|name| !sources.contains(&format!("fn {name}")))
            .collect();

        assert!(
            missing.is_empty(),
            "CLAUDE.md names {} test(s) that do not exist: {missing:?}\n\n\
             Either the test was renamed and the notes still cite the old name, \
             or the claim around it was never true. Fix whichever it is — do not \
             delete the citation to make this pass, because the sentence around \
             it is asserting that something is pinned.\n\n\
             If one of these is not a test at all but a long snake_case name \
             this check mistook for one, rename it in the notes or widen the \
             filter deliberately.",
            missing.len()
        );
    }

    /// The released version in the notes has to match the manifest.
    ///
    /// That line carries a comment warning that it goes stale every release,
    /// and it went stale anyway — by six releases, on the very line that says
    /// so. A warning to a reader is not a check.
    #[test]
    fn the_released_version_in_the_notes_matches_the_manifest() {
        let notes = notes();
        let claimed = notes
            .lines()
            .find_map(|line| {
                let rest = line.strip_prefix("- **`")?;
                let (version, tail) = rest.split_once('`')?;
                tail.starts_with(" is released**")
                    .then(|| version.to_string())
            })
            .expect(
                "CLAUDE.md must carry a line of the form \"- **`X.Y.Z` is released**\"; \
                 if that housekeeping note was reworded, reword this check with it",
            );

        assert_eq!(
            claimed,
            env!("CARGO_PKG_VERSION"),
            "CLAUDE.md says {claimed} is released; Cargo.toml says {}. \
             The version bump and the note belong in the same commit.",
            env!("CARGO_PKG_VERSION")
        );
    }

    /// A source path the notes cite has to exist.
    ///
    /// **Much narrower than the other two, and worth knowing how narrow.** As
    /// written it sees only the paths the notes happen to put in backticks,
    /// which today are under `.github/`, `assets/` and `docs/` — the source
    /// files are almost always named in prose without them, so *no* `src/`
    /// path is covered. Verifying this test the first time appeared to show it
    /// working when the mutation had simply landed on a path it never reads.
    ///
    /// Kept anyway: a moved workflow or a deleted asset is exactly the change
    /// that leaves prose pointing at nothing, and it costs nothing to run.
    /// Widening it means backticking the paths in the notes, not loosening the
    /// filter here — a looser filter would start matching prose.
    #[test]
    fn every_repository_path_the_notes_cite_exists() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let missing: Vec<PathBuf> = cited_paths(&notes())
            .into_iter()
            .filter(|path| !root.join(path).exists())
            .collect();

        assert!(
            missing.is_empty(),
            "CLAUDE.md cites {} path(s) that are not there: {missing:?}\n\n\
             Something was moved or renamed and the notes were not brought along.",
            missing.len()
        );
    }

    /// Every released version has a changelog link definition.
    ///
    /// `CHANGELOG.md` uses reference-style headings — `## [1.5.0]` — which
    /// render as plain text unless a matching `[1.5.0]: <url>` sits at the foot
    /// of the file. Seven consecutive releases shipped without one, so seven
    /// version headings on GitHub read as literal brackets rather than as
    /// compare links.
    ///
    /// Nothing noticed, because the file still parsed, the release still went
    /// out, and the defect is only visible rendered. That is the shape of every
    /// entry in this module: true-looking prose that no build step reads.
    #[test]
    fn every_released_version_has_a_changelog_link() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("CHANGELOG.md");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
            .replace("\r\n", "\n");

        let versions: Vec<String> = text
            .lines()
            .filter_map(|line| line.strip_prefix("## ["))
            .filter_map(|rest| rest.split(']').next())
            .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
            .map(str::to_string)
            .collect();
        assert!(
            versions.len() > 10,
            "only {} version headings found — the format probably changed and \
             this check has stopped looking at anything",
            versions.len()
        );

        let missing: Vec<&String> = versions
            .iter()
            .filter(|v| !text.contains(&format!("\n[{v}]: http")))
            .collect();
        assert!(
            missing.is_empty(),
            "{} released version(s) have no link definition: {missing:?}\n\n\
             Add `[X.Y.Z]: <compare url>` at the foot of CHANGELOG.md beside \
             the others. Without it the heading renders as literal brackets.",
            missing.len()
        );

        // The moving one, which is wrong rather than merely absent when stale.
        let newest = versions.first().expect("at least one version");
        assert!(
            text.contains(&format!(
                "[Unreleased]: https://github.com/jchultarsky/mirador/compare/v{newest}...HEAD"
            )),
            "[Unreleased] should compare against v{newest}, the newest release. \
             A stale one presents shipped work as unreleased, which is worse \
             than a missing link."
        );
    }

    /// Every module in `src/` appears in the architecture map.
    ///
    /// The map is the first thing a reader consults and the easiest thing to
    /// forget: it was missing `docs.rs`, `upgrade.rs` and `clipboard.rs` at
    /// once — one added by a maintainer, one by an outside contributor, one
    /// mentioned in the prose below but never in the list. None of the other
    /// checks here could see it, because a module that is absent cites no test
    /// and no path.
    ///
    /// Absence is the failure mode this file exists to catch, and it is the
    /// one that a citation-based check is structurally blind to. So this
    /// compares the map against the tree instead.
    #[test]
    fn every_module_is_named_in_the_architecture_map() {
        fn directory_entry(map: &str, directory: &str) -> String {
            let heading = format!("{directory}/");
            let mut entry = String::new();
            let mut found = false;
            for line in map.lines() {
                if line.starts_with(&heading) {
                    found = true;
                    entry.push_str(line);
                } else if found && line.starts_with(' ') {
                    entry.push(' ');
                    entry.push_str(line.trim());
                } else if found {
                    break;
                }
            }
            entry
        }

        fn names_module(entry: &str, file: &str) -> bool {
            let stem = file.strip_suffix(".rs").unwrap_or(file);
            entry
                .split(|character: char| {
                    !(character.is_ascii_alphanumeric() || matches!(character, '_' | '.'))
                })
                .any(|word| word == file || word == stem)
        }

        fn walk(
            root: &Path,
            dir: &Path,
            map: &str,
            inspected: &mut usize,
            missing: &mut Vec<String>,
        ) {
            for entry in std::fs::read_dir(dir)
                .expect("source directory must be readable")
                .flatten()
            {
                let path = entry.path();
                if path.is_dir() {
                    walk(root, &path, map, inspected, missing);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "rs") {
                    continue;
                }

                let relative = path
                    .strip_prefix(root)
                    .expect("walked source stays below src")
                    .to_string_lossy()
                    .replace('\\', "/");
                if relative == "main.rs" {
                    continue;
                }
                *inspected += 1;

                let Some((directory, file)) = relative.split_once('/') else {
                    if !map.contains(&relative) {
                        missing.push(relative);
                    }
                    continue;
                };
                let entry = directory_entry(map, directory);
                if entry.is_empty() || (file != "mod.rs" && !names_module(&entry, file)) {
                    missing.push(relative);
                }
            }
        }

        let notes = notes();
        let map = notes
            .split("## Architecture")
            .nth(1)
            .and_then(|rest| rest.split("```").nth(1))
            .expect("CLAUDE.md must carry an ```-fenced map under `## Architecture`");

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        let mut inspected = 0usize;
        walk(&root, &root, map, &mut inspected, &mut missing);
        missing.sort();

        assert!(
            inspected > 40,
            "the architecture sweep saw only {inspected} modules; it must recurse into \
             config/, plugin/ and widgets/ rather than pass on top-level files alone"
        );

        assert!(
            missing.is_empty(),
            "the architecture map does not mention {}: {missing:?}\n\n\
             Add a line for each. The map is what a reader consults first, and \
             a module missing from it is a module they will not know exists — \
             which is exactly how three of them went unlisted at once.",
            missing.len()
        );
    }

    /// The README draws eight panels in fenced code blocks, and a box whose
    /// sides do not line up reads as a rendering fault in the program rather
    /// than a typo in the documentation — it is the first thing a reader sees
    /// of what mirador looks like.
    ///
    /// One had been wrong since #138: the news panel's footer gained
    /// `o show link` and its trailing rule was padded one cell too far, so the
    /// bottom-right corner sat a column outside the box on the project's front
    /// page for weeks. Nobody re-reads a drawing they have already approved,
    /// which is exactly the kind of claim worth pinning mechanically.
    ///
    /// Measured in display cells rather than `chars()`, the same rule the
    /// program itself draws by — see invariant 9.
    #[test]
    fn every_panel_drawing_in_the_readme_is_a_rectangle() {
        let text = readme();
        let mut inside = false;
        let mut block: Vec<&str> = Vec::new();
        let mut start = 0usize;
        let mut checked = 0usize;
        let mut faults: Vec<String> = Vec::new();

        for (number, line) in text.lines().enumerate() {
            if line.starts_with("```") {
                if inside {
                    let art: Vec<&str> = block
                        .iter()
                        .copied()
                        .filter(|l| !l.trim().is_empty() && l.chars().any(|c| "╭╰│┤├".contains(c)))
                        .collect();
                    // Two lines cannot disagree about a rectangle in any way
                    // worth reporting; a real panel drawing has a top, a
                    // bottom and something between them.
                    if art.len() > 2 {
                        checked += 1;
                        let mut widths: Vec<usize> =
                            art.iter().map(|l| crate::grid::display_width(l)).collect();
                        widths.sort_unstable();
                        widths.dedup();
                        if widths.len() > 1 {
                            faults.push(format!(
                                "the block at README.md:{start} has lines of {widths:?} cells:\n{}",
                                art.iter()
                                    .map(|l| format!("  {:>3}  {l}", crate::grid::display_width(l)))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            ));
                        }
                    }
                    inside = false;
                    block.clear();
                } else {
                    inside = true;
                    start = number + 2;
                }
            } else if inside {
                block.push(line);
            }
        }

        assert!(faults.is_empty(), "{}", faults.join("\n\n"));
        // A filter that stops selecting anything passes for the wrong reason,
        // which is the failure this whole module exists to prevent.
        assert!(
            checked >= 6,
            "only {checked} panel drawing(s) found in README.md — the filter \
             has probably stopped matching, and a check that inspects nothing \
             passes every time"
        );
    }

    /// Each widget checks its documented keys against its own `BINDINGS`, and
    /// the README's key tables were checked against nothing — the last
    /// docs-versus-code seam with no guard, after two drawings went stale in
    /// an hour. Both directions: a key the README names must be one the panel
    /// declares, and every *primary* key — the ones shown without pressing
    /// `?` — must be in the README. Extras may be left out of the README;
    /// they are aliases and expert keys, and a table that lists `Esc clear
    /// filter` beside `/ filter` is longer without being more useful.
    ///
    /// The day it was written it found the calculator accepting `.` and `=`,
    /// documented in the README, with neither in the labels `?` shows.
    #[test]
    fn every_key_table_in_the_readme_matches_the_bindings_it_describes() {
        let tables = readme_key_tables();
        let declared: [(&str, &[crate::frame::Binding]); 4] = [
            ("global", crate::app::GLOBAL),
            ("todo", crate::widgets::todo::BINDINGS),
            ("pomodoro", crate::widgets::pomodoro::BINDINGS),
            ("calculator", crate::widgets::calculator::TAPE_BINDINGS),
        ];
        let mut faults = Vec::new();
        for (name, bindings) in declared {
            let rows = tables
                .get(name)
                .unwrap_or_else(|| panic!("no README key table found for `{name}`"));
            assert!(
                rows.len() >= 3,
                "the `{name}` table has {} rows — the parser has stopped matching",
                rows.len()
            );

            let documented: std::collections::BTreeSet<String> = rows
                .iter()
                .flat_map(|cell| readme_cell_keys(cell))
                .collect();
            let all: std::collections::BTreeSet<String> =
                bindings.iter().flat_map(|b| key_tokens(&b.key)).collect();
            let primary: std::collections::BTreeSet<String> = bindings
                .iter()
                .filter(|b| b.primary)
                .flat_map(|b| key_tokens(&b.key))
                .collect();

            for key in documented.difference(&all) {
                faults.push(format!(
                    "README documents `{key}` for `{name}`, which declares no such key"
                ));
            }
            for key in primary.difference(&documented) {
                faults.push(format!(
                    "`{name}` advertises `{key}` in its border and the README's table omits it"
                ));
            }
        }
        assert!(faults.is_empty(), "{}", faults.join("\n"));
    }

    #[test]
    fn key_tokens_split_the_way_both_sides_write_them() {
        let set = |s: &str| key_tokens(s).into_iter().collect::<Vec<_>>();
        assert_eq!(set("j / k"), ["j", "k"]);
        assert_eq!(set("+/-"), ["+", "-"]);
        assert_eq!(set("0-9 . + - * /"), ["*", "+", "-", ".", "/", "0-9"]);
        assert_eq!(set("Enter / ="), ["=", "↵"]);
        assert_eq!(set("Ctrl+←→↑↓"), ["Ctrl+←→↑↓"]);
        assert_eq!(set("PgUp / PgDn"), ["PgDn", "PgUp"]);
        let cell = |c: &str| readme_cell_keys(c).into_iter().collect::<Vec<_>>();
        assert_eq!(cell("`1` – `9`"), ["1-9"]);
        assert_eq!(cell("`0`–`9` `.`"), [".", "0-9"]);
        assert_eq!(cell("`+` `-` `*` `/`"), ["*", "+", "-", "/"]);
        assert_eq!(cell("`+` / `-`"), ["+", "-"]);
        assert_eq!(cell("`Enter` or `=`"), ["=", "↵"]);
        assert_eq!(cell("`j` / `k`, `↑` / `↓`"), ["j", "k", "↑", "↓"]);
        assert_eq!(cell("`PageUp` / `PageDown`"), ["PgDn", "PgUp"]);
        assert_eq!(cell("`Space`"), ["space"]);
    }

    /// The discriminator is empirical, so it is worth knowing when it stops
    /// selecting anything — a filter that matches nothing passes every time
    /// and checks nothing, which is the shape of failure this whole module
    /// exists to prevent.
    #[test]
    fn the_check_is_actually_looking_at_something() {
        let names = cited_test_names(&notes());
        assert!(
            names.len() >= 10,
            "only {} test name(s) recognised in CLAUDE.md — the filter has \
             probably stopped matching, and a check that inspects nothing \
             passes for the wrong reason: {names:?}",
            names.len()
        );
        assert!(
            !cited_paths(&notes()).is_empty(),
            "no repository paths recognised in CLAUDE.md; same concern"
        );
    }
}
