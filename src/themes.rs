//! Resolving `theme = "name"` into a [`Theme`].
//!
//! The `[theme]` table has always let you set every colour. What it could not
//! do is let you *keep* more than one set of them, or hand one to somebody
//! else. A theme is a file now: four ship inside the binary, and anything in
//! `<config>/mirador/themes/<name>.toml` is found first, so a bundled theme can
//! be replaced without editing it in place.
//!
//! Two things make a theme file short. `inherits` names another theme to start
//! from, so a variant is the handful of lines that differ rather than a copy of
//! all eighteen keys. And `[palette]` names colours once — `brass = "#d7af87"` —
//! after which any key can say `brass`, which is the difference between a theme
//! you can adjust and a theme you have to search and replace.
//!
//! Deliberately *not* built, and worth saying why rather than leaving the next
//! reader to wonder:
//!
//! - **Dotted-scope fallback** (`ui.border.focused` → `ui.border` → `ui`).
//!   Helix needs it because it has hundreds of syntax scopes and no chance of
//!   a theme naming them all. mirador has thirteen flat semantic keys, every
//!   one of which a theme can reasonably set, and `inherits` already covers the
//!   ones it does not. The machinery would be larger than the problem.
//! - **Full style objects** (`fg`/`bg`/modifiers per key). Every one of the
//!   ~240 places that reads a theme reads a flat `Color`, and nothing in the
//!   dashboard wants a themed background: panels draw on the terminal's own,
//!   which is what invariant 8 is about. Bold and reversed are decided by the
//!   widget that knows what it is emphasising, not by the palette.
//!
//! Both are additions rather than corrections if they are ever wanted.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::theme::Theme;

/// Themes compiled into the binary, so a fresh install has something to name.
///
/// `include_str!`-baked, with the same trap as `assets/default_config.toml`:
/// editing one does nothing until you rebuild.
const BUNDLED: &[(&str, &str)] = &[
    ("default", include_str!("../assets/themes/default.toml")),
    (
        "default-light",
        include_str!("../assets/themes/default-light.toml"),
    ),
    (
        "high-contrast",
        include_str!("../assets/themes/high-contrast.toml"),
    ),
    ("ansi", include_str!("../assets/themes/ansi.toml")),
];

/// How deep an `inherits` chain may go before it is treated as a mistake.
///
/// Cycles are caught by name, so this only bounds the honest-but-silly case.
const MAX_DEPTH: usize = 16;

/// The names that ship, for error messages.
pub fn bundled_names() -> Vec<&'static str> {
    BUNDLED.iter().map(|(name, _)| *name).collect()
}

/// Where a user's own themes live.
pub fn user_dir(config_path: &Path) -> Option<PathBuf> {
    config_path.parent().map(|dir| dir.join("themes"))
}

/// Resolve `name` into a theme, following `inherits` and applying `[palette]`.
///
/// `themes_dir` is searched before the bundled set, so a theme called `default`
/// on disk wins — replacing a shipped theme should not require a different name
/// for it.
pub fn resolve(name: &str, themes_dir: Option<&Path>) -> Result<Theme> {
    // Root first, so that each document in turn overwrites what it inherited.
    let mut chain: Vec<toml::Table> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut next = Some(name.to_string());

    while let Some(current) = next.take() {
        if !seen.insert(current.clone()) {
            bail!(
                "theme `{current}` inherits from itself, directly or through \
                 another theme. Break the loop by removing one `inherits`."
            );
        }
        if chain.len() >= MAX_DEPTH {
            bail!("theme `{name}` inherits through more than {MAX_DEPTH} files");
        }

        let mut table = read(&current, themes_dir)?;
        check_palette_ordering(&current, &table)?;
        if let Some(parent) = table.remove("inherits") {
            let toml::Value::String(parent) = parent else {
                bail!("`inherits` in theme `{current}` must be a theme name in quotes");
            };
            next = Some(parent);
        }
        chain.push(table);
    }
    chain.reverse();

    // Merge, child over parent, keeping palettes separate until the end: a
    // child that redefines `brass` must recolour the keys its *parent* set with
    // it, which merging them together as we go would not do.
    let mut palette: BTreeMap<String, toml::Value> = BTreeMap::new();
    let mut merged = toml::Table::new();
    for mut table in chain {
        if let Some(toml::Value::Table(entries)) = table.remove("palette") {
            palette.extend(entries);
        }
        merged.extend(table);
    }

    substitute(&mut merged, &palette);

    Theme::deserialize_table(merged)
        .with_context(|| format!("in theme `{name}`"))
        .map(|theme| theme.named(name))
}

/// Read one theme document, preferring the user's copy.
fn read(name: &str, themes_dir: Option<&Path>) -> Result<toml::Table> {
    if let Some(path) = themes_dir.map(|dir| dir.join(format!("{name}.toml")))
        && path.exists()
    {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading theme {}", path.display()))?;
        return toml::from_str(&raw).with_context(|| format!("parsing theme {}", path.display()));
    }

    if let Some((_, raw)) = BUNDLED.iter().find(|(bundled, _)| *bundled == name) {
        return toml::from_str(raw).with_context(|| format!("parsing the bundled `{name}` theme"));
    }

    let mut known = bundled_names();
    known.sort_unstable();
    let where_to_put_it = themes_dir.map_or_else(
        || "your themes directory".to_string(),
        |dir| dir.display().to_string(),
    );
    bail!(
        "no theme called `{name}`. Built in: {}. For anything else, put \
         `{name}.toml` in {where_to_put_it}.",
        known.join(", ")
    )
}

/// Catch colour keys that have fallen inside `[palette]`.
///
/// In TOML every key after a table header belongs to that table, so a file
/// written in the obvious order —
///
/// ```text
/// [palette]
/// brass = "#d7af87"
///
/// accent = "brass"
/// ```
///
/// puts `accent` *in the palette*, where it sets nothing. Nothing is
/// misspelled and nothing fails to parse; the theme simply comes out as the
/// defaults, which is the silent-ignore failure this crate refuses to ship.
///
/// I wrote the shipped `default.toml` this way and did not notice, because the
/// test comparing it against `Theme::default()` passed: with no keys set, the
/// result *was* the default. Hence both this check and a drift test that can
/// actually fail.
fn check_palette_ordering(name: &str, table: &toml::Table) -> Result<()> {
    let Some(toml::Value::Table(palette)) = table.get("palette") else {
        return Ok(());
    };
    let mut stray: Vec<&str> = palette
        .keys()
        .map(String::as_str)
        .filter(|key| Theme::KEYS.contains(key))
        .collect();
    if stray.is_empty() {
        return Ok(());
    }
    stray.sort_unstable();
    bail!(
        "theme `{name}` has {} inside `[palette]`, where {} nothing. In TOML \
         every key after a table header belongs to that table, so the colour \
         keys have to come *before* `[palette]`, not after it.",
        stray.join(", "),
        if stray.len() == 1 {
            "it sets"
        } else {
            "they set"
        }
    )
}

/// Replace palette names with the colours they stand for.
///
/// Only strings that are palette *keys* change; everything else is left for the
/// colour parser to accept or reject, so a typo still produces "`brss` is not a
/// colour" rather than being silently dropped.
fn substitute(table: &mut toml::Table, palette: &BTreeMap<String, toml::Value>) {
    for (_, value) in table.iter_mut() {
        // Taken by value and put back, because a palette hit replaces the whole
        // entry rather than editing the string in place.
        let replacement = match value {
            toml::Value::String(text) => palette.get(text.as_str()).cloned(),
            // One level down is enough: gradients are the only nested tables a
            // theme has, and their stops are plain strings.
            toml::Value::Table(nested) => {
                substitute(nested, palette);
                None
            }
            _ => None,
        };
        if let Some(resolved) = replacement {
            *value = resolved;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn themes_dir(name: &str) -> (PathBuf, TempDir) {
        let dir =
            std::env::temp_dir().join(format!("mirador-themes-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test directory");
        (dir.clone(), TempDir(dir))
    }

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(format!("{name}.toml")), body).expect("writing a theme");
    }

    #[test]
    fn every_bundled_theme_resolves() {
        for name in bundled_names() {
            resolve(name, None).unwrap_or_else(|e| panic!("bundled `{name}` failed: {e:#}"));
        }
    }

    /// The Rust `Theme::default()` and the shipped `default.toml` are reached
    /// by different routes — a config with no `theme` key takes the first, and
    /// `theme = "default"` takes the second — so they have to be pinned
    /// together. The layout has the same hazard and the same kind of test; when
    /// those two drifted, a config without a `[layout]` section silently lost
    /// three panels.
    #[test]
    fn the_bundled_default_theme_matches_the_rust_default() {
        let from_file = resolve("default", None).expect("resolves");
        let built_in = Theme::default();
        assert_eq!(
            from_file.colours(),
            built_in.colours(),
            "assets/themes/default.toml has drifted from Theme::default()"
        );
    }

    /// The comparison above cannot fail on its own, and for a while it did not.
    /// Every colour key in `default.toml` had fallen inside `[palette]`, so the
    /// file set *nothing*, the resolved theme was therefore the default, and
    /// the equality passed while the file was meaningless. Comparing against
    /// the default cannot detect that — the file resolving to the default is
    /// the whole point — so the property has to be checked on the file: a
    /// standalone theme names every key it is responsible for.
    #[test]
    fn a_standalone_theme_sets_every_key_rather_than_leaning_on_defaults() {
        // `high-contrast` is excluded on purpose: it inherits `ansi` and is
        // meant to be the handful of lines that differ.
        for name in ["default", "default-light", "ansi"] {
            let (_, raw) = BUNDLED
                .iter()
                .find(|(bundled, _)| *bundled == name)
                .expect("bundled");
            let table: toml::Table = toml::from_str(raw).expect("parses");

            let missing: Vec<&str> = Theme::KEYS
                .iter()
                .copied()
                .filter(|key| !table.contains_key(*key))
                .collect();
            assert!(
                missing.is_empty(),
                "`{name}.toml` never sets: {}",
                missing.join(", ")
            );
        }
    }

    /// The trap that produced the bug above. It is silent by construction —
    /// nothing is misspelled and nothing fails to parse — so it has to be
    /// caught rather than left for the theme author to puzzle over.
    #[test]
    fn colour_keys_that_fell_inside_the_palette_are_reported() {
        let (dir, _g) = themes_dir("ordering");
        write(
            &dir,
            "t",
            "[palette]\nbrass = \"#d7af87\"\naccent = \"brass\"\nborder = \"brass\"\n",
        );

        let err = format!("{:#}", resolve("t", Some(&dir)).expect_err("must fail"));
        assert!(err.contains("accent"), "names the key: {err}");
        assert!(err.contains("border"), "and every key: {err}");
        assert!(err.contains("before"), "and says what to do: {err}");
    }

    /// A palette entry that merely *shares a name* with nothing in the theme is
    /// fine; the check must not fire on an ordinary palette.
    #[test]
    fn an_ordinary_palette_is_left_alone() {
        let (dir, _g) = themes_dir("ordinary");
        write(
            &dir,
            "t",
            "accent = \"brass\"\n\n[palette]\nbrass = \"#010203\"\n",
        );
        resolve("t", Some(&dir)).expect("a well-ordered theme resolves");
    }

    #[test]
    fn a_theme_on_disk_wins_over_the_bundled_one_of_the_same_name() {
        let (dir, _g) = themes_dir("override");
        write(&dir, "default", "accent = \"#010203\"\n");

        let theme = resolve("default", Some(&dir)).expect("resolves");
        assert_eq!(theme.accent, ratatui::style::Color::Rgb(1, 2, 3));
    }

    #[test]
    fn inherits_starts_from_the_parent_and_the_child_overrides_it() {
        let (dir, _g) = themes_dir("inherits");
        write(
            &dir,
            "child",
            "inherits = \"default\"\naccent = \"#010203\"\n",
        );

        let child = resolve("child", Some(&dir)).expect("resolves");
        let parent = Theme::default();
        assert_eq!(
            child.accent,
            ratatui::style::Color::Rgb(1, 2, 3),
            "overridden"
        );
        assert_eq!(child.border, parent.border, "and the rest is inherited");
    }

    /// Without this the loop runs until the depth cap and reports the wrong
    /// thing, which sends someone looking for a file that is fine.
    #[test]
    fn a_cycle_is_named_rather_than_run_until_it_gives_up() {
        let (dir, _g) = themes_dir("cycle");
        write(&dir, "a", "inherits = \"b\"\n");
        write(&dir, "b", "inherits = \"a\"\n");

        let err = format!("{:#}", resolve("a", Some(&dir)).expect_err("must fail"));
        assert!(err.contains("inherits from itself"), "got: {err}");
    }

    /// A child that redefines a palette entry has to recolour the keys its
    /// *parent* set with it. Merging each document's palette as it is read
    /// would leave the parent's keys holding the parent's colour.
    #[test]
    fn a_redefined_palette_colour_reaches_the_keys_the_parent_set_with_it() {
        let (dir, _g) = themes_dir("palette");
        // The parent sets `accent` from the palette; the child changes only
        // what `brass` means.
        write(
            &dir,
            "base",
            "accent = \"brass\"\n\n[palette]\nbrass = \"#d7af87\"\n",
        );
        write(
            &dir,
            "child",
            "inherits = \"base\"\n\n[palette]\nbrass = \"#010203\"\n",
        );

        let theme = resolve("child", Some(&dir)).expect("resolves");
        assert_eq!(
            theme.accent,
            ratatui::style::Color::Rgb(1, 2, 3),
            "the parent's `accent = brass` follows the child's redefinition"
        );
    }

    #[test]
    fn an_unknown_name_lists_what_is_available() {
        let err = format!("{:#}", resolve("nord", None).expect_err("must fail"));
        assert!(err.contains("no theme called `nord`"), "got: {err}");
        for name in bundled_names() {
            assert!(err.contains(name), "`{name}` missing from: {err}");
        }
    }

    /// A palette name that is not in the palette must still reach the colour
    /// parser, so the message names the offending word.
    #[test]
    fn a_misspelled_palette_name_is_reported_as_a_bad_colour() {
        let (dir, _g) = themes_dir("typo");
        write(
            &dir,
            "t",
            "accent = \"brss\"\n\n[palette]\nbrass = \"#d7af87\"\n",
        );

        let err = format!("{:#}", resolve("t", Some(&dir)).expect_err("must fail"));
        assert!(err.contains("brss"), "the message names the typo: {err}");
    }

    #[test]
    fn a_misspelled_key_in_a_theme_file_is_refused() {
        let (dir, _g) = themes_dir("badkey");
        write(&dir, "t", "acent = \"#010203\"\n");

        let err = format!("{:#}", resolve("t", Some(&dir)).expect_err("must fail"));
        assert!(err.contains("acent"), "the message names the key: {err}");
    }
}
