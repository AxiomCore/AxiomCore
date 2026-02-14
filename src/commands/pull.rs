use crate::state::{PullStep, State};
use anyhow::{anyhow, Result};
use axiom_cloud::CloudClient;
use axiom_lib::config::{load_config, AxiomConfig};
use crossterm::event::KeyCode;
use std::path::{Path, PathBuf};

pub async fn handle_pull_auto(path_arg: Option<PathBuf>) -> Result<()> {
    // 1. Check if configured
    if !Path::new("axiom.yaml").exists() {
        run_setup_wizard().await?;
    }

    // 2. Load Config
    let config = load_config()?;
    let project_root = std::env::current_dir()?;

    // 3. Determine Source
    let contract_bytes = if let Some(proj) = config.project {
        // Remote Pull
        println!("⬇️  Pulling from Axiom Cloud (Project: {})...", proj.id);
        let auth = crate::auth_store::load_auth_data()?;
        let client = CloudClient::new(auth.access_token);
        client.pull_contract_content(&proj.id, "latest").await?
    } else {
        // Local File Copy
        // We look for a .axiom file in the root if no path provided
        let local_path = path_arg.unwrap_or_else(|| PathBuf::from("project.axiom"));
        if !local_path.exists() {
            anyhow::bail!("Local contract file not found at: {}", local_path.display());
        }
        std::fs::read(&local_path)?
    };

    // 4. Save Artifact
    let artifact_path = project_root.join("project.axiom");
    std::fs::write(&artifact_path, &contract_bytes)?;

    // 5. Run Generator
    let contract = axiom_build::core::unpackager::unpack_axiom_bytes(&contract_bytes)?;

    if let Some(fe) = config.frontend {
        println!("⚙️  Generating {} SDK...", fe.framework);
        // Note: You might need to expose generate_from_fbs logic publicly from axiom-build
        axiom_build::core::client_sdk::flutter::generate_from_fbs::generate_from_fbs(
            &project_root,
            &fe,
            contract.schema_fbs.as_deref().unwrap_or(&[]),
            artifact_path.to_str().unwrap(),
        )
        .await?;
    }

    println!("✅ Pull complete.");
    Ok(())
}

async fn run_setup_wizard() -> Result<()> {
    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    let mut state = State::new();

    // Pre-load projects if logged in
    let auth = crate::auth_store::load_auth_data().ok();
    let projects = if let Some(a) = auth {
        let client = CloudClient::new(a.access_token);
        client
            .list_projects()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect()
    } else {
        vec![]
    };
    state.pull_context.available_projects = projects;

    tui.enter().map_err(|e| anyhow::anyhow!(e))?;

    loop {
        tui.draw(|f| crate::components::pull_wizard::render_pull_wizard(f, f.size(), &state))?;

        if let Some(crate::tui::Event::Key(key)) = tui.event_rx.recv().await {
            match key.code {
                KeyCode::Esc => {
                    tui.exit().map_err(|e| anyhow::anyhow!(e))?;
                    return Err(anyhow!("Setup cancelled"));
                }
                KeyCode::Up => match state.pull_context.step {
                    PullStep::SourceSelection => {
                        state.pull_context.source_mode =
                            state.pull_context.source_mode.saturating_sub(1)
                    }
                    PullStep::ProjectLink => {
                        state.pull_context.selected_project_idx =
                            state.pull_context.selected_project_idx.saturating_sub(1)
                    }
                    PullStep::FrontendSelection => {
                        state.pull_context.selected_framework =
                            state.pull_context.selected_framework.saturating_sub(1)
                    }
                    _ => {}
                },
                KeyCode::Down => {
                    match state.pull_context.step {
                        PullStep::SourceSelection => {
                            state.pull_context.source_mode =
                                (state.pull_context.source_mode + 1).min(1)
                        }
                        PullStep::ProjectLink => {
                            state.pull_context.selected_project_idx =
                                (state.pull_context.selected_project_idx + 1).min(
                                    state
                                        .pull_context
                                        .available_projects
                                        .len()
                                        .saturating_sub(1),
                                )
                        }
                        PullStep::FrontendSelection => {
                            state.pull_context.selected_framework =
                                (state.pull_context.selected_framework + 1).min(0)
                        } // Only Flutter enabled
                        _ => {}
                    }
                }
                KeyCode::Char(c) if state.pull_context.step == PullStep::LocalPathInput => {
                    state.pull_context.local_file_path.push(c);
                }
                KeyCode::Backspace if state.pull_context.step == PullStep::LocalPathInput => {
                    state.pull_context.local_file_path.pop();
                }
                KeyCode::Enter => {
                    match state.pull_context.step {
                        PullStep::SourceSelection => {
                            if state.pull_context.source_mode == 0 {
                                // Remote
                                if state.pull_context.available_projects.is_empty() {
                                    // Todo: handle login prompt
                                    state.pull_context.step = PullStep::FrontendSelection;
                                } else {
                                    state.pull_context.step = PullStep::ProjectLink;
                                }
                            } else {
                                state.pull_context.step = PullStep::LocalPathInput;
                            }
                        }
                        PullStep::ProjectLink => {
                            state.pull_context.step = PullStep::FrontendSelection
                        }
                        PullStep::LocalPathInput => {
                            state.pull_context.step = PullStep::FrontendSelection
                        }
                        PullStep::FrontendSelection => {
                            // Generate config
                            create_axiom_yaml(&state)?;
                            state.pull_context.step = PullStep::Success;
                        }
                        PullStep::Success => break,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    tui.exit().map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

fn create_axiom_yaml(state: &State) -> Result<()> {
    let mut config = AxiomConfig::default();

    // Set Frontend
    config.frontend = Some(axiom_lib::config::FrontendConfig {
        framework: "flutter".to_string(),
        output_dir: Some("lib/axiom_generated".to_string()),
    });

    // Set Project if Remote
    if state.pull_context.source_mode == 0 && !state.pull_context.available_projects.is_empty() {
        let (pid, _) =
            &state.pull_context.available_projects[state.pull_context.selected_project_idx];
        config.project = Some(axiom_lib::config::ProjectConfig {
            id: pid.clone(),
            version: "latest".to_string(),
        });

        // Also link locally
        let current = std::env::current_dir()?;
        crate::auth_store::link_project(&current, pid)?;
    }

    let content = serde_yaml::to_string(&config)?;
    std::fs::write("axiom.yaml", content)?;
    Ok(())
}
