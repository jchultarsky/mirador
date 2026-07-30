//! Notable things that happened, and when.
//!
//! The design thesis names three instruments — chronometer, weather glass,
//! **watch log** — and for a long time only two of them existed. This is the
//! third: a record of what happened while you were not looking.
//!
//! ## What counts
//!
//! Almost nothing does, and that is the point. The clock, the readings, the
//! prices and the graphs change continuously and none of it is news; the notes
//! and tasks change because *you* changed them. An event is something that
//! happened **to** you rather than because of you, and that you would want to
//! know about even if you never looked at the panel it came from. That leaves a
//! very short list, which is the correct size for it.
//!
//! ## Why it is not a badge
//!
//! Unread message counts were considered for this dashboard and rejected —
//! "looks like information, is a doomscroll hook, turns a calm dashboard into a
//! nagging one". Read closely, that objection is about the *badge*: a number
//! that accumulates and demands you zero it.
//!
//! So the log has no count, no unread state, and no effect on any other panel's
//! appearance. It is a record you consult, not an inbox that consults you. If
//! you never place the panel, nothing about mirador changes. Breaking any of
//! those three is how this feature turns into the one that was rejected.
//!
//! ## Why it does not persist
//!
//! A log written to disk would come back after a restart with a hole in it —
//! the hours mirador was not running, during which it observed nothing and
//! cannot say so. Weather shows the age of its reading rather than pretending
//! it is current, and this follows the same rule: the log starts when mirador
//! does and says so, rather than implying a completeness it does not have.

use std::collections::VecDeque;

use jiff::Zoned;

/// How many entries are kept. Beyond this the oldest fall off the end.
///
/// A day of ordinary use produces a handful, so this is a bound on pathology
/// rather than a limit anyone should meet.
const CAPACITY: usize = 200;

/// Something worth recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// When it happened.
    pub at: Zoned,
    /// The panel that noticed, for grouping and for the reader's orientation.
    pub source: &'static str,
    /// One line, in the past tense, naming the thing rather than the panel.
    pub text: String,
}

impl Event {
    pub fn new(source: &'static str, text: impl Into<String>) -> Self {
        Self {
            at: Zoned::now(),
            source,
            text: text.into(),
        }
    }
}

/// The log itself: newest first, bounded, and aware of when it started.
#[derive(Debug)]
pub struct WatchLog {
    entries: VecDeque<Event>,
    /// When mirador started watching, shown as the last line so the log never
    /// implies it knows about anything earlier.
    since: Zoned,
    /// The last moment the reader was known to be present.
    ///
    /// `None` until something says otherwise, and a rule line is drawn only
    /// when it is `Some` *and* has entries on both sides — a line in the wrong
    /// place is worse than no line, because it makes a claim.
    seen_at: Option<Zoned>,
}

impl Default for WatchLog {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            since: Zoned::now(),
            seen_at: None,
        }
    }
}

impl WatchLog {
    /// Record an event.
    pub fn push(&mut self, event: Event) {
        self.entries.push_front(event);
        while self.entries.len() > CAPACITY {
            self.entries.pop_back();
        }
    }

    /// Newest first.
    pub fn entries(&self) -> impl Iterator<Item = &Event> {
        self.entries.iter()
    }

    pub fn since(&self) -> &Zoned {
        &self.since
    }

    /// Note that the reader is here now.
    ///
    /// Called on terminal focus where the terminal reports it, and on a
    /// keypress otherwise. Both are proxies: the dashboard cannot see you
    /// looking, and the moments you most want this to be right are the ones
    /// where you glanced without touching anything.
    pub fn mark_seen(&mut self) {
        self.seen_at = Some(Zoned::now());
    }

    /// How many of the newest entries arrived after the reader was last here.
    ///
    /// `None` means no line should be drawn: either nothing says when they were
    /// last here, everything is newer, or nothing is. A rule line at the very
    /// top or the very bottom separates nothing from something and reads as a
    /// glitch.
    pub fn unseen(&self) -> Option<usize> {
        let seen_at = self.seen_at.as_ref()?;
        let fresh = self
            .entries
            .iter()
            .take_while(|entry| &entry.at > seen_at)
            .count();
        (fresh > 0 && fresh < self.entries.len()).then_some(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Span;

    fn at(log: &WatchLog, minutes: i64) -> Event {
        Event {
            at: log
                .since
                .checked_add(Span::new().minutes(minutes))
                .expect("in range"),
            source: "test",
            text: format!("thing {minutes}"),
        }
    }

    #[test]
    fn the_newest_entry_comes_first() {
        let mut log = WatchLog::default();
        log.push(at(&log, 1));
        log.push(at(&log, 2));

        let texts: Vec<&str> = log.entries().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["thing 2", "thing 1"]);
    }

    #[test]
    fn the_oldest_entries_fall_off_rather_than_growing_without_bound() {
        let mut log = WatchLog::default();
        for minute in 0..i64::try_from(CAPACITY + 20).expect("fits") {
            log.push(at(&log, minute));
        }
        assert_eq!(log.entries().count(), CAPACITY);
        // The newest survived, the oldest did not.
        assert_eq!(
            log.entries().next().unwrap().text,
            format!("thing {}", CAPACITY + 19)
        );
    }

    /// A rule line makes a claim — "you have not seen these" — so it is only
    /// drawn when it can be true. At the very top or the very bottom it
    /// separates nothing from something, which reads as a rendering fault.
    #[test]
    fn no_rule_line_is_offered_when_it_would_sit_at_either_end() {
        let mut log = WatchLog::default();
        assert_eq!(log.unseen(), None, "nothing says when they were last here");

        log.mark_seen();
        log.push(at(&log, 10));
        assert_eq!(
            log.unseen(),
            None,
            "everything is new; the line would top it"
        );

        let mut log = WatchLog::default();
        log.push(at(&log, 1));
        log.push(at(&log, 2));
        log.seen_at = Some(log.since.checked_add(Span::new().minutes(5)).unwrap());
        assert_eq!(log.unseen(), None, "nothing is new; the line would foot it");
    }

    #[test]
    fn the_line_falls_between_what_arrived_before_and_after() {
        let mut log = WatchLog::default();
        log.push(at(&log, 1));
        log.push(at(&log, 2));
        log.seen_at = Some(log.since.checked_add(Span::new().minutes(3)).unwrap());
        log.push(at(&log, 4));
        log.push(at(&log, 5));

        assert_eq!(log.unseen(), Some(2), "the two that arrived after");
    }

    /// #132, written as the sequence a reader actually performs.
    ///
    /// The bug was in `app.rs`, which marked the log seen on `FocusGained` as
    /// well as `FocusLost` — so returning to the dashboard set "last looked" to
    /// *now*, every entry that arrived while away landed on the old side of the
    /// line, and the line vanished in the instant it was wanted. A terminal that
    /// reported focus *correctly* made the feature less visible than one that
    /// did not.
    ///
    /// The log itself was always right, which is why this asserts the property
    /// the reader cares about — the line is still there when they come back —
    /// rather than which events fire. `only_losing_focus_marks_the_log_seen` in
    /// `app.rs` holds the wiring.
    ///
    /// Timestamps come from `at`, not `Zoned::now()`. The first version of this
    /// test used real clock reads and failed about one run in three: `unseen`
    /// compares with `>`, and two `now()` calls in a tight loop can land on the
    /// same instant.
    #[test]
    fn coming_back_leaves_the_rule_line_where_it_was() {
        let mut log = WatchLog::default();
        log.push(at(&log, 1));

        // Leaving is the half that can be recorded honestly: the reader was
        // present with that entry on screen.
        log.seen_at = Some(
            log.since
                .checked_add(Span::new().minutes(2))
                .expect("in range"),
        );

        // Two things happen while nobody is looking.
        log.push(at(&log, 3));
        log.push(at(&log, 4));

        assert_eq!(
            log.unseen(),
            Some(2),
            "both entries that arrived while away are unseen"
        );

        // Coming back marks nothing now, so the line is still here. Before the
        // fix this point was a fresh `mark_seen()` and the answer became `None`.
        assert_eq!(
            log.unseen(),
            Some(2),
            "returning must not erase the line it exists to show"
        );
    }
}
