//! The rolling sample history behind the CPU and network graphs.
//!
//! Both panels keep the same shape of buffer under the same rule, and both had
//! their own copy of it. The rule is the part worth naming: the configured
//! history is a **floor, not a ceiling**.

use std::collections::VecDeque;

/// How many samples to retain, given a configured minimum and a graph width.
///
/// The graphs pack two samples into every cell and fill from the right, so a
/// buffer of N samples can only ever cover N/2 cells. On a panel wider than
/// that the graph stops short of its own left edge and the rest is dead space.
/// Letting the buffer grow to the width fixes that; the span readout beside the
/// figure is computed from the live sample count, so it stays honest rather
/// than continuing to claim the configured number.
pub fn capacity(configured: usize, graph_cells: usize) -> usize {
    configured.max(1).max(graph_cells * 2)
}

/// Append to a bounded ring buffer, dropping from the front to make room.
///
/// Trims in a loop rather than popping one sample: the capacity *shrinks* when
/// the panel is narrowed, and one pop per push would leave the buffer oversized
/// for as many ticks as the window lost cells.
///
/// A capacity of zero means "keep one", not "keep none". `buffer.len() >= 0` is
/// always true for a `usize`, so the loop had no way to end: `pop_front` on an
/// empty deque returns `None` without breaking anything, and the call spun for
/// ever. [`capacity`] floors at 1 and both callers go through it, so nothing
/// reached it — but this is a public function with a spin-forever hole in it,
/// and the floor costs one line.
pub fn push_bounded(buffer: &mut VecDeque<u64>, value: u64, capacity: usize) {
    let capacity = capacity.max(1);
    while buffer.len() >= capacity {
        buffer.pop_front();
    }
    buffer.push_back(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_never_exceeds_its_capacity() {
        let mut buffer = VecDeque::new();
        for value in 0..100 {
            push_bounded(&mut buffer, value, 8);
            assert!(buffer.len() <= 8, "grew to {}", buffer.len());
        }
        assert_eq!(buffer.back().copied(), Some(99), "the newest is kept");
        assert_eq!(buffer.front().copied(), Some(92), "the oldest is dropped");
    }

    #[test]
    fn narrowing_the_panel_shrinks_the_buffer_in_one_push() {
        // The reason for the `while` loop. A single `pop_front` would leave the
        // buffer oversized for as many pushes as the panel lost cells, and on a
        // one-second tick that is visible.
        let mut buffer: VecDeque<u64> = (0..200).collect();
        push_bounded(&mut buffer, 999, 10);
        assert_eq!(buffer.len(), 10);
    }

    /// `buffer.len() >= 0` is always true for a `usize`, so this used to spin
    /// for ever rather than keeping nothing. Unreachable through `capacity`,
    /// which floors at 1 — but the function is public and a hang is the one
    /// class of bug this release cycle exists to remove.
    #[test]
    fn a_capacity_of_zero_keeps_one_sample_rather_than_spinning_for_ever() {
        let mut buffer: VecDeque<u64> = VecDeque::new();
        push_bounded(&mut buffer, 1, 0);
        push_bounded(&mut buffer, 2, 0);
        assert_eq!(buffer.len(), 1, "a zero capacity means one, not none");
        assert_eq!(buffer.back().copied(), Some(2), "and it is the newest");
    }

    #[test]
    fn the_configured_history_is_a_floor_not_a_ceiling() {
        // Narrow panel: the configured value wins.
        assert_eq!(capacity(120, 20), 120);
        // Wide panel: the width wins, so the graph reaches its own left edge.
        assert_eq!(capacity(120, 400), 800);
        // A zero history is still a buffer.
        assert_eq!(capacity(0, 0), 1);
    }
}
