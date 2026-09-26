use anyhow::{bail, Context, Result};
use axiom_lib::{
    extension_authority::{decode_authority_lock, verify_authority_lock_for_load},
    package::{
        verify_axiom_package, verify_extension_module, DecodedAxiomPackage,
        ExtensionPackagePayload, PackageContents, PackageTarget,
    },
    package_resolver::PackageDependencyLock,
};
use axiom_ui::UiTarget;
use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{header, Response, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use dialoguer::{theme::ColorfulTheme, Select};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::{IsTerminal, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const MAX_ARCHIVE_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SingleManifest {
    format: String,
    application_id: String,
    target: String,
    mode: String,
    graph_revision: String,
    host: HostIdentity,
}

#[derive(Debug, Clone, Deserialize)]
struct HostIdentity {
    version: String,
    variant: String,
    sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MultiManifest {
    format: String,
    application_id: String,
    targets: Vec<MultiTarget>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MultiTarget {
    target: String,
    path: String,
    sha256: String,
    mode: String,
    graph_revision: String,
}

#[derive(Debug, Clone)]
struct TargetApplication {
    manifest: SingleManifest,
    bytes: Vec<u8>,
    files: HashMap<String, Vec<u8>>,
}

#[derive(Debug)]
struct ApplicationArchive {
    application_id: String,
    archive_sha256: String,
    bytes: Vec<u8>,
    targets: BTreeMap<String, TargetApplication>,
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_safe_path(path: &str) -> bool {
    !path.is_empty()
        && !Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn read_zip(bytes: &[u8], label: &str) -> Result<HashMap<String, Vec<u8>>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .with_context(|| format!("{label} is not a valid .axiomapp ZIP archive"))?;
    let mut files = HashMap::new();
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if !is_safe_path(&name) {
            bail!("AXIOM_APP_PATH: {label} contains unsafe entry `{name}`");
        }
        if entry.size() > MAX_ARCHIVE_FILE_BYTES {
            bail!("AXIOM_APP_SIZE: {label} entry `{name}` exceeds the 1 GiB limit");
        }
        total = total.saturating_add(entry.size());
        if total > MAX_ARCHIVE_BYTES {
            bail!("AXIOM_APP_SIZE: {label} exceeds the 2 GiB expanded-size limit");
        }
        let mut value = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut value)?;
        if files.insert(name.clone(), value).is_some() {
            bail!("AXIOM_APP_PATH: {label} contains duplicate entry `{name}`");
        }
    }
    Ok(files)
}

fn verify_checksums(files: &HashMap<String, Vec<u8>>, label: &str) -> Result<()> {
    let checksum_bytes = files
        .get("checksums.sha256")
        .with_context(|| format!("AXIOM_APP_CHECKSUM: {label} has no checksums.sha256"))?;
    let checksums = std::str::from_utf8(checksum_bytes)
        .with_context(|| format!("AXIOM_APP_CHECKSUM: {label} checksum list is not UTF-8"))?;
    let mut verified = std::collections::HashSet::new();
    for (line_number, line) in checksums.lines().enumerate() {
        let (expected, path) = line.split_once("  ").with_context(|| {
            format!(
                "AXIOM_APP_CHECKSUM: malformed line {} in {label}",
                line_number + 1
            )
        })?;
        if expected.len() != 64
            || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !is_safe_path(path)
        {
            bail!(
                "AXIOM_APP_CHECKSUM: invalid entry on line {} in {label}",
                line_number + 1
            );
        }
        let bytes = files.get(path).with_context(|| {
            format!("AXIOM_APP_CHECKSUM: {label} is missing declared entry `{path}`")
        })?;
        if sha256_bytes(bytes) != expected.to_ascii_lowercase() {
            bail!("AXIOM_APP_CHECKSUM: digest mismatch for `{path}` in {label}");
        }
        verified.insert(path);
    }
    for path in files
        .keys()
        .filter(|path| path.as_str() != "checksums.sha256")
    {
        if !verified.contains(path.as_str()) {
            bail!("AXIOM_APP_CHECKSUM: `{path}` is not covered by {label} checksums");
        }
    }
    Ok(())
}

fn package_target(target: &str) -> Result<PackageTarget> {
    match target {
        "android" => Ok(PackageTarget::Android),
        "ios" => Ok(PackageTarget::Ios),
        "web" => Ok(PackageTarget::Web),
        _ => bail!("AXIOM_APP_TARGET: unsupported application target `{target}`"),
    }
}

fn required_extension_string<'a>(
    entry: &'a serde_json::Value,
    key: &str,
    label: &str,
) -> Result<&'a str> {
    entry[key]
        .as_str()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("AXIOM_APP_EXTENSION: {label} has no valid `{key}`"))
}

fn require_extension_file<'a>(
    files: &'a HashMap<String, Vec<u8>>,
    path: &str,
    expected_sha256: &str,
    label: &str,
) -> Result<&'a [u8]> {
    if !is_safe_path(path)
        || expected_sha256.len() != 64
        || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("AXIOM_APP_EXTENSION: {label} has an unsafe artifact path or digest");
    }
    let bytes = files
        .get(path)
        .with_context(|| format!("AXIOM_APP_EXTENSION: {label} is missing `{path}`"))?;
    if sha256_bytes(bytes) != expected_sha256 {
        bail!("AXIOM_APP_EXTENSION: {label} digest mismatch for `{path}`");
    }
    Ok(bytes)
}

/// Re-run the executable package, authority, and byte checks from embedded
/// facts before an archive reaches any target host. The archive checksum is
/// necessary but not sufficient: this check also prevents an internally
/// consistent replacement from changing the signed module, target, or grant.
fn verify_embedded_extensions(
    files: &HashMap<String, Vec<u8>>,
    runtime_config: &serde_json::Value,
    target: &str,
    label: &str,
) -> Result<()> {
    let Some(extensions) = runtime_config["extensions"].as_array() else {
        // Pre-extension application artifacts remain readable. A current
        // assembler always emits the explicit empty array and extension lock.
        return Ok(());
    };
    if extensions.is_empty() && !files.contains_key("runtime/extensions/extension-lock.json") {
        return Ok(());
    }
    let lock_bytes = files
        .get("runtime/extensions/extension-lock.json")
        .with_context(|| format!("AXIOM_APP_EXTENSION: {label} has no extension lock"))?;
    let extension_lock: serde_json::Value = serde_json::from_slice(lock_bytes)
        .with_context(|| format!("AXIOM_APP_EXTENSION: invalid extension lock in {label}"))?;
    if extension_lock["format"].as_str() != Some("axiom-application-extension-lock/v1")
        || extension_lock["target"].as_str() != Some(target)
        || extension_lock["extensions"] != serde_json::Value::Array(extensions.clone())
    {
        bail!("AXIOM_APP_EXTENSION: {label} extension lock does not match runtime configuration");
    }
    let package_target = package_target(target)?;
    for entry in extensions {
        let alias = required_extension_string(entry, "extension", label)?;
        if entry["target"].as_str() != Some(target)
            || entry["verified"].as_bool() != Some(true)
            || !entry["exports"].as_array().is_some_and(|exports| {
                !exports.is_empty()
                    && exports
                        .iter()
                        .all(|value| value.as_str().is_some_and(|name| !name.is_empty()))
            })
        {
            bail!("AXIOM_APP_EXTENSION: `{alias}` is not a verified target-bound export set");
        }
        let module_sha256 = required_extension_string(entry, "moduleSha256", label)?;
        let package_sha256 = required_extension_string(entry, "packageSha256", label)?;
        let authority_sha256 = required_extension_string(entry, "authoritySha256", label)?;
        let package_lock_sha256 = required_extension_string(entry, "packageLockSha256", label)?;
        let package_lock_path = required_extension_string(entry, "packageLockPath", label)?;
        let expected_module_path =
            format!("runtime/extensions/modules/{module_sha256}/module.wasm");
        let expected_package_path = format!("runtime/extensions/packages/{package_sha256}.axiom");
        let expected_authority_path =
            format!("runtime/extensions/authority/{authority_sha256}.json");
        let expected_package_lock_path =
            format!("runtime/extensions/locks/{package_lock_sha256}.json");
        if entry["modulePath"].as_str() != Some(expected_module_path.as_str())
            || entry["packagePath"].as_str() != Some(expected_package_path.as_str())
            || entry["authorityLockPath"].as_str() != Some(expected_authority_path.as_str())
            || package_lock_path != expected_package_lock_path
        {
            bail!("AXIOM_APP_EXTENSION: `{alias}` uses non-canonical embedded artifact paths");
        }
        let module = require_extension_file(files, &expected_module_path, module_sha256, alias)?;
        let package = require_extension_file(files, &expected_package_path, package_sha256, alias)?;
        let authority =
            require_extension_file(files, &expected_authority_path, authority_sha256, alias)?;
        let package_lock_bytes = files.get(&expected_package_lock_path).with_context(|| {
            format!("AXIOM_APP_EXTENSION: `{alias}` has no embedded package lock")
        })?;
        if sha256_bytes(package_lock_bytes) != package_lock_sha256 {
            bail!("AXIOM_APP_EXTENSION: `{alias}` package lock path does not bind its bytes");
        }
        let package_lock: PackageDependencyLock = serde_json::from_slice(package_lock_bytes)
            .with_context(|| format!("AXIOM_APP_EXTENSION: `{alias}` package lock is invalid"))?;
        if package_lock.format != "axiom-package-lock/v1"
            || !package_lock.targets.contains(&package_target)
        {
            bail!("AXIOM_APP_EXTENSION: `{alias}` package lock does not support `{target}`");
        }
        let locked = package_lock.packages.get(alias).with_context(|| {
            format!("AXIOM_APP_EXTENSION: `{alias}` is absent from its embedded package lock")
        })?;
        if locked.artifact_sha256 != package_sha256
            || !locked.targets.contains(&package_target)
            || locked.kind != axiom_lib::package::PackageKind::Extension
        {
            bail!("AXIOM_APP_EXTENSION: `{alias}` package lock does not authorize this target package");
        }
        if locked
            .extension_module
            .as_ref()
            .is_none_or(|module_metadata| {
                module_metadata.sha256 != module_sha256
                    || module_metadata.byte_length != module.len() as u64
                    || module_metadata.media_type != "application/wasm"
            })
        {
            bail!("AXIOM_APP_EXTENSION: `{alias}` package lock does not bind the embedded WASM module");
        }
        let decoded = verify_axiom_package(
            package,
            &locked.artifact_sha256,
            locked.signature.as_deref(),
            locked.public_key.as_deref(),
        )?;
        let DecodedAxiomPackage::Package(decoded) = decoded else {
            bail!("AXIOM_APP_EXTENSION: `{alias}` is not an executable package envelope");
        };
        let PackageContents::Extension(payload @ ExtensionPackagePayload::V2(executable)) =
            &decoded.contents
        else {
            bail!("AXIOM_APP_EXTENSION: `{alias}` is not an axiom-extension/v2 package");
        };
        verify_extension_module(payload, module)?;
        let authority = decode_authority_lock(authority)?;
        let application: axiom_lib::package::PackageIdentity =
            serde_json::from_value(entry["application"].clone()).with_context(|| {
                format!("AXIOM_APP_EXTENSION: `{alias}` has invalid application identity")
            })?;
        verify_authority_lock_for_load(
            &authority,
            &application,
            &decoded.package,
            executable,
            &decoded.targets,
            false,
        )?;
        let effective = authority.effective.get(&package_target).with_context(|| {
            format!("AXIOM_APP_EXTENSION: `{alias}` authority lock has no {target} grant")
        })?;
        if serde_json::to_value(effective)? != entry["effectiveAuthority"]
            || authority.interface_sha256
                != required_extension_string(entry, "interfaceSha256", alias)?
            || authority.module_sha256 != module_sha256
            || authority.limits_sha256 != required_extension_string(entry, "limitsSha256", alias)?
        {
            bail!("AXIOM_APP_EXTENSION: `{alias}` embedded authority disagrees with runtime configuration");
        }
        let bindings = entry["bindings"].as_array().with_context(|| {
            format!("AXIOM_APP_EXTENSION: `{alias}` has no compiler-owned frontend bindings")
        })?;
        let mut action_ids = std::collections::BTreeSet::new();
        for binding in bindings {
            let action = required_extension_string(binding, "actionSemanticId", alias)?;
            let export = required_extension_string(binding, "export", alias)?;
            let interface = required_extension_string(binding, "interfaceSha256", alias)?;
            let abi_symbol = required_extension_string(binding, "abiSymbol", alias)?;
            if !action_ids.insert(action.to_owned())
                || !entry["exports"].as_array().is_some_and(|exports| {
                    exports.iter().any(|value| value.as_str() == Some(export))
                })
                || interface != required_extension_string(entry, "interfaceSha256", alias)?
                || !abi_symbol.starts_with("axiom_extension__")
            {
                bail!("AXIOM_APP_EXTENSION: `{alias}` has an invalid frontend action binding");
            }
            if let Some(scope) = binding["stateScope"].as_str() {
                let permitted = effective.permissions.iter().any(|permission| {
                    matches!(
                        permission,
                        axiom_lib::extension_authority::AuthorityPermission::UiState {
                            scope: granted_scope,
                            ..
                        } if granted_scope == scope
                    )
                });
                if !permitted {
                    bail!("AXIOM_APP_EXTENSION: `{alias}` action `{action}` selects UI scope `{scope}` without effective authority");
                }
            } else if !binding["stateScope"].is_null() {
                bail!(
                    "AXIOM_APP_EXTENSION: `{alias}` action `{action}` has an invalid state scope"
                );
            }
        }
    }
    Ok(())
}

fn parse_single(bytes: Vec<u8>, label: &str) -> Result<TargetApplication> {
    let files = read_zip(&bytes, label)?;
    verify_checksums(&files, label)?;
    let manifest: SingleManifest = serde_json::from_slice(
        files
            .get("manifest.json")
            .with_context(|| format!("AXIOM_APP_MANIFEST: {label} has no manifest.json"))?,
    )
    .with_context(|| format!("AXIOM_APP_MANIFEST: invalid manifest in {label}"))?;
    if manifest.format != "axiom-application/v1" {
        bail!(
            "AXIOM_APP_FORMAT: unsupported target artifact format `{}`",
            manifest.format
        );
    }
    parse_target(&manifest.target)?;
    let required = match manifest.target.as_str() {
        "ios" => vec![
            "runtime/config.json",
            "payload/main.lynx.bundle",
            "platform/AxiomUIHost.app.zip",
        ],
        "android" => vec![
            "runtime/config.json",
            "payload/main.lynx.bundle",
            "platform/AxiomUIHost.apk",
        ],
        "web" => vec![
            "runtime/config.json",
            "index.html",
            "host.css",
            "host.js",
            "foreign-island.js",
            "axiom-extension-browser-kernel.mjs",
            "axiom-extension-worker.mjs",
            "wasm-policy.mjs",
            "axiom_runtime.js",
            "axiom_runtime_bg.wasm",
            "__axiom/app.json",
        ],
        _ => unreachable!("target was validated above"),
    };
    for path in required {
        if !files.get(path).is_some_and(|value| !value.is_empty()) {
            bail!("AXIOM_APP_CONTENT: {label} is missing required `{path}`");
        }
    }
    if manifest.target != "web" {
        let host_path = if manifest.target == "ios" {
            "platform/AxiomUIHost.app.zip"
        } else {
            "platform/AxiomUIHost.apk"
        };
        if sha256_bytes(&files[host_path]) != manifest.host.sha256 {
            bail!("AXIOM_APP_HOST_TAMPERED: {label} embedded host disagrees with its manifest");
        }
    }
    let runtime_config: serde_json::Value = serde_json::from_slice(&files["runtime/config.json"])
        .with_context(|| {
        format!("AXIOM_APP_CONTENT: invalid runtime/config.json in {label}")
    })?;
    verify_embedded_extensions(&files, &runtime_config, &manifest.target, label)?;
    Ok(TargetApplication {
        manifest,
        bytes,
        files,
    })
}

fn load_archive(path: &Path) -> Result<ApplicationArchive> {
    if path.extension().and_then(|value| value.to_str()) != Some("axiomapp") {
        bail!(
            "AXIOM_APP_EXTENSION: expected a .axiomapp artifact: {}",
            path.display()
        );
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read Axiom application {}", path.display()))?;
    let archive_sha256 = sha256_bytes(&bytes);
    let files = read_zip(&bytes, &path.display().to_string())?;
    let manifest_bytes = files
        .get("manifest.json")
        .context("AXIOM_APP_MANIFEST: application has no manifest.json")?;
    let format = serde_json::from_slice::<serde_json::Value>(manifest_bytes)?["format"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if format == "axiom-application/v1" {
        let target = parse_single(bytes.clone(), &path.display().to_string())?;
        let application_id = target.manifest.application_id.clone();
        return Ok(ApplicationArchive {
            application_id,
            archive_sha256,
            bytes,
            targets: BTreeMap::from([(target.manifest.target.clone(), target)]),
        });
    }
    if format != "axiom-application-set/v1" {
        bail!("AXIOM_APP_FORMAT: unsupported application format `{format}`");
    }
    verify_checksums(&files, &path.display().to_string())?;
    let manifest: MultiManifest = serde_json::from_slice(manifest_bytes)?;
    if manifest.format != "axiom-application-set/v1" || manifest.targets.is_empty() {
        bail!("AXIOM_APP_MANIFEST: multi-target application manifest is incomplete");
    }
    let mut targets = BTreeMap::new();
    for member in manifest.targets {
        parse_target(&member.target)?;
        if !is_safe_path(&member.path) {
            bail!("AXIOM_APP_PATH: unsafe target member `{}`", member.path);
        }
        let member_bytes = files.get(&member.path).with_context(|| {
            format!(
                "AXIOM_APP_MANIFEST: missing target member `{}`",
                member.path
            )
        })?;
        if sha256_bytes(member_bytes) != member.sha256 {
            bail!(
                "AXIOM_APP_CHECKSUM: target member `{}` does not match its manifest digest",
                member.path
            );
        }
        let target = parse_single(member_bytes.clone(), &member.path)?;
        if target.manifest.target != member.target
            || target.manifest.application_id != manifest.application_id
            || target.manifest.mode != member.mode
            || target.manifest.graph_revision != member.graph_revision
        {
            bail!(
                "AXIOM_APP_MANIFEST: target member `{}` disagrees with its envelope",
                member.path
            );
        }
        if targets.insert(member.target.clone(), target).is_some() {
            bail!("AXIOM_APP_MANIFEST: duplicate target `{}`", member.target);
        }
    }
    Ok(ApplicationArchive {
        application_id: manifest.application_id,
        archive_sha256,
        bytes,
        targets,
    })
}

fn parse_target(value: &str) -> Result<UiTarget> {
    match value {
        "ios" => Ok(UiTarget::Ios),
        "android" => Ok(UiTarget::Android),
        "web" => Ok(UiTarget::Web),
        _ => bail!("AXIOM_APP_TARGET: unsupported application target `{value}`"),
    }
}

fn select_target<'a>(
    archive: &'a ApplicationArchive,
    requested: Option<&str>,
) -> Result<&'a TargetApplication> {
    if let Some(target) = requested {
        return archive.targets.get(target).with_context(|| {
            format!(
                "AXIOM_APP_TARGET: target `{target}` is unavailable; choose one of: {}",
                archive
                    .targets
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
    }
    if archive.targets.len() == 1 {
        return Ok(archive.targets.values().next().expect("one target"));
    }
    if !std::io::stdin().is_terminal() {
        bail!(
            "AXIOM_APP_TARGET_REQUIRED: this artifact contains {}; pass --target <{}>",
            archive
                .targets
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            archive
                .targets
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join("|")
        );
    }
    let options = archive.targets.keys().cloned().collect::<Vec<_>>();
    let selected = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select an application target to run")
        .items(&options)
        .default(0)
        .interact()?;
    Ok(archive
        .targets
        .get(&options[selected])
        .expect("selected target"))
}

pub async fn handle_inspect(path: PathBuf) -> Result<()> {
    let archive = load_archive(&path)?;
    println!("Axiom application: {}", archive.application_id);
    println!("Archive: {}", path.display());
    println!("SHA-256: {}", archive.archive_sha256);
    println!("Targets:");
    for target in archive.targets.values() {
        println!(
            "  {}  mode={}  graph={}  host={} ({})",
            target.manifest.target,
            target.manifest.mode,
            target.manifest.graph_revision,
            target.manifest.host.version,
            target.manifest.host.variant
        );
    }
    println!("Integrity: verified");
    Ok(())
}

pub async fn handle_install(path: PathBuf) -> Result<()> {
    let archive = load_archive(&path)?;
    let root = super::ui::application_cache_root()?
        .join(safe_name(&archive.application_id))
        .join(&archive.archive_sha256);
    std::fs::create_dir_all(&root)?;
    let destination = root.join("application.axiomapp");
    // Write the bytes that passed verification rather than reopening the
    // caller's path, avoiding a verification/copy race.
    std::fs::write(&destination, &archive.bytes)?;
    let current = root
        .parent()
        .expect("application cache has parent")
        .join("current.json");
    std::fs::write(
        current,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({
                "format": "axiom-installed-application/v1",
                "applicationId": archive.application_id,
                "sha256": archive.archive_sha256,
                "artifact": destination,
                "targets": archive.targets.keys().collect::<Vec<_>>(),
            }))?
        ),
    )?;
    println!("Installed verified Axiom application without unpacking it.");
    println!("Artifact: {}", destination.display());
    Ok(())
}

fn safe_name(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let value = value.trim_matches('-');
    if value.is_empty() {
        "app".into()
    } else {
        value.into()
    }
}

fn deterministic_zip(files: &BTreeMap<String, Vec<u8>>, output: &Path) -> Result<()> {
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = output.with_extension("axiomapp.tmp");
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&temporary)?);
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for (path, bytes) in files {
        archive.start_file(path, options)?;
        archive.write_all(bytes)?;
    }
    archive.finish()?.sync_all()?;
    std::fs::rename(temporary, output)?;
    Ok(())
}

pub async fn handle_package(inputs: Vec<PathBuf>, output: PathBuf) -> Result<()> {
    if inputs.is_empty() {
        bail!("AXIOM_APP_PACKAGE: provide at least one .axiomapp input");
    }
    let mut application_id = None::<String>;
    let mut targets = BTreeMap::<String, TargetApplication>::new();
    for input in inputs {
        let archive = load_archive(&input)?;
        match &application_id {
            Some(existing) if existing != &archive.application_id => bail!(
                "AXIOM_APP_PACKAGE: application IDs differ (`{existing}` and `{}`)",
                archive.application_id
            ),
            None => application_id = Some(archive.application_id.clone()),
            _ => {}
        }
        for (name, target) in archive.targets {
            if targets.insert(name.clone(), target).is_some() {
                bail!("AXIOM_APP_PACKAGE: duplicate target `{name}`");
            }
        }
    }
    let application_id = application_id.expect("at least one archive");
    let mut files = BTreeMap::new();
    let mut members = Vec::new();
    for (target, application) in targets {
        let path = format!("targets/{target}.axiomapp");
        members.push(MultiTarget {
            target,
            path: path.clone(),
            sha256: sha256_bytes(&application.bytes),
            mode: application.manifest.mode.clone(),
            graph_revision: application.manifest.graph_revision.clone(),
        });
        files.insert(path, application.bytes);
    }
    let manifest = serde_json::json!({
        "format": "axiom-application-set/v1",
        "applicationId": application_id,
        "targets": members,
    });
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    manifest_bytes.push(b'\n');
    files.insert("manifest.json".into(), manifest_bytes);
    let checksums = files
        .iter()
        .map(|(path, bytes)| format!("{}  {}", sha256_bytes(bytes), path))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    files.insert("checksums.sha256".into(), checksums.into_bytes());
    deterministic_zip(&files, &output)?;
    println!(
        "Packaged multi-target Axiom application: {}",
        output.display()
    );
    Ok(())
}

#[derive(Clone)]
struct PackagedWebState {
    files: Arc<HashMap<String, Vec<u8>>>,
}

async fn packaged_web_file(
    State(state): State<PackagedWebState>,
    AxumPath(path): AxumPath<String>,
) -> Response<Body> {
    packaged_web_response(&state, &path)
}

async fn packaged_web_index(State(state): State<PackagedWebState>) -> Response<Body> {
    packaged_web_response(&state, "index.html")
}

fn packaged_web_response(state: &PackagedWebState, path: &str) -> Response<Body> {
    let selected = state
        .files
        .get(path)
        .map(|bytes| (path, bytes))
        .or_else(|| {
            (!path.contains('.'))
                .then(|| {
                    state
                        .files
                        .get("index.html")
                        .map(|bytes| ("index.html", bytes))
                })
                .flatten()
        });
    let Some((name, bytes)) = selected else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap();
    };
    Response::builder()
        .header(
            header::CONTENT_TYPE,
            mime_guess::from_path(name).first_or_octet_stream().as_ref(),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes.clone()))
        .unwrap()
}

async fn packaged_web_diagnostic(Json(value): Json<serde_json::Value>) -> impl IntoResponse {
    let severity = value["severity"].as_str().unwrap_or("error");
    let code = value["code"].as_str().unwrap_or("AXIOM_WEB");
    let message = value["message"]
        .as_str()
        .unwrap_or("unknown browser diagnostic");
    eprintln!("UI {severity} {code}: {message}");
    StatusCode::NO_CONTENT
}

async fn run_web(application: &TargetApplication, launch: bool) -> Result<()> {
    let state = PackagedWebState {
        files: Arc::new(application.files.clone()),
    };
    let router = Router::new()
        .route("/", get(packaged_web_index))
        .route("/__axiom/diagnostics", post(packaged_web_diagnostic))
        .route("/*path", get(packaged_web_file))
        .with_state(state);
    let port = std::env::var("AXIOM_UI_WEB_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let address = listener.local_addr()?;
    let url = format!("http://{address}/");
    if launch {
        super::ui::open_application_browser(&url)?;
    }
    println!("Running packaged web application at {url}. Press Ctrl-C to stop.");
    tokio::select! {
        result = axum::serve(listener, router) => result.context("packaged web server stopped")?,
        _ = tokio::signal::ctrl_c() => {}
    }
    Ok(())
}

pub async fn handle_run(
    path: PathBuf,
    requested_target: Option<String>,
    launch: bool,
) -> Result<()> {
    let archive = load_archive(&path)?;
    let application = select_target(&archive, requested_target.as_deref())?;
    match parse_target(&application.manifest.target)? {
        UiTarget::Web => run_web(application, launch).await,
        target if !launch => {
            println!(
                "Verified packaged {} application; pass --launch to open the simulator or device",
                target.as_str()
            );
            Ok(())
        }
        target => super::ui::run_packaged_native_application(
            target,
            &archive.archive_sha256,
            &application.manifest.application_id,
            &application.manifest.graph_revision,
            &application.manifest.host.version,
            &application.manifest.host.variant,
            &application.manifest.host.sha256,
            &application.files,
        ),
    }
}

pub fn is_axiom_application(path: &Path) -> bool {
    path.extension().and_then(|value| value.to_str()) == Some("axiomapp")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "axiom-app-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn target_artifact(root: &Path, target: &str) -> PathBuf {
        let output = root.join(format!("{target}.axiomapp"));
        let host = format!("host-{target}").into_bytes();
        let host_sha256 = sha256_bytes(&host);
        let mut files = BTreeMap::from([
            (
                "manifest.json".into(),
                format!(
                    "{{\n  \"format\": \"axiom-application/v1\",\n  \"applicationId\": \"example.app\",\n  \"target\": \"{target}\",\n  \"mode\": \"development\",\n  \"graphRevision\": \"graph-{target}\",\n  \"host\": {{ \"version\": \"0.6.1\", \"variant\": \"{}\", \"sha256\": \"{}\" }}\n}}\n",
                    if target == "ios" { "simulator" } else if target == "android" { "emulator" } else { "browser" },
                    host_sha256,
                )
                .into_bytes(),
            ),
            ("runtime/config.json".into(), b"{\"contracts\":[]}".to_vec()),
        ]);
        if target == "web" {
            for (path, bytes) in [
                ("index.html", b"<html></html>".as_slice()),
                ("host.css", b"body{}".as_slice()),
                ("host.js", b"void 0".as_slice()),
                ("foreign-island.js", b"export {}".as_slice()),
                (
                    "axiom-extension-browser-kernel.mjs",
                    b"export {}".as_slice(),
                ),
                ("axiom-extension-worker.mjs", b"void 0".as_slice()),
                ("wasm-policy.mjs", b"export {}".as_slice()),
                ("axiom_runtime.js", b"void 0".as_slice()),
                ("axiom_runtime_bg.wasm", host.as_slice()),
                ("__axiom/app.json", b"{}".as_slice()),
            ] {
                files.insert(path.into(), bytes.to_vec());
            }
        } else {
            files.insert("payload/main.lynx.bundle".into(), b"bundle".to_vec());
            files.insert(
                if target == "ios" {
                    "platform/AxiomUIHost.app.zip"
                } else {
                    "platform/AxiomUIHost.apk"
                }
                .into(),
                host,
            );
        }
        let checksums = files
            .iter()
            .map(|(path, bytes)| format!("{}  {}", sha256_bytes(bytes), path))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        files.insert("checksums.sha256".into(), checksums.into_bytes());
        deterministic_zip(&files, &output).unwrap();
        output
    }

    #[test]
    fn unsafe_archive_paths_are_rejected() {
        assert!(!is_safe_path("../escape"));
        assert!(!is_safe_path("/absolute"));
        assert!(is_safe_path("targets/web.axiomapp"));
    }

    #[tokio::test]
    async fn packages_and_reads_multiple_intact_targets() {
        let root = temporary_directory("multi");
        let ios = target_artifact(&root, "ios");
        let web = target_artifact(&root, "web");
        let output = root.join("application.axiomapp");
        handle_package(vec![ios.clone(), web.clone()], output.clone())
            .await
            .unwrap();
        let archive = load_archive(&output).unwrap();
        assert_eq!(archive.application_id, "example.app");
        assert_eq!(
            archive.targets.keys().cloned().collect::<Vec<_>>(),
            ["ios", "web"]
        );
        assert_eq!(archive.targets["ios"].bytes, std::fs::read(ios).unwrap());
        assert_eq!(archive.targets["web"].bytes, std::fs::read(web).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn duplicate_targets_are_rejected() {
        let root = temporary_directory("duplicate");
        let web = target_artifact(&root, "web");
        let error = handle_package(vec![web.clone(), web], root.join("application.axiomapp"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("duplicate target `web`"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_content_is_rejected_by_the_checksum_layer() {
        let root = temporary_directory("tamper");
        let artifact = target_artifact(&root, "web");
        let original = std::fs::read(&artifact).unwrap();
        let mut files = read_zip(&original, "fixture").unwrap();
        files.insert(
            "runtime/config.json".into(),
            b"{\"contracts\":[\"tampered\"]}".to_vec(),
        );
        let ordered = files.into_iter().collect::<BTreeMap<_, _>>();
        deterministic_zip(&ordered, &artifact).unwrap();
        let error = load_archive(&artifact).unwrap_err();
        assert!(error.to_string().contains("digest mismatch"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
