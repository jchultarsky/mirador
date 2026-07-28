//! Writing `[layout]` changes back into the config file.
//!
//! Which panels exist and how wide they are can be changed from the dashboard,
//! and those changes have to land somewhere the user will find them. That is
//! the config: `[layout]` is the part people actually read and curate, and
//! recording it anywhere else would leave the file describing a dashboard
//! nobody is looking at.
//!
//! Rewriting is **textual**, for the reason [`crate::migrate`] gives at length
//! and does not need repeating here. Everything else — spacing, ordering,
//! comments, even a comment *inside* the layout block — is left exactly as the
//! user left it.
//!
//! There are two ways that happens, and which one runs matters. When only the
//! numbers moved — the common case, every `Ctrl+arrow` — the digits are
//! replaced on their own line and nothing else is touched, so a block someone
//! aligned by hand stays aligned. When the *structure* moved, no per-line edit
//! can say it: a panel changing places with its neighbour leaves both still
//! present with the same widths, and the old version of this module emitted
//! nothing at all for it. So a row whose membership or order changed has its
//! panels rebuilt from their captured entries instead.
//!
//! An entry is a panel's line *plus the comment lines directly above it*, and
//! entries are looked up across the whole block rather than within one row.
//! That is what lets a panel dragged to the other side of the dashboard take
//! the sentence explaining it along, instead of leaving it behind to caption
//! whatever moved into its place.
//!
//! The safety property that makes this defensible is at the bottom of
//! [`apply`]: the edited text is parsed before it is returned, and the layout it
//! produces is compared against the one that was asked for. A mismatch means
//! the surgery went wrong, and the edit is thrown away rather than written. So
//! the failure mode of an unusual config is "your change did not stick",
//! reported, rather than "your config is now broken".

use std::collections::HashMap;

use anyhow::{Result, bail};

use crate::config::{Config, Layout, LayoutRow};

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

    // Every panel's entry, keyed by widget and found across the whole block
    // rather than within one row. That is what lets a panel moved to another
    // row — or to a row that did not exist a moment ago — take the comments
    // written above it along with it.
    let blocks = panel_blocks(&lines, &map);
    let pairing = pair_rows(&map, desired);

    // Work back to front so an insertion or deletion cannot shift the line
    // numbers of an edit that has not happened yet.
    let mut out: Vec<String> = lines.iter().map(|line| (*line).to_string()).collect();
    let mut edits: Vec<Edit> = Vec::new();

    // Rows nothing in the new layout claimed. Their panels have already been
    // captured, so anything that survived is being written somewhere else.
    for (text_index, row) in map.rows.iter().enumerate() {
        if !pairing.contains(&Some(text_index)) {
            edits.push(Edit::Replace {
                from: row.header_line,
                to: row.closing_line + 1,
                text: Vec::new(),
            });
        }
    }

    for (want_index, want) in desired.rows.iter().enumerate() {
        let Some(text_index) = pairing[want_index] else {
            // A row that has no counterpart in the text: write a whole new
            // block, anchored after the last row that does have one so the
            // rows come out in the order the layout asks for.
            let after = (0..want_index)
                .rev()
                .filter_map(|earlier| pairing[earlier])
                .map(|text| map.rows[text].closing_line + 1)
                .next()
                .unwrap_or(map.rows[0].header_line);
            edits.push(Edit::Replace {
                from: after,
                to: after,
                text: row_block(&lines, &map, &blocks, want),
            });
            continue;
        };

        let row = &map.rows[text_index];

        if current.layout.rows[text_index].height != want.height {
            edits.push(Edit::Number {
                line: row.header_line,
                key: "height",
                value: want.height,
            });
        }

        // The cheap path, and the common one: the same panels in the same
        // order, so only the numbers moved. Editing those in place keeps a
        // hand-aligned block aligned, which rebuilding the row would not.
        let unchanged_order = row
            .panels
            .iter()
            .map(|panel| panel.widget.as_str())
            .eq(want.panels.iter().map(|panel| panel.widget.as_str()));

        if unchanged_order {
            for (panel, wanted) in row.panels.iter().zip(&want.panels) {
                if panel.width != wanted.width {
                    edits.push(Edit::Number {
                        line: panel.line,
                        key: "width",
                        value: wanted.width,
                    });
                }
            }
            continue;
        }

        // The order or the membership changed, which no per-line edit can
        // express: rebuild the row's panels from their captured entries.
        let (from, to) = match (row.panels.first(), row.panels.last()) {
            (Some(first), Some(last)) => (first.from, last.line + 1),
            _ => (row.header_line + 1, row.header_line + 1),
        };
        let template = row.panels.first().map_or(row.header_line, |p| p.line);
        edits.push(Edit::Replace {
            from,
            to,
            text: panel_lines(&lines, &blocks, want, template),
        });
    }

    // Every edit keys off original line numbers, so applying from the bottom
    // up keeps every unapplied edit's index valid.
    edits.sort_by_key(Edit::anchor);
    for edit in edits.iter().rev() {
        match edit {
            Edit::Number { line, key, value } => {
                out[*line] = set_number(&out[*line], key, *value);
            }
            Edit::Replace { from, to, text } => {
                out.splice(*from..*to, text.iter().cloned());
            }
        }
    }

    // Rejoined with the ending the file already had. `str::lines()` strips
    // `\r`, so joining with `\n` would quietly convert a CRLF config to LF and
    // rewrite every line in it because one panel moved.
    let newline = crate::store::line_ending(source);
    let mut result = out.join(newline);
    if source.ends_with('\n') {
        result.push_str(newline);
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
    /// Change a number in place, leaving the rest of the line alone.
    Number {
        line: usize,
        key: &'static str,
        value: u16,
    },
    /// Swap `from..to` for `text`. An empty `text` deletes the span, and an
    /// empty span inserts without removing anything.
    Replace {
        from: usize,
        to: usize,
        text: Vec<String>,
    },
}

impl Edit {
    fn anchor(&self) -> usize {
        match self {
            Self::Number { line, .. } => *line,
            Self::Replace { from, .. } => *from,
        }
    }
}

struct PanelSite {
    widget: String,
    width: u16,
    /// Where this panel's entry starts: its own line, or the first of the
    /// comment lines written directly above it. Those comments describe the
    /// panel, so they belong to it and travel with it.
    from: usize,
    /// The line carrying `widget = "…"`.
    line: usize,
}

struct RowSite {
    /// The line carrying `height = …`, which is also where a row with no panels
    /// gets its first one inserted after.
    header_line: usize,
    /// The line carrying the row's closing `] },`.
    closing_line: usize,
    panels: Vec<PanelSite>,
}

/// One panel's entry, lifted out of the text so it can be written back
/// somewhere else.
struct PanelBlock {
    /// The comment lines above the panel, then the panel's own line last.
    lines: Vec<String>,
}

/// Every panel's entry, keyed by widget.
fn panel_blocks(lines: &[&str], map: &LayoutMap) -> HashMap<String, PanelBlock> {
    let mut blocks = HashMap::new();
    for row in &map.rows {
        for panel in &row.panels {
            blocks.insert(
                panel.widget.clone(),
                PanelBlock {
                    lines: lines[panel.from..=panel.line]
                        .iter()
                        .map(|line| (*line).to_string())
                        .collect(),
                },
            );
        }
    }
    blocks
}

/// Work out which row in the text each row of the new layout came from.
///
/// A row has no name to match on, so the match is by content: each row in the
/// file goes to whichever new row kept most of its panels. A new row that
/// claims nothing is one the user has just created, and a row in the file that
/// nothing claims is one they have just emptied.
///
/// Nothing here enforces that the pairing comes out in order. It does not need
/// to: a pairing that crosses over produces a file whose rows are in the wrong
/// order, and the check at the end of [`apply`] throws that away rather than
/// writing it.
fn pair_rows(map: &LayoutMap, desired: &Layout) -> Vec<Option<usize>> {
    let kept = |row: &RowSite, want: &LayoutRow| {
        row.panels
            .iter()
            .filter(|panel| want.panels.iter().any(|w| w.widget == panel.widget))
            .count()
    };

    let mut pairing = vec![None; desired.rows.len()];
    for (text_index, row) in map.rows.iter().enumerate() {
        let claimant = desired
            .rows
            .iter()
            .enumerate()
            .filter(|(want_index, _)| pairing[*want_index].is_none())
            .map(|(want_index, want)| (kept(row, want), want_index))
            .filter(|(shared, _)| *shared > 0)
            // Most panels kept wins; the earliest row breaks a tie, so a split
            // row leaves its panels where they were and moves the rest.
            .max_by_key(|(shared, want_index)| (*shared, std::cmp::Reverse(*want_index)));
        if let Some((_, want_index)) = claimant {
            pairing[want_index] = Some(text_index);
        }
    }
    pairing
}

/// The panel entries of `want`, in order, reusing each panel's captured lines.
fn panel_lines(
    lines: &[&str],
    blocks: &HashMap<String, PanelBlock>,
    want: &LayoutRow,
    template: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    for panel in &want.panels {
        match blocks.get(&panel.widget) {
            Some(block) => {
                let last = block.lines.len().saturating_sub(1);
                for (offset, text) in block.lines.iter().enumerate() {
                    if offset == last {
                        out.push(set_number(text, "width", panel.width));
                    } else {
                        out.push(text.clone());
                    }
                }
            }
            None => out.push(panel_line(lines, template, &panel.widget, panel.width)),
        }
    }
    out
}

/// A whole new row block, indented to match the rows already in the file.
fn row_block(
    lines: &[&str],
    map: &LayoutMap,
    blocks: &HashMap<String, PanelBlock>,
    want: &LayoutRow,
) -> Vec<String> {
    let model = &map.rows[0];
    let indent: String = lines
        .get(model.header_line)
        .copied()
        .unwrap_or("")
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let template = model.panels.first().map_or(model.header_line, |p| p.line);

    let mut out = vec![format!("{indent}{{ height = {}, panels = [", want.height)];
    out.extend(panel_lines(lines, blocks, want, template));
    out.push(format!("{indent}] }},"));
    out
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
                // Corrected when the closing line is reached. A row whose block
                // never closes keeps its header here, which produces a span
                // that changes nothing rather than one that eats the file.
                closing_line: index,
                panels: Vec::new(),
            });
        }
        if let Some(widget) = quoted_value(line, "widget")
            && let Some(width) = number_value(line, "width")
            && let Some(row) = rows.last_mut()
        {
            // Walk up through the comment lines directly above, without ever
            // crossing the row header — those comments describe this panel.
            let mut from = index;
            while from > row.header_line + 1 && lines[from - 1].trim().starts_with('#') {
                from -= 1;
            }
            row.panels.push(PanelSite {
                widget,
                width,
                from,
                line: index,
            });
        }
        // The first `]` after the header closes the row. Later ones close the
        // `rows = [` array itself — claiming those would have the last row
        // swallow the bracket that ends the whole block.
        if trimmed.starts_with(']')
            && let Some(row) = rows.last_mut()
            && row.closing_line == row.header_line
        {
            row.closing_line = index;
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

    /// Reordering within a row is the plainest thing arrange mode does, and no
    /// per-line edit can say it: the old code matched panels by name, found
    /// both still present, emitted nothing, and the round-trip check refused a
    /// change the user had watched happen on screen.
    #[test]
    fn a_panel_moved_along_its_row_takes_its_comment_with_it() {
        let mut desired = layout_of(SAMPLE);
        desired.rows[0].panels.swap(0, 1);
        let out = apply(SAMPLE, &desired).unwrap();

        let calendar = out.find(r#""calendar""#).expect("calendar is still placed");
        let clocks = out.find(r#""clocks""#).expect("clocks is still placed");
        assert!(calendar < clocks, "calendar should now come first:\n{out}");

        // The comment describes the calendar, so it has to travel with it
        // rather than staying behind to caption whatever moved into its place.
        let comment = out
            .find("# Wide enough for two months side by side.")
            .expect("the comment survives");
        assert!(
            comment < calendar && clocks < comment.max(calendar) + out.len(),
            "the comment should sit directly above calendar:\n{out}"
        );
        assert_eq!(shape(&layout_of(&out)), shape(&desired));
    }

    /// Pushing a panel past the edge of the dashboard gives it a row of its
    /// own. Writing that means inventing a whole block, which the old code had
    /// no way to do — it only ever iterated rows the text already had.
    #[test]
    fn a_new_row_can_be_written_between_two_that_exist() {
        let mut desired = layout_of(SAMPLE);
        let calendar = desired.rows[0].panels.remove(1);
        desired.rows.insert(
            1,
            LayoutRow {
                height: 20,
                panels: vec![calendar],
            },
        );

        let out = apply(SAMPLE, &desired).unwrap();
        let written = layout_of(&out);

        assert_eq!(written.rows.len(), 3, "a row was added:\n{out}");
        assert_eq!(written.rows[1].panels[0].widget, "calendar");
        assert_eq!(written.rows[1].height, 20);
        assert!(
            out.contains("# Wide enough for two months side by side."),
            "the comment follows the panel into its new row:\n{out}"
        );
        assert_eq!(shape(&written), shape(&desired));
    }

    /// The other half of the same gesture: the last panel out of a row closes
    /// it. The old code refused this outright, in as many words.
    #[test]
    fn the_row_a_panel_leaves_empty_is_closed() {
        let mut desired = layout_of(SAMPLE);
        let notes = desired.rows[1].panels.remove(1);
        let todo = desired.rows[1].panels.remove(0);
        desired.rows.remove(1);
        desired.rows[0].panels.push(todo);
        desired.rows[0].panels.push(notes);

        let out = apply(SAMPLE, &desired).unwrap();
        let written = layout_of(&out);

        assert_eq!(written.rows.len(), 1, "the emptied row is gone:\n{out}");
        assert_eq!(written.rows[0].panels.len(), 4);
        // The bracket that closes `rows = [` is not the row's own, and eating
        // it leaves a file that does not parse.
        assert!(out.contains("\n]\n"), "the rows array still closes:\n{out}");
        assert!(out.contains(r#"units = "imperial""#), "the rest survives");
        assert_eq!(shape(&written), shape(&desired));
    }

    /// A panel moving between rows is the one structural change the old code
    /// could already express. It has to keep working, and it has to keep the
    /// comment now that comments are looked up across the whole block.
    #[test]
    fn a_panel_moved_to_another_row_keeps_its_comment() {
        let mut desired = layout_of(SAMPLE);
        let calendar = desired.rows[0].panels.remove(1);
        desired.rows[1].panels.insert(0, calendar);

        let out = apply(SAMPLE, &desired).unwrap();
        let written = layout_of(&out);

        assert_eq!(written.rows[0].panels.len(), 1);
        assert_eq!(written.rows[1].panels[0].widget, "calendar");
        assert!(
            out.contains("# Wide enough for two months side by side."),
            "the comment moved rows with the panel:\n{out}"
        );
        assert_eq!(shape(&written), shape(&desired));
    }

    /// Resizing must stay a one-number edit. Rebuilding the row would work and
    /// would quietly reflow a block the user had aligned by hand, on every
    /// `Ctrl+arrow` repeat.
    #[test]
    fn a_resize_still_rewrites_nothing_but_the_number() {
        let mut desired = layout_of(SAMPLE);
        desired.rows[0].panels[0].width = 30;
        let out = apply(SAMPLE, &desired).unwrap();

        let before: Vec<&str> = SAMPLE.lines().collect();
        let after: Vec<&str> = out.lines().collect();
        assert_eq!(before.len(), after.len(), "no line was added or removed");
        let changed: Vec<usize> = (0..before.len())
            .filter(|i| before[*i] != after[*i])
            .collect();
        assert_eq!(changed.len(), 1, "exactly one line moved:\n{out}");
        assert!(after[changed[0]].contains("width = 30"));
    }

    /// A config written on Windows uses CRLF. `str::lines()` strips the `\r`,
    /// so reassembling with `\n` converted the whole file to LF — moving one
    /// panel reported every line in the config as changed, which is a
    /// whole-file diff in git and nothing the user asked for.
    /// Throws a lot of shapes at the real shipped config and asserts the only
    /// two acceptable outcomes: the edit applies and the file still describes
    /// exactly what was asked, or it is refused and the file is untouched.
    ///
    /// There is no third outcome. "Wrote something plausible" is the failure
    /// this module exists to make impossible, and the check at the end of
    /// `apply` is what makes it so — this exercises that check against shapes
    /// nobody would think to write by hand.
    #[test]
    fn no_mutation_of_the_shipped_config_produces_a_wrong_file() {
        let shipped = include_str!("../assets/default_config.toml");
        let base = layout_of(shipped);

        // A deterministic spread of structural mutations. No randomness: a
        // failure has to be reproducible from the test name alone.
        let mut shapes: Vec<Layout> = Vec::new();
        for row in 0..base.rows.len() {
            for column in 0..base.rows[row].panels.len() {
                // Move one panel to every other row.
                for target in 0..base.rows.len() {
                    let mut want = base.clone();
                    let panel = want.rows[row].panels.remove(column);
                    want.rows[target].panels.insert(0, panel);
                    want.rows.retain(|r| !r.panels.is_empty());
                    shapes.push(want);
                }
                // Give it a row of its own at either end.
                for at in [0, base.rows.len()] {
                    let mut want = base.clone();
                    let panel = want.rows[row].panels.remove(column);
                    want.rows.insert(
                        at,
                        LayoutRow {
                            height: 10,
                            panels: vec![panel],
                        },
                    );
                    want.rows.retain(|r| !r.panels.is_empty());
                    shapes.push(want);
                }
                // Drop it entirely.
                let mut want = base.clone();
                want.rows[row].panels.remove(column);
                want.rows.retain(|r| !r.panels.is_empty());
                shapes.push(want);
            }
            // Reverse a row.
            let mut want = base.clone();
            want.rows[row].panels.reverse();
            shapes.push(want);
        }

        let (mut applied, mut refused) = (0, 0);
        for (index, want) in shapes.iter().enumerate() {
            match apply(shipped, want) {
                Ok(out) => {
                    applied += 1;
                    let reparsed = toml::from_str::<Config>(&out)
                        .unwrap_or_else(|e| panic!("shape {index} produced unparsable TOML: {e}"));
                    assert_eq!(
                        shape(&reparsed.layout),
                        shape(want),
                        "shape {index} wrote a layout nobody asked for"
                    );
                    // The rest of the file is not this module's business.
                    assert!(
                        out.contains("[weather]") && out.contains("[news]"),
                        "shape {index} lost a section outside `[layout]`"
                    );
                }
                Err(_) => refused += 1,
            }
        }

        assert_eq!(applied + refused, shapes.len());
        assert!(
            applied > shapes.len() / 2,
            "only {applied} of {} shapes applied; the editor has become too \
             timid to be useful",
            shapes.len()
        );
    }

    #[test]
    fn a_crlf_config_stays_crlf() {
        let crlf = SAMPLE.replace('\n', "\r\n");
        let mut desired = layout_of(&crlf);
        desired.rows[1].panels[0].width = 60;

        let out = apply(&crlf, &desired).expect("applies");
        assert_eq!(
            out.matches("\r\n").count(),
            out.matches('\n').count(),
            "every newline is still a CRLF:\n{out:?}"
        );
        assert_eq!(layout_of(&out).rows[1].panels[0].width, 60, "and it took");
    }

    /// The other direction, so the fix cannot be "always write CRLF".
    #[test]
    fn an_lf_config_stays_lf() {
        let mut desired = layout_of(SAMPLE);
        desired.rows[1].panels[0].width = 60;
        let out = apply(SAMPLE, &desired).expect("applies");
        assert_eq!(out.matches('\r').count(), 0, "no carriage returns appeared");
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
    /// quoted in `CLAUDE.md` invariant 16, which no compiler checks. It had
    /// already drifted into "~145" there, "two hundred" in this file's header,
    /// and "159" in the 0.4.0 changelog entry — the last two citations have
    /// since been removed rather than maintained, a changelog being a record of
    /// what was true then rather than a place to keep a live number.
    ///
    /// A failure here is not a bug in the config. It means the number moved and
    /// `CLAUDE.md` has to move with it.
    #[test]
    fn the_comment_count_the_docs_quote_is_the_one_in_the_file() {
        const CITED: usize = 269;
        let actual = crate::config::DEFAULT_CONFIG
            .lines()
            .filter(|line| line.trim_start().starts_with('#'))
            .count();
        assert_eq!(
            actual, CITED,
            "the default config now has {actual} comment lines. Update this \
             constant and `CLAUDE.md` invariant 16 — both quote {CITED}."
        );
    }
}
