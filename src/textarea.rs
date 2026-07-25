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

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone)]
pub struct TextArea {
    /// Always at least one line, so there is somewhere for the cursor to be.
    lines: Vec<String>,
    /// Cursor line.
    row: usize,
    /// Cursor position within the line, in characters.
    col: usize,
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
        Self { lines, row, col }
    }

    /// The text, with lines joined by `\n`.
    pub fn value(&self) -> String {
        self.lines.join("\n")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Cursor as `(row, column)`, both zero-based and in characters.
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

    pub fn insert(&mut self, c: char) {
        let byte = Self::byte_at(&self.lines[self.row], self.col);
        self.lines[self.row].insert(byte, c);
        self.col += 1;
    }

    /// Split the current line at the cursor.
    pub fn newline(&mut self) {
        let byte = Self::byte_at(&self.lines[self.row], self.col);
        let tail = self.lines[self.row].split_off(byte);
        self.lines.insert(self.row + 1, tail);
        self.row += 1;
        self.col = 0;
    }

    /// Delete backwards, joining with the previous line at the start of a line.
    pub fn backspace(&mut self) {
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
        if self.col < self.line_len(self.row) {
            let byte = Self::byte_at(&self.lines[self.row], self.col);
            self.lines[self.row].remove(byte);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    pub fn left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.line_len(self.row);
        }
    }

    pub fn right(&mut self) {
        if self.col < self.line_len(self.row) {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    pub fn up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            // Keep the column where it was if the new line is long enough,
            // which is what makes walking up a block of text feel right.
            self.col = self.col.min(self.line_len(self.row));
        }
    }

    pub fn down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = self.col.min(self.line_len(self.row));
        }
    }

    pub fn home(&mut self) {
        self.col = 0;
    }

    pub fn end(&mut self) {
        self.col = self.line_len(self.row);
    }

    /// Handle a key, returning whether it was used.
    ///
    /// Deliberately does not claim Esc or Tab: those belong to the form around
    /// the editor, and swallowing them would trap the user inside the body.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char(c) if !ctrl => {
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
