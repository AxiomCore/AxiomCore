pub mod commands;
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
        variant: String,
        #[arg(long)]
        entrypoint: String,
        #[arg(long)]
        target: String,
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
        } => commands::build::build(variant, entrypoint, target).await,
        Commands::Release { file_path } => {
            commands::release::handle_release(file_path.to_str().unwrap()).await
        }
        Commands::Pull { path, package } => {
            if let Some(file_path) = path {
                // Assuming commands::pull module exists and has handle_pull_path
                commands::pull::handle_pull_path(file_path.to_str().unwrap()).await
            } else if let Some(pkg_name) = package {
                // TODO: Implement logic for `axiom pull stripe/official`
                println!("Pulling package '{}'...", pkg_name);
                Ok(())
            } else {
                unreachable!(); // Should not happen due to required_unless_present
            }
        }
    };

    if let Err(e) = result {
        eprintln!("{} {}", style("Error:").red().bold(), e);
        std::process::exit(1);
    }
}
