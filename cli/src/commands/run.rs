use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::SystemTime,
};

use anyhow::{bail, Context, Result};
use axiom_lib::{
    extension_source::resolve_extension_sources,
    ui_contract::{
        read_manifest, resolve_manifest, verify_lock, write_lock, UiContractLock, UI_LOCK_FORMAT,
    },
};
use clap::ValueEnum;
use regex::Regex;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunMode {
    Service,
    Mock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcoreSourceKind {
    Frontend,
    Backend,
}

#[derive(Debug, Deserialize)]
struct DevelopmentApplication {
    name: String,
    #[serde(default = "default_application_version")]
    version: String,
}

#[derive(Debug, Default, Deserialize)]
struct DevelopmentManifestMetadata {
    application: Option<DevelopmentApplication>,
}

fn default_application_version() -> String {
    "0.1.0".into()
}

pub fn classify_acore_source(source: &Path) -> Result<AcoreSourceKind> {
    if source.extension().and_then(|value| value.to_str()) != Some("acore") {
        bail!(
            "`axiom run` expected an .acore source or .axiomapp package, got {}",
            source.display()
        );
    }
    let text = fs::read_to_string(source)
        .with_context(|| format!("read Acore source {}", source.display()))?;
    let module = Regex::new(r"(?m)^\s*module\s+[A-Za-z_][A-Za-z0-9_.]*\s*$")?;
    let ui_declaration = Regex::new(r"(?m)^\s*(app|page|component)\s+[A-Za-z_]")?;
    if module.is_match(&text) && ui_declaration.is_match(&text) {
        Ok(AcoreSourceKind::Frontend)
    } else {
        Ok(AcoreSourceKind::Backend)
    }
}

pub async fn handle_backend(
    source: PathBuf,
    mode: RunMode,
    host: String,
    port: u16,
    debug: bool,
) -> Result<()> {
    match mode {
        RunMode::Mock => crate::commands::serve::handle_serve(Some(source), port, debug).await,
        RunMode::Service => run_declared_service(&source, &host, port, debug).await,
    }
}

async fn run_declared_service(source: &Path, host: &str, port: u16, debug: bool) -> Result<()> {
    let canonical = fs::canonicalize(source)
        .with_context(|| format!("resolve backend source {}", source.display()))?;
    let source_text = canonical
        .to_str()
        .context("backend source path is not valid UTF-8")?;
    let config = axiom_extractor::evaluate_acore_config(source_text, Some("default"))?;
    let Some(backend) = config.backend else {
        println!(
            "Starting Acore service on {host}:{port}. This release uses the deterministic Acore runtime shared with mock mode; the service boundary is preserved for future runtime differentiation."
        );
        return crate::commands::serve::handle_serve(Some(canonical), port, debug).await;
    };
    let root = canonical
        .parent()
        .context("backend source has no containing directory")?;
    let language = backend.language.trim().to_ascii_lowercase();
    let mut command = match language.as_str() {
        "python" => python_service_command(&backend.entrypoint, host, port)?,
        "go" => go_service_command(&backend.entrypoint, host, port)?,
        other => bail!(
            "service mode does not yet know how to launch backend language `{other}`; run the service directly or use `--mode mock`"
        ),
    };
    command
        .current_dir(root)
        .env("HOST", host)
        .env("PORT", port.to_string())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    println!(
        "Starting {} backend service `{}` on {}:{}...",
        backend.language, backend.entrypoint, host, port
    );
    let status = command
        .status()
        .await
        .with_context(|| format!("launch {} backend service", backend.language))?;
    if !status.success() {
        bail!("backend service exited with status {status}");
    }
    Ok(())
}

fn python_service_command(entrypoint: &str, host: &str, port: u16) -> Result<Command> {
    let (module, symbol) = entrypoint.split_once(':').with_context(|| {
        format!("Python backend entrypoint `{entrypoint}` must use file.py:application syntax")
    })?;
    let module = module
        .trim_end_matches(".py")
        .trim_start_matches("./")
        .replace(['/', '\\'], ".");
    let mut command = Command::new("python3");
    command.args([
        "-m",
        "uvicorn",
        &format!("{module}:{symbol}"),
        "--host",
        host,
        "--port",
        &port.to_string(),
    ]);
    Ok(command)
}

fn go_service_command(entrypoint: &str, _host: &str, _port: u16) -> Result<Command> {
    if entrypoint.trim().is_empty() {
        bail!("Go backend entrypoint cannot be empty");
    }
    let mut command = Command::new("go");
    command.args(["run", entrypoint]);
    Ok(command)
}

pub async fn prepare_frontend(
    source: &Path,
    explicit_lock: Option<&Path>,
    frozen: bool,
) -> Result<PathBuf> {
    let source = fs::canonicalize(source)
        .with_context(|| format!("resolve frontend source {}", source.display()))?;
    let root = source
        .parent()
        .context("frontend source has no containing directory")?;

    if let Some(lock) = explicit_lock {
        let lock = if lock.is_absolute() {
            lock.to_path_buf()
        } else {
            std::env::current_dir()?.join(lock)
        };
        if !lock.is_file() {
            bail!("explicit UI lock does not exist: {}", lock.display());
        }
        prepare_extensions(root, &source, frozen).await?;
        return Ok(lock);
    }

    let development_root = root.join(".axiom").join("dev");
    fs::create_dir_all(&development_root)?;
    let lock = if frozen {
        root.join("axiom.ui.lock.json")
    } else {
        development_root.join("axiom.ui.lock.json")
    };
    let contract_manifest = discover_contract_manifest(root)?;

    match contract_manifest {
        Some(manifest) if frozen => {
            if !lock.is_file() {
                bail!(
                    "frozen frontend run requires the reviewed lock {}; run without --frozen to prepare local development dependencies",
                    lock.display()
                );
            }
            let locked = axiom_lib::ui_contract::read_lock(&lock)?;
            verify_lock(&manifest, &locked)?;
        }
        Some(manifest) => {
            prepare_local_contracts(&manifest).await?;
            let mut resolved = resolve_manifest(&manifest)?;
            stage_development_contracts(&manifest, &lock, &mut resolved)?;
            write_lock(&lock, &resolved)?;
            println!(
                "Prepared {} local contract alias(es).",
                resolved.contracts.len()
            );
        }
        None if frozen => {
            if !lock.is_file() {
                bail!(
                    "frozen frontend run requires {}; no contract manifest or reviewed lock was found",
                    lock.display()
                );
            }
        }
        None => {
            write_lock(
                &lock,
                &UiContractLock {
                    format: UI_LOCK_FORMAT.into(),
                    contracts: BTreeMap::new(),
                },
            )?;
        }
    }

    prepare_extensions(root, &source, frozen).await?;
    Ok(lock)
}

/// Runtime lock inputs are deliberately confined beneath the lock directory.
/// A development manifest may reference a sibling backend with `..`, so copy
/// the already-resolved and verified bytes into the compiler-owned development
/// directory and make the generated lock point at that safe snapshot.
fn stage_development_contracts(
    manifest: &Path,
    lock: &Path,
    resolved: &mut UiContractLock,
) -> Result<()> {
    let manifest_root = manifest
        .parent()
        .context("contract manifest has no containing directory")?;
    let lock_root = lock
        .parent()
        .context("development UI lock has no containing directory")?;
    let staged_root = lock_root.join("contracts");
    fs::create_dir_all(&staged_root)?;

    for (alias, contract) in &mut resolved.contracts {
        if !alias
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        {
            bail!("contract alias `{alias}` cannot be used as a development artifact name");
        }
        let source = manifest_root.join(&contract.artifact);
        let bytes = fs::read(&source)
            .with_context(|| format!("read resolved contract artifact {}", source.display()))?;
        let relative = PathBuf::from("contracts").join(format!("{alias}.axiom"));
        let staged = lock_root.join(&relative);
        let needs_write = fs::read(&staged)
            .map(|existing| existing != bytes)
            .unwrap_or(true);
        if needs_write {
            fs::write(&staged, &bytes).with_context(|| {
                format!("stage development contract artifact {}", staged.display())
            })?;
        }
        contract.artifact = relative;
    }
    Ok(())
}

pub fn is_managed_development_lock(lock: &Path) -> bool {
    lock.ends_with(Path::new(".axiom/dev/axiom.ui.lock.json"))
}

pub fn frontend_dependency_paths(source: &Path) -> Result<Vec<PathBuf>> {
    let source = fs::canonicalize(source)
        .with_context(|| format!("resolve frontend source {}", source.display()))?;
    let root = source
        .parent()
        .context("frontend source has no containing directory")?;
    let mut paths = Vec::new();
    if let Some(manifest) = discover_contract_manifest(root)? {
        let parsed = read_manifest(&manifest)?;
        paths.push(manifest.clone());
        let manifest_root = manifest
            .parent()
            .context("contract manifest has no containing directory")?;
        for reference in parsed.contracts.values() {
            let artifact = manifest_root.join(&reference.artifact);
            paths.push(artifact.clone());
            let local_source = local_contract_source(&artifact);
            if local_source.exists() {
                paths.push(local_source);
            }
        }
    }
    let deps = root.join("AxiomDeps.toml");
    if deps.is_file() && manifest_has_extensions(&deps)? {
        paths.push(deps.clone());
        for extension in resolve_extension_sources(&deps)?.values() {
            paths.push(extension.source.clone());
            paths.extend(extension.shared_sources.iter().cloned());
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn discover_contract_manifest(root: &Path) -> Result<Option<PathBuf>> {
    let dedicated = root.join("AxiomContracts.toml");
    if dedicated.is_file() {
        return Ok(Some(dedicated));
    }
    let shared = root.join("AxiomDeps.toml");
    if !shared.is_file() {
        return Ok(None);
    }
    let value: toml::Value = toml::from_str(&fs::read_to_string(&shared)?)?;
    Ok(value.get("contracts").map(|_| shared))
}

async fn prepare_local_contracts(manifest: &Path) -> Result<()> {
    let parsed = read_manifest(manifest)?;
    let root = manifest
        .parent()
        .context("contract manifest has no containing directory")?;
    for (alias, reference) in parsed.contracts {
        if reference.signature.is_some() {
            continue;
        }
        let artifact = root.join(&reference.artifact);
        let source = local_contract_source(&artifact);
        if !source.is_file() {
            if artifact.is_file() {
                continue;
            }
            bail!(
                "local contract `{alias}` is missing {} and no sibling Acore source exists at {}",
                artifact.display(),
                source.display()
            );
        }
        if !artifact.is_file()
            || fs::metadata(&artifact)?.len() == 0
            || newer_than(&source, &artifact)?
        {
            build_local_contract(&source, &artifact).await?;
            println!("Built local contract `{alias}` from {}.", source.display());
        }
    }
    Ok(())
}

fn local_contract_source(artifact: &Path) -> PathBuf {
    if artifact.file_name().and_then(|value| value.to_str()) == Some("axiom.axiom") {
        artifact.with_file_name("axiom.acore")
    } else {
        artifact.with_extension("acore")
    }
}

fn newer_than(source: &Path, artifact: &Path) -> Result<bool> {
    let source_time = fs::metadata(source)?
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let artifact_time = fs::metadata(artifact)?
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH);
    Ok(source_time > artifact_time)
}

async fn build_local_contract(source: &Path, artifact: &Path) -> Result<()> {
    let source = fs::canonicalize(source)?;
    let root = source.parent().context("contract source has no parent")?;
    let original = std::env::current_dir()?;
    std::env::set_current_dir(root)?;
    let source_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .context("contract source filename is not valid UTF-8")?;
    let result = axiom_build::core::build::handle_build(source_name, "default", "", "", None).await;
    std::env::set_current_dir(&original)?;
    let generated = root.join(result?);
    let artifact = normalized_destination(artifact)?;
    if generated != artifact {
        if let Some(parent) = artifact.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&generated, &artifact)?;
    }
    Ok(())
}

fn normalized_destination(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .context("contract artifact has no containing directory")?;
    let parent = fs::canonicalize(parent)
        .with_context(|| format!("resolve contract artifact directory {}", parent.display()))?;
    Ok(parent.join(
        path.file_name()
            .context("contract artifact has no filename")?,
    ))
}

async fn prepare_extensions(root: &Path, source: &Path, frozen: bool) -> Result<()> {
    let deps = root.join("AxiomDeps.toml");
    if !deps.is_file() || !manifest_has_extensions(&deps)? {
        return Ok(());
    }
    let workflow = root.join("AxiomExtensions.toml");
    if frozen {
        if !workflow.is_file() {
            bail!(
                "frozen frontend run imports sandbox extensions but {} is missing",
                workflow.display()
            );
        }
        return crate::commands::extensions::handle_verify(workflow).await;
    }
    let extensions = resolve_extension_sources(&deps)?;
    let application = development_application(&deps, source)?;
    for alias in extensions.keys() {
        crate::commands::extensions::handle_source_release(
            deps.clone(),
            alias.clone(),
            PathBuf::from(".axiom/extensions"),
            PathBuf::from("AxiomExtensions.toml"),
            application.name.clone(),
            application.version.clone(),
            false,
            false,
        )
        .await?;
    }
    if !extensions.is_empty() {
        println!(
            "Prepared and verified {} sandbox extension(s).",
            extensions.len()
        );
    }
    Ok(())
}

fn manifest_has_extensions(path: &Path) -> Result<bool> {
    let value: toml::Value = toml::from_str(&fs::read_to_string(path)?)?;
    Ok(value.get("extensions").is_some())
}

fn development_application(deps: &Path, source: &Path) -> Result<DevelopmentApplication> {
    let metadata: DevelopmentManifestMetadata = toml::from_str(&fs::read_to_string(deps)?)?;
    if let Some(application) = metadata.application {
        return Ok(application);
    }
    let text = fs::read_to_string(source)?;
    let module = Regex::new(r"(?m)^\s*module\s+([A-Za-z_][A-Za-z0-9_.]*)\s*$")?
        .captures(&text)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().trim_end_matches(".ui").to_string())
        .unwrap_or_else(|| "local.acore.application".into());
    Ok(DevelopmentApplication {
        name: module,
        version: default_application_version(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_file(name: &str, source: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("axiom-run-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join(name);
        fs::write(&path, source).unwrap();
        path
    }

    #[test]
    fn classifies_frontend_and_backend_acore_sources() {
        let frontend = temporary_file(
            "main.acore",
            "module example.ui\napp Example { route \"/\" => Home }\npage Home { view { Text(\"Hi\") } }",
        );
        let backend = temporary_file(
            "axiom.acore",
            "project { id = \"example\" version = \"v0.1.0\" }",
        );
        assert_eq!(
            classify_acore_source(&frontend).unwrap(),
            AcoreSourceKind::Frontend
        );
        assert_eq!(
            classify_acore_source(&backend).unwrap(),
            AcoreSourceKind::Backend
        );
    }

    #[test]
    fn derives_standard_local_contract_source() {
        assert_eq!(
            local_contract_source(Path::new("../backend/axiom.axiom")),
            PathBuf::from("../backend/axiom.acore")
        );
        assert_eq!(
            local_contract_source(Path::new("contracts/shop.axiom")),
            PathBuf::from("contracts/shop.acore")
        );
    }

    #[test]
    fn normalizes_contract_destinations_before_self_copy_checks() {
        let root = std::env::temp_dir().join(format!("axiom-run-test-{}", uuid::Uuid::new_v4()));
        let frontend = root.join("frontend");
        let backend = root.join("backend");
        fs::create_dir_all(&frontend).unwrap();
        fs::create_dir_all(&backend).unwrap();
        assert_eq!(
            normalized_destination(&frontend.join("../backend/axiom.axiom")).unwrap(),
            fs::canonicalize(backend).unwrap().join("axiom.axiom")
        );
    }
}
