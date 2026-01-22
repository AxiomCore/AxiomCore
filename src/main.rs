pub mod commands;
use axiom_build::core::build::handle_build;
use axiom_cloud::CloudClient;
use clap::{Parser, Subcommand};
use console::style;
use std::path::PathBuf;

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
    },
    /// Releases a built .axiom artifact to Axiom Cloud.
    Release { file_path: PathBuf },
    /// Pulls a remote Axiom package or a local file.
    Pull {
        /// Pull a specific file from a remote package.
        #[arg(long)]
        path: Option<PathBuf>,

        /// The package name to pull (e.g., 'stripe/official').
        #[arg(required_unless_present = "path")]
        package: Option<String>,

        /// Override the runtime source (URL or local file path).
        #[arg(long)]
        runtime: Option<String>,
    },
    /// Watch an .axiom file and automatically run 'pull' on changes.
    Watch {
        /// Path to the .axiom contract file to watch.
        #[arg(long)]
        path: PathBuf,

        /// Optional override for the runtime source (URL or local file path).
        #[arg(long)]
        runtime: Option<String>,
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
async fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Init => commands::init::handle_init().await,
        Commands::Login => CloudClient::login().await.map(|_| ()),
        Commands::Install { extractor, package } => {
            if let Some(extractor_name) = extractor {
                commands::install::handle_install(extractor_name).await
            } else if let Some(pkg_name) = package {
                // TODO: Implement logic for `axiom install stripe-official/stripe`
                println!("Pulling package '{}'...", pkg_name);
                Ok(())
            } else {
                unreachable!();
            }
        }
        Commands::Build {
            variant,
            entrypoint,
            target,
        } => {
            let variant_val = variant.clone().unwrap_or_else(|| "mobile".to_string());
            let entrypoint_val = entrypoint.clone().unwrap_or_default();
            let target_val = target.clone().unwrap_or_default();

            handle_build(&variant_val, &entrypoint_val, &target_val).await
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
            runtime,
        } => {
            if let Some(file_path) = path {
                // Assuming commands::pull module exists and has handle_pull_path
                commands::pull::handle_pull_path(
                    file_path.to_str().unwrap(),
                    runtime.as_deref(), // Pass the Option<&str>
                )
                .await
            } else if let Some(pkg_name) = package {
                // TODO: Implement logic for `axiom pull stripe/official`
                println!("Pulling package '{}'...", pkg_name);
                Ok(())
            } else {
                unreachable!(); // Should not happen due to required_unless_present
            }
        }
        Commands::Cache { action } => match action {
            CacheAction::Ls { db_path } => commands::cache::handle_ls(db_path).await,
            CacheAction::Get { db_path, key } => commands::cache::handle_get(db_path, key).await,
            CacheAction::Clear { db_path } => commands::cache::handle_clear(db_path).await,
        },
    };

    if let Err(e) = result {
        eprintln!("{} {:?}", style("Error:").red().bold(), e);
        std::process::exit(1);
    }
}
