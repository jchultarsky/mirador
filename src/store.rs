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

    std::fs::rename(&tmp, path)
        .with_context(|| format!("replacing {} with {}", path.display(), tmp.display()))?;

    Ok(())
}

/// Where the temporary copy goes while it is being written.
///
/// Appended rather than substituted, because `with_extension` would turn
/// `mirador.toml` into `mirador.tmp` — and if the rename then failed, the file
/// left behind would not say what it came from.
fn temp_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
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
    fn the_temp_name_keeps_the_original_extension() {
        // `with_extension` would turn `mirador.toml` into `mirador.tmp`, which
        // collides with the temp file for `mirador.md` in the same directory.
        let toml = temp_path(Path::new("/data/mirador.toml"));
        let md = temp_path(Path::new("/data/mirador.md"));
        assert_eq!(toml, Path::new("/data/mirador.toml.tmp"));
        assert_ne!(toml, md, "two files must not share one temp path");
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
