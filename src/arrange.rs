//! Moving a panel around the grid.
//!
//! Split out from [`crate::app`] because the interesting part is arithmetic on
//! a [`Layout`] and nothing else — no terminal, no panels, no config file — so
//! it can be tested by moving a panel and looking at where it went.
//!
//! Two rules shape all of it. **Widths travel with the panel**, so a panel you
//! sized stays the size you made it rather than inheriting whatever the slot it
//! landed in happened to be. And **the weights always sum to what they summed
//! to before**, so a row you tuned is never quietly rescaled by a move
//! somewhere else on the dashboard.

use crate::config::{Layout, LayoutRow};

/// Which way a panel was asked to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Move the panel at `(row, column)`, returning where it ended up.
///
/// `None` means the move was refused and the layout is untouched — the panel is
/// already against the edge it was pushed at, and has nowhere further to go.
pub fn move_panel(
    layout: &mut Layout,
    row: usize,
    column: usize,
    direction: Direction,
) -> Option<(usize, usize)> {
    if column >= layout.rows.get(row)?.panels.len() {
        return None;
    }
    match direction {
        Direction::Left | Direction::Right => {
            let entry = layout.rows.get_mut(row)?;
            let target = if direction == Direction::Left {
                column.checked_sub(1)?
            } else {
                (column + 1 < entry.panels.len()).then_some(column + 1)?
            };
            // Swapping the whole entry rather than the two widget names is what
            // makes the width travel: swapping names would leave the panel in a
            // slot sized for its neighbour.
            entry.panels.swap(column, target);
            Some((row, target))
        }
        Direction::Up | Direction::Down => {
            vertical(layout, row, column, direction == Direction::Down)
        }
    }
}

/// Move a panel to the row above or below, or off the edge into a row of its
/// own.
fn vertical(layout: &mut Layout, row: usize, column: usize, down: bool) -> Option<(usize, usize)> {
    let alone = layout.rows[row].panels.len() == 1;
    let off_the_end = if down {
        row + 1 >= layout.rows.len()
    } else {
        row == 0
    };

    if off_the_end {
        // A panel that already has a row to itself has nowhere to go: giving it
        // another one would be a move that changes nothing, repeated for as
        // long as the key is held.
        return (!alone).then(|| promote(layout, row, column, down))?;
    }

    let target = if down { row + 1 } else { row - 1 };

    // Land where the panel already looks like it is, rather than at the same
    // index. The rows hold different numbers of panels, so the rightmost of
    // three moved down into a row of four belongs on the right, not at index 2.
    let centre = centre_of(&layout.rows[row], column);
    let at = insertion_point(&layout.rows[target], centre);

    let panel = layout.rows[row].panels.remove(column);
    layout.rows[target].panels.insert(at, panel);

    Some((close_if_empty(layout, row, target), at))
}

/// Give the panel a row of its own at the outer edge.
fn promote(layout: &mut Layout, row: usize, column: usize, below: bool) -> Option<(usize, usize)> {
    let source = layout.rows.get_mut(row)?;
    let panel = source.panels.remove(column);

    // The new row's height comes out of the row the panel left, so the total is
    // unchanged and every other row keeps the share the user gave it.
    let height = source.height.max(2);
    let taken = height / 2;
    source.height = height - taken;

    let at = if below { row + 1 } else { row };
    layout.rows.insert(
        at,
        LayoutRow {
            height: taken.max(1),
            panels: vec![panel],
        },
    );
    Some((at, 0))
}

/// Close `emptied` if the departure left it with nothing, returning where
/// `target` ended up once the rows shifted.
fn close_if_empty(layout: &mut Layout, emptied: usize, target: usize) -> usize {
    if !layout.rows[emptied].panels.is_empty() {
        return target;
    }
    // The height goes to the row that took the panel, rather than being shared
    // out: the panel is still on screen, and it keeps roughly the room it had.
    let freed = layout.rows[emptied].height;
    layout.rows[target].height = layout.rows[target].height.saturating_add(freed);
    layout.rows.remove(emptied);
    if emptied < target { target - 1 } else { target }
}

/// Where the middle of this panel sits across its row, as a fraction.
fn centre_of(row: &LayoutRow, column: usize) -> f64 {
    let weight = |panel: &crate::config::LayoutPanel| f64::from(panel.width.max(1));
    let total: f64 = row.panels.iter().map(weight).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let before: f64 = row.panels.iter().take(column).map(weight).sum();
    let own = row.panels.get(column).map_or(1.0, weight);
    (before + own / 2.0) / total
}

/// The gap between panels nearest to `centre`, as an index to insert at.
fn insertion_point(row: &LayoutRow, centre: f64) -> usize {
    let weight = |panel: &crate::config::LayoutPanel| f64::from(panel.width.max(1));
    let total: f64 = row.panels.iter().map(weight).sum();
    if total <= 0.0 {
        return 0;
    }

    let mut best = 0;
    let mut closest = f64::MAX;
    let mut running = 0.0;
    for index in 0..=row.panels.len() {
        let gap = (running / total - centre).abs();
        if gap < closest {
            closest = gap;
            best = index;
        }
        if let Some(panel) = row.panels.get(index) {
            running += weight(panel);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LayoutPanel;

    fn layout(rows: &[(u16, &[(&str, u16)])]) -> Layout {
        Layout {
            rows: rows
                .iter()
                .map(|(height, panels)| LayoutRow {
                    height: *height,
                    panels: panels
                        .iter()
                        .map(|(widget, width)| LayoutPanel {
                            widget: (*widget).into(),
                            width: *width,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn shape(layout: &Layout) -> Vec<Vec<String>> {
        layout
            .rows
            .iter()
            .map(|row| row.panels.iter().map(|p| p.widget.clone()).collect())
            .collect()
    }

    fn heights(layout: &Layout) -> Vec<u16> {
        layout.rows.iter().map(|row| row.height).collect()
    }

    #[test]
    fn a_panel_swaps_with_its_neighbour_and_keeps_its_width() {
        let mut l = layout(&[(100, &[("clocks", 26), ("calendar", 34), ("weather", 40)])]);
        assert_eq!(move_panel(&mut l, 0, 0, Direction::Right), Some((0, 1)));

        assert_eq!(shape(&l)[0], ["calendar", "clocks", "weather"]);
        // The width travelled. Had the widgets swapped instead of the entries,
        // clocks would be sitting in calendar's 34 and calendar in clocks' 26.
        let clocks = l.rows[0]
            .panels
            .iter()
            .find(|p| p.widget == "clocks")
            .unwrap();
        assert_eq!(clocks.width, 26, "clocks kept the width it was given");
    }

    #[test]
    fn the_ends_of_a_row_refuse_rather_than_wrapping() {
        let mut l = layout(&[(100, &[("clocks", 50), ("weather", 50)])]);
        assert_eq!(move_panel(&mut l, 0, 0, Direction::Left), None);
        assert_eq!(move_panel(&mut l, 0, 1, Direction::Right), None);
        assert_eq!(shape(&l)[0], ["clocks", "weather"], "nothing moved");
    }

    /// The whole reason a move lands by position rather than by index: the
    /// rightmost panel of a narrow row belongs on the right of a wider one.
    #[test]
    fn a_panel_lands_under_where_it_already_was() {
        let mut l = layout(&[
            (50, &[("a", 10), ("b", 10), ("c", 80)]),
            (50, &[("w", 25), ("x", 25), ("y", 25), ("z", 25)]),
        ]);
        // `c` occupies the right 80% of its row, so its middle is at 60%.
        assert_eq!(move_panel(&mut l, 0, 2, Direction::Down), Some((1, 2)));
        assert_eq!(shape(&l)[1], ["w", "x", "c", "y", "z"]);

        // And the leftmost lands leftmost, not at the same index it left.
        let mut l = layout(&[
            (50, &[("a", 10), ("b", 90)]),
            (50, &[("w", 25), ("x", 25), ("y", 25), ("z", 25)]),
        ]);
        assert_eq!(move_panel(&mut l, 0, 0, Direction::Down), Some((1, 0)));
        assert_eq!(shape(&l)[1], ["a", "w", "x", "y", "z"]);
    }

    #[test]
    fn the_last_panel_out_of_a_row_closes_it_and_hands_over_the_height() {
        let mut l = layout(&[(30, &[("clocks", 100)]), (70, &[("todo", 100)])]);
        assert_eq!(move_panel(&mut l, 0, 0, Direction::Down), Some((0, 0)));

        assert_eq!(l.rows.len(), 1, "the emptied row closed");
        assert_eq!(shape(&l)[0], ["clocks", "todo"]);
        assert_eq!(heights(&l), [100], "and its height went to the survivor");
    }

    #[test]
    fn pushing_past_the_edge_opens_a_row_without_inventing_weight() {
        let mut l = layout(&[
            (40, &[("clocks", 50), ("weather", 50)]),
            (60, &[("todo", 100)]),
        ]);
        assert_eq!(move_panel(&mut l, 0, 0, Direction::Up), Some((0, 0)));

        assert_eq!(shape(&l), [vec!["clocks"], vec!["weather"], vec!["todo"]]);
        assert_eq!(heights(&l), [20, 20, 60]);
        assert_eq!(
            heights(&l).iter().sum::<u16>(),
            100,
            "a new row is paid for out of the one it left, not conjured"
        );
    }

    /// Otherwise holding the key at the edge would spawn a row per repeat, each
    /// one a move that changed nothing.
    #[test]
    fn a_panel_that_already_has_its_own_row_will_not_take_another() {
        let mut l = layout(&[(50, &[("clocks", 100)]), (50, &[("todo", 100)])]);
        assert_eq!(move_panel(&mut l, 0, 0, Direction::Up), None);
        assert_eq!(l.rows.len(), 2, "no row was opened");
        assert_eq!(heights(&l), [50, 50], "and no weight was moved");
    }

    #[test]
    fn a_move_never_changes_what_the_weights_add_up_to() {
        let mut l = layout(&[
            (34, &[("clocks", 26), ("calendar", 34), ("weather", 40)]),
            (42, &[("todo", 40), ("agenda", 32), ("notes", 28)]),
            (24, &[("cpu", 50), ("network", 50)]),
        ]);
        let before: u16 = heights(&l).iter().sum();

        for direction in [
            Direction::Down,
            Direction::Down,
            Direction::Up,
            Direction::Right,
            Direction::Up,
        ] {
            move_panel(&mut l, 0, 0, direction);
            assert_eq!(
                heights(&l).iter().sum::<u16>(),
                before,
                "after {direction:?}: {:?}",
                heights(&l)
            );
        }
    }
}
