//! Writing a file without the risk of losing the old one.
//!
//! Every file mirador owns — tasks, notes, the watchlist, remembered
//! preferences, and the config itself — goes through [`write_atomic`]. Four
//! near-identical copies of this used to sit in the modules that needed it, and
//! `App::write_layout` had a fifth that was not atomic at all: it overwrote the
//! user's config with a bare `fs::write`, so a crash or a full disk part-way
//! through left them with a truncated config file and no way back.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};

/// Replace `path`'s contents with `contents`, or leave the file as it was.
///
/// Write to a temp file, flush it to the disk, then rename it over the target.
/// The temp file is created in the same directory so the rename stays on one
/// filesystem, which is what makes it atomic: a reader sees either the old file
/// or the new one, never a half-written one.
///
/// `sync_all` before the rename is the part that is easy to leave out and hard
/// to notice missing. Without it the rename can reach the disk before the
/// contents do, so a power loss can leave the *new* name pointing at an empty
/// or partial file — the one outcome the temp-and-rename dance is there to
/// prevent. It is not free, but these files are a few kilobytes and are written
/// when the user changes something, not on a timer.
///
/// **A symlink is written through, not over.** A rename replaces whatever has
/// the name, so renaming onto a link put a regular file where the link had
/// been: a config symlinked into a dotfiles repository lost its link on the
/// first layout change, and the copy under version control silently stopped
/// hearing about anything after that. The file at the end of the link is found
/// first, by [`resolve`], and the temporary goes beside *it* — which is also
/// what keeps the rename on one file system when the link crosses to another.
///
/// **A failed save takes its temporary with it.** The name is unique per write
/// and a failed save is tried again at the next change, so a full disk, or a
/// file held open by an editor on Windows, left one more temporary beside the
/// user's data for every attempt, each holding as much of the file as fitted.
pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let target = resolve(path)?;

    if let Some(parent) = folder_of(&target) {
        if target == path {
            // Not `is_dir()` first: the check would be a race, and
            // `create_dir_all` is already a no-op when the directory exists.
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        } else if !parent.is_dir() {
            // A link into a folder that is not there yet — a sync folder on a
            // new machine, a dotfiles checkout not cloned — is a promise that
            // the file will arrive. Making the folder and a default file in it
            // would put a conflict where the real one is about to land.
            anyhow::bail!(
                "{} links to {}, whose folder does not exist",
                path.display(),
                target.display()
            );
        }
    }

    let tmp = temp_path(&target);
    let installed = install(&tmp, &target, contents);
    if installed.is_err() {
        // Best effort, and quietly: a temporary that will not go either is no
        // reason to report anything but the failure that left it there.
        let _ = std::fs::remove_file(&tmp);
    }
    installed
}

/// Write `contents` to `tmp`, flush it, and rename it over `target`.
///
/// Split out of [`write_atomic`] so that every way this can fail passes
/// through one place on the way out, which is where the temporary is removed.
fn install(tmp: &Path, target: &Path, contents: &str) -> Result<()> {
    use std::io::Write;

    // Scoped so the file is closed before the rename. Windows refuses to rename
    // over an open file, and this runs there.
    {
        // The reason first, then the file: a status bar cuts the end off a long
        // message, and a temporary's path beside a read-only target — a config
        // linked into /nix/store — is long enough that only the path survived.
        let mut file = create_temp(tmp, target)
            .map_err(|e| anyhow::anyhow!("{e}: cannot write beside {}", target.display()))?;
        carry_permissions_across(target, &file);
        file.write_all(contents.as_bytes())
            .map_err(|e| anyhow::anyhow!("{e}: cannot write {}", target.display()))?;
        file.sync_all()
            .map_err(|e| anyhow::anyhow!("{e}: cannot flush {} to disk", target.display()))?;
    }

    std::fs::rename(tmp, target)
        .map_err(|e| anyhow::anyhow!("{e}: cannot replace {}", target.display()))
}

/// The folder `path` is in, as a path that can be asked about.
///
/// `Path::parent` gives `""` for a bare name such as `mirador.toml`. That
/// means the working directory, but it is not a path to anything — `is_dir`
/// says no — so `--config mirador.toml`, linking to a sibling by a bare name,
/// was refused as a link into a folder that does not exist.
fn folder_of(path: &Path) -> Option<&Path> {
    path.parent().map(|parent| {
        if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        }
    })
}

/// Symbolic links followed before a write gives up: the figure Linux allows.
const MAX_LINKS: usize = 40;

/// The file a write to `path` has to land in, past any symlinks it names.
///
/// Only the last component needs following. A rename resolves the directories
/// in a path as any other call does; it is only the final name that it
/// replaces instead.
///
/// Followed by hand rather than with `canonicalize`, for two reasons. A link to
/// a file not written yet does not canonicalize — there is nothing at the end
/// of it to find — and a dotfiles checkout made ready before the first run is
/// exactly that. And on Windows `canonicalize` turns every path into its
/// `\\?\` form, which would then appear in the error messages for files that
/// were never links at all.
///
/// Anything that is not a link — a file, a directory, nothing at all — ends
/// the walk where it stands, and whatever is wrong with it is left for the
/// write to report. A chain that never ends is refused: following it for ever
/// would hang the dashboard, and putting a file over one of its links would
/// quietly undo whatever it was set up for.
fn resolve(path: &Path) -> Result<PathBuf> {
    let mut at = path.to_path_buf();
    // One more look than there are links allowed: after the fortieth link the
    // walk still has to see that what it reached is not a link.
    for _ in 0..=MAX_LINKS {
        let Ok(link) = std::fs::read_link(&at) else {
            return Ok(at);
        };
        // A relative link is read from the directory the link is in.
        at = match at.parent() {
            Some(dir) => dir.join(link),
            None => link,
        };
    }
    anyhow::bail!("{} leads through too many symbolic links", path.display())
}

/// Open the temporary, already no wider than the original it will replace.
///
/// A rename puts a *new* file in place, created with whatever the umask says —
/// usually 0644. Somebody who ran `chmod 600` on their task list did so on
/// purpose, and had it silently widened to world-readable the next time they
/// added a task. Tasks and notes hold whatever the user decided to write down.
///
/// The mode is given to the file as it is made, and that is the point. It used
/// to be copied on after the write and the flush, so for as long as those took
/// the whole task list sat in a file anyone could read — and anyone who opened
/// it then could go on reading after the mode was tightened, since permission
/// is checked when a file is opened and never again.
///
/// Unix only, as [`carry_permissions_across`] is, and for the same reason.
#[cfg_attr(not(unix), allow(unused_variables))]
fn create_temp(tmp: &Path, original: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    // `create_new`, so the mode below is the mode the file is born with. An
    // open of a name that already exists keeps that file's old mode — and the
    // name can exist: a write killed before its rename leaves its temporary,
    // and a later process can be given the same id. That leftover is ours to
    // remove; anything else in the way is reported.
    options.write(true).create_new(true);
    #[cfg(unix)]
    if let Ok(existing) = std::fs::metadata(original) {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(existing.permissions().mode() & 0o7777);
    }
    match options.open(tmp) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(tmp)?;
            options.open(tmp)
        }
        opened => opened,
    }
}

/// Give the temporary exactly the permissions the original had.
///
/// [`create_temp`] made it with the original's mode, but through the umask,
/// which only takes bits away, so a task list made group-writable on purpose
/// would come back narrowed. This puts them back, and does it while the file is
/// still empty, so it can only ever widen the file as far as the original was.
///
/// Best effort: a failure here is not a reason to abandon a save that is
/// otherwise fine, and the alternative — refusing to write — loses the data the
/// permissions were protecting.
///
/// Unix only. Windows inherits an ACL from the containing directory rather than
/// carrying a mode on the file, so there is nothing of the same shape to copy.
#[cfg_attr(not(unix), allow(unused_variables))]
fn carry_permissions_across(from: &Path, to: &std::fs::File) {
    #[cfg(unix)]
    if let Ok(existing) = std::fs::metadata(from) {
        let _ = to.set_permissions(existing.permissions());
    }
}

/// Where the temporary copy goes while it is being written.
///
/// Appended rather than substituted, because `with_extension` would turn
/// `mirador.toml` into `mirador.tmp` — and if the rename then failed, the file
/// left behind would not say what it came from.
///
/// **Unique per write, and that is not tidiness.** The name used to be a plain
/// `.tmp`, shared by every writer of that file. Two mirador windows — one per
/// monitor, which is an ordinary way to use a dashboard — then raced: both
/// created the same temporary, and whoever renamed second found it already
/// gone. Measured with eight concurrent writers over three hundred rounds:
/// 2,100 of 2,400 writes failed, and a failed save is reported to the user, so
/// the second window filled with "could not be saved" for no reason.
///
/// There was a narrower hazard behind it too. `File::create` truncates, so a
/// second writer opening the shared temporary emptied the first writer's
/// half-written file underneath it; the first would then have renamed that into
/// place. That never reproduced in the probe above, but it needs no reproducing
/// to be worth removing.
///
/// Process id and a counter, rather than randomness: no dependency, unique
/// across processes and within one, and the leftovers of a crash are
/// identifiable rather than mysterious.
fn temp_path(path: &Path) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
}

/// A `.bak` name beside `path` that nothing is using yet.
///
/// Numbered rather than overwritten, so a second reset does not destroy what
/// the first one preserved. Lived in `config` while the config was the only
/// thing ever set aside; it belongs here now that state and the data files use
/// it too, because "do not lose the old file" is this module's whole job.
///
/// A name is taken if *anything* has it, a link included, and a link is asked
/// about as itself. A backup can be a link — [`move_aside`] moves one as it
/// stands — and once the file at its end was gone, `exists` followed it, found
/// nothing, and called the name free: the next reset then copied the config
/// through the old backup into whatever folder it pointed at.
pub(crate) fn free_backup_path(path: &Path) -> std::path::PathBuf {
    let taken = |candidate: &Path| std::fs::symlink_metadata(candidate).is_ok();
    let first = path.with_extension("toml.bak");
    if !taken(&first) {
        return first;
    }
    for n in 2..1000 {
        let candidate = path.with_extension(format!("toml.bak.{n}"));
        if !taken(&candidate) {
            return candidate;
        }
    }
    first
}

/// Move `path` out of the way, keeping its contents under a `.bak` name.
///
/// Returns where it went, or `None` if there was nothing there.
///
/// Renamed rather than deleted, and that is the point. A factory reset has to
/// leave the reader where a fresh install would — which means these files must
/// be *gone* from where mirador looks — but "gone" and "destroyed" are not the
/// same thing, and this program does not destroy a task list. The same reason
/// `write_atomic` exists.
///
/// A symlink is moved as itself, which is the one place in this module a link
/// is not followed, and on purpose: the link is what puts the file where
/// mirador looks, so setting the link aside is setting the file aside, and the
/// file at the end of it is left exactly as it was. That holds for a link
/// whose file is missing too, so the question is asked of the link and not
/// through it: left standing, it is what the next launch would write through.
pub fn move_aside(path: &Path) -> Result<Option<std::path::PathBuf>> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => anyhow::bail!("could not check whether {} exists: {e}", path.display()),
    }

    let to = free_backup_path(path);
    std::fs::rename(path, &to)
        .with_context(|| format!("moving {} to {}", path.display(), to.display()))?;
    Ok(Some(to))
}

/// Record the outcome of a save where the caller cannot handle a failure.
///
/// Every data store — tasks, notes, world clocks, the watchlist — keeps the
/// reason for its last failed save so its panel can render it. Swallowing the
/// error is deliberate — a read-only disk must not take the dashboard down —
/// but swallowing it *silently* is not: an edit that never reached the disk is
/// exactly what the user needs told.
pub fn report(result: Result<()>, last_error: &mut Option<String>) {
    *last_error = match result {
        Ok(()) => None,
        Err(e) => Some(format!("{e:#}")),
    };
}

/// One of the lists mirador keeps as TOML beside the config — the tasks, the
/// notes, the watchlist, the world clocks — as far as reading and writing it.
///
/// Each of the four stores used to carry its own copy of this, and the next
/// list would have been a fifth. What stays in a store is what is genuinely
/// its own: the shape of the file, and what happens when there is none. That
/// is two policies on purpose — the task list and the notes seed examples and
/// write them at once, the watchlist and the clocks seed from the config and
/// leave the write to their panel — so [`TomlFile::read`] says only that there
/// was nothing to read, and the store decides.
pub struct TomlFile {
    /// What the file holds, as an error names it: `tasks`, `the watchlist`.
    pub what: &'static str,
    /// The comment the file opens with, without the newline that ends it.
    pub header: &'static str,
}

impl TomlFile {
    /// The file at `path`, or `None` when there is no file there.
    ///
    /// "No file" is whatever `Path::exists` says, which is what each store
    /// asked before this was shared; a store that seeds asks the same
    /// question, and the two must not disagree about a first run.
    pub fn read<T: DeserializeOwned>(&self, path: &Path) -> Result<Option<T>> {
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading {} from {}", self.what, path.display()))?;
        toml::from_str(&raw)
            .map(Some)
            .with_context(|| format!("parsing {} in {}", self.what, path.display()))
    }

    /// Replace the file at `path` with `value` under the header, through
    /// [`write_atomic`].
    ///
    /// A borrow, and that is the point of the signature: each store used to
    /// copy its whole list into a wrapper only to serialise it, and now hands
    /// over a view of the list where it stands.
    pub fn write<T: Serialize + ?Sized>(&self, path: &Path, value: &T) -> Result<()> {
        let body =
            toml::to_string_pretty(value).with_context(|| format!("serialising {}", self.what))?;
        write_atomic(path, &format!("{}\n\n{body}", self.header))
    }
}

/// The line ending a file already uses.
///
/// Two modules rewrite files a person wrote by hand — [`crate::layout_edit`]
/// and [`crate::migrate`] — and both reassemble them from `str::lines()`, which
/// strips `\r` and hands back bare lines. Joining those with `\n` silently
/// converts a CRLF file to LF: on Windows, moving one panel rewrote every line
/// in the config, which shows up as a whole-file diff in git and is not
/// remotely what the user asked for.
///
/// Judged by the first ending in the file rather than by counting. A file with
/// mixed endings is already inconsistent and there is no answer that preserves
/// it; matching the first is at least predictable.
pub fn line_ending(source: &str) -> &'static str {
    match source.find('\n') {
        Some(at) if at > 0 && source.as_bytes()[at - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

/// A line up to the `#` that starts its TOML comment, if it has one.
///
/// Only a `#` outside a string starts one. TOML quotes with `'` as well as
/// `"`, and a `\"` inside a `"` string is part of it, so both are tracked:
/// knowing only `"`, a scan cuts `[plugins.config.'chan#1']` at the `#` and
/// leaves a line that is no longer a header. Each line is scanned on its own,
/// so the later lines of a multi-line string are read as though outside it,
/// the limit both of its callers already live with.
///
/// Three modules read a hand-written file line by line, and they have to
/// agree on where a line's text ends. Two of them did not: [`crate::layout_edit`]
/// stripped comments and [`crate::migrate`] did not, so `[theme] # my colours`
/// — a header, by the parser's reckoning — was no header to the migration,
/// and a retired key under it was neither rewritten nor hinted at. Both call
/// this now. The third, [`crate::keymap`], keeps its own copy of the same
/// scan in `bracket_depth_change`, because it counts brackets in the same
/// walk; a change to the quoting rule here belongs there too.
pub fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if ch == '\\' && q == '"' {
                    escaped = true;
                } else if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                '#' => return &line[..index],
                _ => {}
            },
        }
    }
    line
}

/// Scaffolding the whole crate's tests share.
#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A scratch directory of a test's own, created empty and removed when
    /// it drops — on a failed assertion too, which is what the hand-rolled
    /// `remove_dir_all` at the foot of a test never managed.
    ///
    /// The name carries the process id *and* a counter, so two tests are
    /// never handed the same directory: not two binaries in one `cargo test`,
    /// not two threads in one binary, not two calls with the same tag. Sharing
    /// one is how the zone tests came to read each other's files on Windows.
    /// The tag is only there to say whose directory it is when one is found.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new(tag: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("mirador-{tag}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a scratch directory");
            Self(dir)
        }
    }

    impl std::ops::Deref for TempDir {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl AsRef<Path> for TempDir {
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The temporaries [`write_atomic`](super::write_atomic) left in `dir`.
    ///
    /// Read off the directory rather than worked out from the target's name:
    /// the name has changed once (from a plain `.tmp` to one unique per
    /// write), and a test checking the old name went on passing against a
    /// path that was never created.
    pub(crate) fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "tmp"))
            .map(|path| path.display().to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{TempDir, leftovers};
    use super::*;

    /// A factory reset has to leave these files gone from where mirador looks,
    /// without destroying them. Both halves matter and the second is the one
    /// worth a test — deleting a task list would be the single most
    /// unforgivable thing this program could do.
    #[test]
    fn moving_aside_leaves_nothing_behind_and_loses_nothing() {
        let dir = TempDir::new("aside");

        let path = dir.join("todos.toml");
        std::fs::write(&path, "title = \"do not lose me\"").expect("writes");

        let moved = move_aside(&path).expect("moves").expect("there was a file");
        assert!(
            !path.exists(),
            "the original must be gone from where mirador looks"
        );
        assert_eq!(
            std::fs::read_to_string(&moved).expect("readable"),
            "title = \"do not lose me\"",
            "and every byte of it kept"
        );

        assert!(
            move_aside(&path).expect("no error").is_none(),
            "moving an absent file is a normal outcome, not a failure"
        );
    }

    /// Reset twice and the first rescue must survive the second.
    #[test]
    fn a_second_move_does_not_overwrite_the_first_backup() {
        let dir = TempDir::new("aside2");

        let path = dir.join("notes.toml");
        std::fs::write(&path, "first").expect("writes");
        let first = move_aside(&path).expect("moves").expect("a file");
        std::fs::write(&path, "second").expect("writes again");
        let second = move_aside(&path).expect("moves").expect("a file");

        assert_ne!(first, second, "the second needs a name of its own");
        assert_eq!(
            std::fs::read_to_string(&first).expect("readable"),
            "first",
            "the first backup must not have been clobbered"
        );
    }

    /// Two mirador windows is an ordinary way to use a dashboard — one per
    /// monitor — and both write the same files. The temporary was a fixed
    /// `.tmp` shared by every writer, so whoever renamed second found it gone.
    /// Eight writers over a hundred rounds failed 87% of their saves, and a
    /// failed save is reported, so the second window filled with "could not be
    /// saved" for no reason at all.
    #[test]
    fn concurrent_writers_do_not_take_each_others_saves_away() {
        let dir = TempDir::new("concurrent");
        let path = dir.join("todos.toml");
        let (a, b) = ("A".repeat(20_000), "B".repeat(20_000));

        let mut failures = 0;
        for _ in 0..100 {
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let p = path.clone();
                    let c = if i % 2 == 0 { a.clone() } else { b.clone() };
                    std::thread::spawn(move || write_atomic(&p, &c))
                })
                .collect();
            for handle in handles {
                // A panicked thread counts as a failure too, which the
                // shorter `is_ok_and(|r| r.is_err())` quietly does not.
                if !handle.join().is_ok_and(|result| result.is_ok()) {
                    failures += 1;
                }
            }

            // And whatever landed is one writer's work entire, never a blend.
            let got = std::fs::read_to_string(&path).expect("readable");
            assert!(
                got == a || got == b,
                "a writer's file was left mixed or truncated: {} bytes",
                got.len()
            );
        }
        assert_eq!(failures, 0, "{failures} of 800 concurrent saves failed");
    }

    /// A rename puts a *new* file in place, created per the umask. Somebody who
    /// ran `chmod 600` on their tasks did it on purpose and had it widened to
    /// world-readable the next time they added one.
    #[cfg(unix)]
    #[test]
    fn a_restricted_file_stays_restricted_after_a_save() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("perms");
        let path = dir.join("todos.toml");

        std::fs::write(&path, "before").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");

        write_atomic(&path, "after").expect("saves");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the file was widened to {mode:o}");
    }

    #[test]
    fn a_write_creates_the_file_and_its_parent_directory() {
        let dir = TempDir::new("create");
        let path = dir.join("nested/deeper/notes.toml");
        write_atomic(&path, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn a_write_replaces_the_previous_contents_completely() {
        let dir = TempDir::new("replace");
        let path = dir.join("notes.toml");
        write_atomic(&path, "a much longer first version").unwrap();
        write_atomic(&path, "short").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "short",
            "a shorter second write must not leave a tail of the first"
        );
    }

    /// A write leaves the file it wrote and nothing else.
    #[test]
    fn no_temp_file_is_left_behind() {
        let dir = TempDir::new("tidy");
        let path = dir.join("notes.toml");
        write_atomic(&path, "x").unwrap();
        let left = leftovers(&dir);
        assert!(left.is_empty(), "left behind: {left:?}");
    }

    #[test]
    fn a_temp_name_says_where_it_came_from_and_is_never_reused() {
        let toml = temp_path(Path::new("/data/mirador.toml"));
        let md = temp_path(Path::new("/data/mirador.md"));

        // `with_extension` would turn `mirador.toml` into `mirador.tmp`, which
        // both collides with `mirador.md`'s temporary and loses the fact that
        // it came from a `.toml` at all. A leftover from a crash should name
        // its origin.
        let shown = toml.to_string_lossy().into_owned();
        assert!(
            shown.contains("mirador.toml"),
            "keeps the full name: {shown}"
        );
        // `ends_with` on the string, not on the path's extension: the point is
        // the literal suffix a person would see in a directory listing.
        assert!(
            shown.rsplit('.').next() == Some("tmp"),
            "and is recognisable: {shown}"
        );
        assert_ne!(toml, md, "two files must not share one temp path");

        // And the same file twice must not either, which is the whole reason
        // two mirador windows stopped taking each other's saves away.
        assert_ne!(
            temp_path(Path::new("/data/mirador.toml")),
            temp_path(Path::new("/data/mirador.toml")),
            "two writes of one file must not share a temp path"
        );
    }

    #[test]
    fn a_failure_is_recorded_and_a_success_clears_it() {
        let mut last = Some("stale".to_string());
        report(Ok(()), &mut last);
        assert_eq!(last, None);

        report(Err(anyhow::anyhow!("disk full")), &mut last);
        assert_eq!(last.as_deref(), Some("disk full"));
    }

    #[test]
    fn writing_where_a_directory_blocks_the_way_fails_rather_than_panics() {
        let dir = TempDir::new("blocked");
        let path = dir.join("occupied");
        std::fs::create_dir_all(&path).unwrap();
        let err = write_atomic(&path, "x").expect_err("a directory is not writable as a file");
        assert!(
            format!("{err:#}").contains("occupied"),
            "the error must name the path: {err:#}"
        );

        // The temporary was written in full before the rename refused it, so
        // this is the failure that leaves the most behind. A failed save is
        // tried again on the next keystroke under a fresh name, so a leftover
        // here is one more file beside the user's data for every key pressed
        // while the fault lasts.
        let left = leftovers(&dir);
        assert!(left.is_empty(), "a failed save left behind: {left:?}");
    }

    /// Whether `path` is still a link rather than a file put where one was.
    #[cfg(unix)]
    fn is_link(path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
    }

    /// A config or task list symlinked into a dotfiles repository or a synced
    /// folder is an ordinary way to keep one. A rename replaces whatever has
    /// the name, so writing to the link replaced the *link*: the first layout
    /// change put a regular file where it had been, and the copy under version
    /// control silently stopped hearing about anything after that.
    #[cfg(unix)]
    #[test]
    fn a_write_through_a_symlink_lands_in_the_file_it_points_at() {
        let dir = TempDir::new("symlink");
        let (config, dotfiles) = (dir.join("config"), dir.join("dotfiles"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&dotfiles).unwrap();

        let real = dotfiles.join("mirador.toml");
        let link = config.join("config.toml");
        std::fs::write(&real, "before").unwrap();
        // Relative, as `ln -s` is usually given one: it has to be read from the
        // link's own directory, not from wherever mirador was started.
        std::os::unix::fs::symlink("../dotfiles/mirador.toml", &link).unwrap();

        write_atomic(&link, "after").expect("saves");

        assert!(is_link(&link), "the link was replaced by a regular file");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "after");

        // Written beside the file it replaces, which is what keeps the rename
        // on one file system, and gone from both directories once it landed.
        let left = [leftovers(&config), leftovers(&dotfiles)].concat();
        assert!(left.is_empty(), "left behind: {left:?}");
    }

    /// A link set up before the file it names exists — a dotfiles checkout
    /// made ready ahead of the first run — is followed rather than replaced,
    /// through as many links as there are, as any other write to it would be.
    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_file_not_yet_written_is_followed_rather_than_replaced() {
        let dir = TempDir::new("dangling");
        let real = dir.join("real.toml");
        let (near, far) = (dir.join("near.toml"), dir.join("far.toml"));
        std::os::unix::fs::symlink(&real, &near).unwrap();
        std::os::unix::fs::symlink("near.toml", &far).unwrap();

        write_atomic(&far, "first run").expect("saves");

        assert!(is_link(&near) && is_link(&far), "a link was replaced");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "first run");
    }

    /// Two links naming each other lead nowhere. Following them for ever would
    /// hang the dashboard on a save, and giving up by putting a regular file
    /// over one of them would quietly undo whatever they were set up for.
    #[cfg(unix)]
    #[test]
    fn a_loop_of_symlinks_is_refused_and_left_alone() {
        let dir = TempDir::new("loop");
        let (a, b) = (dir.join("a.toml"), dir.join("b.toml"));
        std::os::unix::fs::symlink("b.toml", &a).unwrap();
        std::os::unix::fs::symlink("a.toml", &b).unwrap();

        let err = write_atomic(&a, "x").expect_err("there is nowhere to write");
        assert!(
            format!("{err:#}").contains("a.toml"),
            "the error must name the path: {err:#}"
        );
        assert!(is_link(&a) && is_link(&b), "a link was replaced");
        let left = leftovers(&dir);
        assert!(left.is_empty(), "left behind: {left:?}");
    }

    /// What the umask does to a file made with the default 0666.
    #[cfg(unix)]
    fn umask_result(dir: &TempDir) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        let probe = dir.join("probe");
        let mode = std::fs::File::create(&probe)
            .and_then(|f| f.metadata())
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap();
        let _ = std::fs::remove_file(&probe);
        mode
    }

    /// Carrying the mode across *after* the write left the whole task list in
    /// a file the umask had made world-readable for as long as the write and
    /// the flush took — and anyone who opened it then could go on reading it
    /// after the mode was tightened, because permission is checked when a
    /// file is opened and not again. The temporary has to be born private.
    ///
    /// Asserted as "no bit the original lacks", and skipped where the umask
    /// already makes a plain `File::create` that private: there the test could
    /// not tell the fix from its absence, and saying so beats passing.
    #[cfg(unix)]
    #[test]
    fn the_temporary_is_born_as_private_as_the_original() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("born");
        let path = dir.join("todos.toml");
        std::fs::write(&path, "before").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        if umask_result(&dir) & !0o600 == 0 {
            eprintln!("skipped: this umask already creates files 0600 or tighter");
            return;
        }

        // Straight from the creation, before anything is written into it or
        // done to it afterwards: the mode it has here is the mode it had when
        // the first byte of the task list arrived.
        let tmp = temp_path(&path);
        let file = create_temp(&tmp, &path).expect("creates");
        let mode = file.metadata().unwrap().permissions().mode() & 0o777;
        drop(file);
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(mode & !0o600, 0, "the temporary was created {mode:o}");
    }

    /// Created with the original's mode, the temporary is still put through
    /// the umask, which can only take bits away. A file made wider on purpose
    /// has to stay that way, not quietly narrow. 0666 is used because every
    /// umask but 000 strips something from it, so the restore always has work
    /// to do — a 0664 file passed without it wherever the umask was 002.
    #[cfg(unix)]
    #[test]
    fn a_permission_the_umask_would_remove_is_carried_across_too() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("umask");
        if umask_result(&dir) == 0o666 {
            eprintln!("skipped: a umask of 000 strips nothing to restore");
            return;
        }
        let path = dir.join("todos.toml");
        std::fs::write(&path, "before").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();

        write_atomic(&path, "after").expect("saves");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o666, "the file came back {mode:o}");
    }

    /// A link into a folder that does not exist yet is a file that has not
    /// arrived — a sync folder on a new machine. Making the folder would put a
    /// default file where the real one is about to land.
    #[cfg(unix)]
    #[test]
    fn a_link_into_a_missing_folder_is_refused_rather_than_built() {
        let dir = TempDir::new("missing-folder");
        let link = dir.join("config.toml");
        std::os::unix::fs::symlink("Sync/not-yet/mirador.toml", &link).unwrap();

        let err = write_atomic(&link, "x").expect_err("must refuse");
        assert!(format!("{err:#}").contains("does not exist"), "{err:#}");
        assert!(!dir.join("Sync").exists(), "the folder was created");
        assert!(is_link(&link), "the link was replaced");
    }

    /// A temporary left by a write that was killed before its rename, at the
    /// name a later write is given, is removed and remade rather than reopened
    /// with whatever mode it had.
    #[cfg(unix)]
    #[test]
    fn a_leftover_temporary_is_remade_with_the_originals_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("leftover");
        let path = dir.join("todos.toml");
        std::fs::write(&path, "before").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let tmp = dir.join("leftover.tmp");
        std::fs::write(&tmp, "stale").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();

        let file = create_temp(&tmp, &path).expect("creates");
        let mode = file.metadata().unwrap().permissions().mode() & 0o777;
        let len = file.metadata().unwrap().len();
        drop(file);
        assert_eq!(
            (mode & !0o600, len),
            (0, 0),
            "reopened at {mode:o}, {len} bytes"
        );
    }

    /// `--config mirador.toml` is a bare name, and `Path::parent` calls the
    /// folder it is in `""` — which is the working directory, but is not a
    /// path to anything, so `is_dir` says no. A link given that way, to a
    /// sibling named the same way, was refused as having no folder at all.
    #[test]
    fn a_bare_name_is_in_the_working_directory() {
        let folder = folder_of(Path::new("a"));
        assert_eq!(folder, Some(Path::new(".")));
        assert!(
            folder.is_some_and(Path::is_dir),
            "{folder:?} is not a folder"
        );
        assert_eq!(folder_of(Path::new("cfg/a")), Some(Path::new("cfg")));
    }

    /// A backup can be a link: a reset moves a linked file aside as the link.
    /// Once the file it points at was gone, `exists` followed the link, found
    /// nothing, and called the name free — so the next reset copied the config
    /// through it, into the reader's dotfiles or onto an error.
    #[cfg(unix)]
    #[test]
    fn a_backup_that_is_a_dangling_link_is_not_a_free_name() {
        let dir = TempDir::new("bak-dangling");
        let path = dir.join("config.toml");
        std::fs::write(&path, "current").unwrap();
        std::os::unix::fs::symlink("dotfiles/mirador.toml", dir.join("config.toml.bak")).unwrap();

        assert_eq!(free_backup_path(&path), dir.join("config.toml.bak.2"));
    }

    /// A link whose file is missing is still what puts that file where
    /// mirador looks: left standing, the next launch writes through it. A
    /// factory reset has to set it aside like anything else it finds there.
    #[cfg(unix)]
    #[test]
    fn a_dangling_link_is_moved_aside_as_itself() {
        let dir = TempDir::new("aside-dangling");
        let path = dir.join("todos.toml");
        std::os::unix::fs::symlink("Sync/todos.toml", &path).unwrap();

        let moved = move_aside(&path)
            .expect("no error")
            .expect("the link was there to move");
        assert!(
            std::fs::symlink_metadata(&path).is_err(),
            "the link is still where mirador looks"
        );
        assert_eq!(
            std::fs::read_link(&moved).unwrap(),
            Path::new("Sync/todos.toml")
        );
    }
}
