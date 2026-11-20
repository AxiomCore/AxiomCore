use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use axiom_lib::authored_dependencies::{
    read_authored_dependency_lock, recover_locked_cache, resolve_authored_dependencies,
    retained_digests, verify_locked_cache, AuthoredDependencyLock, ContentAddressedStore,
    HttpRegistryTransport, ResolveMode,
};
use axiom_lib::deps_manifest::{
    axiom_deps_completions, axiom_deps_hover, axiom_deps_schema, canonical_axiom_deps_v2_bytes,
    diagnose_axiom_deps, migrate_axiom_deps_to_v2, parse_axiom_deps_v2,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// w1-p D9: why the canonical manifest digest differs from an earlier
/// manifest. The canonical form is what the author wrote, so omitting an
/// `exports` list that equals the derived one changes the digest; the review
/// names that instead of showing a bare digest change.
pub(crate) fn explain_canonical_change(before: &str, after: &str, after_path: &Path) -> Result<Vec<String>> {
    if canonical_axiom_deps_v2_bytes(before)? == canonical_axiom_deps_v2_bytes(after)? {
        return Ok(Vec::new());
    }
    fn exports(value: &toml::Value) -> BTreeMap<String, Vec<String>> {
        value
            .get("extensions")
            .and_then(toml::Value::as_table)
            .map(|extensions| {
                extensions
                    .iter()
                    .map(|(alias, table)| {
                        let mut list = table
                            .get("exports")
                            .and_then(toml::Value::as_array)
                            .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect::<Vec<_>>())
                            .unwrap_or_default();
                        list.sort();
                        (alias.clone(), list)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    fn without_exports(source: &str) -> Result<toml::Value> {
        let mut value = toml::Value::try_from(parse_axiom_deps_v2(source)?)?;
        if let Some(extensions) = value.get_mut("extensions").and_then(toml::Value::as_table_mut) {
            for (_, table) in extensions.iter_mut() {
                if let Some(table) = table.as_table_mut() {
                    table.remove("exports");
                }
            }
        }
        Ok(value)
    }
    let (before_value, after_value) = (toml::Value::try_from(parse_axiom_deps_v2(before)?)?, toml::Value::try_from(parse_axiom_deps_v2(after)?)?);
    let (before_exports, after_exports) = (exports(&before_value), exports(&after_value));
    let derived = axiom_lib::extension_source::derive_extension_exports(after_path).ok();
    let mut notes = Vec::new();
    for (alias, listed) in &before_exports {
        let Some(now) = after_exports.get(alias) else { continue };
        if listed == now {
            continue;
        }
        if now.is_empty() {
            match derived.as_ref().and_then(|derived| derived.get(alias)) {
                Some(list) if list == listed => notes.push(format!("extension `{alias}`: exports now derived; the removed list [{}] equals the derived exports, so the binding and authority are unchanged", listed.join(", "))),
                Some(list) => notes.push(format!("extension `{alias}`: exports now derived as [{}], which differs from the removed list [{}]", list.join(", "), listed.join(", "))),
                None => notes.push(format!("extension `{alias}`: exports now derived (removed [{}]); the derived list is not recorded yet, build the extension to record it", listed.join(", "))),
            }
        } else {
            notes.push(format!("extension `{alias}`: exports changed from [{}] to [{}]", listed.join(", "), now.join(", ")));
        }
    }
    // w3-b: requested budgets (`maxRounds`, `deadlineMs`): an increase is a
    // reviewed permission increase, as in the authority diff.
    let (before_manifest, after_manifest) = (parse_axiom_deps_v2(before)?, parse_axiom_deps_v2(after)?);
    for (alias, extension) in &after_manifest.extensions {
        let Some(earlier) = before_manifest.extensions.get(alias) else { continue };
        let (old, new) = (earlier.budgets, extension.budgets);
        for (name, previous, current) in [
            ("maxRounds", u64::from(old.max_rounds_or_default()), u64::from(new.max_rounds_or_default())),
            ("deadlineMs", old.deadline_ms_or_default(), new.deadline_ms_or_default()),
        ] {
            if current > previous {
                notes.push(format!("extension `{alias}`: budgets.{name} {previous} -> {current}: permission increase (needs review)"));
            } else if current < previous {
                notes.push(format!("extension `{alias}`: budgets.{name} {previous} -> {current}: reduced"));
            }
        }
    }
    // w3-c (X10d): each permission added or removed, so a generated request
    // written with `axiom dependencies request --from-summary --write` is
    // reviewed item by item here; an added permission is a permission
    // increase that needs review, as in the authority lock diff.
    for (alias, extension) in &after_manifest.extensions {
        let Some(earlier) = before_manifest.extensions.get(alias) else { continue };
        let lower = |permissions| axiom_lib::extension_binding::canonical_requested_authority(permissions).map(|values| values.into_iter().collect::<std::collections::BTreeSet<_>>());
        let (old, new) = (lower(&earlier.permissions)?, lower(&extension.permissions)?);
        for permission in new.difference(&old) {
            notes.push(format!("extension `{alias}`: permission added: {}: permission increase (needs review)", axiom_lib::permission_request::permission_text(permission)));
        }
        for permission in old.difference(&new) {
            notes.push(format!("extension `{alias}`: permission removed: {}: reduced", axiom_lib::permission_request::permission_text(permission)));
        }
    }
    if without_exports(before)? != without_exports(after)? {
        notes.push("other declarations changed as well (the canonical manifest differs beyond `exports`)".into());
    }
    Ok(notes)
}

/// w1-p D6: one summary line per manifest for deprecated-but-valid forms
/// (the editor shows them per item). `None` when there is nothing to say.
pub(crate) fn deprecation_summary(path: &Path, source: &str) -> Option<String> {
    let aliases = axiom_lib::deps_manifest::permission_shorthand_candidates(source);
    (!aliases.is_empty()).then(|| {
        format!(
            "hint: {}: {} extension(s) ({}) use long-form permissions where the shorthand applies; `axiom dependencies format --permissions shorthand --write` rewrites them [{}]",
            path.display(),
            aliases.len(),
            aliases.join(", "),
            axiom_lib::deps_manifest::PERMISSIONS_SHORTHAND_HINT
        )
    })
}

pub async fn handle_check(deps: PathBuf, json: bool, against: Option<PathBuf>) -> Result<()> {
    handle_check_with(deps, json, against, None).await
}

/// `axiom dependencies check [--from-summary [--out <dir>]]`: with
/// `from_summary` (the build output root), the X10d report of every
/// extension is added (missing / unused / not derivable), and a missing
/// permission fails the check (`AXIOM_PERMISSION_REQUEST_MISSING`).
pub async fn handle_check_with(deps: PathBuf, json: bool, against: Option<PathBuf>, from_summary: Option<PathBuf>) -> Result<()> {
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
    let review = match &against {
        Some(path) => {
            let before = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
            Some((digest_hex(&canonical_axiom_deps_v2_bytes(&before)?), explain_canonical_change(&before, &source, &deps)?))
        }
        None => None,
    };
    let requests = match &from_summary {
        Some(out) => {
            let out = if out.is_absolute() { out.clone() } else { deps.parent().filter(|dir| !dir.as_os_str().is_empty()).map(|dir| dir.join(out)).unwrap_or_else(|| out.clone()) };
            Some(super::dependencies_request::reports(&deps, None, &out)?)
        }
        None => None,
    };
    if json {
        let mut report = serde_json::json!({
            "status": "valid",
            "path": deps,
            "format": manifest.format,
            "canonicalSha256": digest_hex(&canonical),
            "contracts": manifest.contracts.len(),
            "packages": manifest.packages.len(),
            "extensions": manifest.extensions.len(),
            "environments": manifest.dependency_environments.len(),
            "registries": manifest.dependency_registries.len(),
        });
        if let Some((before, changes)) = &review {
            report["against"] = serde_json::json!({"canonicalSha256": before, "changes": changes});
        }
        let shorthand = axiom_lib::deps_manifest::permission_shorthand_candidates(&source);
        if !shorthand.is_empty() {
            report["hints"] = serde_json::json!([{"code": axiom_lib::deps_manifest::PERMISSIONS_SHORTHAND_HINT, "extensions": shorthand}]);
        }
        if let Some(requests) = &requests {
            report["extensions"] = serde_json::to_value(requests)?;
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Valid AxiomDeps.toml v2: {}", deps.display());
        println!("Canonical manifest: {}", digest_hex(&canonical));
        if let Some(summary) = deprecation_summary(&deps, &source) {
            eprintln!("{summary}");
        }
        if let Some((before, changes)) = &review {
            if changes.is_empty() {
                println!("Canonical manifest unchanged from {before}.");
            } else {
                println!("Canonical manifest changed from {before}:");
                for change in changes {
                    println!("  - {change}");
                }
            }
        }
        println!(
            "{} contract(s), {} Axiom package(s), {} extension(s), {} build environment(s).",
            manifest.contracts.len(),
            manifest.packages.len(),
            manifest.extensions.len(),
            manifest.dependency_environments.len(),
        );
        for request in requests.iter().flatten() {
            print!("{}", request.render());
        }
    }
    let missing: Vec<&str> = requests.iter().flatten().filter(|request| request.missing_required).map(|request| request.alias.as_str()).collect();
    if !missing.is_empty() {
        bail!("{}: AxiomDeps.toml lacks permission(s) a known effect needs or the code attempts without a permission (ungranted): {}", axiom_lib::permission_request::PERMISSION_REQUEST_MISSING, missing.join(", "));
    }
    Ok(())
}

/// `axiom dependencies format --permissions` (w1-p D7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum PermissionsArg {
    Long,
    Shorthand,
    Preserve,
}

impl From<PermissionsArg> for axiom_lib::deps_manifest::PermissionStyle {
    fn from(value: PermissionsArg) -> Self {
        match value {
            PermissionsArg::Long => Self::Long,
            PermissionsArg::Shorthand => Self::Shorthand,
            PermissionsArg::Preserve => Self::Preserve,
        }
    }
}

pub async fn handle_format(
    deps: PathBuf,
    check: bool,
    write: bool,
    permissions: PermissionsArg,
) -> Result<()> {
    let source =
        std::fs::read_to_string(&deps).with_context(|| format!("read {}", deps.display()))?;
    let formatted =
        axiom_lib::deps_manifest::format_axiom_deps_v2_with_style(&source, permissions.into())?;
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

/// `axiom dependencies check-compat <package>@<version>`: builds and runs a
/// probe extension that imports the package, using this application's
/// TypeScript environment, registry and policy, and reports the first stage
/// that fails with what to do about it.
pub async fn handle_check_compat(
    spec: String,
    deps: PathBuf,
    registry: Option<String>,
    cache: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    use axiom_build::core::typescript_compat::{check_typescript_package, CompatibilityRequest};
    let (package, version) = split_package_spec(&spec)?;
    let cache = dependency_cache_root(cache)?;
    let work = tempfile::Builder::new().prefix("axiom-check-compat-").tempdir()?;
    let request = CompatibilityRequest {
        deps,
        package,
        version,
        registry,
        cache,
        work_root: work.path().to_path_buf(),
    };
    // Resolution, the build tools and the sandbox all block; keep them off
    // Tokio's async workers (see `handle_resolve`).
    let report = tokio::task::spawn_blocking(move || {
        check_typescript_package(&request, &HttpRegistryTransport)
    })
    .await
    .context("compatibility check worker failed")??;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        let verdict = if report.compatible { "compatible" } else { "not compatible" };
        println!("{}@{}: {verdict}", report.package, report.version);
        for stage in &report.stages {
            println!("  {} {}", if stage.passed { "ok  " } else { "FAIL" }, stage.stage);
            if let Some(detail) = &stage.detail {
                for line in detail.lines().take(8) {
                    println!("       {line}");
                }
            }
        }
        if let Some(remediation) = &report.remediation {
            println!("  What to do: {remediation}");
        }
        if let Some(caveat) = &report.caveat {
            println!("  Caveat: {caveat}");
        }
        for lint in &report.lints {
            println!("  note: {lint}");
        }
    }
    if !report.compatible {
        std::process::exit(1);
    }
    Ok(())
}

/// `name@version`, with scoped names (`@scope/name@1.2.3`).
fn split_package_spec(spec: &str) -> Result<(String, String)> {
    let at = spec[1..]
        .rfind('@')
        .map(|index| index + 1)
        .context("give the package as <name>@<exact version>, for example zod@4.1.12")?;
    let (name, version) = (&spec[..at], &spec[at + 1..]);
    if name.is_empty() || version.is_empty() {
        bail!("give the package as <name>@<exact version>, for example zod@4.1.12");
    }
    Ok((name.to_string(), version.to_string()))
}

#[cfg(test)]
mod format_permissions_tests {
    use super::*;

    const MANIFEST: &str = "format = \"axiom-deps/v2\"\ntargets = [\"server\"]\n\n[extensions.pricing]\nlanguage = \"python\"\nsource = \"guest.py\"\nexports = [\"call\"]\ntargets = [\"server\"]\n\n[extensions.pricing.permissions]\nresources = [\n{ kind = \"resource\", import = {namespace = \"secrets\", operation = \"payment\", mode = \"call\"}, resource = {type = \"secret-use\", name = \"payment\"}}\n]\n\n[extensions.pricing.permissions.ui-state.cart]\nread = [\"subtotal_cents\"]\n";

    /// w1-p D9: the canonical digest is sensitive to an omitted `exports`
    /// list (the manifest is what the author wrote); the review says
    /// "exports now derived" instead of a bare digest change.
    #[test]
    fn d9_review_names_exports_now_derived() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("logic")).unwrap();
        std::fs::write(root.join("logic/pricing.rs"), "pub fn pricing() {}\n").unwrap();
        let with = "format = \"axiom-deps/v2\"\ntargets = [\"web\"]\n\n[extensions.pricing]\nlanguage = \"rust\"\nsource = \"logic/pricing.rs\"\nexports = [\"apply\"]\ntargets = [\"web\"]\n";
        let without = with.replace("exports = [\"apply\"]\n", "");
        let deps = root.join("AxiomDeps.toml");
        std::fs::write(&deps, &without).unwrap();
        assert_ne!(canonical_axiom_deps_v2_bytes(with).unwrap(), canonical_axiom_deps_v2_bytes(&without).unwrap());
        assert!(explain_canonical_change(with, with, &deps).unwrap().is_empty());
        let notes = explain_canonical_change(with, &without, &deps).unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("exports now derived") && notes[0].contains("not recorded yet"), "{notes:?}");
        // After a build recorded the derived list (in .axiom/AxiomDeps.lock).
        axiom_lib::authored_dependencies::record_derived_extension_exports(
            &axiom_lib::project_state::deps_lock(root), "pricing", &["apply".into()]).unwrap();
        let notes = explain_canonical_change(with, &without, &deps).unwrap();
        assert!(notes[0].contains("equals the derived exports"), "{notes:?}");
        let other = without.replace("targets = [\"web\"]\n\n", "targets = [\"ios\", \"web\"]\n\n");
        let notes = explain_canonical_change(with, &other, &deps).unwrap();
        assert!(notes.iter().any(|note| note.contains("beyond `exports`")), "{notes:?}");
    }

    /// w3-b: raising `deadlineMs` (or `maxRounds`) is named as a permission
    /// increase in the review; lowering it as a reduction.
    #[test]
    fn budget_increases_are_named_as_permission_increases() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logic")).unwrap();
        std::fs::write(dir.path().join("logic/pricing.rs"), "pub fn pricing() {}\n").unwrap();
        let deps = dir.path().join("AxiomDeps.toml");
        let base = "format = \"axiom-deps/v2\"\ntargets = [\"web\"]\n\n[extensions.pricing]\nlanguage = \"rust\"\nsource = \"logic/pricing.rs\"\nexports = [\"apply\"]\ntargets = [\"web\"]\n";
        let longer = format!("{base}\n[extensions.pricing.budgets]\nmaxRounds = 4\ndeadlineMs = 10000\n");
        let notes = explain_canonical_change(base, &longer, &deps).unwrap();
        assert!(notes.iter().any(|n| n.contains("budgets.deadlineMs 1000 -> 10000: permission increase")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("budgets.maxRounds 1 -> 4: permission increase")), "{notes:?}");
        let notes = explain_canonical_change(&longer, base, &deps).unwrap();
        assert!(notes.iter().any(|n| n.contains("budgets.deadlineMs 10000 -> 1000: reduced")), "{notes:?}");
    }

    /// w3-c (X10d): the existing review names every permission a written
    /// request adds (an increase that needs review) or removes.
    #[test]
    fn permission_changes_are_named_in_the_review() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logic")).unwrap();
        std::fs::write(dir.path().join("logic/pricing.rs"), "pub fn pricing() {}\n").unwrap();
        let deps = dir.path().join("AxiomDeps.toml");
        let base = "format = \"axiom-deps/v2\"\ntargets = [\"web\"]\n\n[extensions.pricing]\nlanguage = \"rust\"\nsource = \"logic/pricing.rs\"\nexports = [\"apply\"]\ntargets = [\"web\"]\n";
        let before = format!("{base}\n[extensions.pricing.permissions.ui.cart]\nread = [\"subtotal\"]\nwrite = [\"note\"]\n");
        let after = format!("{base}\n[extensions.pricing.permissions]\nsecrets = [\"payment\"]\n\n[extensions.pricing.permissions.ui.cart]\nread = [\"subtotal\"]\n");
        let notes = explain_canonical_change(&before, &after, &deps).unwrap();
        assert!(notes.iter().any(|n| n == "extension `pricing`: permission added: secret payment: permission increase (needs review)"), "{notes:?}");
        assert!(notes.iter().any(|n| n == "extension `pricing`: permission removed: ui-state cart.note (write): reduced"), "{notes:?}");
        assert!(!notes.iter().any(|n| n.contains("subtotal")), "{notes:?}");
    }

    /// w1-p D7: `--permissions long|shorthand|preserve` reaches the formatter
    /// and both conversions round-trip through `--write` and `--check`.
    #[tokio::test]
    async fn d7_format_permissions_option_converts_both_ways() {
        let dir = tempfile::tempdir().unwrap();
        let deps = dir.path().join("AxiomDeps.toml");
        std::fs::write(&deps, MANIFEST).unwrap();
        handle_format(deps.clone(), false, true, PermissionsArg::Preserve).await.unwrap();
        let long = std::fs::read_to_string(&deps).unwrap();
        assert!(long.contains("resources") && long.contains("ui-state"), "{long}");
        handle_format(deps.clone(), false, true, PermissionsArg::Shorthand).await.unwrap();
        let short = std::fs::read_to_string(&deps).unwrap();
        assert!(short.contains("secrets = [\"payment\"]") && short.contains("permissions.ui.cart") && !short.contains("resources"), "{short}");
        // Preserve keeps the shorthand; --check agrees in that style only.
        handle_format(deps.clone(), true, false, PermissionsArg::Preserve).await.unwrap();
        handle_format(deps.clone(), true, false, PermissionsArg::Shorthand).await.unwrap();
        assert!(handle_format(deps.clone(), true, false, PermissionsArg::Long).await.is_err());
        handle_format(deps.clone(), false, true, PermissionsArg::Long).await.unwrap();
        assert_eq!(std::fs::read_to_string(&deps).unwrap(), long);
    }
}

#[cfg(test)]
mod check_compat_tests {
    use super::split_package_spec;

    #[test]
    fn package_specs_split_at_the_version_separator() {
        assert_eq!(split_package_spec("zod@4.1.12").unwrap(), ("zod".into(), "4.1.12".into()));
        assert_eq!(split_package_spec("@scope/name@1.0.0").unwrap(), ("@scope/name".into(), "1.0.0".into()));
        assert!(split_package_spec("zod").is_err());
        assert!(split_package_spec("@scope/name").is_err());
        assert!(split_package_spec("zod@").is_err());
    }
}

fn dependency_cache_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    explicit
        .or_else(|| dirs::cache_dir().map(|root| root.join("axiom/dependencies/v1")))
        .context("cannot determine the Axiom dependency cache directory; pass --cache explicitly")
}

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
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
