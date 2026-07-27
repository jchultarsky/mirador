//! The dashboard grid: which panels are placed, and in what proportions.
//!
//! Weights rather than cells, so a layout keeps its proportions at any terminal
//! size. This is also the only part of the config mirador writes back — the
//! panel picker edits it, and [`crate::layout_edit`] applies the change to the
//! file textually so the user's comments survive.

use serde::Deserialize;

/// A grid of rows, each holding one or more side-by-side panels.
///
/// `height` and `width` are relative weights, not absolute cells, so a layout
/// keeps its proportions at any terminal size.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layout {
    pub rows: Vec<LayoutRow>,
}

impl Default for Layout {
    fn default() -> Self {
        // Kept identical to the `[layout]` block in assets/default_config.toml,
        // and `the_rust_default_layout_matches_the_shipped_one` fails if they
        // drift. They are reached by different routes — the shipped file on a
        // true first run, this on any config that omits `[layout]` — and when
        // they disagreed, deleting the section silently cost three panels.
        let row = |height: u16, panels: &[(&str, u16)]| LayoutRow {
            height,
            panels: panels
                .iter()
                .map(|(widget, width)| LayoutPanel {
                    widget: (*widget).into(),
                    width: *width,
                })
                .collect(),
        };

        Self {
            rows: vec![
                row(34, &[("clocks", 26), ("calendar", 34), ("weather", 40)]),
                row(
                    42,
                    &[
                        ("todo", 30),
                        ("agenda", 26),
                        ("notes", 22),
                        ("watchlog", 22),
                    ],
                ),
                row(
                    24,
                    &[
                        ("pomodoro", 28),
                        ("stocks", 32),
                        ("cpu", 18),
                        ("network", 22),
                    ],
                ),
            ],
        }
    }
}

impl Layout {
    /// Every widget this layout places, in row-major order.
    pub fn widgets(&self) -> Vec<&str> {
        self.rows
            .iter()
            .flat_map(|row| row.panels.iter())
            .map(|panel| panel.widget.as_str())
            .collect()
    }

    /// Whether `widget` appears anywhere.
    pub fn places(&self, widget: &str) -> bool {
        self.widgets().contains(&widget)
    }

    /// Place `widget`, appending it to the row carrying the fewest panels.
    ///
    /// A dashboard has no obvious "end" to add to, so the rule is the one that
    /// keeps the grid even: the emptiest row, and the last of those on a tie so
    /// repeated additions walk downward rather than piling into row one. The
    /// new panel takes the average weight of that row's existing panels, which
    /// leaves the others in proportion to each other instead of squeezing one.
    ///
    /// Deliberately crude, because it has to be undoable by hand: this only
    /// ever *adds*, and `Ctrl+arrow` or the config can put it where you want.
    /// Guessing more cleverly would be harder to predict, not easier.
    pub fn add_widget(&mut self, widget: &str) {
        if self.places(widget) {
            return;
        }

        let target = self
            .rows
            .iter()
            .enumerate()
            .min_by_key(|(index, row)| (row.panels.len(), usize::MAX - index))
            .map(|(index, _)| index);

        let Some(index) = target else {
            // No rows at all: give the widget one of its own rather than
            // dropping it silently.
            self.rows.push(LayoutRow {
                height: 100,
                panels: vec![LayoutPanel {
                    widget: widget.to_string(),
                    width: 100,
                }],
            });
            return;
        };

        let row = &mut self.rows[index];
        let width = if row.panels.is_empty() {
            100
        } else {
            let total: u32 = row.panels.iter().map(|p| u32::from(p.width)).sum();
            u16::try_from(total / row.panels.len() as u32)
                .unwrap_or(25)
                .max(1)
        };
        row.panels.push(LayoutPanel {
            widget: widget.to_string(),
            width,
        });
    }

    /// Remove every placement of `widget`, and any row left empty by it.
    ///
    /// Refuses to remove the last panel on the dashboard and says so, because
    /// an empty layout is rejected at startup — turning off the final panel
    /// would produce a config that cannot be loaded next time.
    pub fn remove_widget(&mut self, widget: &str) -> bool {
        if self.widgets().iter().all(|placed| *placed == widget) {
            return false;
        }
        for row in &mut self.rows {
            row.panels.retain(|panel| panel.widget != widget);
        }
        self.rows.retain(|row| !row.panels.is_empty());
        true
    }
}

/// One horizontal band of the dashboard.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutRow {
    /// Relative height weight.
    pub height: u16,
    pub panels: Vec<LayoutPanel>,
}

impl Default for LayoutRow {
    fn default() -> Self {
        Self {
            height: 1,
            panels: Vec::new(),
        }
    }
}

/// One panel within a row.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutPanel {
    /// Widget id: `clocks`, `weather`, `todo`, `cpu` or `network`.
    pub widget: String,
    /// Relative width weight.
    pub width: u16,
}

impl Default for LayoutPanel {
    fn default() -> Self {
        Self {
            widget: String::new(),
            width: 1,
        }
    }
}
