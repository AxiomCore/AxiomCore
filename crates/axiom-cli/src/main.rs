use axiom_cloud::CloudClient;
use axiom_lib::commands;
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
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // The binary's only job is to parse args and call the library.
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
        } => commands::build::handle_build(variant, entrypoint, target).await,
        Commands::Release { file_path } => {
            // Using `as_deref` to convert &PathBuf to &str for the function signature
            commands::release::handle_release(file_path.to_str().unwrap()).await
        }
    };

    if let Err(e) = result {
        eprintln!("{} {}", style("Error:").red().bold(), e);
        std::process::exit(1);
    }
}
