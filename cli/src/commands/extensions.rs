use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use axiom_build::core::extension_source::{
    inspect_rust_source_extension, source_authority_request, SourceBuildOptions,
};
use axiom_build::core::python_source::inspect_python_source_extension;
use axiom_build::core::typescript_source::{
    build_source_extension, inspect_typescript_source_extension,
};
use axiom_extension_abi::{Field, Invocation, Value};
use axiom_extension_broker::{DeterministicHost, InvocationContext};
use axiom_extension_host::{HostProfile, LifecycleEvent, TargetExtensionHost};
use axiom_extension_kernel::{AuthorizedModule, KernelPolicy, SandboxKernel};
use axiom_lib::{
    extension_authority::{
        decode_authority_lock, encode_authority_lock, encode_signed_document, resolve_authority,
        verify_authority_inputs, ApplicationGrant, AuthorityTrust, GrantMode, SignedDocument,
        TargetPolicy, APPLICATION_GRANT_SCHEMA, TARGET_POLICY_SCHEMA,
    },
    extension_binding::generate_extension_binding,
    extension_source::resolve_extension_sources,
    extension_workflow::{
        diff_extension_authority, extension_graph, inspect_extension,
        load_verified_extension_target, require_extension_diff_approval, verify_extension_workflow,
        ExtensionWorkflowEntry, ExtensionWorkflowManifest, RuntimeIdentities,
        EXTENSION_WORKFLOW_FORMAT,
    },
    package::{
        decode_axiom_package, DecodedAxiomPackage, ExtensionPackagePayload, PackageContents,
        PackageIdentity, PackageKind, PackageTarget,
    },
    package_resolver::{
        resolve_package_dependencies, write_package_lock, PackageDependencyManifest,
        PackageReference, PACKAGE_MANIFEST_FORMAT,
    },
    rust_source_migration::migrate_legacy_rust_extension,
    sdk_interface::{
        decode_sdk_interface_artifact, diff_sdk_interfaces, encode_sdk_interface_artifact,
        resolve_effective_sdk_interface, EffectiveSdkInterface, SdkInterfaceArtifact,
        SdkInterfaceChangeImpact,
    },
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageProof {
    format: &'static str,
    artifact_sha256: String,
    signature: String,
    public_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InterfaceInspection {
    artifact: SdkInterfaceArtifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    effective: Option<EffectiveSdkInterface>,
}

pub async fn handle_interface(
    deps: PathBuf,
    alias: String,
    authority_lock: Option<PathBuf>,
    target: Option<String>,
    out: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    let sources = resolve_extension_sources(&deps)?;
    let source = sources
        .get(&alias)
        .with_context(|| format!("AxiomDeps.toml has no extension `{alias}`"))?;
    let binding = generate_extension_binding(source)?;
    let effective = match (authority_lock, target) {
        (Some(path), Some(target)) => {
            let target = parse_target(&target)?;
            let lock = decode_authority_lock(&fs::read(&path)?)?;
            if lock.interface_sha256 != binding.interface_sha256 {
                bail!("authority lock interface digest does not match the generated SDK IR");
            }
            let authority = lock
                .effective
                .get(&target)
                .with_context(|| format!("authority lock has no effective {target:?} surface"))?;
            Some(resolve_effective_sdk_interface(
                &binding.interface,
                target,
                &authority.permissions,
            )?)
        }
        (None, None) => None,
        _ => bail!("--authority-lock and --target must be supplied together"),
    };
    if let Some(path) = out {
        fs::write(&path, encode_sdk_interface_artifact(&binding.interface)?)
            .with_context(|| format!("write SDK interface artifact {}", path.display()))?;
    }
    let inspection = InterfaceInspection {
        artifact: binding.interface,
        effective,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&inspection)?);
    } else {
        println!("{}", inspection.artifact.interface.module_alias);
        println!("  interface: {}", inspection.artifact.interface_sha256);
        println!(
            "  generator: {}@{}",
            inspection.artifact.generator.name, inspection.artifact.generator.version
        );
        println!(
            "  ABI: {}@{} (unchanged)",
            inspection.artifact.interface.abi.name, inspection.artifact.interface.abi.version
        );
        println!("  exports: {}", inspection.artifact.interface.exports.len());
        println!(
            "  requested imports: {}",
            inspection.artifact.interface.imports.len()
        );
        println!("  types: {}", inspection.artifact.interface.types.len());
        println!(
            "  language mappings: {}",
            inspection
                .artifact
                .interface
                .language_mappings
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        if let Some(effective) = inspection.effective {
            println!("  effective target: {:?}", effective.target);
            println!("  effective imports: {}", effective.imports.len());
            println!("  effective digest: {}", effective.effective_sha256);
        }
    }
    Ok(())
}

pub async fn handle_interface_diff(before: PathBuf, after: PathBuf, json: bool) -> Result<()> {
    let before = decode_sdk_interface_artifact(&fs::read(&before)?)?;
    let after = decode_sdk_interface_artifact(&fs::read(&after)?)?;
    let changes = diff_sdk_interfaces(&before, &after)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&changes)?);
    } else if changes.is_empty() {
        println!("SDK interfaces are semantically identical.");
    } else {
        for change in changes {
            let impact = match change.impact {
                SdkInterfaceChangeImpact::Compatible => "compatible",
                SdkInterfaceChangeImpact::AuthorityIncrease => "authority-increase",
                SdkInterfaceChangeImpact::Breaking => "breaking",
            };
            println!("{impact:18} {}: {}", change.path, change.summary);
        }
    }
    Ok(())
}

pub async fn handle_source_build(
    deps: PathBuf,
    alias: String,
    out: PathBuf,
    target: String,
    clean: bool,
) -> Result<()> {
    let target = parse_target(&target)?;
    let result = build_source_extension(&SourceBuildOptions {
        deps,
        alias,
        output_root: out,
        target,
        clean,
    })?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

pub async fn handle_typescript_inspect(
    deps: PathBuf,
    alias: String,
    view: String,
    json: bool,
) -> Result<()> {
    let inspection = inspect_typescript_source_extension(&deps, &alias)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&inspection)?);
        return Ok(());
    }
    match view.as_str() {
        "summary" => {
            println!("TypeScript extension: {}", inspection.alias);
            println!("Driver: axiom-typescript-source-driver/v1");
            println!("Sources: {}", inspection.source_files.join(", "));
            println!("Interface: {}", inspection.interface.interface_sha256);
            println!(
                "Dependencies: {}",
                inspection
                    .dependencies
                    .as_ref()
                    .map(|value| value.packages.len())
                    .unwrap_or_default()
            );
            println!("Use --view declarations|entry|interface for generated evidence.");
        }
        "declarations" => print!("{}", inspection.declarations),
        "entry" => print!("{}", inspection.generated_entry),
        "interface" => println!("{}", serde_json::to_string_pretty(&inspection.interface)?),
        _ => bail!("--view must be summary, declarations, entry, or interface"),
    }
    Ok(())
}

pub async fn handle_python_inspect(
    deps: PathBuf,
    alias: String,
    view: String,
    json: bool,
) -> Result<()> {
    let inspection = inspect_python_source_extension(&deps, &alias)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&inspection)?);
        return Ok(());
    }
    match view.as_str() {
        "summary" => {
            println!("Python extension: {}", inspection.alias);
            println!("Driver: axiom-python-source-driver/v1");
            println!("Profile: {}", inspection.profile);
            println!("Sources: {}", inspection.source_files.join(", "));
            println!("Interface: {}", inspection.interface.interface_sha256);
            println!(
                "Dependencies: {}",
                inspection
                    .dependencies
                    .as_ref()
                    .map(|value| value.packages.len())
                    .unwrap_or_default()
            );
            println!("Use --view stubs|lowering|source-map|interface for generated evidence.");
        }
        "stubs" => print!("{}", inspection.type_stubs),
        "lowering" => print!("{}", inspection.generated_program),
        "source-map" => println!("{}", serde_json::to_string_pretty(&inspection.source_map)?),
        "interface" => println!("{}", serde_json::to_string_pretty(&inspection.interface)?),
        _ => bail!("--view must be summary, stubs, lowering, source-map, or interface"),
    }
    Ok(())
}

pub async fn handle_rust_inspect(
    deps: PathBuf,
    alias: String,
    view: String,
    json: bool,
) -> Result<()> {
    let inspection = inspect_rust_source_extension(&deps, &alias)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&inspection)?);
        return Ok(());
    }
    match view.as_str() {
        "summary" => {
            println!("Rust extension: {}", inspection.alias);
            println!("Driver: {}", inspection.source_driver);
            println!("Sources: {}", inspection.source_files.join(", "));
            println!(
                "Exports: {}",
                inspection
                    .interface
                    .interface
                    .exports
                    .iter()
                    .map(|export| export.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!(
                "Requested imports: {}",
                inspection.interface.interface.imports.len()
            );
            println!(
                "Third-party packages: {}",
                inspection
                    .authored_dependencies
                    .as_ref()
                    .map(|graph| graph.packages.len())
                    .unwrap_or_default()
            );
            println!("Use --view macros|interface|workspace|bindings for evidence.");
        }
        "macros" => print!("{}", inspection.macro_expansion),
        "interface" => println!("{}", serde_json::to_string_pretty(&inspection.interface)?),
        "workspace" => print!("{}", inspection.generated_workspace_manifest),
        "bindings" => print!("{}", inspection.generated_bindings),
        _ => bail!("--view must be summary, macros, interface, workspace, or bindings"),
    }
    Ok(())
}

pub async fn handle_migrate_rust(
    deps: PathBuf,
    alias: String,
    out: Option<PathBuf>,
    write: bool,
    check: bool,
    json: bool,
) -> Result<()> {
    let sources = resolve_extension_sources(&deps)?;
    let extension = sources
        .get(&alias)
        .with_context(|| format!("AxiomDeps.toml has no extension `{alias}`"))?;
    if extension.language != "rust" {
        bail!(
            "extension `{alias}` uses `{}`; migrate-rust accepts only Rust sources",
            extension.language
        );
    }
    let source = fs::read_to_string(&extension.source)
        .with_context(|| format!("read {}", extension.source.display()))?;
    let migration = migrate_legacy_rust_extension(&alias, &source, &extension.exports)?;

    if check && migration.report.changed {
        bail!(
            "{} still uses legacy SDK imports; run `axiom extensions migrate-rust {alias} --deps {} --write`",
            extension.source.display(),
            deps.display()
        );
    }

    let destination = if write {
        write_migrated_source(&extension.source, &migration.source)?;
        Some(extension.source.clone())
    } else if let Some(path) = out {
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        };
        if path == extension.source {
            bail!("use --write to replace the registered source atomically");
        }
        if path.exists() {
            bail!("migration output already exists: {}", path.display());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("create migration output {}", path.display()))?;
        file.write_all(migration.source.as_bytes())?;
        file.sync_all()?;
        Some(path)
    } else {
        None
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "source": extension.source,
                "destination": destination,
                "report": migration.report,
            }))?
        );
    } else {
        println!("Rust extension migration: {alias}");
        println!("Source: {}", extension.source.display());
        println!("Classification: {:?}", migration.report.classification);
        println!("Behavior preserved: yes");
        println!(
            "Source changed: {}",
            if migration.report.changed {
                "yes"
            } else {
                "no"
            }
        );
        for change in &migration.report.changes {
            println!("  changed: {change}");
        }
        for step in &migration.report.next_steps {
            println!("  next: {step}");
        }
        if let Some(path) = destination {
            println!("Wrote: {}", path.display());
        } else if migration.report.changed {
            println!("Dry run only. Use --out or --write after review.");
        }
    }
    Ok(())
}

fn write_migrated_source(path: &Path, contents: &str) -> Result<()> {
    let parent = path
        .parent()
        .context("Rust source has no parent directory")?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .context("Rust source filename is not UTF-8")?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos();
    let temporary = parent.join(format!(".{file_name}.axiom-migrate-{nonce}.tmp"));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("create {}", temporary.display()))?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::set_permissions(&temporary, fs::metadata(path)?.permissions())?;
        fs::rename(&temporary, path)
            .with_context(|| format!("replace migrated Rust source {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() && temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Turn a registered source module into the complete signed local workflow
/// consumed by ordinary UI run/build commands. Generated Cargo/WASM/package
/// machinery stays below `out`; the application owns only Acore, AxiomDeps,
/// and authored source. Signing keys are ephemeral and never written to disk.
pub async fn handle_source_release(
    deps: PathBuf,
    alias: String,
    out: PathBuf,
    workflow: PathBuf,
    application: String,
    application_version: String,
    clean: bool,
    print_report: bool,
) -> Result<()> {
    let deps = fs::canonicalize(&deps)
        .with_context(|| format!("resolve source manifest {}", deps.display()))?;
    let project_root = deps
        .parent()
        .context("AxiomDeps.toml has no containing directory")?;
    let workflow = if workflow.is_absolute() {
        workflow
    } else {
        project_root.join(workflow)
    };
    if workflow.parent() != Some(project_root) {
        bail!("source release workflow must be written at the AxiomDeps.toml project root");
    }
    let sources = resolve_extension_sources(&deps)?;
    let source = sources
        .get(&alias)
        .with_context(|| format!("AxiomDeps.toml has no extension `{alias}`"))?;
    if !source.dependencies.is_empty() {
        bail!("source release currently requires dependency modules to be released separately before `{alias}`");
    }
    let target = *source
        .targets
        .first()
        .context("source extension declares no targets")?;
    let build = build_source_extension(&SourceBuildOptions {
        deps: deps.clone(),
        alias: alias.clone(),
        output_root: out.clone(),
        target,
        clean,
    })?;
    let package_bytes = fs::read(&build.package)?;
    let DecodedAxiomPackage::Package(package) = decode_axiom_package(&package_bytes)? else {
        bail!("source build produced a legacy package");
    };
    let PackageContents::Extension(ExtensionPackagePayload::V2(executable)) = &package.contents
    else {
        bail!("source build did not produce axiom-extension/v2");
    };
    let binding = generate_extension_binding(source)?;
    let request = source_authority_request(source, &binding, &build.module_sha256);
    let request_sha256 = digest_json(&request)?;
    if executable.request_sha256 != request_sha256 {
        bail!("source package authority request does not match generated bindings");
    }

    let output_root = if out.is_absolute() {
        out
    } else {
        project_root.join(out)
    };
    let release_root = output_root.join("release").join(&alias);
    let key_root = output_root.join("keys").join(&alias);
    fs::create_dir_all(&release_root)?;
    let package_name = format!("{alias}.axiom");
    let module_name = format!("{alias}.wasm");
    fs::write(release_root.join(&package_name), &package_bytes)?;
    fs::copy(&build.module, release_root.join(&module_name))?;

    let package_key = managed_signing_key(&key_root.join("package.key"))?;
    let package_digest = Sha256::digest(&package_bytes);
    let package_signature = BASE64.encode(package_key.sign(package_digest.as_slice()).to_bytes());
    let package_public_key = BASE64.encode(package_key.verifying_key().to_bytes());
    let package_manifest = PackageDependencyManifest {
        format: PACKAGE_MANIFEST_FORMAT.into(),
        targets: source.targets.clone(),
        extensions: Default::default(),
        packages: std::collections::BTreeMap::from([(
            alias.clone(),
            PackageReference {
                artifact: PathBuf::from(&package_name),
                name: package.package.name.clone(),
                kind: PackageKind::Extension,
                version: package.package.version.clone(),
                dependencies: Vec::new(),
                module_artifact: Some(PathBuf::from(&module_name)),
                signature: Some(package_signature),
                public_key: Some(package_public_key),
            },
        )]),
    };
    let package_manifest_path = release_root.join("AxiomDeps.toml");
    fs::write(&package_manifest_path, toml::to_string(&package_manifest)?)?;
    let package_lock = resolve_package_dependencies(&release_root, &package_manifest)?;
    let package_lock_path = release_root.join("AxiomPackages.lock");
    write_package_lock(&package_lock_path, &package_lock)?;

    let application = PackageIdentity {
        name: application,
        version: application_version,
    };
    let grant = ApplicationGrant {
        schema: APPLICATION_GRANT_SCHEMA.into(),
        application: application.clone(),
        extension: package.package.clone(),
        request_sha256: request_sha256.clone(),
        targets: source.targets.clone(),
        permissions: binding.requested_authority.clone(),
        budgets: request.budgets,
        mode: GrantMode::Release,
    };
    let request_key = managed_signing_key(&key_root.join("request.key"))?;
    let grant_key = managed_signing_key(&key_root.join("grant.key"))?;
    let authority_budgets = request.budgets;
    let signed_request = sign_document(request, &request_key)?;
    let signed_grant = sign_document(grant, &grant_key)?;
    fs::write(
        release_root.join("request.signed.json"),
        encode_signed_document(&signed_request)?,
    )?;
    fs::write(
        release_root.join("grant.signed.json"),
        encode_signed_document(&signed_grant)?,
    )?;

    let policy_key = managed_signing_key(&key_root.join("policy.key"))?;
    let mut policy_bytes = std::collections::BTreeMap::new();
    let mut policy_paths = std::collections::BTreeMap::new();
    let mut policy_trust = std::collections::BTreeMap::new();
    for target in &source.targets {
        let policy = TargetPolicy {
            schema: TARGET_POLICY_SCHEMA.into(),
            target: *target,
            permissions: binding.requested_authority.clone(),
            budgets: authority_budgets,
            allow_development_grants: false,
        };
        let signed = sign_document(policy, &policy_key)?;
        let bytes = encode_signed_document(&signed)?;
        let name = format!("{}.policy.signed.json", target_name(*target));
        fs::write(release_root.join(&name), &bytes)?;
        policy_bytes.insert(*target, bytes);
        policy_paths.insert(
            *target,
            relative_from(project_root, &release_root.join(name))?,
        );
        policy_trust.insert(
            *target,
            BASE64.encode(policy_key.verifying_key().to_bytes()),
        );
    }
    let verified = verify_authority_inputs(
        &encode_signed_document(&signed_request)?,
        &encode_signed_document(&signed_grant)?,
        &policy_bytes,
        &AuthorityTrust {
            request_public_key: BASE64.encode(request_key.verifying_key().to_bytes()),
            grant_public_key: BASE64.encode(grant_key.verifying_key().to_bytes()),
            target_policy_public_keys: policy_trust,
        },
    )?;
    let authority = resolve_authority(
        &application,
        &package.package,
        executable,
        &package.targets,
        verified,
        vec![vec![package.package.name.clone()]],
    )?;
    let authority_bytes = encode_authority_lock(&authority)?;
    let authority_path = release_root.join("authority.lock.json");
    fs::write(&authority_path, &authority_bytes)?;
    let authority_sha256 = hex::encode(Sha256::digest(&authority_bytes));

    let workflow_manifest = ExtensionWorkflowManifest {
        format: EXTENSION_WORKFLOW_FORMAT.into(),
        application,
        package_manifest: relative_from(project_root, &package_manifest_path)?,
        package_lock: relative_from(project_root, &package_lock_path)?,
        extensions: std::collections::BTreeMap::from([(
            alias.clone(),
            ExtensionWorkflowEntry {
                authority_lock: relative_from(project_root, &authority_path)?,
                authority_sha256,
                request: relative_from(project_root, &release_root.join("request.signed.json"))?,
                grant: relative_from(project_root, &release_root.join("grant.signed.json"))?,
                policies: policy_paths,
                dependencies: Vec::new(),
                runtime: RuntimeIdentities {
                    engine: match executable.sdk.language.as_str() {
                        "typescript" => "javy@9.1.0+wasmi@2.0.0",
                        "python" => "axiom-python-aot-javy@0.1.0+javy@9.1.0+wasmi@2.0.0",
                        _ => "wasmi@2.0.0",
                    }
                    .into(),
                    broker: "axiom-extension-broker@0.1.0".into(),
                    host: "axiom-extension-host@0.1.0".into(),
                    translator: None,
                },
            },
        )]),
    };
    fs::write(&workflow, toml::to_string(&workflow_manifest)?)?;
    verify_extension_workflow(&workflow)?;
    if print_report {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "format": "axiom-extension-source-release/v1",
                "alias": alias,
                "sourceSha256": build.source_sha256,
                "moduleSha256": build.module_sha256,
                "workflow": workflow,
                "generatedRoot": release_root,
                "status": "signed-and-verified",
            }))?
        );
    }
    Ok(())
}

fn managed_signing_key(path: &Path) -> Result<SigningKey> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!(
                "managed signing key must be a regular file: {}",
                path.display()
            );
        }
        let bytes = fs::read(path)?;
        let seed: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
            anyhow::anyhow!(
                "managed signing key must contain 32 bytes, found {}: {}",
                bytes.len(),
                path.display()
            )
        })?;
        return Ok(SigningKey::from_bytes(&seed));
    }
    let parent = path.parent().context("managed signing key has no parent")?;
    fs::create_dir_all(parent)?;
    let mut hasher = Sha256::new();
    hasher.update(Uuid::new_v4().as_bytes());
    hasher.update(Uuid::new_v4().as_bytes());
    let seed: [u8; 32] = hasher.finalize().into();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("create managed signing key {}", path.display()))?;
    file.write_all(&seed)?;
    file.sync_all()?;
    Ok(SigningKey::from_bytes(&seed))
}

fn sign_document<T: Serialize>(document: T, signer: &SigningKey) -> Result<SignedDocument<T>> {
    let bytes = serde_jcs::to_vec(&document)?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let digest = hex::decode(&sha256)?;
    Ok(SignedDocument {
        document,
        sha256,
        signature: BASE64.encode(signer.sign(&digest).to_bytes()),
        public_key: BASE64.encode(signer.verifying_key().to_bytes()),
    })
}

fn digest_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_jcs::to_vec(value)?)))
}

fn relative_from(root: &std::path::Path, path: &std::path::Path) -> Result<PathBuf> {
    Ok(path
        .strip_prefix(root)
        .with_context(|| format!("generated release {} escaped project root", path.display()))?
        .to_path_buf())
}

fn target_name(target: PackageTarget) -> &'static str {
    match target {
        PackageTarget::Android => "android",
        PackageTarget::Ios => "ios",
        PackageTarget::Web => "web",
        PackageTarget::Server => "server",
    }
}

pub async fn handle_build(source: PathBuf, out: PathBuf) -> Result<()> {
    let package: axiom_lib::package::AxiomPackageEnvelope =
        serde_json::from_slice(&fs::read(&source)?)?;
    if !matches!(
        package.contents,
        axiom_lib::package::PackageContents::Extension(_)
    ) {
        bail!("extension build requires an extension package draft");
    }
    let encoded = axiom_lib::package::encode_axiom_package(&package)?;
    fs::write(&out, encoded.bytes)?;
    println!("Built {} ({})", out.display(), encoded.sha256);
    Ok(())
}

pub async fn handle_sign(artifact: PathBuf, key: PathBuf, out: PathBuf) -> Result<()> {
    let bytes = fs::read(&artifact)?;
    decode_axiom_package(&bytes).context("artifact is not a valid canonical Axiom package")?;
    let digest = Sha256::digest(&bytes);
    let seed: [u8; 32] = hex::decode(fs::read_to_string(key)?.trim())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Ed25519 seed must contain exactly 32 bytes"))?;
    let signer = SigningKey::from_bytes(&seed);
    let proof = PackageProof {
        format: "axiom-package-proof/v1",
        artifact_sha256: hex::encode(digest),
        signature: BASE64.encode(signer.sign(digest.as_slice()).to_bytes()),
        public_key: BASE64.encode(signer.verifying_key().to_bytes()),
    };
    fs::write(&out, serde_jcs::to_vec(&proof)?)?;
    println!("Signed {} into {}", artifact.display(), out.display());
    Ok(())
}

pub async fn handle_resolve(deps: PathBuf, lock: PathBuf) -> Result<()> {
    super::packages::handle_resolve(deps, lock).await
}

pub async fn handle_verify(manifest: PathBuf) -> Result<()> {
    let reports = verify_extension_workflow(&manifest)?;
    println!(
        "Verified {} signed extension release(s) offline from {}.",
        reports.len(),
        manifest.display()
    );
    Ok(())
}

pub async fn handle_inspect(manifest: PathBuf, alias: String) -> Result<()> {
    let report = inspect_extension(&manifest, &alias)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

pub async fn handle_permissions(manifest: PathBuf, alias: String) -> Result<()> {
    let report = inspect_extension(&manifest, &alias)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "format": "axiom-extension-permissions/v1",
            "alias": report.alias,
            "package": report.provenance.package,
            "requested": report.requested,
            "granted": report.granted,
            "effective": report.effective,
            "dependencyPaths": report.dependency_paths,
            "developmentOnly": report.development_only,
        }))?
    );
    Ok(())
}

pub async fn handle_graph(manifest: PathBuf) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&extension_graph(&manifest)?)?
    );
    Ok(())
}

pub async fn handle_diff(before: PathBuf, after: PathBuf, approvals: Vec<String>) -> Result<()> {
    let report = diff_extension_authority(&before, &after)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    let approvals: BTreeSet<_> = approvals.into_iter().collect();
    let unknown: Vec<_> = approvals
        .iter()
        .filter(|approval| approval.as_str() != "permission-increase")
        .collect();
    if !unknown.is_empty() {
        bail!("unknown extension approval(s): {unknown:?}");
    }
    require_extension_diff_approval(&report, approvals.contains("permission-increase"))
}

pub async fn handle_test(manifest: PathBuf) -> Result<()> {
    let reports = verify_extension_workflow(&manifest)?;
    for report in &reports {
        if report.provenance.targets.is_empty() || report.effective.is_empty() {
            bail!("extension `{}` has no executable target", report.alias);
        }
    }
    println!(
        "Extension workflow conformance passed for {} release(s).",
        reports.len()
    );
    Ok(())
}

pub async fn handle_run(
    manifest: PathBuf,
    alias: String,
    target: String,
    export: Option<String>,
    input: String,
    state: Option<PathBuf>,
    audit_out: Option<PathBuf>,
) -> Result<()> {
    let report = inspect_extension(&manifest, &alias)?;
    let target = parse_target(&target)?;
    let authority = report
        .effective
        .get(&target)
        .cloned()
        .with_context(|| format!("extension `{alias}` has no authority for {target:?}"))?;
    if export.is_none() {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
            "format": "axiom-extension-run-handoff/v1",
            "alias": alias,
            "target": target,
            "package": report.provenance.package,
            "moduleSha256": report.provenance.module_sha256,
            "effectiveAuthority": authority,
            "engine": report.provenance.engine,
            "broker": report.provenance.broker,
            "host": report.provenance.host,
            "status": "verified-ready-for-host",
            }))?
        );
        return Ok(());
    }
    if target != PackageTarget::Server {
        bail!("AXIOM_EXTENSION_REFERENCE_HOST: --export is currently available only for --target server; frontend exports run through `axiom run <app>.acore --target android|ios|web`");
    }
    let loaded = load_verified_extension_target(&manifest, &alias, target)?;
    let authority_lock_sha256 = hex::encode(Sha256::digest(&loaded.authority_lock_bytes));
    let package_lock_sha256 = hex::encode(Sha256::digest(&loaded.package_lock_bytes));
    let DecodedAxiomPackage::Package(package) = decode_axiom_package(&loaded.package_bytes)? else {
        bail!("AXIOM_EXTENSION_REFERENCE_HOST: legacy extension packages cannot execute");
    };
    let PackageContents::Extension(ExtensionPackagePayload::V2(executable)) = &package.contents
    else {
        bail!("AXIOM_EXTENSION_REFERENCE_HOST: selected package is not axiom-extension/v2");
    };
    let authority_lock = decode_authority_lock(&loaded.authority_lock_bytes)?;
    let effective = axiom_lib::extension_authority::EffectiveTargetAuthority {
        permissions: loaded.effective.permissions.clone(),
        budgets: loaded.effective.budgets.clone(),
    };
    let services = Arc::new(DeterministicHost::default());
    let state_fixture_sha256 = state.as_ref().map(|path| file_sha256(path)).transpose()?;
    if let Some(path) = state.as_ref() {
        seed_reference_state(&services, &path)?;
    }
    let kernel = SandboxKernel::new(KernelPolicy::default())?;
    let authorized = AuthorizedModule::verify_brokered(
        &loaded.application,
        &package.package,
        executable,
        &package.targets,
        &authority_lock,
        target,
        &loaded.module_bytes,
        &KernelPolicy::default(),
        false,
    )?;
    let host = match executable.sdk.language.as_str() {
        "typescript" => TargetExtensionHost::load_typescript(
            HostProfile::backend(),
            authorized,
            &effective,
            Arc::clone(&services),
            1,
            executable.limits.max_effects,
            executable.limits.max_stream_batch,
        )?,
        "python" => TargetExtensionHost::load_python(
            HostProfile::backend(),
            authorized,
            &effective,
            Arc::clone(&services),
            1,
            executable.limits.max_effects,
            executable.limits.max_stream_batch,
        )?,
        _ => TargetExtensionHost::load(
            HostProfile::backend(),
            &kernel,
            authorized,
            &effective,
            Arc::clone(&services),
            1,
            executable.limits.max_effects,
            executable.limits.max_stream_batch,
        )?,
    };
    let export = export.expect("checked above");
    let input_json: serde_json::Value = serde_json::from_str(&input)
        .context("AXIOM_EXTENSION_REFERENCE_HOST: --input must be valid JSON")?;
    let input_sha256 = hex::encode(Sha256::digest(serde_jcs::to_vec(&input_json)?));
    let input = json_to_value(input_json)?;
    let invocation_result = host
        .invoke(
            Invocation {
                export: export.clone(),
                input,
                snapshots: Vec::new(),
                deadline_unix_ms: now_unix_ms().saturating_add(1_000),
            },
            InvocationContext::default(),
        )
        .await;
    // The reference command is one bounded application execution.  Close the
    // extension explicitly so its audit includes the same lifecycle boundary
    // a server process shutdown uses; no guest or state handle can outlive it.
    let teardown_result = host.lifecycle(LifecycleEvent::AppShutdown);
    let (status, output, patch_count, transaction_count, event_count, invocation_error) =
        match &invocation_result {
            Ok(result) => (
                "completed",
                Some(value_to_json(result.output.clone())),
                result.patches.len(),
                result.transactions.len(),
                result.emitted_events.len(),
                None,
            ),
            Err(error) => ("rejected", None, 0, 0, 0, Some(error.to_string())),
        };
    let audit = serde_json::json!({
        "format": "axiom-extension-audit-export/v1",
        "execution": {
            "application": loaded.application,
            "alias": alias,
            "target": "server",
            "export": export,
            "inputSha256": input_sha256,
            "moduleSha256": report.provenance.module_sha256,
            "interfaceSha256": report.provenance.interface_sha256,
            "effectiveAuthoritySha256": authority_lock_sha256,
            "packageLockSha256": package_lock_sha256,
            "stateFixtureSha256": state_fixture_sha256,
        },
        "verification": {
            "package": report.provenance.package,
            "targets": report.provenance.targets,
            "engine": report.provenance.engine,
            "broker": report.provenance.broker,
            "host": report.provenance.host,
            "effectiveAuthority": authority,
        },
        "outcome": {
            "status": status,
            "output": output,
            "patchCount": patch_count,
            "transactionCount": transaction_count,
            "eventCount": event_count,
            "error": invocation_error,
        },
        "evidence": {
            "host": {
                "provenance": host.provenance(),
                "records": host.audit_records(),
                "retention": host.audit_retention(),
            },
            "broker": {
                "records": host.broker().audit_records(),
                "retention": host.broker().audit_retention(),
                "activeHandles": host.broker().active_handle_count(),
            },
            "state": {
                "records": services.state_store().audit_records(),
                "retention": services.state_store().audit_retention(),
            },
        },
    });
    if let Some(path) = audit_out {
        let bytes = serde_jcs::to_vec(&audit)?;
        fs::write(&path, bytes)
            .with_context(|| format!("AXIOM_EXTENSION_AUDIT_EXPORT: write {}", path.display()))?;
    }
    println!("{}", serde_json::to_string_pretty(&audit)?);
    teardown_result?;
    if let Err(error) = invocation_result {
        return Err(error.into());
    }
    Ok(())
}

fn file_sha256(path: &std::path::Path) -> Result<String> {
    Ok(hex::encode(Sha256::digest(fs::read(path).with_context(
        || format!("AXIOM_EXTENSION_AUDIT_EXPORT: read {}", path.display()),
    )?)))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReferenceStateFixture {
    #[serde(default)]
    ui: Vec<ReferenceUiState>,
    #[serde(default)]
    stores: Vec<ReferenceStoreState>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReferenceUiState {
    scope: String,
    value: serde_json::Value,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReferenceStoreState {
    object: String,
    id: String,
    value: serde_json::Value,
}

fn seed_reference_state(host: &DeterministicHost, path: &std::path::Path) -> Result<()> {
    let fixture: ReferenceStateFixture = serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("AXIOM_EXTENSION_REFERENCE_HOST: parse {}", path.display()))?;
    let mut schemas = std::collections::BTreeMap::<String, Value>::new();
    for ui in fixture.ui {
        host.state_store()
            .define_ui(&ui.scope, json_to_value(ui.value)?, Vec::new())?;
    }
    for store in &fixture.stores {
        let value = json_to_value(store.value.clone())?;
        schemas.entry(store.object.clone()).or_insert(value);
    }
    for (object, schema) in schemas {
        host.state_store()
            .define_store(&object, schema, Vec::new())?;
    }
    for store in fixture.stores {
        host.state_store().put_store_object(
            &store.object,
            &store.id,
            json_to_value(store.value)?,
        )?;
    }
    Ok(())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn json_to_value(value: serde_json::Value) -> Result<Value> {
    Ok(match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(value) => Value::Bool(value),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_u64() {
                Value::Unsigned(value)
            } else if let Some(value) = value.as_i64() {
                Value::Signed(value)
            } else {
                bail!("AXIOM_EXTENSION_REFERENCE_HOST: ABI v1 does not accept floating-point JSON values")
            }
        }
        serde_json::Value::String(value) => Value::String(value),
        serde_json::Value::Array(values) => Value::List(
            values
                .into_iter()
                .map(json_to_value)
                .collect::<Result<_>>()?,
        ),
        serde_json::Value::Object(values) => Value::Record(
            values
                .into_iter()
                .map(|(name, value)| {
                    Ok(Field {
                        name,
                        value: json_to_value(value)?,
                    })
                })
                .collect::<Result<_>>()?,
        ),
    })
}

fn value_to_json(value: Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(value) => value.into(),
        Value::Signed(value) => value.into(),
        Value::Unsigned(value) => value.into(),
        Value::String(value) => value.into(),
        Value::Bytes(value) => serde_json::json!({"$bytesBase64": BASE64.encode(value)}),
        Value::List(values) => {
            serde_json::Value::Array(values.into_iter().map(value_to_json).collect())
        }
        Value::Record(fields) => serde_json::Value::Object(
            fields
                .into_iter()
                .map(|field| (field.name, value_to_json(field.value)))
                .collect(),
        ),
        Value::Variant { case, value } => {
            serde_json::json!({"$variant": case, "value": value.map(|value| value_to_json(*value))})
        }
        Value::Handle(handle) => {
            serde_json::json!({"$handle": {"id": handle.id, "generation": handle.generation, "kind": format!("{:?}", handle.kind)}})
        }
    }
}

fn parse_target(value: &str) -> Result<PackageTarget> {
    match value {
        "android" => Ok(PackageTarget::Android),
        "ios" => Ok(PackageTarget::Ios),
        "web" => Ok(PackageTarget::Web),
        "server" => Ok(PackageTarget::Server),
        _ => bail!("target must be android, ios, web, or server"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_names_are_closed() {
        assert_eq!(parse_target("server").unwrap(), PackageTarget::Server);
        assert!(parse_target("native").is_err());
    }
}
