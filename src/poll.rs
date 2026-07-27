//! The sleep between two rounds of a background fetch.
//!
//! Only the wait is shared. The weather and stocks loops look alike from a
//! distance — a thread, an `Arc<Mutex<_>>` of results, an `Arc<AtomicBool>` to
//! stop it — but past the skeleton they agree on very little: stocks reads its
//! *work list* from the shared request while weather's flag is write-only from
//! the panel, stocks iterates symbols with a configurable stagger, and weather
//! carries a resolved location across iterations. A shared loop would take all
//! of that as parameters, at which point the parameter list is the abstraction.
//!
//! The wait is different. It is the same twelve lines in both, and the mistake
//! it is easy to make — checking the stop flag after the sleep instead of
//! before, or not at all — costs the user a hang on quit rather than a wrong
//! number on screen.

use std::sync::Arc;
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
/// the full interval and quitting does not wait out anything — and without the
/// channel or condvar that a `recv_timeout` would need, which for two callers
/// would be more machinery than it saves.
///
/// `wake` is polled once per slice and is expected to *consume* the request: it
/// returns whether a refresh was asked for, and leaves the flag clear.
///
/// The stop flag is checked before the first sleep as well as after each one,
/// so a stop set while the previous round was still fetching is seen
/// immediately rather than one slice later.
pub fn wait(interval: Duration, stop: &Arc<AtomicBool>, mut wake: impl FnMut() -> bool) -> Wake {
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
        let stop = Arc::new(AtomicBool::new(true));
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
        let stop = Arc::new(AtomicBool::new(false));
        let started = std::time::Instant::now();
        let outcome = wait(Duration::from_hours(1), &stop, || true);
        assert_eq!(outcome, Wake::Poll);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_zero_interval_polls_immediately_without_consulting_the_wake_flag() {
        let stop = Arc::new(AtomicBool::new(false));
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
        let stop = Arc::new(AtomicBool::new(false));
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
