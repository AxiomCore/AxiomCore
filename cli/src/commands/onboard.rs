use anyhow::{Context, Result};
use clap::ValueEnum;
use console::style;
use std::path::Path;

use crate::commands::pull::Framework;

#[derive(Debug, Clone, ValueEnum)]
pub enum OnboardRole {
    Backend,
    Frontend,
    Fullstack,
}

impl OnboardRole {
    fn label(&self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Frontend => "frontend",
            Self::Fullstack => "fullstack",
        }
    }
}

/// Give a new repository an intentional, non-magical first path. The command
/// only writes an axiom.acore when `--apply` is supplied. A pull happens only
/// when the caller explicitly gives `--contract`.
pub async fn handle_onboard(
    role: Option<OnboardRole>,
    entrypoint: Option<String>,
    module: Option<String>,
    framework: Option<String>,
    contract: Option<String>,
    apply: bool,
) -> Result<()> {
    let root = std::env::current_dir().context("Could not determine the current directory")?;
    let role = role.unwrap_or_else(|| detect_role(&root));

    println!("{}", style("AxiomCore onboarding").bold());
    println!("  Detected path: {}\n", style(role.label()).cyan());

    if matches!(role, OnboardRole::Backend | OnboardRole::Fullstack) {
        onboard_backend(&root, entrypoint.as_deref(), module.as_deref(), apply).await?;
    }
    if matches!(role, OnboardRole::Frontend | OnboardRole::Fullstack) {
        onboard_frontend(&root, framework.as_deref(), contract.as_deref()).await?;
    }

    println!(
        "\n{}",
        style("Before a release, verify the project:").bold()
    );
    println!("  axiom doctor");
    Ok(())
}

fn detect_role(root: &Path) -> OnboardRole {
    let backend = root.join("axiom.acore").exists()
        || root.join("main.py").exists()
        || root.join("app.py").exists()
        || root.join("pyproject.toml").exists()
        || root.join("go.mod").exists()
        || root.join("main.go").exists();
    let frontend = root.join("package.json").exists()
        || root.join("pubspec.yaml").exists()
        || root.join("AxiomDeps.toml").exists();
    match (backend, frontend) {
        (true, true) => OnboardRole::Fullstack,
        (false, true) => OnboardRole::Frontend,
        _ => OnboardRole::Backend,
    }
}

async fn onboard_backend(
    root: &Path,
    entrypoint_flag: Option<&str>,
    module_flag: Option<&str>,
    apply: bool,
) -> Result<()> {
    let acore = root.join("axiom.acore");
    if acore.exists() {
        println!("{} axiom.acore already exists", style("✓").green());
        println!("  Next: `axiom build` to create an immutable local artifact.");
        println!("  Then: `axiom login` and `axiom build --release` to publish it.\n");
        return Ok(());
    }

    let detected = detect_backend(root);
    let entrypoint = entrypoint_flag
        .map(str::to_string)
        .or_else(|| detected.as_ref().map(|value| value.0.clone()));
    let module = module_flag
        .map(str::to_string)
        .or_else(|| detected.map(|value| value.1));

    match (entrypoint, module) {
        (Some(entrypoint), Some(module)) if apply => {
            crate::commands::init::handle_init(Some(entrypoint.clone()), Some(module.clone()))
                .await?;
            println!(
                "{} Created a contract source for {}.",
                style("✓").green(),
                style(&entrypoint).cyan()
            );
            println!("  Next: `axiom build`, then `axiom login && axiom build --release`.\n");
        }
        (Some(entrypoint), Some(module)) => {
            println!(
                "{} Detected {} using {}.",
                style("•").cyan(),
                entrypoint,
                module
            );
            println!("  Review the plan, then create the contract source with:");
            println!("  {}", style("axiom onboard --role backend --apply").cyan());
            println!("  This creates axiom.acore only; it never uploads a release.\n");
        }
        _ => {
            println!(
                "{} Could not identify a supported backend entrypoint.",
                style("!").yellow()
            );
            println!("  Create one explicitly:");
            println!(
                "  {}",
                style("axiom init main.py:app --module axiom-fastapi").cyan()
            );
            println!("  Supported backend extractors: FastAPI and Go.\n");
        }
    }
    Ok(())
}

async fn onboard_frontend(
    root: &Path,
    framework_flag: Option<&str>,
    contract: Option<&str>,
) -> Result<()> {
    let deps = root.join("AxiomDeps.toml");
    let framework = framework_flag
        .and_then(Framework::from_str)
        .or_else(|| detect_frontend_framework(root));

    if deps.exists() {
        println!("{} AxiomDeps.toml already exists", style("✓").green());
        println!("  Next: `axiom pull` to refresh its configured contract dependencies.\n");
        return Ok(());
    }

    let Some(framework) = framework else {
        println!(
            "{} Could not identify a supported client framework.",
            style("!").yellow()
        );
        println!("  Supply one explicitly, for example:");
        println!(
            "  {}",
            style("axiom onboard --role frontend --framework atmx-web --contract org/project")
                .cyan()
        );
        return Ok(());
    };

    if let Some(contract) = contract {
        println!(
            "{} Pulling {} for {}…",
            style("•").cyan(),
            contract,
            framework.as_str()
        );
        crate::commands::pull::handle_pull(
            Some(contract.to_string()),
            None,
            None,
            Some(framework.as_str().to_string()),
            None,
            None,
        )
        .await?;
        println!(
            "{} Client dependency and generated SDK are ready.\n",
            style("✓").green()
        );
    } else {
        println!("{} Detected {}.", style("•").cyan(), framework.as_str());
        println!("  Pull a release when you have its project reference:");
        println!(
            "  {}",
            style(format!(
                "axiom pull <organization>/<project> --framework {}",
                framework.as_str()
            ))
            .cyan()
        );
        println!("  Or make it a single step with `axiom onboard --role frontend --contract <reference>`.\n");
    }
    Ok(())
}

fn detect_backend(root: &Path) -> Option<(String, String)> {
    if root.join("main.py").is_file() {
        return Some(("main.py:app".to_string(), "axiom-fastapi".to_string()));
    }
    if root.join("app.py").is_file() {
        return Some(("app.py:app".to_string(), "axiom-fastapi".to_string()));
    }
    if root.join("main.go").is_file() {
        return Some(("./main.go".to_string(), "axiom-go".to_string()));
    }
    None
}

fn detect_frontend_framework(root: &Path) -> Option<Framework> {
    if root.join("pubspec.yaml").is_file() {
        return Some(Framework::Flutter);
    }
    let package = std::fs::read_to_string(root.join("package.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&package).ok()?;
    let deps = value
        .get("dependencies")
        .and_then(|value| value.as_object())
        .into_iter()
        .flat_map(|dependencies| dependencies.keys());
    if deps
        .into_iter()
        .any(|name| name == "react" || name == "atmx-react")
    {
        return Some(Framework::AtmxReact);
    }
    Some(Framework::AtmxWeb)
}
