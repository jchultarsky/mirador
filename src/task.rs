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

use anyhow::Result;
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

    /// True if the title, a tag or the notes contain `needle`, ignoring case.
    /// Used by the panel's filter box.
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let needle = Needle::new(needle);
        needle.is_in(&self.title)
            || self.tags.iter().any(|t| needle.is_in(t))
            || self.notes.as_deref().is_some_and(|n| needle.is_in(n))
    }
}

/// Text searched for without regard to case, by the task filter and by the
/// notes search alike.
///
/// One type so the two cannot disagree about what "contains" means. They
/// did: the filter folded only `A`–`Z` while the notes search folded every
/// letter, so `übung` found a note called `Übung` and not a task.
pub(crate) struct Needle(String);

impl Needle {
    pub(crate) fn new(text: &str) -> Self {
        Self(fold(text))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `haystack` contains this, case folded on both sides the same
    /// way.
    pub(crate) fn is_in(&self, haystack: &str) -> bool {
        fold(haystack).contains(&self.0)
    }
}

/// Lower case a letter at a time, with the final sigma `ς` read as `σ`.
///
/// Not `str::to_lowercase`, which reads context: a capital sigma ending a word
/// becomes `ς` and one inside a word `σ`. A needle is a word cut short, so
/// `ΚΩΣ` typed on the way to `ΚΩΣΤΑΣ` folded to `κως` against `κωστας` and
/// stopped matching. Each letter on its own folds the same on both sides, and
/// the two sigmas are one letter for the purpose of finding a word.
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c == 'ς' { 'σ' } else { c })
        .collect()
}

/// Whole-day difference `to - from`. Positive means `to` is in the future.
pub fn days_between(from: Date, to: Date) -> i32 {
    // Ask for the difference in days specifically. The default unit for
    // `since` is years, which would give a mixed span we would have to
    // reassemble with a made-up month length.
    to.since((jiff::Unit::Day, from))
        .map_or(0, |span| span.get_days())
}

/// The last day a horizon of `days` shows, or `None` when it shows
/// everything: at 0 by definition, and at a figure that runs past the end of
/// the calendar, since no due date can lie beyond that. Fallible arithmetic
/// throughout, because the figure comes straight from a config and
/// `Span::days` panics on one that `u32` holds easily.
fn horizon_end(today: Date, days: u32) -> Option<Date> {
    if days == 0 {
        return None;
    }
    let span = jiff::Span::new().try_days(i64::from(days)).ok()?;
    today.checked_add(span).ok()
}

/// Whether a task due on `due` is hidden by a horizon of `days` from `today`.
pub fn beyond_horizon(due: Option<Date>, today: Date, days: u32) -> bool {
    past(horizon_end(today, days), due)
}

fn past(last_day: Option<Date>, due: Option<Date>) -> bool {
    last_day.zip(due).is_some_and(|(last, due)| due > last)
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
    /// Every mode, in the order `s` cycles through them. Parsing reads its
    /// words from here, so the list a refused `[todo].sort` quotes back is the
    /// list the parser accepts.
    pub const ALL: [Self; 5] = [
        Self::Smart,
        Self::Due,
        Self::Priority,
        Self::Created,
        Self::Title,
    ];

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
        let wanted = s.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|mode| mode.label() == wanted)
            .ok_or_else(|| {
                let words = Self::ALL.map(Self::label).join(", ");
                anyhow::anyhow!("`{wanted}` is not a sort mode ({words})")
            })
    }
}

/// Serialisation wrapper so the file reads as a list of `[[task]]` tables.
///
/// Owned when it is read and borrowed when it is written, so the one
/// `rename` serves both directions and a save serialises the list where it
/// stands rather than a copy of it.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TaskFile<L = Vec<Task>> {
    #[serde(default, rename = "task")]
    tasks: L,
}

/// The task file's header, and what its errors call it.
const FILE: crate::store::TomlFile = crate::store::TomlFile {
    what: "tasks",
    header: "# mirador tasks. Safe to edit by hand or keep in version control.\n\
             # Fields: id, title, notes, due (YYYY-MM-DD), priority \
             (high|medium|low|none), tags, done, completed, created.",
};

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
        let mut tasks = FILE
            .read::<TaskFile>(&path)?
            .map(|file| file.tasks)
            .unwrap_or_default();

        let mut next_id = tasks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        let dirty = renumber_repeated_ids(tasks.iter_mut().map(|t| &mut t.id), &mut next_id);
        Ok(Self {
            path,
            tasks,
            dirty,
            next_id,
            last_error: None,
        })
    }

    /// Load, seeding a handful of example tasks when there is no file yet.
    ///
    /// An empty list on first run is the worst version of this panel: the one
    /// thing it exists to show is missing, and every key that would fill it is
    /// invisible until you press `?`. The examples are ordinary tasks — they
    /// carry the due dates, priorities, tags and notes a real one would, so the
    /// columns have something to line up — and deleting them is the point.
    ///
    /// Seeded only when the file is *absent*, never when it is present and
    /// empty. Clearing the list writes an empty file, so a user who deletes all
    /// of these does not meet them again on the next run.
    pub fn load_or_seed(path: impl Into<PathBuf>, today: Date) -> Result<Self> {
        let path = path.into();
        let first_run = !path.exists();
        let mut store = Self::load(path)?;
        if first_run {
            for task in example_tasks(today) {
                store.add(task);
            }
            // Reported rather than propagated: a seeding failure is not worth
            // refusing to start over, and `last_error` puts it in the panel.
            store.save_reporting();
        }
        Ok(store)
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
    ///
    /// `horizon_days` hides a task due more than that many days after
    /// `today`, and 0 hides nothing. A task with no due date, or one already
    /// late, is never beyond it. The horizon is measured from `today` on every
    /// call rather than fixed when the panel was built, so the rebuild at
    /// midnight brings a task into view on the day it comes into range.
    pub fn view(
        &self,
        sort: SortMode,
        show_completed: bool,
        filter: &str,
        horizon_days: u32,
        today: Date,
    ) -> Vec<u64> {
        let last_day = horizon_end(today, horizon_days);
        let mut visible: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|t| show_completed || !t.done)
            .filter(|t| t.matches(filter))
            .filter(|t| !past(last_day, t.due))
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

    /// Write to disk if there are pending changes. See [`crate::store`] for
    /// what "atomically" costs and buys.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        FILE.write(
            &self.path,
            &TaskFile {
                tasks: self.tasks.as_slice(),
            },
        )?;

        self.dirty = false;
        Ok(())
    }

    /// Save, recording any failure in [`TaskStore::last_error`] rather than
    /// propagating it. The panel renders that message, so a read-only disk
    /// shows up in the UI instead of vanishing.
    pub fn save_reporting(&mut self) {
        crate::store::report(self.save(), &mut self.last_error);
    }
}

/// Give each id after its first appearance the next free one, and say whether
/// any needed it. Shared with [`crate::note::NoteStore`], whose file has the
/// same shape.
///
/// A hand-edited file can repeat an id — copying a block is the natural way to
/// add an entry — and every key acts by id, so two entries sharing one were
/// deleted together and edited as each other. The first keeps its id, so a
/// file that was right loads untouched; a store that renumbered anything is
/// dirty, so its next save writes the repair down.
pub(crate) fn renumber_repeated_ids<'a>(
    ids: impl Iterator<Item = &'a mut u64>,
    next_id: &mut u64,
) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut renumbered = false;
    for id in ids {
        if !seen.insert(*id) {
            *id = *next_id;
            *next_id += 1;
            renumbered = true;
        }
    }
    renumbered
}

/// The tasks written on first run. Ids are assigned by the store.
///
/// These teach the panel's keys by being the thing the keys operate on, which
/// is why they are worded as instructions rather than as a plausible errand
/// list. Between them they exercise every column — priority, tags, a due date
/// in each direction, and a note — so a first run shows the table doing its
/// job rather than a blank grid with headers over it.
fn example_tasks(today: Date) -> Vec<Task> {
    // Deliberately relative to `today`: fixed dates would be years overdue by
    // the time anyone reads them, and "overdue" is a state worth showing on
    // purpose rather than by accident.
    let in_days = |n: i32| today.checked_add(jiff::Span::new().days(n)).ok();

    vec![
        Task {
            priority: Priority::High,
            tags: vec!["mirador".into()],
            due: in_days(1),
            notes: Some(
                "Everything here is editable in place. Tab and Shift+Tab move \
                 between fields, Enter saves, Esc cancels. While a form is open \
                 the global keys are suppressed, so typing a q into a title \
                 cannot quit the dashboard."
                    .into(),
            ),
            ..Task::new(0, "Press e to edit this task", today)
        },
        Task {
            priority: Priority::Medium,
            tags: vec!["mirador".into()],
            due: in_days(3),
            ..Task::new(0, "Press a to add your own", today)
        },
        Task {
            tags: vec!["mirador".into()],
            notes: Some(
                "Sort cycles with s, / filters on title, tag and note text, and \
                 c shows completed tasks. Press o for the path to the TOML file \
                 behind all of this — it is plain text you can edit or keep in \
                 git."
                    .into(),
            ),
            ..Task::new(0, "Press ? to see every key", today)
        },
        Task {
            priority: Priority::Low,
            tags: vec!["example".into()],
            due: in_days(-1),
            notes: Some(
                "A task past its due date reads in red, and the counter in the \
                 border frames it as overdue. This one is here to show that; \
                 delete it with d once you have seen it."
                    .into(),
            ),
            ..Task::new(0, "This one is overdue", today)
        },
    ]
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
    use crate::store::testing::TempDir;

    /// The README shows a `todos.toml` and tells people they can edit it by
    /// hand. A wrong example there is worse than no example, because it fails
    /// on *their* file after they have typed something into it.
    ///
    /// So the example is extracted from the README and parsed. The one this
    /// replaced had drifted into a real task from this project's own
    /// development, dated and long since done — nothing checked it, so nothing
    /// noticed.
    #[test]
    fn the_task_file_example_in_the_readme_actually_parses() {
        let readme = crate::docs::repo_text("README.md");
        let start = readme
            .find("## Task file format")
            .expect("the README documents the task file format");
        let block = readme[start..]
            .split("```toml")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .expect("that section shows a toml example");

        let parsed: TaskFile = toml::from_str(block)
            .unwrap_or_else(|e| panic!("the README example does not parse: {e}\n\n{block}"));

        assert_eq!(parsed.tasks.len(), 2, "both examples must survive parsing");
        assert!(
            parsed.tasks.iter().any(|t| t.done && t.completed.is_some()),
            "the example should show what a finished task looks like"
        );
        assert!(
            parsed
                .tasks
                .iter()
                .any(|t| t.notes.is_none() && t.tags.is_empty()),
            "and that the optional fields really are optional"
        );
    }

    /// The README's compatibility section promises a task file keeps working in
    /// both directions. Both are checked here rather than asserted there.
    ///
    /// `Task` has not changed shape since `0.1.0`, so the backward half is a
    /// statement about history — pinned so that adding a *required* field, which
    /// is the one change that would break every existing file, fails here rather
    /// than on somebody's tasks.
    #[test]
    fn a_task_file_survives_a_version_in_either_direction() {
        // As 0.1.0 would have written it: only the three fields that have
        // always been required, and none of the optional ones.
        let ancient = "\
[[task]]
id = 1
title = \"Renew the domain\"
created = \"2026-08-01\"
";
        let parsed: TaskFile =
            toml::from_str(ancient).expect("a task file from 0.1.0 must still open");
        assert_eq!(parsed.tasks.len(), 1);
        assert_eq!(parsed.tasks[0].title, "Renew the domain");

        // And from a mirador newer than this one: an unknown field is ignored
        // rather than refusing the file. `TaskFile` deliberately does not set
        // `deny_unknown_fields` — unlike the config, nobody hand-writes a typo
        // here that a refusal would helpfully catch, and refusing would mean a
        // newer mirador's file locks an older one out of every task in it.
        let from_the_future = "\
[[task]]
id = 1
title = \"Renew the domain\"
created = \"2026-08-01\"
energy_level = \"high\"
";
        let parsed: TaskFile =
            toml::from_str(from_the_future).expect("an unknown field must not cost the whole file");
        assert_eq!(parsed.tasks.len(), 1, "the task itself must survive");
        assert_eq!(parsed.tasks[0].title, "Renew the domain");
    }
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

    /// `ALL` is where the parser and a refused `[todo].sort` both find their
    /// words, so a mode missing from it could be cycled to and never parsed.
    /// `next` is an exhaustive match, which makes it the list to check against.
    #[test]
    fn every_sort_mode_is_listed_once_and_parses_from_its_label() {
        let mut cycle = vec![SortMode::Smart];
        while cycle.last().unwrap().next() != SortMode::Smart {
            cycle.push(cycle.last().unwrap().next());
        }
        assert_eq!(cycle, SortMode::ALL, "ALL is the order `s` cycles in");
        for mode in SortMode::ALL {
            assert_eq!(mode.label().parse::<SortMode>().unwrap(), mode);
            let shouted = format!(" {} ", mode.label().to_ascii_uppercase());
            assert_eq!(shouted.parse::<SortMode>().unwrap(), mode);
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

    /// The filter folds case the way the notes search does, beyond ASCII.
    /// It folded only `A`–`Z`, so `übung` did not find a task called
    /// `Übung buchen` while the same words in a note were found — one program
    /// giving two answers to "does this contain that".
    ///
    /// The Greek cases are the fold's own trap. `str::to_lowercase` reads
    /// context, and a capital sigma ending a word becomes `ς` where one inside
    /// a word becomes `σ`, so `ΚΩΣ` typed on the way to `ΚΩΣΤΑΣ` folded to
    /// `κως`, which `κωστας` does not contain, and the task vanished three
    /// letters into its own name.
    #[test]
    fn the_filter_ignores_case_beyond_ascii() {
        let mut t = task(1, "Übung buchen");
        t.tags = vec!["ÉTÉ".into()];
        t.notes = Some("Ärger mit der Bahn".into());
        for needle in ["übung", "ÜBUNG", "été", "ärger"] {
            assert!(t.matches(needle), "{needle}");
        }
        assert!(!t.matches("übungen"));

        let greek = task(2, "ΚΩΣΤΑΣ");
        for needle in ["ΚΩΣ", "κωσ", "κως", "ΚΩΣΤΑΣ", "κωστας"] {
            assert!(greek.matches(needle), "{needle}");
        }
        assert!(!greek.matches("ΚΩΣΤΑΣΗ"));
    }

    /// Copying a `[[task]]` block is the natural way to add a task to a file
    /// whose header calls it safe to edit by hand, and it leaves two tasks
    /// sharing an id. Every key acts by id, so `d` on either deleted both and
    /// `e` on the copy rewrote the original — in silence, and saved.
    #[test]
    fn a_repeated_id_in_the_file_is_renumbered_so_each_key_acts_on_one_task() {
        let dir = tempdir();
        let path = dir.join("todos.toml");
        let block = |title: &str| {
            format!("[[task]]\nid = 3\ntitle = \"{title}\"\ncreated = \"2026-07-25\"\n\n")
        };
        std::fs::write(&path, block("original") + &block("copy")).unwrap();

        let mut store = TaskStore::load(&path).unwrap();
        let ids: Vec<u64> = store.tasks().iter().map(|t| t.id).collect();
        assert_eq!(ids[0], 3, "the first keeps the id the file gave it");
        assert_ne!(ids[1], 3, "the copy needs one of its own: {ids:?}");

        let mut copy = store.get(ids[1]).unwrap().clone();
        copy.title = "edited".into();
        assert!(store.update(copy));
        assert!(store.remove(ids[1]));
        let titles: Vec<&str> = store.tasks().iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["original"], "one key, one task");
        assert!(
            store.add(task(0, "new")) > ids[1],
            "and new ids climb past it"
        );

        // The file is put right by the next save, even with nothing changed.
        let mut untouched = TaskStore::load(&path).unwrap();
        untouched.save().unwrap();
        // Read the text, not a reload: loading renumbers again in memory,
        // so a reloaded store looks repaired whether or not the file is.
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            written.matches("id = 3\n").count(),
            1,
            "the save must write the new id:\n{written}"
        );
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

        let view = store.view(SortMode::Smart, false, "", 0, today());
        assert_eq!(view[0], overdue_id);
    }

    #[test]
    fn completed_tasks_are_hidden_unless_requested() {
        let dir = tempdir();
        let mut store = TaskStore::load(dir.join("todos.toml")).unwrap();
        let id = store.add(task(0, "done thing"));
        store.with_task(id, |t| t.toggle_done(today()));

        let view = store.view(SortMode::Smart, false, "", 0, today());
        assert!(view.is_empty(), "{view:?}");
        assert_eq!(store.view(SortMode::Smart, true, "", 0, today()).len(), 1);
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
            store
                .view(SortMode::Smart, false, "errand", 0, today())
                .len(),
            2
        );
        assert_eq!(
            store.view(SortMode::Smart, false, "milk", 0, today()).len(),
            1
        );
        assert_eq!(
            store.view(SortMode::Smart, false, "zzz", 0, today()).len(),
            0
        );
        assert_eq!(store.view(SortMode::Smart, false, "", 0, today()).len(), 2);
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

    /// A task file exactly as 1.20.0 wrote it, captured from that build's
    /// store: a quote in a title, a note across two lines, a non-ASCII dash,
    /// tags, both dates and a bare task.
    const WRITTEN_BY_1_20_0: &str = r#"# mirador tasks. Safe to edit by hand or keep in version control.
# Fields: id, title, notes, due (YYYY-MM-DD), priority (high|medium|low|none), tags, done, completed, created.

[[task]]
id = 1
title = 'Call "the" bank'
notes = """
Line one
Line two — with a dash"""
due = "2026-10-09"
priority = "high"
tags = [
    "home",
    "money",
]
done = false
created = "2026-10-01"

[[task]]
id = 2
title = "Renew the domain"
priority = "low"
done = true
completed = "2026-10-05"
created = "2026-09-30"

[[task]]
id = 3
title = "Plain"
priority = "low"
done = false
created = "2026-10-07"
"#;

    /// Reading and writing go through [`crate::store::TomlFile`] now, and a
    /// file in somebody's version control must not change by so much as a
    /// blank line because of it. The file is removed between the load and
    /// the save, so what is compared is what the save wrote.
    #[test]
    fn a_file_from_1_20_0_is_written_back_byte_for_byte() {
        let dir = tempdir();
        let path = dir.join("todos.toml");
        std::fs::write(&path, WRITTEN_BY_1_20_0).unwrap();

        let mut store = TaskStore::load(&path).unwrap();
        assert_eq!(store.tasks().len(), 3, "every task was read");
        std::fs::remove_file(&path).unwrap();
        store.dirty = true;
        store.save().unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), WRITTEN_BY_1_20_0);
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
    fn a_first_run_is_seeded_with_examples_and_written_to_disk() {
        let dir = tempdir();
        let path = dir.join("todos.toml");
        let store = TaskStore::load_or_seed(&path, today()).unwrap();

        assert!(!store.tasks().is_empty(), "first run must not be empty");
        assert_eq!(store.last_error, None, "seeding must not fail silently");
        assert!(
            path.exists(),
            "the seed must be written, not held in memory"
        );

        // The examples exist to demonstrate the columns, so every column the
        // table can draw needs at least one task exercising it.
        let tasks = store.tasks();
        assert!(tasks.iter().any(|t| t.due.is_some()), "no due date");
        assert!(tasks.iter().any(|t| t.notes.is_some()), "no note");
        assert!(tasks.iter().any(|t| !t.tags.is_empty()), "no tag");
        assert!(
            tasks.iter().any(|t| t.priority != Priority::None),
            "no priority"
        );
        assert!(
            tasks
                .iter()
                .any(|t| matches!(t.due_state(today()), DueState::Overdue(_))),
            "no overdue task, so nothing shows what overdue looks like"
        );
    }

    #[test]
    fn seeded_titles_fit_the_task_column_at_an_ordinary_width() {
        // The default layout gives the task panel 58% of the width, and the
        // DONE, PRI, TAGS and DUE columns take the rest, which leaves twenty-five
        // cells for the title on a 120-column terminal. These titles
        // are instructions, so a truncated one is a instruction you cannot
        // read — "Press ? for every key, h…" was the version that prompted
        // this test. Measured in display cells, not chars, for the usual
        // reason.
        const BUDGET: usize = 25;
        for task in example_tasks(today()) {
            let width = unicode_width::UnicodeWidthStr::width(task.title.as_str());
            assert!(
                width <= BUDGET,
                "seeded title is {width} cells, over the {BUDGET} the column \
                 can show at 120 columns: {:?}",
                task.title
            );
        }
    }

    #[test]
    fn seeding_happens_only_when_the_file_is_absent() {
        let dir = tempdir();
        let path = dir.join("todos.toml");

        // Deleting every example writes an empty file. Meeting them again on
        // the next run would make them impossible to get rid of.
        let mut store = TaskStore::load_or_seed(&path, today()).unwrap();
        let ids: Vec<u64> = store.tasks().iter().map(|t| t.id).collect();
        for id in ids {
            store.remove(id);
        }
        store.save().unwrap();

        let reopened = TaskStore::load_or_seed(&path, today()).unwrap();
        assert!(
            reopened.tasks().is_empty(),
            "an emptied list must stay empty across a restart"
        );
    }

    #[test]
    fn seeded_ids_are_unique_and_do_not_collide_with_later_additions() {
        let dir = tempdir();
        let mut store = TaskStore::load_or_seed(dir.join("todos.toml"), today()).unwrap();
        let seeded: Vec<u64> = store.tasks().iter().map(|t| t.id).collect();

        let added = store.add(Task::new(0, "mine", today()));
        assert!(
            !seeded.contains(&added),
            "a task added after seeding reused a seeded id"
        );

        let mut unique = seeded.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), seeded.len(), "seeded ids are not unique");
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
        TempDir::new("task")
    }
}
