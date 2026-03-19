use crate::state::{PullStep, State};
use anyhow::{anyhow, Result};
use axiom_cloud::CloudClient;
use axiom_lib::config::{load_config, AxiomConfig};
use crossterm::event::KeyCode;
use std::path::{Path, PathBuf};

// Import ensure_deps from the build crate
use axiom_build::core::client_sdk::flutter::ensure_deps::ensure_deps;

pub async fn handle_pull_auto(path_arg: Option<PathBuf>) -> Result<()> {
    let mut wizard_local_path: Option<String> = None;

    // 1. Check if configured. If not, run wizard and capture local path if provided.
    if !Path::new("axiom.yaml").exists() {
        wizard_local_path = run_setup_wizard().await?;
    }

    // 2. Load Config (created by wizard or already existing)
    let config = load_config()?;
    let project_root = std::env::current_dir()?;

    // 3. Determine Source and get Bytes
    let contract_bytes = if let Some(proj) = config.project {
        println!("⬇️  Pulling from Axiom Cloud (Project: {})...", proj.id);
        let auth = crate::auth_store::load_auth_data()?;
        let client = CloudClient::new(auth.access_token);
        client.pull_contract_content(&proj.id, "latest").await?
    } else {
        // Use path from: 1. command line arg, 2. wizard input, or 3. default project.axiom
        let raw_path = path_arg
            .map(|p| p.to_string_lossy().into_owned())
            .or(wizard_local_path)
            .unwrap_or_else(|| "project.axiom".to_string());

        let local_path = PathBuf::from(raw_path);
        if !local_path.exists() {
            anyhow::bail!("Local contract file not found at: {}", local_path.display());
        }
        println!("📂 Copying local contract from {}...", local_path.display());
        std::fs::read(&local_path)?
    };

    // 4. Save Artifact to local root (Always named .axiom for internal consistency)
    let axiom_filename = ".axiom";
    let artifact_path = project_root.join(axiom_filename);
    std::fs::write(&artifact_path, &contract_bytes)?;

    // 5. Post-Pull Steps
    let contract = axiom_lib::unpackager::unpack_axiom_bytes(&contract_bytes)?;

    if let Some(fe) = config.frontend {
        if fe.framework == "flutter" {
            println!("📦 Ensuring Flutter dependencies and assets...");
            ensure_deps(&project_root, axiom_filename)?;

            println!("⚙️  Generating Flutter SDK code...");
            axiom_build::core::client_sdk::flutter::generate_from_fbs::generate_from_fbs(
                &project_root,
                &fe,
                contract.schema_fbs.as_deref().unwrap_or(&[]),
                artifact_path.to_str().unwrap(),
            )
            .await?;
        } else {
            println!("⚙️  Codegen for {} is coming soon!", fe.framework);
        }
    }

    println!("✅ axiom pull finished successfully.");
    Ok(())
}

async fn run_setup_wizard() -> Result<Option<String>> {
    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    let mut state = State::new();

    // Pre-load projects
    if let Ok(auth) = crate::auth_store::load_auth_data() {
        let client = CloudClient::new(auth.access_token);
        state.pull_context.available_projects = client
            .list_projects()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect();
    }

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
                KeyCode::Down => match state.pull_context.step {
                    PullStep::SourceSelection => {
                        state.pull_context.source_mode = (state.pull_context.source_mode + 1).min(1)
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
                    }
                    _ => {}
                },
                KeyCode::Char(c) => match state.pull_context.step {
                    PullStep::LocalPathInput => state.pull_context.local_file_path.push(c),
                    PullStep::Success if c == 'q' => break,
                    _ => {}
                },
                KeyCode::Backspace => {
                    if state.pull_context.step == PullStep::LocalPathInput {
                        state.pull_context.local_file_path.pop();
                    }
                }
                KeyCode::Enter => match state.pull_context.step {
                    PullStep::SourceSelection => {
                        if state.pull_context.source_mode == 0 {
                            state.pull_context.step =
                                if state.pull_context.available_projects.is_empty() {
                                    PullStep::FrontendSelection
                                } else {
                                    PullStep::ProjectLink
                                };
                        } else {
                            state.pull_context.step = PullStep::LocalPathInput;
                        }
                    }
                    PullStep::ProjectLink | PullStep::LocalPathInput => {
                        state.pull_context.step = PullStep::FrontendSelection
                    }
                    PullStep::FrontendSelection => {
                        create_axiom_yaml(&state)?;
                        state.pull_context.step = PullStep::Success;
                    }
                    PullStep::Success => break,
                    _ => {}
                },
                _ => {} // Catch-all prevents crashes from unhandled keys
            }
        }
    }

    let path = if state.pull_context.source_mode == 1 {
        Some(state.pull_context.local_file_path.clone())
    } else {
        None
    };
    tui.exit().map_err(|e| anyhow::anyhow!(e))?;
    Ok(path)
}

fn create_axiom_yaml(state: &State) -> Result<()> {
    let mut config = AxiomConfig::default();

    config.frontend = Some(axiom_lib::config::FrontendConfig {
        framework: "flutter".to_string(),
        output_dir: Some("lib/axiom_generated".to_string()),
    });

    if state.pull_context.source_mode == 0 && !state.pull_context.available_projects.is_empty() {
        let (pid, _) =
            &state.pull_context.available_projects[state.pull_context.selected_project_idx];
        config.project = Some(axiom_lib::config::ProjectConfig {
            id: pid.clone(),
            version: "latest".to_string(),
        });
        crate::auth_store::link_project(&std::env::current_dir()?, pid)?;
    }

    let content = serde_yaml::to_string(&config)?;
    std::fs::write("axiom.yaml", content)?;
    Ok(())
}
