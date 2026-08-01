//! A small multi-line text editor, for note bodies.
//!
//! [`crate::textfield::TextField`] handles one line and is the right thing for
//! a task title. A note body is prose, so it needs real lines, and the two are
//! different enough that folding them together would make both worse.
//!
//! As in `TextField`, the cursor is a *character* index rather than a byte
//! index, so multi-byte text does not put it between the halves of a glyph.
//! Lines are hard lines: the editor never reflows what you typed, because a
//! soft wrap that moves as you type makes the cursor impossible to follow. The
//! reader wraps; the editor does not.

use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone)]
pub struct TextArea {
    /// Always at least one line, so there is somewhere for the cursor to be.
    lines: Vec<String>,
    /// Cursor line.
    row: usize,
    /// Cursor position within the line, in characters.
    col: usize,
    /// Fixed end of a keyboard selection. The cursor is its moving end.
    selection_anchor: Option<(usize, usize)>,
}

impl Default for TextArea {
    fn default() -> Self {
        Self::new()
    }
}

impl TextArea {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            row: 0,
            col: 0,
            selection_anchor: None,
        }
    }

    /// Load existing text, placing the cursor at the very end — where you want
    /// it when reopening a note to add to it.
    pub fn with_value(value: impl AsRef<str>) -> Self {
        let mut lines: Vec<String> = value.as_ref().split('\n').map(str::to_string).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        let row = lines.len() - 1;
        let col = lines[row].chars().count();
        Self {
            lines,
            row,
            col,
            selection_anchor: None,
        }
    }

    /// The text, with lines joined by `\n`.
    pub fn value(&self) -> String {
        self.lines.join("\n")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Cursor as `(row, column)`, both zero-based and in characters.
    ///
    /// Rendering goes through [`TextArea::visible`], which places the caret in
    /// display cells; this raw accessor is for tests.
    #[cfg(test)]
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    /// Byte offset of character index `col` within `line`.
    fn byte_at(line: &str, col: usize) -> usize {
        line.char_indices()
            .nth(col)
            .map_or(line.len(), |(index, _)| index)
    }

    fn line_len(&self, row: usize) -> usize {
        self.lines.get(row).map_or(0, |l| l.chars().count())
    }

    fn position(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    /// The selection in document order, excluding a collapsed anchor.
    fn selection(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.selection_anchor?;
        let cursor = self.position();
        if anchor == cursor {
            return None;
        }
        Some(if anchor < cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        })
    }

    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    /// Text between the anchor and cursor, preserving hard line breaks.
    pub fn selected_text(&self) -> Option<String> {
        let ((start_row, start_col), (end_row, end_col)) = self.selection()?;
        if start_row == end_row {
            let line = &self.lines[start_row];
            return Some(
                line[Self::byte_at(line, start_col)..Self::byte_at(line, end_col)].to_string(),
            );
        }

        let mut selected = String::new();
        let first = &self.lines[start_row];
        selected.push_str(&first[Self::byte_at(first, start_col)..]);
        selected.push('\n');
        for row in start_row + 1..end_row {
            selected.push_str(&self.lines[row]);
            selected.push('\n');
        }
        let last = &self.lines[end_row];
        selected.push_str(&last[..Self::byte_at(last, end_col)]);
        Some(selected)
    }

    /// Select the complete body. An empty body has nothing to select.
    pub fn select_all(&mut self) {
        let end_row = self.lines.len() - 1;
        let end_col = self.line_len(end_row);
        self.row = end_row;
        self.col = end_col;
        self.selection_anchor = ((end_row, end_col) != (0, 0)).then_some((0, 0));
    }

    /// Delete the selected range and leave the cursor where it began.
    fn delete_selection(&mut self) -> bool {
        let Some(((start_row, start_col), (end_row, end_col))) = self.selection() else {
            self.selection_anchor = None;
            return false;
        };

        if start_row == end_row {
            let line = &mut self.lines[start_row];
            let start = Self::byte_at(line, start_col);
            let end = Self::byte_at(line, end_col);
            line.replace_range(start..end, "");
        } else {
            let first = &self.lines[start_row];
            let mut joined = first[..Self::byte_at(first, start_col)].to_string();
            let last = &self.lines[end_row];
            joined.push_str(&last[Self::byte_at(last, end_col)..]);
            self.lines
                .splice(start_row..=end_row, std::iter::once(joined));
        }

        self.row = start_row;
        self.col = start_col;
        self.selection_anchor = None;
        true
    }

    fn insert_at_cursor(&mut self, c: char) {
        let byte = Self::byte_at(&self.lines[self.row], self.col);
        self.lines[self.row].insert(byte, c);
        self.col += 1;
    }

    pub fn insert(&mut self, c: char) {
        self.delete_selection();
        self.insert_at_cursor(c);
    }

    /// Insert a block at the cursor, replacing the active selection.
    /// Windows and old-Mac line endings become the editor's `\n` hard lines.
    pub fn insert_text(&mut self, text: &str) {
        self.delete_selection();
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut parts = text.split('\n');
        let first = parts.next().unwrap_or_default();
        let rest: Vec<&str> = parts.collect();

        let byte = Self::byte_at(&self.lines[self.row], self.col);
        let tail = self.lines[self.row].split_off(byte);
        self.lines[self.row].push_str(first);
        self.col += first.chars().count();

        if rest.is_empty() {
            self.lines[self.row].push_str(&tail);
            return;
        }

        let start_row = self.row;
        for (offset, part) in rest.iter().enumerate() {
            self.lines
                .insert(start_row + offset + 1, (*part).to_string());
        }
        self.row = start_row + rest.len();
        self.col = rest.last().map_or(0, |part| part.chars().count());
        self.lines[self.row].push_str(&tail);
    }

    /// Split the current line at the cursor.
    pub fn newline(&mut self) {
        self.delete_selection();
        let byte = Self::byte_at(&self.lines[self.row], self.col);
        let tail = self.lines[self.row].split_off(byte);
        self.lines.insert(self.row + 1, tail);
        self.row += 1;
        self.col = 0;
    }

    /// Delete backwards, joining with the previous line at the start of a line.
    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.col > 0 {
            let byte = Self::byte_at(&self.lines[self.row], self.col - 1);
            self.lines[self.row].remove(byte);
            self.col -= 1;
        } else if self.row > 0 {
            let current = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.line_len(self.row);
            self.lines[self.row].push_str(&current);
        }
    }

    /// Delete forwards, pulling the next line up at the end of a line.
    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.col < self.line_len(self.row) {
            let byte = Self::byte_at(&self.lines[self.row], self.col);
            self.lines[self.row].remove(byte);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    fn move_left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.line_len(self.row);
        }
    }

    fn move_right(&mut self) {
        if self.col < self.line_len(self.row) {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    fn move_up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            // Keep the column where it was if the new line is long enough,
            // which is what makes walking up a block of text feel right.
            self.col = self.col.min(self.line_len(self.row));
        }
    }

    fn move_down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = self.col.min(self.line_len(self.row));
        }
    }

    fn move_home(&mut self) {
        self.col = 0;
    }

    fn move_end(&mut self) {
        self.col = self.line_len(self.row);
    }

    fn extend(&mut self, movement: fn(&mut Self)) {
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.position());
        }
        movement(self);
        if self.selection_anchor == Some(self.position()) {
            self.selection_anchor = None;
        }
    }

    pub fn left(&mut self) {
        if let Some((start, _)) = self.selection() {
            (self.row, self.col) = start;
            self.selection_anchor = None;
        } else {
            self.selection_anchor = None;
            self.move_left();
        }
    }

    pub fn right(&mut self) {
        if let Some((_, end)) = self.selection() {
            (self.row, self.col) = end;
            self.selection_anchor = None;
        } else {
            self.selection_anchor = None;
            self.move_right();
        }
    }

    pub fn up(&mut self) {
        self.selection_anchor = None;
        self.move_up();
    }

    pub fn down(&mut self) {
        self.selection_anchor = None;
        self.move_down();
    }

    pub fn home(&mut self) {
        self.selection_anchor = None;
        self.move_home();
    }

    pub fn end(&mut self) {
        self.selection_anchor = None;
        self.move_end();
    }

    /// Handle a key, returning whether it was used.
    ///
    /// Deliberately does not claim Esc or Tab: those belong to the form around
    /// the editor, and swallowing them would trap the user inside the body.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char('a') if ctrl => {
                self.select_all();
                true
            }
            // Alt is excluded as well as Ctrl, which it was not: `Alt+x` typed
            // an `x` into the note. `TextField` had always excluded both, and
            // two editors sitting side by side disagreeing about what a chord
            // means is the kind of difference nobody reports and everybody
            // trips over.
            KeyCode::Char(c) if !ctrl && !alt => {
                self.insert(c);
                true
            }
            KeyCode::Enter => {
                self.newline();
                true
            }
            KeyCode::Backspace => {
                self.backspace();
                true
            }
            KeyCode::Delete => {
                self.delete();
                true
            }
            KeyCode::Left if shift => {
                self.extend(Self::move_left);
                true
            }
            KeyCode::Right if shift => {
                self.extend(Self::move_right);
                true
            }
            KeyCode::Up if shift => {
                self.extend(Self::move_up);
                true
            }
            KeyCode::Down if shift => {
                self.extend(Self::move_down);
                true
            }
            KeyCode::Home if shift => {
                self.extend(Self::move_home);
                true
            }
            KeyCode::End if shift => {
                self.extend(Self::move_end);
                true
            }
            KeyCode::Left => {
                self.left();
                true
            }
            KeyCode::Right => {
                self.right();
                true
            }
            KeyCode::Up => {
                self.up();
                true
            }
            KeyCode::Down => {
                self.down();
                true
            }
            KeyCode::Home => {
                self.home();
                true
            }
            KeyCode::End => {
                self.end();
                true
            }
            _ => false,
        }
    }

    /// The first line to draw so that the cursor stays on screen in a viewport
    /// `height` rows tall.
    pub fn scroll_offset(&self, height: usize) -> usize {
        if height == 0 {
            return 0;
        }
        // Only ever scrolls far enough to bring the cursor back into view, so
        // the text does not jump when the cursor is already visible.
        self.row.saturating_sub(height - 1)
    }

    /// Display cells to skip at the left of *every* line, so the caret stays on
    /// screen in a viewport `width` cells wide.
    ///
    /// Shared across all rows rather than computed per row, or the lines shear
    /// against each other and the block stops reading as a block.
    ///
    /// The editor deliberately does not wrap — a soft wrap that moves as you
    /// type makes the cursor impossible to follow — and for a long time that
    /// meant a long line simply ran off the right-hand edge, taking the caret
    /// with it. You could keep typing; you could not see any of it. Not
    /// wrapping is a decision about *layout*; it was never a decision to stop
    /// showing people what they are writing.
    fn h_offset(&self, width: usize) -> usize {
        if width == 0 {
            return 0;
        }
        let line = self.lines.get(self.row).map_or("", String::as_str);
        let before: usize = line
            .chars()
            .take(self.col)
            .map(crate::grid::char_width)
            .sum();
        // One cell held back for the caret, which is drawn between characters
        // and still has to land somewhere.
        before.saturating_sub(width - 1)
    }

    /// The slice of `row` to draw in a viewport `width` cells wide, and the byte
    /// offset within it where the caret belongs — `None` when the caret is on
    /// another row.
    ///
    /// Measured in cells throughout, per invariant 9: a note written in
    /// Japanese would otherwise scroll by half a glyph at a time.
    #[cfg(test)]
    pub fn visible(&self, row: usize, width: usize) -> (String, Option<usize>) {
        let (text, caret, _) = self.visible_with_selection(row, width);
        (text, caret)
    }

    /// The visible line plus the selected byte range inside that visible text.
    /// Byte offsets are safe to slice because they are recorded only at UTF-8
    /// character boundaries while the string is assembled.
    pub fn visible_with_selection(
        &self,
        row: usize,
        width: usize,
    ) -> (String, Option<usize>, Option<Range<usize>>) {
        if width == 0 {
            return (String::new(), None, None);
        }
        let line = self.lines.get(row).map_or("", String::as_str);
        let skip = self.h_offset(width);
        let here = row == self.row;
        let selection = self.selection();
        // The caret occupies a cell of its own on the row that carries it.
        let budget = if here { width - 1 } else { width };

        let mut out = String::new();
        let mut consumed = 0usize;
        let mut drawn = 0usize;
        let mut caret = None;
        let mut selected_start = None;
        let mut selected_end = 0usize;
        for (index, c) in line.chars().enumerate() {
            if here && index == self.col && caret.is_none() && consumed >= skip {
                caret = Some(out.len());
            }
            let w = crate::grid::char_width(c);
            if consumed >= skip {
                if drawn + w > budget {
                    break;
                }
                let selected = selection
                    .is_some_and(|(start, end)| (row, index) >= start && (row, index) < end);
                let at = out.len();
                out.push(c);
                if selected {
                    if selected_start.is_none() {
                        selected_start = Some(at);
                    }
                    selected_end = out.len();
                }
                drawn += w;
            }
            consumed += w;
        }
        // A cursor sitting past the last character — the common case, since
        // that is where typing leaves it.
        if here && caret.is_none() && self.col >= line.chars().count() {
            caret = Some(out.len());
        }
        // A line break is text too. Represent a selected newline with one
        // reversed cell at the line's end; without it, selecting the break
        // between two lines (or an empty line) looked exactly like selecting
        // nothing even though copy and replacement included `\n`.
        let line_end = (row, line.chars().count());
        let newline_selected = row + 1 < self.lines.len()
            && selection.is_some_and(|(start, end)| start <= line_end && end > line_end);
        let reached_line_end = consumed >= crate::grid::display_width(line);
        if newline_selected && reached_line_end && drawn < budget {
            let at = out.len();
            out.push(' ');
            if selected_start.is_none() {
                selected_start = Some(at);
            }
            selected_end = out.len();
        }
        let selected = selected_start.map(|start| start..selected_end);
        (out, caret, selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(text: &str) -> TextArea {
        TextArea::with_value(text)
    }

    fn press(a: &mut TextArea, code: KeyCode) -> bool {
        a.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn chord(a: &mut TextArea, code: KeyCode, modifiers: KeyModifiers) -> bool {
        a.handle_key(KeyEvent::new(code, modifiers))
    }

    #[test]
    fn a_new_area_has_one_empty_line_to_put_the_cursor_on() {
        let a = TextArea::new();
        assert_eq!(a.lines().len(), 1);
        assert_eq!(a.cursor(), (0, 0));
        assert_eq!(a.value(), "");
    }

    #[test]
    fn loading_text_puts_the_cursor_at_the_end() {
        let a = area("one\ntwo");
        assert_eq!(a.lines(), ["one", "two"]);
        assert_eq!(a.cursor(), (1, 3));
    }

    #[test]
    fn enter_splits_the_line_at_the_cursor() {
        let mut a = area("hello world");
        a.home();
        for _ in 0..5 {
            a.right();
        }
        a.newline();
        assert_eq!(a.value(), "hello\n world");
        assert_eq!(a.cursor(), (1, 0));
    }

    #[test]
    fn backspace_at_the_start_of_a_line_joins_it_to_the_one_above() {
        let mut a = area("one\ntwo");
        a.home();
        assert_eq!(a.cursor(), (1, 0));
        a.backspace();
        assert_eq!(a.value(), "onetwo");
        assert_eq!(a.cursor(), (0, 3), "cursor lands at the join");
    }

    #[test]
    fn backspace_at_the_very_start_does_nothing() {
        let mut a = area("abc");
        a.home();
        a.up();
        a.backspace();
        assert_eq!(a.value(), "abc");
    }

    #[test]
    fn delete_at_the_end_of_a_line_pulls_the_next_one_up() {
        let mut a = area("one\ntwo");
        a.up();
        a.end();
        a.delete();
        assert_eq!(a.value(), "onetwo");
    }

    #[test]
    fn moving_up_keeps_the_column_where_the_line_allows() {
        let mut a = area("longer line\nabc");
        a.end();
        assert_eq!(a.cursor(), (1, 3));
        a.up();
        assert_eq!(a.cursor(), (0, 3), "column is kept when it fits");

        let mut b = area("ab\nlonger line");
        b.end();
        b.up();
        assert_eq!(b.cursor(), (0, 2), "and clamped when it does not");
    }

    #[test]
    fn multibyte_text_is_edited_by_character_not_by_byte() {
        let mut a = area("日本語");
        assert_eq!(a.cursor(), (0, 3));
        a.backspace();
        assert_eq!(a.value(), "日本", "one glyph removed, not one byte");
        a.insert('é');
        assert_eq!(a.value(), "日本é");
        a.left();
        a.insert('x');
        assert_eq!(a.value(), "日本xé");
    }

    #[test]
    fn arrows_cross_line_boundaries_in_both_directions() {
        let mut a = area("ab\ncd");
        a.home();
        a.left();
        assert_eq!(
            a.cursor(),
            (0, 2),
            "left at column 0 wraps to the line above"
        );
        a.right();
        assert_eq!(a.cursor(), (1, 0), "and right wraps back down");
    }

    #[test]
    fn the_editor_leaves_esc_and_tab_for_the_form_around_it() {
        let mut a = TextArea::new();
        assert!(!press(&mut a, KeyCode::Esc), "Esc must not be swallowed");
        assert!(!press(&mut a, KeyCode::Tab), "Tab must not be swallowed");
        assert!(press(&mut a, KeyCode::Char('x')), "but typing is");
    }

    #[test]
    fn ctrl_chords_are_left_alone_rather_than_typed_as_letters() {
        let mut a = TextArea::new();
        assert!(!a.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert_eq!(a.value(), "", "Ctrl+C must not insert a `c`");
    }

    #[test]
    fn shift_navigation_selects_exact_text_in_either_direction() {
        let mut a = area("one\ntwö");
        for _ in 0..3 {
            chord(&mut a, KeyCode::Left, KeyModifiers::SHIFT);
        }
        assert_eq!(a.selected_text().as_deref(), Some("twö"));

        chord(&mut a, KeyCode::Up, KeyModifiers::SHIFT);
        assert_eq!(
            a.selected_text().as_deref(),
            Some("one\ntwö"),
            "a backwards multi-line selection keeps its hard line break"
        );
        assert_eq!(a.cursor(), (0, 0), "the cursor is the moving end");
    }

    #[test]
    fn typing_and_deletion_replace_the_selection() {
        for key in [KeyCode::Char('x'), KeyCode::Backspace, KeyCode::Delete] {
            let mut a = area("red blue");
            for _ in 0..4 {
                chord(&mut a, KeyCode::Left, KeyModifiers::SHIFT);
            }
            press(&mut a, key);
            let want = if matches!(key, KeyCode::Char(_)) {
                "red x"
            } else {
                "red "
            };
            assert_eq!(a.value(), want, "replacement with {key:?}");
            assert!(!a.has_selection());
        }
    }

    #[test]
    fn ctrl_a_selects_the_whole_body_and_paste_replaces_it() {
        let mut a = area("first\nsecond");
        chord(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(a.selected_text().as_deref(), Some("first\nsecond"));

        a.insert_text("new\r\nbody");
        assert_eq!(a.value(), "new\nbody");
        assert_eq!(a.cursor(), (1, 4));
        assert!(!a.has_selection());
    }

    #[test]
    fn the_visible_window_reports_which_bytes_are_selected() {
        let mut a = area("aé日z");
        a.home();
        for _ in 0..3 {
            chord(&mut a, KeyCode::Right, KeyModifiers::SHIFT);
        }
        let (text, caret, selected) = a.visible_with_selection(0, 20);
        assert_eq!(text, "aé日z");
        assert_eq!(caret, Some("aé日".len()));
        assert_eq!(selected, Some(0.."aé日".len()));
    }

    #[test]
    fn selecting_only_a_line_break_is_still_visible() {
        let mut a = area("one\n");
        chord(&mut a, KeyCode::Left, KeyModifiers::SHIFT);
        assert_eq!(a.selected_text().as_deref(), Some("\n"));

        let (text, _, selected) = a.visible_with_selection(0, 20);
        assert_eq!(text, "one ");
        assert_eq!(selected, Some(3..4), "the final cell marks the newline");
    }

    /// The two editors have to agree about this. `TextField` excluded Alt from
    /// the start and this did not, so `Alt+x` typed an `x` into a note body and
    /// did nothing in a task title.
    #[test]
    fn alt_chords_are_left_alone_too_just_as_the_single_line_field_does() {
        let mut a = TextArea::new();
        assert!(!a.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)));
        assert_eq!(a.value(), "", "Alt+x must not insert an `x`");

        let mut f = crate::textfield::TextField::new();
        assert!(!f.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)));
        assert_eq!(f.value(), "", "and the single-line field still agrees");
    }

    /// The bug: the editor does not wrap, so a long line ran off the right and
    /// took the caret with it. You could keep typing and see none of it — the
    /// same complaint as the zone picker's missing scroll, one axis over.
    #[test]
    fn a_long_line_scrolls_sideways_so_the_caret_stays_on_screen() {
        let mut a = TextArea::new();
        for c in "the quick brown fox jumps over the lazy dog".chars() {
            a.insert(c);
        }
        let (text, caret) = a.visible(0, 20);
        let caret = caret.expect("the caret is on this row");
        assert!(
            crate::grid::display_width(&text) < 20,
            "drew {} cells into 20 columns",
            crate::grid::display_width(&text)
        );
        assert!(caret <= text.len(), "caret {caret} outside {text:?}");
        assert!(
            text.ends_with("dog"),
            "the window should be pinned to the caret at the end, got {text:?}"
        );

        // And it comes back as the cursor returns.
        a.home();
        let (text, caret) = a.visible(0, 20);
        assert_eq!(caret, Some(0), "caret back at the left");
        assert!(
            text.starts_with("the quick"),
            "the window followed it back, got {text:?}"
        );
    }

    /// Every row shares one offset. Per-row offsets would shear the block.
    #[test]
    fn the_whole_block_scrolls_together_rather_than_shearing() {
        let long = "x".repeat(60);
        let mut a = TextArea::with_value(format!("{long}\nshort\n{long}"));
        a.end();
        let (top, _) = a.visible(0, 20);
        let (bottom, _) = a.visible(2, 20);
        assert_eq!(
            crate::grid::display_width(&top),
            crate::grid::display_width(&bottom),
            "two identical lines drew different windows"
        );
        // The short line is entirely to the left of the window, so it shows
        // nothing — which is what a horizontally scrolled editor does.
        let (short, _) = a.visible(1, 20);
        assert!(short.is_empty(), "expected an empty window, got {short:?}");
    }

    /// Measured in cells, or a note in Japanese scrolls by half a glyph.
    #[test]
    fn the_sideways_window_is_measured_in_cells_not_characters() {
        for text in [
            "日本語のノートです、とても長い行",
            "aé日b🦀cd",
            "🦀🦀🦀🦀🦀🦀🦀🦀",
        ] {
            let full: Vec<char> = text.chars().collect();
            for len in 0..=full.len() {
                let value: String = full[..len].iter().collect();
                let mut a = TextArea::with_value(value);
                for back in 0..=len {
                    for _ in 0..back {
                        a.left();
                    }
                    for width in 1..10usize {
                        let (window, caret) = a.visible(0, width);
                        assert!(
                            crate::grid::display_width(&window) <= width,
                            "{text:?} len={len} width={width} drew {} cells",
                            crate::grid::display_width(&window)
                        );
                        if let Some(at) = caret {
                            assert!(
                                window.is_char_boundary(at),
                                "caret {at} splits a character in {window:?}"
                            );
                        }
                    }
                    a.end();
                }
            }
        }
    }

    #[test]
    fn a_zero_width_viewport_cannot_panic() {
        let a = TextArea::with_value("anything at all");
        assert_eq!(a.visible(0, 0), (String::new(), None));
        assert_eq!(
            a.visible(99, 10).1,
            None,
            "no caret on a row that is not there"
        );
    }

    #[test]
    fn scrolling_only_moves_once_the_cursor_would_leave_the_viewport() {
        let mut a = area("1\n2\n3\n4\n5");
        // Cursor on the last line of five, viewport three tall.
        assert_eq!(a.scroll_offset(3), 2, "shows lines 3-5");
        a.up();
        a.up();
        assert_eq!(
            a.scroll_offset(3),
            0,
            "cursor is visible again, so no scroll"
        );
        assert_eq!(a.scroll_offset(0), 0, "a zero-height viewport cannot panic");
    }
}
