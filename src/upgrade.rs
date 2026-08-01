//! An explicit upgrade, handed back to whichever installer owns this copy.
//!
//! cargo-dist installs `mirador-update` beside release binaries, while
//! `cargo install mirador` installs only `mirador`. The dashboard cannot tell
//! those apart when it prints an update notice, so the stable entry point is
//! `mirador --update`: use the sibling updater when it exists, otherwise ask
//! Cargo whether it owns this executable and hand the install back to it.
//!
//! Windows adds one constraint. Neither updater can replace an executable that
//! is still running, so this process renames itself aside before waiting for
//! the updater. The next launch removes that old image after Windows releases
//! it. Unix permits replacing a running executable and needs no handoff.

#[cfg(windows)]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

#[derive(Debug, PartialEq, Eq)]
enum Method {
    Installer(PathBuf),
    Cargo { root: PathBuf },
}

/// Update this installation and wait for the owning installer to finish.
pub fn run() -> Result<()> {
    let current = std::env::current_exe().context("finding the running mirador executable")?;
    let method = method(&current)?;

    #[cfg(windows)]
    {
        run_on_windows(&current, &method)
    }

    #[cfg(not(windows))]
    execute(&method)
}

/// Remove the executable a successful Windows update left behind.
///
/// Best-effort on purpose: failing to clean yesterday's binary must not stop
/// today's dashboard from opening. A later `--update` reports the failure
/// before it moves anything.
pub fn cleanup_stale() {
    #[cfg(windows)]
    if let Ok(current) = std::env::current_exe() {
        let _ = std::fs::remove_file(backup_path(&current));
    }
}

fn method(current: &Path) -> Result<Method> {
    let updater = current.with_file_name(if cfg!(windows) {
        "mirador-update.exe"
    } else {
        "mirador-update"
    });
    if updater
        .try_exists()
        .with_context(|| format!("checking for {}", updater.display()))?
    {
        return Ok(Method::Installer(updater));
    }

    let root = cargo_root(current).ok_or_else(|| {
        anyhow::anyhow!(
            "this copy has no installer updater beside it and is not in a Cargo bin directory.\n\n\
             Re-run mirador's installer, or update it with the package manager that put {} here.",
            current.display()
        )
    })?;
    let output = Command::new("cargo")
        .args(["install", "--list", "--root"])
        .arg(&root)
        .output()
        .with_context(|| {
            format!(
                "this looks like a Cargo installation at {}, but `cargo` could not be started",
                root.display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!(
            "Cargo could not inspect the installation at {} ({}).",
            root.display(),
            output.status
        );
    }
    let installed = String::from_utf8_lossy(&output.stdout);
    if !crates_io_owns_mirador(&installed) {
        anyhow::bail!(
            "this copy has no installer updater, and Cargo does not record it as a crates.io install.\n\n\
             Update it with the package manager or source checkout that put {} here.",
            current.display()
        );
    }

    Ok(Method::Cargo { root })
}

fn cargo_root(current: &Path) -> Option<PathBuf> {
    let bin = current.parent()?;
    if !bin
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("bin")
    {
        return None;
    }
    bin.parent().map(Path::to_path_buf)
}

/// A registry install has no source in parentheses; path and git installs do.
fn crates_io_owns_mirador(installed: &str) -> bool {
    installed.lines().any(|line| {
        line.strip_prefix("mirador v")
            .is_some_and(|rest| rest.ends_with(':') && !rest.contains(" ("))
    })
}

fn execute(method: &Method) -> Result<()> {
    let mut command = match method {
        Method::Installer(updater) => {
            println!("Updating this installer-managed copy.");
            Command::new(updater)
        }
        Method::Cargo { root } => {
            println!("Updating this Cargo installation.");
            let mut cargo = Command::new("cargo");
            cargo
                .args(["install", "mirador", "--locked", "--root"])
                .arg(root)
                .args(["--bin", "mirador"]);
            cargo
        }
    };

    let status = command.status().context("starting the updater")?;
    if !status.success() {
        anyhow::bail!("the updater exited with {status}");
    }
    Ok(())
}

#[cfg(windows)]
fn run_on_windows(current: &Path, method: &Method) -> Result<()> {
    let mut moved = MovedExecutable::new(current)?;
    match execute(method) {
        Ok(()) => {
            if current
                .try_exists()
                .with_context(|| format!("checking for updated {}", current.display()))?
            {
                moved.keep();
            } else {
                // The installer updater returns success when this version is
                // already current. In that case it writes nothing, so put the
                // unchanged executable straight back.
                moved.restore()?;
            }
            Ok(())
        }
        Err(update_error) => {
            moved
                .restore()
                .with_context(|| format!("the update failed ({update_error:#})"))?;
            Err(update_error)
        }
    }
}

#[cfg(windows)]
fn backup_path(current: &Path) -> PathBuf {
    let mut name = current
        .file_stem()
        .map_or_else(|| OsString::from("mirador"), OsString::from);
    name.push("-update-old");
    if let Some(extension) = current.extension() {
        name.push(".");
        name.push(extension);
    }
    current.with_file_name(name)
}

/// A running Windows image can be renamed but not deleted or overwritten.
#[cfg(windows)]
struct MovedExecutable {
    original: PathBuf,
    backup: PathBuf,
    settled: bool,
}

#[cfg(windows)]
impl MovedExecutable {
    fn new(current: &Path) -> Result<Self> {
        let backup = backup_path(current);
        match std::fs::remove_file(&backup) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("removing the previous update copy at {}", backup.display())
                });
            }
        }
        std::fs::rename(current, &backup).with_context(|| {
            format!(
                "moving the running executable aside from {} to {}",
                current.display(),
                backup.display()
            )
        })?;
        Ok(Self {
            original: current.to_path_buf(),
            backup,
            settled: false,
        })
    }

    fn restore(&mut self) -> Result<()> {
        if self.original.try_exists()? {
            self.settled = true;
            anyhow::bail!(
                "the updater wrote {}, so the previous executable was kept at {} instead of overwriting it",
                self.original.display(),
                self.backup.display()
            );
        }
        std::fs::rename(&self.backup, &self.original).with_context(|| {
            format!(
                "restoring {} from {}",
                self.original.display(),
                self.backup.display()
            )
        })?;
        self.settled = true;
        Ok(())
    }

    fn keep(&mut self) {
        self.settled = true;
    }
}

#[cfg(windows)]
impl Drop for MovedExecutable {
    fn drop(&mut self) {
        if !self.settled && !self.original.exists() {
            let _ = std::fs::rename(&self.backup, &self.original);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_root_is_the_parent_of_a_bin_directory() {
        assert_eq!(
            cargo_root(Path::new("/opt/mirador/bin/mirador")),
            Some(PathBuf::from("/opt/mirador"))
        );
        assert_eq!(cargo_root(Path::new("/opt/mirador/debug/mirador")), None);
    }

    #[test]
    fn only_a_crates_io_install_is_claimed_by_cargo() {
        assert!(crates_io_owns_mirador(
            "other v1.0.0:\n    other\nmirador v1.4.1:\n    mirador.exe\n"
        ));
        assert!(!crates_io_owns_mirador(
            "mirador v1.4.1 (C:\\src\\mirador):\n    mirador.exe\n"
        ));
        assert!(!crates_io_owns_mirador(
            "mirador v1.4.1 (https://github.com/example/mirador):\n    mirador\n"
        ));
        assert!(!crates_io_owns_mirador("some-mirador v1.4.1:\n"));
    }

    #[test]
    fn a_sibling_updater_wins_without_a_cargo_receipt() {
        let dir =
            std::env::temp_dir().join(format!("mirador-upgrade-method-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let current = dir.join(if cfg!(windows) {
            "mirador.exe"
        } else {
            "mirador"
        });
        let updater = current.with_file_name(if cfg!(windows) {
            "mirador-update.exe"
        } else {
            "mirador-update"
        });
        std::fs::write(&updater, b"updater").unwrap();

        assert_eq!(method(&current).unwrap(), Method::Installer(updater));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    mod windows {
        use super::*;

        struct TempDir(PathBuf);

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        fn executable(name: &str) -> (PathBuf, TempDir) {
            let dir =
                std::env::temp_dir().join(format!("mirador-upgrade-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("mirador.exe");
            std::fs::write(&path, b"old").unwrap();
            (path, TempDir(dir))
        }

        #[test]
        fn a_moved_executable_can_be_restored_after_failure() {
            let (current, _guard) = executable("restore");
            let backup = backup_path(&current);
            let mut moved = MovedExecutable::new(&current).unwrap();
            assert!(!current.exists());
            assert!(backup.exists());

            moved.restore().unwrap();
            assert_eq!(std::fs::read(&current).unwrap(), b"old");
            assert!(!backup.exists());
        }

        #[test]
        fn a_failed_updater_restores_the_running_executable() {
            let (current, guard) = executable("failed-command");
            let method = Method::Installer(guard.0.join("missing-updater.exe"));

            let error = run_on_windows(&current, &method).unwrap_err();
            assert!(error.to_string().contains("starting the updater"));
            assert_eq!(std::fs::read(&current).unwrap(), b"old");
            assert!(!backup_path(&current).exists());
        }

        #[test]
        fn a_successful_replacement_leaves_the_old_image_for_next_launch() {
            let (current, _guard) = executable("replace");
            let backup = backup_path(&current);
            let mut moved = MovedExecutable::new(&current).unwrap();
            std::fs::write(&current, b"new").unwrap();
            moved.keep();
            drop(moved);

            assert_eq!(std::fs::read(&current).unwrap(), b"new");
            assert_eq!(std::fs::read(&backup).unwrap(), b"old");
            std::fs::remove_file(&backup).unwrap();
            assert!(!backup.exists());
        }
    }
}
