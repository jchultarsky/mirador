//! Moving a cursor through a list, working out which row was clicked, and
//! which rows a list shows.
//!
//! Free functions over `ListState` rather than a wrapper type. Four panels
//! move a cursor with [`up`] and [`down`] — tasks, markets, news and the watch
//! log — and three map a click with [`row_at`]: tasks, notes and markets. They
//! share these mechanics and disagree about everything around them — todo and
//! notes preserve the selection by id across a refilter where stocks clamps by
//! index, and notes resets its body scroll on every move. Wrapping `ListState`
//! would mean growing hooks for those, which costs more than the duplication
//! it removes. Two of them draw only the rows on screen, and [`window`] and
//! [`in_order`] are what tasks and notes share for that.

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
///
/// Clamps the *starting* position against `len` as well, which [`down`] has
/// always done and this had not. A selection can be left past the end of a list
/// that has since got shorter — `news` and `watchlog` both count what they last
/// managed to draw, so making the panel shorter does exactly that — and the two
/// directions then disagreed about it. `down` pulled a stale index back into
/// range; `up` walked it one row at a time from wherever it was, so the
/// highlight stayed invisible and the key looked broken until you had pressed
/// it as many times as the list had shrunk.
pub fn up(state: &mut ListState, n: usize, len: usize) {
    let Some(last) = len.checked_sub(1) else {
        return;
    };
    let current = state.selected().unwrap_or(0).min(last);
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

/// The rows of a list `len` long that a `List` `height` rows high shows from
/// a scroll at `offset`, and the selection clamped into the list.
///
/// This is `List`'s own arithmetic for items one row high with no scroll
/// padding, which is every task and note row: the scroll stays put while the
/// selection is in view, moves just far enough to bring it back when it is
/// not, and is never pulled back to fill the rows below a short tail. It is
/// copied rather than called because `List` works it out only once it holds
/// every item, and building every item is the cost this avoids.
/// `the_window_is_the_one_list_would_have_scrolled_to` holds the copy to the
/// original.
pub fn window(
    selected: Option<usize>,
    offset: usize,
    len: usize,
    height: usize,
) -> (std::ops::Range<usize>, Option<usize>) {
    let last = len.saturating_sub(1);
    let selected = selected.map(|at| at.min(last));
    let offset = offset.min(last);
    let first = match selected {
        Some(at) if at < offset => at,
        Some(at) if at >= offset + height => at + 1 - height,
        _ => offset,
    };
    (first..len.min(first + height), selected)
}

/// The items with these ids, in this order, found in one pass over `items`.
///
/// The task and note stores' `get` is a linear scan, so a lookup per row
/// costs a pass each. The view used to be mapped through an index of the
/// whole store built on every frame, which is an allocation in proportion to
/// a store that never drops a completed task. This allocates in proportion
/// to `ids`, which is a screen of rows. Ids are unique — both stores
/// renumber a repeated one as they read the file — so each is found once.
pub fn in_order<'a, T>(items: &'a [T], ids: &[u64], id: impl Fn(&T) -> u64) -> Vec<&'a T> {
    let mut wanted: Vec<(u64, usize)> = ids.iter().copied().zip(0..).collect();
    wanted.sort_unstable();
    let mut found: Vec<Option<&T>> = vec![None; ids.len()];
    for item in items {
        if let Ok(at) = wanted.binary_search_by_key(&id(item), |&(id, _)| id) {
            found[wanted[at].1] = Some(item);
        }
    }
    found.into_iter().flatten().collect()
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

    /// A list that gets shorter can leave the selection past its end — `news`
    /// and `watchlog` both bound movement by what they last drew, so shrinking
    /// the panel does it. Both directions have to pull it back, and `up` did
    /// not: it stepped down from the stale index one row at a time, with
    /// nothing highlighted the whole way.
    #[test]
    fn both_directions_pull_a_selection_left_past_the_end_back_into_range() {
        let mut state = at(30);
        down(&mut state, 1, 5);
        assert_eq!(state.selected(), Some(4), "down clamps into the new list");

        let mut state = at(30);
        up(&mut state, 1, 5);
        assert_eq!(
            state.selected(),
            Some(3),
            "up must land one above the last row, not one below a row that is gone"
        );

        let mut state = at(30);
        up(&mut state, usize::MAX, 5);
        assert_eq!(
            state.selected(),
            Some(0),
            "and `first` still goes to the top"
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

    /// The list builds only the rows it can show, so the window has to be
    /// worked out before `List` sees any of them — which means copying the
    /// arithmetic `List` uses. Every length, height, scroll and selection
    /// small enough to enumerate, against `List` itself: the scroll it
    /// leaves, the selection it settles on, and the rows it draws.
    #[test]
    fn the_window_is_the_one_list_would_have_scrolled_to() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        for len in 1..=8usize {
            for height in 1..=5u16 {
                for offset in 0..=10 {
                    for selected in std::iter::once(None).chain((0..=9).map(Some)) {
                        let list = ratatui::widgets::List::new(
                            (0..len).map(|i| ratatui::widgets::ListItem::new(i.to_string())),
                        );
                        let mut state = ListState::default()
                            .with_offset(offset)
                            .with_selected(selected);
                        let mut terminal = Terminal::new(TestBackend::new(4, height)).unwrap();
                        terminal
                            .draw(|frame| {
                                frame.render_stateful_widget(list, frame.area(), &mut state);
                            })
                            .unwrap();
                        let buffer = terminal.backend().buffer();
                        let drawn: Vec<usize> = (0..height)
                            .filter_map(|y| buffer[(0, y)].symbol().parse().ok())
                            .collect();

                        let case =
                            format!("{len} long, {height} high, from {offset} with {selected:?}");
                        let (shown, chosen) = window(selected, offset, len, usize::from(height));
                        assert_eq!(shown.start, state.offset(), "the scroll: {case}");
                        assert_eq!(chosen, state.selected(), "the selection: {case}");
                        assert_eq!(drawn, shown.collect::<Vec<_>>(), "the rows: {case}");
                    }
                }
            }
        }
    }
}
