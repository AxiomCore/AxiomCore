pub mod auth_store;
pub mod commands;
use axiom_build::core::build::handle_build;
use axiom_cloud::CloudClient;
use axiom_extractor::evaluate_acore_config;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "axiom", author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initializes a new AxiomCore project.
    Init,
    /// Authenticates with Axiom Cloud.
    Login,
    /// Inspect or clear the Axiom runtime cache.
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Install an extractor plugin or a remote axiom package
    Install {
        /// Install a language extractor (e.g., 'fastapi', 'spring')
        #[arg(long)]
        extractor: Option<String>,

        /// The package name if pulling from registry (e.g. 'stripe/official')
        #[arg(required_unless_present = "extractor")]
        package: Option<String>,
    },
    /// Build the .axiom artifact from local source
    Build {
        #[arg(long)]
        variant: Option<String>,
        #[arg(long)]
        entrypoint: Option<String>,
        #[arg(long)]
        target: Option<String>,
        /// Skip upload and signing (local build only).
        #[arg(long)]
        local: bool,
    },
    /// Releases a built .axiom artifact to Axiom Cloud.
    Release { file_path: PathBuf },
    /// Pulls a remote Axiom package or a local file.
    Pull {
        /// Pull a specific file from a remote package.
        #[arg(long)]
        path: Option<PathBuf>,

        /// The package name/project ID to pull (e.g., 'stripe/official').
        /// If omitted, tries to use the linked project.
        #[arg(long)]
        package: Option<String>,

        /// Specific version to pull (e.g., 'v1.0.0'). Defaults to latest.
        #[arg(long)]
        version: Option<String>,

        /// Override the runtime source (URL or local file path).
        #[arg(long)]
        runtime: Option<String>,
    },
    /// Watch an .axiom file and automatically run 'pull' on changes.
    /// Watch an .axiom file and automatically run 'pull' on changes.
    Watch {
        /// Path to the .axiom contract file to watch.
        #[arg(long)]
        path: PathBuf,

        /// Optional override for the runtime source (URL or local file path).
        #[arg(long)]
        runtime: Option<String>,
    },
    /// Manage your Axiom projects.
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
}

#[derive(Subcommand)]
enum ProjectAction {
    /// List all your projects.
    List,
    /// Create a new project.
    Create {
        /// The name of the new project.
        #[arg(long)]
        name: Option<String>,
        /// Optional description for the project.
        #[arg(long)]
        description: Option<String>,
    },
    /// Link the current directory to an existing project.
    Link {
        /// The Project ID to link. If omitted, prompts for selection.
        #[arg(long)]
        project_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    /// List all keys in a given cache database.
    Ls {
        #[arg(long)]
        db_path: PathBuf,
    },
    /// Show the value for a specific cache key.
    Get {
        #[arg(long)]
        db_path: PathBuf,
        #[arg(long)] // <-- ADD THIS for consistency
        key: String,
    },
    /// Clear the entire cache database.
    Clear {
        #[arg(long)]
        db_path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Init => commands::init::handle_init().await,
        Commands::Login => {
            let base_url = std::env::var("AXIOM_CLOUD_URL")
                .unwrap_or_else(|_| "http://localhost:8080".to_string());
            // Login returns single string currently, need to update CloudClient::login or handle it
            // perform_device_flow returns just access token string in original code
            // But we updated it to return Result<String> in auth.rs... wait
            // Let's check auth.rs again. I updated `perform_device_flow` to return String?
            // No, I updated `refresh_access_token` to return `TokenResponse`.
            // I need to update `perform_device_flow` to return `TokenResponse` serialized or struct.

            // Re-reading auth.rs changes... I only updated `refresh_access_token`.
            // `perform_device_flow` still returns `Result<String>` (access token only).
            // I need to update `perform_device_flow` to return `TokenResponse` so we get refresh token.

            // For now, let's fix the call assuming I will fix `CloudClient::login` next.
            let token_json = CloudClient::login(&base_url).await?;
            // `perform_device_flow` returns JSON string of TokenResponse now (I edited it to serde_json::to_string)
            // So deserializing it here
            let val: serde_json::Value = serde_json::from_str(&token_json)?;
            let access = val["access_token"].as_str().unwrap_or_default();
            let refresh = val["refresh_token"].as_str().unwrap_or_default();

            auth_store::save_tokens(access, refresh)?;
            println!("Token saved to config.");
            Ok(())
        }
        Commands::Install { extractor, package } => {
            if let Some(extractor_name) = extractor {
                commands::install::handle_install(extractor_name).await
            } else if let Some(pkg_name) = package {
                // TODO: Implement logic for `axiom install stripe-official/stripe`
                println!("Installing package '{}'...", pkg_name);
                Ok(())
            } else {
                unreachable!();
            }
        }
        Commands::Build {
            variant,
            entrypoint,
            target,
            local,
        } => {
            let variant_val = variant.clone().unwrap_or_else(|| "mobile".to_string());
            let entrypoint_val = entrypoint.clone().unwrap_or_default();
            let target_val = target.clone().unwrap_or_default();

            // 1. Build the artifact
            let output_path = handle_build(&variant_val, &entrypoint_val, &target_val).await?;

            if *local {
                Ok(())
            } else {
                // 2. Upload and Sign
                println!("🚀 Uploading to Axiom Cloud...");

                // Load tokens and check for linked project
                let mut auth_data = auth_store::load_auth_data()?;
                let current_dir = std::env::current_dir()?;
                let abs_path = std::fs::canonicalize(&current_dir).unwrap_or(current_dir.clone());

                let project_id = match auth_data.projects.get(&abs_path) {
                    Some(id) => id.clone(),
                    None => {
                        println!("⚠️  No project linked to this directory.");
                        // Interactive flow
                        let base_url = std::env::var("AXIOM_CLOUD_URL")
                            .unwrap_or_else(|_| "http://localhost:8080".to_string());
                        let client = CloudClient::with_auth(
                            base_url,
                            auth_data.access_token.clone(),
                            auth_data.refresh_token.clone(),
                            auth_store::get_auth_file_path().ok(),
                        );

                        let projects = client.list_projects().await?;

                        // Select or Create logic
                        // We use the same logic as `axiom project link` but inline here for smoothness
                        // Or better, call `commands::project::handle_project_link(None)` but that returns ()
                        // Let's reuse the selection logic from `commands::project::handle_project_link` by modifying it to return the ID
                        // For now, let's duplicate/inline simple selection to avoid signature churn if we want quick results.

                        use dialoguer::{theme::ColorfulTheme, Select};

                        let mut items = projects
                            .iter()
                            .map(|p| format!("{} ({})", p.name, p.id))
                            .collect::<Vec<_>>();
                        items.push("➕ Create New Project".to_string());

                        let selection = Select::with_theme(&ColorfulTheme::default())
                            .with_prompt("Select a project to link")
                            .default(0)
                            .items(&items)
                            .interact()?;

                        let pid = if selection < projects.len() {
                            projects[selection].id.clone()
                        } else {
                            // Create new
                            commands::project::handle_project_create(None, None).await?;
                            // The create handler links it, so re-read or just trust it?
                            // The handler links it to CWD.
                            auth_data = auth_store::load_auth_data()?;
                            auth_data
                                .projects
                                .get(&abs_path)
                                .cloned()
                                .ok_or_else(|| anyhow::anyhow!("Failed to link new project"))?
                        };

                        // Link it if we selected existing (Create already linked it)
                        if selection < projects.len() {
                            auth_store::link_project(&abs_path, &pid)?;
                        }

                        pid
                    }
                };

                // Get version from axiom.acore (still needed for versioning the artifact)
                let config = evaluate_acore_config("axiom.acore")?;
                let project_config = config
                    .project
                    .ok_or_else(|| anyhow::anyhow!("Missing project config in axiom.acore"))?;

                let base_url = std::env::var("AXIOM_CLOUD_URL")
                    .unwrap_or_else(|_| "http://localhost:8080".to_string());
                // usage of refreshed token if needed should happen inside CloudClient or here
                // For now assuming token is valid or we add retry logic later
                let client = CloudClient::with_auth(
                    base_url,
                    auth_data.access_token,
                    auth_data.refresh_token,
                    auth_store::get_auth_file_path().ok(),
                );

                let metadata = client
                    .upload_contract(
                        &project_id,
                        &project_config.version,
                        Path::new(&output_path),
                    )
                    .await?;

                println!("✅ Uploaded. Contract ID: {}", metadata.id);
                println!("✍️  Signing contract...");

                client.sign_contract(&metadata.id).await?;
                println!("✅ Signed and released!");

                Ok(())
            }
        }
        Commands::Release { file_path } => {
            commands::release::handle_release(file_path.to_str().unwrap()).await
        }
        Commands::Watch { path, runtime } => {
            // NEW: Call the new watch handler
            commands::watch::handle_watch(path, runtime.as_deref()).await
        }
        Commands::Pull {
            path,
            package,
            version,
            runtime,
        } => {
            if let Some(file_path) = path {
                commands::pull::handle_pull_path(
                    file_path.to_str().unwrap(),
                    runtime.as_deref(), // Pass the Option<&str>
                )
                .await
            } else {
                // Determine project ID (package or linked or interactive)
                let project_id = if let Some(pkg) = package {
                    pkg.clone()
                } else {
                    // Check linked project
                    let auth_data = auth_store::load_auth_data()?;
                    let current_dir = std::env::current_dir()?;
                    let abs_path =
                        std::fs::canonicalize(&current_dir).unwrap_or(current_dir.clone());

                    match auth_data.projects.get(&abs_path) {
                        Some(id) => id.clone(),
                        None => {
                            println!("⚠️  No project linked to this directory.");
                            // Interactive selection (reusing logic from Build command, should probably refactor)
                            let base_url = std::env::var("AXIOM_CLOUD_URL")
                                .unwrap_or_else(|_| "http://localhost:8080".to_string());
                            let client = CloudClient::with_auth(
                                base_url,
                                auth_data.access_token.clone(),
                                auth_data.refresh_token.clone(),
                                auth_store::get_auth_file_path().ok(),
                            );

                            let projects = client.list_projects().await?;
                            use dialoguer::{theme::ColorfulTheme, Select};
                            // Select from list
                            let items = projects
                                .iter()
                                .map(|p| format!("{} ({})", p.name, p.id))
                                .collect::<Vec<_>>();

                            let selection = Select::with_theme(&ColorfulTheme::default())
                                .with_prompt("Select a project to pull from")
                                .default(0)
                                .items(&items)
                                .interact()?;

                            let pid = projects[selection].id.clone();
                            // Link it
                            auth_store::link_project(&abs_path, &pid)?;
                            println!("🔗 Linked project: {}", pid);
                            pid
                        }
                    }
                };

                commands::pull::handle_pull_package(
                    &project_id,
                    version.as_deref(),
                    runtime.as_deref(),
                )
                .await
            }
        }
        Commands::Project { action } => match action {
            ProjectAction::List => commands::project::handle_project_list().await,
            ProjectAction::Create { name, description } => {
                commands::project::handle_project_create(name.clone(), description.clone()).await
            }
            ProjectAction::Link { project_id } => {
                commands::project::handle_project_link(project_id.clone()).await
            }
        },
        Commands::Cache { action } => match action {
            CacheAction::Ls { db_path } => commands::cache::handle_ls(db_path).await,
            CacheAction::Get { db_path, key } => commands::cache::handle_get(db_path, key).await,
            CacheAction::Clear { db_path } => commands::cache::handle_clear(db_path).await,
        },
    };

    // Result is directly returned or bubbled up now
    result
}
