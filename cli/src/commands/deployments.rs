//! `axiom deploy frontend|backend` and `axiom deployments …`: hosted deployments of a
//! project's configuration environment through `axiom-deployments/v1`
//! (internal-docs planning/deployments/B-W0-REVIEW.md §3.7, B-W0-FRONTEND-AND-PACKAGING.md §6).
//! The dashboard sends the same requests; `cli/tests/fixtures/deployments-parity.json`
//! records them.
//!
//! `axiom deploy` checks the project link and the folder before any sign-in or request
//! ([`prepare_frontend`], [`prepare_backend`]; an activating deploy without `--yes` and
//! without a terminal is refused right after, still sending nothing). Then: one read of the
//! deployment (a project outside the hosting pilot is told so here, before hashing,
//! packaging or the y/N question) → hash the files and build the manifest → ask y/N on a
//! terminal → create upload → PUT the missing objects
//! to their presigned URLs → complete → create version → wait until ready → activate
//! (unless `--no-activate`) with `If-Match` from the slot's `ETag` → wait → print the URL.
//!
//! Resume journal: `<CLI config dir>/deployments/<project>-<environment id>-<kind>.json`
//! (beside `auth.json`, per user and per Cloud profile; never inside the repository). It holds
//! the intent id, manifest SHA-256, original slot generation and deployment id (generation 0
//! when no live slot existed: the slot this deploy creates starts at generation 1, and that is
//! the activation's precondition), upload/version/operation ids and
//! each step's result. Every mutation's `Idempotency-Key` is the hex SHA-256 of
//! `intent|step|manifest_sha256|generation`, so a rerun after a crash or lost response
//! replays the same requests instead of starting over. A rerun after a successful activation
//! reports that result; `--new` (or a different build) starts a fresh intent; a version that
//! failed, or a refused request, ends the intent so the next run starts fresh.
use super::deploy_bundle::{self as bundle, Bundle, Spa};
use anyhow::{anyhow, bail, Context, Result};
use axiom_cloud::deployments::{bodies, next_step, DeployError, Kind, LogQuery, Operation, Scope, Slot, Upload};
use axiom_cloud::CloudClient;
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const SCHEMA: &str = axiom_cloud::deployments::SCHEMA;
/// Concurrent object uploads.
const UPLOAD_CONCURRENCY: usize = 4;

/// Where and as whom: shared by every deploy and deployments command.
#[derive(Args, Debug, Clone)]
pub struct Target {
    /// Cloud project: slug, name or id (default: the project linked to this directory)
    #[arg(long, global = true)]
    pub project: Option<String>,
    /// Configuration environment: name or id
    #[arg(long, global = true, default_value = "development")]
    pub environment: String,
    /// Print one JSON object on stdout; progress stays on stderr
    #[arg(long, global = true)]
    pub json: bool,
}

/// Flags of `axiom deploy frontend` and `axiom deploy backend`.
#[derive(Args, Debug, Clone)]
pub struct DeployFlags {
    /// Build and check the version but do not make it live (no confirmation needed)
    #[arg(long)]
    pub no_activate: bool,
    /// Accepted for scripts: deploy always waits until the version is ready (and live)
    #[arg(long)]
    pub wait: bool,
    /// Make the version live without asking (required without a terminal)
    #[arg(long)]
    pub yes: bool,
    /// A note stored with the version
    #[arg(long)]
    pub message: Option<String>,
    /// Start a fresh deploy instead of resuming or reporting the last one of this exact build
    #[arg(long)]
    pub new: bool,
    #[command(flatten)]
    pub target: Target,
}

#[derive(Args, Debug, Clone)]
pub struct FrontendArgs {
    /// Build output folder (default: the first of dist/, build/, out/)
    pub dir: Option<PathBuf>,
    /// Single-page app: unknown page paths serve index.html. Default: on, unless the folder has its own 404.html
    #[arg(long, conflicts_with = "no_spa")]
    pub spa: bool,
    /// Unknown paths answer 404 instead of index.html
    #[arg(long)]
    pub no_spa: bool,
    #[command(flatten)]
    pub flags: DeployFlags,
}

#[derive(Args, Debug, Clone)]
pub struct BackendArgs {
    /// Upload this container input folder (`axiom server package … --output-input DIR`) instead of packaging this project
    #[arg(long, value_name = "DIR")]
    pub from_input: Option<PathBuf>,
    /// Backend contract to package (default: backend.axiom, then axiom.axiom)
    #[arg(long, conflicts_with = "from_input")]
    pub contract: Option<PathBuf>,
    /// Server plan (default: <contract name>.server.json beside the contract)
    #[arg(long, conflicts_with = "from_input")]
    pub plan: Option<PathBuf>,
    /// Deployment file (default: deployment.json)
    #[arg(long, conflicts_with = "from_input")]
    pub deployment: Option<PathBuf>,
    #[command(flatten)]
    pub flags: DeployFlags,
}

#[derive(Args, Debug)]
pub struct DeploymentsArgs {
    #[command(flatten)]
    pub target: Target,
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum KindArg {
    Frontend,
    Backend,
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Kind {
        match k {
            KindArg::Frontend => Kind::Frontend,
            KindArg::Backend => Kind::Backend,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Stream {
    /// Deploys, activations, rollbacks and deletes
    Lifecycle,
    /// Application output (not available yet)
    Runtime,
    /// Requests served (not available yet)
    Access,
}

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Every deployment of the environment: state, live version and URL
    List,
    /// One deployment: URL, state, live/desired/previous version and the last operation
    #[command(visible_alias = "show")]
    Status { kind: KindArg },
    /// Versions of a deployment, newest first; `*` marks the live one
    Versions { kind: KindArg },
    /// Make a ready version live
    Activate {
        kind: KindArg,
        /// Version id (`ver_…`, see `axiom deployments versions`)
        version: String,
        /// Wait until the version is live
        #[arg(long)]
        wait: bool,
        /// Do not ask for confirmation (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// Make the previous version live again, or --to another ready version
    Rollback {
        kind: KindArg,
        /// Version id to roll back to (default: the previous version)
        #[arg(long, value_name = "VERSION")]
        to: Option<String>,
        /// Wait until the rollback is live
        #[arg(long)]
        wait: bool,
        /// Do not ask for confirmation (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// Logs of a deployment: lifecycle events (deploys, activations, rollbacks, deletes) by default
    Logs {
        kind: KindArg,
        /// Start: a duration ago (30s, 10m, 1h, 2d) or an RFC 3339 time
        #[arg(long)]
        since: Option<String>,
        /// End: a duration ago (30s, 10m, 1h, 2d) or an RFC 3339 time
        #[arg(long)]
        until: Option<String>,
        /// Only this version
        #[arg(long)]
        version: Option<String>,
        #[arg(long, value_enum, default_value = "lifecycle")]
        stream: Stream,
    },
    /// Stop serving a deployment and delete it
    Delete {
        kind: KindArg,
        /// Wait until the delete has finished
        #[arg(long)]
        wait: bool,
        /// Do not ask for confirmation (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// Health of the live version
    Health { kind: KindArg },
}

// ---------------------------------------------------------------------------------------
// Confirmation (asked in `main::confirmation_preflight`, before registration or requests)

/// `Some(yes)` when an `axiom deployments` action changes traffic and must be confirmed.
pub(crate) fn asks_first(args: &DeploymentsArgs) -> Option<bool> {
    match &args.action {
        Action::Activate { yes, .. } | Action::Rollback { yes, .. } | Action::Delete { yes, .. } => Some(*yes),
        _ => None,
    }
}

fn place(t: &Target) -> String {
    let project = t.project.as_deref().map(|p| format!("project {}", super::cloud_cli::clean(p))).unwrap_or_else(|| "the linked project".into());
    format!("{project} ({})", super::cloud_cli::clean(&t.environment))
}

/// The y/N question for a deploy that makes a version live.
pub(crate) fn deploy_question(kind: Kind, dir: Option<&Path>, t: &Target) -> String {
    let found = || std::env::current_dir().ok().and_then(|cwd| bundle::default_frontend_dir(&cwd).ok()).and_then(|d| d.file_name().map(|n| format!("{}/", n.to_string_lossy())));
    let what = match (kind, dir) {
        (Kind::Frontend, Some(d)) => format!("{} as the frontend", d.display()),
        (Kind::Frontend, None) => format!("{} as the frontend", found().unwrap_or_else(|| "the build output".into())),
        (Kind::Backend, Some(d)) => format!("{} as the backend", d.display()),
        (Kind::Backend, None) => "this project's backend".into(),
    };
    format!("Deploy {what} of {} and make it live?", place(t))
}

/// The y/N question for `axiom deployments activate|rollback|delete`.
pub(crate) fn action_question(args: &DeploymentsArgs) -> Option<String> {
    let at = place(&args.target);
    match &args.action {
        Action::Activate { kind, version, .. } => Some(format!("Make {} the live {} of {at}?", super::cloud_cli::clean(version), Kind::from(*kind))),
        Action::Rollback { kind, to, .. } => Some(format!("Roll the {} of {at} back to {}?", Kind::from(*kind), to.as_deref().map(super::cloud_cli::clean).unwrap_or_else(|| "the previous version".into()))),
        Action::Delete { kind, .. } => Some(format!("Delete the {} deployment of {at}? Its URL stops serving.", Kind::from(*kind))),
        _ => None,
    }
}

/// Ask on the terminal; `false` when declined.
pub(crate) fn ask(question: &str) -> bool {
    dialoguer::Confirm::new().with_prompt(question).default(false).interact().unwrap_or(false)
}

// ---------------------------------------------------------------------------------------
// Local preflight (before registration, sign-in and any request)

/// The local half of a deploy, checked in `main` before registration, sign-in or any request.
pub enum Pending {
    /// A folder that passed its checks; hashed after the pilot check.
    Checked(bundle::Checked),
    /// `axiom deploy backend` without `--from-input`: the files to package this project from.
    Package { contract: PathBuf, plan: PathBuf, deployment: PathBuf },
}

impl Pending {
    /// Hash the checked folder (or package this project and check its input): the manifest
    /// the upload is keyed by. The temporary folder of a packaged input lives as long as the
    /// returned guard.
    pub fn finish(self) -> Result<(Bundle, Option<tempfile::TempDir>)> {
        match self {
            Pending::Checked(c) => Ok((c.hash()?, None)),
            Pending::Package { contract, plan, deployment } => {
                let temp = tempfile::tempdir()?;
                let input = temp.path().join("input");
                axiom_server::container::package_input_only(&contract, &plan, &deployment, None, "container-amd64", &input, None)
                    .with_context(|| "Packaging the backend failed (the same check as `axiom server package --output-input`)".to_string())?;
                Ok((bundle::backend(&input)?, Some(temp)))
            }
        }
    }
}

static PREPARED: Mutex<Option<Pending>> = Mutex::new(None);

fn spa_choice(args: &FrontendArgs) -> Spa {
    if args.spa {
        Spa::On
    } else if args.no_spa {
        Spa::Off
    } else {
        Spa::Auto
    }
}

/// The project comes from `--project` or the link of this directory; without either the
/// deploy cannot start, which is said before any question or request.
pub(crate) fn check_link(t: &Target, cwd: &Path) -> Result<()> {
    if t.project.as_deref().map(str::trim).is_some_and(|p| !p.is_empty()) {
        return Ok(());
    }
    match crate::auth_store::get_project_id(cwd)? {
        Some(_) => Ok(()),
        None => bail!("This directory is not linked to an Axiom project. Link it with `axiom project link` (after `axiom login`), or pass --project <slug>."),
    }
}

fn frontend_pending(args: &FrontendArgs, cwd: &Path) -> Result<Pending> {
    let dir = match &args.dir {
        Some(d) => d.clone(),
        None => bundle::default_frontend_dir(cwd)?,
    };
    Ok(Pending::Checked(bundle::check_frontend(&dir, spa_choice(args))?))
}

#[cfg(test)]
fn frontend_bundle(args: &FrontendArgs, cwd: &Path) -> Result<Bundle> {
    Ok(frontend_pending(args, cwd)?.finish()?.0)
}

/// The link and the folder (no file is read): `axiom deploy frontend` exits 1 on failure.
pub fn prepare_frontend(args: &FrontendArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    check_link(&args.flags.target, &cwd)?;
    *PREPARED.lock().unwrap() = Some(frontend_pending(args, &cwd)?);
    Ok(())
}

/// The link, then `--from-input` (its input.json and the files it lists) or the files to
/// package this project from, all locally.
pub fn prepare_backend(args: &BackendArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    check_link(&args.flags.target, &cwd)?;
    *PREPARED.lock().unwrap() = Some(backend_pending(args, &cwd)?);
    Ok(())
}

/// What `axiom deploy backend` packages itself (no flags needed): the hint every
/// package-input error ends with.
const PACKAGE_HINT: &str = "or upload a folder you packaged yourself: `axiom server package <contract> <plan> --deployment <file> --target container-amd64 --output-input <dir>`, then `axiom deploy backend --from-input <dir>`";

fn backend_pending(args: &BackendArgs, cwd: &Path) -> Result<Pending> {
    if let Some(dir) = &args.from_input {
        return Ok(Pending::Checked(bundle::check_backend(dir)?));
    }
    let found = |given: &Option<PathBuf>, candidates: &[&str]| -> Option<PathBuf> {
        match given {
            Some(p) => Some(p.clone()),
            None => candidates.iter().map(|c| cwd.join(c)).find(|p| p.is_file()),
        }
    };
    let contract = found(&args.contract, &["backend.axiom", "axiom.axiom"]).filter(|p| p.is_file()).with_context(|| match &args.contract {
        Some(p) => format!("No backend contract at {}. Build it with `axiom build --out backend.axiom` and pass that file to --contract, {PACKAGE_HINT}.", p.display()),
        None => format!("No backend contract found (looked for backend.axiom and axiom.axiom). Build it with `axiom build --out backend.axiom` and run `axiom deploy backend` again (it packages the project itself), pass --contract <file>, {PACKAGE_HINT}."),
    })?;
    let plan = match &args.plan {
        Some(p) => p.clone(),
        None => {
            let stem = contract.file_stem().and_then(|s| s.to_str()).unwrap_or("backend");
            contract.with_file_name(format!("{stem}.server.json"))
        }
    };
    if !plan.is_file() {
        bail!("No server plan at {} (`axiom build` writes it beside the contract). Build again and run `axiom deploy backend`, pass --plan <file>, {PACKAGE_HINT}.", plan.display());
    }
    let deployment = found(&args.deployment, &["deployment.json"]).filter(|p| p.is_file()).with_context(|| {
        format!(
            "No deployment file found ({}). Create it with `axiom server deployment {} {} --output deployment.json` and run `axiom deploy backend` again, pass --deployment <file>, {PACKAGE_HINT}.",
            args.deployment.as_deref().map(|p| format!("looked for {}", p.display())).unwrap_or_else(|| "looked for deployment.json".into()),
            contract.display(),
            plan.display()
        )
    })?;
    Ok(Pending::Package { contract, plan, deployment })
}

#[cfg(test)]
fn backend_bundle(args: &BackendArgs, cwd: &Path) -> Result<Bundle> {
    Ok(backend_pending(args, cwd)?.finish()?.0)
}

fn take_prepared() -> Option<Pending> {
    PREPARED.lock().unwrap().take()
}

// ---------------------------------------------------------------------------------------
// Context, scope and journal

/// A Cloud client plus the knobs tests shorten.
pub struct Ctx {
    pub client: CloudClient,
    pub poll: Duration,
    pub timeout: Duration,
    pub journal_dir: PathBuf,
    pub cwd: PathBuf,
}

impl Ctx {
    pub fn from_profile() -> Result<Self> {
        Ok(Self {
            // Only the sentence: the missing auth file's "os error 2" is no help.
            client: crate::auth_store::authenticated_cloud_client().map_err(|e| anyhow!(sign_in_error(&e)))?,
            poll: Duration::from_secs(2),
            timeout: Duration::from_secs(30 * 60),
            journal_dir: crate::auth_store::get_config_dir()?.join("deployments"),
            cwd: std::env::current_dir()?,
        })
    }
}

struct Resolved {
    scope: Scope,
    environment: String,
}

async fn resolve(ctx: &Ctx, t: &Target) -> Result<Resolved> {
    let projects = ctx.client.list_projects().await.map_err(|e| anyhow!(projects_error(&e)))?;
    let slug = match t.project.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => super::auth::pick_project(&projects, p)?,
        None => {
            let linked = crate::auth_store::get_project_id(&ctx.cwd)?.context("No project: pass --project <slug>, or link this directory with `axiom project link`")?;
            projects
                .iter()
                .find(|p| p.id == linked || p.slug == linked)
                .map(|p| p.slug.clone())
                .context("The project linked to this directory is not available to you; pass --project <slug>")?
        }
    };
    let environment_id = super::secrets_cloud::environment_id(&ctx.client, &slug, t.environment.trim()).await?;
    Ok(Resolved { scope: Scope::new(slug, environment_id), environment: t.environment.trim().to_string() })
}

/// No usable saved session, in one sentence: none saved (or signed out), or the saved one
/// cannot be read (without the OS error number).
fn sign_in_error(e: &anyhow::Error) -> String {
    let unreadable = e.chain().filter_map(|c| c.downcast_ref::<std::io::Error>()).find(|io| io.kind() != std::io::ErrorKind::NotFound);
    match unreadable {
        Some(io) => {
            let reason = io.to_string();
            let reason = reason.find(" (os error").map(|i| reason[..i].to_string()).unwrap_or(reason);
            format!("Could not read your saved Axiom sign-in ({}). Run `axiom login` again, then re-run this command.", clean(&reason.to_lowercase()))
        }
        None if e.chain().any(|c| c.is::<serde_json::Error>()) => "Your saved Axiom sign-in is damaged. Run `axiom login` again, then re-run this command.".into(),
        None => "Not signed in to Axiom Cloud. Run `axiom login`, then re-run this command.".into(),
    }
}

/// Why the project list could not be read, in one sentence: the network (with the reason, no
/// OS error numbers), the session, or the server's answer.
fn projects_error(e: &anyhow::Error) -> String {
    let network = e.chain().filter_map(|c| c.downcast_ref::<reqwest::Error>()).find(|r| r.is_connect() || r.is_timeout() || r.is_request());
    if let Some(r) = network {
        let reason = if r.is_timeout() {
            "timed out".to_string()
        } else {
            let root = e.root_cause().to_string();
            let root = match root.find(" (os error") {
                Some(i) => root[..i].to_string(),
                None => root,
            };
            let root = clean(root.trim());
            if root.is_empty() { "connection failed".into() } else { root.to_lowercase() }
        };
        return format!("Could not reach Axiom Cloud ({reason}). Check your connection (and AXIOM_CLOUD_URL if you set it), then re-run.");
    }
    let text = format!("{e:#}");
    let lower = text.to_lowercase();
    // A refresh that could not be sent is the network, not the session.
    if lower.contains("failed to send") || lower.contains("error sending request") || lower.contains("connection refused") || lower.contains("timed out") {
        return "Could not reach Axiom Cloud (connection failed). Check your connection (and AXIOM_CLOUD_URL if you set it), then re-run.".into();
    }
    if lower.contains("401") || lower.contains("unauthorized") || lower.contains("unable to authenticate") || lower.contains("not logged in") {
        return "Not signed in to Axiom Cloud (the session expired or was revoked). Run `axiom login`, then re-run this command.".into();
    }
    let detail = clean(text.strip_prefix("Failed to list projects: ").unwrap_or(&text));
    let detail: String = if detail.trim_start().starts_with('<') { "an unexpected web page".into() } else { detail.chars().take(200).collect() };
    format!("Axiom Cloud did not return your projects ({}). Try again in a minute.", if detail.is_empty() { "no reason given" } else { &detail })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Journal {
    schema: String,
    project: String,
    environment_id: String,
    kind: String,
    intent_id: String,
    manifest_sha256: String,
    /// Slot generation when this intent started; 0 when there was no live slot (none, or
    /// only a deleted/deleting one): the slot this deploy creates starts at 1.
    generation: i64,
    /// `deployment_id` of the live slot `generation` was read from (`None`: no live slot, or a
    /// journal written by an earlier build without this field).
    slot_id: Option<String>,
    /// `deployment_id` of the slot the version was created on (from the create-version reply).
    version_slot_id: Option<String>,
    upload_id: Option<String>,
    upload_sealed: bool,
    version_id: Option<String>,
    version_operation_id: Option<String>,
    version_ready: bool,
    activate_operation_id: Option<String>,
    activated: bool,
    /// Re-activations of this build after it stopped being live (rolled back, replaced).
    /// Each uses a fresh activation key, so the server checks the original precondition
    /// instead of replaying the first activation's stored success.
    reactivations: u32,
    url: Option<String>,
    /// The intent ended without a live version; the next run starts a fresh one.
    failed: bool,
    /// Step → result, for support.
    steps: BTreeMap<String, String>,
}

const JOURNAL_SCHEMA: &str = "axiom-deploy-journal/v1";

impl Journal {
    /// `Idempotency-Key`: hex SHA-256 of `intent|step|manifest_sha256|generation`.
    fn key(&self, step: &str) -> String {
        idempotency_key(&self.intent_id, step, &self.manifest_sha256, self.generation)
    }
    fn step(&mut self, step: &str, result: impl Into<String>) {
        self.steps.insert(step.into(), result.into());
    }
}

/// The activation step name: `activate`, then `activate-<n>` for the n-th re-activation.
fn activation_step(reactivations: u32) -> String {
    if reactivations == 0 { "activate".into() } else { format!("activate-{reactivations}") }
}

/// A finished activation is reported live only if the slot serves this version: a replayed
/// or superseded activation can finish while another version is live.
fn confirm_live(done: &Slot, version: &str, kind: Kind, hint: &str) -> Result<()> {
    if done.active_version_id.as_deref() == Some(version) {
        return Ok(());
    }
    bail!(
        "Version {version} did not become live: the {kind} deployment now serves {}. Review with `axiom deployments status {kind}{hint}`, then run `axiom deployments activate {kind} {version}{hint}` to make it live.",
        done.active_version_id.as_deref().unwrap_or("no version")
    )
}

pub fn idempotency_key(intent: &str, step: &str, manifest_sha256: &str, generation: i64) -> String {
    bundle::sha256_hex(format!("{intent}|{step}|{manifest_sha256}|{generation}").as_bytes())
}

fn journal_path(ctx: &Ctx, scope: &Scope, kind: Kind) -> PathBuf {
    let safe = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect::<String>();
    ctx.journal_dir.join(format!("{}-{}-{}.json", safe(&scope.project), safe(&scope.environment_id), kind))
}

fn load_journal(path: &Path) -> Option<Journal> {
    let j: Journal = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (j.schema == JOURNAL_SCHEMA).then_some(j)
}

fn save_journal(path: &Path, j: &Journal) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temp, serde_json::to_vec_pretty(j)?)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Output

/// The one JSON object `--json` prints.
#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
    pub schema: &'static str,
    pub kind: String,
    pub project: String,
    pub environment: String,
    pub version_id: Option<String>,
    pub operation_id: Option<String>,
    pub url: Option<String>,
    pub state: Option<String>,
    pub activated: bool,
}

/// Progress and status lines go to stderr (also with --json, whose object is on stdout).
fn progress(line: &str) {
    eprintln!("{line}");
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn clean(s: &str) -> String {
    super::cloud_cli::clean(s)
}

fn opt(s: &Option<String>) -> String {
    s.as_deref().map(clean).unwrap_or_else(|| "-".into())
}

/// `--project/--environment` to repeat in a suggested command (only those given).
fn flags_hint(t: &Target) -> String {
    let mut out = String::new();
    if let Some(p) = &t.project {
        out.push_str(&format!(" --project {}", clean(p)));
    }
    if t.environment != "development" {
        out.push_str(&format!(" --environment {}", clean(&t.environment)));
    }
    out
}

/// A failure object from a version or operation, with the shared next step.
fn failure_text(failure: &Value) -> String {
    let code = failure.get("code").and_then(Value::as_str).map(clean);
    let message = failure.get("message").and_then(Value::as_str).map(clean);
    let mut text = match (&code, &message) {
        (Some(c), Some(m)) => format!("{c}: {m}"),
        (Some(c), None) => c.clone(),
        (None, Some(m)) => m.clone(),
        (None, None) => match failure {
            Value::String(s) => clean(s),
            other => clean(&other.to_string()),
        },
    };
    if let Some(c) = code.filter(|c| c.starts_with("AXDEPLOY_")) {
        let step = next_step(&c, 0, None);
        if !step.starts_with("Deployment request failed") {
            text.push_str(&format!(". {step}"));
        }
    }
    text
}

// ---------------------------------------------------------------------------------------
// Deploy

pub async fn handle_frontend(args: &FrontendArgs) -> Result<()> {
    let pending = match take_prepared() {
        Some(p) => p,
        None => frontend_pending(args, &std::env::current_dir()?)?,
    };
    let ctx = Ctx::from_profile()?;
    let question = asks(&args.flags).then(|| deploy_question(Kind::Frontend, args.dir.as_deref(), &args.flags.target));
    deploy_pending(&ctx, Kind::Frontend, pending, &args.flags, question.as_deref(), ask).await
}

pub async fn handle_backend(args: &BackendArgs) -> Result<()> {
    let pending = match take_prepared() {
        Some(p) => p,
        None => backend_pending(args, &std::env::current_dir()?)?,
    };
    let ctx = Ctx::from_profile()?;
    let question = asks(&args.flags).then(|| deploy_question(Kind::Backend, args.from_input.as_deref(), &args.flags.target));
    deploy_pending(&ctx, Kind::Backend, pending, &args.flags, question.as_deref(), ask).await
}

/// Whether the deploy asks y/N (it makes a version live and `--yes` was not given). Without a
/// terminal `main` has already refused it.
pub(crate) fn asks(flags: &DeployFlags) -> bool {
    !flags.no_activate && !flags.yes
}

/// The deploy after the local checks: resolve the project, read the deployment once (a project
/// outside the hosting pilot stops here), hash or package, ask, then [`deploy_from`].
pub async fn deploy_pending(ctx: &Ctx, kind: Kind, pending: Pending, flags: &DeployFlags, question: Option<&str>, ask: impl FnOnce(&str) -> bool) -> Result<()> {
    let r = resolve(ctx, &flags.target).await?;
    let start = ctx.client.deployment_slot(&r.scope, kind).await?;
    if kind == Kind::Backend && matches!(pending, Pending::Package { .. }) {
        progress("Packaging the backend…");
    }
    let (b, _temp) = pending.finish()?;
    if let Some(question) = question {
        if !ask(question) {
            return Err(crate::commands::auth::Exit { code: 2, message: "cancelled; nothing was changed".into() }.into());
        }
    }
    deploy_from(ctx, kind, &b, flags, r, start).await
}

/// Whether a failed step ends the intent (the next run starts a fresh one) rather than
/// leaving it for a resume with the same keys.
fn ends_intent(error: &anyhow::Error) -> bool {
    if error.downcast_ref::<OperationFailed>().is_some() {
        return true;
    }
    match error.downcast_ref::<DeployError>() {
        Some(d) => !(d.retryable || d.status == 0 || d.status == 401 || d.status == 412 || d.status == 429 || d.status >= 500),
        None => false,
    }
}

/// A deploy of a hashed bundle (no question): resolve, read the deployment, [`deploy_from`].
pub async fn deploy(ctx: &Ctx, kind: Kind, b: &Bundle, flags: &DeployFlags) -> Result<()> {
    let r = resolve(ctx, &flags.target).await?;
    let start = ctx.client.deployment_slot(&r.scope, kind).await?;
    deploy_from(ctx, kind, b, flags, r, start).await
}

/// A live slot: present and neither deleted nor being deleted. `GET …/{kind}` answers the
/// newest deleted slot when no live one exists (axiom-backend `findSlot`), and creating an
/// upload then makes a new slot at generation 1 (`ensureSlot`).
fn live(slot: Option<&Slot>) -> Option<&Slot> {
    slot.filter(|s| !matches!(s.state.as_deref(), Some("deleted" | "deleting")))
}

/// Whether a journal's upload and version still belong to the live slot. With no live slot
/// (the deployment was deleted since) or another one, the journal's steps would act on a
/// deleted slot, and a replayed activation would answer the old success, so the deploy
/// starts afresh. A journal without slot ids (an earlier build) is resumed while a live slot
/// exists.
fn same_slot(j: &Journal, now: Option<&Slot>) -> bool {
    let Some(now) = now else {
        return false;
    };
    match (j.version_slot_id.as_deref().or(j.slot_id.as_deref()), now.deployment_id.as_deref()) {
        (Some(was), Some(is)) => was == is,
        _ => true,
    }
}

/// The `If-Match` generation of this deploy's activation, given the slot read now: the
/// generation read when the intent started, never silently refreshed (D2 §6), or 1 when no
/// live slot existed then (the slot this deploy created starts at 1). A journal written
/// by an earlier build (no `slot_id`) may hold the generation of a deleted slot; a pinned
/// generation above the current slot's cannot be this slot's (generations only grow), so it
/// is read as "no live slot then" and pins 1.
fn activation_generation(pinned: i64, slot_id: Option<&str>, now: &Slot) -> i64 {
    if pinned <= 0 {
        return 1;
    }
    match (slot_id, now.generation) {
        (None, Some(current)) if current < pinned => 1,
        _ => pinned,
    }
}

async fn deploy_from(ctx: &Ctx, kind: Kind, b: &Bundle, flags: &DeployFlags, r: Resolved, start: Option<axiom_cloud::deployments::Reply<Slot>>) -> Result<()> {
    let json = flags.target.json;
    let path = journal_path(ctx, &r.scope, kind);
    let now = live(start.as_ref().map(|s| &s.body));
    let previous = load_journal(&path).filter(|j| !flags.new && !j.failed && j.manifest_sha256 == b.manifest_sha256 && same_slot(j, now));
    let mut j = match previous {
        Some(mut j) if j.activated => {
            // This exact build went live in an earlier run. Report it only if the server
            // still serves it; otherwise make it live again under the original precondition.
            match start.map(|s| s.body) {
                Some(s) if s.active_version_id.is_some() && s.active_version_id == j.version_id && matches!(s.state.as_deref(), Some("active" | "degraded")) => {
                    let url = s.url.clone().or(j.url.clone());
                    let out = outcome(&r, kind, j.version_id.clone(), j.activate_operation_id.clone(), url.clone(), s.state.clone(), true);
                    if json {
                        return print_json(&out);
                    }
                    println!("✓ This build is already live: version {} at {}. Pass --new to deploy it again.", opt(&j.version_id), opt(&url));
                    return Ok(());
                }
                _ => {
                    progress(&format!("Version {} of this build is no longer live; activating it again.", opt(&j.version_id)));
                    j.activated = false;
                    j.activate_operation_id = None;
                    j.reactivations += 1;
                    j
                }
            }
        }
        Some(j) => {
            progress(&format!("Resuming the previous deploy of this build (version {}).", opt(&j.version_id)));
            j
        }
        None => {
            let slot = live(start.as_ref().map(|s| &s.body));
            let generation = slot.and_then(|s| s.generation).unwrap_or(0);
            let slot_id = slot.and_then(|s| s.deployment_id.clone());
            let j = Journal {
                schema: JOURNAL_SCHEMA.into(),
                project: r.scope.project.clone(),
                environment_id: r.scope.environment_id.clone(),
                kind: kind.to_string(),
                intent_id: uuid::Uuid::new_v4().to_string(),
                manifest_sha256: b.manifest_sha256.clone(),
                generation,
                slot_id,
                ..Default::default()
            };
            save_journal(&path, &j)?;
            j
        }
    };
    let result = deploy_steps(ctx, kind, b, flags, &r, &path, &mut j).await;
    if let Err(error) = &result {
        if ends_intent(error) {
            j.failed = true;
            j.step("error", error.to_string());
            let _ = save_journal(&path, &j);
        }
    }
    let out = result?;
    if json {
        return print_json(&out);
    }
    let version = opt(&out.version_id);
    if out.activated {
        println!("✓ Live at {} (version {version})", opt(&out.url));
    } else {
        println!("Version {version} is ready. Activate it with: axiom deployments activate {kind} {version}{}", flags_hint(&flags.target));
    }
    Ok(())
}

fn outcome(r: &Resolved, kind: Kind, version_id: Option<String>, operation_id: Option<String>, url: Option<String>, state: Option<String>, activated: bool) -> Outcome {
    Outcome { schema: SCHEMA, kind: kind.to_string(), project: r.scope.project.clone(), environment: r.environment.clone(), version_id, operation_id, url, state, activated }
}

async fn deploy_steps(ctx: &Ctx, kind: Kind, b: &Bundle, flags: &DeployFlags, r: &Resolved, path: &Path, j: &mut Journal) -> Result<Outcome> {
    let scope = &r.scope;
    if !j.upload_sealed {
        let upload = match j.upload_id.clone() {
            None => {
                let body = bodies::create_upload(&b.manifest_sha256, b.manifest_bytes(), kind.purpose(), &b.entries());
                let upload = ctx.client.deployment_create_upload(scope, kind, &body, &j.key("create_upload")).await?;
                j.upload_id = Some(upload.id.clone());
                j.step("create_upload", upload.state.clone().unwrap_or_default());
                save_journal(path, j)?;
                upload
            }
            Some(id) => ctx.client.deployment_upload(scope, kind, &id).await?,
        };
        match upload.state.as_deref() {
            Some("sealed") => {}
            Some("expired" | "failed") => {
                j.failed = true;
                save_journal(path, j)?;
                bail!("The upload {} before it finished. Re-run the deploy to start over.", if upload.state.as_deref() == Some("expired") { "expired" } else { "failed" });
            }
            _ => {
                put_missing(ctx, scope, kind, b, upload).await?;
                let sealed = ctx.client.deployment_complete_upload(scope, kind, &upload_id(j)?, &j.key("complete_upload")).await?;
                if sealed.state.as_deref() != Some("sealed") {
                    return Err(DeployError::new(409, "AXDEPLOY_UPLOAD_NOT_SEALED").into());
                }
            }
        }
        j.upload_sealed = true;
        j.step("complete_upload", "sealed");
        save_journal(path, j)?;
    }
    if j.version_id.is_none() {
        let body = bodies::create_version(kind, &upload_id(j)?, flags.message.as_deref());
        let reply = ctx.client.deployment_create_version(scope, kind, &body, &j.key("create_version")).await?;
        let version = reply.body.version_id.clone().or_else(|| reply.body.desired_version_id.clone()).ok_or_else(|| anyhow!("AXDEPLOY_INVALID_RESPONSE: the server did not return a version id"))?;
        j.version_id = Some(version);
        j.version_operation_id = reply.body.operation_id.clone();
        j.version_slot_id = reply.body.deployment_id.clone();
        j.url = reply.body.url.clone();
        j.step("create_version", reply.body.state.clone().unwrap_or_default());
        save_journal(path, j)?;
    }
    let version = j.version_id.clone().unwrap_or_default();
    if !j.version_ready {
        progress(&format!("Preparing version {version}…"));
        if let Some(op) = j.version_operation_id.clone() {
            wait_op(ctx, scope, kind, &op, "Preparing").await?;
        }
        let v = ctx.client.deployment_version(scope, kind, &version).await?;
        match v.status.as_deref() {
            Some("ready") => {
                j.version_ready = true;
                j.step("version", "ready");
                save_journal(path, j)?;
            }
            Some("failed") => {
                j.failed = true;
                j.step("version", "failed");
                save_journal(path, j)?;
                let why = v.failure.as_ref().map(failure_text).unwrap_or_else(|| "no reason given".into());
                bail!("Version {version} failed: {why}. Fix it and deploy again; `axiom deployments logs {kind}` shows the steps.");
            }
            other => bail!("Version {version} is still {}; check it with `axiom deployments versions {kind}` and re-run the deploy to continue.", other.unwrap_or("pending")),
        }
    }
    if flags.no_activate {
        let url = match &j.url {
            Some(u) => Some(u.clone()),
            None => ctx.client.deployment_slot(scope, kind).await?.and_then(|s| s.body.url),
        };
        return Ok(outcome(r, kind, Some(version), j.version_operation_id.clone(), url, Some("ready".into()), false));
    }
    progress(&format!("Activating version {version}…"));
    // D2 §6: the activation's precondition is the generation read when this deploy started
    // (a slot created by this deploy starts at 1); it is never silently refreshed.
    let (intent, manifest, pinned, slot_id) = (j.intent_id.clone(), j.manifest_sha256.clone(), j.generation, j.slot_id.clone());
    let step = activation_step(j.reactivations);
    let pin = move |now: &Slot| activation_generation(pinned, slot_id.as_deref(), now);
    let done = activate(ctx, scope, kind, &version, true, Some(&pin), |g| idempotency_key(&intent, &step, &manifest, g), |op| {
        j.activate_operation_id = Some(op.to_string());
        let _ = save_journal(path, j);
    })
    .await
    .map_err(|e| match e.downcast_ref::<DeployError>() {
        Some(d) if d.status == 412 => anyhow!(
            "AXDEPLOY_PRECONDITION_FAILED: The deployment changed while you were deploying (someone activated, rolled back or deleted). Review with `axiom deployments status {kind}{hint}`, then make this version live with `axiom deployments activate {kind} {version}{hint}`.",
            hint = flags_hint(&flags.target)
        ),
        _ => e,
    })?;
    // Belt and braces: never report a deleted deployment as live.
    if matches!(done.state.as_deref(), Some("deleted" | "deleting")) {
        j.failed = true;
        j.step("activate", "deployment deleted");
        save_journal(path, j)?;
        bail!("The {kind} deployment was deleted while this deploy ran. Run the deploy again to create a new one.");
    }
    if let Err(e) = confirm_live(&done, &version, kind, &flags_hint(&flags.target)) {
        // Like a 412, keep the intent: a plain rerun replays this activation and stops
        // again, instead of starting a new deploy that would undo the change made meanwhile.
        j.step("activate", "not live");
        save_journal(path, j)?;
        return Err(e);
    }
    j.activated = true;
    j.url = done.url.clone().or(j.url.take());
    j.step("activate", "active");
    save_journal(path, j)?;
    Ok(outcome(r, kind, Some(version), j.activate_operation_id.clone(), j.url.clone(), done.state.clone().or(Some("active".into())), true))
}

fn upload_id(j: &Journal) -> Result<String> {
    j.upload_id.clone().ok_or_else(|| anyhow!("internal: no upload id"))
}

#[derive(Clone)]
enum Source {
    File(PathBuf),
    Manifest,
}

/// PUT every missing object (deduplicated by hash) to its presigned URL, 4 at a time, with
/// progress on stderr. Missing URLs (or a failed PUT, e.g. an expired URL) re-read the upload
/// once for fresh URLs.
async fn put_missing(ctx: &Ctx, scope: &Scope, kind: Kind, b: &Bundle, upload: Upload) -> Result<()> {
    use futures::stream::{self, StreamExt};
    let mut sources: HashMap<&str, (Source, u64)> = HashMap::new();
    for f in &b.files {
        sources.entry(f.sha256.as_str()).or_insert((Source::File(f.source.clone()), f.size));
    }
    sources.entry(b.manifest_sha256.as_str()).or_insert((Source::Manifest, b.manifest.len() as u64));
    let mut upload = upload;
    let mut refreshed = false;
    let terminal = std::io::stderr().is_terminal();
    loop {
        let mut missing: Vec<String> = upload.missing_hashes.clone();
        missing.sort();
        missing.dedup();
        if missing.is_empty() {
            if !refreshed {
                progress("All files are already uploaded.");
            }
            return Ok(());
        }
        if let Some(unknown) = missing.iter().find(|h| !sources.contains_key(h.as_str())) {
            bail!("AXDEPLOY_INVALID_RESPONSE: the server asked for object {} that is not part of this deploy", clean(unknown.get(..12).unwrap_or(unknown)));
        }
        if missing.iter().any(|h| upload.target(h).is_none()) {
            if refreshed {
                return Err(DeployError::new(0, "AXDEPLOY_UPLOAD_URLS_UNAVAILABLE").into());
            }
            refreshed = true;
            upload = ctx.client.deployment_upload(scope, kind, &upload.id).await?;
            continue;
        }
        let total = missing.len();
        let total_bytes: u64 = missing.iter().map(|h| sources[h.as_str()].1).sum();
        let done = AtomicUsize::new(0);
        let done_bytes = AtomicU64::new(0);
        progress(&format!("Uploading {total} file{} ({})…", if total == 1 { "" } else { "s" }, bundle::human_bytes(total_bytes)));
        let jobs = missing.iter().map(|h| (h.clone(), upload.target(h).cloned().unwrap_or_default(), sources[h.as_str()].0.clone()));
        let failures: Vec<anyhow::Error> = stream::iter(jobs)
            .map(|(hash, target, source)| {
                let (done, done_bytes, manifest) = (&done, &done_bytes, &b.manifest);
                async move {
                    let bytes = match &source {
                        Source::Manifest => manifest.clone(),
                        Source::File(p) => std::fs::read(p).with_context(|| format!("cannot read {}", p.display()))?,
                    };
                    if bundle::sha256_hex(&bytes) != hash {
                        let shown = match &source {
                            Source::File(p) => p.display().to_string(),
                            Source::Manifest => "the manifest".into(),
                        };
                        bail!("{shown} changed while deploying. Re-run the deploy once your build has finished writing.");
                    }
                    let size = bytes.len() as u64;
                    ctx.client.deployment_put_object(&target, bytes).await?;
                    let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                    let sent = done_bytes.fetch_add(size, Ordering::SeqCst) + size;
                    if terminal {
                        eprint!("\rUploading {n}/{total} files ({})   ", bundle::human_bytes(sent));
                    }
                    Ok(())
                }
            })
            .buffer_unordered(UPLOAD_CONCURRENCY)
            .filter_map(|r: Result<()>| async move { r.err() })
            .collect()
            .await;
        if terminal {
            eprintln!();
        }
        let Some(first) = failures.into_iter().next() else {
            progress(&format!("Uploaded {total}/{total} files ({}).", bundle::human_bytes(total_bytes)));
            return Ok(());
        };
        if refreshed || first.to_string().contains("changed while deploying") {
            return Err(first);
        }
        refreshed = true;
        upload = ctx.client.deployment_upload(scope, kind, &upload.id).await?;
    }
}

/// A deployment operation that ended `failed` or `cancelled`.
#[derive(Debug)]
struct OperationFailed(String);

impl std::fmt::Display for OperationFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OperationFailed {}

/// Follow an operation to its end; a failed or cancelled one is an error with its reason.
async fn wait_op(ctx: &Ctx, scope: &Scope, kind: Kind, op: &str, doing: &str) -> Result<Operation> {
    let mut last_phase = String::new();
    let done = ctx
        .client
        .deployment_wait(scope, kind, op, ctx.poll, ctx.timeout, |o| {
            let phase = o.phase.clone().unwrap_or_else(|| o.state.clone());
            if phase != last_phase && !o.finished() {
                progress(&format!("  {doing}: {}", clean(&phase)));
                last_phase = phase;
            }
        })
        .await?;
    match done.state.as_str() {
        "succeeded" => Ok(done),
        state => {
            let why = done.failure.as_ref().map(failure_text).unwrap_or_else(|| "no reason given".into());
            Err(OperationFailed(format!("Operation {} {state}: {why}. See `axiom deployments status {kind}` and `axiom deployments logs {kind}`.", clean(op))).into())
        }
    }
}

/// The slot, or a "nothing deployed" error naming the next step.
async fn slot(ctx: &Ctx, scope: &Scope, kind: Kind) -> Result<axiom_cloud::deployments::Reply<Slot>> {
    ctx.client.deployment_slot(scope, kind).await?.ok_or_else(|| anyhow!("There is no {kind} deployment in this environment yet. Deploy one with `axiom deploy {kind}`."))
}

fn if_match(reply: &axiom_cloud::deployments::Reply<Slot>) -> Result<String> {
    reply.etag.clone().or_else(|| reply.body.etag()).ok_or_else(|| anyhow!("AXDEPLOY_INVALID_RESPONSE: the deployment has no generation (ETag)"))
}

/// Make `version` live: GET the slot, POST activate with `If-Match` (the generation `pinned`
/// derives from the slot just read when given, else that slot's own), and follow the operation when `wait`.
/// An activation of this version already in flight is followed instead of repeated; an
/// already-live version is reported as such.
#[allow(clippy::too_many_arguments)]
async fn activate(ctx: &Ctx, scope: &Scope, kind: Kind, version: &str, wait: bool, pinned: Option<&dyn Fn(&Slot) -> i64>, key: impl Fn(i64) -> String, mut started: impl FnMut(&str)) -> Result<Slot> {
    let current = slot(ctx, scope, kind).await?;
    let s = &current.body;
    if s.active_version_id.as_deref() == Some(version) && matches!(s.state.as_deref(), Some("active" | "degraded")) {
        return Ok(current.body);
    }
    let op = if s.desired_version_id.as_deref() == Some(version) && s.state.as_deref() == Some("activating") && s.operation_id.is_some() {
        s.operation_id.clone().unwrap_or_default()
    } else {
        let (tag, generation) = match pinned {
            Some(pin) => {
                let g = pin(s);
                (format!("\"g{g}\""), g)
            }
            None => (if_match(&current)?, s.generation.unwrap_or(0)),
        };
        let reply = ctx.client.deployment_activate(scope, kind, version, &tag, &key(generation)).await?;
        reply.body.operation_id.clone().ok_or_else(|| anyhow!("AXDEPLOY_INVALID_RESPONSE: the activation returned no operation"))?
    };
    started(&op);
    if !wait {
        let mut s = current.body;
        s.operation_id = Some(op);
        s.state = Some("activating".into());
        return Ok(s);
    }
    wait_op(ctx, scope, kind, &op, "Activating").await?;
    let mut live = slot(ctx, scope, kind).await?.body;
    live.operation_id = Some(op);
    Ok(live)
}

// ---------------------------------------------------------------------------------------
// axiom deployments

pub async fn handle(args: &DeploymentsArgs) -> Result<()> {
    let ctx = Ctx::from_profile()?;
    run(&ctx, args).await
}

/// A standalone traffic action's `Idempotency-Key`: random for each invocation. A
/// deterministic key would replay an earlier answer (the server keeps them 24 h) when a
/// re-created slot reaches the same generation. A rerun after a lost response is a new
/// request: its `If-Match` is re-read, so it acts on the current state (a second
/// `rollback` rolls back again; a second `activate` of a live version is a no-op).
fn action_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub async fn run(ctx: &Ctx, args: &DeploymentsArgs) -> Result<()> {
    let t = &args.target;
    let json = t.json;
    // Local input errors come before any request.
    let (since, until) = match &args.action {
        Action::Logs { since, until, .. } => (since.as_deref().map(|s| parse_time(s, "--since")).transpose()?, until.as_deref().map(|s| parse_time(s, "--until")).transpose()?),
        _ => (None, None),
    };
    let r = resolve(ctx, t).await?;
    let scope = &r.scope;
    match &args.action {
        Action::List => {
            let mut items = Vec::new();
            let mut cursor: Option<String> = None;
            for _ in 0..20 {
                let page = ctx.client.deployments_list(scope, None, cursor.as_deref()).await?;
                items.extend(page.items);
                cursor = page.next_cursor.filter(|c| !c.is_empty());
                if cursor.is_none() {
                    break;
                }
            }
            if json {
                return print_json(&json!({"schema": SCHEMA, "project": scope.project, "environment": r.environment, "items": items}));
            }
            for line in list_lines(&items, &r.environment) {
                println!("{line}");
            }
            Ok(())
        }
        Action::Status { kind } => {
            let kind = Kind::from(*kind);
            let s = slot(ctx, scope, kind).await?.body;
            let op = match &s.operation_id {
                Some(id) => ctx.client.deployment_operation(scope, kind, id).await.ok(),
                None => None,
            };
            if json {
                let mut value = serde_json::to_value(&s)?;
                value["last_operation"] = serde_json::to_value(&op)?;
                return print_json(&value);
            }
            for line in status_lines(kind, &s, op.as_ref()) {
                println!("{line}");
            }
            Ok(())
        }
        Action::Versions { kind } => {
            let kind = Kind::from(*kind);
            let mut versions = Vec::new();
            let mut cursor: Option<String> = None;
            for _ in 0..20 {
                let page = ctx.client.deployment_versions(scope, kind, cursor.as_deref()).await?;
                versions.extend(page.items);
                cursor = page.next_cursor.filter(|c| !c.is_empty());
                if cursor.is_none() {
                    break;
                }
            }
            let live = ctx.client.deployment_slot(scope, kind).await?.and_then(|s| s.body.active_version_id);
            if json {
                return print_json(&json!({"schema": SCHEMA, "active_version_id": live, "items": versions, "next_cursor": cursor}));
            }
            if versions.is_empty() {
                println!("No {kind} versions yet. Deploy one with `axiom deploy {kind}`.");
                return Ok(());
            }
            let rows: Vec<Vec<String>> = versions
                .iter()
                .map(|v| {
                    let mark = if live.as_deref() == Some(v.id.as_str()) { "*" } else { " " };
                    let failure = v.failure.as_ref().map(failure_text).unwrap_or_default();
                    vec![format!("{mark} {}", clean(&v.id)), opt(&v.status), opt(&v.created_at), failure]
                })
                .collect();
            for line in table(&["  VERSION", "STATUS", "CREATED", "FAILURE"], &rows) {
                println!("{line}");
            }
            Ok(())
        }
        Action::Activate { kind, version, wait, .. } => {
            let kind = Kind::from(*kind);
            let key = action_key();
            let done = activate(ctx, scope, kind, version, *wait, None, |_| key.clone(), |_| {}).await?;
            let live = *wait || (done.active_version_id.as_deref() == Some(version.as_str()) && matches!(done.state.as_deref(), Some("active" | "degraded")));
            finish_traffic(&r, kind, &done, live, json, &format!("{} is live", clean(version)), "Activation")
        }
        Action::Rollback { kind, to, wait, .. } => {
            let kind = Kind::from(*kind);
            let current = slot(ctx, scope, kind).await?;
            let tag = if_match(&current)?;
            let key = action_key();
            let reply = ctx.client.deployment_rollback(scope, kind, to.as_deref(), &tag, &key).await?;
            let op = reply.body.operation_id.clone();
            let done = match (&op, wait) {
                (Some(op), true) => {
                    wait_op(ctx, scope, kind, op, "Rolling back").await?;
                    let mut s = slot(ctx, scope, kind).await?.body;
                    s.operation_id = Some(op.clone());
                    s
                }
                _ => reply.body,
            };
            let target = done.active_version_id.clone().filter(|_| *wait).or(to.clone()).unwrap_or_else(|| "the previous version".into());
            finish_traffic(&r, kind, &done, *wait, json, &format!("Rolled back to {}", clean(&target)), "Rollback")
        }
        Action::Delete { kind, wait, .. } => {
            let kind = Kind::from(*kind);
            let current = slot(ctx, scope, kind).await?;
            let tag = if_match(&current)?;
            let key = action_key();
            let reply = ctx.client.deployment_delete(scope, kind, &tag, &key).await?;
            let op = reply.body.operation_id.clone();
            let mut done = reply.body;
            if let (Some(op), true) = (&op, wait) {
                wait_op(ctx, scope, kind, op, "Deleting").await?;
                done = ctx.client.deployment_slot(scope, kind).await?.map(|s| s.body).unwrap_or_default();
                done.operation_id = Some(op.clone());
                if done.state.is_none() {
                    done.state = Some("deleted".into());
                }
            }
            if json {
                return print_json(&outcome(&r, kind, None, done.operation_id.clone(), done.url.clone(), done.state.clone(), false));
            }
            if *wait {
                println!("✓ Deleted the {kind} deployment of {} ({}); its URL no longer serves.", scope.project, r.environment);
            } else {
                println!("Delete requested (operation {}). Follow it with `axiom deployments status {kind}{}`.", opt(&done.operation_id), flags_hint(t));
            }
            Ok(())
        }
        Action::Logs { kind, version, stream, .. } => {
            let kind = Kind::from(*kind);
            let stream = match stream {
                Stream::Lifecycle => "lifecycle",
                Stream::Runtime => "runtime",
                Stream::Access => "access",
            };
            let mut items = Vec::new();
            let mut cursor: Option<String> = None;
            for _ in 0..20 {
                let q = LogQuery { stream: Some(stream), since: since.as_deref(), until: until.as_deref(), version: version.as_deref(), cursor: cursor.as_deref(), limit: None };
                let page = ctx.client.deployment_logs(scope, kind, &q).await?;
                items.extend(page.items);
                cursor = page.next_cursor.filter(|c| !c.is_empty());
                if cursor.is_none() {
                    break;
                }
            }
            // The server pages newest first; print oldest first, like a log.
            oldest_first(&mut items);
            if json {
                return print_json(&json!({"schema": SCHEMA, "items": items}));
            }
            if items.is_empty() {
                println!("No {stream} events{}.", if since.is_some() { " in that time" } else { "" });
            }
            for e in &items {
                let result = match &e.result {
                    Some(Value::String(s)) => clean(s),
                    Some(Value::Null) | None => "-".into(),
                    Some(other) => {
                        let text = clean(&other.to_string());
                        if text.chars().count() > 120 { format!("{}…", text.chars().take(120).collect::<String>()) } else { text }
                    }
                };
                println!("{}  {}  {}  {}  {}", opt(&e.created_at), opt(&e.action), opt(&e.step), opt(&e.version_id), result);
            }
            Ok(())
        }
        Action::Health { kind } => {
            let kind = Kind::from(*kind);
            let health = ctx.client.deployment_health(scope, kind).await?;
            if json {
                return print_json(&health);
            }
            let h = health.get("health").cloned().unwrap_or(Value::Null);
            let text = match &h {
                Value::String(s) => clean(s),
                Value::Object(o) => o.get("status").and_then(Value::as_str).map(clean).unwrap_or_else(|| clean(&h.to_string())),
                Value::Null => "unknown".into(),
                other => clean(&other.to_string()),
            };
            println!("{kind} health: {text}");
            Ok(())
        }
    }
}

/// `waited`: the operation finished (or the version was already live).
fn finish_traffic(r: &Resolved, kind: Kind, done: &Slot, waited: bool, json: bool, live_text: &str, what: &str) -> Result<()> {
    if json {
        let active = waited && matches!(done.state.as_deref(), Some("active" | "degraded"));
        return print_json(&outcome(r, kind, done.active_version_id.clone().filter(|_| waited).or(done.desired_version_id.clone()), done.operation_id.clone(), done.url.clone(), done.state.clone(), active));
    }
    if waited {
        println!("✓ {live_text} at {}", opt(&done.url));
    } else {
        println!("{what} requested (operation {}). Follow it with `axiom deployments status {kind}`, or pass --wait.", opt(&done.operation_id));
    }
    Ok(())
}

/// Lifecycle events in log order: oldest first by `created_at`, then id.
pub fn oldest_first(items: &mut [axiom_cloud::deployments::LogEvent]) {
    items.sort_by(|a, b| (a.created_at.as_deref(), a.id.as_deref()).cmp(&(b.created_at.as_deref(), b.id.as_deref())));
}

pub fn list_lines(items: &[Slot], environment: &str) -> Vec<String> {
    if items.is_empty() {
        return vec![format!("No deployments in {} yet. Deploy with `axiom deploy frontend` or `axiom deploy backend`.", clean(environment))];
    }
    let rows: Vec<Vec<String>> = items.iter().map(|s| vec![opt(&s.kind), opt(&s.state), opt(&s.active_version_id), opt(&s.url), opt(&s.updated_at)]).collect();
    table(&["KIND", "STATE", "ACTIVE VERSION", "URL", "UPDATED"], &rows)
}

fn table(headers: &[&str], rows: &[Vec<String>]) -> Vec<String> {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| if i + 1 == cells.len() { c.to_string() } else { format!("{c:<w$}", w = widths[i]) })
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    std::iter::once(line(headers.to_vec())).chain(rows.iter().map(|r| line(r.iter().map(String::as_str).collect()))).collect()
}

pub fn status_lines(kind: Kind, s: &Slot, op: Option<&Operation>) -> Vec<String> {
    let mut lines = vec![
        format!("Kind:      {kind}"),
        format!("URL:       {}", opt(&s.url)),
        format!("State:     {}{}", opt(&s.state), s.suspend_reason.as_deref().map(|r| format!(" ({})", clean(r))).unwrap_or_default()),
        format!("Live:      {}", opt(&s.active_version_id)),
        format!("Desired:   {}", opt(&s.desired_version_id)),
        format!("Previous:  {}", opt(&s.previous_version_id)),
        format!("Updated:   {}", opt(&s.updated_at)),
    ];
    if let Some(o) = op {
        lines.push(format!("Last operation: {} {} {}{}", clean(&o.operation_id), opt(&o.action), clean(&o.state), o.phase.as_deref().map(|p| format!(" ({})", clean(p))).unwrap_or_default()));
        if let Some(f) = &o.failure {
            lines.push(format!("Failure:   {}", failure_text(f)));
        }
    }
    if let Some(f) = s.failure.as_ref().filter(|f| !f.is_null()) {
        lines.push(format!("Failure:   {}", failure_text(f)));
    }
    if s.state.as_deref() == Some("suspended") {
        lines.push(format!("Next step: {}", next_step("AXDEPLOY_SUSPENDED", 409, None)));
    }
    lines
}

/// `--since/--until`: `30s`, `10m`, `1h`, `2d` ago, or an RFC 3339 time, as RFC 3339 UTC.
pub fn parse_time(value: &str, flag: &str) -> Result<String> {
    parse_time_at(value, flag, chrono::Utc::now())
}

fn parse_time_at(value: &str, flag: &str, now: chrono::DateTime<chrono::Utc>) -> Result<String> {
    let v = value.trim();
    let bad = || anyhow!("{flag} takes a duration such as 30s, 10m, 1h or 2d, or an RFC 3339 time such as 2026-11-16T09:00:00Z");
    if let Some(unit) = v.chars().last().filter(|c| matches!(c, 's' | 'm' | 'h' | 'd')) {
        let digits = &v[..v.len() - 1];
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            let n: i64 = digits.parse().map_err(|_| bad())?;
            let seconds = match unit {
                's' => n,
                'm' => n.checked_mul(60).ok_or_else(bad)?,
                'h' => n.checked_mul(3600).ok_or_else(bad)?,
                _ => n.checked_mul(86_400).ok_or_else(bad)?,
            };
            let at = now.checked_sub_signed(chrono::Duration::try_seconds(seconds).ok_or_else(bad)?).ok_or_else(bad)?;
            return Ok(at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        }
    }
    let at = chrono::DateTime::parse_from_rfc3339(v).map_err(|_| bad())?;
    Ok(at.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

#[cfg(test)]
mod tests;
