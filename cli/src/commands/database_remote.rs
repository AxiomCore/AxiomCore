//! Hosted database commands. They reuse the local release and catalog logic in
//! `database_dev` and replace the database connection with the hosted
//! migrations API: releases are signed locally, uploaded, and applied by the
//! hosted executor; nothing connects to the database from this machine.
use super::database_dev::{self as dev, MigrateArgs, ReadArgs, RollbackArgs};
use anyhow::{bail, Context, Result};
use axiom_cloud::migrations::{self as api, Mode, Operation, OperationTarget, Signer, WaitPolicy};
use axiom_cloud::CloudClient;
use axiom_database::developer::{self as db, Release};
use clap::Args;
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// Selects a hosted environment instead of a local connection.
#[derive(Args, Clone, Debug, Default)]
pub struct RemoteArgs {
    /// AxiomCore Cloud project slug. With --environment, run against that project's hosted database through the control plane instead of a local connection. Cannot be combined with local connection flags.
    #[arg(long, requires = "environment", conflicts_with = "ConnectionArgs")]
    pub project: Option<String>,
    /// Hosted environment slug, for example `development`. It selects exactly one hosted database. Requires --project.
    #[arg(long, requires = "project")]
    pub environment: Option<String>,
}
impl RemoteArgs {
    pub fn target(&self) -> Option<(&str, &str)> {
        match (&self.project, &self.environment) {
            (Some(project), Some(environment)) => Some((project, environment)),
            _ => None,
        }
    }
}

#[derive(Args, Clone, Debug)]
pub struct PullArgs {
    #[arg(value_name = "CONTRACT")]
    pub source: PathBuf,
    /// AxiomCore Cloud project slug whose hosted history to download
    #[arg(long)]
    pub project: String,
    /// Hosted environment slug, for example `development`
    #[arg(long)]
    pub environment: String,
    /// Print machine-readable JSON instead of text
    #[arg(long)]
    pub json: bool,
}

/// One hosted environment of one project, reached through the control plane.
pub(crate) struct Hosted<'a> {
    pub client: &'a CloudClient,
    pub project: &'a str,
    pub environment: &'a str,
    pub wait: WaitPolicy,
}

// Entry points. Input is validated before the session is loaded, so a bad
// slug fails before authentication or any network call. Clap already refused
// --project and --environment with local connection flags.
pub(crate) fn target<'a>(remote: Option<(&'a str, &'a str)>) -> Result<(&'a str, &'a str)> {
    let (project, environment) =
        remote.context("hosted commands need both --project and --environment")?;
    api::check_environment(environment)?;
    if project.is_empty() || project.len() > 128 {
        bail!("AXRESOURCE_INVALID: invalid project slug");
    }
    Ok((project, environment))
}
macro_rules! hosted {
    ($remote:expr, |$h:ident| $body:expr) => {{
        let (project, environment) = target($remote)?;
        let client = crate::auth_store::authenticated_cloud_client()?;
        let $h = Hosted {
            client: &client,
            project,
            environment,
            wait: WaitPolicy::default(),
        };
        $body
    }};
}
pub async fn migrate(args: &MigrateArgs) -> Result<()> {
    hosted!(args.remote.target(), |h| migrate_with(&h, args, &mut std::io::stdout()).await)
}
pub async fn rollback(args: &RollbackArgs) -> Result<()> {
    hosted!(args.remote.target(), |h| rollback_with(&h, args, &mut std::io::stdout()).await)
}
pub async fn status(args: &ReadArgs) -> Result<()> {
    hosted!(args.remote.target(), |h| status_with(&h, args, &mut std::io::stdout()).await)
}
pub async fn history(source: &Path, remote: &RemoteArgs, json: bool) -> Result<()> {
    hosted!(remote.target(), |h| history_with(&h, source, json, &mut std::io::stdout()).await)
}
pub async fn diff(source: &Path, project: &str, environment: &str, json: bool) -> Result<()> {
    hosted!(Some((project, environment)), |h| diff_with(
        &h,
        source,
        json,
        &mut std::io::stdout()
    )
    .await)
}
pub async fn pull(args: &PullArgs) -> Result<()> {
    hosted!(Some((&args.project, &args.environment)), |h| pull_with(
        &h,
        args,
        &mut std::io::stdout()
    )
    .await)
}

pub(crate) fn idempotency_key() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn short(sha: &str) -> &str {
    &sha[..sha.len().min(12)]
}
pub(crate) fn engine_error(error: axiom_database::Error) -> anyhow::Error {
    anyhow::anyhow!(error.to_string())
}

/// Requests an operation and waits for it to finish.
async fn run_operation(h: &Hosted<'_>, mode: Mode, release: Option<&str>) -> Result<Operation> {
    let operation = h
        .client
        .request_migration_operation(h.project, h.environment, mode, release, &idempotency_key())
        .await?;
    if operation.finished() {
        return Ok(operation);
    }
    h.client
        .wait_migration_operation(h.project, &operation.id, h.wait)
        .await
}
fn outcome_word(operation: &Operation) -> &'static str {
    match operation.outcome.as_deref() {
        Some("validated") => "validated",
        Some("applied") => "applied",
        Some("already_applied") => "already_applied",
        Some("rejected") => "rejected",
        Some("failed") => "failed",
        _ if operation.succeeded() => "succeeded",
        _ => "failed",
    }
}
fn failure(operation: &Operation) -> anyhow::Error {
    let outcome = outcome_word(operation);
    match operation.known_code() {
        Some(code) => anyhow::anyhow!(
            "{code}: {} Hosted operation {} {outcome}.",
            api::known_code(code).unwrap_or_default(),
            operation.id
        ),
        None => anyhow::anyhow!(
            "Hosted operation {} {outcome} without a known code. Inspect it with `axiom database history`.",
            operation.id
        ),
    }
}

/// The ledger head of `stream`, from a hosted `status` run.
pub(crate) async fn hosted_head(h: &Hosted<'_>, stream: &str) -> Result<Option<OperationTarget>> {
    let operation = run_operation(h, Mode::Status, None).await?;
    if !operation.succeeded() {
        return Err(failure(&operation));
    }
    Ok(operation.target(stream).cloned())
}

/// Downloads one release and verifies its digest, signature and source.
pub(crate) async fn fetch_release(h: &Hosted<'_>, sha256: &str) -> Result<Release> {
    let bytes = h.client.migration_release(h.project, sha256).await?;
    let release: Release = serde_json::from_slice(&bytes).map_err(|_| {
        anyhow::anyhow!(
            "ADB1101: hosted release {} is not a database release",
            short(sha256)
        )
    })?;
    if release.sha256 != sha256 {
        bail!(
            "ADB1101: hosted release {} does not carry its own digest",
            short(sha256)
        );
    }
    release.verify().map_err(|error| {
        anyhow::anyhow!(
            "{}: hosted release {} failed verification: {}",
            error.code,
            short(sha256),
            error.message
        )
    })?;
    Ok(release)
}

/// The applied history ending at `head`, oldest first. Local copies are used
/// when the digest matches (the catalog verified them on read); the rest are
/// downloaded and verified. The chain must follow parent links and state
/// continuity exactly as a ledger does, end at the head with its generation
/// and key, and, when `signers` is given, be signed by registry keys (revoked
/// keys included, because history signed before revocation stays valid).
async fn hosted_chain(
    h: &Hosted<'_>,
    stream: &str,
    head: Option<&OperationTarget>,
    local: &[Release],
    signers: Option<&[Signer]>,
) -> Result<Vec<Release>> {
    let Some(head) = head else {
        return Ok(vec![]);
    };
    if head.generation == 0 || head.generation > 4096 {
        bail!("ADB1101: the hosted head generation is out of bounds");
    }
    let mut chain = vec![];
    let mut cursor = Some(head.head.clone());
    while let Some(sha256) = cursor {
        if chain.len() as u64 >= head.generation {
            bail!("ADB1101: the hosted history is longer than its head generation");
        }
        if !api::is_lower_hex(&sha256, 32) {
            bail!("ADB1101: the hosted history has an invalid parent digest");
        }
        let release = match local.iter().find(|r| r.sha256 == sha256) {
            Some(release) => release.clone(),
            None => fetch_release(h, &sha256).await?,
        };
        cursor = release.revision.parent.clone();
        chain.push(release);
    }
    chain.reverse();
    verify_chain(&chain, head, stream, signers)?;
    Ok(chain)
}
fn verify_chain(
    chain: &[Release],
    head: &OperationTarget,
    stream: &str,
    signers: Option<&[Signer]>,
) -> Result<()> {
    let mut previous: Option<&Release> = None;
    for release in chain {
        let version = &release.revision.identity.version;
        if release.revision.identity.name != stream {
            bail!("ADB1101: hosted release {version} belongs to another contract ID");
        }
        if release.revision.parent.as_deref() != previous.map(|p| p.sha256.as_str())
            || release.revision.base.as_ref() != previous.map(|p| &p.revision.desired)
            || release.revision.previous_inputs.as_ref() != previous.map(|p| &p.revision.inputs)
        {
            bail!("ADB1101: hosted release {version} does not follow its parent; nothing was written");
        }
        if let Some(signers) = signers {
            if !signers.iter().any(|s| s.public_key == release.public_key) {
                bail!("ADB1109: hosted release {version} is signed by a key outside the project signer registry");
            }
        }
        previous = Some(release);
    }
    if chain.len() as u64 != head.generation
        || chain.last().map(|r| r.sha256.as_str()) != Some(head.head.as_str())
        || chain.last().map(|r| r.public_key.as_str()) != Some(head.public_key.as_str())
    {
        bail!("ADB1101: the hosted history does not match its ledger head");
    }
    Ok(())
}

/// Local releases of `stream` that conflict with the hosted history: the same
/// version bound to other content, or a release that branches off the hosted
/// history below its head (or from another root).
fn divergence<'a>(chain: &[Release], local: &'a [Release], stream: &str) -> Vec<&'a Release> {
    let head = chain.last().map(|r| r.sha256.as_str());
    let in_chain = |sha: &str| chain.iter().any(|r| r.sha256 == sha);
    local
        .iter()
        .filter(|l| l.revision.identity.name == stream && !in_chain(&l.sha256))
        .filter(|l| {
            if chain
                .iter()
                .any(|r| r.revision.identity == l.revision.identity)
            {
                return true;
            }
            let mut cursor = l.revision.parent.clone();
            for _ in 0..=local.len() {
                match cursor {
                    None => return !chain.is_empty(),
                    Some(ref parent) if in_chain(parent) => return Some(parent.as_str()) != head,
                    Some(parent) => match local.iter().find(|r| r.sha256 == parent) {
                        Some(r) => cursor = r.revision.parent.clone(),
                        None => return false,
                    },
                }
            }
            false
        })
        .collect()
}

/// Fails unless every key is enrolled and unrevoked for the project.
pub(crate) async fn require_enrolled<'k>(
    h: &Hosted<'_>,
    keys: impl IntoIterator<Item = &'k str>,
) -> Result<()> {
    let signers = h.client.migration_signers(h.project).await?;
    for key in keys {
        match signers.iter().find(|s| s.public_key == key) {
            None => bail!(
                "ADB1109: signing key {key} is not enrolled for project {}. Run `axiom project signers add --project {} --schema <CONTRACT>` first.",
                h.project,
                h.project
            ),
            Some(signer) if signer.revoked_at.is_some() => bail!(
                "ADB1109: signing key {key} is revoked for project {}; enroll a new key with `axiom project signers add`.",
                h.project
            ),
            Some(_) => {}
        }
    }
    Ok(())
}

pub(crate) async fn upload(h: &Hosted<'_>, release: &Release) -> Result<()> {
    // The catalog stores this exact canonical encoding.
    let body = axiom_database::canonical_bytes(release).map_err(engine_error)?;
    h.client
        .upload_migration_release(h.project, &body, &idempotency_key())
        .await?;
    Ok(())
}

/// Attaches the release's signed impact sidecar, when the catalog holds a valid one
/// (`data-tools/v1`, "Storage: signed sidecar"). Never fails the migration: the sidecar
/// is re-attachable, and a refusal is reported on standard error outside JSON mode.
async fn upload_impact(h: &Hosted<'_>, root: &Path, release: &Release, json: bool) {
    let Some(body) = super::database_impact::upload_body(root, release) else { return };
    if let Err(error) = h.client.upload_migration_impact(h.project, &release.sha256, &body, &idempotency_key()).await {
        if !json {
            eprintln!("The impact report for {} was not attached: {error}", release.revision.identity.version);
        }
    }
}

/// Prints a finished `migrate` or `check` operation, or fails with its code.
fn report(
    out: &mut dyn Write,
    operation: &Operation,
    release: &Release,
    stream: &str,
    json: bool,
) -> Result<()> {
    let outcome = outcome_word(operation);
    if !operation.succeeded() || matches!(outcome, "rejected" | "failed") {
        return Err(failure(operation));
    }
    let target = operation.target(stream);
    if outcome == "applied" && target.map(|t| t.head.as_str()) != Some(release.sha256.as_str()) {
        bail!(
            "ADB1104: hosted operation {} reported applied, but the hosted head is not this release",
            operation.id
        );
    }
    let version = &release.revision.identity.version;
    if json {
        writeln!(
            out,
            "{}",
            serde_json::json!({"format":"axiom-database-migration-result/v1","version":version,"kind":release.revision.kind,"release":release.sha256,"operation":operation.id,"outcome":outcome,"head":target.map(|t| &t.head),"generation":target.map(|t| t.generation)})
        )?;
        return Ok(());
    }
    let head = target.map_or("none".to_string(), |t| {
        format!("{} (generation {})", t.head, t.generation)
    });
    let noun = if release.revision.kind == "rollback" {
        "rollback"
    } else {
        "migration"
    };
    match outcome {
        "applied" => writeln!(out, "Applied {noun} {version} · hosted head {head}")?,
        "already_applied" => writeln!(
            out,
            "The {noun} {version} was already applied · hosted head {head}"
        )?,
        "validated" => writeln!(
            out,
            "Hosted check passed for {noun} {version}; nothing was changed · hosted head {head}"
        )?,
        _ => writeln!(
            out,
            "Hosted operation {} {outcome} · hosted head {head}",
            operation.id
        )?,
    }
    Ok(())
}

fn banner(out: &mut dyn Write, h: &Hosted<'_>, json: bool) -> Result<()> {
    if !json {
        writeln!(out, "Hosted database {}/{}", h.project, h.environment)?;
    }
    Ok(())
}

/// `migrate` against a hosted environment: the local release selection and
/// catalog, then upload, a `migrate` (or `check`) operation per pending release,
/// and the new head.
pub(crate) async fn migrate_with(
    h: &Hosted<'_>,
    args: &MigrateArgs,
    out: &mut dyn Write,
) -> Result<()> {
    let mut loaded = dev::load(&args.source)?;
    let dry_run = args.policy.dry_run;
    let json = args.policy.json;
    banner(out, h, json)?;
    let _lock = db::CatalogLock::acquire(&loaded.root).map_err(engine_error)?;
    if !dry_run && args.source.extension().is_some_and(|e| e == "acore") {
        axiom_lib::project_state::import_legacy(&args.source)?;
    }
    dev::recover_source_edit(&loaded.root)?;
    loaded = dev::load(&args.source)?;
    let stream = loaded.compiled.identity.name.clone();
    let head = hosted_head(h, &stream).await?;
    let local = db::archive_versions(&loaded.root).map_err(engine_error)?;
    let history = hosted_chain(h, &stream, head.as_ref(), &local, None).await?;
    // data-tools/v1: a hosted head the developer has not pulled, whose unpulled history
    // holds a dashboard capture, would recompile stale sources over the capture.
    if head.as_ref().is_some_and(|t| !local.iter().any(|l| l.sha256 == t.head))
        && history.iter().any(|r| r.revision.capture.is_some() && !local.iter().any(|l| l.sha256 == r.sha256))
    {
        bail!("ADB1103: the hosted head includes a dashboard capture that is not in the local catalog; run `axiom database pull` first");
    }
    let catalog = dev::migration_catalog(&loaded, &history)?;
    let (_, pending_preview) =
        db::working_preview(history.last(), &loaded.compiled).map_err(engine_error)?;
    let original = loaded.input.clone();
    let Some(release) = dev::select_release(
        args,
        &mut loaded,
        &catalog,
        history.last(),
        &pending_preview,
        true,
    )?
    else {
        return Ok(());
    };
    let applied = history.last().map(|r| r.sha256.as_str());
    if applied == Some(release.sha256.as_str()) {
        if json {
            writeln!(
                out,
                "{}",
                serde_json::json!({"format":"axiom-database-migration-result/v1","version":release.revision.identity.version,"status":"current","applied":false,"head":release.sha256})
            )?;
        } else {
            writeln!(
                out,
                "Database is current · contract {} · hosted head {}",
                release.revision.identity.version, release.sha256
            )?;
        }
        return export(args, history);
    }
    if !json {
        dev::show_preview(
            &release.revision.identity.version,
            &release.revision.preview,
        );
    }
    let pending = dev::prefix(&release, &catalog, applied)?;
    // A check validates the next release against the current head, so a dry
    // run checks only the first pending release.
    let candidates = if dry_run { &pending[..1] } else { &pending[..] };
    if !dry_run {
        for candidate in candidates {
            dev::approve(&args.policy, &candidate.revision.preview)?;
        }
    }
    require_enrolled(h, candidates.iter().map(|r| r.public_key.as_str())).await?;
    let mut chain = history;
    for (index, candidate) in candidates.iter().enumerate() {
        if !dry_run {
            let fresh = super::database_impact::fresh(&loaded.root, candidate);
            db::archive(&loaded.root, candidate).map_err(engine_error)?;
            if fresh {
                super::database_impact::write_quietly(&loaded.root, &args.source, &args.impact, candidate, json);
            }
            if index + 1 == pending.len()
                && loaded.input.files != original.files
                && !loaded.immutable
            {
                dev::save_input(&loaded.root, &original, &loaded.input)?;
            }
        }
        upload(h, candidate).await?;
        if !dry_run {
            upload_impact(h, &loaded.root, candidate, json).await;
        }
        let mode = if dry_run { Mode::Check } else { Mode::Migrate };
        let operation = run_operation(h, mode, Some(&candidate.sha256)).await?;
        report(out, &operation, candidate, &stream, json)?;
        chain.push(candidate.clone());
    }
    if dry_run {
        if pending.len() > 1 && !json {
            writeln!(
                out,
                "{} more pending release(s) can be checked after {} is applied.",
                pending.len() - 1,
                pending[0].revision.identity.version
            )?;
        }
        return Ok(());
    }
    export(args, chain)
}
fn export(args: &MigrateArgs, chain: Vec<Release>) -> Result<()> {
    if let Some(path) = &args.policy.export {
        if !args.policy.dry_run && !chain.is_empty() {
            db::write_bundle(path, chain).map_err(engine_error)?;
        }
    }
    Ok(())
}

/// `rollback` against a hosted environment: the local compensating release,
/// then upload and a `migrate` (or `check`) operation.
pub(crate) async fn rollback_with(
    h: &Hosted<'_>,
    args: &RollbackArgs,
    out: &mut dyn Write,
) -> Result<()> {
    let (loaded, diagnostic) = dev::load_existing(&args.source)?;
    if let Some(diagnostic) = diagnostic {
        eprintln!("{diagnostic}");
    }
    let json = args.policy.json;
    let dry_run = args.policy.dry_run;
    banner(out, h, json)?;
    let _lock = db::CatalogLock::acquire(&loaded.root).map_err(engine_error)?;
    let stream = loaded.compiled.identity.name.clone();
    let head = hosted_head(h, &stream).await?;
    let local = db::archive_versions(&loaded.root).map_err(engine_error)?;
    let history = hosted_chain(h, &stream, head.as_ref(), &local, None).await?;
    let Some((revision, _version)) = dev::rollback_revision(args, &loaded, &history)? else {
        return Ok(());
    };
    if !dry_run {
        dev::approve(&args.policy, &revision.preview)?;
    }
    let release = db::sign(revision).map_err(engine_error)?;
    require_enrolled(h, [release.public_key.as_str()]).await?;
    if !dry_run {
        db::archive(&loaded.root, &release).map_err(engine_error)?;
    }
    upload(h, &release).await?;
    let mode = if dry_run { Mode::Check } else { Mode::Migrate };
    let operation = run_operation(h, mode, Some(&release.sha256)).await?;
    report(out, &operation, &release, &stream, json)?;
    if !dry_run {
        if let Some(path) = &args.policy.export {
            let mut chain = history;
            chain.push(release);
            db::write_bundle(path, chain).map_err(engine_error)?;
        }
    }
    Ok(())
}

/// `status` against a hosted environment: a hosted `status` run for the head,
/// then the local comparison. Drift here means the hosted history diverges
/// from the local catalog; the hosted executor checks database drift itself
/// before every apply.
pub(crate) async fn status_with(
    h: &Hosted<'_>,
    args: &ReadArgs,
    out: &mut dyn Write,
) -> Result<()> {
    let (loaded, diagnostic) = dev::load_existing(&args.source)?;
    let stream = loaded.compiled.identity.name.clone();
    let head = hosted_head(h, &stream).await?;
    let local = db::archive_versions(&loaded.root).map_err(engine_error)?;
    let chain = hosted_chain(h, &stream, head.as_ref(), &local, None).await?;
    let (mut pending, preview) =
        db::working_preview(chain.last(), &loaded.compiled).map_err(engine_error)?;
    let mut working = loaded.compiled.identity.version.clone();
    if let Some(diagnostic) = diagnostic {
        eprintln!("{diagnostic}");
        pending = true;
        working = "invalid".into();
    }
    let missing = chain
        .iter()
        .filter(|r| !local.iter().any(|l| l.sha256 == r.sha256))
        .map(|r| r.revision.identity.version.clone())
        .collect::<Vec<_>>();
    let diverged = divergence(&chain, &local, &stream)
        .into_iter()
        .map(|r| r.revision.identity.version.clone())
        .collect::<Vec<_>>();
    let drift = !diverged.is_empty();
    let applied = chain.last().map(|r| r.revision.identity.version.clone());
    if args.json {
        writeln!(
            out,
            "{}",
            serde_json::json!({"format":"axiom-database-hosted-status/v1","project":stream,"engine":loaded.compiled.contract.engine,"cloudProject":h.project,"environment":h.environment,"workingVersion":working,"appliedVersion":applied,"head":head.as_ref().map(|t| &t.head),"generation":head.as_ref().map(|t| t.generation),"drift":drift,"pending":pending,"divergedLocalVersions":diverged,"missingLocalVersions":missing,"preview":preview})
        )?;
    } else {
        writeln!(
            out,
            "Contract {stream} · {} · hosted {}/{}",
            loaded.compiled.contract.engine, h.project, h.environment
        )?;
        writeln!(
            out,
            "Applied: {} · Working: {working} · Drift: {}",
            applied.as_deref().unwrap_or("not initialized"),
            if drift { "detected" } else { "none" }
        )?;
        if let Some(head) = &head {
            writeln!(out, "Hosted head: {} (generation {})", head.head, head.generation)?;
        }
        for version in &diverged {
            writeln!(
                out,
                "  Local release {version} diverges from the hosted history (ADB1103)."
            )?;
        }
        if !missing.is_empty() {
            writeln!(
                out,
                "Not in the local catalog: {}. Run `axiom database pull` to download them.",
                missing.join(", ")
            )?;
        }
        if pending {
            dev::show_preview(&working, &preview);
        } else {
            writeln!(out, "Database matches the contract.")?;
        }
    }
    if args.check && (drift || pending) {
        bail!("ADB1105: database is not current");
    }
    Ok(())
}

/// `history` against a hosted environment: the applied releases and the
/// environment's operations, newest operation first, with local presence
/// markers. Reads only; nothing runs and nothing is written.
pub(crate) async fn history_with(
    h: &Hosted<'_>,
    source: &Path,
    json: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let (loaded, diagnostic) = dev::load_existing(source)?;
    if let Some(diagnostic) = diagnostic {
        eprintln!("{diagnostic}");
    }
    let stream = loaded.compiled.identity.name.clone();
    let operations = h
        .client
        .migration_operations(h.project, Some(h.environment))
        .await?;
    let releases = h.client.migration_releases(h.project, Some(&stream)).await?;
    let local = db::archive_versions(&loaded.root).map_err(engine_error)?;
    let head = operations
        .iter()
        .filter(|o| o.succeeded())
        .find_map(|o| o.target(&stream))
        .cloned();
    let mut applied = vec![];
    let mut cursor = head.as_ref().map(|t| t.head.clone());
    while let Some(sha256) = cursor {
        let Some(release) = releases.iter().find(|r| r.sha256 == sha256) else {
            break;
        };
        if applied.len() > 4096 {
            break;
        }
        applied.push(release);
        cursor = release.parent.clone();
    }
    applied.reverse();
    let marker = |sha256: &str, version: &str| {
        if local.iter().any(|l| l.sha256 == sha256) {
            "local"
        } else if local.iter().any(|l| l.revision.identity.version == version) {
            "differs from local"
        } else {
            "not in local catalog"
        }
    };
    let version_of = |sha256: &Option<String>| {
        sha256.as_ref().map(|sha| {
            releases
                .iter()
                .find(|r| &r.sha256 == sha)
                .map_or_else(|| short(sha).to_string(), |r| r.version.clone())
        })
    };
    if json {
        let releases = applied.iter().map(|r| serde_json::json!({"version":r.version,"kind":r.kind,"sha256":r.sha256,"parent":r.parent,"publicKey":r.public_key,"uploadedAt":r.uploaded_at,"local":marker(&r.sha256,&r.version)})).collect::<Vec<_>>();
        let operations = operations.iter().map(|o| serde_json::json!({"id":o.id,"mode":o.mode,"status":o.status,"outcome":o.outcome.as_ref().map(|_| outcome_word(o)),"code":o.known_code(),"release":o.release,"version":version_of(&o.release),"createdAt":o.created_at,"finishedAt":o.finished_at})).collect::<Vec<_>>();
        writeln!(
            out,
            "{}",
            serde_json::json!({"format":"axiom-database-hosted-history/v1","project":stream,"cloudProject":h.project,"environment":h.environment,"head":head.as_ref().map(|t| &t.head),"generation":head.as_ref().map(|t| t.generation),"releases":releases,"operations":operations})
        )?;
        return Ok(());
    }
    writeln!(
        out,
        "Contract {stream} · hosted {}/{}",
        h.project, h.environment
    )?;
    match &head {
        Some(head) => writeln!(out, "Head: {} (generation {})", head.head, head.generation)?,
        None => writeln!(out, "No applied migrations recorded.")?,
    }
    for release in &applied {
        let captured = local.iter().find(|l| l.sha256 == release.sha256).and_then(super::database_capture::label);
        writeln!(
            out,
            "{}  {}  {}  {}{}",
            release.version,
            release.kind,
            short(&release.sha256),
            marker(&release.sha256, &release.version),
            captured.map(|c| format!("  {c}")).unwrap_or_default()
        )?;
    }
    if head
        .as_ref()
        .is_some_and(|h| applied.len() as u64 != h.generation)
    {
        writeln!(
            out,
            "Older applied releases are not listed by the hosted release index."
        )?;
    }
    if !operations.is_empty() {
        writeln!(out, "Operations, newest first:")?;
    }
    for operation in &operations {
        let mut line = format!(
            "{}  {}  {}",
            operation.created_at,
            operation.mode,
            match operation.status.as_str() {
                s @ ("pending" | "running" | "retrying" | "succeeded" | "failed") => s,
                _ => "unknown",
            }
        );
        if operation.outcome.is_some() {
            line.push_str(&format!("  {}", outcome_word(operation)));
        }
        if let Some(code) = operation.known_code() {
            line.push_str(&format!("  {code}"));
        }
        if let Some(version) = version_of(&operation.release) {
            line.push_str(&format!("  {version}"));
        }
        writeln!(out, "{line}")?;
    }
    Ok(())
}

/// `diff` against a hosted environment: the hosted head release compared with
/// the working contract.
pub(crate) async fn diff_with(
    h: &Hosted<'_>,
    source: &Path,
    json: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let loaded = dev::load(source)?;
    let stream = loaded.compiled.identity.name.clone();
    let head = hosted_head(h, &stream).await?;
    let local = db::archive_versions(&loaded.root).map_err(engine_error)?;
    let release = match &head {
        None => None,
        Some(target) => Some(match local.iter().find(|r| r.sha256 == target.head) {
            Some(release) => release.clone(),
            None => fetch_release(h, &target.head).await?,
        }),
    };
    let preview = match &release {
        Some(release) => db::comparison(
            &release.revision.desired,
            &loaded.compiled.contract,
            release.revision.evolution.as_ref().map(|e| e.source.as_str()),
            loaded.compiled.migration_source.as_deref(),
        )
        .map_err(engine_error)?,
        None => axiom_lib::database::migration::preview(None, &loaded.compiled.contract)?,
    };
    if json {
        writeln!(out, "{}", serde_json::to_string(&preview)?)?;
    } else {
        let from = release
            .as_ref()
            .map_or("empty".to_string(), |r| r.revision.identity.version.clone());
        writeln!(out, "hosted {from} → working")?;
        out.flush()?;
        dev::show_preview("working", &preview);
    }
    Ok(())
}

/// `pull`: downloads the applied history into the local catalog. Every
/// release is verified before anything is written; a history that diverges
/// from local releases is refused; existing local releases are never
/// rewritten; nothing is executed against the database.
pub(crate) async fn pull_with(h: &Hosted<'_>, args: &PullArgs, out: &mut dyn Write) -> Result<()> {
    let (loaded, diagnostic) = dev::load_existing(&args.source)?;
    let working = diagnostic.is_none() && !loaded.immutable;
    if let Some(diagnostic) = diagnostic {
        eprintln!("{diagnostic}");
    }
    let stream = loaded.compiled.identity.name.clone();
    let _lock = db::CatalogLock::acquire(&loaded.root).map_err(engine_error)?;
    let head = hosted_head(h, &stream).await?;
    let signers = h.client.migration_signers(h.project).await?;
    let local = db::archive_versions(&loaded.root).map_err(engine_error)?;
    let chain = hosted_chain(h, &stream, head.as_ref(), &local, Some(&signers)).await?;
    let conflicts = divergence(&chain, &local, &stream);
    if !conflicts.is_empty() {
        bail!(
            "ADB1103: the hosted history diverges from local release(s) {}; nothing was written. Reconcile the local catalog before pulling.",
            conflicts
                .iter()
                .map(|r| r.revision.identity.version.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    // Dashboard captures (data-tools/v1): the incoming region and the mechanical edits are
    // planned, and the result compiled, before anything is written.
    // Fail closed (review H-4): a capture that still has to be applied is never archived
    // unless its region can be written to a working source tree.
    let pulled = local.iter().map(|r| r.sha256.clone()).collect::<std::collections::BTreeSet<_>>();
    if super::database_capture::pending(&chain, &pulled).is_some() && !working {
        bail!("ADB1104: the hosted history holds a dashboard capture, and the working source does not compile (or is a release file); fix the source and pull again. Nothing was written.");
    }
    let sources = if working {
        super::database_capture::plan(&chain, &pulled, &loaded.input.entry, &loaded.input.files)?
    } else {
        None
    };
    if let Some((_, plan)) = &sources {
        let mut input = loaded.input.clone();
        for (path, text) in &plan.files {
            match text {
                Some(text) => input.files.insert(path.clone(), text.clone()),
                None => input.files.remove(path),
            };
        }
        if acore::database::compile(&input).is_err() {
            bail!("ADB1104: the working sources with the pulled dashboard region do not compile; nothing was written");
        }
    }
    // Every path check runs, and every new text is staged, before anything is archived.
    let staged = match &sources {
        Some((incoming, plan)) if !plan.files.is_empty() => Some(super::database_capture::stage(&loaded.root, incoming, plan)?),
        _ => None,
    };
    let mut added = vec![];
    let mut present = vec![];
    for release in &chain {
        if local.iter().any(|l| l.sha256 == release.sha256) {
            present.push(release);
        } else {
            let path = db::archive(&loaded.root, release).map_err(engine_error)?;
            added.push((release, path));
        }
    }
    let written = staged.map(super::database_capture::Staged::apply).transpose()?;
    let region = sources.as_ref().map(|(incoming, plan)| serde_json::json!({"incoming":incoming.revision.identity.version,"replaced":plan.replaced,"kept":plan.kept,"version":plan.version,"removedMigration":plan.removed_migration,"files":plan.files.keys().collect::<Vec<_>>(),"backup":written.as_ref().map(|b| b.strip_prefix(&loaded.root).unwrap_or(b).display().to_string())}));
    if args.json {
        writeln!(
            out,
            "{}",
            serde_json::json!({"format":"axiom-database-hosted-pull/v1","project":stream,"head":head.as_ref().map(|t| &t.head),"generation":head.as_ref().map(|t| t.generation),"added":added.iter().map(|(r, _)| &r.revision.identity.version).collect::<Vec<_>>(),"present":present.iter().map(|r| &r.revision.identity.version).collect::<Vec<_>>(),"dashboard":region,"executed":false})
        )?;
        return Ok(());
    }
    let Some(head) = &head else {
        writeln!(
            out,
            "The hosted database has no applied migrations for {stream}; nothing was written."
        )?;
        return Ok(());
    };
    writeln!(
        out,
        "Hosted history for {stream} from {}/{}: {} release(s), head {} (generation {})",
        h.project,
        h.environment,
        chain.len(),
        head.head,
        head.generation
    )?;
    for (release, path) in &added {
        writeln!(
            out,
            "  added {}  {}  {}  {}{}",
            release.revision.identity.version,
            release.revision.kind,
            short(&release.sha256),
            path.strip_prefix(&loaded.root).unwrap_or(path).display(),
            super::database_capture::label(release).map(|l| format!("  {l}")).unwrap_or_default()
        )?;
    }
    if let Some((incoming, plan)) = &sources {
        for schema in &plan.replaced {
            writeln!(out, "  dashboard region of schema {schema} updated from {}", incoming.revision.identity.version)?;
        }
        for schema in &plan.kept {
            writeln!(out, "  dashboard region of schema {schema} kept (local edit)")?;
        }
        if let Some(version) = &plan.version {
            writeln!(out, "  project version set to {version}")?;
        }
        if let Some(path) = &plan.removed_migration {
            writeln!(out, "  migration(source: \"{path}\") and its file removed (applied by the capture)")?;
        }
        if let Some(backup) = &written {
            writeln!(out, "  originals copied to {}", backup.strip_prefix(&loaded.root).unwrap_or(backup).display())?;
        }
    }
    for release in &present {
        writeln!(
            out,
            "  kept  {}  already in the local catalog",
            release.revision.identity.version
        )?;
    }
    if added.is_empty() {
        writeln!(out, "The local catalog already has the hosted history.")?;
    }
    writeln!(out, "Nothing was executed.")?;
    Ok(())
}

/// A loopback fake of the hosted migrations API with a simulated ledger.
#[cfg(test)]
pub(crate) mod fake {
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub const PROJECT_ID: &str = "0b0e7c8e-2f6c-4c79-9b9f-1c1d2e3f4a5b";

    #[derive(Default)]
    pub struct Cloud {
        pub signers: Vec<Value>,
        pub releases: Vec<(String, Vec<u8>)>,
        /// (head, generation, public key) of the one ledger stream.
        pub head: Option<(String, u64, String)>,
        pub stream: String,
        pub operations: Vec<Value>,
        pub polls: BTreeMap<String, usize>,
        /// When set, the next `migrate` or `check` run is rejected with this code.
        pub reject: Option<String>,
        /// Method, path (with query) and body of every request.
        pub requests: Vec<(String, String, Vec<u8>)>,
    }
    impl Cloud {
        pub fn count(&self, method: &str, path: &str) -> usize {
            self.requests
                .iter()
                .filter(|(m, p, _)| m == method && p.starts_with(path))
                .count()
        }
        pub fn bodies(&self, method: &str, path: &str) -> Vec<Vec<u8>> {
            self.requests
                .iter()
                .filter(|(m, p, _)| m == method && p == path)
                .map(|(_, _, b)| b.clone())
                .collect()
        }
        fn target(&self) -> Value {
            match &self.head {
                Some((head, generation, key)) => {
                    json!([{"project": self.stream, "head": head, "generation": generation, "public_key": key}])
                }
                None => json!([]),
            }
        }
        fn summary(body: &[u8]) -> Option<Value> {
            let v: Value = serde_json::from_slice(body).ok()?;
            Some(json!({"sha256": v["sha256"], "stream": v["revision"]["identity"]["name"], "version": v["revision"]["identity"]["version"], "parent": v["revision"]["parent"], "kind": v["revision"]["kind"], "engine": v["revision"]["desired"]["engine"], "public_key": v["publicKey"], "uploaded_at": "2026-10-20T00:00:00Z"}))
        }
        fn finish(&mut self, index: usize) {
            let op = self.operations[index].clone();
            let mode = op["mode"].as_str().unwrap_or_default().to_string();
            let release = op["release"].as_str().map(str::to_string);
            let parent_ok = |cloud: &Cloud| -> Option<Value> {
                let sha = release.clone()?;
                let (_, body) = cloud.releases.iter().find(|(s, _)| *s == sha)?;
                let v: Value = serde_json::from_slice(body).ok()?;
                let parent = v["revision"]["parent"].as_str().map(str::to_string);
                (parent == cloud.head.as_ref().map(|h| h.0.clone())).then_some(v)
            };
            let rejected = (mode != "status").then(|| self.reject.take()).flatten();
            let (status, outcome, code) = match mode.as_str() {
                "status" => ("succeeded", "validated", Value::Null),
                _ if rejected.is_some() => ("failed", "rejected", json!(rejected)),
                "check" => match parent_ok(self) {
                    Some(_) => ("succeeded", "validated", Value::Null),
                    None => ("failed", "rejected", json!("ADB1104")),
                },
                _ => match parent_ok(self) {
                    Some(v) => {
                        let generation = self.head.as_ref().map_or(0, |h| h.1) + 1;
                        self.head = Some((
                            release.clone().unwrap(),
                            generation,
                            v["publicKey"].as_str().unwrap().to_string(),
                        ));
                        ("succeeded", "applied", Value::Null)
                    }
                    None => ("failed", "rejected", json!("ADB1104")),
                },
            };
            let targets = self.target();
            let op = &mut self.operations[index];
            op["status"] = json!(status);
            op["outcome"] = json!(outcome);
            op["code"] = code;
            op["targets"] = targets;
            op["finished_at"] = json!("2026-10-20T00:00:01Z");
        }
        fn handle(&mut self, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
            self.requests
                .push((method.to_string(), path.to_string(), body.to_vec()));
            let route = path.split('?').next().unwrap_or_default();
            let Some(rest) = route.strip_prefix("/api/v1/projects/acme") else {
                return (404, b"{}".to_vec());
            };
            let ok = |v: Value| (200, v.to_string().into_bytes());
            match (method, rest) {
                ("GET", "") => ok(json!({"id": PROJECT_ID, "slug": "acme"})),
                ("GET", "/migrations/signers") => ok(json!({"signers": self.signers})),
                ("POST", "/migrations/signers") => {
                    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
                    let v: Value = serde_json::from_slice(body).unwrap();
                    let key = v["public_key"].as_str().unwrap();
                    let message = axiom_cloud::migrations::signer_proof_message(PROJECT_ID, key);
                    let verified = (|| {
                        let key = VerifyingKey::from_bytes(&hex::decode(key).ok()?.try_into().ok()?).ok()?;
                        let signature = Signature::from_bytes(&hex::decode(v["proof"].as_str()?).ok()?.try_into().ok()?);
                        key.verify(message.as_bytes(), &signature).ok()
                    })();
                    if verified.is_none() {
                        return (400, br#"{"error":{"code":"AXRESOURCE_INVALID","message":"x"}}"#.to_vec());
                    }
                    let signer = json!({"public_key": key, "label": v["label"], "added_at": "2026-10-20T00:00:00Z", "revoked_at": null});
                    self.signers.push(signer.clone());
                    (201, signer.to_string().into_bytes())
                }
                ("POST", "/migrations/releases") => {
                    let Some(summary) = Self::summary(body) else {
                        return (400, br#"{"error":{"code":"AXRESOURCE_INVALID"}}"#.to_vec());
                    };
                    let sha = summary["sha256"].as_str().unwrap().to_string();
                    let created = !self.releases.iter().any(|(s, _)| *s == sha);
                    if created {
                        self.releases.push((sha, body.to_vec()));
                    }
                    (if created { 201 } else { 200 }, json!({"release": summary}).to_string().into_bytes())
                }
                ("PUT", r) if r.starts_with("/migrations/releases/") && r.ends_with("/impact") => {
                    let sha = r["/migrations/releases/".len()..r.len() - "/impact".len()].to_string();
                    if !self.releases.iter().any(|(s, _)| *s == sha) {
                        return (404, br#"{"error":{"code":"AXRESOURCE_DENIED"}}"#.to_vec());
                    }
                    (201, json!({"impact": {"release_sha256": sha, "public_key": "x", "uploaded_at": "2026-10-20T00:00:00Z"}}).to_string().into_bytes())
                }
                ("GET", "/migrations/releases") => ok(json!({"releases": self.releases.iter().rev().filter_map(|(_, b)| Self::summary(b)).collect::<Vec<_>>()})),
                ("GET", r) if r.starts_with("/migrations/releases/") => {
                    let sha = &r["/migrations/releases/".len()..];
                    match self.releases.iter().find(|(s, _)| s == sha) {
                        Some((_, body)) => (200, body.clone()),
                        None => (404, br#"{"error":{"code":"AXRESOURCE_INVALID"}}"#.to_vec()),
                    }
                }
                ("POST", "/migrations/operations") => {
                    let v: Value = serde_json::from_slice(body).unwrap();
                    let id = format!("00000000-0000-4000-8000-{:012}", self.operations.len() + 1);
                    let op = json!({"id": id, "environment": v["environment"], "mode": v["mode"], "release": v.get("release").cloned().unwrap_or(Value::Null), "status": "pending", "outcome": null, "code": null, "targets": [], "created_at": format!("2026-10-20T00:00:{:02}Z", self.operations.len()), "finished_at": null});
                    self.operations.push(op.clone());
                    (202, json!({"operation": op}).to_string().into_bytes())
                }
                ("GET", "/migrations/operations") => ok(json!({"operations": self.operations.iter().rev().collect::<Vec<_>>()})),
                ("GET", r) if r.starts_with("/migrations/operations/") => {
                    let id = &r["/migrations/operations/".len()..];
                    let Some(index) = self.operations.iter().position(|o| o["id"] == id) else {
                        return (403, br#"{"error":{"code":"AXRESOURCE_DENIED"}}"#.to_vec());
                    };
                    let polls = self.polls.entry(id.to_string()).or_default();
                    *polls += 1;
                    if *polls == 1 {
                        self.operations[index]["status"] = json!("running");
                    } else if self.operations[index]["finished_at"].is_null() {
                        self.finish(index);
                    }
                    ok(json!({"operation": self.operations[index]}))
                }
                _ => (404, b"{}".to_vec()),
            }
        }
    }

    pub async fn serve(cloud: Arc<Mutex<Cloud>>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                let cloud = cloud.clone();
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let mut chunk = vec![0u8; 65536];
                    let at = loop {
                        let n = stream.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..n]);
                        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                            break at;
                        }
                    };
                    let head = String::from_utf8_lossy(&buffer[..at]).to_string();
                    let mut body = buffer[at + 4..].to_vec();
                    let mut lines = head.split("\r\n");
                    let mut start = lines.next().unwrap_or_default().split(' ');
                    let method = start.next().unwrap_or_default().to_string();
                    let path = start.next().unwrap_or_default().to_string();
                    let length = lines
                        .filter_map(|l| l.split_once(':'))
                        .find(|(n, _)| n.trim().eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    while body.len() < length {
                        let n = stream.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        body.extend_from_slice(&chunk[..n]);
                    }
                    let (status, text) = cloud.lock().unwrap().handle(&method, &path, &body);
                    let mut response = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        text.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(&text);
                    let _ = stream.write_all(&response).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        base
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{serve, Cloud};
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    pub(crate) const NAME: &str = "test.hosted.cli";
    const SEED: [u8; 32] = [7; 32];
    const TASKS: &str = "table tasks(strict: true) { id: Sqlite.Text title: Sqlite.Text primaryKey(columns: [.id]) }";
    const NOTES: &str = "table notes(strict: true) { id: Sqlite.Text primaryKey(columns: [.id]) }";
    const LABELS: &str = "table labels(strict: true) { id: Sqlite.Text primaryKey(columns: [.id]) }";
    const DEPS: &str = "type = \"database\"\ntargets = [\"server\"]\n";

    /// Every signing call in this process uses one fixed test key.
    pub(crate) fn signing_key() -> String {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            std::env::set_var(
                "AXIOM_DATABASE_SIGNING_KEY",
                SEED.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            )
        });
        db::local_public_key(NAME).unwrap()
    }
    fn schema(version: &str, tables: &str) -> String {
        format!("profile database 1\nproject(id: \"{NAME}\", version: \"{version}\")\nmodule {NAME}\ndatabase tasks(engine: DatabaseEngine.sqlite) {{ manage(schemas: [Schemas.main], mode: ManagementMode.ownedObjects) schema main {{ {tables} }} }}")
    }
    fn project(version: &str, tables: &str) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("AxiomDeps.toml"), DEPS).unwrap();
        std::fs::write(root.path().join("schema.acore"), schema(version, tables)).unwrap();
        root
    }
    fn release(version: &str, tables: &str, parent: Option<&Release>) -> Release {
        let input = acore::database::ProjectInput {
            entry: "schema.acore".into(),
            files: [
                ("AxiomDeps.toml".to_string(), DEPS.to_string()),
                ("schema.acore".to_string(), schema(version, tables)),
            ]
            .into(),
            packages: Default::default(),
        };
        let compiled = acore::database::compile(&input).unwrap();
        db::sign_with_key(db::revision(&compiled, input, parent, "migration", None).unwrap(), SEED).unwrap()
    }
    fn bytes(release: &Release) -> Vec<u8> {
        axiom_database::canonical_bytes(release).unwrap()
    }
    async fn cloud(enrolled: bool) -> (Arc<Mutex<Cloud>>, CloudClient) {
        let key = signing_key();
        let mut state = Cloud {
            stream: NAME.into(),
            ..Default::default()
        };
        if enrolled {
            state.signers.push(serde_json::json!({"public_key": key, "label": "test", "added_at": "2026-10-20T00:00:00Z", "revoked_at": null}));
        }
        let state = Arc::new(Mutex::new(state));
        let base = serve(state.clone()).await;
        let client = CloudClient::for_loopback(&base, "access".into(), "refresh".into()).unwrap();
        (state, client)
    }
    fn hosted(client: &CloudClient) -> Hosted<'_> {
        Hosted {
            client,
            project: "acme",
            environment: "development",
            wait: WaitPolicy {
                initial: Duration::from_millis(5),
                max_interval: Duration::from_millis(10),
                timeout: Duration::from_secs(20),
            },
        }
    }
    fn migrate_args(root: &Path, dry_run: bool) -> MigrateArgs {
        MigrateArgs {
            source: root.join("schema.acore"),
            connection: Default::default(),
            remote: RemoteArgs {
                project: Some("acme".into()),
                environment: Some("development".into()),
            },
            policy: dev::Policy {
                non_interactive: true,
                dry_run,
                ..Default::default()
            },
            frozen: false,
            to: None,
            impact: Default::default(),
        }
    }
    fn pull_args(root: &Path) -> PullArgs {
        PullArgs {
            source: root.join("schema.acore"),
            project: "acme".into(),
            environment: "development".into(),
            json: false,
        }
    }
    fn text(out: Vec<u8>) -> String {
        String::from_utf8(out).unwrap()
    }
    fn catalog(root: &Path, version: &str) -> PathBuf {
        root.join(".axiom/database/migrations")
            .join(version)
            .join("release.axiom")
    }

    #[tokio::test]
    async fn migrate_uploads_the_signed_release_waits_and_prints_the_new_head() {
        let (state, client) = cloud(true).await;
        let h = hosted(&client);
        let root = project("1.0.0", TASKS);
        let mut out = vec![];
        migrate_with(&h, &migrate_args(root.path(), false), &mut out).await.unwrap();
        let out = text(out);
        let archived = std::fs::read(catalog(root.path(), "1.0.0")).unwrap();
        let first: Release = serde_json::from_slice(&archived).unwrap();
        {
            let state = state.lock().unwrap();
            // status run, upload of the exact catalog bytes, migrate run.
            assert_eq!(state.bodies("POST", "/api/v1/projects/acme/migrations/releases"), vec![archived.clone()]);
            let ops = state.bodies("POST", "/api/v1/projects/acme/migrations/operations");
            let ops = ops.iter().map(|b| serde_json::from_slice::<serde_json::Value>(b).unwrap()).collect::<Vec<_>>();
            assert_eq!(ops[0], serde_json::json!({"environment": "development", "mode": "status"}));
            assert_eq!(ops[1], serde_json::json!({"environment": "development", "mode": "migrate", "release": first.sha256}));
            assert!(state.count("GET", "/api/v1/projects/acme/migrations/operations/00000000-0000-4000-8000-000000000002") >= 2);
            assert_eq!(state.head.as_ref().unwrap().0, first.sha256);
            // The signed impact sidecar written next to the release is attached to it.
            let sidecar = std::fs::read(root.path().join(".axiom/database/migrations/1.0.0/impact.json")).unwrap();
            assert_eq!(state.bodies("PUT", &format!("/api/v1/projects/acme/migrations/releases/{}/impact", first.sha256)), vec![sidecar]);
        }
        assert!(out.contains(&format!("Applied migration 1.0.0 · hosted head {} (generation 1)", first.sha256)), "{out}");

        // A second run finds the hosted head current and uploads nothing.
        let mut out = vec![];
        migrate_with(&h, &migrate_args(root.path(), false), &mut out).await.unwrap();
        assert!(text(out).contains("Database is current · contract 1.0.0"));
        assert_eq!(state.lock().unwrap().count("POST", "/api/v1/projects/acme/migrations/releases"), 1);

        // The next version is parented on the hosted head.
        std::fs::write(root.path().join("schema.acore"), schema("1.1.0", &format!("{TASKS} {NOTES}"))).unwrap();
        let mut out = vec![];
        migrate_with(&h, &migrate_args(root.path(), false), &mut out).await.unwrap();
        let second = db::read_release(&catalog(root.path(), "1.1.0")).unwrap();
        assert_eq!(second.revision.parent.as_deref(), Some(first.sha256.as_str()));
        assert!(text(out).contains(&format!("Applied migration 1.1.0 · hosted head {} (generation 2)", second.sha256)));
    }

    #[tokio::test]
    async fn dry_run_uploads_then_checks_and_writes_nothing_locally() {
        let (state, client) = cloud(true).await;
        let root = project("1.0.0", TASKS);
        let mut out = vec![];
        migrate_with(&hosted(&client), &migrate_args(root.path(), true), &mut out).await.unwrap();
        assert!(text(out).contains("Hosted check passed for migration 1.0.0; nothing was changed"));
        let state = state.lock().unwrap();
        assert_eq!(state.releases.len(), 1);
        let modes = state.operations.iter().map(|o| o["mode"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(modes, ["status", "check"]);
        assert!(state.head.is_none());
        assert!(!catalog(root.path(), "1.0.0").exists());
    }

    #[tokio::test]
    async fn a_rejected_operation_fails_with_only_a_known_code() {
        let (state, client) = cloud(true).await;
        let root = project("1.0.0", TASKS);
        state.lock().unwrap().reject = Some("ADB1105".into());
        let error = migrate_with(&hosted(&client), &migrate_args(root.path(), false), &mut Vec::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("ADB1105:") && error.contains("rejected"), "{error}");
        assert!(state.lock().unwrap().head.is_none());

        state.lock().unwrap().reject = Some("SYNTHETIC_PRIVATE_CANARY".into());
        let error = migrate_with(&hosted(&client), &migrate_args(root.path(), false), &mut Vec::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("without a known code") && !error.contains("SYNTHETIC_PRIVATE_CANARY"), "{error}");
    }

    #[tokio::test]
    async fn an_unenrolled_key_fails_before_upload() {
        let (state, client) = cloud(false).await;
        let root = project("1.0.0", TASKS);
        let error = migrate_with(&hosted(&client), &migrate_args(root.path(), false), &mut Vec::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("ADB1109:"), "{error}");
        assert_eq!(state.lock().unwrap().count("POST", "/api/v1/projects/acme/migrations/releases"), 0);
    }

    async fn pull_cloud(chain: &[&Release], enrolled: bool) -> (Arc<Mutex<Cloud>>, CloudClient) {
        let (state, client) = cloud(enrolled).await;
        {
            let mut s = state.lock().unwrap();
            for release in chain {
                s.releases.push((release.sha256.clone(), bytes(release)));
            }
            let head = chain.last().unwrap();
            s.head = Some((head.sha256.clone(), chain.len() as u64, head.public_key.clone()));
        }
        (state, client)
    }

    #[tokio::test]
    async fn pull_writes_a_verified_chain_and_executes_nothing() {
        let r1 = release("1.0.0", TASKS, None);
        let r2 = release("1.1.0", &format!("{TASKS} {NOTES}"), Some(&r1));
        let (state, client) = pull_cloud(&[&r1, &r2], true).await;
        let root = project("1.1.0", &format!("{TASKS} {NOTES}"));
        let mut out = vec![];
        pull_with(&hosted(&client), &pull_args(root.path()), &mut out).await.unwrap();
        let out = text(out);
        assert!(out.contains("added 1.0.0") && out.contains("added 1.1.0") && out.contains("Nothing was executed."), "{out}");
        assert_eq!(db::read_release(&catalog(root.path(), "1.0.0")).unwrap().sha256, r1.sha256);
        assert_eq!(db::read_release(&catalog(root.path(), "1.1.0")).unwrap().sha256, r2.sha256);
        let modes = state.lock().unwrap().operations.iter().map(|o| o["mode"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(modes, ["status"]);
    }

    #[tokio::test]
    async fn pull_refuses_a_tampered_signature_and_writes_nothing() {
        let r1 = release("1.0.0", TASKS, None);
        let mut r2 = release("1.1.0", &format!("{TASKS} {NOTES}"), Some(&r1));
        let last = if r2.signature.ends_with('0') { "1" } else { "0" };
        r2.signature.replace_range(127.., last);
        let (_, client) = pull_cloud(&[&r1, &r2], true).await;
        let root = project("1.1.0", &format!("{TASKS} {NOTES}"));
        let error = pull_with(&hosted(&client), &pull_args(root.path()), &mut Vec::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("ADB1101:") && error.contains("signature"), "{error}");
        assert!(!root.path().join(".axiom/database/migrations").exists());
    }

    #[tokio::test]
    async fn pull_refuses_a_broken_parent_link() {
        let r1 = release("1.0.0", TASKS, None);
        let other = release("1.0.0", LABELS, None);
        let good = release("1.1.0", &format!("{TASKS} {NOTES}"), Some(&r1));
        // Correctly signed, but its parent is not the state it was built on.
        let mut revision = good.revision.clone();
        revision.parent = Some(other.sha256.clone());
        let broken = db::sign_with_key(revision, SEED).unwrap();
        let (_, client) = pull_cloud(&[&other, &broken], true).await;
        let root = project("1.1.0", &format!("{TASKS} {NOTES}"));
        let error = pull_with(&hosted(&client), &pull_args(root.path()), &mut Vec::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not follow its parent"), "{error}");
        assert!(!root.path().join(".axiom/database/migrations").exists());
    }

    #[tokio::test]
    async fn pull_refuses_history_that_diverges_from_local_releases() {
        let r1 = release("1.0.0", TASKS, None);
        let r2 = release("1.1.0", &format!("{TASKS} {NOTES}"), Some(&r1));
        let local = release("1.1.0", &format!("{TASKS} {LABELS}"), Some(&r1));
        let (_, client) = pull_cloud(&[&r1, &r2], true).await;
        let root = project("1.1.0", &format!("{TASKS} {NOTES}"));
        db::archive(root.path(), &local).unwrap();
        let before = std::fs::read(catalog(root.path(), "1.1.0")).unwrap();
        let error = pull_with(&hosted(&client), &pull_args(root.path()), &mut Vec::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("ADB1103:") && error.contains("1.1.0"), "{error}");
        assert_eq!(std::fs::read(catalog(root.path(), "1.1.0")).unwrap(), before);
        assert!(!catalog(root.path(), "1.0.0").exists());

        // A local branch off an older hosted release diverges too, even with
        // a version the hosted history never used.
        let r3 = release("1.2.0", &format!("{TASKS} {NOTES} {LABELS}"), Some(&r2));
        let branch = release("1.1.5", &format!("{TASKS} {LABELS}"), Some(&r1));
        let (_, client) = pull_cloud(&[&r1, &r2, &r3], true).await;
        let root = project("1.2.0", &format!("{TASKS} {LABELS}"));
        db::archive(root.path(), &r1).unwrap();
        db::archive(root.path(), &branch).unwrap();
        let error = pull_with(&hosted(&client), &pull_args(root.path()), &mut Vec::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("ADB1103:") && error.contains("1.1.5"), "{error}");
        assert!(!catalog(root.path(), "1.1.0").exists());
    }

    #[tokio::test]
    async fn pull_keeps_an_existing_local_release_unchanged() {
        let r1 = release("1.0.0", TASKS, None);
        let r2 = release("1.1.0", &format!("{TASKS} {NOTES}"), Some(&r1));
        let (state, client) = pull_cloud(&[&r1, &r2], true).await;
        let root = project("1.1.0", &format!("{TASKS} {NOTES}"));
        let path = db::archive(root.path(), &r1).unwrap();
        let before = (std::fs::read(&path).unwrap(), std::fs::metadata(&path).unwrap().modified().unwrap());
        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut out = vec![];
        pull_with(&hosted(&client), &pull_args(root.path()), &mut out).await.unwrap();
        let out = text(out);
        assert!(out.contains("kept  1.0.0") && out.contains("added 1.1.0"), "{out}");
        let after = (std::fs::read(&path).unwrap(), std::fs::metadata(&path).unwrap().modified().unwrap());
        assert_eq!(before, after);
        // The local copy was used; only the missing release was downloaded.
        let state = state.lock().unwrap();
        assert_eq!(state.count("GET", &format!("/api/v1/projects/acme/migrations/releases/{}", r1.sha256)), 0);
        assert_eq!(state.count("GET", &format!("/api/v1/projects/acme/migrations/releases/{}", r2.sha256)), 1);
    }

    // --- dashboard captures (data-tools/v1, "CLI: pull, migrate and version skew") ---------
    const FILL: &str = "evolution fill(from: Previous, to: Current, coordination: EvolutionCoordination.maintenanceFenced) { backfill(table: Current.Tables.main.tasks, key: Previous.Columns.main.tasks.id, column: Current.Columns.main.tasks.title, using: value(literal: \"filled\"), batchSize: 10, batchesPerRun: 1, maxRows: 10, catchUpRows: 10) }";

    /// A capture on `parent` as the dashboard builds it (preview, then build with the
    /// receipt's block), signed by the test key.
    fn captured(parent: &Release, sql: &str) -> Release {
        use axiom_database::capture;
        let preview = capture::build(parent, &[], None, sql, None).unwrap();
        let block = db::capture::CaptureBlock {
            format: "axiom-dashboard-capture/v1".into(),
            project_id: "5d2c1b0a-9e8f-4a7b-8c6d-5e4f3a2b1c0d".into(),
            environment_id: "c0ffee00-0000-4000-8000-000000000001".into(),
            receipt_id: "c0ffee00-0000-4000-8000-000000000002".into(),
            parent_sha256: parent.sha256.clone(),
            sql_sha256: "0".repeat(64),
            statements_sha256: preview.statements_sha256.clone(),
            captured_by: "owner@example.com".into(),
            captured_at: "2026-10-28T10:00:00Z".into(),
            title: "Add notes".into(),
        };
        let built = capture::build(parent, &[], Some(&preview.version), sql, Some(block)).unwrap();
        db::sign_with_key(built.revision, SEED).unwrap()
    }
    /// A working tree with exactly the sources of `release`.
    fn tree(release: &Release) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (path, text) in &release.revision.sources {
            let target = root.path().join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, text).unwrap();
        }
        root
    }
    fn with_intent(parent: &Release, version: &str) -> Release {
        let mut files = parent.revision.sources.clone();
        let entry = files.get_mut("schema.acore").unwrap();
        *entry = entry.replace(&format!("version: \"{}\"", parent.revision.identity.version), &format!("version: \"{version}\""));
        entry.push_str("\nmigration(source: \"migration.acore\")\n");
        files.insert("migration.acore".into(), FILL.into());
        let input = acore::database::ProjectInput { entry: "schema.acore".into(), files, packages: Default::default() };
        let compiled = acore::database::compile(&input).unwrap();
        let release = db::sign_with_key(db::revision(&compiled, input, Some(parent), "migration", None).unwrap(), SEED).unwrap();
        assert!(release.revision.evolution.is_some());
        release
    }
    /// The developer moves `table` out of the dashboard region (above the begin marker).
    fn move_out(root: &Path, table: &str) {
        let path = root.join("schema.acore");
        let mut text = std::fs::read_to_string(&path).unwrap();
        let start = text.find(&format!("  table {table}")).unwrap();
        let end = start + text[start..].find("\n  }\n").unwrap() + "\n  }\n".len();
        let block = text[start..end].to_string();
        text.replace_range(start..end, "");
        let begin = text.find(&format!("  {}", axiom_database::capture::region::BEGIN)).unwrap();
        text.insert_str(begin, &block);
        std::fs::write(path, text).unwrap();
    }

    #[tokio::test]
    async fn pull_writes_the_capture_region_and_mechanical_edits_then_migrate_is_current() {
        let r1 = release("1.0.0", TASKS, None);
        let r2 = with_intent(&r1, "1.1.0");
        let c3 = captured(&r2, "CREATE TABLE notes (id TEXT PRIMARY KEY, body TEXT)");
        assert!(!c3.revision.sources.contains_key("migration.acore"));
        let (state, client) = pull_cloud(&[&r1, &r2, &c3], true).await;
        // The developer's tree is at 1.1.0 with the typed intent, catalog r1 and r2.
        let root = tree(&r2);
        db::archive(root.path(), &r1).unwrap();
        db::archive(root.path(), &r2).unwrap();
        let mut out = vec![];
        pull_with(&hosted(&client), &pull_args(root.path()), &mut out).await.unwrap();
        let out = text(out);
        assert!(out.contains("captured in dashboard: Add notes by owner@example.com") && out.contains("dashboard region of schema main updated") && out.contains("Nothing was executed."), "{out}");
        let schema = std::fs::read_to_string(root.path().join("schema.acore")).unwrap();
        assert_eq!(schema, c3.revision.sources["schema.acore"], "local equals catalog: the incoming region and edits");
        assert!(!root.path().join("migration.acore").exists(), "no stale typed intent stays");
        assert_eq!(std::fs::read_to_string(root.path().join(".axiom/database/pulled/1.1.1/schema.acore")).unwrap(), r2.revision.sources["schema.acore"]);
        assert_eq!(std::fs::read_to_string(root.path().join(".axiom/database/pulled/1.1.1/migration.acore")).unwrap(), FILL);
        assert_eq!(db::read_release(&catalog(root.path(), "1.1.1")).unwrap().sha256, c3.sha256);
        let modes = state.lock().unwrap().operations.iter().map(|o| o["mode"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(modes, ["status"], "pull executes nothing");
        // A following local migrate keeps the dashboard table: the hosted head is current.
        let mut out = vec![];
        migrate_with(&hosted(&client), &migrate_args(root.path(), false), &mut out).await.unwrap();
        let out = text(out);
        assert!(out.contains("Database is current") && out.contains(&c3.sha256), "{out}");
    }

    #[tokio::test]
    async fn pull_keeps_a_local_region_edit_when_the_capture_is_already_pulled() {
        let r1 = release("1.0.0", TASKS, None);
        let c2 = captured(&r1, "CREATE TABLE notes (id TEXT PRIMARY KEY)");
        let (_, client) = pull_cloud(&[&r1, &c2], true).await;
        let root = tree(&c2);
        db::archive(root.path(), &r1).unwrap();
        db::archive(root.path(), &c2).unwrap();
        move_out(root.path(), "notes");
        let before = std::fs::read_to_string(root.path().join("schema.acore")).unwrap();
        let mut out = vec![];
        pull_with(&hosted(&client), &pull_args(root.path()), &mut out).await.unwrap();
        // The capture is already in the local catalog, so pull applies nothing (review H-3).
        let out = text(out);
        assert!(!out.contains("dashboard region") && out.contains("kept  1.0.1"), "{out}");
        assert_eq!(std::fs::read_to_string(root.path().join("schema.acore")).unwrap(), before, "the local move stands");
        assert!(!root.path().join(".axiom/database/pulled").exists());
    }

    #[tokio::test]
    async fn pull_refuses_when_local_and_incoming_regions_both_changed() {
        let r1 = release("1.0.0", TASKS, None);
        let c2 = captured(&r1, "CREATE TABLE notes (id TEXT PRIMARY KEY)");
        let c3 = captured(&c2, "CREATE TABLE labels (id TEXT PRIMARY KEY)");
        let (_, client) = pull_cloud(&[&r1, &c2, &c3], true).await;
        let root = tree(&c2);
        db::archive(root.path(), &r1).unwrap();
        db::archive(root.path(), &c2).unwrap();
        move_out(root.path(), "notes");
        let before = std::fs::read_to_string(root.path().join("schema.acore")).unwrap();
        let error = pull_with(&hosted(&client), &pull_args(root.path()), &mut Vec::new()).await.unwrap_err().to_string();
        assert!(error.starts_with("ADB1104:") && error.contains("catalog to local") && error.contains("catalog to incoming") && error.contains("+   table labels"), "{error}");
        assert_eq!(std::fs::read_to_string(root.path().join("schema.acore")).unwrap(), before);
        assert!(!catalog(root.path(), &c3.revision.identity.version).exists(), "nothing was written");
    }

    #[tokio::test]
    async fn pull_applies_a_capture_once_and_keeps_later_local_work() {
        // (a) After the capture is pulled, a new migration declaration of the developer's
        // survives every later pull (review H-3).
        let r1 = release("1.0.0", TASKS, None);
        let r2 = with_intent(&r1, "1.1.0");
        let c3 = captured(&r2, "CREATE TABLE notes (id TEXT PRIMARY KEY)");
        let (_, client) = pull_cloud(&[&r1, &r2, &c3], true).await;
        let root = tree(&r2);
        db::archive(root.path(), &r1).unwrap();
        db::archive(root.path(), &r2).unwrap();
        pull_with(&hosted(&client), &pull_args(root.path()), &mut Vec::new()).await.unwrap();
        let applied: serde_json::Value = serde_json::from_slice(&std::fs::read(root.path().join(".axiom/database/pulled/1.1.1/applied.json")).unwrap()).unwrap();
        assert_eq!(applied["release"], c3.sha256.as_str(), "what was applied is recorded");
        let schema = root.path().join("schema.acore");
        let mut schema_text = std::fs::read_to_string(&schema).unwrap();
        schema_text.push_str("migration(source: \"migration.acore\")\n");
        std::fs::write(&schema, &schema_text).unwrap();
        std::fs::write(root.path().join("migration.acore"), FILL).unwrap();
        let mut out = vec![];
        pull_with(&hosted(&client), &pull_args(root.path()), &mut out).await.unwrap();
        assert_eq!(std::fs::read_to_string(&schema).unwrap(), schema_text, "{}", String::from_utf8_lossy(&out));
        assert!(root.path().join("migration.acore").exists());
        assert!(!root.path().join(".axiom/database/pulled/1.1.1.2").exists());

        // (b) A CLI release after the capture that moved the table out: pull never writes
        // the capture's older region back.
        let r1 = release("1.0.0", TASKS, None);
        let c2 = captured(&r1, "CREATE TABLE notes (id TEXT PRIMARY KEY)");
        let moved = tree(&c2);
        move_out(moved.path(), "notes");
        let mut files = c2.revision.sources.clone();
        files.insert("schema.acore".into(), std::fs::read_to_string(moved.path().join("schema.acore")).unwrap().replace("version: \"1.0.1\"", "version: \"1.1.0\""));
        let input = acore::database::ProjectInput { entry: "schema.acore".into(), files, packages: Default::default() };
        let compiled = acore::database::compile(&input).unwrap();
        let r3 = db::sign_with_key(db::revision(&compiled, input, Some(&c2), "migration", None).unwrap(), SEED).unwrap();
        let (_, client) = pull_cloud(&[&r1, &c2, &r3], true).await;
        let root = tree(&r3);
        for r in [&r1, &c2, &r3] {
            db::archive(root.path(), r).unwrap();
        }
        let before = std::fs::read_to_string(root.path().join("schema.acore")).unwrap();
        pull_with(&hosted(&client), &pull_args(root.path()), &mut Vec::new()).await.unwrap();
        assert_eq!(std::fs::read_to_string(root.path().join("schema.acore")).unwrap(), before);
        // A following local migrate keeps the moved table: the head is current.
        let mut out = vec![];
        migrate_with(&hosted(&client), &migrate_args(root.path(), false), &mut out).await.unwrap();
        assert!(text(out).contains("Database is current"));

        // Code releases after an unpulled capture, not local: refused, nothing written.
        let root = tree(&r1);
        db::archive(root.path(), &r1).unwrap();
        let error = pull_with(&hosted(&client), &pull_args(root.path()), &mut Vec::new()).await.unwrap_err().to_string();
        assert!(error.starts_with("ADB1104:") && error.contains("1.1.0"), "{error}");
        assert!(!catalog(root.path(), "1.0.1").exists());
    }

    #[tokio::test]
    async fn pull_fails_closed_when_the_capture_region_cannot_be_written() {
        // (review H-4) A tree that does not compile: nothing is archived.
        let r1 = release("1.0.0", TASKS, None);
        let c2 = captured(&r1, "CREATE TABLE notes (id TEXT PRIMARY KEY)");
        let (_, client) = pull_cloud(&[&r1, &c2], true).await;
        let root = tree(&r1);
        db::archive(root.path(), &r1).unwrap();
        std::fs::write(root.path().join("schema.acore"), "this does not compile").unwrap();
        let error = pull_with(&hosted(&client), &pull_args(root.path()), &mut Vec::new()).await.unwrap_err().to_string();
        assert!(error.starts_with("ADB1104:"), "{error}");
        assert!(!catalog(root.path(), "1.0.1").exists());
        // A schema file that is a symlink to a file outside the project: refused before
        // anything is archived, and the outside file is untouched.
        let root = tree(&r1);
        db::archive(root.path(), &r1).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::rename(root.path().join("schema.acore"), outside.path().join("schema.acore")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("schema.acore"), root.path().join("schema.acore")).unwrap();
        let error = pull_with(&hosted(&client), &pull_args(root.path()), &mut Vec::new()).await.unwrap_err().to_string();
        assert!(error.starts_with("ADB1101:") || error.starts_with("ADB1104:"), "{error}");
        assert!(!catalog(root.path(), "1.0.1").exists());
        assert_eq!(std::fs::read_to_string(outside.path().join("schema.acore")).unwrap(), r1.revision.sources["schema.acore"]);
    }

    #[tokio::test]
    async fn migrate_refuses_an_unpulled_capture_head() {
        let r1 = release("1.0.0", TASKS, None);
        let c2 = captured(&r1, "CREATE TABLE notes (id TEXT PRIMARY KEY)");
        let (state, client) = pull_cloud(&[&r1, &c2], true).await;
        let root = project("1.1.0", &format!("{TASKS} {LABELS}"));
        db::archive(root.path(), &r1).unwrap();
        let error = migrate_with(&hosted(&client), &migrate_args(root.path(), false), &mut Vec::new()).await.unwrap_err().to_string();
        assert!(error.starts_with("ADB1103:") && error.contains("axiom database pull"), "{error}");
        assert_eq!(state.lock().unwrap().count("POST", "/api/v1/projects/acme/migrations/releases"), 0);
    }

    #[test]
    fn pull_writes_only_existing_project_files_under_the_source_key_rule() {
        use super::super::database_capture::{stage, Plan};
        let write = |root: &Path, version: &str, plan: &Plan, _: Option<&str>| -> anyhow::Result<PathBuf> {
            stage(root, &release(version, TASKS, None), plan)?.apply()
        };
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("schema.acore"), "x").unwrap();
        for path in ["../x.acore", "/abs.acore", "missing.acore", "notes.txt"] {
            let plan = Plan { files: [(path.to_string(), Some("y".to_string()))].into(), ..Default::default() };
            assert!(write(root.path(), "1.0.1", &plan, None).unwrap_err().to_string().starts_with("ADB1101"), "{path}");
        }
        std::os::unix::fs::symlink(root.path().join("schema.acore"), root.path().join("link.acore")).unwrap();
        let plan = Plan { files: [("link.acore".to_string(), Some("y".to_string()))].into(), ..Default::default() };
        assert!(write(root.path(), "1.0.1", &plan, None).is_err(), "symlinks are never written");
        // A symlinked directory on the path (lstat on every component), inside or outside.
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("x.acore"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("sub")).unwrap();
        let plan = Plan { files: [("sub/x.acore".to_string(), Some("y".to_string()))].into(), ..Default::default() };
        assert!(write(root.path(), "1.0.1", &plan, None).unwrap_err().to_string().starts_with("ADB1101"));
        assert_eq!(std::fs::read_to_string(outside.path().join("x.acore")).unwrap(), "x");
        std::fs::create_dir(root.path().join("dir")).unwrap();
        std::fs::write(root.path().join("dir/y.acore"), "y").unwrap();
        std::os::unix::fs::symlink(root.path().join("dir"), root.path().join("alias")).unwrap();
        let plan = Plan { files: [("alias/y.acore".to_string(), Some("z".to_string()))].into(), ..Default::default() };
        assert!(write(root.path(), "1.0.1", &plan, None).unwrap_err().to_string().starts_with("ADB1101"));
        let plan = Plan { files: [("schema.acore".to_string(), Some("y".repeat(8 * 1024 * 1024 + 1)))].into(), ..Default::default() };
        assert!(write(root.path(), "1.0.1", &plan, None).is_err(), "8 MiB bound");
        assert_eq!(std::fs::read_to_string(root.path().join("schema.acore")).unwrap(), "x");
        let plan = Plan { files: [("schema.acore".to_string(), Some("y".to_string()))].into(), ..Default::default() };
        write(root.path(), "1.0.1", &plan, None).unwrap();
        assert_eq!(std::fs::read_to_string(root.path().join("schema.acore")).unwrap(), "y");
        assert_eq!(std::fs::read_to_string(root.path().join(".axiom/database/pulled/1.0.1/schema.acore")).unwrap(), "x");
    }

    #[tokio::test]
    async fn pull_refuses_releases_signed_outside_the_registry() {
        let r1 = release("1.0.0", TASKS, None);
        let (_, client) = pull_cloud(&[&r1], false).await;
        let root = project("1.0.0", TASKS);
        let error = pull_with(&hosted(&client), &pull_args(root.path()), &mut Vec::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("ADB1109:"), "{error}");
        assert!(!catalog(root.path(), "1.0.0").exists());
    }

    #[tokio::test]
    async fn status_history_and_diff_compare_the_hosted_head_with_local_state() {
        let r1 = release("1.0.0", TASKS, None);
        let (state, client) = pull_cloud(&[&r1], true).await;
        state.lock().unwrap().operations.push(serde_json::json!({"id": "00000000-0000-4000-8000-999999999999", "environment": "development", "mode": "migrate", "release": r1.sha256, "status": "succeeded", "outcome": "applied", "code": null, "targets": [{"project": NAME, "head": r1.sha256, "generation": 1, "public_key": r1.public_key}], "created_at": "2026-10-19T00:00:00Z", "finished_at": "2026-10-19T00:00:01Z"}));
        let root = project("1.1.0", &format!("{TASKS} {NOTES}"));
        let h = hosted(&client);
        let read = ReadArgs {
            source: root.path().join("schema.acore"),
            connection: Default::default(),
            remote: RemoteArgs { project: Some("acme".into()), environment: Some("development".into()) },
            json: false,
            watch: false,
            check: true,
        };
        let mut out = vec![];
        let error = status_with(&h, &read, &mut out).await.unwrap_err().to_string();
        assert!(error.starts_with("ADB1105"), "{error}");
        let out = text(out);
        assert!(out.contains("Applied: 1.0.0 · Working: 1.1.0 · Drift: none"), "{out}");
        assert!(out.contains("Not in the local catalog: 1.0.0"), "{out}");

        let mut out = vec![];
        history_with(&h, &root.path().join("schema.acore"), false, &mut out).await.unwrap();
        let out = text(out);
        assert!(out.contains(&format!("1.0.0  migration  {}  not in local catalog", &r1.sha256[..12])), "{out}");
        assert!(out.contains("migrate  succeeded  applied  1.0.0"), "{out}");

        let mut out = vec![];
        diff_with(&h, &root.path().join("schema.acore"), true, &mut out).await.unwrap();
        let preview: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert!(preview["changes"].as_array().unwrap().iter().any(|c| c["path"].as_str().unwrap_or_default().contains("notes")), "{preview}");
        // Reads write nothing locally.
        assert!(!root.path().join(".axiom/database/migrations").exists());
    }

    #[tokio::test]
    async fn signer_enrollment_signs_the_contract_proof_with_the_local_key() {
        let (state, client) = cloud(false).await;
        let root = project("1.0.0", TASKS);
        let action = crate::commands::signers::Action::Add {
            project: "acme".into(),
            label: None,
            schema: Some(root.path().join("schema.acore")),
        };
        let key = crate::commands::signers::prepare(&action).unwrap();
        let mut out = vec![];
        crate::commands::signers::run(&client, &action, key, &mut out).await.unwrap();
        let out = text(out);
        let public_key = signing_key();
        assert!(out.contains(&format!("Enrolled signing key {public_key} as \"axiom-cli")), "{out}");
        let seed = SEED.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert!(!out.contains(&seed));
        let state = state.lock().unwrap();
        // The fake verified the Ed25519 proof over the contract string.
        assert_eq!(state.signers.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&state.bodies("POST", "/api/v1/projects/acme/migrations/signers")[0]).unwrap();
        assert_eq!(body["public_key"], public_key);
        assert_eq!(body["proof"].as_str().unwrap().len(), 128);
        assert!(!String::from_utf8_lossy(&state.requests.iter().flat_map(|r| r.2.clone()).collect::<Vec<_>>()).contains(&seed));
    }

    #[tokio::test]
    async fn invalid_targets_fail_before_authentication() {
        let root = project("1.0.0", TASKS);
        let mut args = migrate_args(root.path(), false);
        args.remote.environment = Some("Not A Slug".into());
        let error = migrate(&args).await.unwrap_err().to_string();
        assert!(error.contains("invalid environment"), "{error}");
    }
}
