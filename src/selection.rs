//! Moving a cursor through a list, and working out which row was clicked.
//!
//! Free functions over `ListState` rather than a wrapper type. The three list
//! panels genuinely share these two mechanics, but they disagree about
//! everything around them — todo and notes preserve the selection by id across
//! a refilter where stocks clamps by index, and notes resets its body scroll on
//! every move. Wrapping `ListState` would mean growing hooks for those, which
//! costs more than the duplication it removes.

use ratatui::layout::{Position, Rect};
use ratatui::widgets::ListState;

/// Move the cursor `n` rows down, stopping at the last row.
///
/// Clamps rather than wrapping. A list that jumps back to the top when you hold
/// Down reads as a scroll that lost its place; stopping at the end is what a
/// file manager, a mail client and `less` all do.
///
/// `n` is deliberately allowed to be `usize::MAX`, which is how all three
/// panels implement "go to the end".
pub fn down(state: &mut ListState, n: usize, len: usize) {
    let Some(last) = len.checked_sub(1) else {
        // Empty list: leave the selection alone rather than inventing row 0,
        // which would render a highlight over nothing.
        return;
    };
    let current = state.selected().unwrap_or(0);
    state.select(Some(current.saturating_add(n).min(last)));
}

/// Move the cursor `n` rows up, stopping at the first row.
pub fn up(state: &mut ListState, n: usize, len: usize) {
    if len == 0 {
        return;
    }
    let current = state.selected().unwrap_or(0);
    state.select(Some(current.saturating_sub(n)));
}

/// Which item a click at `at` landed on, if any.
///
/// The `offset` is the first item currently on screen, so the row under the
/// pointer counts from there rather than from the top of the list — otherwise
/// clicking a scrolled list selects the wrong item by exactly the scroll
/// distance. That bug was found and fixed once; keeping the arithmetic in one
/// place is what stops it coming back in the other two panels.
///
/// Returns `None` for a click outside the list, and for the empty space below
/// the last item — which is not a click on the last item.
pub fn row_at(state: &ListState, area: Rect, at: Position, len: usize) -> Option<usize> {
    if !area.contains(at) {
        return None;
    }
    let row = usize::from(at.y.saturating_sub(area.y));
    let index = state.offset() + row;
    (index < len).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(index: usize) -> ListState {
        let mut state = ListState::default();
        state.select(Some(index));
        state
    }

    #[test]
    fn movement_clamps_at_both_ends_rather_than_wrapping() {
        let mut state = at(0);
        up(&mut state, 1, 5);
        assert_eq!(
            state.selected(),
            Some(0),
            "the top does not wrap to the end"
        );

        let mut state = at(4);
        down(&mut state, 1, 5);
        assert_eq!(
            state.selected(),
            Some(4),
            "the end does not wrap to the top"
        );
    }

    #[test]
    fn usize_max_is_how_a_panel_says_first_and_last() {
        let mut state = at(2);
        down(&mut state, usize::MAX, 5);
        assert_eq!(state.selected(), Some(4));
        up(&mut state, usize::MAX, 5);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn an_empty_list_keeps_no_selection() {
        let mut state = ListState::default();
        down(&mut state, 1, 0);
        up(&mut state, 1, 0);
        assert_eq!(
            state.selected(),
            None,
            "a highlight over an empty list has nothing under it"
        );
    }

    #[test]
    fn a_click_on_a_scrolled_list_accounts_for_the_offset() {
        // The bug this exists to prevent: without the offset, clicking the top
        // visible row of a list scrolled down by 10 selects item 0.
        let area = Rect::new(0, 5, 20, 4);
        let mut state = ListState::default();
        *state.offset_mut() = 10;

        assert_eq!(row_at(&state, area, Position::new(3, 5), 40), Some(10));
        assert_eq!(row_at(&state, area, Position::new(3, 7), 40), Some(12));
    }

    #[test]
    fn a_click_outside_the_list_or_past_its_end_selects_nothing() {
        let area = Rect::new(0, 5, 20, 10);
        let state = ListState::default();

        assert_eq!(row_at(&state, area, Position::new(3, 4), 40), None, "above");
        assert_eq!(
            row_at(&state, area, Position::new(30, 6), 40),
            None,
            "right"
        );
        assert_eq!(
            row_at(&state, area, Position::new(3, 9), 3),
            None,
            "the blank space below the last item is not the last item"
        );
    }
}
