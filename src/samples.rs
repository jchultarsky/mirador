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
pub fn push_bounded(buffer: &mut VecDeque<u64>, value: u64, capacity: usize) {
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
