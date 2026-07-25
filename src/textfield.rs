//! A minimal single-line text input.
//!
//! The cursor is tracked as a *character* index, not a byte index, so multi-byte
//! input (accents, CJK, emoji) cannot split a character or panic on a slice.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A single-line editable string with a cursor.
#[derive(Debug, Clone, Default)]
pub struct TextField {
    value: String,
    /// Cursor position in characters, in `0..=len_chars()`.
    cursor: usize,
}

impl TextField {
    /// An empty field.
    pub fn new() -> Self {
        Self::default()
    }

    /// A field pre-filled with `value`, cursor at the end.
    pub fn with_value(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Self { value, cursor }
    }

    /// The current text.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The current text, trimmed.
    pub fn trimmed(&self) -> &str {
        self.value.trim()
    }

    /// True when the field holds nothing but whitespace.
    pub fn is_blank(&self) -> bool {
        self.value.trim().is_empty()
    }

    /// Cursor position in characters.
    ///
    /// Rendering uses [`TextField::visible`], which returns a scroll-adjusted
    /// column; this raw accessor exists for tests and for callers that lay the
    /// field out themselves.
    #[allow(dead_code)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Length in characters.
    pub fn len_chars(&self) -> usize {
        self.value.chars().count()
    }

    /// Empty the field.
    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// Byte offset of character index `idx`.
    fn byte_at(&self, idx: usize) -> usize {
        self.value
            .char_indices()
            .nth(idx)
            .map_or(self.value.len(), |(b, _)| b)
    }

    /// Insert a character at the cursor.
    pub fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.value.insert(at, c);
        self.cursor += 1;
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let at = self.byte_at(self.cursor - 1);
        self.value.remove(at);
        self.cursor -= 1;
    }

    /// Delete the character at the cursor.
    pub fn delete(&mut self) {
        if self.cursor >= self.len_chars() {
            return;
        }
        let at = self.byte_at(self.cursor);
        self.value.remove(at);
    }

    /// Delete from the cursor back to the start of the previous word.
    pub fn delete_word_before(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        let start = self.byte_at(i);
        let end = self.byte_at(self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor = i;
    }

    /// Delete from the cursor to the end of the line.
    pub fn delete_to_end(&mut self) {
        let at = self.byte_at(self.cursor);
        self.value.truncate(at);
    }

    /// Move the cursor one character left.
    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the cursor one character right.
    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len_chars());
    }

    /// Move the cursor to the start.
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Move the cursor to the end.
    pub fn end(&mut self) {
        self.cursor = self.len_chars();
    }

    /// Apply a key event. Returns true if the key was used.
    ///
    /// Navigation and editing keys only; the caller owns Enter, Esc and Tab so
    /// that form-level semantics stay in one place.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('a') if ctrl => self.home(),
            KeyCode::Char('e') if ctrl => self.end(),
            KeyCode::Char('u') if ctrl => {
                let at = self.byte_at(self.cursor);
                self.value.replace_range(..at, "");
                self.cursor = 0;
            }
            KeyCode::Char('k') if ctrl => self.delete_to_end(),
            KeyCode::Char('w') if ctrl => self.delete_word_before(),
            // A plain character, or one with only Shift held.
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.insert(c);
            }
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.home(),
            KeyCode::End => self.end(),
            _ => return false,
        }
        true
    }

    /// The window of text to display in a field `width` columns wide, plus the
    /// cursor's column within that window. Scrolls horizontally to keep the
    /// cursor visible in a field narrower than its contents.
    pub fn visible(&self, width: usize) -> (String, usize) {
        if width == 0 {
            return (String::new(), 0);
        }
        let chars: Vec<char> = self.value.chars().collect();
        if chars.len() < width {
            return (self.value.clone(), self.cursor);
        }
        // Keep the cursor inside the window, reserving one column for it.
        let start = (self.cursor + 1).saturating_sub(width);
        let end = (start + width).min(chars.len());
        let window: String = chars[start..end].iter().collect();
        (window, self.cursor - start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn typing_inserts_at_the_cursor() {
        let mut f = TextField::new();
        for c in "abc".chars() {
            f.insert(c);
        }
        f.left();
        f.insert('X');
        assert_eq!(f.value(), "abXc");
        assert_eq!(f.cursor(), 3);
    }

    #[test]
    fn with_value_puts_the_cursor_at_the_end() {
        let f = TextField::with_value("hello");
        assert_eq!(f.cursor(), 5);
    }

    #[test]
    fn backspace_at_the_start_is_a_no_op() {
        let mut f = TextField::with_value("ab");
        f.home();
        f.backspace();
        assert_eq!(f.value(), "ab");
        assert_eq!(f.cursor(), 0);
    }

    #[test]
    fn delete_at_the_end_is_a_no_op() {
        let mut f = TextField::with_value("ab");
        f.delete();
        assert_eq!(f.value(), "ab");
    }

    #[test]
    fn multibyte_characters_never_split() {
        let mut f = TextField::with_value("héllo wörld");
        f.home();
        f.right();
        f.delete(); // removes 'é'
        assert_eq!(f.value(), "hllo wörld");

        let mut g = TextField::with_value("日本語");
        g.backspace();
        assert_eq!(g.value(), "日本");
        assert_eq!(g.cursor(), 2);
    }

    #[test]
    fn emoji_are_handled_as_single_units() {
        let mut f = TextField::new();
        f.insert('🦀');
        f.insert('!');
        assert_eq!(f.len_chars(), 2);
        f.backspace();
        f.backspace();
        assert_eq!(f.value(), "");
    }

    #[test]
    fn cursor_cannot_escape_the_value() {
        let mut f = TextField::with_value("ab");
        for _ in 0..10 {
            f.right();
        }
        assert_eq!(f.cursor(), 2);
        for _ in 0..10 {
            f.left();
        }
        assert_eq!(f.cursor(), 0);
    }

    #[test]
    fn ctrl_w_deletes_the_previous_word() {
        let mut f = TextField::with_value("one two three");
        f.handle_key(ctrl('w'));
        assert_eq!(f.value(), "one two ");
        f.handle_key(ctrl('w'));
        assert_eq!(f.value(), "one ");
    }

    #[test]
    fn ctrl_u_clears_to_the_start_and_ctrl_k_to_the_end() {
        let mut f = TextField::with_value("hello world");
        f.home();
        for _ in 0..6 {
            f.right();
        }
        f.handle_key(ctrl('k'));
        assert_eq!(f.value(), "hello ");
        f.handle_key(ctrl('u'));
        assert_eq!(f.value(), "");
    }

    #[test]
    fn ctrl_a_and_ctrl_e_jump_to_the_ends() {
        let mut f = TextField::with_value("hello");
        f.handle_key(ctrl('a'));
        assert_eq!(f.cursor(), 0);
        f.handle_key(ctrl('e'));
        assert_eq!(f.cursor(), 5);
    }

    #[test]
    fn control_chars_do_not_leak_into_the_value() {
        let mut f = TextField::new();
        // Ctrl+A is a movement, not the letter "a".
        f.handle_key(ctrl('a'));
        assert_eq!(f.value(), "");
    }

    #[test]
    fn unhandled_keys_are_reported_as_unused() {
        let mut f = TextField::new();
        assert!(!f.handle_key(key(KeyCode::Enter)));
        assert!(!f.handle_key(key(KeyCode::Tab)));
        assert!(!f.handle_key(key(KeyCode::Esc)));
        assert!(f.handle_key(key(KeyCode::Char('x'))));
    }

    #[test]
    fn visible_window_scrolls_to_follow_the_cursor() {
        // Cursor at the end: the window shows the tail, and the cursor sits in
        // the last column, so the text itself is one char shorter than `width`.
        let f = TextField::with_value("abcdefghij");
        let (text, col) = f.visible(5);
        assert_eq!(text, "ghij");
        assert_eq!(col, 4);

        // Cursor in the middle: a full window, cursor still inside it.
        let mut mid = TextField::with_value("abcdefghij");
        for _ in 0..5 {
            mid.left();
        }
        let (text, col) = mid.visible(5);
        assert_eq!(text.chars().count(), 5);
        assert!(col < 5);
    }

    #[test]
    fn visible_never_exceeds_the_requested_width() {
        for len in 0..20usize {
            let value: String = "abcdefghijklmnopqrst".chars().take(len).collect();
            let mut field = TextField::with_value(value);
            for cursor_moves in 0..=len {
                for _ in 0..cursor_moves {
                    field.left();
                }
                for width in 1..8usize {
                    let (text, col) = field.visible(width);
                    assert!(
                        text.chars().count() <= width,
                        "len={len} width={width} produced {} chars",
                        text.chars().count()
                    );
                    assert!(col <= width, "cursor column {col} escaped width {width}");
                }
                field.end();
            }
        }
    }

    #[test]
    fn visible_returns_the_whole_value_when_it_fits() {
        let short = TextField::with_value("ab");
        assert_eq!(short.visible(10), ("ab".to_string(), 2));
    }

    #[test]
    fn visible_handles_zero_width() {
        let f = TextField::with_value("abc");
        assert_eq!(f.visible(0), (String::new(), 0));
    }
}
