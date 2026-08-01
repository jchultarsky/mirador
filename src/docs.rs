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
