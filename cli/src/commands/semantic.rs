//! `axiom semantic` and `axiom mcp` delegate to the separately installed
//! `acore-semantic` binary. The CLI has no protocol knowledge here: arguments
//! pass through verbatim and stdin, stdout and stderr are the child's own.
use anyhow::{bail, Result};
use std::{
    convert::Infallible,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
};

/// Overrides the lookup with an explicit executable path.
pub const BINARY_ENV: &str = "AXIOM_ACORE_SEMANTIC";

fn binary_name() -> String {
    format!("acore-semantic{}", std::env::consts::EXE_SUFFIX)
}

/// `axiom semantic ARGS` runs `acore-semantic ARGS`; `axiom mcp ARGS` runs
/// `acore-semantic mcp ARGS`. This dispatch runs before clap, CLI hooks, cloud
/// access, or telemetry so that stdout carries only the delegated protocol and
/// no argument (`--`, `--help`, non-UTF-8) is reinterpreted by the CLI.
pub fn native_entry() -> Result<bool> {
    let arguments: Vec<OsString> = std::env::args_os().collect();
    let Some(subcommand) = arguments.get(1) else {
        return Ok(false);
    };
    let forwarded = if subcommand == "semantic" {
        arguments[2..].to_vec()
    } else if subcommand == "mcp" {
        std::iter::once(OsString::from("mcp"))
            .chain(arguments[2..].iter().cloned())
            .collect()
    } else {
        return Ok(false);
    };
    match delegate(&forwarded)? {}
}

/// Replaces this process with `acore-semantic` on Unix, so signals and the exit
/// status are the child's own. Elsewhere, waits and exits with its code.
pub fn delegate(arguments: &[OsString]) -> Result<Infallible> {
    let binary = locate()?;
    let mut command = Command::new(&binary);
    command.args(arguments);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        bail!("Cannot run {}: {error}", binary.display());
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|error| anyhow::anyhow!("Cannot run {}: {error}", binary.display()))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

/// Lookup order: `AXIOM_ACORE_SEMANTIC` (no fallthrough when set), the
/// directory of the resolved `axiom` executable, the directory it was invoked
/// from when that is a symlink elsewhere (macOS only: `current_exe` is already
/// resolved on Linux), then the absolute directories of `PATH`.
///
/// The result is always an absolute path, so the file that was checked is the
/// file that runs, and nothing is ever taken from the current directory: an
/// empty or relative `PATH` entry is skipped.
pub fn locate() -> Result<PathBuf> {
    let current = std::env::current_exe().ok();
    locate_from(
        std::env::var_os(BINARY_ENV).as_deref(),
        current.as_deref(),
        std::env::var_os("PATH").as_deref(),
    )
}

fn locate_from(
    explicit: Option<&OsStr>,
    current_exe: Option<&Path>,
    path: Option<&OsStr>,
) -> Result<PathBuf> {
    if let Some(explicit) = explicit {
        // A relative value names a file under the current directory; it
        // must not be searched for on `PATH` when it runs.
        let explicit = if explicit.is_empty() {
            PathBuf::new()
        } else {
            std::path::absolute(explicit).unwrap_or_else(|_| PathBuf::from(explicit))
        };
        if explicit.is_absolute() && explicit.is_file() {
            return Ok(explicit);
        }
        bail!(
            "{BINARY_ENV} is set to {}, which is not an existing file. Fix or unset {BINARY_ENV}.",
            explicit.display()
        );
    }
    let name = binary_name();
    let mut searched = Vec::new();
    let mut directories = Vec::new();
    if let Some(current_exe) = current_exe {
        if let Some(parent) = current_exe
            .canonicalize()
            .ok()
            .as_deref()
            .and_then(Path::parent)
        {
            directories.push(parent.to_path_buf());
        }
        if let Some(parent) = current_exe.parent() {
            if parent.is_absolute() && !directories.iter().any(|directory| directory == parent) {
                directories.push(parent.to_path_buf());
            }
        }
    }
    for directory in directories {
        let candidate = directory.join(&name);
        if candidate.is_file() {
            return Ok(candidate);
        }
        searched.push(candidate.display().to_string());
    }
    if let Some(path) = path {
        for directory in absolute_directories(path) {
            let candidate = directory.join(&name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    searched.push("PATH".to_string());
    bail!(
        "acore-semantic is not installed. Looked in {BINARY_ENV} (unset), {}. \
         Install it with `brew install axiomcore/tap/acore-semantic`, place it next to the axiom executable, \
         or set {BINARY_ENV} to its path.",
        searched.join(", ")
    );
}

/// The `PATH` entries that name an absolute directory. An empty entry and
/// `.` mean the current directory, where a project could plant a binary.
fn absolute_directories(path: &OsStr) -> Vec<PathBuf> {
    std::env::split_paths(path)
        .filter(|directory| directory.is_absolute())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable(directory: &Path, name: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, "").unwrap();
        path
    }

    #[test]
    fn explicit_path_wins_and_never_falls_through() {
        let directory = tempfile::tempdir().unwrap();
        let explicit = executable(directory.path(), "custom");
        let sibling = executable(directory.path(), &binary_name());
        let axiom = executable(directory.path(), "axiom");
        assert_eq!(
            locate_from(Some(explicit.as_os_str()), Some(&axiom), None).unwrap(),
            explicit
        );
        let missing = directory.path().join("missing");
        let error = locate_from(Some(missing.as_os_str()), Some(&axiom), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains(BINARY_ENV), "{error}");
        assert!(sibling.is_file());
    }

    #[test]
    fn relative_explicit_path_is_resolved_before_it_runs() {
        // A file under the current directory, named relatively.
        let directory = tempfile::tempdir_in(".").unwrap();
        executable(directory.path(), "custom");
        let relative = Path::new(directory.path().file_name().unwrap()).join("custom");
        assert!(relative.is_relative());
        let found = locate_from(Some(relative.as_os_str()), None, None).unwrap();
        assert!(found.is_absolute(), "{}", found.display());
        assert_eq!(found, std::env::current_dir().unwrap().join(&relative));
        // An empty value and a directory are errors, with no fallthrough.
        let on_path = tempfile::tempdir().unwrap();
        executable(on_path.path(), &binary_name());
        let path = std::env::join_paths([on_path.path()]).unwrap();
        for value in [OsStr::new(""), directory.path().as_os_str()] {
            let error = locate_from(Some(value), None, Some(&path))
                .unwrap_err()
                .to_string();
            assert!(error.contains(BINARY_ENV), "{error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn empty_and_relative_path_entries_are_never_searched() {
        // The binary exists under the current directory; `PATH` reaches it
        // only through an empty entry, `.` and a relative directory.
        let directory = tempfile::tempdir_in(".").unwrap();
        executable(directory.path(), &binary_name());
        let relative = PathBuf::from(directory.path().file_name().unwrap());
        assert!(relative.join(binary_name()).is_file());
        let mut path = OsString::from(":.:");
        path.push(relative.as_os_str());
        path.push(":");
        assert!(absolute_directories(&path).is_empty());
        let error = locate_from(None, None, Some(&path))
            .unwrap_err()
            .to_string();
        assert!(error.contains("is not installed"), "{error}");
        // An absolute entry next to them is still searched.
        let absolute = std::env::current_dir().unwrap().join(&relative);
        path.push(absolute.as_os_str());
        assert_eq!(
            locate_from(None, None, Some(&path)).unwrap(),
            absolute.join(binary_name())
        );
    }

    #[test]
    fn sibling_of_resolved_executable_precedes_path() {
        let install = tempfile::tempdir().unwrap();
        let links = tempfile::tempdir().unwrap();
        let on_path = tempfile::tempdir().unwrap();
        let axiom = executable(install.path(), "axiom");
        let sibling = executable(install.path(), &binary_name());
        executable(on_path.path(), &binary_name());
        let link = links.path().join("axiom");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&axiom, &link).unwrap();
        #[cfg(not(unix))]
        std::fs::copy(&axiom, &link).unwrap();
        let path = std::env::join_paths([on_path.path()]).unwrap();
        let found = locate_from(None, Some(&link), Some(&path)).unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            sibling.canonicalize().unwrap()
        );
    }

    #[test]
    fn path_is_searched_last_and_missing_names_every_location() {
        let install = tempfile::tempdir().unwrap();
        let on_path = tempfile::tempdir().unwrap();
        let axiom = executable(install.path(), "axiom");
        let path = std::env::join_paths([on_path.path()]).unwrap();
        let error = locate_from(None, Some(&axiom), Some(&path))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("brew install axiomcore/tap/acore-semantic"),
            "{error}"
        );
        assert!(error.contains("PATH"), "{error}");
        let expected = executable(on_path.path(), &binary_name());
        assert_eq!(
            locate_from(None, Some(&axiom), Some(&path)).unwrap(),
            expected
        );
    }
}
