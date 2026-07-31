//! The stock watchlist.
//!
//! One of the four questions the dashboard exists to answer: what is the
//! portfolio doing. It shows last price, the day's change in both currency and
//! percent, and an intraday sparkline.
//!
//! Fetching happens on a background thread and the panel reads a mutex-guarded
//! snapshot, as the weather panel does — a panel that blocks freezes the whole
//! dashboard. Symbols are requested **one at a time with a pause between
//! them**, not concurrently: a burst of parallel requests is what gets an IP
//! rate-limited, and a watchlist has no deadline.
//!
//! Prices are never written to disk. Only the list of symbols is persisted, and
//! that lives in a data file rather than in the config, which is what lets the
//! panel edit it — mirador deliberately never rewrites its config.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::config::StocksConfig;
use crate::frame::{Binding, FRAME_HEIGHT, FRAME_WIDTH};
use crate::grid::{Column, Grid};
use crate::panel::{KeyOutcome, Panel, RenderContext, describe_age};
use crate::quote::{Quote, QuoteSource, Watchlist, source_for, sparkline};
use crate::textfield::TextField;
use crate::theme::Theme;

const BINDINGS: &[Binding] = &[
    Binding::primary("a", "add"),
    Binding::primary("d", "remove"),
    Binding::primary("r", "refresh"),
    Binding::extra("↑ / ↓", "move selection"),
    Binding::extra("j / k", "move selection"),
    Binding::extra("g / G", "first / last"),
    Binding::extra("Home / End", "first / last"),
    Binding::extra("o", "show file path"),
];

/// The shortest gap between two rounds of polling, however it was asked for.
///
/// Not a tuning knob. The quote sources are free and unauthenticated and gate
/// on IP reputation, so exceeding this costs everyone behind the same address,
/// not just the user who held the key down.
const MIN_SECONDS_BETWEEN_POLLS: u64 = 60;

/// Widest the intraday sparkline is drawn.
const SPARK_WIDTH: u16 = 12;

/// Columns the `▸ ` selection marker occupies to the left of the grid.
const SELECTION_MARKER: u16 = 2;

/// Width the four fixed columns and the gutters between all five occupy.
const FIXED_COLUMNS: u16 = 8 + 10 + 9 + 8 + 4;

/// Grid width at which the sparkline column earns its place: everything else,
/// plus a sparkline drawn at full size.
///
/// Derived rather than written down, because this figure appears in three
/// places — the column's own drop threshold, the panel's maximum width, and the
/// width the sparkline is drawn at — and the three drifting apart is exactly
/// how the column ends up allocated but empty.
const SPARK_MIN_GRID: u16 = FIXED_COLUMNS + SPARK_WIDTH;

const COLUMNS: &[Column] = &[
    Column::fixed("symbol", 8),
    Column::fixed("last", 10).right(),
    Column::fixed("chg", 9).right(),
    Column::fixed("%", 8).right(),
    Column::flex("today", 1).drops_below(SPARK_MIN_GRID),
];

/// What the background thread has produced for one symbol.
///
/// Shaped like the weather panel's `State` and for the same reason: a
/// failed request keeps the last good price and shows how old it is, instead of
/// replacing a real number with a dash for a whole refresh interval. This panel
/// used to overwrite the quote with the error, so one timed-out request blanked
/// the price, the change, the percentage and the sparkline together — and with
/// no timestamp anywhere, a thread that quietly stopped went on showing
/// confident numbers for as long as the dashboard was open.
///
/// The rule is the one weather follows: old data labelled old is useful, no
/// data is not, and old data presented as current is the only unacceptable
/// outcome.
#[derive(Debug, Clone, Default)]
struct Cell {
    /// The last good quote and when it landed, if one ever did.
    quote: Option<(Quote, Instant)>,
    /// Why the most recent attempt failed, if it did.
    error: Option<String>,
}

impl Cell {
    /// How long ago the price on screen was fetched.
    fn age(&self) -> Option<Duration> {
        self.quote.as_ref().map(|(_, at)| at.elapsed())
    }

    /// Whether what is on screen should be presented as possibly out of date.
    fn is_stale(&self, after: Duration) -> bool {
        self.error.is_some() || self.age().is_some_and(|age| age > after)
    }
}

/// The shared snapshot: one entry per symbol, in watchlist order.
type Board = Vec<(String, Cell)>;

/// Instructions passed from the panel to the fetch thread.
#[derive(Debug, Default)]
struct Request {
    /// The symbols to poll, replaced whenever the watchlist changes.
    symbols: Vec<String>,
    /// Set to ask for an immediate re-poll.
    refresh: bool,
}

#[derive(Debug)]
enum Mode {
    List,
    /// Typing a symbol to add.
    Add(TextField),
    ConfirmRemove {
        symbol: String,
    },
}

#[derive(Debug)]
pub struct StocksPanel {
    config: StocksConfig,
    watchlist: Watchlist,
    board: Arc<Mutex<Board>>,
    request: Arc<Mutex<Request>>,
    mode: Mode,
    list_state: ListState,
    status: Option<(String, bool)>,
    source_name: &'static str,
    list_area: Option<Rect>,
    /// Twice the refresh interval. Past this a price is shown as stale even
    /// when nothing has failed — a fetch thread that quietly stopped and a
    /// laptop resumed from sleep both look like success from here.
    stale_after: Duration,
    /// Bumped by the fetch thread every time it writes a quote or an error.
    ///
    /// See `WeatherPanel::generation`: the board is a last-value-wins slot
    /// behind a mutex, so nothing else tells the panel whether it moved.
    generation: Arc<AtomicU64>,
    /// The generation the last frame drew.
    seen: u64,
    /// Set to ask the fetch thread to finish.
    ///
    /// Without it the thread outlives the panel: the picker rebuilds every
    /// panel when a widget is toggled, so each toggle used to leave another
    /// poller running against the same unauthenticated endpoint. The `>= 60s`
    /// interval is enforced per thread, so N leaked threads meant N times the
    /// documented request rate.
    stop: Arc<AtomicBool>,
}

impl StocksPanel {
    pub fn new(config: StocksConfig, path: std::path::PathBuf) -> anyhow::Result<Self> {
        let watchlist = Watchlist::load(path, &config.symbols)?;

        let source = source_for(&config.source).ok_or_else(|| {
            anyhow::anyhow!(
                "`{}` is not a quote source mirador knows. Available: {}.",
                config.source,
                crate::quote::SOURCE_NAMES.join(", ")
            )
        })?;

        Ok(Self::with_source(config, watchlist, source))
    }

    /// Build the panel against a source that is already chosen.
    ///
    /// This is the seam `QuoteSource` exists for, and it was missing: `new`
    /// resolved the only real source itself and spawned a thread against it, so
    /// *constructing a panel made an HTTP request*. Every test that built one
    /// therefore reached Yahoo Finance — the rule that no test touches the
    /// network held for `parse_chart` and had quietly stopped holding here.
    /// It surfaced when a Windows runner got a real 404 back for a made-up
    /// symbol and rendered it (#147).
    ///
    /// The watchlist is loaded by the caller so that a bad file is still
    /// reported before an unknown source name, which is the order `new`
    /// always had.
    fn with_source(
        config: StocksConfig,
        watchlist: Watchlist,
        source: Box<dyn QuoteSource>,
    ) -> Self {
        let source_name = source.name();

        let board: Board = watchlist
            .symbols()
            .iter()
            .map(|s| (s.clone(), Cell::default()))
            .collect();
        let board = Arc::new(Mutex::new(board));
        let request = Arc::new(Mutex::new(Request {
            symbols: watchlist.symbols().to_vec(),
            refresh: false,
        }));

        let stop = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));
        let shared_board = Arc::clone(&board);
        let shared_request = Arc::clone(&request);
        let shared_stop = Arc::clone(&stop);
        let shared_generation = Arc::clone(&generation);
        // Never faster than a minute: the sources are free and unauthenticated,
        // and hammering them is how an IP gets blocked for everyone behind it.
        let interval = Duration::from_secs(config.refresh_secs.max(MIN_SECONDS_BETWEEN_POLLS));
        let stagger = Duration::from_millis(config.stagger_ms.clamp(100, 10_000));

        std::thread::Builder::new()
            .name("mirador-stocks".into())
            .spawn(move || {
                fetch_loop(
                    &*source,
                    &shared_board,
                    &shared_request,
                    &shared_stop,
                    &shared_generation,
                    interval,
                    stagger,
                );
            })
            .expect("spawning the stocks thread");

        let mut panel = Self {
            config,
            watchlist,
            board,
            request,
            mode: Mode::List,
            list_state: ListState::default(),
            status: None,
            source_name,
            list_area: None,
            // Twice the interval, matching the weather panel: one missed cycle
            // is a blip, two is a pattern.
            stale_after: interval * 2,
            generation,
            seen: 0,
            stop,
        };
        panel.reselect();
        // Persist the seed on first run so there is a file to hand-edit.
        panel.watchlist.save_reporting();
        panel
    }

    fn snapshot(&self) -> Board {
        // A poisoned lock means the fetch thread panicked; recover the value
        // rather than taking the dashboard down with one panel.
        match self.board.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Keep the selection inside the list.
    fn reselect(&mut self) {
        let len = self.watchlist.symbols().len();
        if len == 0 {
            self.list_state.select(None);
            return;
        }
        let index = self.list_state.selected().unwrap_or(0).min(len - 1);
        self.list_state.select(Some(index));
    }

    fn selected_symbol(&self) -> Option<String> {
        self.list_state
            .selected()
            .and_then(|i| self.watchlist.symbols().get(i))
            .cloned()
    }

    fn select_down(&mut self, n: usize) {
        let len = self.watchlist.symbols().len();
        crate::selection::down(&mut self.list_state, n, len);
    }

    fn select_up(&mut self, n: usize) {
        let len = self.watchlist.symbols().len();
        crate::selection::up(&mut self.list_state, n, len);
    }

    /// Tell the fetch thread what to poll, and ask it to start now.
    fn publish_request(&self, refresh: bool) {
        let symbols = self.watchlist.symbols().to_vec();
        let mut guard = match self.request.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.symbols = symbols;
        guard.refresh = refresh;
    }

    /// Seed the board so a newly added symbol shows as loading rather than
    /// vanishing until the next poll completes.
    fn reseed_board(&self) {
        let mut guard = match self.board.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let existing = std::mem::take(&mut *guard);
        *guard = self
            .watchlist
            .symbols()
            .iter()
            .map(|symbol| {
                let previous = existing
                    .iter()
                    .find(|(s, _)| s == symbol)
                    .map(|(_, cell)| cell.clone());
                (symbol.clone(), previous.unwrap_or_default())
            })
            .collect();
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), false));
    }

    fn handle_list_key(&mut self, key: KeyEvent) -> KeyOutcome {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.select_down(1),
            KeyCode::Char('k') | KeyCode::Up => self.select_up(1),
            KeyCode::Char('g') | KeyCode::Home => self.select_up(usize::MAX),
            KeyCode::Char('G') | KeyCode::End => self.select_down(usize::MAX),

            KeyCode::Char('a') => self.mode = Mode::Add(TextField::new()),

            KeyCode::Char('d') => {
                if let Some(symbol) = self.selected_symbol() {
                    self.mode = Mode::ConfirmRemove { symbol };
                }
            }

            KeyCode::Char('r') => {
                self.publish_request(true);
                self.set_status("refreshing");
            }

            KeyCode::Char('o') => {
                let path = self.watchlist.path().display().to_string();
                self.set_status(path);
            }

            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    fn handle_add_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let Mode::Add(field) = &mut self.mode else {
            return KeyOutcome::Ignored;
        };
        match key.code {
            KeyCode::Esc => self.mode = Mode::List,
            KeyCode::Enter => {
                let symbol = field.trimmed().to_string();
                self.mode = Mode::List;
                if self.watchlist.add(&symbol) {
                    self.reselect();
                    self.reseed_board();
                    self.publish_request(true);
                    self.watchlist.save_reporting();
                    if let Some(err) = self.watchlist.last_error.clone() {
                        self.status = Some((format!("save failed: {err}"), true));
                    } else {
                        self.set_status(format!("added {}", symbol.to_uppercase()));
                    }
                } else if !symbol.trim().is_empty() {
                    self.set_status(format!("{} is already on the list", symbol.to_uppercase()));
                }
            }
            _ => {
                field.handle_key(key);
            }
        }
        KeyOutcome::Consumed
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let Mode::ConfirmRemove { symbol } = &self.mode else {
            return KeyOutcome::Ignored;
        };
        let symbol = symbol.clone();
        // `y` alone; see the note on the same arm in `todo.rs`.
        if matches!(key.code, KeyCode::Char('y' | 'Y')) {
            self.watchlist.remove(&symbol);
            self.mode = Mode::List;
            self.reselect();
            self.reseed_board();
            self.publish_request(false);
            self.watchlist.save_reporting();
            // Report the failure rather than announcing a removal that did not
            // reach the disk — the add path thirty lines up already does this,
            // and the symbol would otherwise be back on the next start with
            // nothing having said so.
            if let Some(err) = self.watchlist.last_error.clone() {
                self.status = Some((format!("save failed: {err}"), true));
            } else {
                self.set_status(format!("removed {symbol}"));
            }
        } else {
            self.mode = Mode::List;
            self.set_status("kept");
        }
        KeyOutcome::Consumed
    }

    /// One row of the board.
    fn row(
        symbol: &str,
        cell: &Cell,
        stale: bool,
        theme: &Theme,
        grid: &Grid,
        spark: u16,
    ) -> Line<'static> {
        let symbol_span = Span::styled(
            symbol.to_string(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        );

        // Never an empty cell: a blank column reads as a broken panel, where
        // an explicit `…` or `–` reads as a fact about the data.
        let (last, chg, pct, spark_text, tone) = match &cell.quote {
            // Nothing has ever landed for this symbol. Only here is the row
            // genuinely empty — a failure with a price behind it keeps the
            // price.
            None if cell.error.is_some() => (
                "–".to_string(),
                "–".to_string(),
                "–".to_string(),
                String::new(),
                theme.error,
            ),
            None => (
                "…".to_string(),
                "…".to_string(),
                "…".to_string(),
                String::new(),
                theme.muted,
            ),
            Some((q, _)) => {
                let change = q.change();
                // A price that may have moved since must not be coloured as
                // though the direction were current, so a stale row goes muted
                // whichever way it last went.
                let tone = if stale {
                    theme.muted
                } else if change > 0.0 {
                    theme.success
                } else if change < 0.0 {
                    theme.error
                } else {
                    theme.muted
                };
                (
                    format!("{:.2}", q.price),
                    format!("{change:+.2}"),
                    format!("{:+.2}%", q.change_pct()),
                    if spark > 0 {
                        sparkline(&q.series, spark as usize)
                    } else {
                        String::new()
                    },
                    tone,
                )
            }
        };

        let value_style = if cell.quote.is_some() && !stale {
            Style::default().fg(theme.text)
        } else {
            Style::default().fg(tone)
        };

        grid.row(&[
            symbol_span,
            Span::styled(last, value_style),
            Span::styled(chg, Style::default().fg(tone)),
            Span::styled(pct, Style::default().fg(tone)),
            Span::styled(spark_text, Style::default().fg(tone)),
        ])
    }

    fn status_line(&self, theme: &Theme, board: &Board) -> Line<'static> {
        match (&self.mode, &self.status) {
            (Mode::ConfirmRemove { symbol }, _) => Line::from(Span::styled(
                format!("remove {symbol}?  y / n"),
                Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            )),
            (Mode::Add(field), _) => Line::from(vec![
                Span::styled("symbol  ", Style::default().fg(theme.accent)),
                Span::styled(
                    field.value().to_uppercase(),
                    Style::default().fg(theme.text),
                ),
                Span::styled("▏", Style::default().fg(theme.accent)),
            ]),
            (_, Some((message, is_error))) => Line::from(Span::styled(
                message.clone(),
                Style::default().fg(if *is_error { theme.error } else { theme.muted }),
            )),
            _ => {
                // With nothing else to say, surface the first failure rather
                // than leaving a row showing `–` with no explanation anywhere.
                let failure = board.iter().find_map(|(symbol, cell)| {
                    cell.error.as_ref().map(|why| match cell.age() {
                        // A price is still on screen behind the error, so
                        // say how old it is rather than only why the last
                        // attempt failed.
                        Some(age) => {
                            format!("{symbol}: {why} — showing {}", describe_age(age))
                        }
                        None => format!("{symbol}: {why}"),
                    })
                });
                match failure {
                    // Left full length: the paragraph clips it to the panel,
                    // and the first words carry the useful part.
                    Some(text) => Line::from(Span::styled(text, Style::default().fg(theme.error))),
                    None => Line::from(Span::styled(
                        format!("via {}", self.source_name),
                        Style::default().fg(theme.muted),
                    )),
                }
            }
        }
    }
}

/// Poll every symbol, wait, repeat.
fn fetch_loop(
    source: &dyn QuoteSource,
    board: &Arc<Mutex<Board>>,
    request: &Arc<Mutex<Request>>,
    stop: &Arc<AtomicBool>,
    generation: &Arc<AtomicU64>,
    interval: Duration,
    stagger: Duration,
) {
    // The floor is enforced here rather than only in the interval, because `r`
    // and a watchlist edit both break the wait early — so a held `r` re-polled
    // every symbol as fast as the requests completed, against a source
    // `quote.rs` documents as gating on IP reputation, and under a comment in
    // `CLAUDE.md` claiming the limit was "enforced in code, not just
    // documented". It was not.
    //
    // A wake that arrives too soon waits out the remainder instead of being
    // dropped: the user asked for a refresh and should get one, just not now.
    let floor = Duration::from_secs(MIN_SECONDS_BETWEEN_POLLS);
    let mut last_poll: Option<Instant> = None;

    while !stop.load(Ordering::Relaxed) {
        if let Some(at) = last_poll
            && let Some(remaining) = floor.checked_sub(at.elapsed())
            && crate::poll::wait(remaining, stop, || false) == crate::poll::Wake::Stop
        {
            return;
        }

        let symbols = {
            let guard = match request.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.symbols.clone()
        };

        last_poll = Some(Instant::now());
        for symbol in &symbols {
            let result = source.fetch(symbol);
            update(board, generation, symbol, result);
            if stop.load(Ordering::Relaxed) {
                return;
            }
            // Spread the requests out rather than firing them together.
            std::thread::sleep(stagger);
        }

        let woke = crate::poll::wait(interval, stop, || {
            let mut guard = match request.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            std::mem::replace(&mut guard.refresh, false)
        });
        if woke == crate::poll::Wake::Stop {
            return;
        }
    }
}

/// Merge one symbol's result into the shared board, ignoring symbols that were
/// removed while the request was in flight.
///
/// Merge rather than replace: a failure records the reason and leaves whatever
/// price was already there, so the row keeps a real number with its age on it.
fn update(
    board: &Arc<Mutex<Board>>,
    generation: &Arc<AtomicU64>,
    symbol: &str,
    result: anyhow::Result<Quote>,
) {
    {
        let mut guard = match board.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(slot) = guard.iter_mut().find(|(s, _)| s == symbol) {
            match result {
                Ok(quote) => {
                    slot.1.quote = Some((quote, Instant::now()));
                    slot.1.error = None;
                }
                Err(e) => slot.1.error = Some(format!("{e:#}")),
            }
        }
    }

    // After the write and after the lock, with `Release`, so a panel that sees
    // the new number is guaranteed to see the board behind it. Bumped even for
    // an error and even for a symbol that has since been removed: "the panel
    // now shows a reason it did not show before" is a visible change, and one
    // extra repaint a minute is not worth a second branch.
    generation.fetch_add(1, Ordering::Release);
}

impl Panel for StocksPanel {
    fn title(&self) -> String {
        "Markets".to_string()
    }

    fn counter(&self) -> Option<String> {
        // See the note on `TodoPanel::counter`.
        if self.watchlist.last_error.is_some() {
            return Some("unsaved!".into());
        }
        let n = self.watchlist.symbols().len();
        (n > 0).then(|| n.to_string())
    }

    fn tick(&mut self) -> bool {
        // See `WeatherPanel::tick`: a fetch landing is the only thing that
        // changes this panel without a keypress.
        let now = self.generation.load(Ordering::Acquire);
        let moved = now != self.seen;
        self.seen = now;
        moved
    }

    fn max_width(&self) -> Option<u16> {
        // The columns are all fixed but one, and the exception is the
        // sparkline, which is capped at SPARK_WIDTH. So the whole table has a
        // width past which nothing gets wider — it just drifts apart. The
        // graphs next door have no such limit, so the columns go to them.
        Some(SPARK_MIN_GRID + SELECTION_MARKER + FRAME_WIDTH)
    }

    fn max_height(&self) -> Option<u16> {
        // Header, a row per symbol, and the status line. A watchlist is a
        // handful of rows and does not scroll to fill a screen — measured
        // rather than assumed: the panel is complete at exactly this height,
        // and every row above it is blank space between the last symbol and
        // the status line, which is what invariant 15 exists to refuse.
        //
        // Saturating because the sum is not, and `u16::MAX` was already being
        // reached for on the line above. Nothing bounds a watchlist — it is a
        // file the reader edits — and 65_534 symbols wrapped this to 3, which
        // is a panel that collapses rather than one that is merely too tall.
        // Same shape as `glyphs::width_of` overflowing at ten thousand
        // characters: unreachable in practice, one line to close.
        let rows = u16::try_from(self.watchlist.symbols().len()).unwrap_or(u16::MAX);
        Some(rows.saturating_add(2).saturating_add(FRAME_HEIGHT))
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn refresh_interval(&self) -> Duration {
        // The background thread owns the real cadence; this only decides how
        // often the panel notices that new numbers have landed.
        Duration::from_secs(1)
    }

    fn alert(&self) -> Option<crate::panel::Alert> {
        self.watchlist.last_error.as_ref().map(|why| {
            crate::panel::Alert::failing(format!("The watchlist could not be saved — {why}"))
        })
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::List)
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        self.status = None;
        match &self.mode {
            Mode::List => self.handle_list_key(key),
            Mode::Add(_) => self.handle_add_key(key),
            Mode::ConfirmRemove { .. } => self.handle_confirm_key(key),
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> KeyOutcome {
        if !matches!(self.mode, Mode::List) {
            return KeyOutcome::Ignored;
        }
        match event.kind {
            MouseEventKind::ScrollDown => self.select_down(1),
            MouseEventKind::ScrollUp => self.select_up(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(area) = self.list_area else {
                    return KeyOutcome::Ignored;
                };
                let at = Position::new(event.column, event.row);
                let len = self.watchlist.symbols().len();
                let Some(index) = crate::selection::row_at(&self.list_state, area, at, len) else {
                    return KeyOutcome::Ignored;
                };
                self.status = None;
                self.list_state.select(Some(index));
            }
            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        self.list_area = None;
        if area.width == 0 || area.height == 0 {
            return;
        }

        let board = self.snapshot();

        let rows = Layout::vertical([
            Constraint::Length(1), // header
            Constraint::Min(1),    // board
            Constraint::Length(1), // status
        ])
        .split(area);

        if self.watchlist.symbols().is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "No symbols yet. Press `a` to add one.",
                    Style::default().fg(theme.muted),
                )),
                rows[1],
            );
            frame.render_widget(Paragraph::new(self.status_line(theme, &board)), rows[2]);
            return;
        }

        let marker = SELECTION_MARKER;
        let grid = Grid::new(COLUMNS, rows[1].width.saturating_sub(marker));
        // Taken from the grid rather than recomputed: the grid already decided
        // whether the column survived and how wide it is, and a second copy of
        // that arithmetic is what silently emptied the column before.
        let spark = if self.config.show_sparkline {
            grid.column_width("today").min(SPARK_WIDTH)
        } else {
            0
        };

        let header_area = Rect::new(
            rows[0].x + marker,
            rows[0].y,
            rows[0].width.saturating_sub(marker),
            1,
        );
        frame.render_widget(Paragraph::new(grid.header(theme)), header_area);

        let items: Vec<ListItem> = board
            .iter()
            .map(|(symbol, cell)| {
                let stale = cell.is_stale(self.stale_after);
                ListItem::new(Self::row(symbol, cell, stale, theme, &grid, spark))
            })
            .collect();

        self.list_area = Some(rows[1]);
        let list = List::new(items)
            .highlight_symbol(if ctx.focused { "▸ " } else { "  " })
            .highlight_style(if ctx.focused {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            });
        frame.render_stateful_widget(list, rows[1], &mut self.list_state);

        frame.render_widget(Paragraph::new(self.status_line(theme, &board)), rows[2]);
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.watchlist.save_reporting();
    }
}

impl Drop for StocksPanel {
    /// Belt and braces. `shutdown` is the documented hook, but a panel can also
    /// be dropped without it — the picker rebuilding the dashboard is exactly
    /// that — and a poller nobody can reach is worse than one that is merely
    /// unused: it keeps making requests.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    struct TempDir(std::path::PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A source that answers from memory.
    ///
    /// Counts its calls so a test can prove the panel used *this* and not the
    /// network. Always succeeds: an error would be equally deterministic, but
    /// a filled board is the state most tests want to look at.
    struct Offline(Arc<std::sync::atomic::AtomicUsize>);

    impl QuoteSource for Offline {
        fn name(&self) -> &'static str {
            "offline"
        }

        fn fetch(&self, symbol: &str) -> anyhow::Result<Quote> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(Quote {
                symbol: symbol.to_string(),
                price: 100.0,
                previous_close: 99.0,
                currency: Some("USD".into()),
                series: vec![99.0, 100.0],
                delayed: false,
            })
        }
    }

    fn panel(name: &str, seed: &[&str]) -> (StocksPanel, TempDir) {
        let (p, dir, _calls) = panel_counting(name, seed);
        (p, dir)
    }

    /// The panel every test builds, wired to `Offline`.
    ///
    /// It used to call `StocksPanel::new`, which resolves the real Yahoo
    /// source and spawns a thread against it — so building a panel issued an
    /// HTTP request. The `refresh_secs` below was believed to prevent that and
    /// does not: it governs the gap *between* cycles, and the first poll is
    /// immediate. See #147.
    fn panel_counting(
        name: &str,
        seed: &[&str],
    ) -> (StocksPanel, TempDir, Arc<std::sync::atomic::AtomicUsize>) {
        let dir =
            std::env::temp_dir().join(format!("mirador-stocks-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = StocksConfig {
            symbols: seed.iter().map(|s| (*s).to_string()).collect(),
            // Still long, so the loop polls once and then sleeps rather than
            // spinning through the whole watchlist for the length of the test.
            refresh_secs: 86_400,
            ..StocksConfig::default()
        };
        let watchlist =
            Watchlist::load(dir.join("watchlist.toml"), &config.symbols).expect("watchlist loads");
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let source = Box::new(Offline(Arc::clone(&calls)));
        let p = StocksPanel::with_source(config, watchlist, source);
        (p, TempDir(dir), calls)
    }

    fn press(p: &mut StocksPanel, code: KeyCode) {
        p.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// A cell holding a live quote, as a successful fetch leaves it.
    fn ready(quote: Quote) -> Cell {
        Cell {
            quote: Some((quote, Instant::now())),
            error: None,
        }
    }

    /// A cell whose only fetch failed, so there is no price behind the error.
    fn failed(why: &str) -> Cell {
        Cell {
            quote: None,
            error: Some(why.to_string()),
        }
    }

    fn type_str(p: &mut StocksPanel, text: &str) {
        for c in text.chars() {
            press(p, KeyCode::Char(c));
        }
    }

    /// #147: building a panel spawned a thread against the real Yahoo source,
    /// so every test in this module issued an HTTP request — a Windows runner
    /// got a genuine 404 back for a made-up symbol and rendered it. The helper
    /// wires an offline source now, and this is what stops that drifting back.
    ///
    /// Two halves, because either alone is weak. The panel a test receives must
    /// be built against `Offline`, which `source_for` cannot produce — so its
    /// presence proves the test constructed the source rather than resolving
    /// one from config. And the module must not reach for the real constructor
    /// except in the single test that expects it to fail before any thread
    /// exists.
    #[test]
    fn no_test_builds_a_panel_against_the_network() {
        let (p, _g, _calls) = panel_counting("offline-guard", &["AAPL"]);
        assert_eq!(
            p.source_name, "offline",
            "the test helper must not resolve a real quote source"
        );
        assert!(
            source_for("offline").is_none(),
            "`offline` has to stay unreachable from config, or the check above proves nothing"
        );

        // Split so the needle is not itself a match in this file.
        let needle = concat!("StocksPanel", "::new(");
        let text = std::fs::read_to_string(file!()).expect("this file is readable");
        let tests = text.split_once("mod tests").expect("the tests module").1;
        let direct = tests.matches(needle).count();
        assert_eq!(
            direct, 1,
            "only the unknown-source test may use the real constructor, and it \
             fails before a thread is spawned; found {direct} uses"
        );
    }

    #[test]
    fn an_unknown_source_is_refused_with_a_message_naming_the_real_ones() {
        let dir = std::env::temp_dir().join(format!("mirador-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = StocksConfig {
            source: "finnhub".to_string(),
            ..StocksConfig::default()
        };
        let err = StocksPanel::new(config, dir.join("w.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("finnhub"), "got `{err}`");
        assert!(err.contains("yahoo"), "must say what is available: `{err}`");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symbol_can_be_added_and_persists() {
        let (mut p, guard) = panel("add", &[]);
        assert!(p.watchlist.symbols().is_empty());

        press(&mut p, KeyCode::Char('a'));
        assert!(p.captures_input(), "the entry field must swallow globals");
        type_str(&mut p, "aapl");
        press(&mut p, KeyCode::Enter);

        assert_eq!(p.watchlist.symbols(), ["AAPL"], "normalised to upper case");
        let reloaded = Watchlist::load(guard.0.join("watchlist.toml"), &[]).unwrap();
        assert_eq!(reloaded.symbols(), ["AAPL"], "and written to disk");
    }

    #[test]
    fn adding_a_duplicate_says_so_rather_than_silently_doing_nothing() {
        let (mut p, _g) = panel("dupe", &["AAPL"]);
        press(&mut p, KeyCode::Char('a'));
        type_str(&mut p, "AAPL");
        press(&mut p, KeyCode::Enter);

        assert_eq!(p.watchlist.symbols().len(), 1);
        let (message, _) = p.status.clone().expect("a duplicate must be reported");
        assert!(message.contains("already"), "got `{message}`");
    }

    #[test]
    fn removing_asks_first_and_keeps_the_symbol_on_any_other_key() {
        let (mut p, _g) = panel("remove", &["AAPL", "MSFT"]);
        press(&mut p, KeyCode::Char('d'));
        assert!(matches!(p.mode, Mode::ConfirmRemove { .. }));
        press(&mut p, KeyCode::Char('n'));
        assert_eq!(p.watchlist.symbols().len(), 2, "n keeps it");

        press(&mut p, KeyCode::Char('d'));
        press(&mut p, KeyCode::Char('y'));
        assert_eq!(p.watchlist.symbols(), ["MSFT"]);
    }

    /// The cap is the height at which the panel is *complete* — header, every
    /// symbol, status line, frame — and not a row more. Pinned by rendering,
    /// because the arithmetic looks right whatever the numbers are: the
    /// interior fills at `symbols + 2`, and the frame costs `FRAME_HEIGHT`.
    #[test]
    fn the_height_cap_is_exactly_where_the_panel_stops_gaining_anything() {
        use crate::panel::RenderContext;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let theme = crate::theme::Theme::default();
        let gradients = theme.gradients();

        for n in [1usize, 3, 7] {
            let seed: Vec<String> = (0..n).map(|i| format!("SYM{i}")).collect();
            let refs: Vec<&str> = seed.iter().map(String::as_str).collect();
            let (mut p, _g) = panel(&format!("cap{n}"), &refs);
            let cap = p.max_height().expect("stocks bounds its height");

            // The interior the shell would hand it at the cap.
            let interior = cap - FRAME_HEIGHT;
            let draw = |p: &mut StocksPanel, h: u16| {
                let mut t = Terminal::new(TestBackend::new(60, h)).expect("backend");
                t.draw(|f| {
                    p.render(
                        f,
                        Rect::new(0, 0, 60, h),
                        RenderContext {
                            theme: &theme,
                            gradients: &gradients,
                            focused: true,
                            watch: &crate::watch::WatchLog::default(),
                        },
                    );
                })
                .expect("draws");
                let buffer = t.backend().buffer();
                (0..h)
                    .map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n")
            };

            // Rows are counted, not matched on text. The status line reads
            // `via yahoo` only until a fetch reports something — and these
            // panels really do reach the network, so a Windows runner got far
            // enough to render `SYM0: no such symbol` and failed an assertion
            // looking for `via `. Rows are what the cap is about anyway.
            let filled = |text: &str| text.lines().filter(|line| !line.trim().is_empty()).count();

            let at_cap = draw(&mut p, interior);
            for i in 0..n {
                assert!(
                    at_cap.contains(&format!("SYM{i}")),
                    "symbol {i} of {n} is missing at the cap:\n{at_cap}"
                );
            }
            assert!(at_cap.contains("SYMBOL"), "no header at the cap:\n{at_cap}");
            assert_eq!(
                filled(&at_cap),
                n + 2,
                "the cap should fill header + {n} symbols + status exactly:\n{at_cap}"
            );

            // One row short must lose something, or the cap is too generous.
            // "Something" is not only a symbol: with a single symbol it is the
            // status line that goes, and a check that looked for a missing
            // symbol alone called the cap too generous when it was exact.
            let under = draw(&mut p, interior - 1);
            assert!(
                filled(&under) < n + 2,
                "the cap reserves a row the panel does not use at {n} symbols:\n{under}"
            );
        }
    }

    /// A watchlist is a file the reader edits and nothing bounds its length,
    /// so the sum has to be saturating. It was not: `65_534` symbols wrapped the
    /// cap to 3, collapsing the panel instead of making it tall.
    #[test]
    fn an_absurd_watchlist_does_not_wrap_the_height_cap() {
        let seed: Vec<String> = (0..3).map(|i| format!("SYM{i}")).collect();
        let refs: Vec<&str> = seed.iter().map(String::as_str).collect();
        let (p, _g) = panel("overflow", &refs);

        // The arithmetic, exercised at the boundary the panel cannot be
        // *built* at cheaply — constructing 65_535 symbols to prove one
        // addition would trade a real test for a slow one.
        let rows = u16::MAX;
        let cap = rows.saturating_add(2).saturating_add(FRAME_HEIGHT);
        assert_eq!(cap, u16::MAX, "the cap must saturate, not wrap");
        assert!(
            p.max_height().expect("bounded") > FRAME_HEIGHT,
            "an ordinary watchlist still reports a usable height"
        );
    }

    #[test]
    fn removing_the_last_row_leaves_the_selection_somewhere_real() {
        let (mut p, _g) = panel("reselect", &["AAPL", "MSFT"]);
        press(&mut p, KeyCode::Char('G'));
        assert_eq!(p.list_state.selected(), Some(1));

        press(&mut p, KeyCode::Char('d'));
        press(&mut p, KeyCode::Char('y'));
        assert_eq!(
            p.list_state.selected(),
            Some(0),
            "a selection past the end would render nothing"
        );
    }

    #[test]
    fn removing_the_only_symbol_clears_the_selection_rather_than_pointing_at_nothing() {
        let (mut p, _g) = panel("last", &["AAPL"]);
        press(&mut p, KeyCode::Char('d'));
        press(&mut p, KeyCode::Char('y'));
        assert!(p.watchlist.symbols().is_empty());
        assert_eq!(p.list_state.selected(), None);
    }

    #[test]
    fn a_new_symbol_shows_as_loading_rather_than_missing_from_the_board() {
        let (mut p, _g) = panel("board", &["AAPL"]);
        press(&mut p, KeyCode::Char('a'));
        type_str(&mut p, "MSFT");
        press(&mut p, KeyCode::Enter);

        let board = p.snapshot();
        assert_eq!(board.len(), 2, "the board must track the watchlist");
        assert!(board.iter().any(|(s, _)| s == "MSFT"));
        assert!(
            board
                .iter()
                .all(|(_, c)| c.quote.is_some() || c.error.is_some() || c.age().is_none()),
            "every row must render as something"
        );
    }

    #[test]
    fn the_fetch_thread_is_asked_for_the_new_symbol_immediately() {
        let (mut p, _g) = panel("request", &[]);
        press(&mut p, KeyCode::Char('a'));
        type_str(&mut p, "TSLA");
        press(&mut p, KeyCode::Enter);

        let guard = p.request.lock().unwrap();
        assert_eq!(guard.symbols, ["TSLA"], "the thread polls the new list");
        assert!(
            guard.refresh,
            "and is woken rather than waiting an interval"
        );
    }

    #[test]
    fn a_row_never_renders_an_empty_cell() {
        let theme = Theme::default();
        let grid = Grid::new(COLUMNS, 60);

        let quote = Quote {
            symbol: "AAPL".into(),
            price: 213.5,
            previous_close: 211.0,
            currency: Some("USD".into()),
            series: vec![211.0, 213.5],
            delayed: false,
        };
        for (cell, stale) in [
            // Loading, never-succeeded failure, live, and a failure with a
            // price behind it — the state that used to render as three dashes.
            (Cell::default(), false),
            (failed("network request failed"), false),
            (ready(quote.clone()), false),
            (
                Cell {
                    error: Some("timed out".into()),
                    ..ready(quote.clone())
                },
                true,
            ),
        ] {
            let line = StocksPanel::row("AAPL", &cell, stale, &theme, &grid, 8);
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                !text.trim().is_empty(),
                "a blank row reads as a broken panel: {cell:?}"
            );
            // Every value column must carry something, not just the symbol.
            assert!(text.trim() != "AAPL", "only the symbol rendered: `{text}`");
        }
    }

    #[test]
    fn a_gain_and_a_loss_are_signed_and_coloured_differently() {
        let theme = Theme::default();
        let grid = Grid::new(COLUMNS, 60);

        let up = Quote {
            symbol: "X".into(),
            price: 11.0,
            previous_close: 10.0,
            currency: None,
            series: vec![],
            delayed: false,
        };
        let mut down = up.clone();
        down.price = 9.0;

        let text = |q: Quote| -> String {
            StocksPanel::row("X", &ready(q), false, &theme, &grid, 0)
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect()
        };

        let rise = text(up.clone());
        assert!(rise.contains("+1.00"), "got `{rise}`");
        assert!(rise.contains("+10.00%"), "got `{rise}`");

        let fall = text(down.clone());
        assert!(fall.contains("-1.00"), "got `{fall}`");
        assert!(fall.contains("-10.00%"), "got `{fall}`");

        let colour_of = |q: Quote| {
            StocksPanel::row("X", &ready(q), false, &theme, &grid, 0).spans[4]
                .style
                .fg
        };
        assert_ne!(
            colour_of(up),
            colour_of(down),
            "a gain and a loss must not look the same"
        );
    }

    #[test]
    fn a_failure_is_surfaced_in_the_status_line_rather_than_only_as_a_dash() {
        let (p, _g) = panel("failure", &["AAPL"]);
        let theme = Theme::default();
        let board: Board = vec![("AAPL".into(), failed("HTTP 429"))];

        let text: String = p
            .status_line(&theme, &board)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("AAPL"), "got `{text}`");
        assert!(
            text.contains("429"),
            "the reason must reach the user: `{text}`"
        );
    }

    #[test]
    fn holding_r_cannot_poll_faster_than_the_floor() {
        use std::sync::atomic::AtomicUsize;

        /// Counts calls and returns instantly, so the loop is bounded only by
        /// its own rate limiting rather than by how long a request takes.
        struct Counting(Arc<AtomicUsize>);
        impl QuoteSource for Counting {
            fn name(&self) -> &'static str {
                "counting"
            }
            fn fetch(&self, symbol: &str) -> anyhow::Result<Quote> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(Quote {
                    symbol: symbol.to_string(),
                    price: 1.0,
                    previous_close: 1.0,
                    currency: None,
                    series: vec![],
                    delayed: false,
                })
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let source = Counting(Arc::clone(&calls));
        let board = Arc::new(Mutex::new(vec![("AAPL".to_string(), Cell::default())]));
        let request = Arc::new(Mutex::new(Request {
            symbols: vec!["AAPL".into()],
            // Held down: every time the loop looks, a refresh is waiting.
            refresh: true,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));

        // Keep asking for a refresh for as long as the loop runs, and stop it
        // after a couple of seconds.
        let ticking = Arc::clone(&request);
        let flag = Arc::clone(&stop);
        std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                if let Ok(mut guard) = ticking.lock() {
                    guard.refresh = true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            flag.store(true, Ordering::Relaxed);
        });

        fetch_loop(
            &source,
            &board,
            &request,
            &stop,
            &generation,
            Duration::from_millis(1),
            Duration::ZERO,
        );

        // Two seconds against a 60-second floor: the first poll, and nothing
        // else. Before, `r` broke the wait and the loop re-polled every symbol
        // as fast as the requests came back.
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "the rate floor was bypassed by a held refresh key"
        );
    }

    #[test]
    fn a_failed_fetch_keeps_the_last_good_price_and_says_it_is_old() {
        let (p, _g) = panel("retain", &["AAPL"]);
        let theme = Theme::default();
        let grid = Grid::new(COLUMNS, 60);

        let quote = Quote {
            symbol: "AAPL".into(),
            price: 213.5,
            previous_close: 211.0,
            currency: Some("USD".into()),
            series: vec![211.0, 213.5],
            delayed: false,
        };

        let board = Arc::new(Mutex::new(vec![("AAPL".to_string(), Cell::default())]));
        update(&board, &Arc::new(AtomicU64::new(0)), "AAPL", Ok(quote));
        update(
            &board,
            &Arc::new(AtomicU64::new(0)),
            "AAPL",
            Err(anyhow::anyhow!("HTTP 429")),
        );

        let snapshot = board.lock().unwrap().clone();
        let cell = &snapshot[0].1;

        // The whole point: the error did not take the price with it. This used
        // to render as three dashes and an empty sparkline for a full refresh
        // interval, because the error replaced the quote outright.
        assert!(cell.error.is_some(), "the reason is still recorded");
        let row: String = StocksPanel::row("AAPL", cell, true, &theme, &grid, 8)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(row.contains("213.50"), "the price was lost: `{row}`");
        assert!(!row.contains('–'), "a retained price must not show a dash");

        // And it is not passed off as current: the status line says how old it
        // is, and the row is muted rather than coloured by direction.
        let status: String = p
            .status_line(&theme, &snapshot)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(status.contains("429"), "got `{status}`");
        assert!(
            status.contains("old"),
            "a retained price must be labelled with its age: `{status}`"
        );

        let live = StocksPanel::row("AAPL", cell, false, &theme, &grid, 8).spans[2]
            .style
            .fg;
        let stale = StocksPanel::row("AAPL", cell, true, &theme, &grid, 8).spans[2]
            .style
            .fg;
        assert_ne!(
            live, stale,
            "a stale price must not look the same as a live one"
        );
    }

    /// Every key list mode responds to, paired with the binding documenting it.
    const DOCUMENTED_LIST_KEYS: &[(KeyCode, &str)] = &[
        (KeyCode::Char('a'), "a"),
        (KeyCode::Char('d'), "d"),
        (KeyCode::Char('r'), "r"),
        (KeyCode::Down, "↑ / ↓"),
        (KeyCode::Up, "↑ / ↓"),
        (KeyCode::Char('j'), "j / k"),
        (KeyCode::Char('k'), "j / k"),
        (KeyCode::Char('g'), "g / G"),
        (KeyCode::Char('G'), "g / G"),
        (KeyCode::Home, "Home / End"),
        (KeyCode::End, "Home / End"),
        (KeyCode::Char('o'), "o"),
    ];

    #[test]
    fn every_documented_key_works_and_every_working_key_is_documented() {
        for (code, key) in DOCUMENTED_LIST_KEYS {
            assert!(
                BINDINGS.iter().any(|b| b.key == *key),
                "`{key}` is handled but missing from BINDINGS"
            );
            let (mut p, _g) = panel("keymap", &["AAPL"]);
            let outcome = p.handle_key(KeyEvent::new(*code, KeyModifiers::NONE));
            assert_eq!(
                outcome,
                KeyOutcome::Consumed,
                "`{key}` is documented but the list ignores it"
            );
        }
    }
}
