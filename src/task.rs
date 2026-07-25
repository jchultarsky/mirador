//! The task model and its on-disk store.
//!
//! Tasks live in a single human-editable TOML file:
//!
//! ```toml
//! [[task]]
//! id = 1
//! title = "Publish 0.0.0 placeholder"
//! notes = "Reserve the crates.io name"
//! due = "2026-07-28"
//! priority = "high"
//! tags = ["mirador"]
//! done = false
//! created = "2026-07-25"
//! ```
//!
//! Writes are atomic (write to a sibling temp file, then rename) so an
//! interrupted save can never truncate the file.

use std::cmp::Ordering;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// Task urgency. Declaration order is sort order: `High` sorts first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    High,
    Medium,
    #[default]
    Low,
    /// Explicitly unprioritised; sorts last.
    None,
}

impl Priority {
    /// All priorities, in sort order. Used for cycling in the editor.
    pub const ALL: [Self; 4] = [Self::High, Self::Medium, Self::Low, Self::None];

    /// The next priority in the cycle, wrapping around.
    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// The previous priority in the cycle, wrapping around.
    pub fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::None => "none",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for Priority {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "high" | "h" => Ok(Self::High),
            "medium" | "med" | "m" => Ok(Self::Medium),
            "low" | "l" => Ok(Self::Low),
            "none" | "n" | "" => Ok(Self::None),
            other => anyhow::bail!("`{other}` is not a priority (high, medium, low, none)"),
        }
    }
}

/// How overdue or imminent a task is, relative to today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueState {
    /// Past its due date and not done.
    Overdue(i32),
    /// Due today.
    Today,
    /// Due within the next week.
    Soon(i32),
    /// Due further out.
    Later(i32),
    /// No due date set.
    None,
}

/// A single to-do item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Stable identifier, unique within the file.
    pub id: u64,
    /// One-line summary.
    pub title: String,
    /// Optional longer body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Optional due date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    /// Urgency.
    #[serde(default)]
    pub priority: Priority,
    /// Free-form labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Completion flag.
    #[serde(default)]
    pub done: bool,
    /// When it was completed, if it has been.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<Date>,
    /// When it was created.
    pub created: Date,
}

impl Task {
    /// Create a task with today's creation date.
    pub fn new(id: u64, title: impl Into<String>, today: Date) -> Self {
        Self {
            id,
            title: title.into(),
            notes: None,
            due: None,
            priority: Priority::default(),
            tags: Vec::new(),
            done: false,
            completed: None,
            created: today,
        }
    }

    /// Classify the due date relative to `today`.
    pub fn due_state(&self, today: Date) -> DueState {
        let Some(due) = self.due else {
            return DueState::None;
        };
        let days = days_between(today, due);
        match days {
            d if d < 0 => DueState::Overdue(-d),
            0 => DueState::Today,
            d if d <= 7 => DueState::Soon(d),
            d => DueState::Later(d),
        }
    }

    /// Toggle completion, stamping or clearing the completion date.
    pub fn toggle_done(&mut self, today: Date) {
        self.done = !self.done;
        self.completed = if self.done { Some(today) } else { None };
    }

    /// True if any tag matches `needle` case-insensitively, or if the title or
    /// notes contain it. Used by the panel's filter box.
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let needle = needle.to_ascii_lowercase();
        self.title.to_ascii_lowercase().contains(&needle)
            || self
                .tags
                .iter()
                .any(|t| t.to_ascii_lowercase().contains(&needle))
            || self
                .notes
                .as_ref()
                .is_some_and(|n| n.to_ascii_lowercase().contains(&needle))
    }
}

/// Whole-day difference `to - from`. Positive means `to` is in the future.
pub fn days_between(from: Date, to: Date) -> i32 {
    // Ask for the difference in days specifically. The default unit for
    // `since` is years, which would give a mixed span we would have to
    // reassemble with a made-up month length.
    to.since((jiff::Unit::Day, from))
        .map_or(0, |span| span.get_days())
}

/// How the list is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    /// Incomplete first, then overdue, then by priority, then by due date.
    #[default]
    Smart,
    /// By due date, undated last.
    Due,
    /// By priority.
    Priority,
    /// Newest first.
    Created,
    /// Alphabetical.
    Title,
}

impl SortMode {
    /// The next mode in the cycle.
    pub fn next(self) -> Self {
        match self {
            Self::Smart => Self::Due,
            Self::Due => Self::Priority,
            Self::Priority => Self::Created,
            Self::Created => Self::Title,
            Self::Title => Self::Smart,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Smart => "smart",
            Self::Due => "due",
            Self::Priority => "priority",
            Self::Created => "created",
            Self::Title => "title",
        }
    }
}

impl std::str::FromStr for SortMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "smart" => Ok(Self::Smart),
            "due" => Ok(Self::Due),
            "priority" => Ok(Self::Priority),
            "created" => Ok(Self::Created),
            "title" => Ok(Self::Title),
            other => {
                anyhow::bail!("`{other}` is not a sort mode (smart, due, priority, created, title)")
            }
        }
    }
}

/// Serialisation wrapper so the file reads as a list of `[[task]]` tables.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TaskFile {
    #[serde(default, rename = "task")]
    tasks: Vec<Task>,
}

/// An owned, persisted collection of tasks.
#[derive(Debug)]
pub struct TaskStore {
    path: PathBuf,
    tasks: Vec<Task>,
    /// Set when the in-memory list has changes not yet written to disk.
    dirty: bool,
    /// High-water mark for ids, which only ever climbs.
    ///
    /// Deriving the next id from `max(id) + 1` hands a deleted task's id
    /// straight to the next one added, so anything still holding the old id —
    /// the selection, an open edit form, a pending delete confirmation — would
    /// silently act on a different task. Rebuilt from the file on load, which
    /// is safe because nothing holds an id across a restart.
    next_id: u64,
    /// The last save error, surfaced in the panel so failures are never silent.
    pub last_error: Option<String>,
}

impl TaskStore {
    /// Load from `path`, treating a missing file as an empty list.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let tasks = if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("reading tasks from {}", path.display()))?;
            let parsed: TaskFile = toml::from_str(&raw)
                .with_context(|| format!("parsing tasks in {}", path.display()))?;
            parsed.tasks
        } else {
            Vec::new()
        };

        let next_id = tasks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        Ok(Self {
            path,
            tasks,
            dirty: false,
            next_id,
            last_error: None,
        })
    }

    /// The file this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// All tasks, in file order.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// Append a task and return its id.
    pub fn add(&mut self, mut task: Task) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        task.id = id;
        self.tasks.push(task);
        self.dirty = true;
        id
    }

    /// Replace the task with the same id. Returns false if no such task exists.
    pub fn update(&mut self, task: Task) -> bool {
        let Some(slot) = self.tasks.iter_mut().find(|t| t.id == task.id) else {
            return false;
        };
        *slot = task;
        self.dirty = true;
        true
    }

    /// Remove a task by id. Returns false if no such task exists.
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.tasks.len();
        self.tasks.retain(|t| t.id != id);
        let removed = self.tasks.len() != before;
        self.dirty |= removed;
        removed
    }

    /// Look up a task by id.
    pub fn get(&self, id: u64) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// Mutate a task in place by id.
    pub fn with_task<F: FnOnce(&mut Task)>(&mut self, id: u64, f: F) -> bool {
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        f(task);
        self.dirty = true;
        true
    }

    /// Ids in display order for the given options.
    pub fn view(
        &self,
        sort: SortMode,
        show_completed: bool,
        filter: &str,
        today: Date,
    ) -> Vec<u64> {
        let mut visible: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|t| show_completed || !t.done)
            .filter(|t| t.matches(filter))
            .collect();

        visible.sort_by(|a, b| compare(a, b, sort, today));
        visible.iter().map(|t| t.id).collect()
    }

    /// Every distinct tag, sorted, for the filter hint.
    pub fn all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .tasks
            .iter()
            .flat_map(|t| t.tags.iter().cloned())
            .collect();
        tags.sort_unstable();
        tags.dedup();
        tags
    }

    /// Write to disk atomically if there are pending changes.
    ///
    /// The temp file is created in the same directory as the target so the
    /// rename stays on one filesystem and is therefore atomic.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating data directory {}", parent.display()))?;
        }

        let file = TaskFile {
            tasks: self.tasks.clone(),
        };
        let body = toml::to_string_pretty(&file).context("serialising tasks")?;
        let contents = format!(
            "# mirador tasks. Safe to edit by hand or keep in version control.\n\
             # Fields: id, title, notes, due (YYYY-MM-DD), priority \
             (high|medium|low|none), tags, done, completed, created.\n\n{body}"
        );

        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, &contents).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("replacing {} with {}", self.path.display(), tmp.display()))?;

        self.dirty = false;
        Ok(())
    }

    /// Save, recording any failure in [`TaskStore::last_error`] rather than
    /// propagating it. The panel renders that message, so a read-only disk
    /// shows up in the UI instead of vanishing.
    pub fn save_reporting(&mut self) {
        match self.save() {
            Ok(()) => self.last_error = None,
            Err(e) => self.last_error = Some(format!("{e:#}")),
        }
    }
}

/// Ordering for two tasks under a given sort mode.
fn compare(a: &Task, b: &Task, sort: SortMode, today: Date) -> Ordering {
    match sort {
        SortMode::Smart => a
            .done
            .cmp(&b.done)
            .then_with(|| urgency_rank(a, today).cmp(&urgency_rank(b, today)))
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| cmp_due(a, b))
            .then_with(|| a.id.cmp(&b.id)),
        SortMode::Due => a
            .done
            .cmp(&b.done)
            .then_with(|| cmp_due(a, b))
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| a.id.cmp(&b.id)),
        SortMode::Priority => a
            .done
            .cmp(&b.done)
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| cmp_due(a, b))
            .then_with(|| a.id.cmp(&b.id)),
        SortMode::Created => b.created.cmp(&a.created).then_with(|| b.id.cmp(&a.id)),
        SortMode::Title => a
            .title
            .to_ascii_lowercase()
            .cmp(&b.title.to_ascii_lowercase())
            .then_with(|| a.id.cmp(&b.id)),
    }
}

/// Lower is more urgent. Undated tasks rank after everything dated.
fn urgency_rank(task: &Task, today: Date) -> u8 {
    match task.due_state(today) {
        DueState::Overdue(_) => 0,
        DueState::Today => 1,
        DueState::Soon(_) => 2,
        DueState::Later(_) => 3,
        DueState::None => 4,
    }
}

/// Order by due date with undated tasks last.
fn cmp_due(a: &Task, b: &Task) -> Ordering {
    match (a.due, b.due) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn today() -> Date {
        date(2026, 7, 25)
    }

    fn task(id: u64, title: &str) -> Task {
        Task::new(id, title, today())
    }

    #[test]
    fn priority_cycles_forward_and_back() {
        assert_eq!(Priority::High.next(), Priority::Medium);
        assert_eq!(Priority::None.next(), Priority::High);
        assert_eq!(Priority::High.prev(), Priority::None);
        for p in Priority::ALL {
            assert_eq!(p.next().prev(), p, "next/prev must be inverses for {p}");
        }
    }

    #[test]
    fn priority_sorts_high_first() {
        let mut ps = vec![
            Priority::None,
            Priority::Low,
            Priority::High,
            Priority::Medium,
        ];
        ps.sort_unstable();
        assert_eq!(
            ps,
            vec![
                Priority::High,
                Priority::Medium,
                Priority::Low,
                Priority::None
            ]
        );
    }

    #[test]
    fn due_state_classifies_relative_to_today() {
        let mut t = task(1, "x");
        assert_eq!(t.due_state(today()), DueState::None);

        t.due = Some(date(2026, 7, 25));
        assert_eq!(t.due_state(today()), DueState::Today);

        t.due = Some(date(2026, 7, 22));
        assert_eq!(t.due_state(today()), DueState::Overdue(3));

        t.due = Some(date(2026, 7, 28));
        assert_eq!(t.due_state(today()), DueState::Soon(3));

        t.due = Some(date(2026, 9, 1));
        assert!(matches!(t.due_state(today()), DueState::Later(_)));
    }

    #[test]
    fn days_between_spans_month_and_year_boundaries() {
        assert_eq!(days_between(date(2026, 7, 25), date(2026, 8, 1)), 7);
        assert_eq!(days_between(date(2026, 12, 31), date(2027, 1, 1)), 1);
        assert_eq!(days_between(date(2026, 8, 1), date(2026, 7, 25)), -7);
        assert_eq!(days_between(date(2026, 7, 25), date(2026, 7, 25)), 0);
    }

    #[test]
    fn toggle_done_stamps_and_clears_completion() {
        let mut t = task(1, "x");
        t.toggle_done(today());
        assert!(t.done);
        assert_eq!(t.completed, Some(today()));
        t.toggle_done(today());
        assert!(!t.done);
        assert_eq!(t.completed, None);
    }

    #[test]
    fn ids_are_unique_and_survive_deletion() {
        let dir = tempdir();
        let mut store = TaskStore::load(dir.join("todos.toml")).unwrap();
        let a = store.add(task(0, "a"));
        let b = store.add(task(0, "b"));
        assert_ne!(a, b);
        assert!(store.remove(b));
        let c = store.add(task(0, "c"));
        assert_ne!(c, a, "a reused id would rewrite the wrong task");
        // The one that actually mattered: `b` was the highest id, so deriving
        // the next id from `max(id) + 1` handed `b` straight back. Anything
        // still holding it — the selection, an open edit form, a pending
        // delete — would then act on `c` instead.
        assert_ne!(c, b, "the removed task's id must not be handed out again");
    }

    #[test]
    fn the_id_mark_only_climbs_across_many_add_and_remove_cycles() {
        let dir = tempdir();
        let mut store = TaskStore::load(dir.join("todos.toml")).unwrap();
        let mut seen = std::collections::HashSet::new();

        // Repeatedly add and immediately remove: every cycle leaves the store
        // empty, which is the case that reset `max(id)` to nothing.
        for _ in 0..20 {
            let id = store.add(task(0, "churn"));
            assert!(seen.insert(id), "id {id} was handed out twice");
            assert!(store.remove(id));
        }
        assert!(store.tasks().is_empty());
    }

    #[test]
    fn ids_resume_above_the_file_after_a_reload() {
        let dir = tempdir();
        let path = dir.join("todos.toml");
        let mut store = TaskStore::load(&path).unwrap();
        let a = store.add(task(0, "a"));
        let b = store.add(task(0, "b"));
        store.save().unwrap();

        // Reloading rebuilds the mark from the file. Safe precisely because
        // nothing holds an id across a restart.
        let mut reloaded = TaskStore::load(&path).unwrap();
        let c = reloaded.add(task(0, "c"));
        assert!(c > a && c > b, "{c} must be above everything in the file");
    }

    #[test]
    fn smart_sort_puts_overdue_before_high_priority_future_work() {
        let dir = tempdir();
        let mut store = TaskStore::load(dir.join("todos.toml")).unwrap();

        let mut overdue = task(0, "overdue but low");
        overdue.due = Some(date(2026, 7, 20));
        overdue.priority = Priority::Low;
        let overdue_id = store.add(overdue);

        let mut future = task(0, "high but far off");
        future.due = Some(date(2026, 12, 1));
        future.priority = Priority::High;
        store.add(future);

        let view = store.view(SortMode::Smart, false, "", today());
        assert_eq!(view[0], overdue_id);
    }

    #[test]
    fn completed_tasks_are_hidden_unless_requested() {
        let dir = tempdir();
        let mut store = TaskStore::load(dir.join("todos.toml")).unwrap();
        let id = store.add(task(0, "done thing"));
        store.with_task(id, |t| t.toggle_done(today()));

        assert!(store.view(SortMode::Smart, false, "", today()).is_empty());
        assert_eq!(store.view(SortMode::Smart, true, "", today()).len(), 1);
    }

    #[test]
    fn filter_matches_title_tags_and_notes() {
        let dir = tempdir();
        let mut store = TaskStore::load(dir.join("todos.toml")).unwrap();

        let mut a = task(0, "Buy milk");
        a.tags = vec!["errand".into()];
        store.add(a);

        let mut b = task(0, "Write docs");
        b.notes = Some("about the errand system".into());
        store.add(b);

        assert_eq!(
            store.view(SortMode::Smart, false, "errand", today()).len(),
            2
        );
        assert_eq!(store.view(SortMode::Smart, false, "milk", today()).len(), 1);
        assert_eq!(store.view(SortMode::Smart, false, "zzz", today()).len(), 0);
        assert_eq!(store.view(SortMode::Smart, false, "", today()).len(), 2);
    }

    #[test]
    fn round_trips_through_disk_preserving_every_field() {
        let dir = tempdir();
        let path = dir.join("todos.toml");

        let mut store = TaskStore::load(&path).unwrap();
        let mut t = task(0, "Publish placeholder");
        t.notes = Some("Reserve the crates.io name".into());
        t.due = Some(date(2026, 7, 28));
        t.priority = Priority::High;
        t.tags = vec!["mirador".into(), "rust".into()];
        let id = store.add(t);
        store.save().unwrap();

        let reloaded = TaskStore::load(&path).unwrap();
        let got = reloaded.get(id).expect("task survives the round trip");
        assert_eq!(got.title, "Publish placeholder");
        assert_eq!(got.notes.as_deref(), Some("Reserve the crates.io name"));
        assert_eq!(got.due, Some(date(2026, 7, 28)));
        assert_eq!(got.priority, Priority::High);
        assert_eq!(got.tags, vec!["mirador".to_string(), "rust".to_string()]);
        assert!(!got.done);
    }

    #[test]
    fn missing_file_loads_as_empty_rather_than_failing() {
        let dir = tempdir();
        let store = TaskStore::load(dir.join("does-not-exist.toml")).unwrap();
        assert!(store.tasks().is_empty());
    }

    #[test]
    fn save_is_a_no_op_when_not_dirty() {
        let dir = tempdir();
        let path = dir.join("todos.toml");
        let mut store = TaskStore::load(&path).unwrap();
        store.save().unwrap();
        assert!(!path.exists(), "a clean store must not create a file");
    }

    #[test]
    fn parse_errors_name_the_offending_file() {
        let dir = tempdir();
        let path = dir.join("broken.toml");
        std::fs::write(&path, "[[task]]\nthis is not toml =").unwrap();
        let err = TaskStore::load(&path).expect_err("must fail");
        assert!(format!("{err:#}").contains("broken.toml"));
    }

    #[test]
    fn priority_parses_from_shorthand() {
        assert_eq!("H".parse::<Priority>().unwrap(), Priority::High);
        assert_eq!("  medium ".parse::<Priority>().unwrap(), Priority::Medium);
        assert_eq!("".parse::<Priority>().unwrap(), Priority::None);
        assert!("urgent".parse::<Priority>().is_err());
    }

    /// A unique scratch directory, removed when the returned guard drops.
    fn tempdir() -> TempDir {
        let base = std::env::temp_dir().join(format!(
            "mirador-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&base).unwrap();
        TempDir(base)
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
