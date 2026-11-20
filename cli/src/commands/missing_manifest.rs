//! Clear error for a backend `.acore` file that has no `AxiomDeps.toml`.
//!
//! Without the manifest the compiler cannot tell a backend contract from the
//! legacy syntax and fails deep in the parser with
//! `ParseError("Unexpected token: Colon")`. When a check, build or run fails
//! with a parse error and no manifest exists in the file's directory or any
//! parent, replace that error with one that names the missing file.

use std::path::{Path, PathBuf};

pub const EXAMPLE: &str = "type = \"backend\"\ntargets = [\"server\"]\naudiences = [\"web\"]\n";

/// The nearest `AxiomDeps.toml` the compiler would use for `source`.
pub fn find_manifest(source: &Path) -> Option<PathBuf> {
    let absolute = if source.is_absolute() {
        source.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(source)
    };
    let mut dir = absolute.parent();
    while let Some(current) = dir {
        let candidate = current.join("AxiomDeps.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = current.parent();
    }
    None
}

fn looks_like_backend(source: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(source) else {
        return false;
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .any(|l| {
            l.starts_with("project(")
                || l.starts_with("endpoint ")
                || l.starts_with("action ")
                || l.starts_with("variant ")
        })
}

pub fn message(source: &Path) -> String {
    let dir = source
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| ".".into());
    format!(
        "{} looks like a backend contract, but no AxiomDeps.toml was found in {dir} or any parent directory.\n\
Without it the compiler cannot select the backend language and fails with a parse error.\n\n\
Create {dir}/AxiomDeps.toml with:\n\n{}\n\n\
then run the command again.",
        source.display(),
        EXAMPLE
            .lines()
            .map(|l| format!("    {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// Replace a parser error with the missing-manifest explanation when that is the cause.
pub fn explain(source: Option<&Path>, error: anyhow::Error) -> anyhow::Error {
    let Some(source) = source else {
        return error;
    };
    if source.extension().and_then(|e| e.to_str()) != Some("acore") {
        return error;
    }
    let text = format!("{error:#} {error:?}");
    let parse_failure = text.contains("ParseError") || text.contains("Unexpected token");
    if parse_failure && find_manifest(source).is_none() && looks_like_backend(source) {
        return anyhow::anyhow!(message(source));
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_parse_error_only_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("main.acore");
        std::fs::write(&file, "project(id: \"x\", version: \"1.0.0\")\n").unwrap();
        let err = explain(Some(&file), anyhow::anyhow!("ParseError(\"Unexpected token: Colon\")"));
        if find_manifest(&file).is_none() {
            assert!(err.to_string().contains("AxiomDeps.toml"));
        }
        std::fs::write(dir.path().join("AxiomDeps.toml"), EXAMPLE).unwrap();
        let err = explain(Some(&file), anyhow::anyhow!("ParseError(\"Unexpected token: Colon\")"));
        assert!(err.to_string().contains("ParseError"));
    }
}
