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
use axiom_cloud::{uses_local_cloud, CliApi, CloudClient};
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
    Init {
        /// Entrypoint path (e.g., main.py:app)
        entrypoint: Option<String>,
        /// The Acore compiler module to use (e.g., axiom-fastapi)
        #[arg(long)]
        module: Option<String>,
    },
    /// Inspect local prerequisites without changing files, logging in, or pulling a contract
    Doctor {
        /// Emit a stable machine-readable report for CI or support tickets
        #[arg(long)]
        json: bool,
        /// Return a non-zero exit status when a required prerequisite fails
        #[arg(long)]
        strict: bool,
    },
    /// Detect a repository's role and print or apply the shortest supported first path
    Onboard {
        #[arg(long, value_enum)]
        role: Option<commands::onboard::OnboardRole>,
        /// Backend entrypoint, for example main.py:app
        #[arg(long)]
        entrypoint: Option<String>,
        /// Extractor module, for example axiom-fastapi
        #[arg(long)]
        module: Option<String>,
        /// Client target: flutter, dart, atmx-web, or atmx-react
        #[arg(long)]
        framework: Option<String>,
        /// Contract source to pull for a frontend path
        #[arg(long)]
        contract: Option<String>,
        /// Create the detected backend axiom.acore; no release is uploaded
        #[arg(long)]
        apply: bool,
    },
    /// Measure local contract build and artifact-load boundaries reproducibly
    Benchmark {
        /// The Acore contract source
        #[arg(default_value = "axiom.acore")]
        file: String,
        #[arg(long)]
        variant: Option<String>,
        #[arg(long, default_value_t = 10)]
        iterations: usize,
        #[arg(long, default_value_t = 2)]
        warmup: usize,
        #[arg(long, default_value = "axiom-benchmark.json")]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Login,
    /// Join the waitlist if you don't have a referral code
    Join {
        email: String,
    },
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Install an intact .axiomapp locally or an Acore compiler module
    Install {
        package: String,
        /// Installs an Acore/Axiom compiler extractor module
        #[arg(long)]
        module: bool,
    },
    /// Combine target-specific .axiomapp builds into one multi-target artifact
    Package {
        #[arg(required = true)]
        artifacts: Vec<PathBuf>,
        #[arg(short, long, default_value = "dist/application.axiomapp")]
        out: PathBuf,
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
        /// Enable verbose debug logging for incoming requests and responses
        #[arg(short, long)]
        debug: bool,
    },
    /// Start the Acore REPL
    Repl,
    /// Start the Axiom/Acore Language Server
    Lsp,
    /// Build the .axiom artifact from local source
    Build {
        /// The Acore file to build (defaults to axiom.acore if not provided)
        #[arg(default_value = "axiom.acore")]
        file: String,

        #[arg(long)]
        variant: Option<String>,

        /// Release the compiled contract to Axiom Cloud
        #[arg(long)]
        release: bool,

        /// Optional Cloud project ID or slug override. By default the linked
        /// directory project is used automatically.
        #[arg(long)]
        project: Option<String>,

        /// Optional immutable Cloud release version. Interactive releases
        /// suggest the next available version on a collision.
        #[arg(long)]
        version: Option<String>,

        /// The branch to deploy to (defaults to main)
        #[arg(long, default_value = "main")]
        branch: String,

        /// Deployment commit message
        #[arg(short, long, default_value = "CLI Deployment")]
        message: String,
    },
    /// Inspect an application workspace or a legacy .axiom/.axiomapp artifact
    Inspect {
        #[command(subcommand)]
        action: Option<commands::inspector::InspectorAction>,
        /// Legacy artifact path; omitted when using an Inspector subcommand
        path: Option<PathBuf>,
    },
    Release {
        /// Path to the .axiom file (defaults to axiom.axiom in the current directory)
        file_path: Option<PathBuf>,

        /// Optional Cloud project ID or slug override. A linked directory is
        /// resolved automatically when this is omitted.
        #[arg(long)]
        project: Option<String>,

        /// Optional immutable Cloud release version override.
        #[arg(long)]
        version: Option<String>,
    },
    Pull {
        /// Local .axiom artifact, AxiomDeps.toml/JSON config, AxiomCore URL,
        /// or organization/project[/version] contract reference.
        source: Option<String>,

        /// Explicit local artifact, config, URL, or contract reference.
        #[arg(long)]
        contract: Option<String>,

        #[arg(long)]
        contract_config: Option<PathBuf>,

        #[arg(long)]
        framework: Option<String>,

        #[arg(long)]
        name: Option<String>,

        #[arg(short, long)]
        out: Option<String>,
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
    /// Resolve and verify signed contract artifacts for a future Acore UI app
    Contract {
        #[command(subcommand)]
        action: ContractAction,
    },
    /// Resolve, verify, inspect, diff, and source-check frontend .axiom packages
    Packages {
        #[command(subcommand)]
        action: PackagesAction,
    },
    /// Build, sign, verify, inspect, and review sandboxed extensions
    Extensions {
        #[command(subcommand)]
        action: ExtensionsAction,
    },
    /// Check an Acore UI module and lower it only into an in-memory graph
    Ui {
        #[command(subcommand)]
        action: UiAction,
    },
    /// Run a backend service/mock, frontend UI session, or packaged .axiomapp
    Run {
        /// Backend/frontend .acore source or a packaged .axiomapp
        source: PathBuf,
        /// Advanced UI lock override; normal development locks are prepared automatically
        #[arg(long)]
        lock: Option<PathBuf>,
        /// Acore defaults to ios; multi-target .axiomapp files prompt when omitted
        #[arg(long)]
        target: Option<String>,
        /// Backend execution mode; defaults to service for backend sources
        #[arg(long, value_enum)]
        mode: Option<commands::run::RunMode>,
        /// Backend host address
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Backend service or mock port
        #[arg(short, long, default_value = "8080")]
        port: u16,
        /// Enable verbose backend mock diagnostics
        #[arg(short, long)]
        debug: bool,
        /// Require reviewed locks and extension workflows without development regeneration
        #[arg(long)]
        frozen: bool,
        /// Compile once and exit; intended for CI and host-independent checks
        #[arg(long)]
        once: bool,
    },
    /// Validate or bootstrap the optional Domain Model v1 layer
    Domain {
        #[command(subcommand)]
        action: DomainAction,
    },
    /// Inspect opt-in security policy, source evidence, and coverage
    Security {
        #[command(subcommand)]
        action: SecurityAction,
    },
}

#[derive(Subcommand)]
enum DomainAction {
    /// Validate domain references and print the canonical manifest hash
    Validate {
        #[arg(default_value = "axiom.acore")]
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Create a reviewable domain block from OpenAPI JSON or exported SQL DDL
    Bootstrap {
        input: PathBuf,
        #[arg(long, value_enum)]
        source: commands::domain::DomainSource,
        #[arg(long, default_value = "axiom.domain.acore")]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
enum SecurityAction {
    /// Analyze an Acore contract without producing an artifact
    Check {
        #[arg(default_value = "axiom.acore")]
        file: PathBuf,
        #[arg(long)]
        json: bool,
        /// Treat warnings as a non-zero result, suitable for a review-only CI job
        #[arg(long)]
        fail_on_warning: bool,
    },
}

#[derive(Subcommand)]
enum ContractAction {
    /// Resolve AxiomDeps.toml contract artifacts into a deterministic lockfile
    Resolve {
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
    },
    /// Recompute and check every AxiomDeps.toml lock input without changing files
    Verify {
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
    },
    /// Show semantic changes between two UI contract locks
    Diff { before: PathBuf, after: PathBuf },
    /// Print read-only metadata for one future virtual facade
    Facade {
        alias: String,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
    },
    /// Validate `use contract` declarations against only a committed lock
    CheckSource {
        source: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
    },
}

#[derive(Subcommand)]
enum PackagesAction {
    /// Validate and canonically encode a typed package envelope
    Build {
        source: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Resolve frontend package dependencies into a canonical committed lock
    Resolve {
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        #[arg(long, default_value = "AxiomPackages.lock")]
        lock: PathBuf,
    },
    /// Re-resolve and verify all locked package bytes and proofs
    Verify {
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        #[arg(long, default_value = "AxiomPackages.lock")]
        lock: PathBuf,
    },
    /// Print one verified locked package envelope
    Inspect {
        alias: String,
        #[arg(long, default_value = "AxiomPackages.lock")]
        lock: PathBuf,
    },
    /// Print a semantic package diff and enforce explicit review approvals
    Diff {
        before: PathBuf,
        after: PathBuf,
        #[arg(long = "approve")]
        approvals: Vec<String>,
    },
    /// Validate package imports and compile the unchanged source for one target
    CheckSource {
        source: PathBuf,
        #[arg(long, default_value = "AxiomPackages.lock")]
        package_lock: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        ui_lock: PathBuf,
        #[arg(long, default_value = "ios")]
        target: String,
    },
    /// Run the package-backed source through the normal target host/session
    Run {
        source: PathBuf,
        #[arg(long, default_value = "AxiomPackages.lock")]
        package_lock: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        ui_lock: PathBuf,
        #[arg(long, default_value = "ios")]
        target: String,
        #[arg(long)]
        once: bool,
    },
}

#[derive(Subcommand)]
enum ExtensionsAction {
    /// Compile one Rust source module registered in AxiomDeps.toml into verified core WASM
    SourceBuild {
        /// Registered AxiomDeps.toml extension alias
        alias: String,
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        /// Application-local generated artifact/cache directory
        #[arg(long, default_value = ".axiom/extensions")]
        out: PathBuf,
        #[arg(long, default_value = "web")]
        target: String,
        /// Ignore a matching verified build cache entry
        #[arg(long)]
        clean: bool,
    },
    /// Build and sign one registered Rust source module into a local release workflow
    SourceRelease {
        /// Registered AxiomDeps.toml extension alias
        alias: String,
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        /// Application-local generated artifact/cache directory
        #[arg(long, default_value = ".axiom/extensions")]
        out: PathBuf,
        /// Root workflow consumed by ordinary `axiom run` commands
        #[arg(long, default_value = "AxiomExtensions.toml")]
        workflow: PathBuf,
        #[arg(long)]
        application: String,
        #[arg(long, default_value = "0.1.0")]
        application_version: String,
        #[arg(long)]
        clean: bool,
    },
    /// Validate and canonically encode an extension package draft
    Build {
        source: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Sign a canonical package and emit a detached proof document
    Sign {
        artifact: PathBuf,
        #[arg(long)]
        key: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Resolve local extension packages into the committed package lock
    Resolve {
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        #[arg(long, default_value = "AxiomPackages.lock")]
        lock: PathBuf,
    },
    /// Verify signed packages, authority inputs, locks, targets, and provenance offline
    Verify {
        #[arg(long, default_value = "AxiomExtensions.toml")]
        manifest: PathBuf,
    },
    /// Explain one extension identity, provenance, and authority
    Inspect {
        alias: String,
        #[arg(long, default_value = "AxiomExtensions.toml")]
        manifest: PathBuf,
    },
    /// Show requested, granted, and target-effective authority
    Permissions {
        alias: String,
        #[arg(long, default_value = "AxiomExtensions.toml")]
        manifest: PathBuf,
    },
    /// Show extension dependencies and transitive origins
    Graph {
        #[arg(long, default_value = "AxiomExtensions.toml")]
        manifest: PathBuf,
    },
    /// Diff two canonical authority locks and enforce explicit increases
    Diff {
        before: PathBuf,
        after: PathBuf,
        #[arg(long = "approve")]
        approvals: Vec<String>,
    },
    /// Run deterministic offline release conformance checks
    Test {
        #[arg(long, default_value = "AxiomExtensions.toml")]
        manifest: PathBuf,
    },
    /// Verify and emit the exact descriptor handed to the selected target host
    Run {
        alias: String,
        #[arg(long, default_value = "AxiomExtensions.toml")]
        manifest: PathBuf,
        #[arg(long, default_value = "server")]
        target: String,
        /// Execute one verified export in the server reference host. Omit to
        /// inspect the selected target handoff without running guest code.
        #[arg(long)]
        export: Option<String>,
        /// JSON ABI input supplied to --export.
        #[arg(long, default_value = "null")]
        input: String,
        /// Optional deterministic UI/store fixture for a server reference
        /// invocation. Guest code never receives a native state reference.
        #[arg(long)]
        state: Option<PathBuf>,
        /// Write canonical, redacted correlated execution evidence to this
        /// explicit path in addition to the readable terminal report.
        #[arg(long)]
        audit_out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum UiAction {
    /// Install or inspect the target UI Host used by Axiom run
    Host {
        #[command(subcommand)]
        action: UiHostAction,
    },
    /// Create a small authored Acore UI starter without generated source
    Init {
        /// Empty directory that will become the application root
        directory: PathBuf,
        #[arg(long, default_value = "ios")]
        target: String,
    },
    /// Parse, validate, and virtually lower a UI module without writing generated source
    Check {
        source: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
        #[arg(long, default_value = "ios")]
        target: String,
    },
    /// Start the virtual compiler/watch loop. A native transport is required for device rendering.
    Run {
        source: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
        #[arg(long, default_value = "ios")]
        target: String,
        #[arg(long)]
        once: bool,
    },
    /// Print a read-only compiler view tied to the current virtual graph revision
    Inspect {
        #[arg(value_enum)]
        view: commands::ui::UiInspectView,
        source: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
        #[arg(long, default_value = "ios")]
        target: String,
        /// Maximum semantic symbols for `context`; this is not a token budget
        #[arg(long, default_value_t = 24)]
        max_symbols: usize,
    },
    /// Return compiler-checked compact, redacted semantic context for AI tooling
    Context {
        source: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
        #[arg(long, default_value = "ios")]
        target: String,
        #[arg(long, default_value_t = 24)]
        max_symbols: usize,
    },
    /// Validate an AI-proposed replacement without writing it to the workspace
    AiCheck {
        source: PathBuf,
        proposed: PathBuf,
        /// Graph revision on which the proposal was based
        #[arg(long)]
        base: String,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
        #[arg(long, default_value = "ios")]
        target: String,
    },
    /// Diagnose UI compiler and target-host prerequisites without modifying the workspace
    Doctor {
        #[arg(long, default_value = "ios")]
        target: String,
        #[arg(long)]
        json: bool,
    },
    /// Inspect the versioned Lynx/Acore capability registry for a target
    Capabilities {
        #[arg(long, default_value = "ios")]
        target: String,
        #[arg(long, value_enum)]
        kind: Option<commands::ui::UiCapabilityKind>,
        /// Case-insensitive match against capability ID, Lynx name, or Acore name
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Run deterministic compiler/session checks for one Acore UI module
    Test {
        source: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
        #[arg(long, default_value = "ios")]
        target: String,
    },
    /// Build a deterministic, target-specific Axiom application artifact
    Build {
        source: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
        #[arg(long, default_value = "ios")]
        target: String,
        /// Artifact path; defaults to dist/<module>-<target>.axiomapp
        #[arg(long)]
        out: Option<PathBuf>,
        /// Permit unsigned local contracts and mark the artifact development-only
        #[arg(long)]
        development: bool,
    },
}

#[derive(Subcommand)]
enum UiHostAction {
    /// Set up the selected UI Host outside the application workspace
    Install {
        #[arg(long, default_value = "ios")]
        target: String,
        /// Axiom UI Host release manifest produced by the Axiom-owned host pipeline
        #[arg(long)]
        release_manifest: Option<PathBuf>,
        /// Platform variant from the release manifest (for example `simulator` or `device`)
        #[arg(long)]
        variant: Option<String>,
        /// Existing host project directory; otherwise Axiom offers its local fixture default
        #[arg(long)]
        host_root: Option<PathBuf>,
        /// Fail instead of prompting when host information is missing
        #[arg(long)]
        non_interactive: bool,
    },
    /// Show the selected UI Host's local setup state
    Status {
        #[arg(long, default_value = "ios")]
        target: String,
    },
    /// Restart the installed development host and discard only stale delivery control records
    Recover {
        #[arg(long, default_value = "ios")]
        target: String,
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
        /// Lowercase project identifier used in release URLs, for example payments-api.
        #[arg(long)]
        slug: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    Link {
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Replace the active signing key while retaining old public keys for verification
    RotateKey {
        /// Project slug, for example `demo-app`
        #[arg(long)]
        project: String,
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

    let local_cloud = uses_local_cloud()?;
    let local_application_command = match &cli.command {
        Commands::Package { .. } => true,
        Commands::Inspect { .. } => true,
        Commands::Install { package, module } => {
            !module && commands::app::is_axiom_application(Path::new(package))
        }
        _ => false,
    };
    let active_config = if local_cloud
        || local_application_command
        || matches!(
            &cli.command,
            Commands::Doctor { .. }
                | Commands::Benchmark { .. }
                // Acore UI development is local compiler work. It must not
                // trigger a cloud-auth prompt before an empty-workspace
                // starter, check, inspection, or virtual reload can run.
                | Commands::Ui { .. }
                | Commands::Extensions { .. }
                | Commands::Run { .. }
                | Commands::Lsp
        ) {
        // The private-alpha referral gate and its telemetry only apply to the
        // public control plane. A loopback endpoint is an explicit developer
        // choice and uses an isolated local CLI profile, never production
        // registration or telemetry state.
        None
    } else {
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

        access_config
    };

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
        Commands::Init { .. } => "init",
        Commands::Doctor { .. } => "doctor",
        Commands::Onboard { .. } => "onboard",
        Commands::Benchmark { .. } => "benchmark",
        Commands::Login => "login",
        Commands::Join { .. } => "join",
        Commands::Cache { .. } => "cache",
        Commands::Install { .. } => "install",
        Commands::Package { .. } => "package",
        Commands::Build { .. } => "build",
        Commands::Inspect { .. } => "inspect",
        Commands::Release { .. } => "release",
        Commands::Pull { .. } => "pull",
        Commands::Watch { .. } => "watch",
        Commands::Project { .. } => "project",
        Commands::Diff { .. } => "diff",
        Commands::Contract { .. } => "contract",
        Commands::Packages { .. } => "packages",
        Commands::Extensions { .. } => "extensions",
        Commands::Ui { .. } => "ui",
        Commands::Run { .. } => "run",
        Commands::Domain { .. } => "domain",
        Commands::Security { .. } => "security",
        Commands::Serve { .. } => "serve",
        Commands::Deploy { .. } => "deploy",
        Commands::Test { .. } => "test",
        Commands::Eval {
            file: _,
            format: _,
            variant: _,
        } => "eval",
        Commands::Repl => "repl",
        Commands::Lsp => "lsp",
    };

    // Send Telemetry (This internally handles the "Kill Switch" / Access Revocation)
    if let Some(active_config) = active_config.as_ref() {
        Telemetry::track(
            active_config,
            cmd_name,
            duration,
            success,
            error_msg.as_deref(),
        )
        .await;
    }

    result
}

// Helper to route commands (Refactored from original main)
async fn execute_command(command: &Commands) -> anyhow::Result<()> {
    match command {
        Commands::Doctor { json, strict } => commands::doctor::handle_doctor(*json, *strict).await,
        Commands::Onboard {
            role,
            entrypoint,
            module,
            framework,
            contract,
            apply,
        } => {
            commands::onboard::handle_onboard(
                role.clone(),
                entrypoint.clone(),
                module.clone(),
                framework.clone(),
                contract.clone(),
                *apply,
            )
            .await
        }
        Commands::Benchmark {
            file,
            variant,
            iterations,
            warmup,
            output,
            json,
        } => {
            commands::benchmark::handle_benchmark(
                file.clone(),
                variant.clone(),
                *iterations,
                *warmup,
                output.clone(),
                *json,
            )
            .await
        }
        Commands::Domain { action } => match action {
            DomainAction::Validate { file, json } => {
                commands::domain::handle_validate(file.clone(), *json).await
            }
            DomainAction::Bootstrap {
                input,
                source,
                output,
            } => {
                commands::domain::handle_bootstrap(input.clone(), source.clone(), output.clone())
                    .await
            }
        },
        Commands::Contract { action } => match action {
            ContractAction::Resolve { deps, lock } => {
                commands::contract::handle_resolve(deps.clone(), lock.clone()).await
            }
            ContractAction::Verify { deps, lock } => {
                commands::contract::handle_verify(deps.clone(), lock.clone()).await
            }
            ContractAction::Diff { before, after } => {
                commands::contract::handle_diff(before.clone(), after.clone()).await
            }
            ContractAction::Facade { alias, lock } => {
                commands::contract::handle_facade(lock.clone(), alias.clone()).await
            }
            ContractAction::CheckSource { source, lock } => {
                commands::contract::handle_check_source(source.clone(), lock.clone()).await
            }
        },
        Commands::Packages { action } => match action {
            PackagesAction::Build { source, out } => {
                commands::packages::handle_build(source.clone(), out.clone()).await
            }
            PackagesAction::Resolve { deps, lock } => {
                commands::packages::handle_resolve(deps.clone(), lock.clone()).await
            }
            PackagesAction::Verify { deps, lock } => {
                commands::packages::handle_verify(deps.clone(), lock.clone()).await
            }
            PackagesAction::Inspect { alias, lock } => {
                commands::packages::handle_inspect(lock.clone(), alias.clone()).await
            }
            PackagesAction::Diff {
                before,
                after,
                approvals,
            } => {
                commands::packages::handle_diff(before.clone(), after.clone(), approvals.clone())
                    .await
            }
            PackagesAction::CheckSource {
                source,
                package_lock,
                ui_lock,
                target,
            } => {
                commands::packages::handle_check_source(
                    source.clone(),
                    package_lock.clone(),
                    ui_lock.clone(),
                    target.clone(),
                )
                .await
            }
            PackagesAction::Run {
                source,
                package_lock,
                ui_lock,
                target,
                once,
            } => {
                commands::ui::handle_run_with_packages(
                    source.clone(),
                    ui_lock.clone(),
                    package_lock.clone(),
                    target.clone(),
                    *once,
                )
                .await
            }
        },
        Commands::Extensions { action } => match action {
            ExtensionsAction::SourceBuild {
                alias,
                deps,
                out,
                target,
                clean,
            } => {
                commands::extensions::handle_source_build(
                    deps.clone(),
                    alias.clone(),
                    out.clone(),
                    target.clone(),
                    *clean,
                )
                .await
            }
            ExtensionsAction::SourceRelease {
                alias,
                deps,
                out,
                workflow,
                application,
                application_version,
                clean,
            } => {
                commands::extensions::handle_source_release(
                    deps.clone(),
                    alias.clone(),
                    out.clone(),
                    workflow.clone(),
                    application.clone(),
                    application_version.clone(),
                    *clean,
                )
                .await
            }
            ExtensionsAction::Build { source, out } => {
                commands::extensions::handle_build(source.clone(), out.clone()).await
            }
            ExtensionsAction::Sign { artifact, key, out } => {
                commands::extensions::handle_sign(artifact.clone(), key.clone(), out.clone()).await
            }
            ExtensionsAction::Resolve { deps, lock } => {
                commands::extensions::handle_resolve(deps.clone(), lock.clone()).await
            }
            ExtensionsAction::Verify { manifest } => {
                commands::extensions::handle_verify(manifest.clone()).await
            }
            ExtensionsAction::Inspect { alias, manifest } => {
                commands::extensions::handle_inspect(manifest.clone(), alias.clone()).await
            }
            ExtensionsAction::Permissions { alias, manifest } => {
                commands::extensions::handle_permissions(manifest.clone(), alias.clone()).await
            }
            ExtensionsAction::Graph { manifest } => {
                commands::extensions::handle_graph(manifest.clone()).await
            }
            ExtensionsAction::Diff {
                before,
                after,
                approvals,
            } => {
                commands::extensions::handle_diff(before.clone(), after.clone(), approvals.clone())
                    .await
            }
            ExtensionsAction::Test { manifest } => {
                commands::extensions::handle_test(manifest.clone()).await
            }
            ExtensionsAction::Run {
                alias,
                manifest,
                target,
                export,
                input,
                state,
                audit_out,
            } => {
                commands::extensions::handle_run(
                    manifest.clone(),
                    alias.clone(),
                    target.clone(),
                    export.clone(),
                    input.clone(),
                    state.clone(),
                    audit_out.clone(),
                )
                .await
            }
        },
        Commands::Ui { action } => match action {
            UiAction::Host { action } => match action {
                UiHostAction::Install {
                    target,
                    release_manifest,
                    variant,
                    host_root,
                    non_interactive,
                } => {
                    commands::ui::handle_host_install(
                        target.clone(),
                        release_manifest.clone(),
                        variant.clone(),
                        host_root.clone(),
                        *non_interactive,
                    )
                    .await
                }
                UiHostAction::Status { target } => {
                    commands::ui::handle_host_status(target.clone()).await
                }
                UiHostAction::Recover { target } => {
                    commands::ui::handle_host_recover(target.clone()).await
                }
            },
            UiAction::Init { directory, target } => {
                commands::ui::handle_init(directory.clone(), target.clone()).await
            }
            UiAction::Check {
                source,
                lock,
                target,
            } => commands::ui::handle_check(source.clone(), lock.clone(), target.clone()).await,
            UiAction::Run {
                source,
                lock,
                target,
                once,
            } => {
                commands::ui::handle_run(source.clone(), lock.clone(), target.clone(), *once).await
            }
            UiAction::Inspect {
                view,
                source,
                lock,
                target,
                max_symbols,
            } => {
                commands::ui::handle_inspect(
                    view.clone(),
                    source.clone(),
                    lock.clone(),
                    target.clone(),
                    *max_symbols,
                )
                .await
            }
            UiAction::Context {
                source,
                lock,
                target,
                max_symbols,
            } => {
                commands::ui::handle_context(
                    source.clone(),
                    lock.clone(),
                    target.clone(),
                    *max_symbols,
                )
                .await
            }
            UiAction::AiCheck {
                source,
                proposed,
                base,
                lock,
                target,
            } => {
                commands::ui::handle_ai_check(
                    source.clone(),
                    proposed.clone(),
                    base.clone(),
                    lock.clone(),
                    target.clone(),
                )
                .await
            }
            UiAction::Doctor { target, json } => {
                commands::ui::handle_doctor(target.clone(), *json).await
            }
            UiAction::Capabilities {
                target,
                kind,
                query,
                json,
            } => {
                commands::ui::handle_capabilities(target.clone(), *kind, query.clone(), *json).await
            }
            UiAction::Test {
                source,
                lock,
                target,
            } => commands::ui::handle_test(source.clone(), lock.clone(), target.clone()).await,
            UiAction::Build {
                source,
                lock,
                target,
                out,
                development,
            } => {
                commands::ui::handle_build(
                    source.clone(),
                    lock.clone(),
                    target.clone(),
                    out.clone(),
                    *development,
                )
                .await
            }
        },
        Commands::Run {
            source,
            lock,
            target,
            mode,
            host,
            port,
            debug,
            frozen,
            once,
        } => {
            if commands::app::is_axiom_application(source) {
                if mode.is_some() {
                    anyhow::bail!("--mode applies to backend .acore sources, not packaged .axiomapp execution");
                }
                if *once {
                    anyhow::bail!("--once applies to authored .acore sessions, not packaged .axiomapp execution");
                }
                commands::app::handle_run(source.clone(), target.clone()).await
            } else {
                match commands::run::classify_acore_source(source)? {
                    commands::run::AcoreSourceKind::Backend => {
                        if target.is_some() {
                            anyhow::bail!("--target applies to frontend .acore sources, not backend execution");
                        }
                        if lock.is_some() || *frozen || *once {
                            anyhow::bail!(
                                "--lock, --frozen, and --once apply to frontend .acore sources"
                            );
                        }
                        commands::run::handle_backend(
                            source.clone(),
                            mode.unwrap_or(commands::run::RunMode::Service),
                            host.clone(),
                            *port,
                            *debug,
                        )
                        .await
                    }
                    commands::run::AcoreSourceKind::Frontend => {
                        if mode.is_some() {
                            anyhow::bail!(
                                "--mode applies to backend .acore sources, not frontend execution"
                            );
                        }
                        let prepared_lock =
                            commands::run::prepare_frontend(source, lock.as_deref(), *frozen)
                                .await?;
                        commands::ui::handle_run(
                            source.clone(),
                            prepared_lock,
                            target.clone().unwrap_or_else(|| "ios".to_string()),
                            *once,
                        )
                        .await
                    }
                }
            }
        }
        Commands::Security { action } => match action {
            SecurityAction::Check {
                file,
                json,
                fail_on_warning,
            } => commands::security::handle_check(file.clone(), *json, *fail_on_warning).await,
        },
        Commands::Test { file, tag } => {
            commands::test::handle_test(file.clone(), tag.clone()).await
        }
        Commands::Serve { file, port, debug } => {
            commands::serve::handle_serve(file.clone(), *port, *debug).await
        }
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
            } else if commands::app::is_axiom_application(Path::new(package)) {
                commands::app::handle_install(PathBuf::from(package)).await
            } else {
                println!("Marketplace installation coming soon for '{}'", package);
                Ok(())
            }
        }
        Commands::Package { artifacts, out } => {
            commands::app::handle_package(artifacts.clone(), out.clone()).await
        }

        Commands::Build {
            file,
            variant,
            release,
            project,
            version,
            branch,
            message,
        } => {
            let variant_str = variant.clone().unwrap_or("default".to_string());

            if std::env::var("CI").is_ok() {
                match axiom_build::core::build::handle_build(&file, &variant_str, "", "", None)
                    .await
                {
                    Ok(out_file) => {
                        println!("✅ Build Succeeded! Generated {}", out_file);
                        let lockfile_path = format!("{}.lockfile", &file);
                        if let Ok(content) = std::fs::read_to_string(&file) {
                            let _ = std::fs::write(&lockfile_path, content);
                        }
                        if *release {
                            commands::release::handle_release(
                                &out_file,
                                project.as_deref(),
                                version.as_deref(),
                                Some(Path::new(file)),
                                &variant_str,
                            )
                            .await?;
                        }
                        Ok(())
                    }
                    Err(e) => {
                        eprintln!("❌ Build failed: {}", e);
                        Err(e.into())
                    }
                }
            } else {
                handle_build_command(
                    file,
                    variant_str,
                    *release,
                    project.as_deref(),
                    version.as_deref(),
                )
                .await
            }
        }
        Commands::Release {
            file_path,
            project,
            version,
        } => {
            // `axiom build` now creates the visible `axiom.axiom` artifact.
            // Retain the legacy hidden filename as a fallback for existing
            // projects and CI scripts.
            let path = file_path.clone().unwrap_or_else(|| {
                let visible = std::path::PathBuf::from("axiom.axiom");
                if visible.is_file() {
                    visible
                } else {
                    std::path::PathBuf::from(".axiom")
                }
            });
            let default_source = Path::new("axiom.acore");
            commands::release::handle_release(
                path.to_str().unwrap(),
                project.as_deref(),
                version.as_deref(),
                default_source.is_file().then_some(default_source),
                "default",
            )
            .await
        }
        Commands::Inspect {
            action: Some(action),
            ..
        } => commands::inspector::handle(action).await,
        Commands::Inspect { path, .. } => {
            let path = path.clone().unwrap_or_else(|| PathBuf::from("axiom.axiom"));
            if path.is_dir()
                || path.extension().and_then(|extension| extension.to_str()) == Some("acore")
            {
                return commands::inspector::handle(
                    &commands::inspector::InspectorAction::Overview {
                        path,
                        format: commands::inspector::InspectorFormat::Human,
                    },
                )
                .await;
            }
            if commands::app::is_axiom_application(&path) {
                return commands::app::handle_inspect(path.clone()).await;
            }
            let artifact = if path.as_path() == Path::new("axiom.axiom") && !path.is_file() {
                PathBuf::from(".axiom")
            } else {
                path.clone()
            };
            handle_inspect(&artifact).await
        }
        Commands::Project { action } => match action {
            ProjectAction::List => commands::project::handle_project_list().await,
            ProjectAction::Create {
                name,
                slug,
                description,
                path,
            } => {
                commands::project::handle_project_create(
                    name.clone(),
                    slug.clone(),
                    description.clone(),
                    path.clone(),
                )
                .await
            }
            ProjectAction::Link { project_id } => {
                commands::project::handle_project_link(project_id.clone()).await
            }
            ProjectAction::RotateKey { project } => {
                commands::project::handle_project_rotate_key(project.clone()).await
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
            source,
            contract,
            contract_config,
            framework,
            name,
            out, // <-- Add this
        } => {
            commands::pull::handle_pull(
                source.clone(),
                contract.clone(),
                contract_config.clone(),
                framework.clone(),
                name.clone(),
                out.clone(), // <-- Pass this
            )
            .await
        }
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
        Commands::Init { entrypoint, module } => {
            crate::commands::init::handle_init(entrypoint.clone(), module.clone()).await
        }
        Commands::Login => handle_login_tui().await,
        Commands::Join { email } => crate::commands::join::handle_join(email.clone()).await,
        Commands::Cache { action } => {
            let mut cache_dir = crate::auth_store::get_config_dir()?;
            cache_dir.push("cache");
            cache_dir.push("sled_db");
            std::fs::create_dir_all(&cache_dir)?;

            match action {
                CacheAction::Ls => crate::commands::cache::handle_ls(&cache_dir).await,
                CacheAction::Get { key } => {
                    crate::commands::cache::handle_get(&cache_dir, key).await
                }
                CacheAction::Clear => crate::commands::cache::handle_clear(&cache_dir).await,
            }
        }
    }
}

// =========================================================================================
//  EXISTING HELPERS (Kept exactly as original to ensure full file correctness)
// =========================================================================================

async fn handle_login_tui() -> anyhow::Result<()> {
    // Do not enter raw/alternate-screen mode until the device flow is ready. If the
    // authentication service rejects or rate-limits the request, the developer gets
    // a normal shell error and no detached TUI task is left running.
    let auth_info = CloudClient::start_login().await?;

    let mut state = crate::state::State::new();
    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    tui.enter().map_err(|e| anyhow::anyhow!(e))?;

    // Put the device-flow details into the TUI state.
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
                            val["expires_in"].as_u64().unwrap_or_default(),
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
                        if let crate::state::LoginStatus::Error(message) =
                            &state.login_context.status
                        {
                            return Err(anyhow::anyhow!(message.clone()));
                        }
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
    file_path: &String,
    variant: String,
    release: bool,
    project_override: Option<&str>,
    version_override: Option<&str>,
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
                crate::commands::release::handle_release(
                    &output_filename,
                    project_override,
                    version_override,
                    Some(Path::new(file_path)),
                    &variant,
                )
                .await?;
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
