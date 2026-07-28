//! Notes: a list of what you wrote down, beside the note you are looking at.
//!
//! Master-detail, the shape a mail client uses. The list alone is not enough —
//! a note's whole value is the text inside it, and making the reader press a
//! key to see any of it turns "glance at the dashboard" into "operate the
//! dashboard". So the body is always on screen for the selected note.
//!
//! The split follows the panel: side by side when there is width for both, and
//! stacked when there is not, rather than squeezing two unreadable columns.

use jiff::civil::Date;
use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::config::NotesConfig;
use crate::frame::Binding;
use crate::grid::{Column, Grid, wrapped_height};
use crate::note::{Note, NoteStore};
use crate::panel::{KeyOutcome, Panel, RenderContext};
use crate::textarea::TextArea;
use crate::textfield::TextField;
use crate::theme::Theme;

/// Every key the list responds to.
///
/// One declaration feeds the border hint, the status bar and the help overlay,
/// and `every_documented_key_works_and_every_working_key_is_documented` fails
/// if this list and the code drift apart.
const BINDINGS: &[Binding] = &[
    Binding::primary("a", "new"),
    Binding::primary("↵", "edit"),
    Binding::primary("d", "delete"),
    Binding::extra("↑ / ↓", "move selection"),
    Binding::extra("j / k", "move selection"),
    Binding::extra("g / G", "first / last"),
    Binding::extra("Home / End", "first / last"),
    Binding::extra("PgUp / PgDn", "scroll the note"),
    Binding::extra("e", "edit"),
    Binding::extra("n", "new"),
    Binding::extra("/", "search"),
    Binding::extra("Esc", "clear search"),
    Binding::extra("o", "show file path"),
];

/// Columns of the note list. The date is right-aligned so the dates line up.
const COLUMNS: &[Column] = &[
    Column::flex("title", 1),
    Column::fixed("date", 6).right().drops_below(24),
];

/// Which field the edit form is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Body,
}

/// The form used for both new and existing notes.
#[derive(Debug)]
struct EditForm {
    /// `None` for a note being created.
    id: Option<u64>,
    title: TextField,
    body: TextArea,
    field: Field,
    error: Option<String>,
}

impl EditForm {
    fn blank() -> Self {
        Self {
            id: None,
            title: TextField::new(),
            body: TextArea::new(),
            field: Field::Title,
            error: None,
        }
    }

    fn from_note(note: &Note) -> Self {
        Self {
            id: Some(note.id),
            title: TextField::with_value(note.title.clone()),
            body: TextArea::with_value(&note.body),
            field: Field::Title,
            error: None,
        }
    }
}

#[derive(Debug)]
enum Mode {
    List,
    Edit(Box<EditForm>),
    Search(TextField),
    ConfirmDelete { id: u64, title: String },
}

pub struct NotesPanel {
    store: NoteStore,
    config: NotesConfig,
    filter: String,
    mode: Mode,
    /// Note ids in display order, recomputed whenever the list changes.
    view: Vec<u64>,
    list_state: ListState,
    /// First body line drawn in the detail pane.
    body_scroll: u16,
    status: Option<(String, bool)>,
    today: Date,
    /// Where the rows and the body were last drawn, so the wheel can act on
    /// whichever one the pointer is over.
    list_area: Option<Rect>,
    detail_area: Option<Rect>,
    /// The body's rectangle as last drawn, so scrolling can clamp against the
    /// wrapped height rather than the number of newlines.
    body_area: Option<Rect>,
}

impl NotesPanel {
    pub fn new(config: NotesConfig, path: std::path::PathBuf) -> anyhow::Result<Self> {
        let today = jiff::Zoned::now().date();
        let store = NoteStore::load_or_seed(path, today)?;
        let mut panel = Self {
            store,
            config,
            filter: String::new(),
            mode: Mode::List,
            view: Vec::new(),
            list_state: ListState::default(),
            body_scroll: 0,
            status: None,
            today,
            list_area: None,
            detail_area: None,
            body_area: None,
        };
        panel.refresh_view();
        Ok(panel)
    }

    /// Recompute the ordered id list, keeping the selection on the same note
    /// where possible so that typing a search does not move the cursor.
    fn refresh_view(&mut self) {
        let previous = self.selected_id();
        self.view = self.store.view(&self.filter);

        let index = previous
            .and_then(|id| self.view.iter().position(|v| *v == id))
            .or_else(|| (!self.view.is_empty()).then_some(0));
        self.list_state
            .select(index.filter(|_| !self.view.is_empty()));

        // A different note under the cursor means the body pane is showing
        // something else now, and an inherited scroll offset would drop the
        // reader into the middle of it.
        if self.selected_id() != previous {
            self.body_scroll = 0;
        }
    }

    fn selected_id(&self) -> Option<u64> {
        self.list_state
            .selected()
            .and_then(|index| self.view.get(index))
            .copied()
    }

    fn selected(&self) -> Option<&Note> {
        self.selected_id().and_then(|id| self.store.get(id))
    }

    fn select_to(&mut self, index: usize) {
        let Some(last) = self.view.len().checked_sub(1) else {
            return;
        };
        let index = index.min(last);
        if self.list_state.selected() != Some(index) {
            self.list_state.select(Some(index));
            self.body_scroll = 0;
        }
    }

    fn select_down(&mut self, n: usize) {
        let current = self.list_state.selected().unwrap_or(0);
        self.select_to(current.saturating_add(n));
    }

    fn select_up(&mut self, n: usize) {
        let current = self.list_state.selected().unwrap_or(0);
        self.select_to(current.saturating_sub(n));
    }

    fn scroll_body(&mut self, delta: i16) {
        // Clamped so the body cannot be scrolled off into blank space, against
        // the height it actually occupies once wrapped rather than its count of
        // newlines. Before the first draw there is no width to wrap against, so
        // fall back to logical lines.
        let area = self.body_area;
        let height = self.selected().map_or(0, |note| match area {
            Some(area) if area.width > 0 => wrapped_height(&note.body, area.width),
            _ => u16::try_from(note.body.lines().count()).unwrap_or(u16::MAX),
        });
        // Stop when the last line reaches the top of the viewport, so a long
        // note does not scroll into emptiness.
        let visible = area.map_or(1, |a| a.height).max(1);
        let max = i32::from(height.saturating_sub(visible));
        let next = i32::from(self.body_scroll) + i32::from(delta);
        self.body_scroll = u16::try_from(next.clamp(0, max.max(0))).unwrap_or(0);
    }

    fn persist(&mut self) {
        self.store.save_reporting();
        if let Some(err) = self.store.last_error.clone() {
            self.status = Some((format!("save failed: {err}"), true));
        }
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), false));
    }

    /// Write the form back, returning an error message to show in the form.
    fn commit_form(&mut self) -> Result<(), String> {
        let Mode::Edit(form) = &self.mode else {
            return Ok(());
        };
        if form.title.is_blank() {
            return Err("a note needs a title".to_string());
        }

        let title = form.title.trimmed().to_string();
        let body = form.body.value();
        let id = form.id;

        let Some(id) = id else {
            let mut note = Note::new(0, title, self.today);
            note.body = body;
            let new_id = self.store.add(note);
            self.mode = Mode::List;
            self.refresh_view();
            // Land on the note just written rather than wherever the sort
            // happened to put the cursor.
            if let Some(index) = self.view.iter().position(|v| *v == new_id) {
                self.list_state.select(Some(index));
                self.body_scroll = 0;
            }
            self.set_status("added");
            self.persist();
            return Ok(());
        };

        self.store.with_note(id, self.today, |n| {
            n.title = title;
            n.body = body;
        });
        self.set_status("saved");
        self.mode = Mode::List;
        self.refresh_view();
        self.persist();
        Ok(())
    }

    fn handle_list_key(&mut self, key: KeyEvent) -> KeyOutcome {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.select_down(1),
            KeyCode::Char('k') | KeyCode::Up => self.select_up(1),
            KeyCode::Char('g') | KeyCode::Home => self.select_up(usize::MAX),
            KeyCode::Char('G') | KeyCode::End => self.select_down(usize::MAX),

            // The list is usually short and the body usually is not, so the
            // paging keys move the note rather than the selection — the
            // opposite of the task list, where the rows are the long thing.
            KeyCode::PageDown => self.scroll_body(5),
            KeyCode::PageUp => self.scroll_body(-5),

            KeyCode::Char('a' | 'n') => {
                self.mode = Mode::Edit(Box::new(EditForm::blank()));
            }

            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(note) = self.selected() {
                    self.mode = Mode::Edit(Box::new(EditForm::from_note(note)));
                }
            }

            KeyCode::Char('d') => {
                if let Some(note) = self.selected() {
                    self.mode = Mode::ConfirmDelete {
                        id: note.id,
                        title: note.title.clone(),
                    };
                }
            }

            KeyCode::Char('/') => {
                self.mode = Mode::Search(TextField::with_value(self.filter.clone()));
            }

            KeyCode::Char('o') => {
                let path = self.store.path().display().to_string();
                self.set_status(path);
            }

            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.set_status("search cleared");
                self.refresh_view();
            }

            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let Mode::Edit(form) = &mut self.mode else {
            return KeyOutcome::Ignored;
        };

        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::List;
                self.set_status("cancelled");
                return KeyOutcome::Consumed;
            }
            // Tab moves between fields; it cannot be Enter, because Enter has
            // to mean "new line" once the cursor is in the body. With only two
            // fields, forward and backward are the same move.
            KeyCode::Tab | KeyCode::BackTab => {
                form.field = match form.field {
                    Field::Title => Field::Body,
                    Field::Body => Field::Title,
                };
                return KeyOutcome::Consumed;
            }
            _ => {}
        }

        // Ctrl+S saves from either field. The footer advertises it while the
        // form is open, and it used to work only in the body — pressing it in
        // the title did nothing at all, so anyone who trusted the footer and
        // then pressed Esc lost the note. Enter also saves, but only from the
        // title, where it cannot be mistaken for a newline.
        let save = (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('s')))
            || (form.field == Field::Title && key.code == KeyCode::Enter);

        if save {
            if let Err(message) = self.commit_form()
                && let Mode::Edit(form) = &mut self.mode
            {
                form.error = Some(message);
            }
            return KeyOutcome::Consumed;
        }

        match form.field {
            Field::Title => {
                form.title.handle_key(key);
            }
            Field::Body => {
                form.body.handle_key(key);
            }
        }
        KeyOutcome::Consumed
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let Mode::Search(field) = &mut self.mode else {
            return KeyOutcome::Ignored;
        };
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::List;
            }
            KeyCode::Enter => {
                self.filter = field.trimmed().to_string();
                self.mode = Mode::List;
                self.refresh_view();
            }
            _ => {
                field.handle_key(key);
                // Filter as you type, so the list answers before you commit.
                self.filter = field.trimmed().to_string();
                self.refresh_view();
            }
        }
        KeyOutcome::Consumed
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let Mode::ConfirmDelete { id, .. } = &self.mode else {
            return KeyOutcome::Ignored;
        };
        let id = *id;
        // `y` alone; see the note on the same arm in `todo.rs`.
        if matches!(key.code, KeyCode::Char('y' | 'Y')) {
            self.store.remove(id);
            self.mode = Mode::List;
            self.refresh_view();
            self.set_status("deleted");
            self.persist();
        } else {
            self.mode = Mode::List;
            self.set_status("kept");
        }
        KeyOutcome::Consumed
    }

    /// One row of the list.
    fn note_line(&self, note: &Note, theme: &Theme, grid: &Grid) -> Line<'static> {
        let date = note
            .shown_date()
            .strftime(&self.config.date_format)
            .to_string();
        // An edited note shows when it changed; the mark says which date this
        // is, so the column is not two different facts sharing a heading.
        let date = if note.updated.is_some() {
            format!("·{date}")
        } else {
            date
        };

        grid.row(&[
            Span::styled(note.title.clone(), Style::default().fg(theme.text)),
            Span::styled(date, Style::default().fg(theme.muted)),
        ])
    }

    /// The detail pane: the selected note's title, date and body.
    fn render_detail(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let Some(note) = self.selected() else {
            let message = if self.view.is_empty() && self.filter.is_empty() {
                "No notes yet. Press `a` to write one."
            } else {
                "Nothing matches this search. Esc to clear."
            };
            frame.render_widget(
                Paragraph::new(Span::styled(message, Style::default().fg(theme.muted))),
                area,
            );
            return;
        };

        let rows = Layout::vertical([
            Constraint::Length(1), // title
            Constraint::Length(1), // dates
            Constraint::Min(0),    // body
        ])
        .split(area);

        // Wrapped by `grid` rather than by ratatui, whose own wrapper panics on
        // text mirador did not write — and a note is exactly that. See
        // `grid::wrapped`.
        frame.render_widget(
            Paragraph::new(crate::grid::wrapped(&note.title, rows[0].width)).style(
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            rows[0],
        );

        let mut dates = vec![Span::styled(
            format!("written {}", note.created.strftime("%d %b %Y")),
            Style::default().fg(theme.muted),
        )];
        if let Some(updated) = note.updated {
            dates.push(Span::styled(
                format!("   edited {}", updated.strftime("%d %b %Y")),
                Style::default().fg(theme.muted),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(dates)), rows[1]);

        if rows[2].height == 0 {
            return;
        }
        let body = if note.body.trim().is_empty() {
            Paragraph::new(Span::styled("(no body)", Style::default().fg(theme.muted)))
        } else {
            Paragraph::new(crate::grid::wrapped(&note.body, rows[2].width))
                .style(Style::default().fg(theme.text))
        };
        // The reader wraps even though the editor does not: prose written at
        // one width has to be readable at another.
        // Remembered so scrolling can clamp against the *wrapped* height. The
        // reader wraps and the clamp used to count `body.lines()`, so a note
        // written as one paragraph — the normal way — reported a single line
        // and would not scroll at all however long it was.
        self.body_area = Some(rows[2]);
        frame.render_widget(body.scroll((self.body_scroll, 0)), rows[2]);
    }

    /// The count line above the split, plus the active search if there is one.
    fn summary_line(&self, theme: &Theme) -> Line<'static> {
        let mut spans = vec![Span::styled(
            match self.store.notes().len() {
                0 => "no notes".to_string(),
                1 => "1 note".to_string(),
                n => format!("{n} notes"),
            },
            Style::default().fg(theme.muted),
        )];
        if !self.filter.is_empty() {
            spans.push(Span::styled(
                format!("   search: {}", self.filter),
                Style::default().fg(theme.label),
            ));
        }
        Line::from(spans)
    }

    /// The bottom line: a delete confirmation, the search prompt, or the last
    /// status message. A save failure outranks nothing — it stays until the
    /// next keypress, because a note that failed to save must not look saved.
    fn status_line(&self, theme: &Theme) -> Line<'static> {
        match (&self.mode, &self.status) {
            (Mode::ConfirmDelete { title, .. }, _) => Line::from(Span::styled(
                format!("delete \"{title}\"?  y / n"),
                Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            )),
            (Mode::Search(field), _) => Line::from(vec![
                Span::styled("search  ", Style::default().fg(theme.accent)),
                Span::styled(field.value().to_string(), Style::default().fg(theme.text)),
                Span::styled("▏", Style::default().fg(theme.accent)),
            ]),
            (_, Some((message, is_error))) => Line::from(Span::styled(
                message.clone(),
                Style::default().fg(if *is_error { theme.error } else { theme.muted }),
            )),
            _ => Line::default(),
        }
    }

    /// The edit form, drawn over the whole panel.
    fn render_form(frame: &mut Frame, area: Rect, theme: &Theme, form: &EditForm) {
        let rows = Layout::vertical([
            Constraint::Length(1), // heading
            Constraint::Length(1), // title field
            Constraint::Length(1), // body label
            Constraint::Min(1),    // body
            Constraint::Length(1), // hint or error
        ])
        .split(area);

        let heading = if form.id.is_some() {
            "EDIT NOTE"
        } else {
            "NEW NOTE"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                heading,
                Style::default()
                    .fg(theme.label)
                    .add_modifier(Modifier::BOLD),
            )),
            rows[0],
        );

        let active = |field: Field| {
            if form.field == field {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.muted)
            }
        };

        let (title_text, title_cursor) =
            form.title.visible(rows[1].width.saturating_sub(7) as usize);
        let mut title_spans = vec![Span::styled("title  ", active(Field::Title))];
        title_spans.push(Span::styled(title_text, Style::default().fg(theme.text)));
        if form.field == Field::Title {
            title_spans.push(Span::styled("▏", Style::default().fg(theme.accent)));
            let _ = title_cursor;
        }
        frame.render_widget(Paragraph::new(Line::from(title_spans)), rows[1]);

        frame.render_widget(
            Paragraph::new(Span::styled("body", active(Field::Body))),
            rows[2],
        );

        if rows[3].height > 0 {
            let height = usize::from(rows[3].height);
            let width = usize::from(rows[3].width);
            let offset = form.body.scroll_offset(height);
            let editing = form.field == Field::Body;
            let last = form.body.lines().len().min(offset + height);
            let lines: Vec<Line> = (offset..last)
                .map(|index| {
                    // The editor scrolls sideways as well as down, so a long
                    // line does not carry the caret off the right-hand edge.
                    // `visible` owns the arithmetic — it is measured in display
                    // cells, and getting that wrong here is what invariant 9 is
                    // about.
                    let (text, caret) = form.body.visible(index, width);
                    match caret.filter(|_| editing) {
                        // Draw the caret inline rather than moving the terminal
                        // cursor: the panel does not own the screen cursor, and
                        // a caret that only appears in the focused field is
                        // what tells the user where typing will land.
                        Some(at) => Line::from(vec![
                            Span::styled(text[..at].to_string(), Style::default().fg(theme.text)),
                            Span::styled("▏", Style::default().fg(theme.accent)),
                            Span::styled(text[at..].to_string(), Style::default().fg(theme.text)),
                        ]),
                        None => Line::from(Span::styled(text, Style::default().fg(theme.text))),
                    }
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), rows[3]);
        }

        let footer = match &form.error {
            Some(message) => Span::styled(message.clone(), Style::default().fg(theme.error)),
            None => Span::styled(
                "Tab field   Ctrl+S save   Esc cancel",
                Style::default().fg(theme.muted),
            ),
        };
        frame.render_widget(Paragraph::new(footer), rows[4]);
    }
}

impl Panel for NotesPanel {
    fn title(&self) -> String {
        "Notes".to_string()
    }

    fn counter(&self) -> Option<String> {
        // See the note on `TodoPanel::counter`: a failed save is a standing
        // condition and outranks the count.
        if self.store.last_error.is_some() {
            return Some("unsaved!".into());
        }
        let total = self.store.notes().len();
        if total == 0 {
            return None;
        }
        if self.filter.is_empty() {
            Some(format!("{total}"))
        } else {
            // While searching, the pair is the useful fact: how much of the
            // pile the search actually matched.
            Some(format!("{}/{total}", self.view.len()))
        }
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    fn refresh_interval(&self) -> std::time::Duration {
        // Nothing here changes on its own; the tick only rolls the date over
        // so a note written after midnight is stamped correctly.
        std::time::Duration::from_mins(1)
    }

    fn tick(&mut self) -> bool {
        // Only the date matters here, and only when it changes: `today` decides
        // whether a note is labelled "today" or by its date.
        let today = jiff::Zoned::now().date();
        let moved = today != self.today;
        self.today = today;
        moved
    }

    fn alert(&self) -> Option<crate::panel::Alert> {
        // A save that is failing is the clearest case there is: the work is on
        // screen and not on disk, and every further edit widens the gap.
        self.store
            .last_error
            .as_ref()
            .map(|why| crate::panel::Alert::failing(format!("Notes could not be saved — {why}")))
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::List)
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        self.status = None;
        match &self.mode {
            Mode::List => self.handle_list_key(key),
            Mode::Edit(_) => self.handle_edit_key(key),
            Mode::Search(_) => self.handle_search_key(key),
            Mode::ConfirmDelete { .. } => self.handle_confirm_key(key),
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> KeyOutcome {
        if !matches!(self.mode, Mode::List) {
            return KeyOutcome::Ignored;
        }
        let at = Position::new(event.column, event.row);
        let over_body = self.detail_area.is_some_and(|a| a.contains(at));

        match event.kind {
            // The wheel acts on whichever half it is pointing at: the list
            // scrolls through notes, the body scrolls through one note.
            MouseEventKind::ScrollDown if over_body => self.scroll_body(2),
            MouseEventKind::ScrollUp if over_body => self.scroll_body(-2),
            MouseEventKind::ScrollDown => self.select_down(1),
            MouseEventKind::ScrollUp => self.select_up(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(area) = self.list_area else {
                    return KeyOutcome::Ignored;
                };
                let Some(index) =
                    crate::selection::row_at(&self.list_state, area, at, self.view.len())
                else {
                    return KeyOutcome::Ignored;
                };
                self.status = None;
                self.select_to(index);
            }
            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Cleared each pass; the branches below set them only where something
        // was really drawn, so a stale rectangle cannot keep catching clicks.
        self.list_area = None;
        self.detail_area = None;

        if let Mode::Edit(form) = &self.mode {
            Self::render_form(frame, area, theme, form);
            return;
        }

        let rows = Layout::vertical([
            Constraint::Length(1), // summary
            Constraint::Min(1),    // master + detail
            Constraint::Length(1), // status
        ])
        .split(area);

        frame.render_widget(Paragraph::new(self.summary_line(theme)), rows[0]);

        // Master-detail split. Stacked by default: side by side divides a
        // finite width between a list that wants room for titles and a body
        // that wants room for prose, and neither gets enough. Stacking gives
        // both the full width and spends height instead.
        let body = rows[1];
        let side_by_side = self.config.preview.eq_ignore_ascii_case("beside");

        // A rule between the two halves. Without it the panel reads as one
        // list whose last few rows have gone strange, rather than as a list
        // and the note it is pointing at — the two are the same kind of text
        // in the same colours, so nothing else separates them.
        let (list_area, detail_area) = if side_by_side {
            let parts = Layout::horizontal([
                Constraint::Percentage(42),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(body);
            for row in 0..parts[1].height {
                frame.render_widget(
                    Paragraph::new(Span::styled("│", Style::default().fg(theme.rule))),
                    Rect::new(parts[1].x + 1, parts[1].y + row, 1, 1),
                );
            }
            (parts[0], parts[2])
        } else {
            let parts = Layout::vertical([
                Constraint::Percentage(45),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(body);
            crate::frame::rule(frame, parts[1], theme, "");
            (parts[0], parts[2])
        };

        if !self.view.is_empty() && list_area.height > 1 {
            let marker = 2u16;
            let grid = Grid::new(COLUMNS, list_area.width.saturating_sub(marker));
            let header_area = Rect::new(
                list_area.x + marker,
                list_area.y,
                list_area.width.saturating_sub(marker),
                1,
            );
            frame.render_widget(Paragraph::new(grid.header(theme)), header_area);

            // `NoteStore::get` is a linear scan; see the same fix in `todo.rs`.
            let by_id: std::collections::HashMap<u64, &Note> =
                self.store.notes().iter().map(|n| (n.id, n)).collect();
            let visible: Vec<&Note> = self
                .view
                .iter()
                .filter_map(|id| by_id.get(id).copied())
                .collect();
            let items: Vec<ListItem> = visible
                .iter()
                .map(|note| ListItem::new(self.note_line(note, theme, &grid)))
                .collect();

            let rows_area = Rect {
                y: list_area.y + 1,
                height: list_area.height - 1,
                ..list_area
            };
            self.list_area = Some(rows_area);

            let list = List::new(items)
                .highlight_symbol(if ctx.focused { "▸ " } else { "  " })
                .highlight_style(if ctx.focused {
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.muted)
                });
            frame.render_stateful_widget(list, rows_area, &mut self.list_state);
        }

        self.detail_area = Some(detail_area);
        self.render_detail(frame, detail_area, theme);

        frame.render_widget(Paragraph::new(self.status_line(theme)), rows[2]);
    }

    fn shutdown(&mut self) {
        self.store.save_reporting();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn panel(name: &str) -> (NotesPanel, TempDir) {
        let dir = std::env::temp_dir().join(format!("mirador-notes-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notes.toml");
        // An empty file rather than no file: the panel seeds an example note
        // when the file is absent, and these tests are about the panel, not
        // the seed. This is the branch every run after the first one takes.
        std::fs::write(&path, "").unwrap();
        let p = NotesPanel::new(NotesConfig::default(), path).unwrap();
        (p, TempDir(dir))
    }

    fn press(p: &mut NotesPanel, code: KeyCode) {
        p.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn type_str(p: &mut NotesPanel, text: &str) {
        for c in text.chars() {
            press(p, KeyCode::Char(c));
        }
    }

    /// Add a note through the form: `a`, title, Tab, body, Ctrl+S.
    fn add_note(p: &mut NotesPanel, title: &str, body: &str) {
        press(p, KeyCode::Char('a'));
        type_str(p, title);
        if body.is_empty() {
            press(p, KeyCode::Enter);
            return;
        }
        press(p, KeyCode::Tab);
        for c in body.chars() {
            if c == '\n' {
                press(p, KeyCode::Enter);
            } else {
                press(p, KeyCode::Char(c));
            }
        }
        p.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    }

    #[test]
    fn a_note_can_be_written_and_survives_a_reload() {
        let (mut p, guard) = panel("write");
        add_note(&mut p, "Shopping", "milk\neggs");

        assert!(matches!(p.mode, Mode::List), "the form must close on save");
        assert_eq!(p.view.len(), 1);

        let reloaded = NotesPanel::new(NotesConfig::default(), guard.0.join("notes.toml")).unwrap();
        assert_eq!(reloaded.store.notes().len(), 1);
        let note = &reloaded.store.notes()[0];
        assert_eq!(note.title, "Shopping");
        assert_eq!(note.body, "milk\neggs");
    }

    #[test]
    fn ctrl_s_saves_from_the_title_field_and_not_only_the_body() {
        // The footer advertises Ctrl+S the whole time the form is open. It used
        // to be handled only under Field::Body, so pressing it in the title did
        // nothing at all — and anyone who trusted the footer then pressed Esc
        // and lost the note.
        let (mut p, _g) = panel("ctrl-s-title");
        press(&mut p, KeyCode::Char('a'));
        type_str(&mut p, "Saved from the title");

        p.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        assert!(
            matches!(p.mode, Mode::List),
            "Ctrl+S in the title must close the form, not be swallowed"
        );
        assert_eq!(p.store.notes().len(), 1);
        assert_eq!(p.store.notes()[0].title, "Saved from the title");
    }

    #[test]
    fn a_single_paragraph_body_can_still_be_scrolled() {
        // The clamp counted `body.lines()` while the reader wraps, so a note
        // written as one long paragraph — the normal way — reported one line
        // and would not scroll however long it was.
        let (mut p, _g) = panel("scroll-wrapped");
        let long = "word ".repeat(400);
        add_note(&mut p, "Wrapped", &long);

        p.body_area = Some(Rect::new(0, 0, 40, 5));
        p.scroll_body(3);
        assert_eq!(p.body_scroll, 3, "a wrapped body must scroll");

        // And it must still stop rather than running off into blank space.
        p.scroll_body(i16::MAX);
        let height = wrapped_height(&long, 40);
        assert_eq!(p.body_scroll, height.saturating_sub(5));
    }

    #[test]
    fn wrapped_height_measures_cells_not_characters() {
        assert_eq!(wrapped_height("", 10), 1, "an empty body still has a row");
        assert_eq!(wrapped_height("a\nb\nc", 10), 3);
        // Ten two-cell glyphs need two rows at width 10, not one.
        assert_eq!(wrapped_height(&"日".repeat(10), 10), 2);
        assert_eq!(wrapped_height("x", 0), 0);
    }

    #[test]
    fn a_note_needs_a_title() {
        let (mut p, _g) = panel("blank");
        press(&mut p, KeyCode::Char('a'));
        press(&mut p, KeyCode::Enter);

        assert!(
            matches!(p.mode, Mode::Edit(_)),
            "an empty title must keep the form open"
        );
        let Mode::Edit(form) = &p.mode else {
            unreachable!()
        };
        assert!(form.error.is_some(), "and say why");
        assert!(p.store.notes().is_empty());
    }

    #[test]
    fn enter_inside_the_body_makes_a_new_line_rather_than_saving() {
        let (mut p, _g) = panel("newline");
        press(&mut p, KeyCode::Char('a'));
        type_str(&mut p, "Title");
        press(&mut p, KeyCode::Tab);
        type_str(&mut p, "one");
        press(&mut p, KeyCode::Enter);
        type_str(&mut p, "two");

        assert!(
            matches!(p.mode, Mode::Edit(_)),
            "Enter in the body must not close the form"
        );
        let Mode::Edit(form) = &p.mode else {
            unreachable!()
        };
        assert_eq!(form.body.value(), "one\ntwo");
    }

    #[test]
    fn editing_an_existing_note_updates_it_in_place() {
        let (mut p, _g) = panel("edit");
        add_note(&mut p, "Original", "body");
        assert_eq!(p.store.notes().len(), 1);

        press(&mut p, KeyCode::Enter);
        assert!(matches!(p.mode, Mode::Edit(_)), "Enter opens the note");
        type_str(&mut p, " revised");
        press(&mut p, KeyCode::Enter);

        assert_eq!(p.store.notes().len(), 1, "editing must not duplicate");
        assert_eq!(p.store.notes()[0].title, "Original revised");
    }

    #[test]
    fn deleting_asks_first_and_keeps_the_note_on_any_other_key() {
        let (mut p, _g) = panel("delete");
        add_note(&mut p, "Keep me", "");

        press(&mut p, KeyCode::Char('d'));
        assert!(matches!(p.mode, Mode::ConfirmDelete { .. }));
        press(&mut p, KeyCode::Char('n'));
        assert_eq!(p.store.notes().len(), 1, "n must keep it");

        press(&mut p, KeyCode::Char('d'));
        press(&mut p, KeyCode::Char('y'));
        assert!(p.store.notes().is_empty(), "y deletes");
    }

    #[test]
    fn searching_filters_as_you_type_and_esc_restores_everything() {
        let (mut p, _g) = panel("search");
        add_note(&mut p, "Groceries", "milk");
        add_note(&mut p, "Meeting", "invoice question");
        assert_eq!(p.view.len(), 2);

        press(&mut p, KeyCode::Char('/'));
        type_str(&mut p, "invoice");
        assert_eq!(p.view.len(), 1, "the body is searched, not just the title");
        press(&mut p, KeyCode::Enter);

        assert!(matches!(p.mode, Mode::List));
        press(&mut p, KeyCode::Esc);
        assert_eq!(p.view.len(), 2, "Esc clears the search");
    }

    #[test]
    fn the_form_captures_input_so_typing_q_cannot_quit() {
        let (mut p, _g) = panel("capture");
        assert!(!p.captures_input(), "the list must not swallow global keys");
        press(&mut p, KeyCode::Char('a'));
        assert!(p.captures_input(), "an open form must swallow them");
    }

    #[test]
    fn moving_the_selection_resets_the_body_scroll() {
        let (mut p, _g) = panel("scroll-reset");
        add_note(&mut p, "Long", &"line\n".repeat(40));
        add_note(&mut p, "Short", "short");

        // Newest first, so the cursor starts on "Short"; step down to the long
        // one, which is the only one with anywhere to scroll.
        press(&mut p, KeyCode::Char('j'));
        assert_eq!(p.selected().unwrap().title, "Long");

        p.scroll_body(10);
        assert!(p.body_scroll > 0, "the long note scrolls");
        press(&mut p, KeyCode::Char('k'));
        assert_eq!(
            p.body_scroll, 0,
            "a different note must start at its own top"
        );
    }

    #[test]
    fn the_body_cannot_be_scrolled_past_its_own_end() {
        let (mut p, _g) = panel("scroll-clamp");
        add_note(&mut p, "Short", "one\ntwo\nthree");

        p.scroll_body(500);
        assert_eq!(p.body_scroll, 2, "clamped to the last line");
        p.scroll_body(-500);
        assert_eq!(p.body_scroll, 0, "and cannot go negative");
    }

    #[test]
    fn the_counter_shows_the_match_count_only_while_searching() {
        let (mut p, _g) = panel("counter");
        assert_eq!(p.counter(), None, "no counter with nothing to count");
        add_note(&mut p, "Groceries", "milk");
        add_note(&mut p, "Meeting", "invoice");
        assert_eq!(p.counter(), Some("2".to_string()));

        press(&mut p, KeyCode::Char('/'));
        type_str(&mut p, "milk");
        assert_eq!(p.counter(), Some("1/2".to_string()));
    }

    /// Every key list mode responds to, paired with the binding documenting it.
    const DOCUMENTED_LIST_KEYS: &[(KeyCode, &str)] = &[
        (KeyCode::Char('a'), "a"),
        (KeyCode::Char('n'), "n"),
        (KeyCode::Enter, "↵"),
        (KeyCode::Char('e'), "e"),
        (KeyCode::Char('d'), "d"),
        (KeyCode::Down, "↑ / ↓"),
        (KeyCode::Up, "↑ / ↓"),
        (KeyCode::Char('j'), "j / k"),
        (KeyCode::Char('k'), "j / k"),
        (KeyCode::Char('g'), "g / G"),
        (KeyCode::Char('G'), "g / G"),
        (KeyCode::Home, "Home / End"),
        (KeyCode::End, "Home / End"),
        (KeyCode::PageUp, "PgUp / PgDn"),
        (KeyCode::PageDown, "PgUp / PgDn"),
        (KeyCode::Char('/'), "/"),
        (KeyCode::Char('o'), "o"),
    ];

    #[test]
    fn every_documented_key_works_and_every_working_key_is_documented() {
        for (code, key) in DOCUMENTED_LIST_KEYS {
            assert!(
                BINDINGS.iter().any(|b| b.key == *key),
                "`{key}` is handled but missing from BINDINGS, so nothing tells the user it exists"
            );

            let (mut p, _g) = panel("keymap");
            add_note(&mut p, "a note", "body");
            assert!(matches!(p.mode, Mode::List));

            let outcome = p.handle_key(KeyEvent::new(*code, KeyModifiers::NONE));
            assert_eq!(
                outcome,
                KeyOutcome::Consumed,
                "`{key}` is documented but the list ignores it"
            );
        }
    }

    /// A note is prose somebody wrote, and prose contains emoji. Handing that
    /// to ratatui's word wrapper crashes the dashboard — see `grid::wrapped`,
    /// which is why this panel wraps its own title and body and renders a
    /// `Paragraph` with no `Wrap`.
    ///
    /// This is the guard against putting `.wrap(…)` back. The grid has its own
    /// test for the wrapping; this one exists because the mistake is made here.
    #[test]
    fn a_note_full_of_wide_glyphs_draws_at_every_width() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();

        let (mut p, _g) = panel("wide-glyphs");
        add_note(
            &mut p,
            "a\u{1F31E}b \u{4E2D}\u{6587}\u{6807}\u{9898}",
            "\u{0301} \u{1F31E}\u{65E5}\u{65E5}\u{65E5}\u{1F31E}\u{1F31E}a\n\
             prose with a \u{1F31E} in it and a \u{65E5}\u{672C}\u{8A9E} word",
        );

        // The height matters as much as the width, and that is not obvious: the
        // fault is on a later row, so a one-row pane never reaches it. This
        // test passed against the very bug it exists for until the panes were
        // given room to wrap into.
        for width in 1..24u16 {
            for height in 1..40u16 {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| {
                        p.render(
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
            }
        }
    }

    #[test]
    fn the_preview_sits_below_the_list_by_default_and_beside_it_on_request() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let config = crate::config::Config::default();
        let gradients = config.theme.gradients();

        let draw = |p: &mut NotesPanel| {
            let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
            terminal
                .draw(|frame| {
                    p.render(
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
            (p.list_area.unwrap(), p.detail_area.unwrap())
        };

        // Default: stacked, even at a width that would fit both side by side.
        // Splitting the width starves the titles and the prose at once.
        let (mut p, _g) = panel("split-below");
        add_note(&mut p, "Note", "body text");
        let (list, detail) = draw(&mut p);
        assert_eq!(detail.x, list.x, "stacked: {list:?} {detail:?}");
        assert!(detail.y > list.y, "and the body is below");
        assert_eq!(detail.width, list.width, "both get the full width");

        // Opt in to beside.
        p.config.preview = "beside".to_string();
        let (list, detail) = draw(&mut p);
        assert!(detail.x > list.x, "beside: {list:?} {detail:?}");
    }
}
