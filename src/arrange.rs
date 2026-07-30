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

/// Move the whole row at `row` one place up or down, returning where it landed.
///
/// `None` means the move was refused: the row is already at the edge it was
/// pushed at.
///
/// **The gap this fills (#100).** `move_panel` moves panels, and a new row is
/// only ever created by promoting one off the top or bottom edge. So a panel
/// alone in a *middle* row could never travel — `Down` merged it into the row
/// below and `close_if_empty` deleted the row it left, dropping the row count.
/// Going from `[clocks] [watchlog] [notes] [cpu]` to
/// `[clocks] [notes] [watchlog] [cpu]` was not expressible with any sequence of
/// keys. Found by the owner rearranging a tall vertical monitor, whose first
/// reading was that four rows was a cap; it is not, and nothing limits the
/// count.
///
/// This is additive rather than a change to what the arrows already do. Merging
/// a panel into a neighbouring row stays exactly as it was — the issue calls
/// that "the right reading of *move this panel into that row*" — and moving a
/// row is a separate gesture on a separate key.
///
/// Swapping whole [`LayoutRow`] entries is what carries each row's `height`
/// with it. Swapping only the panel lists would leave every row's height where
/// it was and silently resize both, which is the failure
/// `a_move_never_changes_what_the_weights_add_up_to` was rewritten to catch:
/// what matters is not the total but that an untouched row keeps its *share*.
pub fn move_row(layout: &mut Layout, row: usize, down: bool) -> Option<usize> {
    if row >= layout.rows.len() {
        return None;
    }
    let target = if down {
        (row + 1 < layout.rows.len()).then_some(row + 1)?
    } else {
        row.checked_sub(1)?
    };
    layout.rows.swap(row, target);
    Some(target)
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
    // Splitting a weight of one in two needs finer granularity than the layout
    // currently has. Scale every row rather than inventing weight for this one:
    // doubling preserves every proportion exactly, which is what the rule at the
    // top of this module is actually about.
    //
    // It used to read `source.height.max(2)`, which for a row of height 1 handed
    // out two rows of 1 and grew the total by one. A `[[layout.rows]]` written
    // without a `height` gets `LayoutRow::default()`, which is height 1, and
    // nothing validates heights — so with rows of `[1, 1]` a promotion turned a
    // 50/50 dashboard into 33/33/33 and rescaled a row the user had not touched.
    // `a_move_never_changes_what_the_weights_add_up_to` could not see it: every
    // height in that fixture is comfortably above two.
    if layout.rows.get(row).is_some_and(|entry| entry.height == 1) {
        for entry in &mut layout.rows {
            entry.height = entry.height.saturating_mul(2);
        }
    }

    let source = layout.rows.get_mut(row)?;
    let panel = source.panels.remove(column);

    // The new row's height comes out of the row the panel left, so the total is
    // unchanged and every other row keeps the share the user gave it.
    //
    // A source of height 0 gives 0, which is right rather than a special case: a
    // row with no weight draws nothing, and a panel promoted out of one has not
    // asked to start being visible.
    let height = source.height;
    let taken = height / 2;
    source.height = height - taken;

    let at = if below { row + 1 } else { row };
    layout.rows.insert(
        at,
        LayoutRow {
            height: taken,
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

    /// The rule at the top of this module, stated as the property it is really
    /// about: an untouched row keeps its *share* of the dashboard.
    ///
    /// The sum-based test below cannot see the case this exists for. Every
    /// height in its fixture is comfortably above two, and the bug only showed
    /// up at one: `source.height.max(2)` turned a row of height 1 into two rows
    /// of 1, so `[1, 1]` became `[1, 1, 1]` and a row nobody had moved went from
    /// half the screen to a third of it.
    #[test]
    fn promoting_out_of_a_thin_row_does_not_rescale_the_rows_around_it() {
        for height in [0u16, 1, 2, 3, 7, 100] {
            let mut l = layout(&[
                (height, &[("clocks", 50), ("weather", 50)]),
                (50, &[("todo", 100)]),
            ]);
            let before: u16 = heights(&l).iter().sum();
            let untouched_share = f64::from(50) / f64::from(before.max(1));

            assert_eq!(
                move_panel(&mut l, 0, 0, Direction::Up),
                Some((0, 0)),
                "height {height}: the move itself must still work"
            );

            let after: u16 = heights(&l).iter().sum();
            let todo = *heights(&l).last().expect("the untouched row");
            let share = f64::from(todo) / f64::from(after.max(1));
            assert!(
                (share - untouched_share).abs() < 1e-9,
                "height {height}: the untouched row went from {:.4} of the screen \
                 to {:.4} — heights {:?}",
                untouched_share,
                share,
                heights(&l)
            );
        }
    }

    /// `validate` refuses a layout with no rows, and a row with no panels. A
    /// gesture the user can hold down must not be able to produce either.
    #[test]
    fn no_sequence_of_moves_can_produce_a_layout_the_config_would_reject() {
        let start = layout(&[
            (1, &[("clocks", 26), ("calendar", 74)]),
            (1, &[("todo", 100)]),
            (2, &[("cpu", 50), ("network", 50)]),
        ]);

        // Every direction, from every position, repeatedly — which is what
        // holding the key down amounts to.
        let mut l = start;
        for step in 0..200usize {
            let direction = match step % 4 {
                0 => Direction::Down,
                1 => Direction::Right,
                2 => Direction::Up,
                _ => Direction::Left,
            };
            let row = step % l.rows.len().max(1);
            let column = step % l.rows[row].panels.len().max(1);
            move_panel(&mut l, row, column, direction);

            assert!(!l.rows.is_empty(), "step {step}: the layout emptied");
            for entry in &l.rows {
                assert!(
                    !entry.panels.is_empty(),
                    "step {step}: an empty row survived: {:?}",
                    shape(&l)
                );
            }
            let mut placed: Vec<&str> = l.widgets();
            placed.sort_unstable();
            assert_eq!(
                placed,
                ["calendar", "clocks", "cpu", "network", "todo"],
                "step {step}: a panel was lost or duplicated"
            );
        }
    }

    /// The one panel on the dashboard has nowhere to go in any direction, and
    /// must not be able to leave the layout empty by trying.
    #[test]
    fn the_only_panel_on_the_dashboard_refuses_every_direction() {
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            let mut l = layout(&[(100, &[("clocks", 100)])]);
            assert_eq!(move_panel(&mut l, 0, 0, direction), None, "{direction:?}");
            assert_eq!(shape(&l), [["clocks"]], "{direction:?} moved something");
        }
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

    /// The move #100 was filed for: a panel alone in a middle row travelling
    /// down past its neighbour, which no sequence of keys could express.
    #[test]
    fn a_row_can_travel_past_its_neighbour() {
        let mut l = layout(&[
            (10, &[("clocks", 10)]),
            (10, &[("watchlog", 10)]),
            (10, &[("notes", 10)]),
            (10, &[("cpu", 10)]),
        ]);

        assert_eq!(move_row(&mut l, 1, true), Some(2));
        assert_eq!(
            shape(&l),
            vec![vec!["clocks"], vec!["notes"], vec!["watchlog"], vec!["cpu"]],
            "the watch log should have swapped with the notes row"
        );
        assert_eq!(l.rows.len(), 4, "moving a row must not change the count");
    }

    /// A row carries its own height. Swapping the panel lists alone would leave
    /// the heights behind and silently resize both rows — and the reader who
    /// tuned them never asked for that.
    #[test]
    fn a_row_takes_its_height_with_it() {
        let mut l = layout(&[
            (30, &[("clocks", 10)]),
            (10, &[("watchlog", 10)]),
            (60, &[("notes", 10)]),
        ]);

        move_row(&mut l, 0, true);
        assert_eq!(
            heights(&l),
            vec![10, 30, 60],
            "each row's height should have travelled with it"
        );
    }

    /// Both refusals, and neither may quietly do something else instead.
    #[test]
    fn a_row_at_the_edge_has_nowhere_to_go() {
        let mut l = layout(&[(10, &[("clocks", 10)]), (10, &[("notes", 10)])]);
        let before = shape(&l);

        assert_eq!(move_row(&mut l, 0, false), None, "the top row cannot rise");
        assert_eq!(move_row(&mut l, 1, true), None, "the last cannot sink");
        assert_eq!(
            move_row(&mut l, 9, true),
            None,
            "nor can a row that is not there"
        );
        assert_eq!(shape(&l), before, "a refused move must change nothing");
    }

    /// Moving a row moves everything in it, not just the panel the cursor is
    /// on. That is what makes it a *row* move rather than a second way to move
    /// a panel.
    #[test]
    fn a_shared_row_travels_whole() {
        let mut l = layout(&[
            (10, &[("clocks", 5), ("weather", 5)]),
            (10, &[("notes", 10)]),
        ]);

        assert_eq!(move_row(&mut l, 0, true), Some(1));
        assert_eq!(
            shape(&l),
            vec![vec!["notes"], vec!["clocks", "weather"]],
            "both panels should have moved together"
        );
    }
}
