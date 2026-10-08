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

    /// The same field in upper case, the cursor between the same two letters.
    ///
    /// For a field that is drawn in capitals, so its window can be cut from
    /// what is drawn. Case mapping is not one to one: `ß` becomes `SS`, one
    /// cell becoming two, so a window cut to the typed text's width and then
    /// uppercased can be twice as wide as its field. The cursor is counted
    /// again over the uppercased text before it, for the same reason.
    pub fn uppercased(&self) -> Self {
        let before = &self.value[..byte_at(&self.value, self.cursor)];
        Self {
            value: self.value.to_uppercase(),
            cursor: before.to_uppercase().chars().count(),
        }
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
    /// column; this raw accessor exists for tests, which are its only callers.
    #[cfg(test)]
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

    /// Insert a character at the cursor.
    pub fn insert(&mut self, c: char) {
        let at = byte_at(&self.value, self.cursor);
        self.value.insert(at, c);
        self.cursor += 1;
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let at = byte_at(&self.value, self.cursor - 1);
        self.value.remove(at);
        self.cursor -= 1;
    }

    /// Delete the character at the cursor.
    pub fn delete(&mut self) {
        if self.cursor >= self.len_chars() {
            return;
        }
        let at = byte_at(&self.value, self.cursor);
        self.value.remove(at);
    }

    /// Delete from the cursor back to the start of the previous word.
    pub fn delete_word_before(&mut self) {
        let i = word_start(&self.value, self.cursor);
        let start = byte_at(&self.value, i);
        let end = byte_at(&self.value, self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor = i;
    }

    /// Delete from the cursor to the end of the line.
    pub fn delete_to_end(&mut self) {
        let at = byte_at(&self.value, self.cursor);
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
                let at = byte_at(&self.value, self.cursor);
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
    ///
    /// This is the window for a caret drawn *over* a cell — the terminal's
    /// own cursor, which the task form and the prompts move there — so the
    /// text may fill every cell. A caret drawn as a cell of its own wants
    /// [`TextField::visible_inline`].
    ///
    /// **Both figures are display cells, not characters** — invariant 9, and it
    /// is not pedantry here either. This measured in `chars()` until it was
    /// reviewed: a field 6 columns wide holding `北京市中心` saw five characters,
    /// decided they fit, and returned ten cells of text and a cursor column of
    /// five. The text drew over whatever was beside it and the caret landed in
    /// the middle of a glyph. Everything a reader types into these prompts is a
    /// place name or a path, which is exactly where non-Latin text turns up.
    pub fn visible(&self, width: usize) -> (String, usize) {
        if width == 0 {
            return (String::new(), 0);
        }
        // One cell held back behind the cursor, for the cursor itself when
        // it is past the last character.
        let (_, before, window) = self.window(width - 1, width);
        (window, before)
    }

    /// The window of text to display in a field `width` cells wide with the
    /// caret drawn inline, as a cell of its own between two characters, and
    /// the byte offset in that window where the caret goes. `None` only for
    /// a field with no cells at all, which has no room for a caret.
    ///
    /// The notes title and search line draw their caret this way, and they
    /// used to draw it after whatever [`TextField::visible`] returned: at the
    /// end of the text wherever the cursor was, and once the text filled the
    /// field, in a cell past the edge where nobody saw it. Here the text is
    /// held to one cell less than the field, so the caret always fits, and
    /// its offset is counted in characters from the window's start rather
    /// than recovered from a column — a combining mark before the cursor is
    /// a character and no cells, and a column cannot say which side of it
    /// the caret is on.
    pub fn visible_inline(&self, width: usize) -> (String, Option<usize>) {
        if width == 0 {
            return (String::new(), None);
        }
        let (start, _, window) = self.window(width - 1, width - 1);
        // Every character from `start` to the cursor is in the window: the
        // walk back took no more than `width - 1` cells of them, and the fill
        // forward allows that many.
        let caret = byte_at(&window, self.cursor - start);
        (window, Some(caret))
    }

    /// The text that keeps the cursor in view: the index of its first
    /// character, the cells between it and the cursor, and the text from it
    /// that fits in `fill` cells, having walked back from the cursor no more
    /// than `keep` cells. `keep` must not exceed `fill`, or the cursor could
    /// fall outside what is returned.
    fn window(&self, keep: usize, fill: usize) -> (usize, usize, String) {
        let chars: Vec<char> = self.value.chars().collect();
        let cells = |c: char| crate::grid::char_width(c);

        // Walk back from the cursor while the text still fits in `keep`.
        // Where the old code could subtract indices, this has to accumulate:
        // a step left is worth one cell or two.
        let mut start = self.cursor;
        let mut before = 0usize;
        while start > 0 {
            let w = cells(chars[start - 1]);
            if before + w > keep {
                break;
            }
            before += w;
            start -= 1;
        }

        // Then fill forward, so a window scrolled to the cursor still shows
        // whatever follows it.
        let mut window = String::new();
        let mut drawn = 0usize;
        for &c in &chars[start..] {
            let w = cells(c);
            if drawn + w > fill {
                break;
            }
            window.push(c);
            drawn += w;
        }
        (start, before, window)
    }
}

/// The byte offset of the character at `index` in `text`, or `text.len()`
/// for an index at or past its end — where an insertion at the end goes.
///
/// Both editors keep their cursors in characters and slice their text in
/// bytes, and every edit goes through this to cross between the two. It was
/// written once here and once in [`crate::textarea`], which put the one place
/// a character-for-byte confusion could come in in two places.
pub(crate) fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

/// The character index where the word before `index` starts: back over any
/// whitespace, then back over the word.
///
/// Ctrl+W in both editors deletes from here to the cursor. Each hands it the
/// text the chord may reach — the whole value of a one-line field, and only
/// the cursor's own line in a note body — so neither can take a line break.
pub(crate) fn word_start(text: &str, index: usize) -> usize {
    text[..byte_at(text, index)]
        .trim_end()
        .trim_end_matches(|c: char| !c.is_whitespace())
        .chars()
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Character positions to byte offsets, across characters of one to four
    /// bytes, with the end and anything past it at the length.
    #[test]
    fn a_character_index_lands_on_the_byte_its_character_starts_at() {
        let text = "aé日🌞";
        let starts: Vec<usize> = (0..=6).map(|index| byte_at(text, index)).collect();
        assert_eq!(starts, [0, 1, 3, 6, 10, 10, 10]);
        assert_eq!(byte_at("", 0), 0);
    }

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

    /// Measured in **cells**, which is the property that can actually fail —
    /// the character-counting version of this test passed throughout, because
    /// counting characters was the bug.
    #[test]
    fn visible_never_exceeds_the_requested_width() {
        // Mixed deliberately: one-cell Latin, two-cell CJK, a combining accent
        // that is zero-cell, and an emoji.
        for source in [
            "abcdefghijklmnopqrst",
            "北京市中心の天気予報です",
            "aé日b🦀cd",
            "🦀🦀🦀🦀🦀🦀",
        ] {
            let full: Vec<char> = source.chars().collect();
            for len in 0..=full.len() {
                let value: String = full[..len].iter().collect();
                let mut field = TextField::with_value(value);
                for cursor_moves in 0..=len {
                    for _ in 0..cursor_moves {
                        field.left();
                    }
                    for width in 1..12usize {
                        let (text, col) = field.visible(width);
                        assert!(
                            crate::grid::display_width(&text) <= width,
                            "{source:?} len={len} width={width} drew {} cells",
                            crate::grid::display_width(&text)
                        );
                        assert!(col <= width, "cursor column {col} escaped width {width}");
                    }
                    field.end();
                }
            }
        }
    }

    /// The case the review found. Five characters "fit" a six-column field only
    /// if you count characters; they are ten cells wide.
    #[test]
    fn a_field_of_wide_characters_is_measured_in_cells_not_characters() {
        let field = TextField::with_value("北京市中心");
        let (text, col) = field.visible(6);
        assert!(
            crate::grid::display_width(&text) <= 6,
            "drew {} cells into 6 columns: {text:?}",
            crate::grid::display_width(&text)
        );
        assert!(col <= 6, "caret at column {col} in a 6-column field");
    }

    /// A caret one cell wide must never land between the halves of a glyph that
    /// is two — the column it reports has to be the sum of what precedes it.
    #[test]
    fn the_reported_column_is_where_the_text_before_the_cursor_actually_ends() {
        let mut field = TextField::with_value("日本語");
        field.home();
        field.right(); // after 日
        let (text, col) = field.visible(20);
        assert_eq!(text, "日本語", "it all fits");
        assert_eq!(col, 2, "one wide glyph precedes the caret, so two cells");
    }

    /// An inline caret is a cell of its own, so the window and the caret
    /// together fit the field; it sits exactly between the characters the
    /// cursor is between, counted in characters so that a combining mark
    /// cannot move it; and the window shows as much of what comes before the
    /// cursor as there is room for, rather than scrolling early.
    #[test]
    fn an_inline_caret_fits_and_sits_where_typing_lands() {
        for source in [
            "abcdefghijklmnopqrst",
            "北京市中心の天気予報です",
            "aé日b🦀cd",
            "cafe\u{301} au lait, s'il vous pla\u{ee}t, plai\u{302}t",
            "🦀🦀🦀🦀🦀🦀",
        ] {
            let full: Vec<char> = source.chars().collect();
            for len in 0..=full.len() {
                let value: String = full[..len].iter().collect();
                for cursor in 0..=len {
                    let mut field = TextField::with_value(value.clone());
                    for _ in cursor..len {
                        field.left();
                    }
                    let (before, after) = value.split_at(byte_at(&value, cursor));
                    for width in 1..12usize {
                        let case = format!("{value:?} cursor={cursor} width={width}");
                        let (text, caret) = field.visible_inline(width);
                        let caret = caret.unwrap_or_else(|| panic!("{case}: no caret"));
                        let drawn = crate::grid::display_width(&text);
                        assert!(drawn < width, "{case}: {drawn} cells and a caret");
                        assert!(text.is_char_boundary(caret), "{case}: {text:?} at {caret}");
                        let (shown, rest) = text.split_at(caret);
                        assert!(before.ends_with(shown), "{case}: {shown:?} before");
                        assert!(after.starts_with(rest), "{case}: {rest:?} after");
                        if let Some(next) = before[..before.len() - shown.len()].chars().last() {
                            let room = width - 1 - crate::grid::display_width(shown);
                            assert!(
                                crate::grid::char_width(next) > room,
                                "{case}: {next:?} would have fitted in {room} cells"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn an_inline_caret_has_no_room_in_a_field_of_no_cells() {
        let f = TextField::with_value("abc");
        assert_eq!(f.visible_inline(0), (String::new(), None));
        assert_eq!(f.visible_inline(1), (String::new(), Some(0)));
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
