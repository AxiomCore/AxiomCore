use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use axiom_lib::authored_dependencies::{
    read_authored_dependency_lock, recover_locked_cache, resolve_authored_dependencies,
    retained_digests, verify_locked_cache, AuthoredDependencyLock, ContentAddressedStore,
    HttpRegistryTransport, ResolveMode,
};
use axiom_lib::deps_manifest::{
    axiom_deps_completions, axiom_deps_hover, axiom_deps_schema, canonical_axiom_deps_v2_bytes,
    diagnose_axiom_deps, format_axiom_deps_v2, migrate_axiom_deps_to_v2, parse_axiom_deps_v2,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub async fn handle_check(deps: PathBuf, json: bool) -> Result<()> {
    let source =
        std::fs::read_to_string(&deps).with_context(|| format!("read {}", deps.display()))?;
    let diagnostics = diagnose_axiom_deps(&source);
    if !diagnostics.is_empty() {
        if json {
            println!("{}", serde_json::to_string_pretty(&diagnostics)?);
        }
        bail!("{}", diagnostics[0].message);
    }
    let manifest = parse_axiom_deps_v2(&source)?;
    let canonical = canonical_axiom_deps_v2_bytes(&source)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "valid",
                "path": deps,
                "format": manifest.format,
                "canonicalSha256": digest_hex(&canonical),
                "contracts": manifest.contracts.len(),
                "packages": manifest.packages.len(),
                "extensions": manifest.extensions.len(),
                "environments": manifest.dependency_environments.len(),
                "registries": manifest.dependency_registries.len(),
            }))?
        );
    } else {
        println!("Valid AxiomDeps.toml v2: {}", deps.display());
        println!("Canonical manifest: {}", digest_hex(&canonical));
        println!(
            "{} contract(s), {} Axiom package(s), {} extension(s), {} build environment(s).",
            manifest.contracts.len(),
            manifest.packages.len(),
            manifest.extensions.len(),
            manifest.dependency_environments.len(),
        );
    }
    Ok(())
}

pub async fn handle_format(deps: PathBuf, check: bool, write: bool) -> Result<()> {
    let source =
        std::fs::read_to_string(&deps).with_context(|| format!("read {}", deps.display()))?;
    let formatted = format_axiom_deps_v2(&source)?;
    if check {
        if source != formatted {
            bail!(
                "{} is not in canonical AxiomDeps.toml v2 format",
                deps.display()
            );
        }
        println!("{} is canonically formatted.", deps.display());
    } else if write {
        atomic_write(&deps, formatted.as_bytes())?;
        println!("Formatted {}.", deps.display());
    } else {
        print!("{formatted}");
    }
    Ok(())
}

pub async fn handle_migrate(deps: PathBuf, out: Option<PathBuf>, write: bool) -> Result<()> {
    let source =
        std::fs::read_to_string(&deps).with_context(|| format!("read {}", deps.display()))?;
    let migrated = migrate_axiom_deps_to_v2(&source)?;
    match (write, out) {
        (true, None) => {
            atomic_write(&deps, migrated.as_bytes())?;
            println!("Migrated {} to axiom-deps/v2.", deps.display());
        }
        (false, Some(path)) => {
            atomic_write(&path, migrated.as_bytes())?;
            println!("Wrote migrated manifest to {}.", path.display());
        }
        (false, None) => print!("{migrated}"),
        (true, Some(_)) => unreachable!("clap rejects --write with --out"),
    }
    Ok(())
}

pub async fn handle_schema() -> Result<()> {
    print!("{}", axiom_deps_schema());
    Ok(())
}

pub async fn handle_completions(path: String, json: bool) -> Result<()> {
    let values = axiom_deps_completions(&path);
    if json {
        println!("{}", serde_json::to_string_pretty(&values)?);
    } else {
        for value in values {
            println!("{:<24} {}", value.label, value.detail);
        }
    }
    Ok(())
}

pub async fn handle_hover(path: String, json: bool) -> Result<()> {
    let explanation = axiom_deps_hover(&path)
        .with_context(|| format!("no AxiomDeps.toml documentation for `{path}`"))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "path": path,
                "documentation": explanation,
            }))?
        );
    } else {
        println!("{path}\n{explanation}");
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolutionSummary {
    lock: PathBuf,
    cache: PathBuf,
    workspace_view: PathBuf,
    manifest_sha256: String,
    domains: usize,
    packages: usize,
    unique_artifacts: usize,
    advisories: usize,
    package_scripts: usize,
    mode: &'static str,
}

pub async fn handle_resolve(
    deps: PathBuf,
    lock: PathBuf,
    cache: Option<PathBuf>,
    workspace_view: PathBuf,
    locked: bool,
    offline: bool,
    json: bool,
) -> Result<()> {
    let cache = dependency_cache_root(cache)?;
    // Registry transport is intentionally blocking: resolution verifies and
    // commits complete artifacts before exposing a lock. Keep that work off
    // Tokio's async worker so reqwest's blocking client cannot create/drop an
    // internal runtime from an asynchronous context.
    let resolve_deps = deps.clone();
    let resolve_lock = lock.clone();
    let resolve_cache = cache.clone();
    let resolve_workspace_view = workspace_view.clone();
    let resolved = tokio::task::spawn_blocking(move || {
        let store = ContentAddressedStore::new(&resolve_cache);
        resolve_authored_dependencies(
            &resolve_deps,
            &resolve_lock,
            &store,
            &resolve_workspace_view,
            ResolveMode { locked, offline },
            &HttpRegistryTransport,
        )
    })
    .await
    .context("dependency resolver worker failed")??;
    let mode = if locked {
        "locked"
    } else if offline {
        "offline"
    } else {
        "resolve"
    };
    print_summary(
        &resolved,
        ResolutionSummary {
            lock,
            cache,
            workspace_view,
            manifest_sha256: resolved.manifest_sha256.clone(),
            domains: resolved.domains.len(),
            packages: resolved.packages.len(),
            unique_artifacts: resolved
                .packages
                .values()
                .map(|package| &package.artifact_sha256)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            advisories: resolved
                .packages
                .values()
                .map(|package| package.advisories.len())
                .sum(),
            package_scripts: resolved
                .packages
                .values()
                .map(|package| package.scripts.len())
                .sum(),
            mode,
        },
        json,
    )
}

pub async fn handle_verify(lock: PathBuf, cache: Option<PathBuf>, json: bool) -> Result<()> {
    let cache = dependency_cache_root(cache)?;
    let store = ContentAddressedStore::new(&cache);
    let locked = read_authored_dependency_lock(&lock)?;
    verify_locked_cache(&locked, &store)?;
    let report = store.verify()?;
    if !report.corrupt.is_empty() || !report.malformed.is_empty() {
        bail!(
            "dependency cache verification failed: {} corrupt, {} malformed",
            report.corrupt.len(),
            report.malformed.len()
        );
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "verified",
                "lock": lock,
                "cache": cache,
                "lockedPackages": locked.packages.len(),
                "verifiedCacheObjects": report.verified,
            }))?
        );
    } else {
        println!(
            "Verified {} locked package(s) and {} content-addressed cache object(s).",
            locked.packages.len(),
            report.verified
        );
    }
    Ok(())
}

pub async fn handle_inspect(lock: PathBuf, json: bool) -> Result<()> {
    let locked = read_authored_dependency_lock(&lock)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&locked)?);
        return Ok(());
    }
    println!("Authored dependencies: {}", lock.display());
    println!("Manifest: {}", locked.manifest_sha256);
    println!("Environment domains: {}", locked.domains.len());
    for (identity, domain) in &locked.domains {
        println!(
            "  {}  {} {} / {}  extensions: {}",
            &identity[..12],
            domain.definition.language,
            domain.definition.runtime_version,
            domain.definition.target_family,
            domain.extensions.join(", ")
        );
    }
    println!("Packages: {}", locked.packages.len());
    for package in locked.packages.values() {
        println!(
            "  {}@{} [{}] <- {}",
            package.name,
            package.version,
            package.licenses.join(", "),
            package.introduced_by.join(", ")
        );
        if !package.advisories.is_empty() {
            println!("    advisories: {}", package.advisories.join(", "));
        }
        if !package.scripts.is_empty() {
            println!(
                "    package scripts (recorded, not executed): {}",
                package.scripts.join(", ")
            );
        }
        println!("    source: {}", package.artifact);
        println!("    sha256: {}", package.artifact_sha256);
    }
    Ok(())
}

pub async fn handle_cache_verify(cache: Option<PathBuf>, json: bool) -> Result<()> {
    let cache = dependency_cache_root(cache)?;
    let report = ContentAddressedStore::new(&cache).verify()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Dependency cache: {} verified, {} corrupt, {} malformed.",
            report.verified,
            report.corrupt.len(),
            report.malformed.len()
        );
        for digest in &report.corrupt {
            println!("  corrupt: {digest}");
        }
        for path in &report.malformed {
            println!("  malformed: {}", path.display());
        }
    }
    if !report.corrupt.is_empty() || !report.malformed.is_empty() {
        bail!("dependency cache integrity verification failed");
    }
    Ok(())
}

pub async fn handle_cache_prune(
    cache: Option<PathBuf>,
    locks: Vec<PathBuf>,
    json: bool,
) -> Result<()> {
    if locks.is_empty() {
        bail!("cache prune requires at least one explicit --lock; no implicit global deletion is allowed");
    }
    let cache = dependency_cache_root(cache)?;
    let locks: Vec<_> = locks
        .iter()
        .map(|path| read_authored_dependency_lock(path))
        .collect::<Result<_>>()?;
    let removed = ContentAddressedStore::new(&cache).prune(&retained_digests(&locks))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "cache": cache,
                "removed": removed,
            }))?
        );
    } else {
        println!(
            "Pruned {} unreferenced dependency cache object(s).",
            removed.len()
        );
    }
    Ok(())
}

pub async fn handle_cache_recover(lock: PathBuf, cache: Option<PathBuf>, json: bool) -> Result<()> {
    let cache = dependency_cache_root(cache)?;
    let locked = read_authored_dependency_lock(&lock)?;
    let recovered = recover_locked_cache(
        &locked,
        &ContentAddressedStore::new(&cache),
        &HttpRegistryTransport,
    )?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "lock": lock,
                "cache": cache,
                "recovered": recovered,
            }))?
        );
    } else {
        println!("Recovered {} dependency cache object(s).", recovered.len());
    }
    Ok(())
}

fn dependency_cache_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    explicit
        .or_else(|| dirs::cache_dir().map(|root| root.join("axiom/dependencies/v1")))
        .context("cannot determine the Axiom dependency cache directory; pass --cache explicitly")
}

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .context("dependency manifest output has no UTF-8 filename")?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn print_summary(
    lock: &AuthoredDependencyLock,
    summary: ResolutionSummary,
    json: bool,
) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!(
            "Resolved {} package(s) in {} environment domain(s); {} unique artifact(s).",
            summary.packages, summary.domains, summary.unique_artifacts
        );
        println!("Canonical lock: {}", summary.lock.display());
        println!("Immutable cache: {}", summary.cache.display());
        println!("Workspace view: {}", summary.workspace_view.display());
        if summary.advisories > 0 || summary.package_scripts > 0 {
            println!(
                "Recorded supply-chain metadata: {} advisory reference(s), {} package script(s).",
                summary.advisories, summary.package_scripts
            );
        }
        if summary.mode != "resolve" {
            println!(
                "Mode: {} (no registry resolution or fetching performed)",
                summary.mode
            );
        }
        debug_assert_eq!(summary.manifest_sha256, lock.manifest_sha256);
    }
    Ok(())
}

#[allow(dead_code)]
fn _assert_safe_explicit_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        bail!("path cannot be empty");
    }
    Ok(())
}
