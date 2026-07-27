//! Headlines: a window on the world, not a feed.
//!
//! This is the panel that came closest to the one this dashboard turned down.
//! Unread counts were rejected for being "a doomscroll hook" that "turns a calm
//! dashboard into a nagging one", and news is *the* doomscroll surface —
//! unbounded, constantly moving, and written by professionals to be clicked.
//!
//! So the commitment is the same shape as the watch log's, and it is
//! presentational:
//!
//! - **However many stories fit, and no more.** There is no scrolling and no
//!   "more below". You cannot work through it, because there is nothing to work
//!   through.
//! - **No count anywhere.** Not in the frame, not in the body.
//! - **Nothing is new.** No unread state, no marker, no reordering to put fresh
//!   items on top beyond the newest-first order they already have.
//! - **Refreshed hourly**, which is slower than news moves and about as often
//!   as a person should want to know.
//!
//! You glance at it the way you glance at the weather glass. Break any of those
//! and this becomes the feature that was turned down.
//!
//! Headlines only, never the summary — see [`crate::feed`] for why. And no
//! browser is launched: `o` shows the link and you copy it yourself, the same
//! answer this project gave to playing a sound file.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jiff::Zoned;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::config::NewsConfig;
use crate::feed::Story;
use crate::frame::{Binding, FRAME_WIDTH};
use crate::panel::{KeyOutcome, Panel, RenderContext, describe_age};

const BINDINGS: &[Binding] = &[
    Binding::primary("r", "refresh"),
    Binding::primary("o", "show link"),
    Binding::extra("↑ / ↓", "select"),
    Binding::extra("j / k", "select"),
];

/// Interior width past which the panel gains nothing.
///
/// A headline runs about seventy cells at the median, measured across five real
/// feeds, so this is two comfortable lines plus room for the longest to breathe.
/// Past it the text just gets further from the eye.
const USEFUL_WIDTH: u16 = 62;

/// How long a request may take before it is given up on.
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// What the fetch thread has produced.
#[derive(Debug, Default)]
struct State {
    stories: Vec<Story>,
    fetched: Option<Instant>,
    /// Why the last attempt failed. The stories are kept either way — an hour
    /// old headline is still news, where an empty panel is nothing.
    error: Option<String>,
}

pub struct NewsPanel {
    state: Arc<Mutex<State>>,
    refresh: Arc<Mutex<bool>>,
    stop: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    seen: u64,
    selected: ListState,
    /// Set by `o`; the link of whatever is selected.
    showing_link: Option<String>,
    /// Stories drawn last frame, for clamping the selection.
    drawn: usize,
    /// The stories as of the last time the fetch thread published.
    ///
    /// Cached rather than read through the mutex on every draw. Stories change
    /// once an hour and a draw happens about once a second, so cloning them per
    /// frame was sixty allocations for the same sixty headlines — the same
    /// mistake the agenda's alert made, found in the same review.
    shown: Vec<Story>,
    fetched: Option<Instant>,
    failing: Option<String>,
}

impl Drop for NewsPanel {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl NewsPanel {
    pub fn new(config: &NewsConfig) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let refresh = Arc::new(Mutex::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));

        // An hour is the floor as well as the default. A dashboard left open
        // for a week should not be re-reading somebody's feed every minute
        // because a config said so.
        let interval = Duration::from_secs(config.refresh_minutes.max(60) * 60);
        let feeds: Vec<(String, String)> = config
            .feeds
            .iter()
            .filter(|feed| !feed.url.trim().is_empty())
            .map(|feed| (feed.name.clone(), feed.url.clone()))
            .collect();
        let per_feed = config.per_feed.clamp(1, 20);

        let shared = (
            Arc::clone(&state),
            Arc::clone(&refresh),
            Arc::clone(&stop),
            Arc::clone(&generation),
        );
        std::thread::Builder::new()
            .name("mirador-news".into())
            .spawn(move || {
                let (state, refresh, stop, generation) = shared;
                fetch_loop(
                    &feeds,
                    per_feed,
                    interval,
                    &state,
                    &refresh,
                    &stop,
                    &generation,
                );
            })
            .expect("spawning the news thread");

        Self {
            state,
            refresh,
            stop,
            generation,
            seen: 0,
            selected: ListState::default(),
            showing_link: None,
            drawn: 0,
            shown: Vec::new(),
            fetched: None,
            failing: None,
        }
    }

    fn snapshot(&self) -> State {
        match self.state.lock() {
            Ok(guard) => State {
                stories: guard.stories.clone(),
                fetched: guard.fetched,
                error: guard.error.clone(),
            },
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                State {
                    stories: guard.stories.clone(),
                    fetched: guard.fetched,
                    error: guard.error.clone(),
                }
            }
        }
    }
}

impl Panel for NewsPanel {
    fn title(&self) -> String {
        "News".into()
    }

    /// Deliberately `None`.
    ///
    /// Every other list panel here carries a count. This one must not: a number
    /// in the border is a badge, a badge accumulates, and an accumulating badge
    /// is the unread-message count this dashboard turned down. The age of the
    /// reading is not a count and would be defensible, but it belongs to the
    /// fetch rather than to the news, and the panel already says it below.
    fn counter(&self) -> Option<String> {
        None
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn max_width(&self) -> Option<u16> {
        Some(USEFUL_WIDTH + FRAME_WIDTH)
    }

    fn refresh_interval(&self) -> Duration {
        // How often a completed fetch reaches the screen, not how often one is
        // made — the thread owns that, and it is an hour.
        Duration::from_secs(20)
    }

    fn tick(&mut self) -> bool {
        let now = self.generation.load(Ordering::Acquire);
        let moved = now != self.seen;
        self.seen = now;
        // The one moment the stories can have changed, so the one moment worth
        // copying them out from under the lock.
        if moved {
            let state = self.snapshot();
            self.shown = state.stories;
            self.fetched = state.fetched;
            self.failing = state.error;
        }
        moved
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        // Nothing here dismisses, hides or marks a story. There is no state to
        // keep up with, which is the point.
        self.showing_link = None;
        match key.code {
            KeyCode::Char('r') => {
                if let Ok(mut flag) = self.refresh.lock() {
                    *flag = true;
                }
            }
            KeyCode::Char('o') => {
                let stories = &self.shown;
                // Shown, not opened. Launching a browser means talking to the
                // platform — `open`, `xdg-open`, `start` — which is the same
                // decision as playing a sound file and gets the same answer.
                self.showing_link = self
                    .selected
                    .selected()
                    .and_then(|index| stories.get(index))
                    .map(|story| story.link.clone())
                    .filter(|link| !link.is_empty());
            }
            KeyCode::Down | KeyCode::Char('j') => {
                crate::selection::down(&mut self.selected, 1, self.drawn);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                crate::selection::up(&mut self.selected, 1, self.drawn);
            }
            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> KeyOutcome {
        match event.kind {
            MouseEventKind::ScrollDown => {
                crate::selection::down(&mut self.selected, 1, self.drawn);
            }
            MouseEventKind::ScrollUp => crate::selection::up(&mut self.selected, 1, self.drawn),
            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.width == 0 || area.height == 0 {
            return;
        }

        let width = usize::from(area.width);

        // One row at the foot for the age, or the link, or a failure.
        let footer = Rect {
            y: area.y + area.height.saturating_sub(1),
            height: 1.min(area.height),
            ..area
        };
        let body = Rect {
            height: area.height.saturating_sub(1),
            ..area
        };

        if self.shown.is_empty() {
            let message = match &self.failing {
                Some(why) => vec![
                    Line::from(Span::styled(
                        "Cannot read the feeds",
                        Style::default()
                            .fg(theme.error)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        crate::grid::truncate(why, width),
                        Style::default().fg(theme.muted),
                    )),
                ],
                None if self.fetched.is_some() => vec![Line::from(Span::styled(
                    "No stories.",
                    Style::default().fg(theme.muted),
                ))],
                None => vec![Line::from(Span::styled(
                    "Reading…",
                    Style::default().fg(theme.muted),
                ))],
            };
            frame.render_widget(Paragraph::new(message), body);
            self.drawn = 0;
            return;
        }

        // Each story is its source and age on one line, the headline wrapped
        // under it, and a blank line after. Air rather than rules: a separator
        // between every story would be more furniture than content at this
        // width, and the panel is meant to read like a page.
        let mut items: Vec<ListItem> = Vec::new();
        let last = self.shown.len().saturating_sub(1);
        for (index, story) in self.shown.iter().enumerate() {
            let mut lines = vec![Line::from(masthead(story, theme))];
            for line in crate::grid::wrap(&story.title, width) {
                lines.push(Line::from(Span::styled(
                    line,
                    Style::default().fg(theme.text),
                )));
            }
            // The gap goes *between* stories, not after every one. A trailing
            // blank on the last item costs a whole row, and `List` draws only
            // items that fit entirely — so that row was routinely the
            // difference between two stories and three.
            if index != last {
                lines.push(Line::from(""));
            }
            items.push(ListItem::new(lines));
        }

        self.drawn = items.len();
        frame.render_stateful_widget(List::new(items), body, &mut self.selected);

        let foot = match (&self.showing_link, &self.failing) {
            (Some(link), _) => Span::styled(
                crate::grid::truncate(link, width),
                Style::default().fg(theme.label),
            ),
            (None, Some(_)) => Span::styled(
                format!(
                    "{} — refresh failing",
                    self.fetched
                        .map_or_else(|| "never read".to_string(), |at| describe_age(at.elapsed()))
                ),
                Style::default().fg(theme.warning),
            ),
            (None, None) => Span::styled(
                self.fetched
                    .map_or_else(String::new, |at| describe_age(at.elapsed())),
                Style::default().fg(theme.muted),
            ),
        };
        if footer.height > 0 {
            frame.render_widget(Paragraph::new(Line::from(foot)), footer);
        }
    }
}

/// `NASA · 2h`, in the engraved-label face.
fn masthead(story: &Story, theme: &crate::theme::Theme) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(
        crate::glyphs::utility(&story.source),
        Style::default()
            .fg(theme.label)
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(at) = &story.published {
        let age = Zoned::now().duration_since(at);
        if let Ok(age) = std::time::Duration::try_from(age) {
            spans.push(Span::styled(
                format!(" · {}", describe_age(age)),
                Style::default().fg(theme.muted),
            ));
        }
    }
    spans
}

/// Read every feed, newest first, then wait.
#[allow(clippy::too_many_arguments)]
fn fetch_loop(
    feeds: &[(String, String)],
    per_feed: usize,
    interval: Duration,
    state: &Arc<Mutex<State>>,
    refresh: &Arc<Mutex<bool>>,
    stop: &Arc<AtomicBool>,
    generation: &Arc<AtomicU64>,
) {
    while !stop.load(Ordering::Relaxed) {
        let mut stories = Vec::new();
        let mut failures = Vec::new();

        for (name, url) in feeds {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            match read_feed(url) {
                Ok(mut found) => {
                    found.truncate(per_feed);
                    for mut story in found {
                        story.source.clone_from(name);
                        stories.push(story);
                    }
                }
                Err(e) => failures.push(format!("{name}: {e:#}")),
            }
        }

        let stories = interleave(stories);

        let failed = (!failures.is_empty()).then(|| failures.join("; "));
        match state.lock() {
            Ok(mut guard) => {
                // A total failure keeps what is on screen; a partial one takes
                // what arrived. Either way the age below says how fresh it is.
                if !stories.is_empty() {
                    guard.stories = stories;
                    guard.fetched = Some(Instant::now());
                }
                guard.error = failed;
            }
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                if !stories.is_empty() {
                    guard.stories = stories;
                    guard.fetched = Some(Instant::now());
                }
                guard.error = failed;
            }
        }
        generation.fetch_add(1, Ordering::Release);

        let woke = crate::poll::wait(interval, stop, || match refresh.lock() {
            Ok(mut flag) => std::mem::replace(&mut *flag, false),
            Err(poisoned) => std::mem::replace(&mut *poisoned.into_inner(), false),
        });
        if woke == crate::poll::Wake::Stop {
            return;
        }
    }
}

/// One story from each feed, then the next from each, and so on.
///
/// Not simply newest-first, which is what this did at first and which is wrong
/// for what the panel is. The feeds publish at wildly different rates, so date
/// order hands the whole visible window to whichever one happens to be chatty
/// — the first run showed three consecutive Phys.org space stories, which is
/// not a window on the world however fresh it is.
///
/// Round-robin guarantees that the first thing you see is the newest from each
/// feed, which is the shape "a handful of stories, however many fit" was meant
/// to have. Within a feed the order is still newest first.
fn interleave(stories: Vec<Story>) -> Vec<Story> {
    let mut by_source: Vec<Vec<Story>> = Vec::new();
    for story in stories {
        match by_source
            .iter_mut()
            .find(|group| group.first().is_some_and(|s| s.source == story.source))
        {
            Some(group) => group.push(story),
            None => by_source.push(vec![story]),
        }
    }

    // Newest first inside each feed. An undated story goes last rather than
    // first — an absent date is not "just now".
    for group in &mut by_source {
        group.sort_by(|a, b| match (&b.published, &a.published) {
            (Some(later), Some(earlier)) => later.cmp(earlier),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => std::cmp::Ordering::Equal,
        });
    }

    let deepest = by_source.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = Vec::new();
    for rank in 0..deepest {
        for group in &by_source {
            if let Some(story) = group.get(rank) {
                out.push(story.clone());
            }
        }
    }
    out
}

/// Fetch and parse one feed.
fn read_feed(url: &str) -> anyhow::Result<Vec<Story>> {
    let body = ureq::get(url)
        .config()
        .timeout_global(Some(HTTP_TIMEOUT))
        .build()
        .call()?
        .body_mut()
        .read_to_string()?;
    crate::feed::parse(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn story(source: &str, title: &str, minutes_ago: i64) -> Story {
        Story {
            source: source.into(),
            title: title.into(),
            link: String::new(),
            published: Zoned::now()
                .checked_sub(jiff::Span::new().minutes(minutes_ago))
                .ok(),
        }
    }

    /// The first version sorted purely by date and handed the whole visible
    /// window to whichever feed published most often — three consecutive
    /// Phys.org stories on the first real run. However fresh that is, it is not
    /// a window on the world.
    #[test]
    fn the_top_of_the_panel_holds_one_story_from_each_feed() {
        let mixed = interleave(vec![
            story("PHYS", "phys newest", 5),
            story("PHYS", "phys second", 10),
            story("PHYS", "phys third", 15),
            story("NASA", "nasa newest", 60),
            story("ARS", "ars newest", 90),
        ]);

        let first_three: Vec<&str> = mixed.iter().take(3).map(|s| s.source.as_str()).collect();
        assert_eq!(first_three, ["PHYS", "NASA", "ARS"], "one from each first");
        assert_eq!(
            mixed[0].title, "phys newest",
            "and the newest within a feed"
        );
        assert_eq!(mixed[3].title, "phys second", "then the second round");
    }

    /// Within a feed the order is still newest first, and a story with no date
    /// sorts last rather than being taken for the freshest thing there is.
    #[test]
    fn an_undated_story_goes_last_within_its_feed() {
        let undated = Story {
            source: "PHYS".into(),
            title: "undated".into(),
            link: String::new(),
            published: None,
        };
        let ordered = interleave(vec![
            undated,
            story("PHYS", "older", 90),
            story("PHYS", "newer", 5),
        ]);
        let titles: Vec<&str> = ordered.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["newer", "older", "undated"]);
    }
}
