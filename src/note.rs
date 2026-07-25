//! Free-form notes and their persisted store.
//!
//! A note is deliberately thinner than a task: a title, a body, and the dates
//! it was written and last touched. There is no due date, no priority and no
//! done flag, because a note is not work to be tracked — it is something you
//! wrote down so you would not have to remember it.
//!
//! Storage mirrors [`crate::task`]: a plain TOML file you can edit by hand or
//! keep in git, written atomically (temp file in the same directory, then
//! rename) so an interrupted save can never truncate the file you already had.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// One note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub title: String,
    /// Free text. Empty is legitimate — a title-only note is a reminder.
    #[serde(default)]
    pub body: String,
    pub created: Date,
    /// Set only once the note has been edited after creation, so the list can
    /// show when something actually changed rather than repeating `created`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<Date>,
}

impl Note {
    pub fn new(id: u64, title: impl Into<String>, today: Date) -> Self {
        Self {
            id,
            title: title.into(),
            body: String::new(),
            created: today,
            updated: None,
        }
    }

    /// The date the list should show: when it last changed, else when it was
    /// written.
    pub fn shown_date(&self) -> Date {
        self.updated.unwrap_or(self.created)
    }

    /// Case-insensitive match against the title *and* the body.
    ///
    /// Searching the body matters more here than it does for tasks: the whole
    /// reason to keep a note is the text inside it, and a title you wrote in a
    /// hurry is often not what you later search for.
    pub fn matches(&self, needle: &str) -> bool {
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        self.title.to_lowercase().contains(&needle) || self.body.to_lowercase().contains(&needle)
    }
}

/// Serialisation wrapper so the file reads as a list of `[[note]]` tables.
#[derive(Debug, Default, Serialize, Deserialize)]
struct NoteFile {
    #[serde(default, rename = "note")]
    notes: Vec<Note>,
}

/// An owned, persisted collection of notes.
#[derive(Debug)]
pub struct NoteStore {
    path: PathBuf,
    notes: Vec<Note>,
    dirty: bool,
    /// High-water mark for ids, which only ever climbs.
    ///
    /// Deriving the next id from `max(id) + 1` would hand a deleted note's id
    /// straight to the next one written, so anything still holding the old id
    /// — a selection, a pending edit — would silently point at a different
    /// note. The mark is rebuilt from the file on load, which is safe: nothing
    /// holds an id across a restart.
    next_id: u64,
    /// The last save error, surfaced in the panel so failures are never silent.
    pub last_error: Option<String>,
}

impl NoteStore {
    /// Load from `path`, treating a missing file as an empty list.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let notes = if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("reading notes from {}", path.display()))?;
            let parsed: NoteFile = toml::from_str(&raw)
                .with_context(|| format!("parsing notes in {}", path.display()))?;
            parsed.notes
        } else {
            Vec::new()
        };

        let next_id = notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        Ok(Self {
            path,
            notes,
            dirty: false,
            next_id,
            last_error: None,
        })
    }

    /// Load, seeding one example note when there is no file yet.
    ///
    /// Same reasoning as the task list: an empty master-detail panel shows
    /// neither a list nor a body, so first run is the one moment it explains
    /// nothing at all. One note rather than several — the panel's whole point
    /// is that you can read a body without pressing anything, and a single
    /// selected note demonstrates that better than a list to scroll.
    ///
    /// Seeded only when the file is *absent*, so deleting it is permanent.
    pub fn load_or_seed(path: impl Into<PathBuf>, today: Date) -> Result<Self> {
        let path = path.into();
        let first_run = !path.exists();
        let mut store = Self::load(path)?;
        if first_run {
            store.add(example_note(today));
            store.save_reporting();
        }
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// Append a note and return its id.
    pub fn add(&mut self, mut note: Note) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        note.id = id;
        self.notes.push(note);
        self.dirty = true;
        id
    }

    pub fn get(&self, id: u64) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// Mutate a note in place, stamping `updated` if anything changed.
    ///
    /// The stamp is driven by an actual before/after comparison rather than by
    /// the fact that an editor was opened, so opening a note and pressing Esc
    /// does not make it look like you revised it.
    pub fn with_note<F: FnOnce(&mut Note)>(&mut self, id: u64, today: Date, f: F) -> bool {
        let Some(slot) = self.notes.iter_mut().find(|n| n.id == id) else {
            return false;
        };
        let before = slot.clone();
        f(slot);
        if slot.title != before.title || slot.body != before.body {
            slot.updated = Some(today);
            self.dirty = true;
            return true;
        }
        // Restore in case the closure touched `updated` without changing text.
        slot.updated = before.updated;
        false
    }

    /// Remove by id. Returns whether anything was removed.
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        let removed = self.notes.len() != before;
        self.dirty |= removed;
        removed
    }

    /// Ids in display order: most recently touched first.
    ///
    /// Notes have no priority to sort by, and the one you want is almost always
    /// the one you were last in.
    pub fn view(&self, filter: &str) -> Vec<u64> {
        let mut visible: Vec<&Note> = self.notes.iter().filter(|n| n.matches(filter)).collect();
        visible.sort_by(|a, b| {
            b.shown_date()
                .cmp(&a.shown_date())
                // Ties broken by id descending so a day's notes stay in a
                // stable order instead of shuffling on every redraw.
                .then(b.id.cmp(&a.id))
        });
        visible.iter().map(|n| n.id).collect()
    }

    /// Write to disk atomically if there are pending changes.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating data directory {}", parent.display()))?;
        }

        let file = NoteFile {
            notes: self.notes.clone(),
        };
        let body = toml::to_string_pretty(&file).context("serialising notes")?;
        let contents = format!(
            "# mirador notes. Safe to edit by hand or keep in version control.\n\
             # Fields: id, title, body, created (YYYY-MM-DD), updated.\n\n{body}"
        );

        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, &contents).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("replacing {} with {}", self.path.display(), tmp.display()))?;

        self.dirty = false;
        Ok(())
    }

    /// Save, recording any failure rather than propagating it, so a read-only
    /// disk shows up in the panel instead of vanishing.
    pub fn save_reporting(&mut self) {
        match self.save() {
            Ok(()) => self.last_error = None,
            Err(e) => self.last_error = Some(format!("{e:#}")),
        }
    }
}

/// The note written on first run. The id is assigned by the store.
fn example_note(today: Date) -> Note {
    Note {
        body: "You are reading it in the detail pane. That is the point of the \
               panel: a note's whole value is the text inside it, so making you \
               press a key to see any of it would turn \"glance at the \
               dashboard\" into \"operate the dashboard\".\n\n\
               a writes a new note and e edits this one. Tab moves between the \
               title and the body, Enter inside the body is an ordinary new \
               line, and Ctrl+S saves. d deletes, and / searches bodies as well \
               as titles — the title you wrote in a hurry is often not what you \
               later search for.\n\n\
               Notes live in a plain TOML file next to your tasks, written \
               atomically, and you can edit it by hand or keep it in git. \
               Delete this one once you have your own."
            .into(),
        ..Note::new(0, "This is a note", today)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(name: &str) -> (NoteStore, TempDir) {
        let dir = std::env::temp_dir().join(format!("mirador-note-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = NoteStore::load(dir.join("notes.toml")).unwrap();
        (store, TempDir(dir))
    }

    fn today() -> Date {
        Date::new(2026, 7, 25).unwrap()
    }

    #[test]
    fn a_first_run_is_seeded_and_an_emptied_file_stays_empty() {
        let dir = std::env::temp_dir().join(format!("mirador-note-{}-seed", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _guard = TempDir(dir.clone());
        let path = dir.join("notes.toml");

        let mut s = NoteStore::load_or_seed(&path, jiff::civil::date(2026, 7, 25)).unwrap();
        assert_eq!(s.notes().len(), 1, "first run must show one example note");
        assert_eq!(s.last_error, None, "seeding must not fail silently");
        assert!(
            path.exists(),
            "the seed must be written, not held in memory"
        );
        assert!(
            !s.notes()[0].body.is_empty(),
            "an empty body defeats the point: the detail pane would be blank"
        );

        // Deleting it writes an empty file; meeting it again next run would
        // make it impossible to get rid of.
        let id = s.notes()[0].id;
        s.remove(id);
        s.save().unwrap();

        let reopened = NoteStore::load_or_seed(&path, jiff::civil::date(2026, 7, 25)).unwrap();
        assert!(
            reopened.notes().is_empty(),
            "an emptied file must stay empty across a restart"
        );
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_store() {
        let (s, _g) = store("missing");
        assert!(s.notes().is_empty());
    }

    #[test]
    fn notes_survive_a_save_and_reload() {
        let (mut s, _g) = store("roundtrip");
        let id = s.add(Note::new(0, "Groceries", today()));
        s.with_note(id, today(), |n| n.body = "milk\neggs".into());
        s.save().unwrap();

        let reloaded = NoteStore::load(s.path()).unwrap();
        assert_eq!(reloaded.notes().len(), 1);
        let note = &reloaded.notes()[0];
        assert_eq!(note.title, "Groceries");
        assert_eq!(note.body, "milk\neggs", "newlines must survive TOML");
    }

    #[test]
    fn editing_stamps_updated_but_opening_and_changing_nothing_does_not() {
        let (mut s, _g) = store("stamp");
        let id = s.add(Note::new(0, "Title", today()));
        assert_eq!(s.get(id).unwrap().updated, None);

        let later = Date::new(2026, 8, 1).unwrap();
        assert!(!s.with_note(id, later, |_| {}), "no change, no stamp");
        assert_eq!(s.get(id).unwrap().updated, None);

        assert!(s.with_note(id, later, |n| n.body = "something".into()));
        assert_eq!(s.get(id).unwrap().updated, Some(later));
    }

    #[test]
    fn the_list_shows_the_most_recently_touched_note_first() {
        let (mut s, _g) = store("order");
        let old = s.add(Note::new(0, "old", Date::new(2026, 1, 1).unwrap()));
        let new = s.add(Note::new(0, "new", Date::new(2026, 6, 1).unwrap()));
        assert_eq!(s.view(""), vec![new, old]);

        // Touching the old one moves it to the top.
        s.with_note(old, Date::new(2026, 12, 1).unwrap(), |n| {
            n.body = "revised".into();
        });
        assert_eq!(s.view(""), vec![old, new]);
    }

    #[test]
    fn filtering_searches_the_body_as_well_as_the_title() {
        let (mut s, _g) = store("filter");
        let id = s.add(Note::new(0, "Meeting", today()));
        s.with_note(id, today(), |n| n.body = "ask about the invoice".into());

        assert_eq!(s.view("invoice"), vec![id], "the body must be searchable");
        assert_eq!(s.view("MEETING"), vec![id], "matching is case-insensitive");
        assert!(s.view("nothing here").is_empty());
        assert_eq!(s.view("   "), vec![id], "a blank filter matches everything");
    }

    #[test]
    fn ids_are_never_reused_after_a_removal() {
        let (mut s, _g) = store("ids");
        let first = s.add(Note::new(0, "one", today()));
        let second = s.add(Note::new(0, "two", today()));
        assert!(s.remove(second));
        let third = s.add(Note::new(0, "three", today()));
        assert_ne!(third, second, "a reused id would alias the removed note");
        assert_ne!(third, first);
    }

    #[test]
    fn saving_is_a_no_op_when_nothing_changed() {
        let (mut s, _g) = store("clean");
        s.add(Note::new(0, "one", today()));
        s.save().unwrap();
        let stamp = std::fs::metadata(s.path()).unwrap().modified().unwrap();

        s.save().unwrap();
        assert_eq!(
            std::fs::metadata(s.path()).unwrap().modified().unwrap(),
            stamp,
            "a clean store must not rewrite the file"
        );
    }
}
