//! Release impact inputs and the signed sidecar (`data-tools/v1`, "Impact (W5.4)").
//!
//! When `migrate` prepares a release, the CLI computes its `axiom-database-impact/v1` report
//! from three inputs and writes the signed envelope next to the release, at
//! `.axiom/database/migrations/<version>/impact.json`:
//!
//! | Input | Source |
//! | --- | --- |
//! | Application | `--application <dir>`, else the nearest ancestor of the database source whose `AxiomDeps.toml` declares `[application]` |
//! | Server plan | `--impact-plan <file>`, else every `*.server.json` backend build output in the application |
//! | UI lock | `axiom.ui.lock.json` of each frontend project (`type = "frontend"`) in the application |
//!
//! A missing input is an `unknown` entry of the report, never a failure of `migrate`: the
//! release and the executor are unchanged, and the sidecar can be attached later.
use anyhow::{Context, Result};
use axiom_database::developer::{self as db, Release};
use axiom_lib::database::impact::{self, Envelope, Plan, Report, Unknown};
use std::path::{Path, PathBuf};

/// Directories never searched for inputs.
const SKIP: &[&str] = &[".git", "node_modules", "target"];
const MAX_DEPTH: usize = 8;
const MAX_PLANS: usize = 64;
const MAX_PLAN_BYTES: u64 = 16 * 1024 * 1024;
/// Ancestors searched for the application manifest.
const MAX_ANCESTORS: usize = 16;

/// The impact inputs of `migrate` (`data-tools/v1`, "Inputs").
#[derive(clap::Args, Clone, Debug, Default)]
pub struct Options {
    /// Application root for the release impact report; default: the nearest ancestor of the database source whose AxiomDeps.toml declares [application]
    #[arg(long = "application", value_name = "DIR")]
    pub application: Option<PathBuf>,
    /// Backend server plan (<output>.server.json) for the release impact report; default: every *.server.json in the application
    #[arg(long = "impact-plan", value_name = "FILE")]
    pub plan: Option<PathBuf>,
}

/// Whether `release` is not yet in the project's migration catalog: `migrate` writes a
/// sidecar only for a release it is preparing now.
pub fn fresh(root: &Path, release: &Release) -> bool {
    !root.join(".axiom/database/migrations").join(&release.revision.identity.version).join("release.axiom").exists()
}
/// The sidecar path of a release in a project's migration catalog.
pub fn sidecar_path(root: &Path, release: &Release) -> PathBuf {
    root.join(".axiom/database/migrations").join(&release.revision.identity.version).join("impact.json")
}

/// The nearest ancestor of `source` (its own directory included) whose `AxiomDeps.toml`
/// declares an application.
pub fn discover_application(source: &Path) -> Option<PathBuf> {
    let start = source.canonicalize().ok()?;
    let start = if start.is_file() { start.parent()?.to_path_buf() } else { start };
    start.ancestors().take(MAX_ANCESTORS).find(|dir| manifest(dir).is_some_and(|m| m.application.is_some())).map(Path::to_path_buf)
}
fn manifest(dir: &Path) -> Option<axiom_lib::deps_manifest::AxiomDepsManifestV2> {
    let text = std::fs::read_to_string(dir.join("AxiomDeps.toml")).ok()?;
    axiom_lib::deps_manifest::parse_axiom_deps_v2(&text).ok()
}

/// Directories under `root` (bounded depth) and their files, skipping [`SKIP`].
fn walk(root: &Path, mut visit: impl FnMut(&Path, bool)) {
    fn go(dir: &Path, depth: usize, visit: &mut dyn FnMut(&Path, bool)) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if depth < MAX_DEPTH && !SKIP.contains(&entry.file_name().to_string_lossy().as_ref()) {
                    visit(&path, true);
                    go(&path, depth + 1, visit);
                }
            } else {
                visit(&path, false);
            }
        }
    }
    visit(root, true);
    go(root, 0, &mut visit);
}

/// Every backend server plan the inputs name.
fn plans(application: Option<&Path>, explicit: Option<&Path>, unknown: &mut Vec<Unknown>) -> Vec<axiom_lib::backend::ServerPlan> {
    let read = |path: &Path| -> Option<axiom_lib::backend::ServerPlan> {
        let meta = std::fs::metadata(path).ok()?;
        if meta.len() > MAX_PLAN_BYTES {
            return None;
        }
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    };
    if let Some(path) = explicit {
        return match read(path) {
            Some(plan) => vec![plan],
            None => {
                unknown.push(Unknown::new("no_plan", None, "plan_unreadable"));
                vec![]
            }
        };
    }
    let Some(root) = application else { return vec![] };
    let mut found = vec![];
    walk(root, |path, dir| {
        if !dir && found.len() < MAX_PLANS && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".server.json")) {
            if let Some(plan) = read(path) {
                found.push(plan);
            }
        }
    });
    found
}

/// `missing_lock` and `stale_lock` for every frontend project of the application: a lock
/// must exist, and every contract artifact it pins must still have the pinned digest.
fn locks(root: &Path, unknown: &mut Vec<Unknown>) {
    use sha2::{Digest, Sha256};
    let mut frontends = vec![];
    walk(root, |path, dir| {
        if dir && manifest(path).is_some_and(|m| m.contract_type.as_deref() == Some("frontend")) {
            frontends.push(path.to_path_buf());
        }
    });
    for dir in frontends {
        let name = relative(root, &dir).unwrap_or_else(|| ".".into());
        let lock = axiom_lib::project_state::ui_lock(&dir);
        let Some(parsed) = std::fs::read(&lock).ok().and_then(|b| serde_json::from_slice::<axiom_lib::ui_contract::UiContractLock>(&b).ok()) else {
            unknown.push(Unknown::new("missing_lock", Some(name.as_str()), "lock_absent"));
            continue;
        };
        let stale = parsed.contracts.values().any(|c| {
            let artifact = if c.artifact.is_absolute() { c.artifact.clone() } else { dir.join(&c.artifact) };
            std::fs::read(artifact).map_or(true, |bytes| hex::encode(Sha256::digest(bytes)) != c.artifact_sha256)
        });
        if stale {
            unknown.push(Unknown::new("stale_lock", Some(name.as_str()), "lock_artifact_mismatch"));
        }
    }
}
/// `path` relative to `root`, `/`-separated, if it is inside it.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let parts = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The impact report of `release` with the application at hand.
pub fn compute(source: &Path, options: &Options, release: &Release) -> Result<Report> {
    let mut unknown = vec![];
    let application = match &options.application {
        Some(dir) => Some(dir.canonicalize().with_context(|| format!("read application {}", dir.display()))?),
        None => discover_application(source),
    };
    let found = plans(application.as_deref(), options.plan.as_deref(), &mut unknown);
    if let Some(root) = &application {
        locks(root, &mut unknown);
    }
    let graph = match &application {
        Some(root) if !found.is_empty() => axiom_lib::application_inspector::inspect_workspace(root, env!("CARGO_PKG_VERSION"))
            .and_then(|g| super::inspector_frontend::enrich(root, g))
            .ok(),
        _ => None,
    };
    let locate = |path: &str, offset: usize| -> Option<(u32, u32)> {
        let root = application.as_ref()?;
        if !impact::relative_path(path) {
            return None;
        }
        let text = std::fs::read_to_string(root.join(path)).ok()?;
        let before = text.get(..offset)?;
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().unwrap_or_default().chars().count() + 1;
        Some((u32::try_from(line).ok()?, u32::try_from(column).ok()?))
    };
    let text = |path: &str| -> Option<String> {
        let root = application.as_ref()?;
        if !impact::relative_path(path) {
            return None;
        }
        std::fs::read_to_string(root.join(path)).ok()
    };
    let plan = if found.is_empty() { Plan::Missing } else { Plan::Found(found.iter().collect()) };
    impact::report(impact::Inputs {
        release_sha256: &release.sha256,
        before: release.revision.base.as_ref(),
        after: &release.revision.desired,
        plan,
        graph: graph.as_ref(),
        locate: &locate,
        text: &text,
        unknown,
    })
}

/// Writes the signed sidecar of a release this CLI just prepared, signed by the same local
/// key as the release. A release signed by another key gets no sidecar. Returns the path
/// written, or `None`.
pub fn write(root: &Path, source: &Path, options: &Options, release: &Release) -> Result<Option<PathBuf>> {
    let name = &release.revision.identity.name;
    write_with(root, source, options, release, |message| db::sign_message_with_local_key(name, message).map_err(|e| anyhow::anyhow!(e.to_string())))
}
/// [`write`] with an explicit signer returning the hex public key and signature.
pub fn write_with(root: &Path, source: &Path, options: &Options, release: &Release, sign: impl FnOnce(&[u8]) -> Result<(String, String)>) -> Result<Option<PathBuf>> {
    let report = compute(source, options, release)?;
    let envelope = Envelope::seal(report, sign)?;
    if envelope.public_key != release.public_key {
        return Ok(None);
    }
    envelope.verify(&release.sha256, &release.public_key)?;
    let path = sidecar_path(root, release);
    let parent = path.parent().context("impact sidecar has no directory")?;
    std::fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut staged, &envelope.canonical_bytes()?)?;
    staged.as_file().sync_all()?;
    staged.persist(&path).map_err(|e| e.error)?;
    Ok(Some(path))
}

/// [`write`] for `migrate`: an impact failure never fails the migration; it is reported on
/// standard error (outside JSON mode) and the release goes ahead without a sidecar.
pub fn write_quietly(root: &Path, source: &Path, options: &Options, release: &Release, json: bool) {
    match write(root, source, options, release) {
        Ok(Some(path)) if !json => {
            if let Ok(envelope) = read(&path, release) {
                eprintln!(
                    "Impact report for {}: {} endpoint(s), {} frontend consumer(s), coverage {}{}",
                    release.revision.identity.version,
                    envelope.report.endpoints.len(),
                    envelope.report.frontend.len(),
                    envelope.report.coverage,
                    if envelope.report.unknown.is_empty() { String::new() } else { format!(" ({} unknown)", envelope.report.unknown.len()) }
                );
            }
        }
        Ok(_) => {}
        Err(error) if !json => eprintln!("No impact report for {}: {error}", release.revision.identity.version),
        Err(_) => {}
    }
}

/// The verified sidecar of `release`, as stored.
pub fn read(path: &Path, release: &Release) -> Result<Envelope> {
    let bytes = std::fs::read(path)?;
    anyhow::ensure!(bytes.len() <= impact::MAX_ENVELOPE, "impact sidecar above 1 MiB");
    let envelope: Envelope = serde_json::from_slice(&bytes).context("invalid impact sidecar")?;
    envelope.verify(&release.sha256, &release.public_key)?;
    Ok(envelope)
}
/// The canonical sidecar bytes to upload for `release`, if the catalog holds a valid one.
pub fn upload_body(root: &Path, release: &Release) -> Option<Vec<u8>> {
    read(&sidecar_path(root, release), release).ok()?.canonical_bytes().ok()
}

// ---------------------------------------------------------------------------
// `axiom database impact check` (branches-v1 "CI policy")
// ---------------------------------------------------------------------------

pub const APPROVALS_FORMAT: &str = "axiom-database-impact-approvals/v1";

#[derive(clap::Subcommand, Debug)]
pub enum ImpactAction {
    /// Fail (exit 10) when a breaking change reaches endpoints or frontend consumers without an approval
    Check {
        /// The release (default: the newest release in .axiom/database/migrations of this directory)
        #[arg(long)]
        release: Option<PathBuf>,
        /// Approvals bound to the release: {"format","release_sha256","approved":[{"object_id","reason"}]}
        #[arg(long, default_value = "db/impact.approvals.json")]
        approvals: PathBuf,
        /// Exit 9 when the impact coverage is partial
        #[arg(long)]
        require_complete: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    pub object_id: String,
    pub reason: String,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Approvals {
    pub format: String,
    pub release_sha256: String,
    pub approved: Vec<Approval>,
}

/// The outcome: 0 clean, 9 partial coverage with `require_complete`, 10 unapproved breaking
/// change with mapped endpoints or frontend consumers.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct ImpactCheck {
    pub exit: i32,
    pub release_sha256: String,
    pub breaking: Vec<String>,
    pub unapproved: Vec<String>,
    pub coverage: String,
}

/// Whether an object (a column, or a table whose columns it prefixes) is read by an endpoint
/// of the report.
fn mapped(report: &Report, object: &str) -> bool {
    report.endpoints.iter().any(|e| e.fields.iter().any(|f| f.column_id == object || f.column_id.starts_with(&format!("{object}."))))
}

pub fn check(release: &Release, report: &Report, approvals: Option<&Approvals>, require_complete: bool) -> Result<ImpactCheck> {
    let breaking = match &release.revision.base {
        Some(base) => axiom_lib::database::change::diff(base, &release.revision.desired)?.changes.into_iter().filter(|c| c.compatibility == "breaking").map(|c| c.object_id).collect::<Vec<_>>(),
        None => vec![],
    };
    let approved = approvals
        .filter(|a| a.format == APPROVALS_FORMAT && a.release_sha256 == release.sha256)
        .map(|a| a.approved.iter().filter(|x| !x.reason.trim().is_empty()).map(|x| x.object_id.as_str()).collect::<std::collections::BTreeSet<_>>())
        .unwrap_or_default();
    let consumed = !report.frontend.is_empty();
    let unapproved = breaking.iter().filter(|id| (mapped(report, id) || consumed && !report.endpoints.is_empty()) && !approved.contains(id.as_str())).cloned().collect::<Vec<_>>();
    let exit = if !unapproved.is_empty() {
        10
    } else if require_complete && report.coverage != "complete" {
        9
    } else {
        0
    };
    Ok(ImpactCheck { exit, release_sha256: release.sha256.clone(), breaking, unapproved, coverage: report.coverage.clone() })
}

/// The newest release of the project's migration catalog.
fn newest_release(root: &Path) -> Result<PathBuf> {
    let dir = root.join(".axiom/database/migrations");
    let mut found = std::fs::read_dir(&dir)
        .with_context(|| format!("no migration catalog at {}", dir.display()))?
        .flatten()
        .map(|e| e.path().join("release.axiom"))
        .filter(|p| p.is_file())
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect::<Vec<_>>();
    found.sort();
    found.pop().map(|(_, p)| p).context("the migration catalog holds no release; pass --release")
}

pub fn run_check(action: &ImpactAction) -> Result<()> {
    let ImpactAction::Check { release, approvals, require_complete, json } = action;
    let path = match release {
        Some(path) => path.clone(),
        None => newest_release(&std::env::current_dir()?)?,
    };
    let release = db::read_release(&path).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let sidecar = path.with_file_name("impact.json");
    let envelope = read(&sidecar, &release).with_context(|| format!("no valid impact sidecar at {}; run `axiom database migrate` to prepare one", sidecar.display()))?;
    let approvals = if approvals.exists() {
        let bytes = std::fs::read(approvals)?;
        let parsed: Approvals = serde_json::from_slice(&bytes).with_context(|| format!("{} is not an {APPROVALS_FORMAT} file", approvals.display()))?;
        if parsed.format != APPROVALS_FORMAT {
            anyhow::bail!("{} is not an {APPROVALS_FORMAT} file", approvals.display());
        }
        Some(parsed)
    } else {
        None
    };
    let outcome = check(&release, &envelope.report, approvals.as_ref(), *require_complete)?;
    if *json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
    } else {
        println!("Release {}: {} breaking change(s), coverage {}", &outcome.release_sha256[..12.min(outcome.release_sha256.len())], outcome.breaking.len(), outcome.coverage);
        for id in &outcome.unapproved {
            println!("Unapproved breaking change with consumers: {id}");
        }
        if approvals.as_ref().is_some_and(|a| a.release_sha256 != release.sha256) {
            println!("The approvals file names another release; approve this release again.");
        }
    }
    if outcome.exit != 0 {
        std::process::exit(outcome.exit);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
