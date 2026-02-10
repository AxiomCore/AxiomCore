use crate::auth_store;
use anyhow::Result;
use axiom_cloud::CloudClient;
use console::style;
use dialoguer::{theme::ColorfulTheme, Input, Select};
use std::env;

pub async fn handle_project_list() -> Result<()> {
    let base_url =
        env::var("AXIOM_CLOUD_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    // Load tokens (TODO: Implement refresh logic wrapper)
    let auth_data = auth_store::load_auth_data()?;
    let client = CloudClient::new(base_url, auth_data.access_token);

    let projects = client.list_projects().await?;

    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    println!("{}", style(" Projects:").bold());
    for p in projects {
        println!("  • {} ({})", style(p.name).cyan(), style(p.id).dim());
    }

    Ok(())
}

pub async fn handle_project_create(name: Option<String>, desc: Option<String>) -> Result<()> {
    let base_url =
        env::var("AXIOM_CLOUD_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let auth_data = auth_store::load_auth_data()?;
    let client = CloudClient::new(base_url, auth_data.access_token);

    let project_name = match name {
        Some(n) => n,
        None => Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Project Name")
            .interact_text()?,
    };

    let _project_desc = match desc {
        Some(d) => Some(d),
        None => {
            let d: String = Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Description (optional)")
                .allow_empty(true)
                .interact_text()?;
            if d.is_empty() {
                None
            } else {
                Some(d)
            }
        }
    };

    let project = client.create_project(&project_name).await?;
    println!("✅ Project created: {}", style(&project.name).green(),);

    // Auto-link to current directory?
    let current_dir = env::current_dir()?;
    auth_store::link_project(&current_dir, &project.id)?;
    println!(
        "🔗 Linked project to current directory: {}",
        style(current_dir.display()).dim()
    );

    Ok(())
}

pub async fn handle_project_link(project_id: Option<String>) -> Result<()> {
    let current_dir = env::current_dir()?;

    let pid = match project_id {
        Some(id) => id,
        None => {
            // Interactive selection
            let base_url =
                env::var("AXIOM_CLOUD_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
            let auth_data = auth_store::load_auth_data()?;
            let client = CloudClient::new(base_url, auth_data.access_token);

            let projects = client.list_projects().await?;
            if projects.is_empty() {
                anyhow::bail!("No projects found. Create one with 'axiom project create'.");
            }

            let selection = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Select a project to link")
                .default(0)
                .items(
                    &projects
                        .iter()
                        .map(|p| format!("{} ({})", p.name, p.id))
                        .collect::<Vec<_>>(),
                )
                .interact()?;

            projects[selection].id.clone()
        }
    };

    auth_store::link_project(&current_dir, &pid)?;
    println!(
        "🔗 Linked project {} to {}",
        style(pid).cyan(),
        style(current_dir.display()).dim()
    );
    Ok(())
}
