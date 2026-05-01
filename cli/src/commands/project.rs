use crate::auth_store;
use crate::components::fuzzy_finder::{render_fuzzy_finder, ProjectItem};
use anyhow::{anyhow, Result};
use axiom_cloud::CloudClient;
use console::style;
use crossterm::event::KeyCode;
use dialoguer::{theme::ColorfulTheme, Input};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use std::env;
use std::path::Path;
use tui_input::backend::crossterm::EventHandler;

pub async fn handle_project_list() -> Result<()> {
    let auth_data = auth_store::load_auth_data()?;
    let client = CloudClient::new(auth_data.access_token);

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

pub async fn handle_project_create(
    name: Option<String>,
    desc: Option<String>,
    path: Option<std::path::PathBuf>,
) -> Result<()> {
    let auth_data = auth_store::load_auth_data()?;
    let client = CloudClient::new(auth_data.access_token);

    // -------------------------------
    // 1. Resolve name (CLI > prompt)
    // -------------------------------
    let project_name = match name {
        Some(n) => n,
        None => Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Project Name")
            .interact_text()?,
    };

    // -------------------------------
    // 2. Resolve description
    // -------------------------------
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

    // -------------------------------
    // 3. Resolve contract path
    // -------------------------------
    let contract_path = match path {
        Some(p) => p,
        None => {
            // default fallback
            std::path::PathBuf::from(".axiom")
        }
    };

    if !contract_path.exists() {
        return Err(anyhow!(
            "Contract file not found at {}",
            contract_path.display()
        ));
    }

    // -------------------------------
    // 4. Extract project slug
    // -------------------------------
    let file_bytes = std::fs::read(&contract_path)?;
    let contract = axiom_lib::unpackager::unpack_axiom_bytes(&file_bytes)?;

    let project_slug = contract.project.project_id.clone();

    if project_slug.is_empty() {
        return Err(anyhow!("Invalid contract: missing project_id"));
    }

    // -------------------------------
    // 5. Create project
    // -------------------------------
    let project = client.create_project(&project_name, &project_slug).await?;

    println!("✅ Project created: {}", style(&project.name).green());

    // -------------------------------
    // 6. Auto-link directory
    // -------------------------------
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
            let auth_data = auth_store::load_auth_data()?;
            let client = CloudClient::new(auth_data.access_token);
            let projects = client.list_projects().await?;
            let items: Vec<ProjectItem> = projects
                .into_iter()
                .map(|p| ProjectItem {
                    id: p.id,
                    name: p.name,
                })
                .collect();

            // Launch TUI Fuzzy Finder
            let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
            tui.enter().map_err(|e| anyhow::anyhow!(e))?;
            let mut input = tui_input::Input::default();
            let mut selected = 0;
            let mut result_id = None;

            loop {
                tui.draw(|f| render_fuzzy_finder(f, f.size(), &input, &items, selected))?;

                if let Some(event) = tui.event_rx.recv().await {
                    match event {
                        crate::tui::Event::Key(key) => match key.code {
                            KeyCode::Enter => {
                                // Logic to get the actual filtered ID
                                let matcher = SkimMatcherV2::default();
                                let filtered: Vec<&ProjectItem> = items
                                    .iter()
                                    .filter(|p| {
                                        input.value().is_empty()
                                            || matcher.fuzzy_match(&p.name, input.value()).is_some()
                                    })
                                    .collect();
                                if let Some(p) = filtered.get(selected) {
                                    result_id = Some(p.id.clone());
                                }
                                break;
                            }
                            KeyCode::Esc => break,
                            KeyCode::Up => selected = selected.saturating_sub(1),
                            KeyCode::Down => selected = selected.saturating_add(1),
                            _ => {
                                input.handle_event(&crossterm::event::Event::Key(key));
                                selected = 0;
                            }
                        },
                        _ => {}
                    }
                }
            }
            let _ = tui.exit().map_err(|e| anyhow::anyhow!(e));
            result_id.ok_or_else(|| anyhow::anyhow!("No project selected"))?
        }
    };

    auth_store::link_project(&current_dir, &pid)?;
    println!("🔗 Linked project {} successfully.", style(pid).cyan());
    Ok(())
}
