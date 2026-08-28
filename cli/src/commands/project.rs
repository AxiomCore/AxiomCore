use crate::auth_store;
use anyhow::{anyhow, Context, Result};
use console::style;
use dialoguer::{theme::ColorfulTheme, Input, Select};
use std::{
    io::IsTerminal,
    path::{Path, PathBuf},
};

use axiom_cloud::{project::Project, CloudClient};

pub async fn handle_project_list() -> Result<()> {
    let client = authenticated_client()?;
    let projects = client.list_projects().await?;

    if projects.is_empty() {
        println!("No projects found. Run `axiom project create` to create one.");
        return Ok(());
    }

    println!("{}", style(" Projects:").bold());
    for project in projects {
        println!(
            "  • {}  {}",
            style(project.name).cyan(),
            style(project.slug).dim()
        );
    }
    Ok(())
}

pub async fn handle_project_create(
    name: Option<String>,
    slug: Option<String>,
    description: Option<String>,
    path: Option<PathBuf>,
) -> Result<()> {
    let client = authenticated_client()?;
    let suggested_slug = artifact_slug_hint(path.as_deref()).unwrap_or_else(directory_slug_hint);
    let project = create_project_interactively(
        &client,
        name.as_deref(),
        slug.as_deref(),
        description.as_deref(),
        &suggested_slug,
    )
    .await?;
    link_current_directory(&project)?;
    println!("✅ Project created: {}", style(&project.name).green());
    println!("🔗 This directory is now linked to `{}`.", project.slug);
    Ok(())
}

pub async fn handle_project_link(project_id: Option<String>) -> Result<()> {
    let client = authenticated_client()?;
    let projects = client.list_projects().await?;
    let project = match project_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(identifier) => find_project(&projects, identifier).cloned().ok_or_else(|| {
            anyhow!(
                "No project matches `{identifier}`. Run `axiom project list` to inspect your available projects."
            )
        })?,
        None => choose_or_create_project(&client, &projects, &directory_slug_hint()).await?,
    };

    link_current_directory(&project)?;
    println!(
        "🔗 Linked this directory to `{}` successfully.",
        style(project.slug).cyan()
    );
    Ok(())
}

/// Resolve the release destination. A saved directory link is authoritative;
/// a first-time interactive release presents an explicit create-or-select
/// choice and persists it immediately for subsequent release commands.
pub async fn resolve_release_project(
    client: &CloudClient,
    project_override: Option<&str>,
    artifact_project_id: &str,
) -> Result<String> {
    let projects = client
        .list_projects()
        .await
        .map_err(|error| anyhow!("Could not load projects for this release: {error}"))?;
    let current_dir =
        std::env::current_dir().context("Could not determine the release directory")?;

    let project = if let Some(identifier) = project_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        find_project(&projects, identifier).cloned().ok_or_else(|| {
            anyhow!(
                "No project matches `{identifier}`. `--project` accepts a project ID or slug; run `axiom project list` to inspect available projects."
            )
        })?
    } else if let Some(linked_id) = auth_store::get_project_id(&current_dir)? {
        match find_project(&projects, &linked_id).cloned() {
            Some(project) => project,
            None => {
                eprintln!(
                    "⚠️  The project previously linked to this directory is no longer available. Choose a new destination."
                );
                choose_or_create_for_release(client, &projects, artifact_project_id).await?
            }
        }
    } else {
        choose_or_create_for_release(client, &projects, artifact_project_id).await?
    };

    auth_store::link_project(&current_dir, &project.id)?;
    println!("🔗 Release destination: {}", style(&project.slug).cyan());
    Ok(project.slug)
}

pub async fn handle_project_rotate_key(project_slug: String) -> Result<()> {
    let client = authenticated_client()?;
    let key = client.rotate_project_key(&project_slug).await?;
    println!(
        "✅ Rotated signing key for {}: {} ({})",
        style(project_slug).cyan(),
        style(key.id).green(),
        key.algorithm
    );
    println!("   Previous public keys remain available to verify older contract releases.");
    Ok(())
}

fn authenticated_client() -> Result<CloudClient> {
    auth_store::authenticated_cloud_client().map_err(|error| {
        anyhow!(
            "You are not logged in to Axiom Cloud. Run `axiom login`, then retry this command. Details: {error}"
        )
    })
}

async fn choose_or_create_for_release(
    client: &CloudClient,
    projects: &[Project],
    artifact_project_id: &str,
) -> Result<Project> {
    choose_or_create_project(client, projects, &normalize_slug(artifact_project_id)).await
}

async fn choose_or_create_project(
    client: &CloudClient,
    projects: &[Project],
    suggested_slug: &str,
) -> Result<Project> {
    ensure_interactive("Axiom needs a release destination")?;
    let mut choices = Vec::with_capacity(projects.len() + 1);
    choices.push("＋ Create a new project".to_string());
    choices.extend(projects.iter().map(|project| {
        let description = project.description.trim();
        if description.is_empty() {
            format!("{}  ({})", project.name, project.slug)
        } else {
            format!("{}  ({}) — {}", project.name, project.slug, description)
        }
    }));

    let selected = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select a project for this directory")
        .items(&choices)
        .default(0)
        .interact_opt()
        .map_err(|error| anyhow!("Could not select a project: {error}"))?
        .ok_or_else(|| anyhow!("Project selection cancelled."))?;

    if selected == 0 {
        return create_project_interactively(client, None, None, None, suggested_slug).await;
    }

    Ok(projects[selected - 1].clone())
}

async fn create_project_interactively(
    client: &CloudClient,
    name_override: Option<&str>,
    slug_override: Option<&str>,
    description_override: Option<&str>,
    suggested_slug: &str,
) -> Result<Project> {
    ensure_interactive("Creating a project")?;
    let default_name = display_name_from_slug(suggested_slug);
    let name = prompt_required("Project name", name_override, &default_name)?;
    let slug_default = normalize_slug(slug_override.unwrap_or(&name));
    let slug = prompt_required("Project slug", slug_override, &slug_default)?;
    if !is_valid_slug(&slug) {
        anyhow::bail!(
            "Project slug `{slug}` is invalid. Use lowercase letters, numbers, and single hyphens."
        );
    }

    let description = match description_override {
        Some(value) => value.trim().to_string(),
        None => Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Project description (optional)")
            .allow_empty(true)
            .interact_text()
            .map_err(|error| anyhow!("Could not read project description: {error}"))?
            .trim()
            .to_string(),
    };

    client
        .create_project(&name, &slug, &description)
        .await
        .map_err(|error| anyhow!("Could not create project `{slug}`: {error}"))
}

fn prompt_required(label: &str, override_value: Option<&str>, default: &str) -> Result<String> {
    let value = match override_value {
        Some(value) => value.trim().to_string(),
        None => Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt(label)
            .default(default.to_string())
            .interact_text()
            .map_err(|error| anyhow!("Could not read {label}: {error}"))?
            .trim()
            .to_string(),
    };
    if value.is_empty() {
        anyhow::bail!("{label} is required.");
    }
    Ok(value)
}

fn find_project<'a>(projects: &'a [Project], identifier: &str) -> Option<&'a Project> {
    projects
        .iter()
        .find(|project| project.id == identifier || project.slug == identifier)
}

fn link_current_directory(project: &Project) -> Result<()> {
    let current_dir =
        std::env::current_dir().context("Could not determine the current directory")?;
    auth_store::link_project(&current_dir, &project.id)
}

fn artifact_slug_hint(path: Option<&Path>) -> Option<String> {
    let path = path?;
    let bytes = std::fs::read(path).ok()?;
    let contract = axiom_lib::unpackager::unpack_axiom_bytes(&bytes).ok()?;
    let slug = normalize_slug(&contract.project.project_id);
    (!slug.is_empty()).then_some(slug)
}

fn directory_slug_hint() -> String {
    let directory = std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|value| value.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "axiom-project".to_string());
    normalize_slug(&directory)
}

fn ensure_interactive(action: &str) -> Result<()> {
    if !std::io::stdin().is_terminal() || std::env::var_os("CI").is_some() {
        anyhow::bail!(
            "{action}, but this shell is non-interactive. Run `axiom project link --project-id <id-or-slug>` first, or pass `--project <id-or-slug>` to the release command."
        );
    }
    Ok(())
}

fn normalize_slug(value: &str) -> String {
    let mut result = String::new();
    let mut previous_was_separator = true;
    for character in value.trim().chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            result.push('-');
            previous_was_separator = true;
        }
    }
    result.trim_matches('-').to_string()
}

fn is_valid_slug(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn display_name_from_slug(slug: &str) -> String {
    slug.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), characters.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{display_name_from_slug, is_valid_slug, normalize_slug};

    #[test]
    fn project_slug_hints_are_safe_for_the_management_api() {
        assert_eq!(normalize_slug("Py_Example API!"), "py-example-api");
        assert!(is_valid_slug("py-example-api"));
        assert!(!is_valid_slug("py--example"));
    }

    #[test]
    fn project_name_defaults_are_readable() {
        assert_eq!(display_name_from_slug("py-example-api"), "Py Example Api");
    }
}
