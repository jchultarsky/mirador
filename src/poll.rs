//! The sleep between two rounds of a background fetch.
//!
//! Only the wait is shared. Five panels run a loop on a thread of their own —
//! weather, stocks and news fetch over the network, the agenda re-reads a
//! file, and disk re-reads its volumes — and from a distance the loops look
//! alike: a thread, an `Arc<Mutex<_>>` of results, an `AtomicBool` to stop it.
//! Past the skeleton they agree on very little. Stocks reads its *work list*
//! from the shared request and staggers its symbols; weather carries a
//! resolved location across rounds; news interleaves its feeds; the agenda
//! rebuilds its window from today on every pass; and disk keeps one `sysinfo`
//! handle alive because its counters are deltas since that handle's last
//! refresh. A shared loop would take all of that as parameters, at which point
//! the parameter list is the abstraction.
//!
//! The wait is different. It is the same few lines in all five, and the
//! mistake it is easy to make — checking the stop flag after the sleep instead
//! of before, or not at all — costs the user a hang on quit rather than a
//! wrong number on screen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How long to sleep before looking at the stop and wake flags again.
///
/// Short enough that quitting and a manual refresh both feel immediate, long
/// enough not to defeat a laptop's timer coalescing. The two loops used 500ms
/// and 250ms for no reason either of them recorded.
const SLICE: Duration = Duration::from_millis(250);

/// Why the wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// The interval elapsed, or someone asked for a refresh. Poll again.
    Poll,
    /// The panel is going away. Return from the loop without polling.
    Stop,
}

/// Sleep for `interval`, waking early if `stop` is set or `wake` returns true.
///
/// Sliced rather than a single `sleep`, so a manual refresh does not wait out
/// the full interval and quitting does not wait out anything.
///
/// A condvar would wake on the instant instead of within a slice, and with six
/// waits in five loops it is still not worth having. Every flag would need a
/// mutex and a condvar beside it and a `notify` at each of its setters, the
/// stop flag included, which is set from `Drop` — and a setter that forgot the
/// `notify` would fail as the slices never can, by sleeping out the interval.
/// What the slices cost is up to a quarter of a second on a key whose answer
/// comes off a network or a disk anyway, and four wakeups a second for each
/// idle thread.
///
/// `wake` is polled once per slice and is expected to *consume* the request: it
/// returns whether a refresh was asked for, and leaves the flag clear. A flag
/// of its own is `|| flag.swap(false, Ordering::AcqRel)`, set by the panel
/// with `Release` after whatever it wants the next round to read; a loop that
/// is never asked passes `|| false`.
///
/// The stop flag is checked before the first sleep as well as after each one,
/// so a stop set while the previous round was still fetching is seen
/// immediately rather than one slice later.
pub fn wait(interval: Duration, stop: &AtomicBool, mut wake: impl FnMut() -> bool) -> Wake {
    let mut waited = Duration::ZERO;
    while waited < interval {
        if stop.load(Ordering::Relaxed) {
            return Wake::Stop;
        }
        std::thread::sleep(SLICE);
        waited += SLICE;
        if wake() {
            break;
        }
    }
    if stop.load(Ordering::Relaxed) {
        Wake::Stop
    } else {
        Wake::Poll
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_stop_set_before_the_wait_returns_without_sleeping() {
        let stop = AtomicBool::new(true);
        let started = std::time::Instant::now();
        // An hour: if this returns at all, it did not sleep.
        let outcome = wait(Duration::from_hours(1), &stop, || false);
        assert_eq!(outcome, Wake::Stop);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "quitting must not wait out the interval"
        );
    }

    #[test]
    fn a_wake_request_ends_the_wait_early_and_asks_for_a_poll() {
        let stop = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let outcome = wait(Duration::from_hours(1), &stop, || true);
        assert_eq!(outcome, Wake::Poll);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_zero_interval_polls_immediately_without_consulting_the_wake_flag() {
        let stop = AtomicBool::new(false);
        let asked = AtomicUsize::new(0);
        let outcome = wait(Duration::ZERO, &stop, || {
            asked.fetch_add(1, Ordering::Relaxed);
            false
        });
        assert_eq!(outcome, Wake::Poll);
        assert_eq!(asked.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn the_interval_is_honoured_when_nothing_interrupts_it() {
        let stop = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let outcome = wait(SLICE * 2, &stop, || false);
        assert_eq!(outcome, Wake::Poll);
        assert!(
            started.elapsed() >= SLICE * 2,
            "returned after {:?}, before the interval was up",
            started.elapsed()
        );
    }
}
