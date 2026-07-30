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
//!   through. `render` keeps only the stories whose whole block fits the body,
//!   so the selection cannot leave the viewport and `List` has nothing to
//!   scroll. This was documentation rather than behaviour until #118 — every
//!   story became an item and the panel scrolled through all twelve — and the
//!   tests at the foot of this file are what stop it drifting back.
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

        // One row at the foot for the age or a failure — but a link gets as
        // many as it needs. It used to be truncated to one row, which left the
        // URL unreadable, uncopyable, and (worse) linkified by the terminal as
        // far as the ellipsis, so clicking it went somewhere that did not exist.
        // Bounded at half the panel so a long link cannot swallow the stories.
        let foot_rows = match &self.showing_link {
            Some(link) => {
                crate::grid::wrapped_height(link, area.width).clamp(1, (area.height / 2).max(1))
            }
            None => 1.min(area.height),
        };
        let footer = Rect {
            y: area.y + area.height.saturating_sub(foot_rows),
            height: foot_rows,
            ..area
        };
        let body = Rect {
            height: area.height.saturating_sub(foot_rows),
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

        let blocks = story_blocks(&self.shown, width, theme);
        let budget = usize::from(body.height);
        let mut keep = how_many_fit(&blocks, budget);
        // A headline too tall for the panel would otherwise leave it blank with
        // stories loaded, which reads as broken rather than as short. Show the
        // first one clipped instead: `List` draws only items that fit entirely,
        // so the clipping has to happen here.
        let clipped = keep == 0 && budget > 0;
        if clipped {
            keep = 1;
        }

        let mut items: Vec<ListItem> = Vec::with_capacity(keep);
        for (index, mut lines) in blocks.into_iter().take(keep).enumerate() {
            if clipped {
                lines.truncate(budget);
            }
            // The gap goes *between* stories, not after every one. A trailing
            // blank on the last item costs a whole row, and `List` draws only
            // items that fit entirely — so that row was routinely the
            // difference between two stories and three.
            if index + 1 != keep {
                lines.push(Line::from(""));
            }
            items.push(ListItem::new(lines));
        }

        self.drawn = items.len();
        // The cursor `o` acts on has to be visible, or the link in the footer
        // belongs to no story the reader can identify. Same marker and weight as
        // `todo`, `notes` and `stocks`, and it recedes with focus like they do.
        //
        // This is navigation, not unread state: it shows where the keyboard is
        // pointing and goes away with the focus. The panel still carries no
        // count and nothing to dismiss, which is what keeps it clear of the
        // unread-badge feature this dashboard turned down.
        //
        // Note these rows are multi-line — a masthead plus a wrapped title —
        // unlike the one-line rows the other three panels draw. `List` puts the
        // symbol on a row's first line and indents the rest to match, which is
        // what makes a wrapped title still line up under its own masthead.
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol(if ctx.focused { "▸ " } else { "  " })
                .highlight_style(if ctx.focused {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.muted)
                }),
            body,
            &mut self.selected,
        );

        // A `(text, style)` pair rather than a `Span`, because the link case is
        // multi-line and a `Span` holding newlines renders as one line with the
        // breaks swallowed. `Paragraph::new(String)` splits on them properly.
        let (foot, foot_style) = match (&self.showing_link, &self.failing) {
            // Wrapped by `grid`, never handed to ratatui's wrapper, and never
            // truncated: the whole point is that the reader can read and copy
            // the entire URL.
            (Some(link), _) => (
                crate::grid::wrapped(link, area.width),
                Style::default().fg(theme.label),
            ),
            (None, Some(_)) => (
                format!(
                    "{} — refresh failing",
                    self.fetched
                        .map_or_else(|| "never read".to_string(), |at| describe_age(at.elapsed()))
                ),
                Style::default().fg(theme.warning),
            ),
            (None, None) => (
                self.fetched
                    .map_or_else(String::new, |at| describe_age(at.elapsed())),
                Style::default().fg(theme.muted),
            ),
        };
        if footer.height > 0 {
            frame.render_widget(Paragraph::new(foot).style(foot_style), footer);
        }
    }
}

/// One block of lines per story: its source and age, then the wrapped headline.
///
/// The blank line that separates stories is *not* here — it belongs between
/// blocks rather than to any one of them, and [`how_many_fit`] has to count it
/// that way. Air rather than rules: a separator between every story would be
/// more furniture than content at this width, and the panel is meant to read
/// like a page.
fn story_blocks(
    stories: &[Story],
    width: usize,
    theme: &crate::theme::Theme,
) -> Vec<Vec<Line<'static>>> {
    stories
        .iter()
        .map(|story| {
            let mut lines = vec![Line::from(masthead(story, theme))];
            for line in crate::grid::wrap(&story.title, width) {
                lines.push(Line::from(Span::styled(
                    line,
                    Style::default().fg(theme.text),
                )));
            }
            lines
        })
        .collect()
}

/// How many of `blocks` fit whole in `budget` rows.
///
/// **This is where "however many stories fit, and no more" is enforced**, and
/// for a long time nothing was: every story became a `ListItem`, so `List`
/// scrolled the moment the selection left the viewport. Twelve stories with
/// three visible made nine of them reachable only by scrolling, which is a feed
/// you work through — the thing this panel exists to not be. Filed as #118 after
/// being found by driving the panel rather than by reading it.
///
/// Bounding the *list* rather than clamping the selection afterwards is the
/// point. The cursor is limited to what was built, so it cannot walk off the
/// screen and `List` is never given anything to scroll. One rule, one place.
///
/// A story after the first is preceded by a blank line, so `n` stories cost
/// their own rows plus `n - 1` separators.
///
/// This stays a floor rather than a ceiling, per the config rule: a taller panel
/// shows more, up to whatever the feeds provided.
fn how_many_fit(blocks: &[Vec<Line<'static>>], budget: usize) -> usize {
    let mut used = 0usize;
    let mut keep = 0usize;
    for block in blocks {
        let cost = block.len() + usize::from(keep > 0);
        if used + cost > budget {
            break;
        }
        used += cost;
        keep += 1;
    }
    keep
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
                        // Bounded for the same reason the feed's own fields are:
                        // `masthead` uppercases this on every frame, so an
                        // over-long name — this one comes from the config rather
                        // than from the feed, but still — would be re-allocated
                        // sixty times a minute.
                        story.source = name.chars().take(crate::feed::MAX_SOURCE).collect();
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

    /// #114: the panel kept a cursor that `j`/`k` moved and `o` acted on, and
    /// drew no highlight at all — so the link in the footer belonged to a story
    /// the reader could not identify. Every other list panel shows one.
    #[test]
    fn the_selected_story_is_visibly_marked() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();

        let mut panel = NewsPanel::new(&NewsConfig {
            feeds: Vec::new(),
            ..NewsConfig::default()
        });
        panel.shown = vec![
            story("NASA", "First story, the one selected", 10),
            story("PHYS.ORG", "Second story", 20),
        ];
        panel.selected.select(Some(1));

        let draw = |panel: &mut NewsPanel, focused: bool| -> String {
            let mut terminal = Terminal::new(TestBackend::new(46, 12)).unwrap();
            terminal
                .draw(|frame| {
                    panel.render(
                        frame,
                        frame.area(),
                        RenderContext {
                            theme: &config.theme,
                            gradients: &gradients,
                            focused,
                            watch: &crate::watch::WatchLog::default(),
                        },
                    );
                })
                .unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..12)
                .map(|y| {
                    (0..46)
                        .filter_map(|x| buffer.cell((x, y)).map(|c| c.symbol().to_string()))
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let focused = draw(&mut panel, true);
        assert!(
            focused.contains('\u{25B8}'),
            "the focused panel must mark which story the cursor is on:\n{focused}"
        );

        // And it recedes with focus, like the other panels' markers do, rather
        // than leaving a stale pointer on an unfocused panel.
        let unfocused = draw(&mut panel, false);
        assert!(
            !unfocused.contains('\u{25B8}'),
            "an unfocused panel should not keep pointing:\n{unfocused}"
        );
    }

    /// #108: `o` truncated the URL to the panel width, so the reader could
    /// neither read nor copy it — and the terminal linkified the visible text,
    /// which now ended in an ellipsis, so clicking landed on a URL that did not
    /// exist. The reporter's browser showed the `…` as `%E2%80%A6`.
    ///
    /// The property: **every character of the link reaches the screen.**
    #[test]
    fn the_whole_link_is_shown_rather_than_truncated() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::crossterm::event::KeyModifiers;

        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();
        let link = "https://arstechnica.com/security/2026/07/mythos-attack-on-3rd-round-pqc-algorithm-puts-it-out-of-commission/";

        let mut panel = NewsPanel::new(&NewsConfig {
            feeds: Vec::new(),
            ..NewsConfig::default()
        });
        panel.shown = vec![Story {
            source: "Ars Technica".into(),
            title: "Mythos attack on 3rd-round PQC algorithm".into(),
            link: link.into(),
            published: Zoned::now().checked_sub(jiff::Span::new().minutes(30)).ok(),
        }];
        panel.selected.select(Some(0));
        panel.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));

        for (width, height) in [(40u16, 14u16), (56, 18), (72, 12), (100, 20)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    panel.render(
                        frame,
                        frame.area(),
                        RenderContext {
                            theme: &config.theme,
                            gradients: &gradients,
                            focused: true,
                            watch: &crate::watch::WatchLog::default(),
                        },
                    );
                })
                .unwrap();
            let buffer = terminal.backend().buffer().clone();
            let screen: String = (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .filter_map(|(x, y)| buffer.cell((x, y)).map(|c| c.symbol().to_string()))
                .collect();
            let squashed: String = screen.chars().filter(|c| !c.is_whitespace()).collect();

            assert!(
                !squashed.contains('\u{2026}'),
                "the link was truncated with an ellipsis at {width}x{height}"
            );
            assert!(
                squashed.contains(&link.replace(' ', "")),
                "the whole link should be on screen at {width}x{height}"
            );
        }
    }

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

    /// The fitting rule on its own, without a terminal in the way.
    ///
    /// `n` stories cost their own rows plus `n - 1` blank separators, so the
    /// arithmetic is worth pinning directly — a test that only goes through
    /// `render` cannot say whether an off-by-one is in the counting or the
    /// drawing.
    #[test]
    fn the_fitting_rule_counts_the_separators_between_stories() {
        let block = |rows: usize| vec![Line::from(""); rows];
        let three = vec![block(2), block(2), block(2)];

        assert_eq!(how_many_fit(&three, 2), 1, "one story, no separator");
        assert_eq!(
            how_many_fit(&three, 4),
            1,
            "two need a separator: 2+1+2 = 5"
        );
        assert_eq!(how_many_fit(&three, 5), 2, "exactly two");
        assert_eq!(how_many_fit(&three, 7), 2, "three need 2+1+2+1+2 = 8");
        assert_eq!(how_many_fit(&three, 8), 3, "exactly three");
        assert_eq!(how_many_fit(&three, 99), 3, "never more than there are");

        assert_eq!(how_many_fit(&three, 0), 0, "no room, no stories");
        assert_eq!(how_many_fit(&three, 1), 0, "not even one block fits");
        assert_eq!(how_many_fit(&[], 10), 0, "nothing to fit");
    }

    /// A panel holding twelve stories, the number the shipped config produces.
    fn loaded_panel() -> NewsPanel {
        let mut panel = NewsPanel::new(&NewsConfig {
            feeds: Vec::new(),
            ..NewsConfig::default()
        });
        panel.shown = (0..12)
            .map(|n| story("NASA", &format!("Story number {n}"), n * 10))
            .collect();
        panel
    }

    /// Draw the panel and return what reached the screen.
    fn draw(panel: &mut NewsPanel, width: u16, height: u16) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                panel.render(
                    frame,
                    frame.area(),
                    RenderContext {
                        theme: &config.theme,
                        gradients: &gradients,
                        focused: true,
                        watch: &crate::watch::WatchLog::default(),
                    },
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .filter_map(|x| buffer.cell((x, y)).map(|c| c.symbol().to_string()))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// **The panel shows what fits and nothing more.**
    ///
    /// #118: it did not. Every story became a `ListItem`, so `List` scrolled as
    /// soon as the selection left the viewport — twelve stories with three
    /// visible made nine reachable only by scrolling. That is a feed you work
    /// through, which is precisely the thing this panel exists to not be, and
    /// the module header claimed the opposite for months because nothing here
    /// checked.
    ///
    /// If this fails, the question is not how to fix the test.
    #[test]
    fn the_panel_builds_only_the_stories_that_fit() {
        let mut panel = loaded_panel();
        draw(&mut panel, 46, 10);
        let short = panel.drawn;
        assert!(
            short > 0 && short < 12,
            "a 10-row panel should hold some of twelve stories, held {short}"
        );

        draw(&mut panel, 46, 30);
        assert!(
            panel.drawn > short,
            "a taller panel must show more, not the same {short}"
        );
    }

    /// The selection is bounded by what was built, so the cursor cannot walk off
    /// the screen — which is what leaves `List` with nothing to scroll.
    #[test]
    fn the_cursor_cannot_leave_the_visible_stories() {
        let mut panel = loaded_panel();
        let visible = {
            draw(&mut panel, 46, 10);
            panel.drawn
        };

        for _ in 0..50 {
            panel.handle_key(KeyEvent::from(KeyCode::Char('j')));
        }
        assert!(
            panel.selected.selected().unwrap_or(0) < visible,
            "the cursor reached {:?} with only {visible} stories on screen",
            panel.selected.selected()
        );

        // And what is on screen after all that walking is still the top of the
        // list: nothing scrolled underneath it.
        let after = draw(&mut panel, 46, 10);
        assert!(
            after.contains("Story number 0"),
            "the first story scrolled away; got:\n{after}"
        );
    }

    /// The panel carries no count, in the frame or the body. #118 found that
    /// `counter()` was right and nothing held it there — unlike the watch log,
    /// which has had this test all along.
    ///
    /// A number in the border is a badge, a badge accumulates, and an
    /// accumulating badge is the unread-message count this dashboard turned
    /// down. If this assertion ever fails, the question to ask is not how to
    /// fix the test.
    #[test]
    fn the_panel_never_offers_a_counter() {
        let mut panel = loaded_panel();
        assert_eq!(panel.counter(), None, "no counter in the frame");

        let screen = draw(&mut panel, 46, 10);
        for claim in ["12", "new", "more", "of 12"] {
            assert!(
                !screen.contains(claim),
                "the panel said `{claim}`, which counts or promises more below:\n{screen}"
            );
        }
    }

    /// A headline taller than the panel must still show something. A blank
    /// panel with stories loaded reads as broken rather than as short.
    #[test]
    fn a_story_too_tall_for_the_panel_is_clipped_rather_than_dropped() {
        let mut panel = NewsPanel::new(&NewsConfig {
            feeds: Vec::new(),
            ..NewsConfig::default()
        });
        panel.shown = vec![story("NASA", &"a very long headline ".repeat(20), 5)];

        let screen = draw(&mut panel, 24, 4);
        assert_eq!(panel.drawn, 1, "the only story must still be built");
        assert!(
            screen.contains("NASA"),
            "nothing was drawn at all; got:\n{screen}"
        );
    }
}
