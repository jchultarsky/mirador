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
use crate::keymap::{KeysConfig, Meta, PanelKeymap};
use crate::note::{Note, NoteStore};
use crate::panel::{KeyOutcome, Panel, RenderContext};
use crate::textarea::TextArea;
use crate::textfield::TextField;
use crate::theme::Theme;

/// What the list's keys do. The editor, the search box and the delete
/// question keep their own keys: they capture input, and a key moved there
/// could never be typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesAction {
    New,
    Edit,
    Delete,
    Up,
    Down,
    First,
    Last,
    ScrollUp,
    ScrollDown,
    Search,
    ShowPath,
}

const NONE: KeyModifiers = KeyModifiers::NONE;

#[cfg(test)]
thread_local! {
    /// Rows `note_line` has built on this thread, which is what
    /// `only_the_notes_on_screen_are_built_into_rows` weighs.
    static NOTE_LINES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Every key the list responds to, under `[notes.keys]`.
///
/// One declaration feeds the border hint, the status bar and the help
/// overlay, since the hints are derived from it.
pub const ACTIONS: &[Meta<NotesAction>] = &[
    Meta {
        action: NotesAction::New,
        name: "new",
        defaults: &[(KeyCode::Char('a'), NONE), (KeyCode::Char('n'), NONE)],
        label: "new",
        primary: true,
        joins: false,
        about: "write a new note",
    },
    Meta {
        action: NotesAction::Edit,
        name: "edit",
        defaults: &[(KeyCode::Enter, NONE), (KeyCode::Char('e'), NONE)],
        label: "edit",
        primary: true,
        joins: false,
        about: "open the selected note",
    },
    Meta {
        action: NotesAction::Delete,
        name: "delete",
        defaults: &[(KeyCode::Char('d'), NONE)],
        label: "delete",
        primary: true,
        joins: false,
        about: "delete the selected note, after asking",
    },
    Meta {
        action: NotesAction::Up,
        name: "up",
        defaults: &[(KeyCode::Up, NONE), (KeyCode::Char('k'), NONE)],
        label: "move selection",
        primary: false,
        joins: false,
        about: "select the note above",
    },
    Meta {
        action: NotesAction::Down,
        name: "down",
        defaults: &[(KeyCode::Down, NONE), (KeyCode::Char('j'), NONE)],
        label: "move selection",
        primary: false,
        joins: true,
        about: "select the note below",
    },
    Meta {
        action: NotesAction::First,
        name: "first",
        defaults: &[(KeyCode::Char('g'), NONE), (KeyCode::Home, NONE)],
        label: "first",
        primary: false,
        joins: false,
        about: "select the first note",
    },
    Meta {
        action: NotesAction::Last,
        name: "last",
        defaults: &[(KeyCode::Char('G'), NONE), (KeyCode::End, NONE)],
        label: "last",
        primary: false,
        joins: true,
        about: "select the last note",
    },
    Meta {
        action: NotesAction::ScrollUp,
        name: "scroll_up",
        defaults: &[(KeyCode::PageUp, NONE)],
        label: "scroll the note",
        primary: false,
        joins: false,
        about: "scroll the note's body up",
    },
    Meta {
        action: NotesAction::ScrollDown,
        name: "scroll_down",
        defaults: &[(KeyCode::PageDown, NONE)],
        label: "scroll the note",
        primary: false,
        joins: true,
        about: "scroll the note's body down",
    },
    Meta {
        action: NotesAction::Search,
        name: "search",
        defaults: &[(KeyCode::Char('/'), NONE)],
        label: "search",
        primary: false,
        joins: false,
        about: "search titles and bodies",
    },
    Meta {
        action: NotesAction::ShowPath,
        name: "show_path",
        defaults: &[(KeyCode::Char('o'), NONE)],
        label: "show file path",
        primary: false,
        joins: false,
        about: "show where the notes are saved",
    },
];

/// Hints for the list's keys that no table moves.
const FIXED: &[Binding] = &[Binding::extra("Esc", "clear search")];

/// `[notes.keys]` laid over [`ACTIONS`], or why it cannot be.
pub fn keymap(keys: &KeysConfig) -> Result<PanelKeymap<NotesAction>, String> {
    PanelKeymap::new("notes", ACTIONS, keys).map(|map| map.with_fixed(FIXED))
}

/// What the panel builds itself with; see [`PanelKeymap::or_defaults`].
fn list_keys(keys: &KeysConfig) -> PanelKeymap<NotesAction> {
    PanelKeymap::or_defaults("notes", ACTIONS, keys).with_fixed(FIXED)
}

/// What the panel says when a search matches nothing.
const NO_MATCH: &str = "Nothing matches this search. Esc to clear.";

/// Editing has a different vocabulary from browsing. Keeping it separate puts
/// the scratchpad's selection and clipboard actions in the border while the
/// form is open instead of continuing to advertise list actions that cannot
/// work there.
const TITLE_EDIT_BINDINGS: &[Binding] = &[
    Binding::primary("Tab", "body"),
    Binding::primary("Ctrl+S", "save"),
    Binding::primary("Esc", "cancel"),
];

/// The body's whole vocabulary. Only the primaries reach a screen — the
/// border — because extras are drawn by the `?` overlay alone, and `?` is
/// typed into a form that holds the keys, so the overlay cannot open over
/// it. The extras are kept so this table stays the full list of what the
/// body takes; the README is where the line keys are written down.
const BODY_EDIT_BINDINGS: &[Binding] = &[
    Binding::primary("Shift+←↑→↓", "select"),
    Binding::primary("Ctrl+V", "paste"),
    Binding::primary("Ctrl+S", "save"),
    Binding::extra("Ctrl+A", "select all"),
    Binding::extra("Ctrl+E", "end of line"),
    Binding::extra("Ctrl+K", "delete to end of line"),
    Binding::extra("Ctrl+U", "delete to start of line"),
    Binding::extra("Ctrl+W", "delete word before"),
    Binding::extra("Ctrl+C", "copy selection"),
    Binding::extra("Tab", "change field"),
    Binding::extra("Esc", "cancel"),
];

/// Once text is selected, the next useful action is copying or replacing it.
const SELECTION_BINDINGS: &[Binding] = &[
    Binding::primary("Ctrl+C", "copy"),
    Binding::primary("Ctrl+V", "paste"),
    Binding::primary("Ctrl+S", "save"),
    Binding::extra("Shift+←↑→↓", "adjust selection"),
    Binding::extra("Ctrl+A", "select all"),
    Binding::extra("Tab", "change field"),
    Binding::extra("Esc", "cancel"),
];

/// Columns of the note list. The date is right-aligned so the dates line up,
/// and is one cell wider than the shipped `%d %b` for the `·` an edited note
/// carries — sized to the date alone, every edited note read `·25 J…`.
pub(crate) const COLUMNS: &[Column] = &[
    Column::flex("title", 1),
    Column::fixed("date", 7).right().drops_below(25),
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

/// A note's body, wrapped once for a given width.
///
/// The reader used to wrap the whole body on *every* frame and then hand the
/// result to a `Paragraph` that scrolled past most of it — so the cost of
/// drawing a note was proportional to the note rather than to what was on
/// screen, which is the rule the agenda panel already had to learn. A 2MB note
/// cost 62ms a frame against a 250ms tick (#178).
///
/// Keyed on the text itself rather than on a note id and a dirty flag. Ids and
/// flags need every mutation site to remember to invalidate, and the one that
/// forgets shows stale prose with nothing to say it is stale. Comparing the
/// source is a memcmp — about 0.2ms for that same 2MB, against the 62ms it
/// saves — and it cannot be got wrong.
#[derive(Debug)]
struct WrappedBody {
    /// What was wrapped. The cache is valid exactly while this still matches.
    source: String,
    width: u16,
    rows: Vec<String>,
}

pub struct NotesPanel {
    store: NoteStore,
    config: NotesConfig,
    /// `[notes.keys]` over the defaults, for the list.
    keys: PanelKeymap<NotesAction>,
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
    /// The selected body, already wrapped. See [`WrappedBody`].
    wrapped_body: Option<WrappedBody>,
    /// The last body selection copied in this session. OSC 52 cannot read a
    /// system clipboard back, so keeping the text here is what makes Ctrl+V
    /// dependable even when the terminal declines the external copy request.
    clipboard: Option<String>,
}

impl NotesPanel {
    pub fn new(config: NotesConfig, path: std::path::PathBuf) -> anyhow::Result<Self> {
        let today = jiff::Zoned::now().date();
        let store = NoteStore::load_or_seed(path, today)?;
        let mut panel = Self {
            store,
            keys: list_keys(&config.keys),
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
            wrapped_body: None,
            clipboard: None,
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

    /// The selected body wrapped to `width`, wrapping only if it has to.
    ///
    /// See [`WrappedBody`] for why this is keyed on the text rather than on an
    /// id and a dirty flag. A function over the cache rather than a method on
    /// `self`, so the body can be borrowed straight from `self.store` while
    /// the cache is filled: as a method it needed `&mut self`, and the reader
    /// cloned the whole body every frame to get one — an allocation the size
    /// of the note, which is the cost the cache exists to avoid.
    fn wrapped_body<'a>(
        cache: &'a mut Option<WrappedBody>,
        body: &str,
        width: u16,
    ) -> &'a [String] {
        let fresh = cache
            .as_ref()
            .is_some_and(|cache| cache.width == width && cache.source == body);
        if !fresh {
            *cache = Some(WrappedBody {
                source: body.to_string(),
                width,
                rows: crate::grid::wrap(body, usize::from(width)),
            });
        }
        cache.as_ref().map_or(&[], |cache| cache.rows.as_slice())
    }

    /// Rows the selected body occupies, from the cache when it is warm.
    fn wrapped_rows(&self, body: &str, width: u16) -> Option<u16> {
        self.wrapped_body
            .as_ref()
            .filter(|cache| cache.width == width && cache.source == body)
            .map(|cache| u16::try_from(cache.rows.len()).unwrap_or(u16::MAX))
    }

    fn scroll_body(&mut self, delta: i16) {
        // Clamped so the body cannot be scrolled off into blank space, against
        // the height it actually occupies once wrapped rather than its count of
        // newlines. Before the first draw there is no width to wrap against, so
        // fall back to logical lines.
        let area = self.body_area;
        // The cache the reader filled on the last draw already knows this;
        // measuring again would walk the whole body on every keypress.
        let cached = self
            .selected()
            .zip(area)
            .and_then(|(note, area)| self.wrapped_rows(&note.body, area.width));
        let height = cached.unwrap_or_else(|| {
            self.selected().map_or(0, |note| match area {
                Some(area) if area.width > 0 => wrapped_height(&note.body, area.width),
                _ => u16::try_from(note.body.lines().count()).unwrap_or(u16::MAX),
            })
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

    /// Send the selected body text outward and retain it for an in-editor
    /// paste. The selection then collapses so another Ctrl+C can quit. The
    /// injected writer keeps the OSC 52 side effect out of tests.
    fn copy_body_selection_with(
        &mut self,
        copy: impl FnOnce(&str) -> std::io::Result<()>,
    ) -> KeyOutcome {
        let selected = match &self.mode {
            Mode::Edit(form) => form.body.selected_text(),
            _ => None,
        };
        let Some(text) = selected else {
            return KeyOutcome::Ignored;
        };

        let count = text.chars().count();
        self.clipboard = Some(text.clone());
        self.status = Some(match copy(&text) {
            // OSC 52 is write-only. "Sent" is true; "copied" is unknowable.
            Ok(()) => (format!("sent {count} chars — Ctrl+V pastes here"), false),
            // The internal copy still succeeded, so lead with what remains
            // usable rather than making the whole action sound lost.
            Err(error) => (
                format!("ready to paste; terminal clipboard failed: {error}"),
                true,
            ),
        });
        if let Mode::Edit(form) = &mut self.mode {
            form.body.clear_selection();
        }
        KeyOutcome::Consumed
    }

    fn paste_body_clipboard(&mut self) -> KeyOutcome {
        let Mode::Edit(form) = &mut self.mode else {
            return KeyOutcome::Ignored;
        };
        if form.field != Field::Body {
            self.set_status("Tab to the body to paste copied text");
            return KeyOutcome::Consumed;
        }
        let Some(text) = self.clipboard.clone() else {
            self.set_status("nothing copied here yet; terminal paste still works");
            return KeyOutcome::Consumed;
        };

        let count = text.chars().count();
        form.body.insert_text(&text);
        form.error = None;
        self.set_status(format!("pasted {count} chars"));
        KeyOutcome::Consumed
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
        // Esc is not in the map: it always backs out, here of a search.
        if key.code == KeyCode::Esc {
            if self.filter.is_empty() {
                return KeyOutcome::Ignored;
            }
            self.filter.clear();
            self.set_status("search cleared");
            self.refresh_view();
            return KeyOutcome::Consumed;
        }
        let Some(action) = self.keys.action(key) else {
            return KeyOutcome::Ignored;
        };
        match action {
            NotesAction::Down => self.select_down(1),
            NotesAction::Up => self.select_up(1),
            NotesAction::First => self.select_up(usize::MAX),
            NotesAction::Last => self.select_down(usize::MAX),

            // The list is usually short and the body usually is not, so the
            // paging keys move the note rather than the selection — the
            // opposite of the task list, where the rows are the long thing.
            NotesAction::ScrollDown => self.scroll_body(5),
            NotesAction::ScrollUp => self.scroll_body(-5),

            NotesAction::New => {
                self.mode = Mode::Edit(Box::new(EditForm::blank()));
            }

            NotesAction::Edit => {
                if let Some(note) = self.selected() {
                    self.mode = Mode::Edit(Box::new(EditForm::from_note(note)));
                }
            }

            NotesAction::Delete => {
                if let Some(note) = self.selected() {
                    self.mode = Mode::ConfirmDelete {
                        id: note.id,
                        title: note.title.clone(),
                    };
                }
            }

            NotesAction::Search => {
                self.mode = Mode::Search(TextField::with_value(self.filter.clone()));
            }

            NotesAction::ShowPath => {
                let path = self.store.path().display().to_string();
                self.set_status(path);
            }
        }
        KeyOutcome::Consumed
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> KeyOutcome {
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('v')) {
            return self.paste_body_clipboard();
        }

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
        form.error = None;
        KeyOutcome::Consumed
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> KeyOutcome {
        let Mode::Search(field) = &mut self.mode else {
            return KeyOutcome::Ignored;
        };
        match key.code {
            // Esc abandons the search rather than leaving a half-typed term
            // filtering the list behind a closed box — the task filter's Esc,
            // which the README promises for both. This one only closed the
            // box, and the term it had applied as it was typed stayed.
            KeyCode::Esc => {
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
        #[cfg(test)]
        NOTE_LINES.with(|built| built.set(built.get() + 1));
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

    /// The column header and the rows of the list that fit `list_area`.
    fn render_list(&mut self, frame: &mut Frame, list_area: Rect, theme: &Theme, focused: bool) {
        let marker = 2u16;
        let grid = Grid::new(COLUMNS, list_area.width.saturating_sub(marker));
        let header_area = Rect::new(
            list_area.x + marker,
            list_area.y,
            list_area.width.saturating_sub(marker),
            1,
        );
        frame.render_widget(Paragraph::new(grid.header(theme)), header_area);

        let rows_area = Rect {
            y: list_area.y + 1,
            height: list_area.height - 1,
            ..list_area
        };
        self.list_area = Some(rows_area);

        // Only the rows the pane can show are built, as in `todo.rs`.
        // Every note in the view used to become a `ListItem`, through an
        // index of the whole store, for a `List` that drew the dozen that
        // fit. The window is the one `List` would have scrolled to.
        let (shown, selected) = crate::selection::window(
            self.list_state.selected(),
            self.list_state.offset(),
            self.view.len(),
            usize::from(rows_area.height),
        );
        let items: Vec<ListItem> =
            crate::selection::in_order(self.store.notes(), &self.view[shown.clone()], |note| {
                note.id
            })
            .into_iter()
            .map(|note| ListItem::new(self.note_line(note, theme, &grid)))
            .collect();

        let list = List::new(items)
            .highlight_symbol(if focused { "▸ " } else { "  " })
            .highlight_style(if focused {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.muted)
            });
        // A list with no room draws nothing and, as `List` itself does,
        // leaves the scroll where it was.
        if !rows_area.is_empty() {
            let mut drawn = ListState::default().with_selected(selected.map(|at| at - shown.start));
            frame.render_stateful_widget(list, rows_area, &mut drawn);
            // Kept as `List` would have kept it, so the next frame scrolls
            // from here and a click maps through the rows on screen.
            self.list_state.select(selected);
            *self.list_state.offset_mut() = shown.start;
        }
    }

    /// The detail pane: the selected note's title, date and body.
    fn render_detail(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Borrowed from the store field rather than through `selected()`, so
        // the wrap cache — a different field — can still be filled below.
        let Some(note) = self.selected_id().and_then(|id| self.store.get(id)) else {
            // The key `[notes.keys]` gave `new`, and no offer when it gave
            // none. Prose in a pane of several rows, so wrapped into them,
            // and the last row says so if they run out (invariant 19).
            let message = if !self.view.is_empty() || !self.filter.is_empty() {
                NO_MATCH.to_string()
            } else if let Some(key) = self.keys.keys(NotesAction::New).first() {
                format!("No notes yet. Press `{key}` to write one.")
            } else {
                "No notes yet.".to_string()
            };
            // Wrapped into the pane, the last row ending in `…` if it is cut.
            // Handed whole to a `Paragraph`, the terminal cut it in silence,
            // and the part it lost was "Esc to clear" — the instruction.
            frame.render_widget(
                Paragraph::new(crate::grid::fitted_rows(&message, area, false))
                    .style(Style::default().fg(theme.muted)),
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

        // One row, so cut to it with an ellipsis. It used to be wrapped — by
        // `grid`, since ratatui's wrapper panics on text mirador did not
        // write — and the row drew the first line of the wrap and dropped the
        // rest in silence. The whole title is in the list above.
        frame.render_widget(
            Paragraph::new(crate::grid::truncate(
                &note.title,
                usize::from(rows[0].width),
            ))
            .style(
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            rows[0],
        );

        // Two labelled values, assembled so each drops whole: `written 10 Sep
        // 202` is a year nobody wrote. The gap rides with the part it
        // introduces, as everywhere else.
        let mut dates = vec![vec![Span::styled(
            format!("written {}", note.created.strftime("%d %b %Y")),
            Style::default().fg(theme.muted),
        )]];
        if let Some(updated) = note.updated {
            dates.push(vec![Span::styled(
                format!("   edited {}", updated.strftime("%d %b %Y")),
                Style::default().fg(theme.muted),
            )]);
        }
        frame.render_widget(
            Paragraph::new(crate::grid::assemble(dates, rows[1].width)),
            rows[1],
        );

        if rows[2].height == 0 {
            return;
        }
        let body = if note.body.trim().is_empty() {
            Paragraph::new(Span::styled("(no body)", Style::default().fg(theme.muted)))
        } else {
            // Only the rows that will be drawn are built. Handing a `Paragraph`
            // the whole wrapped body and asking it to `scroll` past most of it
            // made the cost of a frame proportional to the note rather than to
            // the pane — see `WrappedBody` and #178.
            //
            // The last of them ends in `…` while there is more below it
            // (invariant 19). The body scrolls, but nothing else on screen
            // says so, and a cut that falls between two sentences looks like
            // the end of the note.
            let height = usize::from(rows[2].height);
            let width = usize::from(rows[2].width);
            let wrapped = Self::wrapped_body(&mut self.wrapped_body, &note.body, rows[2].width);
            let start = usize::from(self.body_scroll).min(wrapped.len());
            let end = start.saturating_add(height).min(wrapped.len());
            let more = end < wrapped.len();
            let visible: Vec<Line<'static>> = wrapped[start..end]
                .iter()
                .enumerate()
                .map(|(at, row)| {
                    if more && start + at + 1 == end {
                        Line::from(crate::grid::truncate(
                            &format!("{}…", row.trim_end()),
                            width,
                        ))
                    } else {
                        Line::from(row.clone())
                    }
                })
                .collect();
            Paragraph::new(visible).style(Style::default().fg(theme.text))
        };
        // The reader wraps even though the editor does not: prose written at
        // one width has to be readable at another.
        // Remembered so scrolling can clamp against the *wrapped* height. The
        // reader wraps and the clamp used to count `body.lines()`, so a note
        // written as one paragraph — the normal way — reported a single line
        // and would not scroll at all however long it was.
        self.body_area = Some(rows[2]);
        // No `.scroll()`: the offset was applied when the visible rows were
        // chosen. Scrolling here as well would skip twice.
        frame.render_widget(body, rows[2]);
    }

    /// The active search, and the count when nothing else on screen is
    /// showing it — or `None` when neither has anything to say.
    ///
    /// The count used to be here unconditionally, which printed the same fact
    /// twice: `┤1├` in the border and `1 note` on the first interior row, two
    /// rows apart. Same shape as the tasks summary, found the same way and
    /// fixed the same way — the border keeps it, and the row comes back where
    /// the border stops carrying it and nothing else says it instead.
    ///
    /// Those cases are worth naming, because they are the moments the line is
    /// most useful. A failed save takes the counter for `unsaved!`, which is
    /// the worst possible moment to also stop saying how much is at stake.
    /// An empty panel has no counter at all, and under a search `no notes`
    /// says why nothing matched. Without one the message in the body says `No
    /// notes yet`, and `no notes` straight above it was the count twice again,
    /// in a row the message needed for the key it names.
    ///
    /// Fitted to `width` as one part, so a long search term ends in `…`
    /// rather than wherever the terminal's edge fell — the term is as long
    /// as the reader made it.
    fn summary_line(&self, theme: &Theme, width: u16) -> Option<Line<'static>> {
        let total = self.store.notes().len();
        let mut spans = Vec::new();
        if self.store.last_error.is_some() || (total == 0 && !self.filter.is_empty()) {
            spans.push(Span::styled(
                match total {
                    0 => "no notes".to_string(),
                    1 => "1 note".to_string(),
                    n => format!("{n} notes"),
                },
                Style::default().fg(theme.muted),
            ));
        }
        if !self.filter.is_empty() {
            // The gap belongs to the item it introduces, so it is spent only
            // when there is something in front of it to be separated from.
            let gap = if spans.is_empty() { "" } else { "   " };
            spans.push(Span::styled(
                format!("{gap}search: {}", self.filter),
                Style::default().fg(theme.label),
            ));
        }
        (!spans.is_empty()).then(|| crate::grid::assemble(vec![spans], width))
    }

    /// The bottom line: a delete confirmation, the search prompt, or the last
    /// status message. A save failure outranks nothing — it stays until the
    /// next keypress, because a note that failed to save must not look saved.
    /// The status line, cut to `width` with an ellipsis rather than by the
    /// terminal. See `StocksPanel::status_line`: a note title in a delete
    /// prompt and a typed search term are both as long as the user made them.
    fn status_line(&self, theme: &Theme, width: u16) -> Line<'static> {
        crate::grid::assemble(vec![self.status_text(theme, width).spans], width)
    }

    fn status_text(&self, theme: &Theme, width: u16) -> Line<'static> {
        match (&self.mode, &self.status) {
            (Mode::ConfirmDelete { title, .. }, _) => Line::from(Span::styled(
                format!("delete \"{title}\"?  y / n"),
                Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            )),
            (Mode::Search(field), _) => {
                // The window, not the whole term: drawn whole and cut by
                // `assemble`, a long term took the caret with it, and the
                // caret followed the term wherever the cursor was.
                const LABEL: &str = "search  ";
                let room = usize::from(width).saturating_sub(LABEL.len());
                let (text, caret) = field.visible_inline(room);
                let mut spans = vec![Span::styled(LABEL, Style::default().fg(theme.accent))];
                spans.extend(Self::editor_line(&text, caret, None, theme).spans);
                Line::from(spans)
            }
            (_, Some((message, is_error))) => Line::from(Span::styled(
                message.clone(),
                Style::default().fg(if *is_error { theme.error } else { theme.muted }),
            )),
            _ => Line::default(),
        }
    }

    /// One editor row, split wherever the selection or caret changes style.
    fn editor_line(
        text: &str,
        caret: Option<usize>,
        selection: Option<std::ops::Range<usize>>,
        theme: &Theme,
    ) -> Line<'static> {
        let selection = selection.filter(|range| range.start < range.end);
        let mut cuts = vec![0, text.len()];
        if let Some(range) = &selection {
            cuts.extend([range.start, range.end]);
        }
        if let Some(at) = caret {
            cuts.push(at);
        }
        cuts.sort_unstable();
        cuts.dedup();

        let mut spans = Vec::new();
        for pair in cuts.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if caret == Some(start) {
                spans.push(Span::styled("▏", Style::default().fg(theme.accent)));
            }
            if start == end {
                continue;
            }
            let selected = selection
                .as_ref()
                .is_some_and(|range| start >= range.start && end <= range.end);
            let style = if selected {
                Style::default()
                    .fg(theme.text)
                    .add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(theme.text)
            };
            spans.push(Span::styled(text[start..end].to_string(), style));
        }
        if caret == Some(text.len()) {
            spans.push(Span::styled("▏", Style::default().fg(theme.accent)));
        }
        Line::from(spans)
    }

    /// The edit form, drawn over the whole panel.
    fn render_form(
        frame: &mut Frame,
        area: Rect,
        theme: &Theme,
        form: &EditForm,
        status: Option<(&str, bool)>,
    ) {
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

        // The caret is a cell of its own, as in the body, and goes where the
        // next key lands. It used to be pushed after the visible text, so
        // after Home it sat at the end while typing went in at the start,
        // and a window that filled the field pushed it past the edge. The
        // window keeps its cell for the caret while the body has the focus
        // too, so Tab does not slide the title along by one.
        let (title_text, title_caret) = form
            .title
            .visible_inline(usize::from(rows[1].width.saturating_sub(7)));
        let mut title_spans = vec![Span::styled("title  ", active(Field::Title))];
        let caret = title_caret.filter(|_| form.field == Field::Title);
        title_spans.extend(Self::editor_line(&title_text, caret, None, theme).spans);
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
                    // `visible_with_selection` owns the arithmetic — it is
                    // measured in display cells, and getting that wrong here
                    // is what invariant 9 is about.
                    let (text, caret, selection) = form.body.visible_with_selection(index, width);
                    // Draw the caret inline rather than moving the terminal
                    // cursor: the panel does not own the screen cursor, and a
                    // caret that only appears in the focused field is what
                    // tells the user where typing will land.
                    Self::editor_line(&text, caret.filter(|_| editing), selection, theme)
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), rows[3]);
        }

        frame.render_widget(
            Paragraph::new(Self::form_footer(form, status, theme, rows[4].width)),
            rows[4],
        );
    }

    /// The form's last row, fitted to `width`. A message — an error, or a
    /// status carrying an operating system's error text — is prose and is cut
    /// with an ellipsis as one part; the keys are parts of their own and drop
    /// whole, so no hint is left half-spelled. Both used to be drawn at their
    /// natural width and cut by the terminal. Where the keys end in the way
    /// out it is the last to go, as in a prompt's help; see
    /// `prompt::way_out_last`.
    fn form_footer(
        form: &EditForm,
        status: Option<(&str, bool)>,
        theme: &Theme,
        width: u16,
    ) -> Line<'static> {
        let muted = Style::default().fg(theme.muted);
        let message = |text: String, style: Style| vec![vec![Span::styled(text, style)]];
        let keys = |hints: [&'static str; 3]| {
            hints
                .iter()
                .enumerate()
                .map(|(index, hint)| {
                    let gap = if index == 0 { "" } else { "   " };
                    vec![Span::styled(format!("{gap}{hint}"), muted)]
                })
                .collect()
        };
        let parts = match (&form.error, status) {
            (Some(text), _) => message(text.clone(), Style::default().fg(theme.error)),
            (None, Some((text, is_error))) => message(
                text.to_string(),
                Style::default().fg(if is_error { theme.error } else { theme.muted }),
            ),
            (None, None) if form.body.has_selection() => {
                keys(["Ctrl+C copy", "Ctrl+V replace", "Ctrl+S save"])
            }
            (None, None) if form.field == Field::Body => {
                keys(["Shift+arrows select", "Ctrl+A all", "Ctrl+V paste"])
            }
            (None, None) => crate::prompt::way_out_last(
                ["Tab body", "Ctrl+S save", "Esc cancel"]
                    .map(str::to_string)
                    .to_vec(),
                "   ",
                width,
                muted,
            ),
        };
        crate::grid::assemble(parts, width)
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

    fn bindings(&self) -> &[Binding] {
        match &self.mode {
            Mode::Edit(form) if form.body.has_selection() => SELECTION_BINDINGS,
            Mode::Edit(form) if form.field == Field::Body => BODY_EDIT_BINDINGS,
            Mode::Edit(_) => TITLE_EDIT_BINDINGS,
            _ => self.keys.bindings(),
        }
    }

    fn set_keys(&mut self, config: &crate::config::Config) {
        self.keys = list_keys(&config.notes.keys);
    }

    fn refresh_interval(&self) -> std::time::Duration {
        // Nothing here changes on its own; the tick only rolls the date over
        // so a note written after midnight is stamped correctly.
        std::time::Duration::from_mins(1)
    }

    fn tick(&mut self) -> bool {
        // Only the date matters here: a note written or edited after midnight
        // is stamped with the new day. Nothing drawn depends on it, so the one
        // redraw a rollover asks for is a spare one, once a day.
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

    fn copy_selection(&mut self) -> KeyOutcome {
        self.copy_body_selection_with(crate::clipboard::copy)
    }

    fn handle_paste(&mut self, text: &str) -> KeyOutcome {
        self.status = None;
        // Claimed and dropped at the delete confirmation. A paste nobody
        // claims is typed in key by key, and one beginning with `y` would
        // answer the question and delete the note.
        if matches!(self.mode, Mode::ConfirmDelete { .. }) {
            return KeyOutcome::Consumed;
        }
        let Mode::Edit(form) = &mut self.mode else {
            return KeyOutcome::Ignored;
        };

        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let count = text.chars().count();
        match form.field {
            Field::Body => form.body.insert_text(&text),
            Field::Title => {
                // A title is one line. Preserve the words from a multiline
                // paste without letting Enter accidentally submit the form.
                for character in text.chars() {
                    match character {
                        '\n' | '\t' => form.title.insert(' '),
                        c if !c.is_control() => form.title.insert(c),
                        _ => {}
                    }
                }
            }
        }
        form.error = None;
        self.set_status(format!("pasted {count} chars from terminal"));
        KeyOutcome::Consumed
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
            let status = self
                .status
                .as_ref()
                .map(|(message, is_error)| (message.as_str(), *is_error));
            Self::render_form(frame, area, theme, form, status);
            return;
        }

        // Computed before the split: whether the row exists is decided by
        // whether it has anything to say, and on a calm unfiltered panel it
        // does not — so the list and the note it is pointing at get the row,
        // or on an empty panel the message saying there are none. The
        // footprint therefore changes when a search opens or a save fails,
        // which are both moments the panel has visibly changed anyway.
        let summary = self.summary_line(theme, area.width);
        let rows = Layout::vertical([
            Constraint::Length(u16::from(summary.is_some())), // summary
            Constraint::Min(1),                               // master + detail
            Constraint::Length(1),                            // status
        ])
        .split(area);

        if let Some(summary) = summary {
            frame.render_widget(Paragraph::new(summary), rows[0]);
        }

        // Master-detail split. Stacked by default: side by side divides a
        // finite width between a list that wants room for titles and a body
        // that wants room for prose, and neither gets enough. Stacking gives
        // both the full width and spends height instead.
        let body = rows[1];

        // With nothing listed there is no list and no note for it to point
        // at, so no split: the message has the whole body. Split, the list's
        // half stood blank above the rule while the message was cut in the
        // half under it, and at a height of 4 that half had no rows at all
        // and the offer was gone without an `…` (invariant 19).
        if self.view.is_empty() {
            self.detail_area = Some(body);
            self.render_detail(frame, body, theme);
            frame.render_widget(
                Paragraph::new(self.status_line(theme, rows[2].width)),
                rows[2],
            );
            return;
        }

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

        if list_area.height > 1 {
            self.render_list(frame, list_area, theme, ctx.focused);
        }

        self.detail_area = Some(detail_area);
        self.render_detail(frame, detail_area, theme);

        frame.render_widget(
            Paragraph::new(self.status_line(theme, rows[2].width)),
            rows[2],
        );
    }

    fn shutdown(&mut self) {
        self.store.save_reporting();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    /// Every note with a row built for it, in this thread: the one count that
    /// shows whether a frame's work scales with the screen or with the store.
    #[test]
    fn only_the_notes_on_screen_are_built_into_rows() {
        let (mut p, _guard) = panel("built");
        for i in 0..60 {
            add_note(&mut p, &format!("Note {i:02}"), "");
        }
        for height in [8u16, 16, 40] {
            draw(&mut p, 40, height);
            let before = NOTE_LINES.with(std::cell::Cell::get);
            draw(&mut p, 40, height);
            let built = NOTE_LINES.with(std::cell::Cell::get) - before;
            let rows = usize::from(p.list_area.expect("a list").height);
            assert!(rows < 60, "the screen is shorter than the store");
            assert!(
                built <= rows,
                "{built} rows built for {rows} on screen at height {height}"
            );
        }
    }

    /// Building only the rows on screen means `List` is handed a window and
    /// never sees the scroll, so the panel keeps it. Walk a list three
    /// screens long down and back: the marked row is always the selected
    /// note, and a click always lands on the note under it — a scroll left
    /// at the top maps a click on the last screen to the first. Which note
    /// is under the row is read off the screen *before* the click: drawn
    /// after it, the frame scrolls to whatever was selected, and any note at
    /// all would look like the one clicked.
    #[test]
    fn a_long_list_scrolls_and_clicks_as_it_always_did() {
        let (mut p, _guard) = panel("window");
        for i in 0..30 {
            add_note(&mut p, &format!("Note {i:02}"), "");
        }
        press(&mut p, KeyCode::Char('g'));

        let selected_title = |p: &NotesPanel| {
            let id = p.selected_id().expect("a selection");
            p.store.get(id).expect("the note").title.clone()
        };
        let keys = std::iter::repeat_n(KeyCode::Char('j'), 29)
            .chain(std::iter::repeat_n(KeyCode::Char('k'), 29));
        for (step, code) in keys.enumerate() {
            press(&mut p, code);
            let screen = draw(&mut p, 40, 24);
            let title = selected_title(&p);
            let marked = screen
                .lines()
                .find(|row| row.contains('▸'))
                .unwrap_or_else(|| panic!("{step}: a row is marked:\n{screen}"));
            assert!(marked.contains(&title), "{step}: {title}:\n{screen}");
        }

        press(&mut p, KeyCode::Char('G'));
        let before = draw(&mut p, 40, 24);
        let area = p.list_area.expect("the list was drawn");
        assert!(p.list_state.offset() > 0, "the last screen is scrolled");
        // Below the top row, so a click that forgets where the list starts on
        // screen misses as surely as one that forgets the scroll.
        let row = area.y + 2;
        let under = before
            .lines()
            .nth(usize::from(row))
            .and_then(|line| line.find("Note ").map(|at| line[at..at + 7].to_owned()))
            .unwrap_or_else(|| panic!("a note is drawn on row {row}:\n{before}"));
        assert_ne!(selected_title(&p), under, "the click has somewhere to go");
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x + 4,
            row,
            modifiers: KeyModifiers::NONE,
        };
        p.handle_mouse(click, area);
        assert_eq!(selected_title(&p), under, "clicked row {row} of:\n{before}");
    }

    fn panel(name: &str) -> (NotesPanel, TempDir) {
        let dir = TempDir::new(&format!("notes-{name}"));
        let path = dir.join("notes.toml");
        // An empty file rather than no file: the panel seeds an example note
        // when the file is absent, and these tests are about the panel, not
        // the seed. This is the branch every run after the first one takes.
        std::fs::write(&path, "").unwrap();
        let p = NotesPanel::new(NotesConfig::default(), path).unwrap();
        (p, dir)
    }

    /// The panel as drawn, one row per line.
    fn rows_of(p: &mut NotesPanel, width: u16, height: u16) -> Vec<String> {
        crate::widgets::testing::rows(&crate::widgets::testing::rendered(p, width, height))
    }

    /// `render_form` was never executed by a test. The caret is the whole
    /// point of the form's drawing: it is what tells the writer which field
    /// the next key lands in, and it has to follow Tab.
    #[test]
    fn the_note_form_draws_its_heading_and_puts_the_caret_in_the_active_field() {
        let (mut p, _g) = panel("form-draw");
        press(&mut p, KeyCode::Char('a'));
        let rows = rows_of(&mut p, 60, 12);
        assert!(rows[0].contains("NEW NOTE"), "{rows:?}");
        assert!(rows[1].starts_with("title"), "{rows:?}");
        assert!(
            rows[1].contains('▏'),
            "the caret starts in the title: {rows:?}"
        );
        assert!(rows[2].starts_with("body"), "{rows:?}");

        press(&mut p, KeyCode::Tab);
        let rows = rows_of(&mut p, 60, 12);
        assert!(
            !rows[1].contains('▏'),
            "after Tab the title has no caret: {rows:?}"
        );
        press(&mut p, KeyCode::Esc);

        add_note(&mut p, "Release checklist", "");
        press(&mut p, KeyCode::Enter);
        let rows = rows_of(&mut p, 60, 12);
        assert!(
            rows[0].contains("EDIT NOTE"),
            "opening an existing note says so: {rows:?}"
        );
        assert!(
            rows[1].contains("Release checklist"),
            "and shows its title: {rows:?}"
        );
    }

    fn press(p: &mut NotesPanel, code: KeyCode) {
        p.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn chord(p: &mut NotesPanel, code: KeyCode, modifiers: KeyModifiers) {
        p.handle_key(KeyEvent::new(code, modifiers));
    }

    fn type_str(p: &mut NotesPanel, text: &str) {
        for c in text.chars() {
            press(p, KeyCode::Char(c));
        }
    }

    /// Add a note through the form: `a`, title, Tab, body, Ctrl+S.
    /// Draw the panel and return what reached the screen.
    fn draw(p: &mut NotesPanel, width: u16, height: u16) -> String {
        rows_of(p, width, height).join("\n")
    }

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

    /// `┤1├` in the border and `1 note` two rows below it is the same fact
    /// twice, and it was costing a row in a panel whose whole job is showing a
    /// list and the note it points at. Same finding as the tasks summary, one
    /// panel over — spotted only because the README's drawing of this panel
    /// was screenshotted beside the drawing of that one.
    ///
    /// The row has to come back where nothing else says the count: a failed
    /// save, where the counter is spent on `unsaved!`; a search, which is a
    /// fact the border does not carry in words; and an empty panel under a
    /// search, where there is no counter at all. An empty panel with no
    /// search has none either, and its message says `No notes yet` — so a
    /// `no notes` above it is the count twice again.
    #[test]
    fn the_note_count_is_shown_once_and_by_the_border() {
        let (mut p, _g) = panel("count-once");
        let theme = Theme::default();
        let text = |p: &NotesPanel| -> Option<String> {
            p.summary_line(&theme, 80)
                .map(|line| line.spans.iter().map(|s| s.content.to_string()).collect())
        };

        // Empty: no counter in the border, and the message says so.
        assert_eq!(p.counter(), None, "an empty panel has no counter");
        assert_eq!(text(&p), None, "the message says it: {:?}", text(&p));
        p.filter = "release".to_string();
        assert_eq!(text(&p).as_deref(), Some("no notes   search: release"));
        p.filter.clear();
        p.store.last_error = Some("read-only file system".to_string());
        assert_eq!(text(&p).as_deref(), Some("no notes"));
        p.store.last_error = None;

        add_note(&mut p, "Release checklist", "Bump the version");
        assert_eq!(
            p.counter().as_deref(),
            Some("1"),
            "the border carries the count"
        );
        assert_eq!(
            text(&p),
            None,
            "so the summary row is not drawn at all: {:?}",
            text(&p)
        );

        // A search is not something the border says in words.
        p.filter = "release".to_string();
        let searching = text(&p).expect("a search shows");
        assert!(searching.starts_with("search: "), "got {searching:?}");
        assert!(
            !searching.contains("1 note"),
            "and still does not repeat the count: {searching:?}"
        );
        p.filter.clear();

        // A failed save takes the counter, so the count comes back.
        p.store.last_error = Some("read-only file system".to_string());
        assert_eq!(p.counter().as_deref(), Some("unsaved!"));
        assert_eq!(text(&p).as_deref(), Some("1 note"));
    }

    #[test]
    fn a_note_can_be_written_and_survives_a_reload() {
        let (mut p, guard) = panel("write");
        add_note(&mut p, "Shopping", "milk\neggs");

        assert!(matches!(p.mode, Mode::List), "the form must close on save");
        assert_eq!(p.view.len(), 1);

        let reloaded = NotesPanel::new(NotesConfig::default(), guard.join("notes.toml")).unwrap();
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

    /// The reader must cost what is on screen, not what is in the note.
    ///
    /// #178: the body was wrapped in full on every frame and then scrolled
    /// past, so a 2MB note cost 62ms a frame against a 250ms tick. The fix is
    /// a cache, and the property worth pinning is not the timing — which is
    /// machine-dependent — but that a second draw of an unchanged note does no
    /// wrapping at all.
    #[test]
    fn an_unchanged_note_is_not_rewrapped_on_every_frame() {
        let (mut p, _g) = panel("rewrap");
        add_note(&mut p, "Long", &"lorem ipsum dolor sit amet ".repeat(40));
        draw(&mut p, 40, 20);

        let first = p
            .wrapped_body
            .as_ref()
            .expect("the first draw fills the cache")
            .rows
            .as_ptr();

        draw(&mut p, 40, 20);
        let second = p.wrapped_body.as_ref().unwrap().rows.as_ptr();
        assert_eq!(
            first, second,
            "the second draw rebuilt the wrap instead of reusing it"
        );
    }

    /// The cache is keyed on the text, so an edit invalidates it with nothing
    /// having to remember to say so.
    #[test]
    fn editing_a_note_rewraps_it() {
        let (mut p, _g) = panel("rewrap-edit");
        add_note(&mut p, "Long", "one two three");
        draw(&mut p, 40, 20);
        let before = p.wrapped_body.as_ref().unwrap().rows.clone();

        add_note(&mut p, "Other", "wholly different prose here");
        draw(&mut p, 40, 20);
        let after = p.wrapped_body.as_ref().unwrap().rows.clone();
        assert_ne!(before, after, "a different body must be rewrapped");
    }

    /// So does a resize, since the rows depend on the width.
    #[test]
    fn resizing_rewraps_the_body() {
        let (mut p, _g) = panel("rewrap-resize");
        add_note(&mut p, "Long", &"lorem ipsum dolor sit amet ".repeat(20));
        draw(&mut p, 40, 20);
        let wide = p.wrapped_body.as_ref().unwrap().rows.len();
        draw(&mut p, 20, 20);
        let narrow = p.wrapped_body.as_ref().unwrap().rows.len();
        assert!(
            narrow > wide,
            "a narrower pane needs more rows: {wide} then {narrow}"
        );
    }

    /// Scrolling still shows the right slice — the offset moved from the
    /// `Paragraph` to the row selection, and applying it in both places would
    /// skip twice.
    #[test]
    fn the_visible_rows_follow_the_scroll_offset() {
        let (mut p, _g) = panel("rewrap-scroll");
        let body = (0..60)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        add_note(&mut p, "Long", &body);
        let top = draw(&mut p, 40, 20);
        assert!(top.contains("line0"), "the top of the body:\n{top}");

        p.scroll_body(5);
        let scrolled = draw(&mut p, 40, 20);
        assert!(
            !scrolled.contains("line0") && scrolled.contains("line5"),
            "scrolling by five should start at line5:\n{scrolled}"
        );
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
        assert!(p.store.notes().is_empty(), "{:?}", p.store.notes());
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
    fn selected_body_text_can_be_copied_and_reused() {
        let (mut p, _g) = panel("copy-paste");
        add_note(&mut p, "Scratch", "red blue");
        press(&mut p, KeyCode::Enter);
        assert_eq!(p.bindings()[0].key, "Tab", "the title points to the body");
        press(&mut p, KeyCode::Tab);
        assert_eq!(p.bindings()[0].action, "select");

        for _ in 0..4 {
            chord(&mut p, KeyCode::Left, KeyModifiers::SHIFT);
        }
        assert_eq!(p.bindings()[0].key, "Ctrl+C");
        assert_eq!(
            p.copy_body_selection_with(|text| {
                assert_eq!(text, "blue");
                Ok(())
            }),
            KeyOutcome::Consumed
        );
        assert_eq!(p.clipboard.as_deref(), Some("blue"));
        let Mode::Edit(form) = &p.mode else {
            unreachable!()
        };
        assert!(!form.body.has_selection(), "copy collapses the selection");
        assert_eq!(
            p.copy_body_selection_with(|_| panic!("a second Ctrl+C must fall through")),
            KeyOutcome::Ignored
        );

        // Select another word and replace it with the internal copy. This does
        // not depend on the terminal accepting OSC 52 or exposing a readable
        // system clipboard.
        press(&mut p, KeyCode::Home);
        for _ in 0..3 {
            chord(&mut p, KeyCode::Right, KeyModifiers::SHIFT);
        }
        chord(&mut p, KeyCode::Char('v'), KeyModifiers::CONTROL);

        let Mode::Edit(form) = &p.mode else {
            unreachable!()
        };
        assert_eq!(form.body.value(), "blue blue");
        assert!(!form.body.has_selection(), "paste consumes the selection");
        assert_eq!(
            p.status.as_ref().map(|s| s.0.as_str()),
            Some("pasted 4 chars")
        );
    }

    #[test]
    fn terminal_paste_preserves_multiline_scratchpad_text() {
        let (mut p, _g) = panel("terminal-paste");
        add_note(&mut p, "Scratch", "red blue");
        press(&mut p, KeyCode::Enter);
        press(&mut p, KeyCode::Tab);
        press(&mut p, KeyCode::Home);
        for _ in 0..3 {
            chord(&mut p, KeyCode::Right, KeyModifiers::SHIFT);
        }

        assert_eq!(p.handle_paste("one\r\ntwo\t"), KeyOutcome::Consumed);
        let Mode::Edit(form) = &p.mode else {
            unreachable!()
        };
        assert_eq!(form.body.value(), "one\ntwo\t blue");
        assert!(!form.body.has_selection());
        assert_eq!(
            p.status.as_ref().map(|s| s.0.as_str()),
            Some("pasted 8 chars from terminal")
        );
    }

    #[test]
    fn ctrl_c_remains_quit_when_the_editor_has_nothing_to_copy() {
        let (mut p, _g) = panel("copy-empty");
        press(&mut p, KeyCode::Char('a'));
        press(&mut p, KeyCode::Tab);
        assert_eq!(
            p.copy_body_selection_with(|_| panic!("copy must not be called")),
            KeyOutcome::Ignored
        );
    }

    #[test]
    fn a_body_selection_is_visibly_reversed() {
        let theme = Theme::default();
        let line = NotesPanel::editor_line("abcd", Some(3), Some(1..3), &theme);
        let selected: String = line
            .spans
            .iter()
            .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(selected, "bc");
        assert!(line.spans.iter().any(|span| span.content == "▏"));
    }

    #[test]
    fn copy_and_paste_both_reach_the_border_at_the_default_width() {
        let line = crate::frame::hint_line(
            SELECTION_BINDINGS,
            &Theme::default(),
            28, // a 36-column default Notes panel reserves eight for its frame
        )
        .expect("the selected-text actions should fit");
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(text.contains("Ctrl+C copy · Ctrl+V paste"), "got {text:?}");
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

    /// Esc in the open search box abandons the search, as it does in the
    /// task filter and as the README says. It used to close the box and
    /// leave the half-typed term filtering the list behind it; the test above
    /// presses Enter first, so it never sent Esc to the open box at all.
    #[test]
    fn esc_in_the_search_box_abandons_the_search() {
        let (mut p, _g) = panel("search-esc");
        add_note(&mut p, "Groceries", "milk");
        add_note(&mut p, "Meeting", "invoice question");

        press(&mut p, KeyCode::Char('/'));
        type_str(&mut p, "invoice");
        assert_eq!(p.view.len(), 1, "the search applies as it is typed");
        press(&mut p, KeyCode::Esc);

        assert!(matches!(p.mode, Mode::List), "Esc closes the box");
        assert!(p.filter.is_empty(), "and drops the term: {:?}", p.filter);
        assert_eq!(p.view.len(), 2, "so every note is listed again");
    }

    /// The sentence for a search that matches nothing says when it has been
    /// cut (invariant 19).
    ///
    /// It was handed to a bare `Paragraph`, so the terminal did the cutting:
    /// "Nothing matches this search. Esc to clea" at 40 columns, with nothing
    /// to say a word was missing, and the instruction the sentence exists to
    /// give was the part lost. `no_panel_cuts_a_value_silently_at_any_width`
    /// never saw it, because the offline notes panel always has a note to
    /// show. The empty panel's own sentence is
    /// `an_empty_panels_message_is_whole_where_it_has_the_rows`; this is the
    /// same sweep over the other sentence the pane can say, with the search
    /// box open — width and height both, and both halves asserted: a
    /// sentence that fits its rows is drawn whole and carries no `…`.
    #[test]
    fn a_search_that_matches_nothing_says_when_its_sentence_has_been_cut() {
        let (mut p, _g) = panel("unmatched-cut");
        add_note(&mut p, "Groceries", "milk");
        press(&mut p, KeyCode::Char('/'));
        type_str(&mut p, "zzz");
        assert!(p.view.is_empty(), "the search matches nothing");

        let (mut cut, mut whole) = (0, 0);
        for height in 3..=10u16 {
            for width in 6..=60u16 {
                let rows = rows_of(&mut p, width, height);
                // Row 0 is the summary's, `search: zzz`, and the last is the
                // search box's; every row between them is the sentence's.
                let body = &rows[1..usize::from(height) - 1];
                let drawn: Vec<&str> = body
                    .iter()
                    .map(|row| row.trim_end())
                    .filter(|row| !row.is_empty())
                    .collect();
                let wrapped = crate::grid::wrap(NO_MATCH, usize::from(width));
                if wrapped.len() > body.len() {
                    cut += 1;
                    let last = drawn.last().copied().unwrap_or("");
                    assert!(
                        last.ends_with('…'),
                        "it needs {} rows and was cut to {} without saying so at \
                         {width}x{height}: {drawn:?}",
                        wrapped.len(),
                        body.len(),
                    );
                } else {
                    whole += 1;
                    let expected: Vec<&str> = wrapped.iter().map(|row| row.trim_end()).collect();
                    assert_eq!(drawn, expected, "it fits at {width}x{height}");
                }
            }
        }
        assert!(
            cut > 0 && whole > 0,
            "the sweep must reach a cut and a whole sentence: cut {cut}, whole {whole}"
        );
    }

    /// The title's caret is drawn where the next key lands. It was pushed
    /// after the visible text whatever the cursor was doing: once the title
    /// outgrew the field the text filled every cell and the caret was drawn
    /// past the edge, and after Home it sat at the end while typing went in
    /// at the start. The long case comes first because it is the one the old
    /// drawing fails; a caret merely pushed after the new window passes it,
    /// since the window there ends at the cursor, and fails the second.
    #[test]
    fn the_title_caret_is_drawn_where_typing_lands() {
        let (mut p, _g) = panel("title-caret");
        press(&mut p, KeyCode::Char('a'));
        // Thirty-six characters in a field of twenty-three cells, the cursor
        // ten from the end: after `z`, before `0`. The window keeps a cell
        // for the caret and gives the rest to what comes before it.
        type_str(&mut p, "abcdefghijklmnopqrstuvwxyz0123456789");
        for _ in 0..10 {
            press(&mut p, KeyCode::Left);
        }
        let rows = rows_of(&mut p, 30, 12);
        assert_eq!(rows[1], "title  efghijklmnopqrstuvwxyz▏", "{rows:?}");

        press(&mut p, KeyCode::Home);
        let rows = rows_of(&mut p, 60, 12);
        assert!(rows[1].starts_with("title  ▏abc"), "{rows:?}");
    }

    /// The search line's caret, likewise: it followed the whole term, so a
    /// term longer than the line lost it to the `…` that cut the term, and
    /// Home left it at the end.
    #[test]
    fn the_search_caret_is_drawn_where_typing_lands() {
        let (mut p, _g) = panel("search-caret");
        press(&mut p, KeyCode::Char('/'));
        // The cursor nine from the end: after `quarterly`.
        type_str(&mut p, "invoice for the quarterly accounts");
        for _ in 0..9 {
            press(&mut p, KeyCode::Left);
        }
        let rows = rows_of(&mut p, 30, 12);
        let last = rows.last().expect("a status row");
        assert_eq!(last, "search  ice for the quarterly▏", "{rows:?}");

        press(&mut p, KeyCode::Home);
        let rows = rows_of(&mut p, 60, 12);
        let last = rows.last().expect("a status row");
        assert!(last.starts_with("search  ▏invoice"), "{rows:?}");
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

    /// Every key in the map works at its default, and every one is
    /// advertised; the hints are derived from the map, so the check that
    /// can fail is the first — that the list answers every action.
    #[test]
    fn every_key_in_the_map_works_and_is_advertised() {
        let map = keymap(&KeysConfig::default()).expect("valid");
        let advertised: Vec<String> = map
            .bindings()
            .iter()
            .flat_map(|b| b.key.split(" / ").map(str::to_string).collect::<Vec<_>>())
            .collect();
        for meta in ACTIONS {
            for &(code, modifiers) in meta.defaults {
                let key = crate::keymap::Key::new(code, modifiers);
                assert!(
                    advertised.contains(&key.to_string()),
                    "`{key}` ({}) is handled but not advertised: {advertised:?}",
                    meta.name
                );
                let (mut p, _g) = panel("keymap");
                add_note(&mut p, "a note", "body");
                assert!(matches!(p.mode, Mode::List));
                assert_eq!(
                    p.handle_key(KeyEvent::new(code, modifiers)),
                    KeyOutcome::Consumed,
                    "`{key}` ({}) is in the map but the list ignores it",
                    meta.name
                );
            }
        }
    }

    /// A moved key takes the action with it, the border follows, and Esc —
    /// never in the map — still clears a search.
    #[test]
    fn a_moved_list_key_works_and_the_old_one_does_not() {
        let (mut p, _g) = panel("moved");
        add_note(&mut p, "a note", "body");
        p.keys = keymap(&toml::from_str("new = \"+\"").expect("a table")).expect("valid");
        assert_eq!(
            p.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            KeyOutcome::Ignored
        );
        assert!(
            p.bindings()
                .iter()
                .any(|b| b.primary && b.key == "+" && b.action == "new")
        );
        press(&mut p, KeyCode::Char('+'));
        assert!(matches!(p.mode, Mode::Edit(_)), "+ writes a new note");
        press(&mut p, KeyCode::Esc);
        p.filter = "note".into();
        press(&mut p, KeyCode::Esc);
        assert!(p.filter.is_empty(), "{:?}", p.filter);
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

    /// An edited note's date carries a `·` saying which date it is, and the
    /// column was sized to the date alone: at the shipped `%d %b` the mark
    /// pushed it a cell over, and every edited note read `·25 J…`. Swept from
    /// the narrowest width that still shows the column.
    #[test]
    fn an_edited_notes_date_is_drawn_whole_beside_its_mark() {
        let (mut p, _g) = panel("edited-date");
        add_note(&mut p, "Release", "Bump the version.");
        press(&mut p, KeyCode::Enter);
        type_str(&mut p, "s");
        chord(&mut p, KeyCode::Char('s'), KeyModifiers::CONTROL);
        let note = &p.store.notes()[0];
        assert!(note.updated.is_some(), "the edit was recorded");
        let date = format!("·{}", note.shown_date().strftime(&p.config.date_format));
        for width in 27..=80u16 {
            let rows = rows_of(&mut p, width, 16);
            let row = rows
                .iter()
                .find(|row| row.contains("Releases"))
                .unwrap_or_else(|| panic!("{width}: {rows:#?}"));
            assert!(
                row.trim_end().ends_with(&date) && !row.contains('…'),
                "{width}: {row:?} should end in {date:?}"
            );
        }
    }

    /// The detail pane gives the title one row. It was handed the title
    /// *wrapped*, so the row showed the first line of a long title and the
    /// rest went with nothing to say it had: a cut along the height, which
    /// the width sweep cannot see. A short title must come through bare, so
    /// an ellipsis stuck on unconditionally fails too.
    #[test]
    fn a_long_title_in_the_detail_pane_ends_in_an_ellipsis() {
        for (title, name) in [
            (
                "Bump the version and run the four gates before tagging",
                "long",
            ),
            ("Groceries", "short"),
        ] {
            let (mut p, _g) = panel(&format!("detail-title-{name}"));
            add_note(&mut p, title, "body text");
            for width in 12..=70u16 {
                let rows = rows_of(&mut p, width, 20);
                let rule = rows
                    .iter()
                    .position(|row| !row.trim().is_empty() && row.trim().chars().all(|c| c == '─'))
                    .unwrap_or_else(|| panic!("{width}: no rule: {rows:#?}"));
                let shown = rows[rule + 1].trim_end();
                if crate::grid::display_width(title) <= usize::from(width) {
                    assert_eq!(shown, title, "{width}: fits, so whole");
                } else {
                    assert!(
                        shown.ends_with('…'),
                        "{width}: {shown:?} is cut and must say so"
                    );
                }
            }
        }
    }

    /// The form's last row carries an error, a status or the keys, and was
    /// drawn as one bare span: `a note needs a ` at fifteen columns, with
    /// nothing saying the rest had gone. A message is prose and is
    /// ellipsised; the keys drop whole.
    #[test]
    fn the_form_footer_is_whole_or_says_it_was_cut() {
        let (mut p, _g) = panel("form-footer");
        press(&mut p, KeyCode::Char('a'));
        chord(&mut p, KeyCode::Char('s'), KeyModifiers::CONTROL);
        let message = "a note needs a title";
        for width in 4..=60u16 {
            let rows = rows_of(&mut p, width, 10);
            let footer = rows[9].trim_end();
            assert!(
                footer == message || footer.ends_with('…'),
                "{width}: {footer:?}"
            );
        }

        // The keys: each is whole or absent, never half a hint. A fresh form,
        // since the error above stays until the form is left.
        press(&mut p, KeyCode::Esc);
        press(&mut p, KeyCode::Char('a'));
        press(&mut p, KeyCode::Tab);
        let whole = [
            "",
            "Shift+arrows select",
            "Shift+arrows select   Ctrl+A all",
            "Shift+arrows select   Ctrl+A all   Ctrl+V paste",
        ];
        for width in 4..=60u16 {
            let rows = rows_of(&mut p, width, 10);
            let footer = rows[9].trim_end();
            assert!(
                whole.contains(&footer) || footer.ends_with('…'),
                "{width}: {footer:?}"
            );
        }

        // On the title the keys end in the way out, and it is the last to
        // go, as in a prompt's help: it dropped first, so a footer 22 to 34
        // cells wide said how to save and not how to leave.
        press(&mut p, KeyCode::BackTab);
        let whole = [
            "Tab body   Ctrl+S save   Esc cancel",
            "Tab body   Esc cancel",
            "Esc cancel",
        ];
        let mut dropped = 0;
        for width in 4..=60u16 {
            let rows = rows_of(&mut p, width, 10);
            let footer = rows[9].trim_end();
            if footer.is_empty() {
                continue;
            }
            assert!(
                whole.contains(&footer) || "Esc cancel".starts_with(footer.trim_end_matches('…')),
                "{width}: {footer:?}"
            );
            if footer != whole[0] {
                dropped += 1;
            }
        }
        assert!(dropped > 0, "the sweep reached a width that drops a part");
    }

    /// The summary row says what is being searched for, and a search term is
    /// as long as the reader made it. It was drawn at its natural width, so
    /// the terminal cut the term wherever the edge fell.
    #[test]
    fn a_long_search_term_in_the_summary_ends_in_an_ellipsis() {
        let (mut p, _g) = panel("search-summary");
        add_note(&mut p, "Groceries", "milk and an invoice question");
        press(&mut p, KeyCode::Char('/'));
        type_str(&mut p, "an invoice question");
        let whole = "search: an invoice question";
        for width in 4..=60u16 {
            let rows = rows_of(&mut p, width, 12);
            let summary = rows[0].trim_end();
            assert!(
                summary == whole || summary.ends_with('…'),
                "{width}: {summary:?}"
            );
        }
    }

    /// Invariant 19 along the height. The reader scrolls, but nothing on
    /// screen said there was anything to scroll to: the rows that fitted were
    /// drawn and the rest dropped in silence, so the default dashboard showed
    /// the seeded note as `You are reading it in the` and stopped. That one
    /// at least breaks off mid-sentence; a cut that falls between sentences
    /// looks like the end of the note. The last row drawn now ends in `…`
    /// while there is more below it.
    ///
    /// Swept over the height with the width pinned, and both halves asserted:
    /// a body that fits is drawn whole with no `…`, so an ellipsis stuck on
    /// unconditionally fails too.
    #[test]
    fn a_body_longer_than_the_reader_ends_in_an_ellipsis() {
        const BODY: &str = "You are reading it in the detail pane. That is the point \
                            of the panel: a note's whole value is the text inside it. \
                            The rest of it is here to be cut.";
        const WIDTH: u16 = 40;
        let (mut p, _dir) = panel("body-cut");
        add_note(&mut p, "Long", BODY);

        let (mut cut, mut whole) = (0, 0);
        for height in 4..=40u16 {
            p.body_area = None;
            let screen = rows_of(&mut p, WIDTH, height);
            let Some(area) = p.body_area else { continue };
            let row = |y: u16| -> String {
                let text: String = screen[usize::from(y)]
                    .chars()
                    .skip(usize::from(area.x))
                    .take(usize::from(area.width))
                    .collect();
                text.trim_end().to_string()
            };
            let wrapped = crate::grid::wrap(BODY, usize::from(area.width));
            let rows = usize::from(area.height);
            let shown: Vec<String> = (area.y..area.bottom()).map(row).collect();
            let all = screen.join("\n");
            if wrapped.len() > rows {
                cut += 1;
                let (last, above) = shown.split_last().expect("a row at least");
                for (drawn, wrapped) in above.iter().zip(&wrapped) {
                    assert_eq!(drawn, wrapped.trim_end(), "at height {height}:\n{all}");
                }
                let kept = last.strip_suffix('…').unwrap_or_else(|| {
                    panic!(
                        "a body needing {} rows was cut to {rows} without saying so \
                         at height {height}:\n{all}",
                        wrapped.len()
                    )
                });
                assert!(
                    wrapped[rows - 1].starts_with(kept),
                    "at height {height}:\n{all}"
                );
            } else {
                whole += 1;
                for (i, drawn) in shown.iter().enumerate() {
                    let expected = wrapped.get(i).map_or("", |line| line.trim_end());
                    assert_eq!(drawn, expected, "at height {height}:\n{all}");
                }
            }
        }
        assert!(
            cut > 0 && whole > 0,
            "the sweep must reach both a cut and a whole body: cut {cut}, whole {whole}"
        );
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

    /// An empty panel says how to write the first note, with the key
    /// `[notes.keys]` gave `new` — it said `a` whatever the table said — and
    /// without the offer when `new` is unbound.
    #[test]
    fn an_empty_panel_names_the_new_key_it_has() {
        let text = |keys: &str| {
            let (mut p, _g) = panel("empty-keys");
            let config: crate::config::Config =
                toml::from_str(&format!("[notes.keys]\n{keys}")).expect("a config");
            p.set_keys(&config);
            rows_of(&mut p, 120, 12).join("\n")
        };
        let moved = text("new = \"c\"");
        assert!(moved.contains("Press `c` to write one."), "{moved}");
        assert!(!moved.contains("`a`"), "{moved}");

        let unbound = text("new = []");
        assert!(unbound.contains("No notes yet."), "{unbound}");
        assert!(!unbound.contains("Press"), "{unbound}");

        assert!(text("").contains("No notes yet. Press `a` to write one."));
    }

    /// Invariant 19 for the empty panel: its message arrives whole wherever
    /// the panel has the rows for it, and otherwise its last row ends in `…`.
    ///
    /// It went to the terminal at its natural width, which cut it where the
    /// pane ended and said nothing. One row ending in `…` would be honest and
    /// still wrong, because the pane is several rows tall: at 20x10 that reads
    /// `No notes yet. Press…` over blank rows, the key it exists to name gone.
    /// Swept over height as well as width, since a cut along the height is
    /// the one a width sweep cannot see.
    ///
    /// The room is every row above the status line, not the note pane's.
    /// With no note to show, the list half above the rule was left blank and
    /// the message was cut in the pane under it: `No notes yet. Press…` at
    /// 20x5 under a blank row, and at a height of 4 the pane had no rows at
    /// all and the offer was gone without an `…`. A sweep that measured the
    /// pane, and skipped it where it had no rows, approved every one of
    /// those. One that started under row 0 approved the next: `no notes`
    /// there, over `No notes yet`, said the count twice and cut the key at
    /// 30x3.
    #[test]
    fn an_empty_panels_message_is_whole_where_it_has_the_rows() {
        let whole = "No notes yet. Press `a` to write one.";
        // The longest word; narrower, a word is broken and the rows cannot
        // be joined back into the sentence.
        let longest = 5;
        let (mut p, _g) = panel("empty-cut");
        let (mut cut, mut wrapped) = (0, 0);
        for height in 2..=16u16 {
            for width in 4..=60u16 {
                let rows = rows_of(&mut p, width, height);
                // The last row is the status line's; the panel has nothing
                // else to show. Row 0 included: a summary there saying `no
                // notes` over `No notes yet` took a row the key needed.
                let body = &rows[..usize::from(height) - 1];
                let lines: Vec<&str> = body
                    .iter()
                    .map(|row| row.trim_end())
                    .take_while(|row| !row.is_empty())
                    .collect();
                let said = lines.join(" ");
                let room = body.len();
                let needed = usize::from(crate::grid::wrapped_height(whole, width));
                if needed > room {
                    cut += 1;
                    assert!(said.ends_with('…'), "at {width}x{height}: {rows:#?}");
                    assert_eq!(lines.len(), room, "at {width}x{height}: {rows:#?}");
                } else if width >= longest {
                    assert_eq!(said, whole, "at {width}x{height}: {rows:#?}");
                    wrapped += usize::from(needed > 1);
                }
            }
        }
        assert!(cut > 0, "the sweep reached a size that cuts the message");
        assert!(wrapped > 0, "the sweep reached a size that wraps it");
    }
}
