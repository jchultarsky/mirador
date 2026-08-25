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

use std::collections::{HashMap, HashSet};

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

    check_editable(&map, &current)?;

    // Every panel's entry, keyed by widget and found across the whole block
    // rather than within one row. That is what lets a panel moved to another
    // row — or to a row that did not exist a moment ago — take the comments
    // written above it along with it.
    let blocks = panel_blocks(&lines, &map);
    let duplicates = duplicated_widgets(&map);
    let pairing = pair_rows(&map, desired);

    let mut out: Vec<String> = lines.iter().map(|line| (*line).to_string()).collect();

    // Rows reordered. Nothing below can express that: every other edit changes
    // a row where it already sits, so a pairing that crosses over would leave
    // the rows in their original order and the check at the end would throw the
    // whole edit away. That is what #100 hit the moment `Shift+↑↓` could move a
    // row — the move worked on screen and could not be saved.
    //
    // Handled by rewriting the rows region as a whole, each row emitted from
    // its captured text so its comments and formatting travel with it. Kept to
    // the case that needs it: any layout whose rows stay in order goes through
    // the per-row edits below, so an ordinary resize still rewrites one number
    // on one line.
    if let Some(reordered) = reorder_rows(&lines, &map, &blocks, &duplicates, &pairing, desired)? {
        let from = map.rows[0].from;
        let to = map.rows[map.rows.len() - 1].closing_line + 1;
        out.splice(from..to, reordered);
        return finish(source, &out, desired);
    }

    // Work back to front so an insertion or deletion cannot shift the line
    // numbers of an edit that has not happened yet.
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
                text: row_block(&lines, &map, &blocks, &duplicates, want)?,
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
            text: panel_lines(&lines, &blocks, &duplicates, want, template)?,
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

    finish(source, &out, desired)
}

/// Rejoin the edited lines and refuse anything that does not say what was asked.
///
/// Both paths through [`apply`] end here, so the round-trip check cannot be
/// skipped by whichever one is taken.
fn finish(source: &str, out: &[String], desired: &Layout) -> Result<String> {
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

/// The whole rows region rewritten in the desired order, or `None` when the
/// rows have not moved and the cheaper per-row edits will do.
///
/// A row that has a counterpart in the text is emitted from its own lines, so
/// its comments, indentation and hand-alignment survive being moved; only its
/// `height` is corrected and its panels rebuilt when their order changed. A row
/// with no counterpart is a new one and is written fresh.
fn reorder_rows(
    lines: &[&str],
    map: &LayoutMap,
    blocks: &HashMap<String, PanelBlock>,
    duplicates: &HashSet<&str>,
    pairing: &[Option<usize>],
    desired: &Layout,
) -> Result<Option<Vec<String>>> {
    let paired: Vec<usize> = pairing.iter().flatten().copied().collect();
    let in_order = paired.windows(2).all(|pair| pair[0] < pair[1]);
    // Rows still in their original relative order: the per-row path handles it,
    // and handles it more gently.
    if in_order {
        return Ok(None);
    }

    let mut out = Vec::new();
    for (want_index, want) in desired.rows.iter().enumerate() {
        let Some(text_index) = pairing[want_index] else {
            out.extend(row_block(lines, map, blocks, duplicates, want)?);
            continue;
        };
        let row = &map.rows[text_index];

        // Reusing a row's lines only works when it has lines to reuse: mirador
        // writes the header, each panel and the closing bracket on their own,
        // in that order. A hand-compacted row — a whole row on one line — does
        // not, and slicing it as though it did read backwards and **panicked**,
        // taking the dashboard down when the move was committed. Found by
        // driving it; a config written that way loads perfectly well.
        //
        // Such a row is refused, not guessed at. Rebuilding it is not the
        // escape it looks like: a panel's captured entry is its own lines, and
        // for a compacted row that entry *is* the whole row, so writing it back
        // inside a fresh block nests the row within itself. Refusing is what
        // this module already promises for text it cannot edit confidently, and
        // it costs the reader a message rather than a mangled config.
        let reusable = row.header_line < row.closing_line
            && row
                .panels
                .iter()
                .all(|p| p.from > row.header_line && p.line < row.closing_line);
        if !reusable {
            bail!(
                "moving a row needs each row written across several lines, as \
                 mirador writes them — `{{ height = … , panels = [`, then one \
                 line per panel, then `] }},`. Row {} is on a single line, so \
                 its panels have no entries of their own to move.",
                text_index + 1
            );
        }

        // Everything above the row's own `height = …` line, which is its
        // comments — the explanation someone wrote for this row.
        out.extend(
            lines[row.from..row.header_line]
                .iter()
                .map(|l| (*l).to_string()),
        );
        out.push(set_number(lines[row.header_line], "height", want.height));

        let unchanged_order = row
            .panels
            .iter()
            .map(|panel| panel.widget.as_str())
            .eq(want.panels.iter().map(|panel| panel.widget.as_str()));

        if unchanged_order {
            // Reuse the panel lines as they stand, fixing only the widths, so a
            // row that merely changed place keeps its alignment.
            let first = row.panels.first().map_or(row.header_line + 1, |p| p.from);
            let mut cursor = first;
            for (panel, wanted) in row.panels.iter().zip(&want.panels) {
                out.extend(lines[cursor..panel.line].iter().map(|l| (*l).to_string()));
                out.push(set_number(lines[panel.line], "width", wanted.width));
                cursor = panel.line + 1;
            }
            out.extend(
                lines[cursor..row.closing_line]
                    .iter()
                    .map(|l| (*l).to_string()),
            );
        } else {
            let template = row.panels.first().map_or(row.header_line, |p| p.line);
            out.extend(panel_lines(lines, blocks, duplicates, want, template)?);
        }
        out.push(lines[row.closing_line].to_string());
    }
    Ok(Some(out))
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
    /// The first line of the row's own comments, or `header_line` when it has
    /// none. Rows are moved whole by #100, and a row's explanation has to travel
    /// with it for the same reason a panel's does.
    from: usize,
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

/// Refuse a `[layout]` block this module cannot edit without guessing.
///
/// Both checks are about the text disagreeing with what was parsed from it,
/// which is the one thing the round-trip check at the end of [`apply`] cannot
/// catch on its own — it compares layouts, and a layout does not carry the
/// formatting or the comments that make the edit worth doing.
fn check_editable(map: &LayoutMap, current: &Config) -> Result<()> {
    if map.rows.len() != current.layout.rows.len() {
        bail!(
            "found {} layout rows in the text but {} when parsed; the `[layout]` \
             block is formatted in a way this cannot edit safely",
            map.rows.len(),
            current.layout.rows.len()
        );
    }
    Ok(())
}

/// Every widget named more than once anywhere in the block.
///
/// A panel's entry is looked up by widget name, so a name used twice has no
/// single entry to be. Rebuilding a row then writes one of the two entries for
/// both panels: the surviving comment captions a panel it does not describe,
/// and the other is gone. The layout still comes out right, so the round-trip
/// check at the end of [`apply`] passes it — **a shape comparison cannot see a
/// lost comment**, and comments surviving is the whole reason this module edits
/// text instead of reserialising.
///
/// Neither the picker (a toggle) nor arrange mode can produce a duplicate, so
/// this is a hand-written config, and refusing is what this module already does
/// when it cannot edit confidently. "Your change did not stick" is recoverable;
/// a deleted sentence someone wrote is not.
///
/// Consulted only where an entry is actually reused — [`panel_lines`] — rather
/// than at the top of [`apply`]. The cheap path rewrites digits on the lines
/// they are already on and never touches an entry, so a plain `Ctrl+arrow`
/// resize is safe even in a file like this, and refusing it would be a wider
/// answer than the problem.
fn duplicated_widgets(map: &LayoutMap) -> HashSet<&str> {
    let mut seen = HashSet::new();
    let mut twice = HashSet::new();
    for panel in map.rows.iter().flat_map(|row| row.panels.iter()) {
        if !seen.insert(panel.widget.as_str()) {
            twice.insert(panel.widget.as_str());
        }
    }
    twice
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
    duplicates: &HashSet<&str>,
    want: &LayoutRow,
    template: usize,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for panel in &want.panels {
        if duplicates.contains(panel.widget.as_str()) {
            bail!(
                "`{}` appears more than once in `[layout]`, so its entry cannot \
                 be told apart from its twin and rewriting the row would caption \
                 one panel with the other's comment. Give each panel a distinct \
                 widget, or move this row by hand",
                panel.widget
            );
        }
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
    Ok(out)
}

/// A whole new row block, indented to match the rows already in the file.
fn row_block(
    lines: &[&str],
    map: &LayoutMap,
    blocks: &HashMap<String, PanelBlock>,
    duplicates: &HashSet<&str>,
    want: &LayoutRow,
) -> Result<Vec<String>> {
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
    out.extend(panel_lines(lines, blocks, duplicates, want, template)?);
    out.push(format!("{indent}] }},"));
    Ok(out)
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
            // The row's own comments, walked up the same way a panel's are —
            // never crossing the row that ended above, so a trailing comment
            // stays with the row it was written under.
            let floor = rows.last().map_or(0, |r: &RowSite| r.closing_line + 1);
            let mut from = index;
            while from > floor && lines[from - 1].trim().starts_with('#') {
                from -= 1;
            }
            rows.push(RowSite {
                from,
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
        // Naming the shape it wants, because the reader is looking at a config
        // that loaded perfectly well and is being told it cannot be written to.
        // `[[layout.rows]]` is valid TOML and mirador parses it happily; this
        // module only knows how to edit the `rows = [ { … } ]` form the shipped
        // config uses, and "no rows found" on a file that visibly has rows reads
        // as a bug rather than as a limitation.
        // The actionable half comes first, because the status bar truncates and
        // the whole of this will not fit on a narrow terminal. The first clause
        // has to be the one worth reading.
        bail!(
            "only the `rows = [ … ]` layout form can be rewritten, not \
             `[[layout.rows]]` sections. Both load; mirador edits the config as \
             text so your comments survive, and it recognises the form that \
             `mirador --print-config` writes"
        );
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
    // `find("")` matches at the cursor every time and consumes nothing, so the
    // loop below would never advance. No caller passes an empty key — all four
    // are literals — but the cost of never having to check that again is one
    // line, and a hang is the one failure a dashboard cannot recover from.
    if key.is_empty() {
        return None;
    }
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

    /// A widget named twice has no entry that is unambiguously its own, so a
    /// rebuild wrote one panel's comment above both and dropped the other's.
    /// The shape still matched, so the round-trip check at the end of `apply`
    /// passed it — a shape comparison cannot see a lost comment, which is why
    /// this needs a check of its own.
    #[test]
    fn a_widget_named_twice_is_refused_rather_than_losing_a_comment() {
        let src = "\
[layout]
rows = [
  { height = 50, panels = [
    # the left one
    { widget = \"todo\",  width = 30 },
    # the right one
    { widget = \"todo\",  width = 40 },
    # notes
    { widget = \"notes\", width = 30 },
  ] },
]
";
        let mut want = layout_of(src);
        // Reorder so the widget set changes and the rebuild path runs.
        want.rows[0].panels.rotate_right(1);

        let err = apply(src, &want).expect_err("a duplicated widget must be refused");
        let message = err.to_string();
        assert!(message.contains("todo"), "name the widget: `{message}`");
        assert!(
            message.contains("more than once"),
            "say what is wrong: `{message}`"
        );
    }

    /// The ordinary case is untouched: one of each is not a duplicate.
    #[test]
    fn a_layout_with_no_repeated_widget_still_edits() {
        let mut want = layout_of(SAMPLE);
        want.rows[0].panels.reverse();
        assert!(apply(SAMPLE, &want).is_ok());
    }

    /// And the refusal is as narrow as the problem. Only a *rebuild* reuses a
    /// panel's captured entry, so a plain resize — every `Ctrl+arrow`, the
    /// common case — is safe even in a file with a widget named twice, and must
    /// keep working. Refusing it would be a wider answer than the question.
    #[test]
    fn a_resize_still_works_even_where_a_rebuild_would_not() {
        let src = "\
[layout]
rows = [
  { height = 50, panels = [
    # the left one
    { widget = \"todo\",  width = 30 },
    # the right one
    { widget = \"todo\",  width = 40 },
  ] },
]
";
        let mut want = layout_of(src);
        want.rows[0].panels[0].width = 55;

        let out = apply(src, &want).expect("a resize must survive a duplicate");
        assert!(out.contains("width = 55"), "the resize did not land: {out}");
        // Both comments still there, and each still above its own panel.
        assert!(out.contains("# the left one") && out.contains("# the right one"));
        assert_eq!(
            out.matches("# the right one").count(),
            1,
            "a comment was duplicated: {out}"
        );
    }

    /// `find("")` matches at the cursor without consuming anything, so the scan
    /// in `after_key` would never advance. Unreachable through its four
    /// callers, which all pass literals — and a hang is the one failure a
    /// dashboard cannot recover from, so it is guarded rather than argued
    /// about. The same reasoning as `samples::push_bounded` at capacity zero.
    #[test]
    fn an_empty_key_terminates_instead_of_spinning() {
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&done);
        std::thread::spawn(move || {
            assert!(after_key("height = 3", "").is_none());
            assert_eq!(set_number("height = 3", "", 9), "height = 3");
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            done.load(std::sync::atomic::Ordering::SeqCst),
            "an empty key never returned; the scan is spinning"
        );
    }

    /// The other axis. `no_mutation_of_the_shipped_config_produces_a_wrong_file`
    /// varies the *desired layout* against one fixed piece of text; this varies
    /// the **text**, which is the half a person actually edits by hand.
    ///
    /// That distinction is the lesson `ical` taught: Phase 1 tested that parser
    /// at scale, never varied its alphabet or its shape, and both crashes in it
    /// were sitting in plain sight the whole time.
    ///
    /// Only two outcomes are acceptable — applied and exactly right, or refused
    /// — and "wrote something plausible" is what this module exists to make
    /// impossible.
    /// Three of these are invalid TOML on purpose and never reach `apply` —
    /// leading zeros in an integer, a table named twice, and a section this
    /// crate does not know.
    const UNPARSABLE_MUTATIONS: usize = 3;

    /// Mutations of the shipped config *text*, which is the half a person edits.
    ///
    /// Normalised to LF first, and that is not decoration. This repository has
    /// no `.gitattributes`, so a Windows checkout writes the file with CRLF —
    /// and then `"crlf"` below, which replaces every `\n`, produces `\r\r\n`
    /// and stops being the mutation it is named after. The asymmetry that hides
    /// this is worth knowing: **rustc normalises CRLF to LF inside string
    /// literals, and `include_str!` does not**, so `SAMPLE` is LF on every
    /// platform while this file is whatever git wrote. Windows CI found it; the
    /// same two mutations pass locally either way.
    fn text_mutations() -> Vec<(&'static str, String)> {
        let shipped = &include_str!("../assets/default_config.toml").replace("\r\n", "\n");
        vec![
            ("shipped", shipped.clone()),
            ("crlf", shipped.replace('\n', "\r\n")),
            ("crlf-mixed", shipped.replacen('\n', "\r\n", 40)),
            ("no-trailing-newline", shipped.trim_end().to_string()),
            ("tabs-for-indent", shipped.replace("  ", "\t")),
            ("no-space-around-eq", shipped.replace(" = ", "=")),
            ("extra-space-around-eq", shipped.replace(" = ", "   =   ")),
            // Alphabet: prose in the places a person writes prose. A comment is
            // the one part of this file mirador did not author.
            (
                "cjk-comments",
                shipped.replace("# ", "# \u{65E5}\u{672C}\u{8A9E} "),
            ),
            ("emoji-comments", shipped.replace("# ", "# \u{1F31E} ")),
            (
                "combining-marks",
                shipped.replace("# ", "# a\u{0301}\u{0301} "),
            ),
            ("rtl-comments", shipped.replace("# ", "# \u{05D0}\u{05D1} ")),
            (
                "quote-in-a-comment",
                shipped.replace("# ", "# it\"s \"quoted\" "),
            ),
            // Numbers at and beyond the edges of the type they parse into.
            ("width-0", shipped.replace("width = 30", "width = 0")),
            (
                "width-65535",
                shipped.replace("width = 30", "width = 65535"),
            ),
            (
                "width-leading-zeros",
                shipped.replace("width = 30", "width = 000030"),
            ),
            // Shape.
            (
                "comment-between-header-and-panels",
                shipped.replace("panels = [\n", "panels = [\n    # stray\n"),
            ),
            (
                "duplicated-layout-header",
                shipped.replace("[layout]", "[layout]\n[layout]"),
            ),
            (
                "no-layout-section",
                shipped.replace("[layout]", "[disabled]"),
            ),
        ]
    }

    /// A spread of layouts to ask each mutated text for.
    fn desired_variants(base: &Layout) -> Vec<Layout> {
        let mut wants = vec![base.clone()];
        if let Some(first) = base.rows.first() {
            let mut wider = base.clone();
            if let Some(panel) = wider.rows[0].panels.first_mut() {
                panel.width = panel.width.saturating_add(7);
            }
            wants.push(wider);

            let mut reversed = base.clone();
            reversed.rows[0].panels.reverse();
            wants.push(reversed);

            let mut taller = base.clone();
            taller.rows[0].height = first.height.saturating_add(3);
            wants.push(taller);

            let mut emptied = base.clone();
            emptied.rows[0].panels.clear();
            emptied.rows.retain(|row| !row.panels.is_empty());
            wants.push(emptied);
        }
        if base.rows.len() > 1 {
            let mut swapped = base.clone();
            swapped.rows.swap(0, 1);
            wants.push(swapped);

            let mut moved = base.clone();
            if !moved.rows[0].panels.is_empty() {
                let panel = moved.rows[0].panels.remove(0);
                moved.rows[1].panels.insert(0, panel);
                moved.rows.retain(|row| !row.panels.is_empty());
                wants.push(moved);
            }
        }
        wants
    }

    /// The other axis. `no_mutation_of_the_shipped_config_produces_a_wrong_file`
    /// varies the *desired layout* against one fixed piece of text; this varies
    /// the **text**, which is the half a person actually edits by hand.
    ///
    /// That distinction is the lesson `ical` taught: Phase 1 tested that parser
    /// at scale, never varied its alphabet or its shape, and both crashes in it
    /// were sitting in plain sight the whole time.
    ///
    /// Only two outcomes are acceptable — applied and exactly right, or refused
    /// — and "wrote something plausible" is what this module exists to make
    /// impossible.
    #[test]
    fn no_mutation_of_the_config_text_produces_a_wrong_file() {
        let mutations = text_mutations();
        let (mut applied, mut refused, mut exercised) = (0usize, 0usize, 0usize);
        for (name, source) in &mutations {
            // A mutation that no longer parses is not this module's problem;
            // `apply` rejects it at the first line.
            let Ok(parsed) = toml::from_str::<Config>(source) else {
                continue;
            };
            exercised += 1;
            for (index, want) in desired_variants(&parsed.layout).iter().enumerate() {
                match apply(source, want) {
                    Err(_) => refused += 1,
                    Ok(text) => {
                        applied += 1;
                        let reparsed = toml::from_str::<Config>(&text).unwrap_or_else(|e| {
                            panic!("`{name}` want {index} produced unparsable TOML: {e}")
                        });
                        assert_eq!(
                            shape(&reparsed.layout),
                            shape(want),
                            "`{name}` want {index} wrote a layout nobody asked for"
                        );
                        assert!(
                            text.contains("[weather]"),
                            "`{name}` want {index} lost a section outside `[layout]`"
                        );
                    }
                }
            }
        }

        // Coverage, asserted exactly rather than assumed: a sweep that quietly
        // stopped parsing its own fixtures would pass while testing nothing.
        assert_eq!(
            exercised,
            mutations.len() - UNPARSABLE_MUTATIONS,
            "{exercised} of {} mutations parsed",
            mutations.len()
        );
        assert!(
            applied > refused,
            "{applied} applied against {refused} refused; the editor has become \
             too timid to be useful"
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
        const CITED: usize = 324;
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

    /// #100: swapping two rows. Before this, `pair_rows` produced a crossed
    /// pairing, every per-row edit wrote its row where it already was, and the
    /// round-trip check threw the whole thing away — so the move worked on
    /// screen and could never be saved.
    #[test]
    fn two_rows_can_swap_places() {
        let mut desired = layout_of(SAMPLE);
        desired.rows.swap(0, 1);

        let edited = apply(SAMPLE, &desired).expect("a row swap must be writable");
        assert_eq!(
            shape(&layout_of(&edited)),
            shape(&desired),
            "the file must describe the swapped layout"
        );
    }

    /// A row's explanation travels with it, which is the whole reason this
    /// module edits text instead of reserialising. The round-trip check cannot
    /// see a lost comment — it compares shapes — so this has to be asserted
    /// directly. Same lesson as the duplicate-widget refusal.
    #[test]
    fn a_moved_row_takes_its_comments_with_it() {
        let mut desired = layout_of(SAMPLE);
        desired.rows.swap(0, 1);

        let edited = apply(SAMPLE, &desired).expect("applies");
        assert!(
            edited.contains("Wide enough for two months side by side."),
            "the calendar's comment was lost:\n{edited}"
        );

        // And it is still attached to the calendar rather than stranded above
        // whatever now sits in that position.
        let lines: Vec<&str> = edited.lines().collect();
        let comment = lines
            .iter()
            .position(|l| l.contains("Wide enough for two months"))
            .expect("comment present");
        assert!(
            lines[comment + 1].contains("calendar"),
            "the comment came away from its panel:\n{edited}"
        );
    }

    /// Heights belong to rows, not to positions. Swapping two rows must carry
    /// each height along, or the reader's proportions silently change.
    #[test]
    fn a_swapped_row_keeps_its_own_height() {
        let mut desired = layout_of(SAMPLE);
        desired.rows.swap(0, 1);

        let edited = apply(SAMPLE, &desired).expect("applies");
        let after = layout_of(&edited);
        assert_eq!(after.rows[0].height, 42, "the todo row kept its 42");
        assert_eq!(after.rows[1].height, 34, "and the clocks row its 34");
    }

    /// A row written entirely on one line has no separate lines to reuse.
    /// Slicing it as though it did read backwards and panicked — found by
    /// driving a move in a real terminal, where it took the dashboard down at
    /// the moment the arrangement was committed. A config in that shape loads
    /// perfectly well, so nothing upstream refuses it.
    #[test]
    fn a_row_written_on_one_line_can_still_be_moved() {
        const COMPACT: &str = r#"[layout]
rows = [
  { height = 25, panels = [ { widget = "clocks", width = 100 } ] },
  { height = 25, panels = [ { widget = "watchlog", width = 100 } ] },
  { height = 25, panels = [ { widget = "notes", width = 100 } ] },
  { height = 25, panels = [ { widget = "cpu", width = 100 } ] },
]
"#;
        let mut desired = layout_of(COMPACT);
        desired.rows.swap(1, 2);

        // The bug was a panic, so reaching an assertion at all is half the test.
        let error =
            apply(COMPACT, &desired).expect_err("a compacted row cannot be moved and must say so");
        let message = format!("{error}");
        assert!(
            message.contains("several lines"),
            "the refusal should name the shape it wants: {message}"
        );
    }

    /// The refusal above must stay as narrow as the problem. A compacted config
    /// still resizes — that path never touches a row's entries — and only the
    /// row *move* is out of reach. Widening the check would take a working
    /// feature away from anyone who tidied their config.
    #[test]
    fn a_compact_config_can_still_be_resized() {
        const COMPACT: &str = r#"[layout]
rows = [
  { height = 25, panels = [ { widget = "clocks", width = 100 } ] },
  { height = 25, panels = [ { widget = "notes", width = 100 } ] },
]
"#;
        let mut desired = layout_of(COMPACT);
        desired.rows[0].height = 40;
        desired.rows[1].height = 10;

        let edited = apply(COMPACT, &desired).expect("a resize must still work");
        assert_eq!(shape(&layout_of(&edited)), shape(&desired));
    }

    /// The reorder path must not steal work from the cheap one. A plain resize
    /// leaves the rows in order, so it still rewrites numbers in place rather
    /// than rebuilding the block — which is what keeps a hand-aligned config
    /// aligned through a held `Ctrl+arrow`.
    #[test]
    fn a_resize_does_not_go_through_the_reorder_path() {
        let mut desired = layout_of(SAMPLE);
        desired.rows[0].panels[0].width = 30;

        let edited = apply(SAMPLE, &desired).expect("applies");
        let before: Vec<&str> = SAMPLE.lines().collect();
        let after: Vec<&str> = edited.lines().collect();
        assert_eq!(before.len(), after.len(), "no lines added or removed");
        let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        assert_eq!(
            changed, 1,
            "a resize should touch exactly one line:\n{edited}"
        );
    }
}
