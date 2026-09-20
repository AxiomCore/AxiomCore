use anyhow::{Context, Result};
use console::style;
use serde::Serialize;
use std::path::Path;
use std::process::Command;

use crate::commands::pull::{read_axiom_deps, Framework};

/// A deterministic, non-mutating environment report. `doctor` deliberately
/// never performs a login, pulls an artifact, or writes project files: it is
/// safe to run in a new checkout and in CI.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub schema_version: u8,
    pub control_plane: String,
    pub project_root: String,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub id: &'static str,
    pub status: DoctorStatus,
    pub summary: String,
    pub detail: String,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}

impl DoctorStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

pub async fn handle_doctor(json: bool, strict: bool) -> Result<()> {
    let report = collect_report()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render_report(&report);
    }

    if strict
        && report
            .checks
            .iter()
            .any(|check| check.status == DoctorStatus::Fail)
    {
        anyhow::bail!("Axiom doctor found required checks that need attention.");
    }
    Ok(())
}

pub fn collect_report() -> Result<DoctorReport> {
    let root = std::env::current_dir().context("Could not determine the current directory")?;
    let control_plane = axiom_cloud::cloud_base_url()?;
    let mut checks = Vec::new();

    checks.push(check(
        "cli",
        DoctorStatus::Pass,
        format!("Axiom CLI {} is available", env!("CARGO_PKG_VERSION")),
        "The current executable is running normally.".to_string(),
        None,
    ));
    checks.push(check_control_plane(&control_plane));
    checks.push(check_auth());

    let acore_path = root.join("axiom.acore");
    let artifact_path = root.join("axiom.axiom");
    let deps_path = root.join("AxiomDeps.toml");
    checks.push(check_contract_source(&acore_path));
    checks.push(check_artifact(&artifact_path));

    let deps_framework = match check_deps(&deps_path) {
        Ok((check, framework)) => {
            checks.push(check);
            framework
        }
        Err(check) => {
            checks.push(check);
            None
        }
    };

    checks.push(check_project_link(&root));
    checks.extend(check_local_tools(&root, deps_framework.as_ref()));

    Ok(DoctorReport {
        schema_version: 1,
        control_plane,
        project_root: root.display().to_string(),
        checks,
    })
}

fn check(
    id: &'static str,
    status: DoctorStatus,
    summary: String,
    detail: String,
    fix: Option<String>,
) -> DoctorCheck {
    DoctorCheck {
        id,
        status,
        summary,
        detail,
        fix,
    }
}

fn check_control_plane(control_plane: &str) -> DoctorCheck {
    let local = axiom_cloud::uses_local_cloud().unwrap_or(false);
    check(
        "control_plane",
        DoctorStatus::Pass,
        if local {
            "Using an isolated local AxiomCore profile".to_string()
        } else {
            "Using the AxiomCore production control plane".to_string()
        },
        control_plane.to_string(),
        None,
    )
}

fn check_auth() -> DoctorCheck {
    match crate::auth_store::load_auth_data_if_present() {
        Ok(Some(auth)) if !auth.access_token.trim().is_empty() && !auth.refresh_token.trim().is_empty() => check(
            "authentication",
            DoctorStatus::Pass,
            "Axiom Cloud session is available".to_string(),
            "The session is stored in the active Axiom profile; credentials are not shown by doctor.".to_string(),
            None,
        ),
        Ok(_) | Err(_) => check(
            "authentication",
            DoctorStatus::Warn,
            "No Axiom Cloud session is configured".to_string(),
            "Local builds and local artifact pulls work without a Cloud session. Releases and private-project pulls require one.".to_string(),
            Some("Run `axiom login` before `axiom build --release` or a private `axiom pull`.".to_string()),
        ),
    }
}

fn check_contract_source(path: &Path) -> DoctorCheck {
    if path.is_file() {
        check(
            "contract_source",
            DoctorStatus::Pass,
            "Found axiom.acore".to_string(),
            path.display().to_string(),
            None,
        )
    } else {
        check(
            "contract_source",
            DoctorStatus::Warn,
            "No axiom.acore found in this directory".to_string(),
            "This is expected for a frontend-only consumer project.".to_string(),
            Some("For a backend project, run `axiom onboard --role backend --apply`.".to_string()),
        )
    }
}

fn check_artifact(path: &Path) -> DoctorCheck {
    if !path.exists() {
        return check(
            "artifact",
            DoctorStatus::Warn,
            "No built axiom.axiom artifact found".to_string(),
            "Build an artifact before releasing or use AxiomDeps.toml in a consumer project."
                .to_string(),
            Some("Run `axiom build` after creating axiom.acore.".to_string()),
        );
    }
    match axiom_lib::unpackager::unpack_axiom_file(path) {
        Ok(contract) => check(
            "artifact",
            DoctorStatus::Pass,
            format!(
                "Artifact is readable: {} {}",
                contract.project.project_id, contract.project.version
            ),
            format!(
                "{} endpoint(s); {} bytes",
                contract.endpoints.len(),
                file_size(path)
            ),
            None,
        ),
        Err(error) => check(
            "artifact",
            DoctorStatus::Fail,
            "axiom.axiom cannot be decoded".to_string(),
            error.to_string(),
            Some("Rebuild with `axiom build`; do not hand-edit compiled artifacts.".to_string()),
        ),
    }
}

fn check_deps(path: &Path) -> std::result::Result<(DoctorCheck, Option<Framework>), DoctorCheck> {
    if !path.exists() {
        return Ok((
            check(
                "dependencies",
                DoctorStatus::Warn,
                "No AxiomDeps.toml found".to_string(),
                "This is expected for a backend-only project.".to_string(),
                Some(
                    "For a client project, run `axiom pull <organization>/<project>`.".to_string(),
                ),
            ),
            None,
        ));
    }
    match read_axiom_deps(path) {
        Ok(deps) => {
            let contracts = deps.contracts.len();
            let framework = deps.framework.clone();
            Ok((
                check(
                    "dependencies",
                    DoctorStatus::Pass,
                    format!("AxiomDeps.toml targets {}", framework.as_str()),
                    format!("{} configured contract dependency/dependencies", contracts),
                    None,
                ),
                Some(framework),
            ))
        }
        Err(error) => Err(check(
            "dependencies",
            DoctorStatus::Fail,
            "AxiomDeps.toml is invalid".to_string(),
            error.to_string(),
            Some(
                "Repair the TOML or rerun `axiom pull` with the intended contract source."
                    .to_string(),
            ),
        )),
    }
}

fn check_project_link(root: &Path) -> DoctorCheck {
    match crate::auth_store::get_project_id_if_present(root) {
        Ok(Some(project)) => check(
            "project_link",
            DoctorStatus::Pass,
            "This directory is linked to an Axiom Cloud project".to_string(),
            format!("Stored project identifier: {project}"),
            None,
        ),
        Ok(None) | Err(_) => check(
            "project_link",
            DoctorStatus::Warn,
            "This directory is not linked to an Axiom Cloud project".to_string(),
            "The first interactive release lets you create or select a project and saves the link."
                .to_string(),
            Some("Run `axiom project link` or `axiom build --release`.".to_string()),
        ),
    }
}

fn check_local_tools(root: &Path, framework: Option<&Framework>) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let has_python_source = root.join("pyproject.toml").exists()
        || root.join("requirements.txt").exists()
        || root.join("main.py").exists()
        || root.join("app.py").exists();
    let has_go_source = root.join("go.mod").exists() || root.join("main.go").exists();
    let has_node_source = root.join("package.json").exists();

    if has_python_source {
        checks.push(check_tool_any(
            "python",
            &["python3", "python"],
            "Python backend source detected",
            "Install Python 3 and the backend dependencies before building a FastAPI contract.",
        ));
    }
    if has_go_source {
        checks.push(check_tool_any(
            "go",
            &["go"],
            "Go backend source detected",
            "Install Go before building a Go contract.",
        ));
    }
    if has_node_source || matches!(framework, Some(Framework::AtmxWeb | Framework::AtmxReact)) {
        checks.push(check_tool_any(
            "node",
            &["node"],
            "Web client generation needs Node.js",
            "Install a current Node.js LTS release, then rerun `axiom pull`.",
        ));
        checks.push(check_tool_any(
            "npm",
            &["npm"],
            "Web client generation needs npm",
            "Install npm with Node.js, then rerun `axiom pull`.",
        ));
    }
    if matches!(framework, Some(Framework::Flutter)) || root.join("pubspec.yaml").exists() {
        checks.push(check_tool_any(
            "flutter",
            &["flutter"],
            "Flutter client source detected",
            "Install Flutter and run `flutter pub get` before generating a Flutter client.",
        ));
    }
    checks
}

fn check_tool_any(id: &'static str, candidates: &[&str], summary: &str, fix: &str) -> DoctorCheck {
    if let Some(tool) = candidates
        .iter()
        .copied()
        .find(|tool| command_available(tool))
    {
        check(
            id,
            DoctorStatus::Pass,
            format!("{summary}: {tool} is available"),
            tool_version(tool),
            None,
        )
    } else {
        check(
            id,
            DoctorStatus::Fail,
            format!("{summary}, but the required tool is missing"),
            format!("Looked for {} on PATH.", candidates.join(" or ")),
            Some(fix.to_string()),
        )
    }
}

fn command_available(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn tool_version(tool: &str) -> String {
    Command::new(tool)
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .or_else(|| {
            Command::new(tool)
                .arg("--version")
                .output()
                .ok()
                .and_then(|output| String::from_utf8(output.stderr).ok())
        })
        .and_then(|output| output.lines().next().map(str::trim).map(str::to_string))
        .filter(|output| !output.is_empty())
        .unwrap_or_else(|| format!("{tool} is on PATH"))
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn render_report(report: &DoctorReport) {
    println!("{}", style("AxiomCore doctor").bold());
    println!("  Project: {}", report.project_root);
    println!("  Control plane: {}\n", report.control_plane);

    for check in &report.checks {
        let label = match check.status {
            DoctorStatus::Pass => style(format!("✓ {}", check.status.label())).green(),
            DoctorStatus::Warn => style(format!("! {}", check.status.label())).yellow(),
            DoctorStatus::Fail => style(format!("✗ {}", check.status.label())).red(),
        };
        println!("{label}  {}", check.summary);
        println!("       {}", style(&check.detail).dim());
        if let Some(fix) = &check.fix {
            println!("       {} {}", style("Fix:").cyan(), fix);
        }
    }
    println!("\nMachine-readable output: `axiom doctor --json`");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_mode_only_treats_failures_as_blocking() {
        assert_ne!(DoctorStatus::Warn, DoctorStatus::Fail);
        assert_eq!(DoctorStatus::Pass.label(), "PASS");
    }
}
