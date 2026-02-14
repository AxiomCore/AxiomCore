pub mod auth_store;
pub mod commands;
pub mod components;
pub mod error_reporter;
pub mod state;
pub mod tui;

use crate::components::build_dashboard::render_build_dashboard;
use crate::components::inspect::endpoint_detail::render_endpoint_detail;
use crate::components::inspect::endpoint_list::render_endpoint_list;
use crate::components::inspect::model_browser::render_model_browser;
use crate::components::login_screen::render_login_screen;
use crate::state::InspectTab;
use axiom_cloud::CloudClient;
use axiom_lib::action::Action; // Correct path
use clap::{Parser, Subcommand};
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "axiom", author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init,
    Login,
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    Install {
        #[arg(long)]
        extractor: Option<String>,
        #[arg(required_unless_present = "extractor")]
        package: Option<String>,
    },
    /// Build the .axiom artifact from local source
    Build {
        #[arg(long)]
        variant: Option<String>,
        /// Skip upload and signing (local build only).
        #[arg(long)]
        local: bool,
    },
    /// Inspect an .axiom file's IR and Policies
    Inspect {
        #[arg(default_value = ".axiom")]
        path: PathBuf,
    },
    Release {
        file_path: PathBuf,
    },
    Pull {
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        package: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
    /// Watch for changes and rebuild/pull automatically
    Watch {
        /// Build a local .axiom file on every change
        #[arg(long)]
        build: bool,
    },
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
}

#[derive(Subcommand)]
enum ProjectAction {
    List,
    Create {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    Link {
        #[arg(long)]
        project_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    Ls, // Removed db_path argument for simplicity - use default
    Get {
        #[arg(long)]
        key: String,
    },
    Clear,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (panic_hook, eyre_hook) = color_eyre::config::HookBuilder::default().into_hooks();
    eyre_hook.install()?;
    std::panic::set_hook(Box::new(move |panic_info| {
        // If we panic, try to disable raw mode so the terminal isn't broken
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
        eprintln!("{}", panic_hook.panic_report(panic_info));
    }));

    let cli = Cli::parse();

    match &cli.command {
        Commands::Init => commands::init::handle_init().await?,
        Commands::Login => handle_login_tui().await?,
        Commands::Cache { action } => {
            let db_path = dirs::home_dir().unwrap().join(".axiom/cache");
            match action {
                CacheAction::Ls => commands::cache::handle_ls(&db_path).await?,
                CacheAction::Get { key } => commands::cache::handle_get(&db_path, key).await?,
                CacheAction::Clear => commands::cache::handle_clear(&db_path).await?,
            }
        }
        Commands::Install { extractor, package } => {
            if let Some(e) = extractor {
                commands::install::handle_install(e).await?;
            }
        }
        Commands::Build { variant, local } => {
            handle_build_command(variant.clone().unwrap_or("default".to_string())).await?;
        }
        Commands::Inspect { path } => {
            handle_inspect(path).await?;
        }
        Commands::Project { action } => match action {
            ProjectAction::List => commands::project::handle_project_list().await?,
            ProjectAction::Create { name, description } => {
                commands::project::handle_project_create(name.clone(), description.clone()).await?
            }
            ProjectAction::Link { project_id } => {
                commands::project::handle_project_link(project_id.clone()).await?
            }
        },
        Commands::Pull { path, .. } => {
            commands::pull::handle_pull_auto(path.clone()).await?;
        }
        Commands::Watch { build } => {
            if *build {
                // Backend Mode (Producer)
                commands::watch::handle_watch_dynamic(true).await?;
            } else {
                // Frontend Mode (Consumer)
                commands::watch::handle_watch_consumer().await?;
            }
        }
        _ => {}
    }

    Ok(())
}

async fn handle_login_tui() -> anyhow::Result<()> {
    let mut state = crate::state::State::new();
    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    tui.enter().map_err(|e| anyhow::anyhow!(e))?;

    // STEP 1: Get the code (Library is now silent!)
    let auth_info = axiom_cloud::CloudClient::start_login().await?;

    // STEP 2: Put the data into the TUI State
    state.login_context.status = crate::state::LoginStatus::WaitingForUser {
        code: auth_info.user_code.clone(),
        url: auth_info.verification_uri.clone(),
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let d_code = auth_info.device_code.clone();
    let d_interval = auth_info.interval;

    // STEP 3: Start polling in background
    let login_task = tokio::spawn(async move {
        let result = axiom_cloud::CloudClient::wait_for_login(&d_code, d_interval).await;
        let _ = tx.send(result);
    });

    let loop_result = async {
        loop {
            if let Ok(res) = rx.try_recv() {
                match res {
                    Ok(token_json) => {
                        // Only update state to Success when we actually have the token
                        let val: serde_json::Value = serde_json::from_str(&token_json)?;
                        crate::auth_store::save_tokens(
                            val["access_token"].as_str().unwrap_or_default(),
                            val["refresh_token"].as_str().unwrap_or_default(),
                        )?;
                        state.login_context.status = crate::state::LoginStatus::Success;
                    }
                    Err(e) => {
                        state.login_context.status =
                            crate::state::LoginStatus::Error(e.to_string());
                    }
                }
            }

            tui.draw(|f| {
                crate::components::login_screen::render_login_screen(f, f.size(), &state)
            })?;

            if let Some(crate::tui::Event::Key(key)) = tui.event_rx.recv().await {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        login_task.abort();
                        return Ok(());
                    }
                    KeyCode::Enter
                        if matches!(
                            state.login_context.status,
                            crate::state::LoginStatus::Success
                        ) =>
                    {
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }
    .await;

    tui.exit().map_err(|e| anyhow::anyhow!(e))?;
    loop_result
}

fn normalize_validator_yaml(raw_spec: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw_spec) {
        serde_yaml::to_string(&value).unwrap_or_else(|_| raw_spec.to_string())
    } else {
        raw_spec.to_string()
    }
}

async fn handle_build_command(variant: String) -> anyhow::Result<()> {
    let mut state = crate::state::State::new();
    let (action_tx, mut action_rx) = tokio::sync::mpsc::unbounded_channel::<Action>();

    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    tui.enter().map_err(|e| anyhow::anyhow!(e))?;

    // 1. We wrap the loop in a block so we can capture the result
    let task_result = async {
        let v_clone = variant.clone();
        let _build_task = tokio::spawn(async move {
            match axiom_build::core::build::handle_build(&v_clone, "", "", Some(action_tx.clone()))
                .await
            {
                Ok(path) => {
                    let _ = action_tx.send(Action::BuildSuccess(path));
                }
                Err(e) => {
                    let _ = action_tx.send(Action::BuildFailed(e.to_string()));
                }
            }
        });

        loop {
            tui.draw(|f| render_build_dashboard(f, f.size(), &state))?;

            // Check for key events
            if let Ok(event) = tui.event_rx.try_recv() {
                if let crate::tui::Event::Key(key) = event {
                    if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                        return Ok(()); // User quit
                    }
                }
            }

            // Process updates from the build thread
            while let Ok(action) = action_rx.try_recv() {
                if let Action::BuildFailed(ref msg) = action {
                    // Stop the loop if build fails so we can show the error or exit
                    return Err(anyhow::anyhow!(msg.clone()));
                }
                if let Action::BuildSuccess(_) = action {
                    // Optionally wait for a keypress or exit automatically
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    return Ok(());
                }
                state.update(action);
            }

            // Short sleep to prevent CPU spiking
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
    .await;

    // 2. ALWAYS call exit, regardless of what task_result is
    tui.exit().map_err(|e| anyhow::anyhow!(e))?;

    // 3. Now handle the result of the build
    if let Err(e) = task_result {
        // Now it's safe to print to the terminal again!
        eprintln!("❌ Build failed: {}", e);
        return Err(e);
    }

    println!("✅ Build Succeeded!");
    Ok(())
}

pub async fn handle_inspect(file_path: &Path) -> anyhow::Result<()> {
    let contract = axiom_build::core::unpackager::unpack_axiom_file(file_path)?;

    let mut state = crate::state::State::new();
    state.inspect_context.contract = Some(contract);

    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    tui.enter().map_err(|e| anyhow::anyhow!(e))?;

    loop {
        tui.draw(|f| {
            let area = f.size();

            // Search Bar at Top
            let chunks = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1), // Bottom Help Bar
            ])
            .split(area);

            // Render Search
            f.render_widget(
                Paragraph::new(format!(" Search: {}_", state.inspect_context.filter_query)).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                ),
                chunks[0],
            );

            // Main Content based on Tab
            match state.inspect_context.active_tab {
                InspectTab::Endpoints => {
                    let body = Layout::horizontal([
                        Constraint::Percentage(40),
                        Constraint::Percentage(60),
                    ])
                    .split(chunks[1]);
                    render_endpoint_list(f, body[0], &state);
                    render_endpoint_detail(f, body[1], &state);
                }
                InspectTab::Models => {
                    render_model_browser(f, chunks[1], &state);
                }
            }

            // Help Bar
            let help = Line::from(vec![Span::raw(
                " [TAB] Switch View  [/] Search  [j/k] Navigate  [q] Quit ",
            )])
            .style(Style::default().bg(Color::Indexed(235)).fg(Color::DarkGray));
            f.render_widget(help, chunks[2]);
        })?;

        if let Some(event) = tui.event_rx.recv().await {
            match event {
                crate::tui::Event::Key(key) => match key.code {
                    KeyCode::Tab => {
                        state.inspect_context.active_tab = match state.inspect_context.active_tab {
                            InspectTab::Endpoints => InspectTab::Models,
                            InspectTab::Models => InspectTab::Endpoints,
                        };

                        state.inspect_context.selected_endpoint_idx = 0;
                        state.inspect_context.selected_model_idx = 0;
                    }
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Up | KeyCode::Char('k') => {
                        state.inspect_context.selected_endpoint_idx = state
                            .inspect_context
                            .selected_endpoint_idx
                            .saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        match state.inspect_context.active_tab {
                            InspectTab::Endpoints => {
                                let count = state
                                    .inspect_context
                                    .contract
                                    .as_ref()
                                    .map(|c| c.endpoints.len())
                                    .unwrap_or(0);
                                state.inspect_context.selected_endpoint_idx =
                                    (state.inspect_context.selected_endpoint_idx + 1)
                                        .min(count.saturating_sub(1));
                            }
                            InspectTab::Models => {
                                if let Some(ref contract) = state.inspect_context.contract {
                                    // IMPORTANT: Use the combined length of models and enums
                                    let total_types =
                                        contract.ir.models.len() + contract.ir.enums.len();
                                    state.inspect_context.selected_model_idx =
                                        (state.inspect_context.selected_model_idx + 1)
                                            .min(total_types.saturating_sub(1));
                                }
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    tui.exit().map_err(|e| anyhow::anyhow!(e))
}
