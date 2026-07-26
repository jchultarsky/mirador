//! The to-do panel: a full create/read/update/delete task list.
//!
//! Every mutation is written straight through to disk, so the task file on disk
//! and the list on screen never disagree. Save failures surface in the panel's
//! status line rather than being swallowed.

use jiff::civil::Date;
use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};

use crate::config::TodoConfig;
use crate::dateinput::parse_due;
use crate::frame::Binding;
use crate::grid::{Column, Grid};
use crate::panel::{KeyOutcome, Panel, RenderContext};
use crate::task::{DueState, Priority, SortMode, Task, TaskStore};
use crate::textfield::TextField;
use crate::theme::Theme;

/// Keys this panel responds to.
///
/// The first few `primary` entries are what fits in the frame hint, so they
/// are ordered by how often they are actually used, not alphabetically.
/// Every key the list responds to.
///
/// This list is the help overlay, the status bar and the border hint all at
/// once, so a key missing here is a key nobody can discover.
/// `every_documented_key_works_and_every_working_key_is_documented` holds the
/// two halves together.
const BINDINGS: &[Binding] = &[
    Binding::primary("a", "add"),
    Binding::primary("↵", "edit"),
    Binding::primary("space", "done"),
    Binding::primary("d", "delete"),
    Binding::extra("↑ / ↓", "move selection"),
    Binding::extra("j / k", "move selection"),
    Binding::extra("g / G", "first / last"),
    Binding::extra("Home / End", "first / last"),
    Binding::extra("PgUp / PgDn", "move ten rows"),
    Binding::extra("e", "edit"),
    Binding::extra("n", "add"),
    Binding::extra("p / P", "cycle priority"),
    Binding::extra("s", "cycle sort"),
    Binding::extra("c", "show completed"),
    Binding::extra("/", "filter"),
    Binding::extra("Esc", "clear filter"),
    Binding::extra("o", "show file path"),
];

/// Columns of the task list.
///
/// `done` and `pri` are glyph columns, but they still carry names: without a
/// header the three-cell gauge is a guess.
const COLUMNS: &[Column] = &[
    Column::fixed("done", 4),
    Column::fixed("pri", 3),
    Column::flex("task", 1),
    Column::fixed("tags", 16).drops_below(58),
    Column::fixed("due", 12).right().drops_below(40),
];

/// Fields in the edit form, in Tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Notes,
    Due,
    Priority,
    Tags,
}

impl Field {
    const ORDER: [Self; 5] = [
        Self::Title,
        Self::Notes,
        Self::Due,
        Self::Priority,
        Self::Tags,
    ];

    fn next(self) -> Self {
        let i = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(i + 1) % Self::ORDER.len()]
    }

    fn prev(self) -> Self {
        let i = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(i + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Notes => "Notes",
            Self::Due => "Due",
            Self::Priority => "Priority",
            Self::Tags => "Tags",
        }
    }
}

/// State of the add/edit form.
#[derive(Debug)]
struct EditForm {
    /// `None` when creating a new task.
    id: Option<u64>,
    field: Field,
    title: TextField,
    notes: TextField,
    due: TextField,
    tags: TextField,
    priority: Priority,
    /// Validation failure from the last save attempt.
    error: Option<String>,
}

impl EditForm {
    /// A blank form for a new task.
    fn blank() -> Self {
        Self {
            id: None,
            field: Field::Title,
            title: TextField::new(),
            notes: TextField::new(),
            due: TextField::new(),
            tags: TextField::new(),
            priority: Priority::default(),
            error: None,
        }
    }

    /// A form pre-filled from an existing task.
    fn from_task(task: &Task) -> Self {
        Self {
            id: Some(task.id),
            field: Field::Title,
            title: TextField::with_value(&task.title),
            notes: TextField::with_value(task.notes.clone().unwrap_or_default()),
            due: TextField::with_value(task.due.map(|d| d.to_string()).unwrap_or_default()),
            tags: TextField::with_value(task.tags.join(", ")),
            priority: task.priority,
            error: None,
        }
    }

    fn active_text_field(&mut self) -> Option<&mut TextField> {
        match self.field {
            Field::Title => Some(&mut self.title),
            Field::Notes => Some(&mut self.notes),
            Field::Due => Some(&mut self.due),
            Field::Tags => Some(&mut self.tags),
            Field::Priority => None,
        }
    }

    fn text_field(&self, field: Field) -> Option<&TextField> {
        match field {
            Field::Title => Some(&self.title),
            Field::Notes => Some(&self.notes),
            Field::Due => Some(&self.due),
            Field::Tags => Some(&self.tags),
            Field::Priority => None,
        }
    }
}

/// What the panel is currently doing.
#[derive(Debug)]
enum Mode {
    /// Browsing the list.
    List,
    /// Typing into the filter box.
    Filter(TextField),
    /// Adding or editing a task.
    Edit(Box<EditForm>),
    /// Confirming a deletion.
    ConfirmDelete { id: u64, title: String },
}

/// The to-do panel.
#[derive(Debug)]
pub struct TodoPanel {
    store: TaskStore,
    config: TodoConfig,
    sort: SortMode,
    show_completed: bool,
    filter: String,
    mode: Mode,
    /// Task ids in display order, recomputed whenever the list changes.
    view: Vec<u64>,
    list_state: ListState,
    /// Transient message with a severity flag.
    status: Option<(String, bool)>,
    today: Date,
    /// Rectangle the task rows were last drawn into, so a click can be turned
    /// back into the row it landed on. `None` until the first draw, and while
    /// the list is empty — there is no row to hit in either case.
    list_area: Option<Rect>,
}

impl TodoPanel {
    /// Build the panel, loading tasks from `path`.
    pub fn new(config: TodoConfig, path: std::path::PathBuf) -> anyhow::Result<Self> {
        let today = jiff::Zoned::now().date();
        let store = TaskStore::load_or_seed(path, today)?;
        let sort = config.sort.parse().unwrap_or_default();
        let show_completed = config.show_completed;

        let mut panel = Self {
            store,
            config,
            sort,
            show_completed,
            filter: String::new(),
            mode: Mode::List,
            view: Vec::new(),
            list_state: ListState::default(),
            status: None,
            today,
            list_area: None,
        };
        panel.refresh_view();
        Ok(panel)
    }

    /// Recompute the ordered id list, keeping the selection on the same task
    /// where possible so that toggling a filter does not move the cursor.
    fn refresh_view(&mut self) {
        let previously_selected = self.selected_id();
        self.view = self
            .store
            .view(self.sort, self.show_completed, &self.filter, self.today);

        let index = previously_selected
            .and_then(|id| self.view.iter().position(|v| *v == id))
            .unwrap_or_else(|| {
                self.list_state
                    .selected()
                    .unwrap_or(0)
                    .min(self.view.len().saturating_sub(1))
            });

        if self.view.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(index.min(self.view.len() - 1)));
        }
    }

    /// The id of the highlighted task, if any.
    fn selected_id(&self) -> Option<u64> {
        self.list_state
            .selected()
            .and_then(|i| self.view.get(i))
            .copied()
    }

    /// Move the selection `n` rows down, stopping at the last task.
    fn select_down(&mut self, n: usize) {
        let Some(last) = self.view.len().checked_sub(1) else {
            return;
        };
        let current = self.list_state.selected().unwrap_or(0);
        self.list_state
            .select(Some(current.saturating_add(n).min(last)));
    }

    /// Move the selection `n` rows up, stopping at the first task.
    fn select_up(&mut self, n: usize) {
        if self.view.is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some(current.saturating_sub(n)));
    }

    /// Persist and report the outcome in the status line.
    fn persist(&mut self) {
        self.store.save_reporting();
        if let Some(err) = self.store.last_error.clone() {
            self.status = Some((format!("save failed: {err}"), true));
        }
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), false));
    }

    /// Turn form contents into a task, or explain why it cannot be.
    fn commit_form(&mut self) -> Result<(), String> {
        let Mode::Edit(form) = &mut self.mode else {
            return Ok(());
        };

        if form.title.is_blank() {
            return Err("a task needs a title".into());
        }

        let due = parse_due(form.due.value(), self.today).map_err(|e| e.to_string())?;

        let notes = {
            let n = form.notes.trimmed();
            if n.is_empty() {
                None
            } else {
                Some(n.to_string())
            }
        };

        let tags: Vec<String> = form
            .tags
            .value()
            .split([',', ' '])
            .map(|t| t.trim().trim_start_matches('#'))
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();

        let title = form.title.trimmed().to_string();
        let priority = form.priority;
        let id = form.id;

        if let Some(id) = id {
            let Some(existing) = self.store.get(id) else {
                return Err("that task no longer exists".into());
            };
            let mut updated = existing.clone();
            updated.title = title;
            updated.notes = notes;
            updated.due = due;
            updated.priority = priority;
            updated.tags = tags;
            self.store.update(updated);
            self.set_status("task updated");
        } else {
            let mut task = Task::new(0, title, self.today);
            task.notes = notes;
            task.due = due;
            task.priority = priority;
            task.tags = tags;
            let new_id = self.store.add(task);
            self.set_status("task added");
            self.mode = Mode::List;
            self.refresh_view();
            if let Some(pos) = self.view.iter().position(|v| *v == new_id) {
                self.list_state.select(Some(pos));
            }
            self.persist();
            return Ok(());
        }

        self.mode = Mode::List;
        self.refresh_view();
        self.persist();
        Ok(())
    }

    /// Keys handled while browsing the list.
    fn handle_list_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.select_down(1),
            KeyCode::Char('k') | KeyCode::Up => self.select_up(1),
            KeyCode::Char('g') | KeyCode::Home => self.select_up(usize::MAX),
            KeyCode::Char('G') | KeyCode::End => self.select_down(usize::MAX),
            KeyCode::PageDown => self.select_down(10),
            KeyCode::PageUp => self.select_up(10),

            KeyCode::Char(' ') => {
                let Some(id) = self.selected_id() else {
                    return KeyOutcome::Consumed;
                };
                let today = self.today;
                self.store.with_task(id, |t| t.toggle_done(today));
                let now_done = self.store.get(id).is_some_and(|t| t.done);
                self.set_status(if now_done { "completed" } else { "reopened" });
                self.refresh_view();
                self.persist();
            }

            KeyCode::Char('a' | 'n') => {
                self.mode = Mode::Edit(Box::new(EditForm::blank()));
            }

            // Enter means "go inside", as it does everywhere else: it opens the
            // task for editing. It used to toggle done, which made the most
            // reflexive key in the list a destructive-looking state change on
            // the wrong row. Toggling is `space`, which reads as a checkbox.
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(task) = self.selected_id().and_then(|id| self.store.get(id)) {
                    self.mode = Mode::Edit(Box::new(EditForm::from_task(task)));
                }
            }

            KeyCode::Char('d') => {
                if let Some(task) = self.selected_id().and_then(|id| self.store.get(id)) {
                    self.mode = Mode::ConfirmDelete {
                        id: task.id,
                        title: task.title.clone(),
                    };
                }
            }

            // Shift+P walks priority back up; p cycles down.
            KeyCode::Char('p' | 'P') => {
                let Some(id) = self.selected_id() else {
                    return KeyOutcome::Consumed;
                };
                self.store.with_task(id, |t| {
                    t.priority = if shift {
                        t.priority.prev()
                    } else {
                        t.priority.next()
                    };
                });
                let label = self
                    .store
                    .get(id)
                    .map_or(String::new(), |t| t.priority.to_string());
                self.set_status(format!("priority: {label}"));
                self.refresh_view();
                self.persist();
            }

            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.set_status(format!("sort: {}", self.sort.label()));
                self.refresh_view();
            }

            KeyCode::Char('c') => {
                self.show_completed = !self.show_completed;
                self.set_status(if self.show_completed {
                    "showing completed"
                } else {
                    "hiding completed"
                });
                self.refresh_view();
            }

            KeyCode::Char('/') => {
                self.mode = Mode::Filter(TextField::with_value(self.filter.clone()));
            }

            KeyCode::Char('o') => {
                let path = self.store.path().display().to_string();
                self.set_status(path);
            }

            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.set_status("filter cleared");
                self.refresh_view();
            }

            _ => return KeyOutcome::Ignored,
        }
        KeyOutcome::Consumed
    }

    /// Keys handled while the edit form is open.
    fn handle_edit_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let Mode::Edit(form) = &mut self.mode else {
            return KeyOutcome::Ignored;
        };

        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::List;
                self.set_status("cancelled");
                return KeyOutcome::Consumed;
            }
            KeyCode::Enter => {
                if let Err(message) = self.commit_form()
                    && let Mode::Edit(form) = &mut self.mode
                {
                    form.error = Some(message);
                }
                return KeyOutcome::Consumed;
            }
            KeyCode::Tab | KeyCode::Down => form.field = form.field.next(),
            KeyCode::BackTab | KeyCode::Up => form.field = form.field.prev(),

            KeyCode::Left | KeyCode::Right if form.field == Field::Priority => {
                form.priority = if key.code == KeyCode::Right {
                    form.priority.next()
                } else {
                    form.priority.prev()
                };
            }
            KeyCode::Char(' ') if form.field == Field::Priority => {
                form.priority = if shift {
                    form.priority.prev()
                } else {
                    form.priority.next()
                };
            }

            _ => {
                let used = form
                    .active_text_field()
                    .is_some_and(|field| field.handle_key(key));
                if used {
                    form.error = None;
                }
                return KeyOutcome::Consumed;
            }
        }
        KeyOutcome::Consumed
    }

    /// Keys handled while typing a filter.
    fn handle_filter_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let Mode::Filter(field) = &mut self.mode else {
            return KeyOutcome::Ignored;
        };
        match key.code {
            // Esc abandons filtering entirely rather than leaving a
            // half-typed filter applied behind a closed input.
            KeyCode::Esc => {
                field.clear();
                self.filter.clear();
                self.mode = Mode::List;
                self.refresh_view();
            }
            KeyCode::Enter => {
                self.filter = field.trimmed().to_string();
                self.mode = Mode::List;
                self.refresh_view();
            }
            _ => {
                if field.handle_key(key) {
                    let live = field.trimmed().to_string();
                    self.filter = live;
                    self.refresh_view();
                }
            }
        }
        KeyOutcome::Consumed
    }

    /// Keys handled at the delete confirmation.
    fn handle_confirm_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let Mode::ConfirmDelete { id, .. } = self.mode else {
            return KeyOutcome::Ignored;
        };
        match key.code {
            // `y` alone. Enter used to delete here while the prompt said "any
            // other key cancel" — the most reflexive key at a confirmation,
            // promised safe, wired to the one action with no undo.
            KeyCode::Char('y' | 'Y') => {
                self.store.remove(id);
                self.mode = Mode::List;
                self.set_status("task deleted");
                self.refresh_view();
                self.persist();
            }
            _ => {
                self.mode = Mode::List;
            }
        }
        KeyOutcome::Consumed
    }

    /// Build one task row against the shared column grid.
    ///
    /// Ragged rows are why a task list reads as noise: if the due date floats
    /// to wherever the title happens to end, the eye cannot scan down it. The
    /// grid guarantees every row agrees on where each column starts.
    fn task_line<'a>(&self, task: &'a Task, theme: &Theme, grid: &Grid) -> Line<'a> {
        let (filled, priority_colour) = match task.priority {
            Priority::High => (3, theme.error),
            Priority::Medium => (2, theme.warning),
            Priority::Low => (1, theme.muted),
            Priority::None => (0, theme.muted),
        };

        let (due_label, due_colour) = self
            .due_badge(task, theme)
            .unwrap_or_else(|| (String::new(), theme.muted));

        let tags = task
            .tags
            .iter()
            .map(|t| format!("#{t}"))
            .collect::<Vec<_>>()
            .join(" ");

        let title_style = if task.done {
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::CROSSED_OUT)
        } else {
            Style::default().fg(theme.text)
        };

        grid.row(&[
            Span::styled(
                if task.done { "[x]" } else { "[ ]" },
                Style::default().fg(if task.done {
                    theme.success
                } else {
                    theme.muted
                }),
            ),
            // A filled gauge reads as a level without needing colour, which
            // matters when a terminal theme flattens the palette.
            Span::styled(
                "\u{25ae}".repeat(filled),
                Style::default().fg(priority_colour),
            ),
            Span::styled(task.title.as_str(), title_style),
            Span::styled(tags, Style::default().fg(theme.label)),
            Span::styled(due_label, Style::default().fg(due_colour)),
        ])
    }

    /// The due-date badge text and colour, if the task has a due date.
    fn due_badge(&self, task: &Task, theme: &Theme) -> Option<(String, ratatui::style::Color)> {
        let due = task.due?;
        if task.done {
            return Some((
                due.strftime(&self.config.date_format).to_string(),
                theme.muted,
            ));
        }
        let badge = match task.due_state(self.today) {
            DueState::Overdue(1) => ("1 day late".to_string(), theme.error),
            DueState::Overdue(n) => (format!("{n} days late"), theme.error),
            DueState::Today => ("today".to_string(), theme.warning),
            DueState::Soon(1) => ("tomorrow".to_string(), theme.warning),
            DueState::Soon(n) => (format!("in {n} days"), theme.text),
            DueState::Later(_) => (
                due.strftime(&self.config.date_format).to_string(),
                theme.muted,
            ),
            DueState::None => return None,
        };
        Some(badge)
    }

    /// The summary line: how much is open, and how it is sorted.
    fn header(&self, theme: &Theme) -> Line<'static> {
        let open = self.store.tasks().iter().filter(|t| !t.done).count();
        let overdue = self
            .store
            .tasks()
            .iter()
            .filter(|t| !t.done && matches!(t.due_state(self.today), DueState::Overdue(_)))
            .count();
        let today = self
            .store
            .tasks()
            .iter()
            .filter(|t| !t.done && matches!(t.due_state(self.today), DueState::Today))
            .count();

        let mut spans = Vec::new();

        // Lead with whatever is actually wrong. On a calm day this line is
        // short and grey, which is the point: you should be able to tell at a
        // glance that there is nothing to deal with.
        if overdue > 0 {
            spans.push(Span::styled(
                format!("{overdue} overdue"),
                Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled("   ", Style::default().fg(theme.muted)));
        }
        if today > 0 {
            spans.push(Span::styled(
                format!("{today} due today"),
                Style::default().fg(theme.warning),
            ));
            spans.push(Span::styled("   ", Style::default().fg(theme.muted)));
        }
        spans.push(Span::styled(
            format!("{open} open"),
            Style::default().fg(theme.muted),
        ));
        spans.push(Span::styled(
            format!("   by {}", self.sort.label()),
            Style::default().fg(theme.muted),
        ));
        if self.show_completed {
            spans.push(Span::styled("   +done", Style::default().fg(theme.muted)));
        }
        if !self.filter.is_empty() {
            spans.push(Span::styled(
                format!("   /{}", self.filter),
                Style::default().fg(theme.accent),
            ));
        }
        Line::from(spans)
    }

    /// Draw the add/edit form as a centred modal.
    #[allow(clippy::too_many_lines)] // A form with five fields; splitting it
    // further would scatter the layout across helpers without making it clearer.
    fn render_form(frame: &mut Frame, area: Rect, theme: &Theme, form: &EditForm) {
        let width = area.width.clamp(24, 66);
        let height = 13u16.min(area.height);
        let popup = centred(area, width, height);

        frame.render_widget(Clear, popup);
        let title = if form.id.is_some() {
            " Edit task "
        } else {
            " New task "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title(Span::styled(title, Style::default().fg(theme.title).bold()));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let rows = Layout::vertical([
            Constraint::Length(1), // title
            Constraint::Length(1), // notes
            Constraint::Length(1), // due
            Constraint::Length(1), // priority
            Constraint::Length(1), // tags
            Constraint::Length(1), // spacer
            Constraint::Length(1), // hint / error
            Constraint::Min(0),
            Constraint::Length(1), // key help
        ])
        .split(inner);

        let label_width = 9u16;
        for (i, field) in Field::ORDER.iter().enumerate() {
            let row = rows[i];
            let active = form.field == *field;
            let label_style = if active {
                Style::default().fg(theme.accent).bold()
            } else {
                Style::default().fg(theme.muted)
            };

            let columns = Layout::horizontal([Constraint::Length(label_width), Constraint::Min(1)])
                .split(row);

            frame.render_widget(
                Paragraph::new(Span::styled(field.label(), label_style)),
                columns[0],
            );

            let value_area = columns[1];
            if *field == Field::Priority {
                let marker = if active { "‹ " } else { "  " };
                let marker_end = if active { " ›" } else { "" };
                let colour = match form.priority {
                    Priority::High => theme.error,
                    Priority::Medium => theme.warning,
                    _ => theme.text,
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(marker, Style::default().fg(theme.muted)),
                        Span::styled(form.priority.to_string(), Style::default().fg(colour)),
                        Span::styled(marker_end, Style::default().fg(theme.muted)),
                    ])),
                    value_area,
                );
            } else if let Some(text) = form.text_field(*field) {
                let (visible, cursor_col) = text.visible(value_area.width as usize);
                let style = if active {
                    Style::default().fg(theme.text)
                } else {
                    Style::default().fg(theme.muted)
                };
                let display = if visible.is_empty() && !active {
                    placeholder(*field).to_string()
                } else {
                    visible
                };
                let display_style = if text.value().is_empty() && !active {
                    Style::default().fg(theme.muted).italic()
                } else {
                    style
                };
                frame.render_widget(
                    Paragraph::new(Span::styled(display, display_style)),
                    value_area,
                );
                if active {
                    frame.set_cursor_position((value_area.x + cursor_col as u16, value_area.y));
                }
            }
        }

        // Hint or validation error.
        let message = match &form.error {
            Some(err) => Span::styled(err.clone(), Style::default().fg(theme.error)),
            None => Span::styled(
                hint_for(form.field).to_string(),
                Style::default().fg(theme.muted),
            ),
        };
        frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: true }), rows[6]);

        frame.render_widget(
            Paragraph::new(Span::styled(
                "Tab/↑↓ field · Enter save · Esc cancel",
                Style::default().fg(theme.muted),
            )),
            rows[8],
        );
    }

    /// Draw the delete confirmation.
    fn render_confirm(frame: &mut Frame, area: Rect, theme: &Theme, title: &str) {
        let width = area.width.clamp(20, 54);
        let popup = centred(area, width, 5.min(area.height));
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.error))
            .title(Span::styled(
                " Delete task ",
                Style::default().fg(theme.error).bold(),
            ));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let text = vec![
            Line::from(Span::styled(
                truncate(title, inner.width as usize),
                Style::default().fg(theme.text),
            )),
            Line::from(Span::styled(
                "y delete · any other key cancel",
                Style::default().fg(theme.muted),
            )),
        ];
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), inner);
    }
}

impl Panel for TodoPanel {
    fn title(&self) -> String {
        "Tasks".to_string()
    }

    fn counter(&self) -> Option<String> {
        // A failed save outranks the count. The status line is transient — it
        // clears on the next keypress — which is the right lifetime for "added
        // a task" and the wrong one for "nothing you do is reaching the disk".
        // Left to the status line alone, the panel went on reporting a tally it
        // could not persist, and said nothing at all on the way out.
        if self.store.last_error.is_some() {
            return Some("unsaved!".into());
        }
        let open = self.store.tasks().iter().filter(|t| !t.done).count();
        Some(format!("{open} open"))
    }

    fn refresh_interval(&self) -> std::time::Duration {
        // Only needed so that due-date colouring rolls over at midnight.
        std::time::Duration::from_mins(1)
    }

    fn tick(&mut self) {
        let today = jiff::Zoned::now().date();
        if today != self.today {
            self.today = today;
            self.refresh_view();
        }
    }

    fn remember(&self, state: &mut crate::state::UiState) {
        state.todo_sort = Some(self.sort.label().to_string());
        state.todo_show_completed = Some(self.show_completed);
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::List)
    }

    fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        // Any keypress clears a stale status message.
        self.status = None;
        match &self.mode {
            Mode::List => self.handle_list_key(key),
            Mode::Edit(_) => self.handle_edit_key(key),
            Mode::Filter(_) => self.handle_filter_key(key),
            Mode::ConfirmDelete { .. } => self.handle_confirm_key(key),
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, _area: Rect) -> KeyOutcome {
        // A form owns the panel while it is open; a click landing behind it
        // must not quietly move the selection out from under the editor.
        if !matches!(self.mode, Mode::List) {
            return KeyOutcome::Ignored;
        }

        match event.kind {
            MouseEventKind::ScrollDown => {
                self.select_down(1);
                KeyOutcome::Consumed
            }
            MouseEventKind::ScrollUp => {
                self.select_up(1);
                KeyOutcome::Consumed
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(area) = self.list_area else {
                    return KeyOutcome::Ignored;
                };
                if !area.contains(Position::new(event.column, event.row)) {
                    return KeyOutcome::Ignored;
                }
                // `offset` is the first task on screen, so the row under the
                // pointer counts from there rather than from the top of the
                // list — otherwise clicking a scrolled list selects the wrong
                // task by exactly the scroll distance.
                let row = usize::from(event.row - area.y);
                let index = self.list_state.offset() + row;
                if index >= self.view.len() {
                    // Clicking the empty space below the last task is not a
                    // selection of the last task.
                    return KeyOutcome::Ignored;
                }
                self.status = None;
                self.list_state.select(Some(index));
                KeyOutcome::Consumed
            }
            _ => KeyOutcome::Ignored,
        }
    }

    fn bindings(&self) -> &'static [Binding] {
        BINDINGS
    }

    #[allow(clippy::too_many_lines)] // Four stacked regions plus two modal
    // overlays; the sub-renderers are already factored out.
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: RenderContext<'_>) {
        let theme = ctx.theme;
        if area.height == 0 || area.width == 0 {
            return;
        }

        let show_notes = self
            .selected_id()
            .and_then(|id| self.store.get(id))
            .and_then(|t| t.notes.as_ref())
            .is_some()
            && area.height > 6;

        // The column header only earns its row once there are rows to label
        // and vertical space to spare.
        let show_columns = !self.view.is_empty() && area.height >= 5;

        let rows = Layout::vertical([
            Constraint::Length(1),                         // summary
            Constraint::Length(u16::from(show_columns)),   // column header
            Constraint::Min(1),                            // list
            Constraint::Length(u16::from(show_notes) * 2), // notes preview
            Constraint::Length(1),                         // status / filter
        ])
        .split(area);

        frame.render_widget(Paragraph::new(self.header(theme)), rows[0]);

        // Recorded every pass so a click maps to the rows actually on screen,
        // and cleared when there are none rather than left pointing at a stale
        // rectangle.
        self.list_area = (!self.view.is_empty()).then_some(rows[2]);

        if self.view.is_empty() {
            let message = if self.filter.is_empty() {
                "No tasks yet. Press `a` to add one."
            } else {
                "Nothing matches this filter. Esc to clear."
            };
            frame.render_widget(
                Paragraph::new(Span::styled(message, Style::default().fg(theme.muted))),
                rows[2],
            );
        } else {
            let visible: Vec<&Task> = self
                .view
                .iter()
                .filter_map(|id| self.store.get(*id))
                .collect();

            // The list is indented by the selection marker, so the grid gets
            // what is left and the header is indented to match.
            let marker = 2u16;
            let grid = Grid::new(COLUMNS, rows[2].width.saturating_sub(marker));

            let items: Vec<ListItem> = visible
                .iter()
                .map(|task| ListItem::new(self.task_line(task, theme, &grid)))
                .collect();

            // Dim the selection when the panel is not focused, so it is
            // obvious which panel the keyboard is talking to.
            let list = List::new(items)
                .highlight_symbol(if ctx.focused { "▸ " } else { "  " })
                .highlight_style(if ctx.focused {
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.muted)
                });
            if show_columns && rows[1].height > 0 {
                // Indent the header past the selection marker so it lines up
                // with the columns it names.
                let header_area = Rect::new(
                    rows[1].x + marker,
                    rows[1].y,
                    rows[1].width.saturating_sub(marker),
                    1,
                );
                frame.render_widget(Paragraph::new(grid.header(theme)), header_area);
            }

            frame.render_stateful_widget(list, rows[2], &mut self.list_state);
        }

        if show_notes
            && let Some(notes) = self
                .selected_id()
                .and_then(|id| self.store.get(id))
                .and_then(|t| t.notes.clone())
        {
            frame.render_widget(
                Paragraph::new(Span::styled(notes, Style::default().fg(theme.muted)))
                    .wrap(Wrap { trim: true }),
                rows[3],
            );
        }

        // Bottom line: filter input takes priority, then status, then hints.
        let bottom = rows[4];
        if let Mode::Filter(field) = &self.mode {
            let (visible, cursor) = field.visible(bottom.width.saturating_sub(2) as usize);
            let mut spans = vec![
                Span::styled("/", Style::default().fg(theme.accent)),
                Span::styled(visible, Style::default().fg(theme.text)),
            ];
            // With nothing typed yet, offer the tags actually in use rather
            // than leaving the user to guess what will match.
            if field.is_blank() {
                let tags = self.store.all_tags();
                if !tags.is_empty() {
                    let hint = tags
                        .iter()
                        .take(6)
                        .map(|t| format!("#{t}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    spans.push(Span::styled(
                        format!("   {hint}"),
                        Style::default().fg(theme.muted),
                    ));
                }
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), bottom);
            frame.set_cursor_position((bottom.x + 1 + cursor as u16, bottom.y));
        } else {
            let line = match &self.status {
                Some((message, is_error)) => Span::styled(
                    truncate(message, bottom.width as usize),
                    Style::default().fg(if *is_error { theme.error } else { theme.muted }),
                ),
                // The frame carries the key hints, so an idle status line
                // stays blank rather than repeating them.
                None => Span::raw(""),
            };
            frame.render_widget(Paragraph::new(line), bottom);
        }

        // Modals draw last so they sit on top.
        match &self.mode {
            Mode::Edit(form) => Self::render_form(frame, area, theme, form),
            Mode::ConfirmDelete { title, .. } => Self::render_confirm(frame, area, theme, title),
            _ => {}
        }
    }

    fn shutdown(&mut self) {
        self.store.save_reporting();
    }
}

/// Placeholder text for an empty, unfocused field.
fn placeholder(field: Field) -> &'static str {
    match field {
        Field::Title => "what needs doing",
        Field::Notes => "optional detail",
        Field::Due => "empty for none",
        Field::Tags => "comma separated",
        Field::Priority => "",
    }
}

/// Contextual hint for the focused field.
fn hint_for(field: Field) -> &'static str {
    match field {
        Field::Title => "A one-line summary. Required.",
        Field::Notes => "Longer detail, shown under the list.",
        Field::Due => "2026-07-28, today, tomorrow, fri, +3d, 2w — or empty.",
        Field::Priority => "←/→ or space to change.",
        Field::Tags => "Comma or space separated, e.g. rust, mirador.",
    }
}

/// A rectangle of the given size, centred inside `area`.
fn centred(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// Truncate to `width` characters with an ellipsis, respecting char boundaries.
fn truncate(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_order_cycles_both_ways() {
        assert_eq!(Field::Title.next(), Field::Notes);
        assert_eq!(Field::Tags.next(), Field::Title);
        assert_eq!(Field::Title.prev(), Field::Tags);
        for f in Field::ORDER {
            assert_eq!(f.next().prev(), f);
        }
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 3), "he…");
        assert_eq!(truncate("日本語テスト", 3), "日本…");
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn centred_never_exceeds_its_container() {
        let area = Rect::new(0, 0, 20, 10);
        let r = centred(area, 100, 100);
        assert!(r.width <= area.width && r.height <= area.height);
        assert!(r.x >= area.x && r.y >= area.y);
        assert!(r.x + r.width <= area.x + area.width);
        assert!(r.y + r.height <= area.y + area.height);
    }

    #[test]
    fn centred_is_actually_centred() {
        let r = centred(Rect::new(0, 0, 20, 10), 10, 4);
        assert_eq!((r.x, r.y, r.width, r.height), (5, 3, 10, 4));
    }

    // ---------------------------------------------------------------------
    // Panel behaviour, driven through the same key events the terminal sends.
    // ---------------------------------------------------------------------

    /// A scratch directory removed when the guard drops.
    struct TempDir(std::path::PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A panel backed by a fresh, empty task file.
    fn panel(name: &str) -> (TodoPanel, TempDir) {
        let dir = std::env::temp_dir().join(format!("mirador-todo-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("todos.toml");
        // An empty file rather than no file: the panel seeds examples when the
        // file is absent, and these tests are about the panel, not the seed.
        // Writing it first exercises the same branch a second run takes.
        std::fs::write(&path, "").unwrap();
        let panel = TodoPanel::new(TodoConfig::default(), path).unwrap();
        (panel, TempDir(dir))
    }

    fn press(panel: &mut TodoPanel, code: KeyCode) {
        panel.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// Every key list mode responds to, paired with the `Binding::key` string
    /// that documents it.
    ///
    /// Keeping the pairing explicit is the point: adding a key here without
    /// adding it to `BINDINGS` fails the test below, which is how a key stops
    /// being able to quietly exist without appearing in the help.
    const DOCUMENTED_LIST_KEYS: &[(KeyCode, &str)] = &[
        (KeyCode::Char('a'), "a"),
        (KeyCode::Char('n'), "n"),
        (KeyCode::Enter, "↵"),
        (KeyCode::Char('e'), "e"),
        (KeyCode::Char(' '), "space"),
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
        (KeyCode::Char('p'), "p / P"),
        (KeyCode::Char('P'), "p / P"),
        (KeyCode::Char('s'), "s"),
        (KeyCode::Char('c'), "c"),
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

            let (mut p, _guard) = panel("keymap");
            add_task(&mut p, "a task");
            // Back to the list after the add form.
            assert!(matches!(p.mode, Mode::List));

            let outcome = p.handle_key(KeyEvent::new(*code, KeyModifiers::NONE));
            assert_eq!(
                outcome,
                KeyOutcome::Consumed,
                "`{key}` is documented but the list ignores it"
            );
        }
    }

    #[test]
    fn enter_opens_the_task_for_editing_rather_than_completing_it() {
        let (mut p, _guard) = panel("enter-edits");
        add_task(&mut p, "buy milk");
        let id = p.selected_id().unwrap();

        press(&mut p, KeyCode::Enter);
        assert!(
            matches!(p.mode, Mode::Edit(_)),
            "Enter must go inside the task"
        );
        assert!(
            !p.store.get(id).is_some_and(|t| t.done),
            "Enter must not complete the task"
        );
    }

    #[test]
    fn space_still_toggles_done() {
        let (mut p, _guard) = panel("space-toggles");
        // `c` first: with completed tasks hidden, finishing one drops it out of
        // the view, and there is then nothing selected to reopen.
        press(&mut p, KeyCode::Char('c'));
        add_task(&mut p, "buy milk");
        let id = p.selected_id().unwrap();

        press(&mut p, KeyCode::Char(' '));
        assert!(p.store.get(id).is_some_and(|t| t.done), "space completes");
        press(&mut p, KeyCode::Char(' '));
        assert!(
            !p.store.get(id).is_some_and(|t| t.done),
            "and reopens again"
        );
    }

    #[test]
    fn completing_a_task_hides_it_while_completed_tasks_are_hidden() {
        let (mut p, _guard) = panel("done-hides");
        add_task(&mut p, "buy milk");
        assert_eq!(p.view.len(), 1);

        press(&mut p, KeyCode::Char(' '));
        assert!(
            p.view.is_empty(),
            "a finished task leaves the list until `c` shows it again"
        );
        press(&mut p, KeyCode::Char('c'));
        assert_eq!(p.view.len(), 1, "`c` brings it back");
    }

    fn type_str(panel: &mut TodoPanel, text: &str) {
        for c in text.chars() {
            press(panel, KeyCode::Char(c));
        }
    }

    /// Add a task through the form: `a`, title, then any extra field edits.
    fn add_task(panel: &mut TodoPanel, title: &str) {
        press(panel, KeyCode::Char('a'));
        type_str(panel, title);
        press(panel, KeyCode::Enter);
    }

    #[test]
    fn adding_a_task_through_the_form_persists_it() {
        let (mut p, _dir) = panel("add");
        add_task(&mut p, "Publish placeholder");

        assert_eq!(p.store.tasks().len(), 1);
        assert_eq!(p.store.tasks()[0].title, "Publish placeholder");
        assert_eq!(p.view.len(), 1, "the new task must be visible");
        assert!(matches!(p.mode, Mode::List), "the form must close on save");

        // And it reached disk, not just memory.
        let reloaded = TaskStore::load(p.store.path()).unwrap();
        assert_eq!(reloaded.tasks().len(), 1);
    }

    #[test]
    fn the_form_captures_input_so_typing_q_cannot_quit() {
        let (mut p, _dir) = panel("capture");
        assert!(!p.captures_input(), "the list must not swallow global keys");

        press(&mut p, KeyCode::Char('a'));
        assert!(p.captures_input(), "an open form must swallow global keys");

        type_str(&mut p, "quit? q!");
        press(&mut p, KeyCode::Enter);
        assert_eq!(p.store.tasks()[0].title, "quit? q!");
    }

    #[test]
    fn a_blank_title_is_rejected_and_the_form_stays_open() {
        let (mut p, _dir) = panel("blank");
        press(&mut p, KeyCode::Char('a'));
        press(&mut p, KeyCode::Enter);

        assert!(p.store.tasks().is_empty(), "nothing should be created");
        match &p.mode {
            Mode::Edit(form) => {
                assert!(form.error.is_some(), "the failure must be explained");
            }
            _ => panic!("the form must stay open after a rejected save"),
        }
    }

    #[test]
    fn escape_cancels_without_creating_anything() {
        let (mut p, _dir) = panel("cancel");
        press(&mut p, KeyCode::Char('a'));
        type_str(&mut p, "never saved");
        press(&mut p, KeyCode::Esc);

        assert!(p.store.tasks().is_empty());
        assert!(matches!(p.mode, Mode::List));
    }

    #[test]
    fn a_bad_due_date_is_rejected_without_losing_the_typed_title() {
        let (mut p, _dir) = panel("baddate");
        press(&mut p, KeyCode::Char('a'));
        type_str(&mut p, "Ship it");
        press(&mut p, KeyCode::Tab); // Notes
        press(&mut p, KeyCode::Tab); // Due
        type_str(&mut p, "someday");
        press(&mut p, KeyCode::Enter);

        assert!(p.store.tasks().is_empty());
        match &p.mode {
            Mode::Edit(form) => {
                assert!(form.error.is_some());
                assert_eq!(form.title.value(), "Ship it", "typed input must survive");
            }
            _ => panic!("the form must stay open"),
        }
    }

    #[test]
    fn the_form_round_trips_every_field() {
        let (mut p, _dir) = panel("fields");
        press(&mut p, KeyCode::Char('a'));
        type_str(&mut p, "Full task");
        press(&mut p, KeyCode::Tab);
        type_str(&mut p, "some detail");
        press(&mut p, KeyCode::Tab);
        type_str(&mut p, "tomorrow");
        press(&mut p, KeyCode::Tab); // Priority
        press(&mut p, KeyCode::Right); // Low -> None
        press(&mut p, KeyCode::Tab);
        type_str(&mut p, "rust, mirador");
        press(&mut p, KeyCode::Enter);

        let task = &p.store.tasks()[0];
        assert_eq!(task.title, "Full task");
        assert_eq!(task.notes.as_deref(), Some("some detail"));
        let tomorrow = p.today.checked_add(jiff::Span::new().days(1)).unwrap();
        assert_eq!(task.due, Some(tomorrow));
        assert_eq!(task.priority, Priority::None);
        assert_eq!(task.tags, vec!["rust".to_string(), "mirador".to_string()]);
    }

    #[test]
    fn tags_accept_a_leading_hash_and_either_separator() {
        let (mut p, _dir) = panel("tags");
        press(&mut p, KeyCode::Char('a'));
        type_str(&mut p, "Tagged");
        for _ in 0..4 {
            press(&mut p, KeyCode::Tab);
        }
        type_str(&mut p, "#one  two,,#three");
        press(&mut p, KeyCode::Enter);

        assert_eq!(
            p.store.tasks()[0].tags,
            vec!["one".to_string(), "two".to_string(), "three".to_string()]
        );
    }

    #[test]
    fn space_toggles_completion_and_hides_the_task() {
        let (mut p, _dir) = panel("toggle");
        add_task(&mut p, "Do the thing");
        assert_eq!(p.view.len(), 1);

        press(&mut p, KeyCode::Char(' '));
        assert!(p.store.tasks()[0].done);
        assert!(p.store.tasks()[0].completed.is_some());
        assert!(p.view.is_empty(), "completed tasks hide by default");

        // Reveal completed tasks, then reopen it.
        press(&mut p, KeyCode::Char('c'));
        assert_eq!(p.view.len(), 1);
        press(&mut p, KeyCode::Char(' '));
        assert!(!p.store.tasks()[0].done);
        assert!(p.store.tasks()[0].completed.is_none());
    }

    #[test]
    fn editing_updates_in_place_rather_than_creating_a_duplicate() {
        let (mut p, _dir) = panel("edit");
        add_task(&mut p, "Original");
        let id = p.store.tasks()[0].id;

        press(&mut p, KeyCode::Char('e'));
        // Clear the pre-filled title, then retype.
        for _ in 0.."Original".len() {
            press(&mut p, KeyCode::Backspace);
        }
        type_str(&mut p, "Revised");
        press(&mut p, KeyCode::Enter);

        assert_eq!(p.store.tasks().len(), 1, "editing must not duplicate");
        assert_eq!(p.store.tasks()[0].id, id, "the id must be stable");
        assert_eq!(p.store.tasks()[0].title, "Revised");
    }

    #[test]
    fn delete_requires_confirmation() {
        let (mut p, _dir) = panel("delete");
        add_task(&mut p, "Doomed");

        press(&mut p, KeyCode::Char('d'));
        assert!(matches!(p.mode, Mode::ConfirmDelete { .. }));

        // Anything other than y backs out.
        press(&mut p, KeyCode::Char('n'));
        assert_eq!(p.store.tasks().len(), 1, "n must not delete");

        // Enter above all: it is the most reflexive key at a confirmation, the
        // prompt promises "any other key cancel", and deleting has no undo.
        press(&mut p, KeyCode::Char('d'));
        press(&mut p, KeyCode::Enter);
        assert_eq!(
            p.store.tasks().len(),
            1,
            "Enter must cancel, exactly as the prompt says it does"
        );

        press(&mut p, KeyCode::Char('d'));
        press(&mut p, KeyCode::Char('y'));
        assert!(p.store.tasks().is_empty());
        assert!(p.view.is_empty());
    }

    #[test]
    fn priority_cycles_from_the_list() {
        let (mut p, _dir) = panel("priority");
        add_task(&mut p, "Rank me");
        assert_eq!(p.store.tasks()[0].priority, Priority::Low);

        press(&mut p, KeyCode::Char('p'));
        assert_eq!(p.store.tasks()[0].priority, Priority::None);
        press(&mut p, KeyCode::Char('p'));
        assert_eq!(p.store.tasks()[0].priority, Priority::High);
    }

    #[test]
    fn navigation_clamps_at_both_ends() {
        let (mut p, _dir) = panel("nav");
        for i in 0..3 {
            add_task(&mut p, &format!("task {i}"));
        }
        assert_eq!(p.view.len(), 3);

        press(&mut p, KeyCode::Char('g'));
        assert_eq!(p.list_state.selected(), Some(0));
        for _ in 0..10 {
            press(&mut p, KeyCode::Char('k'));
        }
        assert_eq!(p.list_state.selected(), Some(0), "must not go negative");

        press(&mut p, KeyCode::Char('G'));
        assert_eq!(p.list_state.selected(), Some(2));
        for _ in 0..10 {
            press(&mut p, KeyCode::Char('j'));
        }
        assert_eq!(
            p.list_state.selected(),
            Some(2),
            "must not run past the end"
        );
    }

    #[test]
    fn navigation_on_an_empty_list_is_harmless() {
        let (mut p, _dir) = panel("empty-nav");
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Char('g'),
            KeyCode::Char('G'),
            KeyCode::Char(' '),
            KeyCode::Char('p'),
            KeyCode::Char('e'),
            KeyCode::Char('d'),
        ] {
            press(&mut p, code);
        }
        assert!(p.store.tasks().is_empty());
        assert_eq!(p.list_state.selected(), None);
    }

    #[test]
    fn filtering_narrows_the_view_and_escape_restores_it() {
        let (mut p, _dir) = panel("filter");
        add_task(&mut p, "Buy milk");
        add_task(&mut p, "Write docs");

        press(&mut p, KeyCode::Char('/'));
        type_str(&mut p, "milk");
        assert_eq!(p.view.len(), 1, "the filter applies as you type");
        press(&mut p, KeyCode::Enter);
        assert_eq!(p.filter, "milk");

        press(&mut p, KeyCode::Char('/'));
        press(&mut p, KeyCode::Esc);
        assert!(p.filter.is_empty(), "Esc must clear the filter");
        assert_eq!(p.view.len(), 2);
    }

    #[test]
    fn the_selection_stays_on_the_same_task_when_the_view_changes() {
        let (mut p, _dir) = panel("selection");
        add_task(&mut p, "alpha");
        add_task(&mut p, "beta");
        add_task(&mut p, "gamma");

        press(&mut p, KeyCode::Char('G'));
        let selected = p.selected_id().unwrap();

        // Changing sort reorders the list; the cursor should follow the task.
        press(&mut p, KeyCode::Char('s'));
        assert_eq!(p.selected_id(), Some(selected));
    }

    #[test]
    fn unhandled_keys_fall_through_to_the_application() {
        let (mut p, _dir) = panel("fallthrough");
        let outcome = p.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(
            outcome,
            KeyOutcome::Ignored,
            "Tab must reach the app so focus can move"
        );
    }
}
