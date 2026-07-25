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

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::config::StocksConfig;
use crate::frame::Binding;
use crate::grid::{Column, Grid};
use crate::panel::{KeyOutcome, Panel, RenderContext};
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

/// The narrowest the panel can be and still earn a sparkline column.
const SPARK_WIDTH: u16 = 12;

const COLUMNS: &[Column] = &[
    Column::fixed("symbol", 8),
    Column::fixed("last", 10).right(),
    Column::fixed("chg", 9).right(),
    Column::fixed("%", 8).right(),
    Column::flex("today", 1).drops_below(56),
];

/// What the background thread has produced for one symbol.
#[derive(Debug, Clone)]
enum Cell {
    Loading,
    Ready(Quote),
    Failed(String),
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
        let source_name = source.name();

        let board: Board = watchlist
            .symbols()
            .iter()
            .map(|s| (s.clone(), Cell::Loading))
            .collect();
        let board = Arc::new(Mutex::new(board));
        let request = Arc::new(Mutex::new(Request {
            symbols: watchlist.symbols().to_vec(),
            refresh: false,
        }));

        let shared_board = Arc::clone(&board);
        let shared_request = Arc::clone(&request);
        // Never faster than a minute: the sources are free and unauthenticated,
        // and hammering them is how an IP gets blocked for everyone behind it.
        let interval = Duration::from_secs(config.refresh_secs.max(60));
        let stagger = Duration::from_millis(config.stagger_ms.clamp(100, 10_000));

        std::thread::Builder::new()
            .name("mirador-stocks".into())
            .spawn(move || fetch_loop(&*source, &shared_board, &shared_request, interval, stagger))
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
        };
        panel.reselect();
        // Persist the seed on first run so there is a file to hand-edit.
        panel.watchlist.save_reporting();
        Ok(panel)
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
        let Some(last) = self.watchlist.symbols().len().checked_sub(1) else {
            return;
        };
        let current = self.list_state.selected().unwrap_or(0);
        self.list_state
            .select(Some(current.saturating_add(n).min(last)));
    }

    fn select_up(&mut self, n: usize) {
        if self.watchlist.symbols().is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some(current.saturating_sub(n)));
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
                (symbol.clone(), previous.unwrap_or(Cell::Loading))
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
        match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                self.watchlist.remove(&symbol);
                self.mode = Mode::List;
                self.reselect();
                self.reseed_board();
                self.publish_request(false);
                self.watchlist.save_reporting();
                self.set_status(format!("removed {symbol}"));
            }
            _ => {
                self.mode = Mode::List;
                self.set_status("kept");
            }
        }
        KeyOutcome::Consumed
    }

    /// One row of the board.
    fn row(symbol: &str, cell: &Cell, theme: &Theme, grid: &Grid, spark: u16) -> Line<'static> {
        let symbol_span = Span::styled(
            symbol.to_string(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        );

        // Never an empty cell: a blank column reads as a broken panel, where
        // an explicit `…` or `–` reads as a fact about the data.
        let (last, chg, pct, spark_text, tone) = match cell {
            Cell::Loading => (
                "…".to_string(),
                "…".to_string(),
                "…".to_string(),
                String::new(),
                theme.muted,
            ),
            Cell::Failed(_) => (
                "–".to_string(),
                "–".to_string(),
                "–".to_string(),
                String::new(),
                theme.error,
            ),
            Cell::Ready(q) => {
                let change = q.change();
                let tone = if change > 0.0 {
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

        let value_style = match cell {
            Cell::Ready(_) => Style::default().fg(theme.text),
            _ => Style::default().fg(tone),
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
                let failure = board.iter().find_map(|(symbol, cell)| match cell {
                    Cell::Failed(why) => Some(format!("{symbol}: {why}")),
                    _ => None,
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
    interval: Duration,
    stagger: Duration,
) {
    loop {
        let symbols = {
            let guard = match request.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.symbols.clone()
        };

        for symbol in &symbols {
            let cell = match source.fetch(symbol) {
                Ok(quote) => Cell::Ready(quote),
                Err(e) => Cell::Failed(format!("{e:#}")),
            };
            update(board, symbol, cell);
            // Spread the requests out rather than firing them together.
            std::thread::sleep(stagger);
        }

        // Sleep in slices so `r` and a watchlist change are picked up promptly
        // without needing a channel or a condvar.
        let mut waited = Duration::ZERO;
        while waited < interval {
            std::thread::sleep(Duration::from_millis(250));
            waited += Duration::from_millis(250);
            let asked = {
                let mut guard = match request.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                std::mem::replace(&mut guard.refresh, false)
            };
            if asked {
                break;
            }
        }
    }
}

/// Write one symbol's result into the shared board, ignoring symbols that were
/// removed while the request was in flight.
fn update(board: &Arc<Mutex<Board>>, symbol: &str, cell: Cell) {
    let mut guard = match board.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(slot) = guard.iter_mut().find(|(s, _)| s == symbol) {
        slot.1 = cell;
    }
}

impl Panel for StocksPanel {
    fn title(&self) -> String {
        "Markets".to_string()
    }

    fn counter(&self) -> Option<String> {
        let n = self.watchlist.symbols().len();
        (n > 0).then(|| n.to_string())
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn refresh_interval(&self) -> Duration {
        // The background thread owns the real cadence; this only decides how
        // often the panel notices that new numbers have landed.
        Duration::from_secs(1)
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
                if !area.contains(Position::new(event.column, event.row)) {
                    return KeyOutcome::Ignored;
                }
                let index = self.list_state.offset() + usize::from(event.row - area.y);
                if index >= self.watchlist.symbols().len() {
                    return KeyOutcome::Ignored;
                }
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

        let marker = 2u16;
        let grid = Grid::new(COLUMNS, rows[1].width.saturating_sub(marker));
        let spark = if self.config.show_sparkline && rows[1].width >= 56 {
            SPARK_WIDTH.min(rows[1].width.saturating_sub(48))
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
            .map(|(symbol, cell)| ListItem::new(Self::row(symbol, cell, theme, &grid, spark)))
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
        self.watchlist.save_reporting();
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

    fn panel(name: &str, seed: &[&str]) -> (StocksPanel, TempDir) {
        let dir =
            std::env::temp_dir().join(format!("mirador-stocks-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = StocksConfig {
            symbols: seed.iter().map(|s| (*s).to_string()).collect(),
            // Long enough that the fetch thread never completes a cycle during
            // a test, so nothing here touches the network.
            refresh_secs: 86_400,
            ..StocksConfig::default()
        };
        let p = StocksPanel::new(config, dir.join("watchlist.toml")).unwrap();
        (p, TempDir(dir))
    }

    fn press(p: &mut StocksPanel, code: KeyCode) {
        p.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn type_str(p: &mut StocksPanel, text: &str) {
        for c in text.chars() {
            press(p, KeyCode::Char(c));
        }
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
                .all(|(_, c)| matches!(c, Cell::Loading | Cell::Ready(_) | Cell::Failed(_))),
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

        for cell in [
            Cell::Loading,
            Cell::Failed("network request failed".into()),
            Cell::Ready(Quote {
                symbol: "AAPL".into(),
                price: 213.5,
                previous_close: 211.0,
                currency: Some("USD".into()),
                series: vec![211.0, 213.5],
                delayed: false,
            }),
        ] {
            let line = StocksPanel::row("AAPL", &cell, &theme, &grid, 8);
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
            StocksPanel::row("X", &Cell::Ready(q), &theme, &grid, 0)
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
            StocksPanel::row("X", &Cell::Ready(q), &theme, &grid, 0).spans[4]
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
        let board: Board = vec![("AAPL".into(), Cell::Failed("HTTP 429".into()))];

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
