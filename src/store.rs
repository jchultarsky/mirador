//! Writing a file without the risk of losing the old one.
//!
//! Every file mirador owns — tasks, notes, the watchlist, remembered
//! preferences, and the config itself — goes through [`write_atomic`]. Four
//! near-identical copies of this used to sit in the modules that needed it, and
//! `App::write_layout` had a fifth that was not atomic at all: it overwrote the
//! user's config with a bare `fs::write`, so a crash or a full disk part-way
//! through left them with a truncated config file and no way back.

use std::path::Path;

use anyhow::{Context, Result};

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
pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        // Not `is_dir()` first: the check would be a race, and `create_dir_all`
        // is already a no-op when the directory exists.
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }

    let tmp = temp_path(path);

    // Scoped so the file is closed before the rename. Windows refuses to rename
    // over an open file, and this runs there.
    {
        let mut file =
            std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(contents.as_bytes())
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("flushing {} to disk", tmp.display()))?;
    }

    carry_permissions_across(path, &tmp);

    std::fs::rename(&tmp, path)
        .with_context(|| format!("replacing {} with {}", path.display(), tmp.display()))?;

    Ok(())
}

/// Give the replacement the permissions the original had.
///
/// A rename puts a *new* file in place, created with whatever the umask says —
/// usually 0644. Somebody who ran `chmod 600` on their task list did so on
/// purpose, and had it silently widened to world-readable the next time they
/// added a task. Tasks and notes hold whatever the user decided to write down.
///
/// Best effort: a failure here is not a reason to abandon a save that is
/// otherwise fine, and the alternative — refusing to write — loses the data the
/// permissions were protecting.
///
/// Unix only. Windows inherits an ACL from the containing directory rather than
/// carrying a mode on the file, so there is nothing of the same shape to copy.
#[allow(unused_variables)]
fn carry_permissions_across(from: &Path, to: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(existing) = std::fs::metadata(from) {
            let mode = existing.permissions().mode();
            let _ = std::fs::set_permissions(to, std::fs::Permissions::from_mode(mode));
        }
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

/// Record the outcome of a save where the caller cannot handle a failure.
///
/// The three data stores all keep the reason for the last failed save so their
/// panel can render it. Swallowing the error is deliberate — a read-only disk
/// must not take the dashboard down — but swallowing it *silently* is not: an
/// edit that never reached the disk is exactly what the user needs told.
pub fn report(result: Result<()>, last_error: &mut Option<String>) {
    *last_error = match result {
        Ok(()) => None,
        Err(e) => Some(format!("{e:#}")),
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("mirador-store-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn join(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mirador-store-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test directory");
        dir
    }

    /// Two mirador windows is an ordinary way to use a dashboard — one per
    /// monitor — and both write the same files. The temporary was a fixed
    /// `.tmp` shared by every writer, so whoever renamed second found it gone.
    /// Eight writers over a hundred rounds failed 87% of their saves, and a
    /// failed save is reported, so the second window filled with "could not be
    /// saved" for no reason at all.
    #[test]
    fn concurrent_writers_do_not_take_each_others_saves_away() {
        let dir = scratch("concurrent");
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

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A rename puts a *new* file in place, created per the umask. Somebody who
    /// ran `chmod 600` on their tasks did it on purpose and had it widened to
    /// world-readable the next time they added one.
    #[cfg(unix)]
    #[test]
    fn a_restricted_file_stays_restricted_after_a_save() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("perms");
        let path = dir.join("todos.toml");

        std::fs::write(&path, "before").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");

        write_atomic(&path, "after").expect("saves");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the file was widened to {mode:o}");

        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn no_temp_file_is_left_behind() {
        let dir = TempDir::new("tidy");
        let path = dir.join("notes.toml");
        write_atomic(&path, "x").unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "tmp"))
            .map(|path| path.display().to_string())
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
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
    }
}
