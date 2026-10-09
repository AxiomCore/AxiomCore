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
use crate::components::inspect::endpoint_detail::render_endpoint_detail;
use crate::components::inspect::endpoint_list::render_endpoint_list;
use crate::components::inspect::model_browser::render_model_browser;
use crate::state::InspectTab;
use crate::telemetry::Telemetry;
use axiom_cloud::{uses_local_cloud, CliApi, CloudClient};
use clap::{Parser, Subcommand};
use console::style;
use crossterm::event::KeyCode;
use dialoguer::{theme::ColorfulTheme, Input};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "axiom",
    bin_name = "axiom",
    author,
    version,
    about = "Check, build, inspect, and run Acore contracts; manage Axiom Cloud releases"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Offline saved-source checking and editor compatibility metadata
    Editor {
        #[command(subcommand)]
        action: commands::editor::Action,
    },
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
    /// Install lsp-server, an intact .axiomapp, or a compiler/extractor module
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
    /// Check a frontend, backend, or database contract without writing project outputs
    Check {
        /// Contract source to validate
        #[arg(default_value = "axiom.acore")]
        file: PathBuf,
        /// Backend contract variant (defaults to default)
        #[arg(long)]
        variant: Option<String>,
        /// Print a machine-readable validation report, including on failure
        #[arg(long)]
        json: bool,
        /// Frontend target override: web, ios, or android
        #[arg(long)]
        target: Option<String>,
        /// Existing frontend contract lock; never resolved or modified by check
        #[arg(long)]
        lock: Option<PathBuf>,
    },
    /// Print expanded backend JSON or the machine-readable declaration catalog
    Explain {
        #[arg(required_unless_present = "schema", conflicts_with = "schema")]
        file: Option<PathBuf>,
        #[arg(long)]
        variant: Option<String>,
        #[arg(long)]
        schema: bool,
        /// Include source spans, compiler defaults, identities and effective policy origins
        #[arg(long, conflicts_with = "schema")]
        provenance: bool,
    },
    /// Inspect and generate typed server plans, OpenAPI, and native handler interfaces offline
    Server {
        #[command(subcommand)]
        action: commands::server::ServerAction,
    },
    /// Migrate a legacy backend contract into a verified new sibling source
    Migrate {
        file: PathBuf,
        /// New sibling .acore file; the original source is preserved
        #[arg(short, long)]
        output: PathBuf,
    },
    Test {
        /// Optional path to an .acore file. If omitted, uses axiom.acore
        file: Option<PathBuf>,
        /// Only run test suites that match this tag
        #[arg(long)]
        tag: Option<String>,
        /// Contract variant used to resolve endpoint references
        #[arg(long)]
        variant: Option<String>,
    },
    /// Validate and replay development scenarios against exact compiled artifacts
    Scenario {
        #[command(subcommand)]
        action: commands::scenario::ScenarioAction,
    },
    /// Start the real local API; never falls back to mocks or cloud
    Serve(commands::server_command_contract::ServeInput),
    /// Run isolated server simulation or explicit local/cloud contract mocks
    Mock(commands::server_command_contract::MockInput),
    /// Start the Acore REPL
    Repl,
    /// Run the Acore language server over stdin/stdout without cloud access
    Lsp {
        #[arg(long, conflicts_with = "version")]
        version_json: bool,
        #[arg(long)]
        version: bool,
    },
    /// Build frontend.axiom, backend.axiom, or database.axiom beside the source
    Build {
        /// The Acore file to build (defaults to axiom.acore if not provided)
        #[arg(default_value = "axiom.acore")]
        file: String,

        /// Output .axiom path; relative paths are relative to the current directory
        #[arg(short, long)]
        out: Option<PathBuf>,

        /// Frontend target override: web, ios, or android
        #[arg(long)]
        target: Option<String>,

        /// Existing frontend contract lock; build does not fetch dependencies
        #[arg(long)]
        lock: Option<PathBuf>,

        /// Backend contract variant (defaults to default)
        #[arg(long)]
        variant: Option<String>,

        /// Existing artifact whose FlatBuffers field slots must remain compatible
        #[arg(long)]
        compatibility_baseline: Option<PathBuf>,

        /// Release the compiled backend contract to Axiom Cloud
        #[arg(long)]
        release: bool,

        /// Optional Cloud project ID or slug override. By default the linked
        /// directory project is used automatically.
        #[arg(long, requires = "release")]
        project: Option<String>,

        /// Optional immutable Cloud release version. Interactive releases
        /// suggest the next available version on a collision.
        #[arg(long, requires = "release")]
        version: Option<String>,
    },
    /// Inspect an application workspace, contract snapshot, or packaged application
    Inspect {
        #[command(subcommand)]
        action: Option<commands::inspector::InspectorAction>,
        /// Artifact or source path; omitted when using an Inspector subcommand
        path: Option<PathBuf>,
    },
    Release {
        /// Backend artifact (defaults to backend.axiom; legacy filenames also accepted)
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
        /// Contract variant used for evaluation and rebuilds
        #[arg(long, requires = "build")]
        variant: Option<String>,
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
    /// Author database contracts and explicitly inspect PostgreSQL read-only
    Database {
        #[command(subcommand)]
        action: DatabaseAction,
    },
    /// Build, sign, verify, inspect, and review sandboxed extensions
    Extensions {
        #[command(subcommand)]
        action: ExtensionsAction,
    },
    /// Resolve and audit third-party packages used by authored extensions
    Dependencies {
        #[command(subcommand)]
        action: DependenciesAction,
    },
    /// Advanced frontend tooling: hosts, application packaging, tests, and inspection
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
        /// Backend contract variant
        #[arg(long)]
        variant: Option<String>,
        /// Reviewed backend deployment configuration
        #[arg(long)]
        deployment: Option<PathBuf>,
        /// Verified sandbox implementation references
        #[arg(long)]
        bindings: Option<PathBuf>,
        /// Reviewed durable controller for backend producer actions
        #[arg(long)]
        delivery: Option<PathBuf>,
        /// Private typed backend simulation fixtures (mock mode only)
        #[arg(long)]
        fixtures: Option<PathBuf>,
        /// Reviewed development identities (mock mode only)
        #[arg(long)]
        identities: Option<PathBuf>,
        /// Reload source-owned action plans within reviewed grants
        #[arg(long)]
        watch: bool,
        /// Redacted backend startup diagnostics
        #[arg(short, long)]
        debug: bool,
        /// Require reviewed locks and extension workflows without development regeneration
        #[arg(long)]
        frozen: bool,
        /// Compile once and exit; intended for CI and host-independent checks
        #[arg(long)]
        once: bool,
        /// Open the browser or native simulator after starting the UI session
        #[arg(long, conflicts_with = "once")]
        launch: bool,
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
enum DatabaseAction {
    /// Schema-derived private backend operations and database-to-API impact
    Runtime {
        #[command(subcommand)]
        command: commands::database_runtime::Action,
    },
    /// Frozen tenant fleets, policy review, offline delivery and drift monitoring
    Fleet {
        #[command(subcommand)]
        command: commands::database_fleet::Action,
    },
    /// Independently scoped SQLite lifecycle; requires an external engine library
    Sqlite {
        #[command(subcommand)]
        command: commands::database_sqlite::Action,
    },
    /// Check the explicit source entry and its imports without database I/O
    Check { source: PathBuf },
    /// Build a canonical server-only schema package
    Build {
        source: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Inspect a source contract or schema package without connecting
    Inspect { source: PathBuf },
    /// Compare two source contracts or typed schema packages
    Diff { before: PathBuf, after: PathBuf },
    /// Format database source; prints to stdout unless --write is supplied
    Fmt {
        source: PathBuf,
        #[arg(long)]
        write: bool,
    },
    /// Describe supported offline declaration/type signatures
    Catalog,
    /// Validate private deployment fields without resolving providers or connecting
    BindingCheck { deployment: PathBuf },
    /// Explicit read-only PostgreSQL catalog observation (private binding required)
    InspectLive {
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        source: Option<PathBuf>,
    },
    /// Prepare Acore source from a static SQL schema; never execute SQL
    ImportSql {
        input: PathBuf,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        database: String,
        #[arg(long = "schema", required = true)]
        schemas: Vec<String>,
        #[arg(long, default_value_t = 17)]
        postgres_major: u16,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Prepare Acore source from static Atlas schema HCL; never evaluate HCL
    ImportHcl {
        input: PathBuf,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        database: String,
        #[arg(long = "schema", required = true)]
        schemas: Vec<String>,
        #[arg(long, default_value_t = 17)]
        postgres_major: u16,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Prepare adoption source; does not accept ownership or register a baseline
    Adopt {
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        source: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Observe target/dev separation without modifying either database
    VerifyIsolation {
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        source: Option<PathBuf>,
    },
    /// Show the contract graph or sanitized D2 inspection graph
    Graph { input: PathBuf },
    /// Explain one contract or catalog object and its dependency impact
    Explain { input: PathBuf, id: String },
    /// Pure typed evolution checking against selected contract snapshots
    EvolutionCheck {
        source: PathBuf,
        #[arg(long)]
        previous: Option<PathBuf>,
        #[arg(long)]
        evolution: PathBuf,
    },
    /// Generate hidden candidate history after replay on an isolated pinned dev DB
    Plan {
        source: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        dev_profile: PathBuf,
        #[arg(long, value_parser = ["empty", "adoption"], conflicts_with = "parent")]
        baseline: Option<String>,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        tests: Option<PathBuf>,
        #[arg(long)]
        evolution: Option<PathBuf>,
        #[arg(long, conflicts_with = "evolution", requires = "baseline")]
        checkpoint: Option<PathBuf>,
    },
    /// Export or verify private candidate history without database I/O
    History {
        #[command(subcommand)]
        action: DatabaseHistoryAction,
    },
    /// Sign or verify an exact private candidate prefix offline
    Release {
        #[command(subcommand)]
        action: DatabaseReleaseAction,
    },
    /// Explicit enrollment, status, verification and bounded reconciliation
    Target {
        #[command(subcommand)]
        action: DatabaseTargetAction,
    },
    /// Apply an authorized prefix to an explicitly enrolled local/staging target
    Apply {
        bundle: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
    },
}

#[derive(Subcommand)]
enum DatabaseHistoryAction {
    /// Export the complete verified prefix into a new private offline directory
    Export {
        source: PathBuf,
        #[arg(long)]
        head: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Export a complete-prefix checkpoint including deploy data lineage
    Checkpoint {
        source: PathBuf,
        #[arg(long)]
        head: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Verify an exported bundle; does not connect or grant apply authority
    Verify { bundle: PathBuf },
    /// Import exact generated history for the matching compiled project
    Import {
        bundle: PathBuf,
        #[arg(long)]
        source: PathBuf,
    },
    /// Adopt a verified Atlas directory/revision snapshot as private provenance
    ImportAtlas {
        bundle: PathBuf,
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        revision_schema: String,
        #[arg(long, default_value = "atlas_schema_revisions")]
        revision_table: String,
        #[arg(long, required = true)]
        legacy_stopped: bool,
    },
}

#[derive(Subcommand)]
enum DatabaseReleaseAction {
    /// Generate a private Ed25519 key in a new owner-only file
    Keygen {
        #[arg(long)]
        out: PathBuf,
    },
    /// Approve the exact regenerated prefix and its explicitly reviewed risks
    Sign {
        bundle: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        allow_destructive: bool,
        #[arg(long)]
        allow_data_dependent: bool,
    },
    /// Sign a reviewed repair for an exact observed pending index OID
    RepairIndex {
        bundle: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        index_id: String,
        #[arg(long)]
        index_oid: String,
        #[arg(long, value_parser = ["drop-rebuild-concurrently", "accept-valid"])]
        strategy: String,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Verify signature, checksums, continuity and typed generated SQL offline
    Verify {
        bundle: PathBuf,
        #[arg(long)]
        trusted_key: String,
    },
}

#[derive(Subcommand)]
enum DatabaseTargetAction {
    BindingCheck {
        deployment: PathBuf,
    },
    Register {
        bundle: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
    },
    Status {
        bundle: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
    },
    Verify {
        bundle: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
    },
    Reconcile {
        bundle: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        repair: Option<PathBuf>,
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
        #[arg(long, conflicts_with = "once")]
        launch: bool,
    },
}

#[derive(Subcommand)]
enum ExtensionsAction {
    /// Prepare digest-bound physical IDE helpers without compiling or executing guests
    PrepareIde {
        alias: String,
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        /// Optional canonical SDK interface; omitted means the manifest-generated SDK ABI
        #[arg(long)]
        interface: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Emit the canonical language-neutral SDK interface for one authored extension
    Interface {
        alias: String,
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        /// Optional resolved authority lock used to show a target-effective surface
        #[arg(long)]
        authority_lock: Option<PathBuf>,
        /// Required with --authority-lock
        #[arg(long, requires = "authority_lock")]
        target: Option<String>,
        /// Write the canonical requested-interface artifact
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Compare two canonical SDK interface artifacts semantically
    InterfaceDiff {
        before: PathBuf,
        after: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Inspect Rust macro lowering, generated bindings, interface, and hidden workspace
    RustInspect {
        alias: String,
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        #[arg(long, default_value = "summary")]
        view: String,
        #[arg(long)]
        json: bool,
    },
    /// Move a legacy low-level Rust extension onto the explicit raw compatibility surface
    MigrateRust {
        /// Registered Rust extension alias
        alias: String,
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        /// Write the behavior-preserving migration to a new file
        #[arg(long, conflicts_with_all = ["write", "check"])]
        out: Option<PathBuf>,
        /// Atomically replace the registered source
        #[arg(long, conflicts_with_all = ["out", "check"])]
        write: bool,
        /// Fail when legacy top-level SDK imports remain
        #[arg(long, conflicts_with_all = ["out", "write"])]
        check: bool,
        #[arg(long)]
        json: bool,
    },
    /// Inspect generated TypeScript declarations, bridge entry, interface, and dependencies
    #[command(name = "typescript-inspect")]
    TypeScriptInspect {
        alias: String,
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        #[arg(long, default_value = "summary")]
        view: String,
        #[arg(long)]
        json: bool,
    },
    /// Inspect generated Python stubs, AOT lowering, source map, interface, and dependencies
    #[command(name = "python-inspect")]
    PythonInspect {
        alias: String,
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        #[arg(long, default_value = "summary")]
        view: String,
        #[arg(long)]
        json: bool,
    },
    /// Compile one Rust, TypeScript, or Python source module into verified WASM
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
    /// Build and sign one registered Rust, TypeScript, or Python source module into a local release workflow
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
enum DependenciesAction {
    /// Validate the complete AxiomDeps.toml v2 document without fetching
    Check {
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Canonically format a v2 manifest
    Format {
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        /// Fail if formatting would change the file
        #[arg(long, conflicts_with = "write")]
        check: bool,
        /// Replace the manifest after successful validation
        #[arg(long)]
        write: bool,
    },
    /// Convert a supported v1 manifest to the strict v2 schema
    Migrate {
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        #[arg(long, conflicts_with = "write")]
        out: Option<PathBuf>,
        /// Replace the input after a successful in-memory migration
        #[arg(long)]
        write: bool,
    },
    /// Print the checked-in AxiomDeps.toml v2 JSON Schema
    Schema,
    /// Return deterministic editor completion candidates for a TOML path
    Completions {
        #[arg(long, default_value = "")]
        path: String,
        #[arg(long)]
        json: bool,
    },
    /// Explain one AxiomDeps.toml v2 field
    Hover {
        path: String,
        #[arg(long)]
        json: bool,
    },
    /// Resolve exact package graphs and write canonical AxiomDeps.lock
    Resolve {
        #[arg(long, default_value = "AxiomDeps.toml")]
        deps: PathBuf,
        #[arg(long, default_value = "AxiomDeps.lock")]
        lock: PathBuf,
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long, default_value = ".axiom/dependencies")]
        workspace_view: PathBuf,
        /// Require the existing lock and cache; never resolve or fetch
        #[arg(long, conflicts_with = "offline")]
        locked: bool,
        /// Reuse the existing lock and cache without network access
        #[arg(long)]
        offline: bool,
        #[arg(long)]
        json: bool,
    },
    /// Validate the canonical lock and every referenced cached byte
    Verify {
        #[arg(long, default_value = "AxiomDeps.lock")]
        lock: PathBuf,
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Explain environments, provenance, licenses, advisories, and package scripts
    Inspect {
        #[arg(long, default_value = "AxiomDeps.lock")]
        lock: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Verify, prune, or recover the immutable authored-dependency cache
    Cache {
        #[command(subcommand)]
        action: DependencyCacheAction,
    },
}

#[derive(Subcommand)]
enum DependencyCacheAction {
    Verify {
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Prune {
        #[arg(long)]
        cache: Option<PathBuf>,
        /// Canonical locks whose objects must be retained
        #[arg(long = "lock", required = true)]
        locks: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Recover {
        #[arg(long, default_value = "AxiomDeps.lock")]
        lock: PathBuf,
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long)]
        json: bool,
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
        #[arg(long, conflicts_with = "once")]
        launch: bool,
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
    /// Run compiler-session smoke, or an explicit application interaction suite
    Test {
        source: PathBuf,
        #[arg(long, default_value = "axiom.ui.lock.json")]
        lock: PathBuf,
        #[arg(long, default_value = "ios")]
        target: String,
        /// Versioned acore-application-test/v1 JSON suite (omitting keeps session smoke)
        #[arg(long)]
        suite: Option<PathBuf>,
        /// Application execution layer; adapter executes emitted native owner hooks
        #[arg(long, value_enum, requires = "suite")]
        layer: Option<commands::ui::UiTestLayer>,
        /// Write a machine-readable report, including failed assertions and traces
        #[arg(long)]
        report: Option<PathBuf>,
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
    if commands::lsp::native_entry().await? {
        return Ok(());
    }
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
    // Mode/input errors must precede cloud credentials, registration and I/O.
    match &cli.command {
        Commands::Serve(input) => {
            input.resolve(&std::env::current_dir()?)?;
        }
        Commands::Mock(input) => {
            input.resolve(&std::env::current_dir()?)?;
        }
        Commands::Release { file_path, .. } => {
            let artifact = file_path.clone().unwrap_or_else(default_backend_artifact);
            if artifact.is_file() {
                commands::contract_build::validate_release(&artifact)?;
            }
        }
        Commands::Install {
            package,
            module: false,
        } if package != "lsp-server"
            && !commands::app::is_axiom_application(Path::new(package)) =>
        {
            anyhow::bail!("Unsupported install target `{package}`. Use `install lsp-server`, `install FILE.axiomapp`, or `install MODULE --module`.");
        }
        Commands::Build {
            file,
            target,
            lock,
            variant,
            compatibility_baseline,
            release,
            ..
        } if Path::new(file).is_file() => {
            let source = commands::contract_build::source(Path::new(file))?;
            commands::contract_build::validate_options(
                source.kind,
                commands::contract_build::Options {
                    variant: variant.as_deref(),
                    target: target.as_deref(),
                    lock: lock.as_deref(),
                    compatibility_baseline: compatibility_baseline.as_deref(),
                    release: *release,
                },
            )?;
        }
        _ => {}
    }

    // 2. GATEKEEPER LOGIC (Private Alpha Check)
    // -------------------------------------------------------------------------

    // Allow 'join' command to pass through without a referral code
    if let Commands::Join { email } = &cli.command {
        return commands::join::handle_join(email.clone()).await;
    }

    let local_application_command = is_local_command(&cli.command);
    // Local compilation must work even when an unrelated cloud endpoint is
    // misconfigured. Resolve cloud settings only for commands that use them.
    let local_cloud = if local_application_command {
        false
    } else {
        uses_local_cloud()?
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
                | Commands::Dependencies { .. }
                | Commands::Run { .. }
                | Commands::Lsp { .. }
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
        Commands::Editor { .. } => "editor",
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
        Commands::Database { .. } => "database",
        Commands::Extensions { .. } => "extensions",
        Commands::Dependencies { .. } => "dependencies",
        Commands::Ui { .. } => "ui",
        Commands::Run { .. } => "run",
        Commands::Domain { .. } => "domain",
        Commands::Security { .. } => "security",
        Commands::Serve(..) => "serve",
        Commands::Mock(..) => "mock",
        Commands::Deploy { .. } => "deploy",
        Commands::Test { .. } => "test",
        Commands::Eval {
            file: _,
            format: _,
            variant: _,
        } => "eval",
        Commands::Check { .. } => "check",
        Commands::Explain { .. } => "explain",
        Commands::Migrate { .. } => "migrate",
        Commands::Server { .. } => "server",
        Commands::Repl => "repl",
        Commands::Lsp { .. } => "lsp",
        Commands::Scenario { .. } => "scenario",
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
/// Local compilation does not require private-alpha cloud registration. Commands
/// that publish or retrieve cloud data retain their existing access checks.
fn is_local_command(command: &Commands) -> bool {
    match command {
        Commands::Editor { .. } => true,
        Commands::Scenario { .. } => true,
        Commands::Package { .. }
        | Commands::Packages { .. }
        | Commands::Database { .. }
        | Commands::Inspect { .. }
        | Commands::Eval { .. }
        | Commands::Check { .. }
        | Commands::Explain { .. }
        | Commands::Migrate { .. }
        | Commands::Server { .. }
        | Commands::Contract { .. }
        | Commands::Init { .. }
        | Commands::Domain { .. }
        | Commands::Security { .. }
        | Commands::Run { .. }
        | Commands::Lsp { .. }
        | Commands::Repl
        | Commands::Onboard { contract: None, .. }
        | Commands::Diff { .. }
        | Commands::Test { .. }
        | Commands::Build { release: false, .. }
        | Commands::Serve(..)
        | Commands::Watch { build: true, .. } => true,
        Commands::Mock(input) => !input.cloud,
        Commands::Pull {
            source,
            contract,
            contract_config: None,
            ..
        } => source
            .as_deref()
            .or(contract.as_deref())
            .is_some_and(commands::pull::is_local_artifact_source),
        Commands::Install { package, module } => {
            package == "lsp-server"
                || (!module && commands::app::is_axiom_application(Path::new(package)))
        }
        _ => false,
    }
}

async fn execute_command(command: &Commands) -> anyhow::Result<()> {
    match command {
        Commands::Editor { action } => commands::editor::handle(action),
        Commands::Check {
            file,
            variant,
            json,
            target,
            lock,
        } => commands::contract_build::check(
            file,
            commands::contract_build::Options {
                variant: variant.as_deref(),
                target: target.as_deref(),
                lock: lock.as_deref(),
                ..Default::default()
            },
            *json,
        ),
        Commands::Explain {
            file,
            variant,
            schema,
            provenance,
        } => commands::backend::explain(file.as_deref(), variant.as_deref(), *schema, *provenance),
        Commands::Migrate { file, output } => commands::backend::migrate(file, output),
        Commands::Server { action } => commands::server::handle_async(action).await,
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
                launch,
            } => {
                commands::ui::handle_run_with_packages(
                    source.clone(),
                    ui_lock.clone(),
                    package_lock.clone(),
                    target.clone(),
                    *once,
                    *launch,
                )
                .await
            }
        },
        Commands::Extensions { action } => match action {
            ExtensionsAction::PrepareIde {
                alias,
                deps,
                interface,
                json,
            } => {
                let result = acore::editor::foreign::prepare(deps, alias, interface.as_deref())?;
                if *json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!(
                        "Prepared {}: {}",
                        alias,
                        result["receipt"].as_str().unwrap_or("")
                    );
                }
                Ok(())
            }
            ExtensionsAction::Interface {
                alias,
                deps,
                authority_lock,
                target,
                out,
                json,
            } => {
                commands::extensions::handle_interface(
                    deps.clone(),
                    alias.clone(),
                    authority_lock.clone(),
                    target.clone(),
                    out.clone(),
                    *json,
                )
                .await
            }
            ExtensionsAction::InterfaceDiff {
                before,
                after,
                json,
            } => {
                commands::extensions::handle_interface_diff(before.clone(), after.clone(), *json)
                    .await
            }
            ExtensionsAction::RustInspect {
                alias,
                deps,
                view,
                json,
            } => {
                commands::extensions::handle_rust_inspect(
                    deps.clone(),
                    alias.clone(),
                    view.clone(),
                    *json,
                )
                .await
            }
            ExtensionsAction::MigrateRust {
                alias,
                deps,
                out,
                write,
                check,
                json,
            } => {
                commands::extensions::handle_migrate_rust(
                    deps.clone(),
                    alias.clone(),
                    out.clone(),
                    *write,
                    *check,
                    *json,
                )
                .await
            }
            ExtensionsAction::TypeScriptInspect {
                alias,
                deps,
                view,
                json,
            } => {
                commands::extensions::handle_typescript_inspect(
                    deps.clone(),
                    alias.clone(),
                    view.clone(),
                    *json,
                )
                .await
            }
            ExtensionsAction::PythonInspect {
                alias,
                deps,
                view,
                json,
            } => {
                commands::extensions::handle_python_inspect(
                    deps.clone(),
                    alias.clone(),
                    view.clone(),
                    *json,
                )
                .await
            }
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
                    true,
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
        Commands::Dependencies { action } => match action {
            DependenciesAction::Check { deps, json } => {
                commands::dependencies::handle_check(deps.clone(), *json).await
            }
            DependenciesAction::Format { deps, check, write } => {
                commands::dependencies::handle_format(deps.clone(), *check, *write).await
            }
            DependenciesAction::Migrate { deps, out, write } => {
                commands::dependencies::handle_migrate(deps.clone(), out.clone(), *write).await
            }
            DependenciesAction::Schema => commands::dependencies::handle_schema().await,
            DependenciesAction::Completions { path, json } => {
                commands::dependencies::handle_completions(path.clone(), *json).await
            }
            DependenciesAction::Hover { path, json } => {
                commands::dependencies::handle_hover(path.clone(), *json).await
            }
            DependenciesAction::Resolve {
                deps,
                lock,
                cache,
                workspace_view,
                locked,
                offline,
                json,
            } => {
                commands::dependencies::handle_resolve(
                    deps.clone(),
                    lock.clone(),
                    cache.clone(),
                    workspace_view.clone(),
                    *locked,
                    *offline,
                    *json,
                )
                .await
            }
            DependenciesAction::Verify { lock, cache, json } => {
                commands::dependencies::handle_verify(lock.clone(), cache.clone(), *json).await
            }
            DependenciesAction::Inspect { lock, json } => {
                commands::dependencies::handle_inspect(lock.clone(), *json).await
            }
            DependenciesAction::Cache { action } => match action {
                DependencyCacheAction::Verify { cache, json } => {
                    commands::dependencies::handle_cache_verify(cache.clone(), *json).await
                }
                DependencyCacheAction::Prune { cache, locks, json } => {
                    commands::dependencies::handle_cache_prune(cache.clone(), locks.clone(), *json)
                        .await
                }
                DependencyCacheAction::Recover { lock, cache, json } => {
                    commands::dependencies::handle_cache_recover(lock.clone(), cache.clone(), *json)
                        .await
                }
            },
        },
        Commands::Database { action } => match action {
            DatabaseAction::Runtime { command } => commands::database_runtime::run(command),
            DatabaseAction::Sqlite { command } => commands::database_sqlite::run(command),
            DatabaseAction::Fleet { command } => commands::database_fleet::run(command).await,
            DatabaseAction::Check { source } => commands::database::check(source),
            DatabaseAction::Build { source, out } => {
                commands::database::build(source, out.as_deref())
            }
            DatabaseAction::Inspect { source } => commands::database::inspect(source),
            DatabaseAction::Diff { before, after } => commands::database::diff(before, after),
            DatabaseAction::Fmt { source, write } => commands::database::format(source, *write),
            DatabaseAction::Catalog => commands::database::catalog(),
            DatabaseAction::BindingCheck { deployment } => {
                commands::database::binding_check(deployment)
            }
            DatabaseAction::InspectLive { deployment, source } => {
                commands::database::inspect_live(deployment, source.as_deref(), None).await
            }
            DatabaseAction::ImportSql {
                input,
                namespace,
                database,
                schemas,
                postgres_major,
                out,
            } => commands::database::import(
                input,
                namespace,
                database,
                schemas,
                *postgres_major,
                "sql",
                out.as_deref(),
            ),
            DatabaseAction::ImportHcl {
                input,
                namespace,
                database,
                schemas,
                postgres_major,
                out,
            } => commands::database::import(
                input,
                namespace,
                database,
                schemas,
                *postgres_major,
                "hcl",
                out.as_deref(),
            ),
            DatabaseAction::Adopt {
                deployment,
                source,
                out,
            } => {
                commands::database::inspect_live(deployment, source.as_deref(), out.as_deref())
                    .await
            }
            DatabaseAction::VerifyIsolation { deployment, source } => {
                commands::database::verify_isolation(deployment, source.as_deref()).await
            }
            DatabaseAction::Graph { input } => commands::database::graph(input, None),
            DatabaseAction::Explain { input, id } => commands::database::graph(input, Some(id)),
            DatabaseAction::EvolutionCheck {
                source,
                previous,
                evolution,
            } => commands::database::evolution_check(source, previous.as_deref(), evolution),
            DatabaseAction::Plan {
                source,
                deployment,
                dev_profile,
                baseline,
                parent,
                tests,
                evolution,
                checkpoint,
            } => {
                commands::database::plan(
                    source,
                    deployment,
                    dev_profile,
                    baseline.as_deref(),
                    parent.as_deref(),
                    tests.as_deref(),
                    evolution.as_deref(),
                    checkpoint.as_deref(),
                )
                .await
            }
            DatabaseAction::History { action } => match action {
                DatabaseHistoryAction::Export { source, head, out } => {
                    commands::database::history_export(source, head.as_deref(), out)
                }
                DatabaseHistoryAction::Checkpoint { source, head, out } => {
                    commands::database::history_checkpoint(source, head.as_deref(), out)
                }
                DatabaseHistoryAction::Verify { bundle } => {
                    commands::database::history_verify(bundle)
                }
                DatabaseHistoryAction::Import { bundle, source } => {
                    commands::database::history_import(bundle, source)
                }
                DatabaseHistoryAction::ImportAtlas {
                    bundle,
                    directory,
                    deployment,
                    revision_schema,
                    revision_table,
                    legacy_stopped,
                } => {
                    commands::database::history_import_atlas(
                        bundle,
                        directory,
                        deployment,
                        revision_schema,
                        revision_table,
                        *legacy_stopped,
                    )
                    .await
                }
            },
            DatabaseAction::Release { action } => match action {
                DatabaseReleaseAction::Keygen { out } => commands::database::release_keygen(out),
                DatabaseReleaseAction::Sign {
                    bundle,
                    key_file,
                    allow_destructive,
                    allow_data_dependent,
                } => commands::database::release_sign(
                    bundle,
                    key_file,
                    *allow_destructive,
                    *allow_data_dependent,
                ),
                DatabaseReleaseAction::RepairIndex {
                    bundle,
                    deployment,
                    index_id,
                    index_oid,
                    strategy,
                    key_file,
                    out,
                } => commands::database::release_repair_index(
                    bundle, deployment, index_id, index_oid, strategy, key_file, out,
                ),
                DatabaseReleaseAction::Verify {
                    bundle,
                    trusted_key,
                } => commands::database::release_verify(bundle, trusted_key),
            },
            DatabaseAction::Target { action } => match action {
                DatabaseTargetAction::BindingCheck { deployment } => {
                    commands::database::target_binding_check(deployment)
                }
                DatabaseTargetAction::Register { bundle, deployment } => {
                    commands::database::target_operation(
                        bundle,
                        deployment,
                        axiom_database::execution::Action::Register,
                    )
                    .await
                }
                DatabaseTargetAction::Status { bundle, deployment } => {
                    commands::database::target_operation(
                        bundle,
                        deployment,
                        axiom_database::execution::Action::Status,
                    )
                    .await
                }
                DatabaseTargetAction::Verify { bundle, deployment } => {
                    commands::database::target_operation(
                        bundle,
                        deployment,
                        axiom_database::execution::Action::Verify,
                    )
                    .await
                }
                DatabaseTargetAction::Reconcile {
                    bundle,
                    deployment,
                    repair,
                } => {
                    commands::database::target_reconcile(bundle, deployment, repair.as_deref())
                        .await
                }
            },
            DatabaseAction::Apply { bundle, deployment } => {
                commands::database::target_operation(
                    bundle,
                    deployment,
                    axiom_database::execution::Action::Apply,
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
                launch,
            } => {
                commands::ui::handle_run(
                    source.clone(),
                    lock.clone(),
                    target.clone(),
                    *once,
                    *launch,
                )
                .await
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
                suite,
                layer,
                report,
            } => {
                commands::ui::handle_test_options(
                    source.clone(),
                    lock.clone(),
                    target.clone(),
                    suite.clone(),
                    *layer,
                    report.clone(),
                )
                .await
            }
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
            launch,
            variant,
            deployment,
            bindings,
            delivery,
            fixtures,
            identities,
            watch,
        } => {
            if commands::app::is_axiom_application(source) {
                if mode.is_some()
                    || variant.is_some()
                    || deployment.is_some()
                    || bindings.is_some()
                    || delivery.is_some()
                    || fixtures.is_some()
                    || identities.is_some()
                    || *watch
                {
                    anyhow::bail!("--mode, --variant, --deployment, --bindings, --delivery, --fixtures, --identities and --watch apply only to backend .acore sources");
                }
                if *once {
                    anyhow::bail!("--once applies to authored .acore sessions, not packaged .axiomapp execution");
                }
                commands::app::handle_run(source.clone(), target.clone(), *launch).await
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
                        if *launch {
                            anyhow::bail!("--launch applies to frontend .acore sources, not backend execution");
                        }
                        commands::run::handle_backend(
                            source.clone(),
                            mode.unwrap_or(commands::run::RunMode::Service),
                            host.clone(),
                            *port,
                            *debug,
                            variant.clone(),
                            deployment.clone(),
                            bindings.clone(),
                            delivery.clone(),
                            fixtures.clone(),
                            identities.clone(),
                            *watch,
                        )
                        .await
                    }
                    commands::run::AcoreSourceKind::Frontend => {
                        if mode.is_some()
                            || variant.is_some()
                            || deployment.is_some()
                            || bindings.is_some()
                            || delivery.is_some()
                            || fixtures.is_some()
                            || identities.is_some()
                            || *watch
                        {
                            anyhow::bail!(
                                "--mode, --variant, --deployment, --bindings, --delivery, --fixtures, --identities and --watch apply only to backend .acore sources"
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
                            *launch,
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
        Commands::Test { file, tag, variant } => {
            commands::test::handle_test(file.clone(), tag.clone(), variant.clone()).await
        }
        Commands::Scenario { action } => commands::scenario::handle(action).await,
        Commands::Serve(input) => commands::serve::handle(input.clone()).await,
        Commands::Mock(input) => commands::mock::handle(input.clone()).await,
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
        Commands::Lsp {
            version_json,
            version,
        } => {
            if *version_json || *version {
                commands::lsp::print_version(*version_json);
            } else {
                acore::server::run_server().await;
            }
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

            let uri = format!(
                "file://{}",
                file.canonicalize()
                    .map_err(|error| anyhow::anyhow!("Cannot read {}: {error}", file.display()))?
                    .display()
            );
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
            if package == "lsp-server" {
                if *module {
                    anyhow::bail!("install lsp-server does not accept --module");
                }
                commands::lsp::install()?;
                Ok(())
            } else if *module {
                acore::package::install_tool(package)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                Ok(())
            } else if commands::app::is_axiom_application(Path::new(package)) {
                commands::app::handle_install(PathBuf::from(package)).await
            } else {
                anyhow::bail!("Unsupported install target `{package}`. Use `install lsp-server`, `install FILE.axiomapp`, or `install MODULE --module`.")
            }
        }
        Commands::Package { artifacts, out } => {
            commands::app::handle_package(artifacts.clone(), out.clone()).await
        }

        Commands::Build {
            file,
            out,
            target,
            lock,
            variant,
            compatibility_baseline,
            release,
            project,
            version,
        } => {
            let output = commands::contract_build::build(
                Path::new(file),
                out.as_deref(),
                commands::contract_build::Options {
                    variant: variant.as_deref(),
                    target: target.as_deref(),
                    lock: lock.as_deref(),
                    compatibility_baseline: compatibility_baseline.as_deref(),
                    release: *release,
                },
            )
            .await?;
            if *release {
                commands::release::handle_release(
                    output
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("Artifact path is not UTF-8"))?,
                    project.as_deref(),
                    version.as_deref(),
                    Some(Path::new(file)),
                    variant.as_deref().unwrap_or("default"),
                )
                .await?;
            }
            Ok(())
        }
        Commands::Release {
            file_path,
            project,
            version,
        } => {
            let path = file_path.clone().unwrap_or_else(default_backend_artifact);
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
            let path = path.clone().unwrap_or_else(default_backend_artifact);
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
            if commands::contract_build::inspect(&path)? {
                return Ok(());
            }
            handle_inspect(&path).await
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
        Commands::Watch { build, variant } => {
            if *build {
                commands::watch::handle_watch_dynamic(true, variant.clone()).await
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

fn default_backend_artifact() -> PathBuf {
    ["backend.axiom", "axiom.axiom", ".axiom"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("backend.axiom"))
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

#[cfg(test)]
mod launch_tests {
    use super::*;

    #[test]
    fn database_controlled_execution_requires_a_bundle_and_explicit_binding() {
        for action in ["register", "status", "verify", "reconcile"] {
            assert!(Cli::try_parse_from([
                "axiom",
                "database",
                "target",
                action,
                "private-release",
                "--deployment",
                "target.json"
            ])
            .is_ok());
            assert!(Cli::try_parse_from([
                "axiom",
                "database",
                "target",
                action,
                "private-release"
            ])
            .is_err());
        }
        assert!(Cli::try_parse_from([
            "axiom",
            "database",
            "apply",
            "private-release",
            "--deployment",
            "target.json"
        ])
        .is_ok());
        assert!(Cli::try_parse_from(["axiom", "database", "apply", "private-release"]).is_err());
        assert!(Cli::try_parse_from([
            "axiom",
            "database",
            "release",
            "sign",
            "private-release",
            "--key-file",
            "signing-key.json",
            "--allow-data-dependent"
        ])
        .is_ok());
        assert!(Cli::try_parse_from([
            "axiom",
            "database",
            "release",
            "verify",
            "private-release",
            "--trusted-key",
            "public-key"
        ])
        .is_ok());
        assert!(Cli::try_parse_from([
            "axiom",
            "database",
            "history",
            "import",
            "private-release",
            "--source",
            "schema.acore"
        ])
        .is_ok());
        assert!(Cli::try_parse_from([
            "axiom",
            "database",
            "history",
            "import-atlas",
            "private-release",
            "--directory",
            "atlas-history",
            "--deployment",
            "target.json",
            "--revision-schema",
            "public",
            "--legacy-stopped"
        ])
        .is_ok());
        assert!(Cli::try_parse_from([
            "axiom",
            "database",
            "history",
            "import-atlas",
            "private-release",
            "--directory",
            "atlas-history",
            "--deployment",
            "target.json",
            "--revision-schema",
            "public"
        ])
        .is_err());
        assert!(
            Cli::try_parse_from(["axiom", "database", "apply", "--sql", "injected.sql"]).is_err()
        );
    }

    #[test]
    fn database_authoring_and_observation_are_local_without_apply() {
        for args in [
            vec!["axiom", "database", "check", "schema.acore"],
            vec!["axiom", "database", "build", "schema.acore"],
            vec!["axiom", "database", "inspect", "schema.axiom"],
            vec!["axiom", "database", "diff", "before.axiom", "after.axiom"],
            vec!["axiom", "database", "fmt", "schema.acore", "--write"],
            vec!["axiom", "database", "catalog"],
            vec!["axiom", "database", "binding-check", "deployment.json"],
            vec![
                "axiom",
                "database",
                "inspect-live",
                "--deployment",
                "deployment.json",
            ],
            vec![
                "axiom",
                "database",
                "import-sql",
                "schema.sql",
                "--namespace",
                "example.storage",
                "--database",
                "storage",
                "--schema",
                "app",
            ],
            vec![
                "axiom",
                "database",
                "import-hcl",
                "schema.hcl",
                "--namespace",
                "example.storage",
                "--database",
                "storage",
                "--schema",
                "app",
            ],
            vec![
                "axiom",
                "database",
                "adopt",
                "--deployment",
                "deployment.json",
            ],
            vec![
                "axiom",
                "database",
                "verify-isolation",
                "--deployment",
                "deployment.json",
                "--source",
                "schema.acore",
            ],
            vec!["axiom", "database", "graph", "observation.json"],
            vec![
                "axiom",
                "database",
                "plan",
                "schema.acore",
                "--deployment",
                "target.json",
                "--dev-profile",
                "dev.json",
                "--baseline",
                "empty",
            ],
            vec![
                "axiom",
                "database",
                "history",
                "export",
                "schema.acore",
                "--out",
                "private-bundle",
            ],
            vec!["axiom", "database", "history", "verify", "private-bundle"],
            vec![
                "axiom",
                "database",
                "explain",
                "observation.json",
                "catalog-object",
            ],
        ] {
            let cli = Cli::try_parse_from(&args).unwrap();
            assert!(is_local_command(&cli.command), "{args:?}");
        }
        for action in ["connect", "sql", "deploy"] {
            assert!(Cli::try_parse_from(["axiom", "database", action]).is_err());
        }
        assert!(Cli::try_parse_from(["axiom", "database", "inspect-live"]).is_err());
    }

    #[test]
    fn frontend_package_commands_are_local_compiler_work() {
        for args in [
            vec![
                "axiom",
                "packages",
                "build",
                "package.json",
                "--out",
                "package.axiom",
            ],
            vec!["axiom", "packages", "resolve"],
            vec!["axiom", "packages", "verify"],
            vec!["axiom", "packages", "inspect", "widgets"],
            vec!["axiom", "packages", "diff", "before.axiom", "after.axiom"],
            vec!["axiom", "packages", "check-source", "main.acore"],
            vec!["axiom", "packages", "run", "main.acore", "--once"],
        ] {
            let cli = Cli::try_parse_from(&args).unwrap();
            assert!(is_local_command(&cli.command), "{args:?}");
        }
    }

    #[test]
    fn backend_local_commands_do_not_require_cloud_registration() {
        for args in [
            vec!["axiom", "eval", "contract.acore"],
            vec!["axiom", "check"],
            vec!["axiom", "domain", "validate"],
            vec!["axiom", "security", "check"],
            vec!["axiom", "lsp"],
            vec!["axiom", "contract", "resolve"],
            vec!["axiom", "contract", "verify"],
            vec!["axiom", "explain", "contract.acore"],
            vec!["axiom", "explain", "--schema"],
            vec!["axiom", "migrate", "old.acore", "--output", "new.acore"],
            vec!["axiom", "init", "main.py:app", "--module", "axiom-fastapi"],
            vec!["axiom", "onboard", "--role", "backend", "--apply"],
            vec!["axiom", "build"],
            vec!["axiom", "diff", "contract.acore"],
            vec!["axiom", "test"],
            vec!["axiom", "serve", "contract.acore"],
            vec!["axiom", "serve"],
            vec!["axiom", "mock"],
            vec!["axiom", "mock", "contract.acore", "--profile", "contract"],
            vec![
                "axiom",
                "serve",
                "contract.acore",
                "--mock",
                "--fixtures",
                "mock.json",
            ],
            vec!["axiom", "watch", "--build"],
            vec!["axiom", "run", "contract.acore"],
            vec![
                "axiom",
                "server",
                "plan",
                "axiom.axiom",
                "--output",
                "axiom.server.json",
            ],
            vec![
                "axiom",
                "server",
                "inspect",
                "axiom.axiom",
                "axiom.server.json",
            ],
        ] {
            let cli = Cli::try_parse_from(&args).unwrap();
            assert!(is_local_command(&cli.command), "{args:?}");
        }
        for args in [
            vec!["axiom", "build", "--release"],
            vec!["axiom", "release"],
            vec!["axiom", "deploy", "mock-server"],
            vec!["axiom", "mock", "--cloud"],
            vec!["axiom", "watch"],
            vec!["axiom", "pull"],
            vec![
                "axiom",
                "onboard",
                "--role",
                "frontend",
                "--contract",
                "org/project",
            ],
        ] {
            let cli = Cli::try_parse_from(&args).unwrap();
            assert!(!is_local_command(&cli.command), "{args:?}");
        }
    }

    #[test]
    fn backend_variant_and_baseline_flags_are_available() {
        for command in ["check", "explain", "build", "test", "serve", "diff"] {
            assert!(Cli::try_parse_from([
                "axiom",
                command,
                "contract.acore",
                "--variant",
                "mobile"
            ])
            .is_ok());
        }
        assert!(Cli::try_parse_from([
            "axiom",
            "build",
            "--compatibility-baseline",
            "released.axiom"
        ])
        .is_ok());
        assert!(Cli::try_parse_from(["axiom", "serve", "--variant", "mobile"]).is_ok());
        assert!(Cli::try_parse_from(["axiom", "explain"]).is_err());
        assert!(Cli::try_parse_from(["axiom", "explain", "contract.acore", "--schema"]).is_err());
    }

    #[test]
    fn run_does_not_launch_unless_requested() {
        let default =
            Cli::try_parse_from(["axiom", "run", "main.acore", "--target", "web"]).unwrap();
        assert!(matches!(
            default.command,
            Commands::Run { launch: false, .. }
        ));
        let explicit =
            Cli::try_parse_from(["axiom", "run", "main.acore", "--target", "web", "--launch"])
                .unwrap();
        assert!(matches!(
            explicit.command,
            Commands::Run { launch: true, .. }
        ));
        assert!(Cli::try_parse_from(["axiom", "run", "main.acore", "--once", "--launch"]).is_err());
    }
}

#[cfg(test)]
mod development_scenario_commands {
    use super::*;
    #[test]
    fn development_scenarios_are_offline_and_require_explicit_artifacts() {
        for args in [
            vec!["axiom", "scenario", "catalog", "axiom.axiom", "--plan", "axiom.server.json"],
            vec!["axiom", "scenario", "check", "axiom.axiom", "--plan", "axiom.server.json", "--scenario", "test.scenario.json"],
            vec!["axiom", "scenario", "run", "axiom.axiom", "--plan", "axiom.server.json", "--deployment", "deployment.json", "--scenario", "test.scenario.json"],
        ] { let cli=Cli::try_parse_from(args).unwrap();assert!(is_local_command(&cli.command)); }
        assert!(Cli::try_parse_from(["axiom","scenario","run","axiom.axiom","--plan","axiom.server.json","--scenario","test.scenario.json"]).is_err());
    }
}
