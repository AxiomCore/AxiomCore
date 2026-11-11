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
/// from when that is a symlink elsewhere, then `PATH`.
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
        let explicit = PathBuf::from(explicit);
        if explicit.is_file() {
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
            if !directories.iter().any(|directory| directory == parent) {
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
        for directory in std::env::split_paths(path) {
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
