// src/main.rs
pub mod access_config;
pub mod auth_store;
pub mod commands;
pub mod components;
pub mod error_reporter;
pub mod state;
pub mod telemetry;
pub mod tui;

use crate::access_config::AccessConfig;
use crate::components::build_dashboard::render_build_dashboard;
use crate::components::inspect::endpoint_detail::render_endpoint_detail;
use crate::components::inspect::endpoint_list::render_endpoint_list;
use crate::components::inspect::model_browser::render_model_browser;
use crate::state::InspectTab;
use crate::telemetry::Telemetry;
use axiom_cloud::{CliApi, CloudClient};
use axiom_lib::action::Action;
use clap::{Parser, Subcommand};
use console::style;
use crossterm::event::KeyCode;
use dialoguer::{theme::ColorfulTheme, Input};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use std::path::{Path, PathBuf};
use std::time::Instant;

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
    /// Join the waitlist if you don't have a referral code
    Join {
        email: String,
    },
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    Install {
        package: String,
        /// Installs an Acore/Axiom compiler extractor module
        #[arg(long)]
        module: bool,
    },
    Eval {
        file: PathBuf,
        #[arg(short, long)]
        format: Option<String>,
        #[arg(short, long)]
        variant: Option<String>,
    },
    Test {
        /// Optional path to an .acore file. If omitted, uses axiom.acore
        file: Option<PathBuf>,
        /// Only run test suites that match this tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// Start a local API Mock Server from your contract
    Serve {
        /// Optional path to an .acore file. If omitted, pulls configuration from Axiom Cloud.
        file: Option<PathBuf>,
        /// Port to bind the server to
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },
    /// Start the Acore REPL
    Repl,
    /// Start the Axiom/Acore Language Server
    Lsp,
    /// Build the .axiom artifact from local source
    Build {
        /// Path to the .acore configuration file
        file: PathBuf,
        #[arg(long)]
        variant: Option<String>,
        /// Automatically release the contract to Axiom Cloud after building
        #[arg(long)]
        release: bool,
    },
    /// Inspect an .axiom file's IR and Policies
    Inspect {
        #[arg(default_value = ".axiom")]
        path: PathBuf,
    },
    Release {
        /// Path to the .axiom file (Defaults to .axiom in the current directory)
        file_path: Option<PathBuf>,
    },
    Pull {
        #[arg(long)]
        contract: Option<PathBuf>,
        #[arg(long)]
        contract_config: Option<PathBuf>,
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
    Deploy {
        #[command(subcommand)]
        target: DeployTarget,
    },
    /// Diff Acore files to see what changed
    Diff {
        /// The main acore file (or the old file if file2 is provided)
        file1: PathBuf,

        /// An optional second file to compare against. If omitted, diffs against the lockfile.
        file2: Option<PathBuf>,

        /// Output format: text, semantic, atom, or changelog (default)
        #[arg(long, default_value = "changelog")]
        format: String,

        /// Specify the variant to evaluate before diffing
        #[arg(long)]
        variant: Option<String>,
    },
}

#[derive(Subcommand)]
enum DeployTarget {
    MockServer {
        /// Optional path to the .acore file. If omitted, uses axiom.acore in current directory.
        file: Option<PathBuf>,
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
    /// Resolves local acore project dependencies
    Resolve {
        dir: PathBuf,
    },
    /// Packages a local acore project
    Package {
        dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    Ls,
    Get {
        #[arg(long)]
        key: String,
    },
    Clear,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Setup Error Hooks
    let (panic_hook, eyre_hook) = color_eyre::config::HookBuilder::default().into_hooks();
    eyre_hook.install()?;
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
        eprintln!("{}", panic_hook.panic_report(panic_info));
    }));

    let cli = Cli::parse();
    let start_time = Instant::now();

    // 2. GATEKEEPER LOGIC (Private Alpha Check)
    // -------------------------------------------------------------------------

    // Allow 'join' command to pass through without a referral code
    if let Commands::Join { email } = &cli.command {
        return commands::join::handle_join(email.clone()).await;
    }

    // Load Access Config
    let mut access_config = AccessConfig::load().await?;

    // If no config exists, force registration
    if access_config.is_none() {
        println!(
            "{}",
            style("🔒 Axiom CLI is currently in Private Alpha.")
                .bold()
                .yellow()
        );
        println!("To proceed, you need a valid referral code.\n");
        let referral_from_env = std::env::var("AXIOM_REFERRAL_CODE").ok();

        let code = if let Some(env_code) = referral_from_env {
            println!("Using referral code from environment variable.");
            env_code
        } else {
            // If running in CI and no referral provided, fail cleanly
            if std::env::var("CI").is_ok() {
                eprintln!("❌ No AXIOM_REFERRAL_CODE provided in CI environment.");
                std::process::exit(1);
            }

            println!(
                "If you don't have one, run: {}",
                style("axiom join <EMAIL>").cyan()
            );
            println!("");

            Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Enter Referral Code")
                .interact_text()?
        };

        // Create temp config to generate machine ID
        let temp_config = AccessConfig::save(&code).await?;

        println!("{}", style("Verifying code...").dim());

        // Register with Server
        match CliApi::register(&temp_config.referral_code, &temp_config.machine_id).await {
            Ok(_) => {
                println!(
                    "✅ {}",
                    style("Access Granted. Welcome to Axiom.").green().bold()
                );
                println!("");
                access_config = Some(temp_config);
            }
            Err(e) => {
                // Registration failed, wipe local config so they try again next time
                let _ = AccessConfig::wipe().await;
                println!(
                    "\n❌ {}",
                    style(format!("Authorization Failed: {}", e)).red()
                );
                std::process::exit(1);
            }
        }
    }

    let active_config = access_config.unwrap();

    // 3. EXECUTE COMMAND
    // -------------------------------------------------------------------------
    let result = execute_command(&cli.command).await;

    // 4. TELEMETRY
    // -------------------------------------------------------------------------
    let duration = start_time.elapsed();
    let (success, error_msg) = match &result {
        Ok(_) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };

    // Extract command name for logging
    let cmd_name = match &cli.command {
        Commands::Init => "init",
        Commands::Login => "login",
        Commands::Join { .. } => "join",
        Commands::Cache { .. } => "cache",
        Commands::Install { .. } => "install",
        Commands::Build { .. } => "build",
        Commands::Inspect { .. } => "inspect",
        Commands::Release { .. } => "release",
        Commands::Pull { .. } => "pull",
        Commands::Watch { .. } => "watch",
        Commands::Project { .. } => "project",
        Commands::Diff { .. } => "diff",
        Commands::Serve { .. } => "serve",
        Commands::Deploy { .. } => "deploy",
        Commands::Test { .. } => "test",
        Commands::Eval {
            file,
            format,
            variant,
        } => "eval",
        Commands::Repl => "repl",
        Commands::Lsp => "lsp",
    };

    // Send Telemetry (This internally handles the "Kill Switch" / Access Revocation)
    Telemetry::track(
        &active_config,
        cmd_name,
        std::env::args().collect(),
        duration,
        success,
        error_msg,
    )
    .await;

    result
}

// Helper to route commands (Refactored from original main)
async fn execute_command(command: &Commands) -> anyhow::Result<()> {
    match command {
        Commands::Test { file, tag } => {
            commands::test::handle_test(file.clone(), tag.clone()).await
        }
        Commands::Serve { file, port } => commands::serve::handle_serve(file.clone(), *port).await,
        Commands::Deploy { target } => match target {
            DeployTarget::MockServer { file } => {
                let path = file
                    .clone()
                    .unwrap_or_else(|| std::path::PathBuf::from("axiom.acore"));
                commands::deploy::handle_deploy_mock_server(path).await
            }
        },
        Commands::Repl => {
            tokio::task::spawn_blocking(|| {
                acore::repl::run_repl();
            })
            .await
            .unwrap();
            Ok(())
        }
        Commands::Lsp => {
            acore::server::run_server().await;
            Ok(())
        }
        Commands::Eval {
            file,
            format,
            variant,
        } => {
            let mut evaluator =
                acore::evaluator::Evaluator::new(acore::security::SecurityManager::allow_all());
            evaluator.active_variant = variant.clone();

            let uri = format!("file://{}", file.canonicalize().unwrap().display());
            let val = evaluator.evaluate_module(&uri)?;

            let out_fmt = match format.as_deref().unwrap_or("pcf").to_lowercase().as_str() {
                "json" => acore::render::OutputFormat::Json,
                "yaml" => acore::render::OutputFormat::Yaml,
                "md" | "markdown" => acore::render::OutputFormat::Markdown,
                _ => acore::render::OutputFormat::Pcf,
            };

            if let Some(out) = acore::render::process_outputs(&mut evaluator, &val, out_fmt, None)?
            {
                println!("{}", out);
            }
            Ok(())
        }
        Commands::Install { package, module } => {
            if *module {
                acore::package::install_tool(package)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                Ok(())
            } else {
                println!("Marketplace installation coming soon for '{}'", package);
                Ok(())
            }
        }

        Commands::Build {
            file,
            variant,
            release,
        } => {
            let variant_str = variant.clone().unwrap_or("default".to_string());
            let file_path = file.to_string_lossy().to_string();

            if std::env::var("CI").is_ok() {
                match axiom_build::core::build::handle_build(&file_path, &variant_str, "", "", None)
                    .await
                {
                    Ok(out_file) => {
                        println!("✅ Build Succeeded! Generated {}", out_file);
                        let lockfile_path = format!("{}.lockfile", file_path);
                        if let Ok(content) = std::fs::read_to_string(&file_path) {
                            let _ = std::fs::write(&lockfile_path, content);
                        }
                        if *release {
                            commands::release::handle_release(&out_file).await?;
                        }
                        Ok(())
                    }
                    Err(e) => {
                        eprintln!("❌ Build failed: {}", e);
                        Err(e.into())
                    }
                }
            } else {
                handle_build_command(file_path, variant_str, *release).await
            }
        }
        Commands::Release { file_path } => {
            // Default to ".axiom" in current directory if no path is passed
            let path = file_path
                .clone()
                .unwrap_or_else(|| std::path::PathBuf::from(".axiom"));
            commands::release::handle_release(path.to_str().unwrap()).await
        }
        Commands::Inspect { path } => handle_inspect(path).await,
        Commands::Project { action } => match action {
            ProjectAction::List => commands::project::handle_project_list().await,
            ProjectAction::Create { name, description } => {
                commands::project::handle_project_create(name.clone(), description.clone()).await
            }
            ProjectAction::Link { project_id } => {
                commands::project::handle_project_link(project_id.clone()).await
            }
            ProjectAction::Resolve { dir } => {
                acore::project_cmd::resolve_project(dir.to_str().unwrap())
                    .map_err(|e| anyhow::anyhow!(e.to_string()))
            }
            ProjectAction::Package { dir } => {
                acore::project_cmd::package_project(dir.to_str().unwrap())
                    .map_err(|e| anyhow::anyhow!(e.to_string()))
            }
        },
        Commands::Pull {
            contract,
            contract_config,
        } => commands::pull::handle_pull_auto(contract.clone(), contract_config.clone()).await,
        Commands::Watch { build } => {
            if *build {
                commands::watch::handle_watch_dynamic(true).await
            } else {
                commands::watch::handle_watch_consumer().await
            }
        }
        Commands::Diff {
            file1,
            file2,
            format,
            variant,
        } => {
            crate::commands::diff::handle_diff(
                file1.clone(),
                file2.clone(),
                format.clone(),
                variant.clone(),
            )
            .await
        }
        Commands::Init => todo!(),
        Commands::Login => todo!(),
        Commands::Join { email } => todo!(),
        Commands::Cache { action } => todo!(),
    }
}

// =========================================================================================
//  EXISTING HELPERS (Kept exactly as original to ensure full file correctness)
// =========================================================================================

async fn handle_login_tui() -> anyhow::Result<()> {
    let mut state = crate::state::State::new();
    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    tui.enter().map_err(|e| anyhow::anyhow!(e))?;

    // STEP 1: Get the code (Library is now silent!)
    let auth_info = CloudClient::start_login().await?;

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
        let result = CloudClient::wait_for_login(&d_code, d_interval).await;
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

async fn handle_build_command(
    file_path: String,
    variant: String,
    release: bool,
) -> anyhow::Result<()> {
    let mut state = crate::state::State::new();
    let (action_tx, mut action_rx) = tokio::sync::mpsc::unbounded_channel::<Action>();

    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    tui.enter().map_err(|e| anyhow::anyhow!(e))?;

    // Capture the generated path from the task_result
    let task_result: anyhow::Result<String> = async {
        let v_clone = variant.clone();
        let f_clone = file_path.clone();

        tokio::spawn(async move {
            match axiom_build::core::build::handle_build(
                &f_clone,
                &v_clone,
                "",
                "",
                Some(action_tx.clone()),
            )
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

            if let Ok(event) = tui.event_rx.try_recv() {
                if let crate::tui::Event::Key(key) = event {
                    if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                        return Err(anyhow::anyhow!("Build cancelled by user."));
                    }
                }
            }

            while let Ok(action) = action_rx.try_recv() {
                if let Action::BuildFailed(ref msg) = action {
                    return Err(anyhow::anyhow!(msg.clone()));
                }
                if let Action::BuildSuccess(path) = action {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    return Ok(path);
                }
                state.update(action);
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
    .await;

    tui.exit().map_err(|e| anyhow::anyhow!(e))?;

    // Process the result after TUI exits
    match task_result {
        Ok(output_filename) => {
            println!("✅ Build Succeeded! Generated: {}", output_filename);

            let lockfile_path = format!("{}.lockfile", file_path);
            let mut eval =
                acore::evaluator::Evaluator::new(acore::security::SecurityManager::allow_all());
            if let Ok(val) = eval.evaluate_module(&format!(
                "file://{}",
                std::fs::canonicalize(&file_path).unwrap().display()
            )) {
                // Render it to JSON
                if let Ok(json_output) =
                    acore::render::render_value(&mut eval, &val, acore::render::OutputFormat::Json)
                {
                    if let Err(e) = std::fs::write(&lockfile_path, json_output) {
                        eprintln!("⚠️ Failed to write lockfile: {}", e);
                    }
                }
            }

            // Trigger release if flag was passed
            if release {
                println!("\n🚀 Initiating Release...");
                crate::commands::release::handle_release(&output_filename).await?;
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("❌ Build failed: {}", e);
            Err(e)
        }
    }
}

pub async fn handle_inspect(file_path: &Path) -> anyhow::Result<()> {
    let contract = axiom_lib::unpackager::unpack_axiom_file(file_path)?;

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
