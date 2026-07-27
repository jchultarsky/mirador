//! Writing `[layout]` changes back into the config file.
//!
//! Which panels exist and how wide they are can be changed from the dashboard,
//! and those changes have to land somewhere the user will find them. That is
//! the config: `[layout]` is the part people actually read and curate, and
//! recording it anywhere else would leave the file describing a dashboard
//! nobody is looking at.
//!
//! Rewriting is **textual**, for the reason [`crate::migrate`] gives at length
//! and does not need repeating here. Lines are edited, inserted and deleted in
//! place; everything else — spacing, ordering, comments, even a comment
//! *inside* the layout block — is left exactly as the user left it.
//!
//! The safety property that makes this defensible is at the bottom of
//! [`apply`]: the edited text is parsed before it is returned, and the layout it
//! produces is compared against the one that was asked for. A mismatch means
//! the surgery went wrong, and the edit is thrown away rather than written. So
//! the failure mode of an unusual config is "your change did not stick",
//! reported, rather than "your config is now broken".

use anyhow::{Result, bail};

use crate::config::{Config, Layout};

/// Rewrite the `[layout]` block of `source` so it describes `desired`.
///
/// Returns the new file text. Fails rather than guessing if the block cannot be
/// edited confidently, leaving the caller to report that and change nothing.
pub fn apply(source: &str, desired: &Layout) -> Result<String> {
    let current: Config = toml::from_str(source)?;
    let lines: Vec<&str> = source.lines().collect();
    let map = map_layout(&lines)?;

    if map.rows.len() != current.layout.rows.len() {
        bail!(
            "found {} layout rows in the text but {} when parsed; the `[layout]` \
             block is formatted in a way this cannot edit safely",
            map.rows.len(),
            current.layout.rows.len()
        );
    }

    // Work back to front so an insertion or deletion cannot shift the line
    // numbers of an edit that has not happened yet.
    let mut out: Vec<String> = lines.iter().map(|line| (*line).to_string()).collect();
    let mut edits: Vec<Edit> = Vec::new();

    for (row_index, row) in map.rows.iter().enumerate() {
        let Some(want_row) = desired.rows.get(row_index) else {
            // The row is gone entirely. Deleting a whole row textually means
            // removing its opening and closing lines too, which is beyond what
            // this does; the caller keeps the row and empties it instead.
            bail!("removing a whole layout row is not supported");
        };

        if current.layout.rows[row_index].height != want_row.height {
            edits.push(Edit::Number {
                line: row.header_line,
                key: "height",
                value: want_row.height,
            });
        }

        for panel in &row.panels {
            match want_row
                .panels
                .iter()
                .find(|wanted| wanted.widget == panel.widget)
            {
                Some(wanted) if wanted.width != panel.width => edits.push(Edit::Number {
                    line: panel.line,
                    key: "width",
                    value: wanted.width,
                }),
                Some(_) => {}
                None => edits.push(Edit::Delete { line: panel.line }),
            }
        }

        for wanted in &want_row.panels {
            if !row.panels.iter().any(|p| p.widget == wanted.widget) {
                let after = row.panels.last().map_or(row.header_line, |p| p.line);
                edits.push(Edit::Insert {
                    after,
                    text: panel_line(&lines, after, &wanted.widget, wanted.width),
                });
            }
        }
    }

    // Deletions and insertions both key off original line numbers, so applying
    // from the bottom up keeps every unapplied edit's index valid.
    edits.sort_by_key(Edit::anchor);
    for edit in edits.iter().rev() {
        match edit {
            Edit::Number { line, key, value } => {
                out[*line] = set_number(&out[*line], key, *value);
            }
            Edit::Delete { line } => {
                out.remove(*line);
            }
            Edit::Insert { after, text } => {
                out.insert(after + 1, text.clone());
            }
        }
    }

    let mut result = out.join("\n");
    if source.ends_with('\n') {
        result.push('\n');
    }

    // The whole reason this is safe to do at all. If the text no longer says
    // what it was meant to say, the edit is wrong and is thrown away.
    let reparsed: Config = toml::from_str(&result)
        .map_err(|e| anyhow::anyhow!("the edited config no longer parses: {e}"))?;
    if shape(&reparsed.layout) != shape(desired) {
        bail!("the edited config does not describe the requested layout");
    }

    Ok(result)
}

/// A layout reduced to what this module promises to reproduce.
fn shape(layout: &Layout) -> Vec<(u16, Vec<(String, u16)>)> {
    layout
        .rows
        .iter()
        .map(|row| {
            (
                row.height,
                row.panels
                    .iter()
                    .map(|p| (p.widget.clone(), p.width))
                    .collect(),
            )
        })
        .collect()
}

enum Edit {
    Number {
        line: usize,
        key: &'static str,
        value: u16,
    },
    Delete {
        line: usize,
    },
    Insert {
        after: usize,
        text: String,
    },
}

impl Edit {
    fn anchor(&self) -> usize {
        match self {
            Self::Number { line, .. } | Self::Delete { line } => *line,
            Self::Insert { after, .. } => *after,
        }
    }
}

struct PanelSite {
    widget: String,
    width: u16,
    line: usize,
}

struct RowSite {
    /// The line carrying `height = …`, which is also where a row with no panels
    /// gets its first one inserted after.
    header_line: usize,
    panels: Vec<PanelSite>,
}

struct LayoutMap {
    rows: Vec<RowSite>,
}

/// Find every row and panel in the `[layout]` block, by line.
///
/// Deliberately literal: a row starts at a line containing `panels = [`, and a
/// panel is a line containing `widget = "…"`. That is the shape mirador writes
/// and the shape anyone editing it by hand will have copied. A file that does
/// not match — everything on one line, say — produces a map that disagrees with
/// the parsed config, which [`apply`] treats as a refusal rather than guessing.
fn map_layout(lines: &[&str]) -> Result<LayoutMap> {
    let mut rows: Vec<RowSite> = Vec::new();
    let mut in_layout = false;

    for (index, raw) in lines.iter().enumerate() {
        let line = strip_comment(raw);
        let trimmed = line.trim();

        if trimmed.starts_with('[') && !trimmed.starts_with("[[") {
            in_layout = trimmed == "[layout]";
            continue;
        }
        if !in_layout {
            continue;
        }

        if line.contains("panels") && line.contains('[') {
            rows.push(RowSite {
                header_line: index,
                panels: Vec::new(),
            });
        }
        if let Some(widget) = quoted_value(line, "widget")
            && let Some(width) = number_value(line, "width")
            && let Some(row) = rows.last_mut()
        {
            row.panels.push(PanelSite {
                widget,
                width,
                line: index,
            });
        }
    }

    if rows.is_empty() {
        bail!("no `[layout]` rows found in the config text");
    }
    Ok(LayoutMap { rows })
}

/// Everything before an unquoted `#`.
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..index],
            _ => {}
        }
    }
    line
}

/// `key = "value"` on this line.
fn quoted_value(line: &str, key: &str) -> Option<String> {
    let rest = after_key(line, key)?;
    let rest = rest.trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// `key = 123` on this line.
fn number_value(line: &str, key: &str) -> Option<u16> {
    let rest = after_key(line, key)?;
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// The text just past `key =`, ignoring keys that are only a suffix of a longer
/// one — `width` must not match inside `max_width`.
fn after_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(found) = line[from..].find(key) {
        let at = from + found;
        let before_ok = at == 0
            || !line[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let rest = &line[at + key.len()..];
        let after = rest.trim_start();
        if before_ok && let Some(value) = after.strip_prefix('=') {
            return Some(value);
        }
        from = at + key.len();
    }
    None
}

/// Replace `key = N` on a line, leaving the rest of it untouched.
fn set_number(line: &str, key: &str, value: u16) -> String {
    let Some(rest) = after_key(line, key) else {
        return line.to_string();
    };
    let start = line.len() - rest.len();
    let spaces = rest.len() - rest.trim_start().len();
    let digits = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .count();
    let new = value.to_string();
    // Keep the column the value started in when it is not getting longer, so a
    // hand-aligned block stays aligned.
    let padding = spaces + digits.saturating_sub(new.len());
    format!(
        "{}{}{}{}",
        &line[..start],
        " ".repeat(padding.max(1)),
        new,
        &line[start + spaces + digits..]
    )
}

/// A new panel line, indented and spaced to match the one it follows.
fn panel_line(lines: &[&str], after: usize, widget: &str, width: u16) -> String {
    let template = lines.get(after).copied().unwrap_or("");
    let indent: String = template
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect::<String>();
    // Rows nest one level deeper than their header, so a panel inserted into an
    // empty row needs the extra step in.
    let indent = if quoted_value(strip_comment(template), "widget").is_some() {
        indent
    } else {
        format!("{indent}  ")
    };
    format!("{indent}{{ widget = \"{widget}\", width = {width} }},")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# a comment at the top
[general]
mouse = true

# ---------------------------------------------------------------------------
# Layout
# ---------------------------------------------------------------------------
[layout]
rows = [
  { height = 34, panels = [
    { widget = "clocks",   width = 26 },
    # Wide enough for two months side by side.
    { widget = "calendar", width = 34 },
  ] },
  { height = 42, panels = [
    { widget = "todo",     width = 48 },
    { widget = "notes",    width = 30 },
  ] },
]

[weather]
units = "imperial"
"#;

    fn layout_of(text: &str) -> Layout {
        toml::from_str::<Config>(text).expect("parses").layout
    }

    #[test]
    fn no_change_produces_an_identical_file() {
        let desired = layout_of(SAMPLE);
        assert_eq!(apply(SAMPLE, &desired).unwrap(), SAMPLE);
    }

    #[test]
    fn a_width_change_touches_only_that_number() {
        let mut desired = layout_of(SAMPLE);
        desired.rows[1].panels[0].width = 60;
        let out = apply(SAMPLE, &desired).unwrap();

        assert!(out.contains(r#"{ widget = "todo",     width = 60 },"#));
        assert!(
            out.contains("# Wide enough for two months side by side."),
            "a comment inside the layout block must survive"
        );
        assert!(out.contains("# a comment at the top"));
        assert!(out.contains(r#"units = "imperial""#));
        assert_eq!(layout_of(&out).rows[1].panels[0].width, 60);
    }

    #[test]
    fn a_height_change_edits_the_row_header() {
        let mut desired = layout_of(SAMPLE);
        desired.rows[0].height = 50;
        let out = apply(SAMPLE, &desired).unwrap();
        assert!(out.contains("{ height = 50, panels = ["));
        assert_eq!(layout_of(&out).rows[0].height, 50);
    }

    #[test]
    fn adding_a_panel_inserts_one_line_after_the_last_of_its_row() {
        let mut desired = layout_of(SAMPLE);
        desired.add_widget("pomodoro");
        let out = apply(SAMPLE, &desired).unwrap();

        assert!(out.contains(r#"{ widget = "pomodoro", width = "#));
        assert_eq!(
            out.lines().count(),
            SAMPLE.lines().count() + 1,
            "exactly one line added"
        );
        assert!(layout_of(&out).places("pomodoro"));
        assert!(out.contains("# Wide enough for two months side by side."));
    }

    #[test]
    fn removing_a_panel_deletes_only_its_line() {
        let mut desired = layout_of(SAMPLE);
        assert!(desired.remove_widget("notes"));
        let out = apply(SAMPLE, &desired).unwrap();

        assert!(!out.contains(r#"widget = "notes""#));
        assert!(out.contains(r#"widget = "todo""#));
        assert_eq!(out.lines().count(), SAMPLE.lines().count() - 1);
        assert!(!layout_of(&out).places("notes"));
    }

    #[test]
    fn several_changes_at_once_do_not_disturb_each_others_line_numbers() {
        let mut desired = layout_of(SAMPLE);
        desired.rows[0].panels[0].width = 20;
        desired.remove_widget("calendar");
        desired.add_widget("cpu");
        desired.rows[1].height = 55;

        let out = apply(SAMPLE, &desired).unwrap();
        let got = layout_of(&out);

        assert_eq!(got.rows[0].panels[0].width, 20);
        assert!(!got.places("calendar"));
        assert!(got.places("cpu"));
        assert_eq!(got.rows[1].height, 55);
    }

    #[test]
    fn a_layout_this_cannot_map_is_refused_rather_than_mangled() {
        // Everything on one line: legal TOML, and not the shape the line-based
        // map understands.
        let flat = "[layout]\nrows = [ { height = 100, panels = [ { widget = \"todo\", width = 100 } ] } ]\n";
        let mut desired = layout_of(flat);
        desired.add_widget("notes");

        // Either it maps it correctly or it refuses; what it must never do is
        // write something that does not say what was asked.
        if let Ok(out) = apply(flat, &desired) {
            assert_eq!(shape(&layout_of(&out)), shape(&desired));
        }
    }

    #[test]
    fn a_file_without_a_layout_block_is_refused() {
        let text = "[general]\nmouse = true\n";
        let desired = layout_of(SAMPLE);
        assert!(apply(text, &desired).is_err());
    }

    #[test]
    fn width_is_not_confused_by_a_key_that_ends_in_the_same_word() {
        assert_eq!(number_value("max_width = 7, width = 42", "width"), Some(42));
        assert_eq!(after_key("max_width = 7", "width"), None);
    }

    #[test]
    fn a_commented_out_panel_is_not_treated_as_a_real_one() {
        let text = SAMPLE.replace(
            r#"    { widget = "notes",    width = 30 },"#,
            r#"    # { widget = "notes",    width = 30 },"#,
        );
        let desired = layout_of(&text);
        assert!(!desired.places("notes"));
        // Round-trips without resurrecting the commented line.
        let out = apply(&text, &desired).unwrap();
        assert!(out.contains(r#"# { widget = "notes""#));
        assert!(!layout_of(&out).places("notes"));
    }

    #[test]
    fn trailing_newline_is_preserved_either_way() {
        let desired = layout_of(SAMPLE);
        assert!(apply(SAMPLE, &desired).unwrap().ends_with('\n'));

        let without = SAMPLE.trim_end_matches('\n');
        assert!(!apply(without, &desired).unwrap().ends_with('\n'));
    }

    /// The comment count is the whole justification for this module, and it is
    /// quoted in two places that no compiler checks: `CLAUDE.md` invariant 16
    /// and the 0.4.0 changelog entry. It had already drifted into "~145" in
    /// both, and "two hundred" in a third citation since removed.
    ///
    /// A failure here is not a bug in the config. It means the number moved and
    /// the citations have to move with it.
    #[test]
    fn the_comment_count_the_docs_quote_is_the_one_in_the_file() {
        const CITED: usize = 159;
        let actual = crate::config::DEFAULT_CONFIG
            .lines()
            .filter(|line| line.trim_start().starts_with('#'))
            .count();
        assert_eq!(
            actual, CITED,
            "the default config now has {actual} comment lines. Update this \
             constant, `CLAUDE.md` invariant 16 and the 0.4.0 changelog entry \
             — they all quote {CITED}."
        );
    }
}
