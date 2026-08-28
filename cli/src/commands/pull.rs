// axiom-cli/src/commands/pull.rs

use anyhow::{Context, Result};
use axiom_cloud::{
    contract::{self, PullContractError, PulledContract},
    CloudClient,
};
use dialoguer::Confirm;
use serde_json::Value;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table};

use axiom_build::core::utils::{ensure_deps, generate_from_fbs};

// ==========================================
// TYPES
// ==========================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Framework {
    Flutter,
    Dart,
    AtmxWeb,
    AtmxReact,
}

impl Framework {
    pub fn as_str(&self) -> &'static str {
        match self {
            Framework::Flutter => "flutter",
            Framework::Dart => "dart",
            Framework::AtmxWeb => "atmx-web",
            Framework::AtmxReact => "atmx-react",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "flutter" => Some(Framework::Flutter),
            "dart" => Some(Framework::Dart),
            "atmx-web" | "atmxweb" => Some(Framework::AtmxWeb),
            "atmx-react" | "atmxreact" => Some(Framework::AtmxReact),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContractEntry {
    pub name: String,
    pub source: Option<String>,
    pub version: Option<String>,
    /// The cloud proof for the exact installed artifact. Both values are
    /// public verification material; neither is a credential.
    pub signature: Option<String>,
    pub public_key: Option<String>,
}

#[derive(Debug)]
pub struct AxiomDeps {
    pub framework: Framework,
    pub contracts: Vec<ContractEntry>,
}

// ==========================================
// ENTRY POINT
// ==========================================

pub async fn handle_pull(
    source: Option<String>,
    contract: Option<String>,
    contract_config: Option<PathBuf>,
    framework_flag: Option<String>,
    name_flag: Option<String>,
    out_flag: Option<String>,
) -> Result<()> {
    let project_root = std::env::current_dir()?;
    let deps_path = project_root.join("AxiomDeps.toml");

    // 1. Determine framework
    let framework = resolve_framework(&project_root, framework_flag.as_deref(), &deps_path)?;

    // 2. Determine output directory
    let out_dir = resolve_out_dir(&framework, out_flag)?;

    // 3. Build contract list
    let contracts = resolve_contracts(
        source.as_deref(),
        contract.as_deref(),
        contract_config.as_deref(),
        name_flag.as_deref(),
        &deps_path,
    )?;

    // 4. Fetch every contract before changing the project's dependency
    // manifest. A failed remote authorization or lookup must not leave a
    // newly-created AxiomDeps.toml pointing at an unusable dependency.
    let mut installed_contracts = Vec::with_capacity(contracts.len());
    for mut entry in contracts {
        let installed = install_contract(&entry).await?;
        match installed.proof {
            Some(proof) => {
                entry.signature = Some(proof.signature);
                entry.public_key = Some(proof.public_key);
            }
            None => {
                // A local source or an older API response has no cryptographic
                // proof. Never preserve proof material from a previous pull.
                entry.signature = None;
                entry.public_key = None;
            }
        }
        installed_contracts.push((entry, installed.path));
    }

    // 5. Persist the dependency manifest only after every source was
    // successfully resolved.
    let axiom_deps = AxiomDeps {
        framework: framework.clone(),
        contracts: installed_contracts
            .iter()
            .map(|(entry, _)| entry.clone())
            .collect(),
    };
    write_axiom_deps(&deps_path, &axiom_deps)?;
    println!("📝 AxiomDeps.toml written → {}", deps_path.display());

    // 6. Generate the selected framework bindings from the verified local
    // contract copies.
    for (entry, installed_path) in installed_contracts {
        run_codegen(&project_root, &framework, &installed_path, &entry, &out_dir).await?;
    }

    println!("\n✅ axiom pull finished successfully.");
    Ok(())
}

// ==========================================
// OUT DIR RESOLUTION
// ==========================================

fn resolve_out_dir(framework: &Framework, out_flag: Option<String>) -> Result<String> {
    if let Some(o) = out_flag {
        return Ok(o);
    }

    let default_dir = match framework {
        Framework::Flutter | Framework::Dart => "lib/axiom_generated",
        Framework::AtmxWeb | Framework::AtmxReact => "src/generated",
    };

    print!("📁 Output directory [{}]: ", default_dir);
    io::stdout().flush()?;

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() || input.trim().is_empty() {
        Ok(default_dir.to_string())
    } else {
        Ok(input.trim().to_string())
    }
}

// ==========================================
// FRAMEWORK RESOLUTION
// ==========================================

fn resolve_framework(
    project_root: &Path,
    framework_flag: Option<&str>,
    deps_path: &Path,
) -> Result<Framework> {
    if let Some(f) = framework_flag {
        return Framework::from_str(f).with_context(|| {
            format!(
                "Unknown framework '{}'. Valid: flutter, dart, atmx-web, atmx-react",
                f
            )
        });
    }

    if deps_path.exists() {
        if let Ok(existing) = read_framework_from_deps(deps_path) {
            println!(
                "📖 Using framework from AxiomDeps.toml: {}",
                existing.as_str()
            );
            return Ok(existing);
        }
    }

    let detected = detect_framework(project_root);
    let detected_str = detected.as_ref().map(|f| f.as_str()).unwrap_or("atmx-web");
    prompt_framework_confirm(detected_str)
}

fn detect_framework(project_root: &Path) -> Option<Framework> {
    let pubspec = project_root.join("pubspec.yaml");
    if pubspec.exists() {
        if let Ok(content) = fs::read_to_string(&pubspec) {
            if pubspec_has_flutter_dep(&content) {
                return Some(Framework::Flutter);
            }
            return Some(Framework::Dart);
        }
    }

    let pkg_json = project_root.join("package.json");
    if pkg_json.exists() {
        if let Ok(content) = fs::read_to_string(&pkg_json) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let all_deps = merge_json_deps(&json);
                if all_deps
                    .iter()
                    .any(|d| d == "atmx-react" || d == "@axiomcore/react" || d == "react")
                {
                    return Some(Framework::AtmxReact);
                }
                // Any JavaScript/TypeScript workspace default to atmx-web
                return Some(Framework::AtmxWeb);
            }
            return Some(Framework::AtmxWeb);
        }
    }
    None
}

fn pubspec_has_flutter_dep(content: &str) -> bool {
    let mut in_deps = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "dependencies:" {
            in_deps = true;
            continue;
        }
        if in_deps {
            if !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.is_empty() {
                in_deps = false;
                continue;
            }
            if trimmed.starts_with("flutter:") {
                return true;
            }
        }
    }
    false
}

fn merge_json_deps(json: &serde_json::Value) -> Vec<String> {
    let mut names = Vec::new();
    for key in &["dependencies", "devDependencies"] {
        if let Some(obj) = json[key].as_object() {
            names.extend(obj.keys().cloned());
        }
    }
    names
}

fn prompt_framework_confirm(detected: &str) -> Result<Framework> {
    print!("🔍 Detected framework: {} (press Enter to confirm, or type override [flutter/dart/atmx-web/atmx-react]): ", detected);
    io::stdout().flush()?;

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() || input.trim().is_empty() {
        Framework::from_str(detected)
            .with_context(|| format!("Could not recognise '{}'.", detected))
    } else {
        Framework::from_str(input.trim()).with_context(|| {
            format!(
                "Unknown framework '{}'. Valid: flutter, dart, atmx-web, atmx-react",
                input.trim()
            )
        })
    }
}

// ==========================================
// CONTRACT RESOLUTION
// ==========================================

fn resolve_contracts(
    source: Option<&str>,
    contract: Option<&str>,
    contract_config: Option<&Path>,
    name_flag: Option<&str>,
    deps_path: &Path,
) -> Result<Vec<ContractEntry>> {
    if source.is_some() && contract.is_some() {
        anyhow::bail!("pass a pull source either positionally or with --contract, not both");
    }
    if (source.is_some() || contract.is_some()) && contract_config.is_some() {
        anyhow::bail!("--contract-config cannot be combined with a contract source");
    }

    let source = source.or(contract);
    if source.is_none() && contract_config.is_none() {
        if deps_path.exists() {
            let entries = read_contracts_from_deps(deps_path)?;
            if !entries.is_empty() {
                println!("📖 Re-pulling contracts from existing AxiomDeps.toml");
                return Ok(entries);
            }
        }
        anyhow::bail!("No contract specified and no AxiomDeps.toml found. Use `axiom pull <artifact|config|URL|organization/project>`.");
    }

    let mut entries = Vec::new();

    if let Some(cfg_path) = contract_config {
        entries.extend(load_contracts_from_config(cfg_path)?);
    } else if let Some(value) = source {
        let path = Path::new(value);
        if path.exists() {
            if is_contract_config_path(path) {
                entries.extend(load_contracts_from_config(path)?);
            } else {
                let abs = canonicalize_or_absolute(path)?;
                let name = resolve_single_contract_name(&abs, name_flag)?;
                entries.push(ContractEntry {
                    name,
                    source: Some(abs.to_string_lossy().to_string()),
                    version: None,
                    signature: None,
                    public_key: None,
                });
            }
        } else {
            let reference = contract::parse_pull_reference(value).with_context(|| {
                "pull source is neither an existing local file nor a valid AxiomCore contract reference"
            })?;
            let name = name_flag
                .map(slugify)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| reference.project_slug.clone());
            entries.push(ContractEntry {
                name,
                source: Some(reference.download_url()),
                version: reference.version,
                signature: None,
                public_key: None,
            });
        }
    }

    Ok(entries)
}

fn resolve_single_contract_name(path: &Path, name_flag: Option<&str>) -> Result<String> {
    if let Some(n) = name_flag {
        return Ok(slugify(n));
    }

    let inferred = path
        .file_stem()
        .map(|s| slugify(&s.to_string_lossy()))
        .filter(|s| !s.is_empty() && s != ".")
        .unwrap_or_else(|| "default".to_string());

    print!(
        "📄 Contract name [{}] (press Enter to confirm, or type a new name): ",
        inferred
    );
    io::stdout().flush()?;
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() || input.trim().is_empty() {
        Ok(inferred)
    } else {
        Ok(slugify(input.trim()))
    }
}

fn is_contract_config_path(path: &Path) -> bool {
    path.file_name().and_then(|value| value.to_str()) == Some("AxiomDeps.toml")
        || path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("toml") || extension.eq_ignore_ascii_case("json")
            })
}

fn load_contracts_from_config(cfg_path: &Path) -> Result<Vec<ContractEntry>> {
    if cfg_path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
        || cfg_path.file_name().and_then(|value| value.to_str()) == Some("AxiomDeps.toml")
    {
        return read_contracts_from_deps(cfg_path);
    }
    load_contracts_from_json_config(cfg_path)
}

fn load_contracts_from_json_config(cfg_path: &Path) -> Result<Vec<ContractEntry>> {
    let content = fs::read_to_string(cfg_path)
        .with_context(|| format!("Cannot read contract config: {}", cfg_path.display()))?;
    let parsed: serde_json::Value = serde_json::from_str(&content)?;
    let contracts_obj = parsed["contracts"]
        .as_object()
        .with_context(|| "contract-config JSON must have a top-level 'contracts' object")?;

    let mut entries = Vec::new();
    for (raw_name, val) in contracts_obj {
        let name = slugify(raw_name);
        let source = val["file"].as_str().map(|s| s.to_string());
        let version = val["version"].as_str().map(|s| s.to_string());
        entries.push(ContractEntry {
            name,
            source,
            version,
            signature: None,
            public_key: None,
        });
    }
    Ok(entries)
}

fn canonicalize_or_absolute(p: &Path) -> Result<PathBuf> {
    if let Ok(canon) = p.canonicalize() {
        Ok(canon)
    } else if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        let joined = std::env::current_dir()?.join(p);
        Ok(joined.canonicalize().unwrap_or(joined))
    }
}

// ==========================================
// CONTRACT INSTALL → ~/.axiom/contracts/
// ==========================================

struct InstalledContract {
    path: PathBuf,
    proof: Option<contract::ContractProof>,
}

async fn install_contract(entry: &ContractEntry) -> Result<InstalledContract> {
    let install_dir = contract_install_dir(&entry.name)?;
    fs::create_dir_all(&install_dir)?;
    let dest = install_dir.join("contract.axiom");
    let legacy_proof = contract_proof_path(&dest);

    let proof = if let Some(ref src_str) = entry.source {
        if is_remote_contract_source(src_str) {
            let reference = contract::parse_pull_reference(src_str)?
                .with_version_override(entry.version.as_deref())?;
            let downloaded = download_remote_contract(&reference).await?;
            write_downloaded_contract(&dest, &downloaded.bytes)?;
            println!(
                "☁️  Pulled '{}' from {}",
                entry.name,
                reference.download_url()
            );
            downloaded.proof
        } else {
            let src = PathBuf::from(src_str);
            if !src.exists() {
                anyhow::bail!(
                    "Source file for contract '{}' not found: {}",
                    entry.name,
                    src.display()
                );
            }
            fs::copy(&src, &dest).with_context(|| {
                format!(
                    "Failed to install contract '{}': {} → {}",
                    entry.name,
                    src.display(),
                    dest.display()
                )
            })?;
            None
        }
    } else if dest.exists() {
        println!(
            "✓  Contract '{}' already installed at {}",
            entry.name,
            dest.display()
        );
        match (&entry.signature, &entry.public_key) {
            (Some(signature), Some(public_key)) => Some(contract::ContractProof {
                signature: signature.clone(),
                public_key: public_key.clone(),
            }),
            _ => None,
        }
    } else {
        anyhow::bail!("Contract '{}' has no source and is not installed at {}. Re-run with --contract <path>.", entry.name, dest.display());
    };

    // Versions before the TOML proof format emitted this temporary file. It
    // contains public material only, but remove it once so AxiomDeps.toml is
    // the single source of dependency and verification metadata.
    remove_contract_proof(&legacy_proof)?;
    if entry.source.is_some() {
        println!("📥 Installed '{}' → {}", entry.name, dest.display());
    }

    Ok(InstalledContract { path: dest, proof })
}

fn is_remote_contract_source(source: &str) -> bool {
    source.trim_start().starts_with("http://") || source.trim_start().starts_with("https://")
}

async fn download_remote_contract(reference: &contract::PullReference) -> Result<PulledContract> {
    loop {
        // Keep the production and loopback CLI profiles hermetic. A user can
        // paste a share URL while running `laxiom`, but their local bearer must
        // never be sent to production (or vice versa). Public references still
        // work anonymously across profiles.
        let active_base = axiom_cloud::cloud_base_url()?;
        let profile_matches_reference =
            active_base.trim_end_matches('/') == reference.base_url.trim_end_matches('/');
        let authenticated = if profile_matches_reference {
            crate::auth_store::authenticated_cloud_client().ok()
        } else {
            None
        };
        let result = match authenticated.as_ref() {
            Some(client) => client.pull_contract_reference(reference).await,
            None => CloudClient::pull_contract_reference_anonymous(reference).await,
        };

        match result {
            Ok(data) => return Ok(data),
            Err(error) => match error.downcast_ref::<PullContractError>() {
                Some(PullContractError::AuthenticationRequired) => {
                    anyhow::bail!(
                        "This contract is private. Run `axiom login`, then retry `axiom pull {}`.",
                        reference.download_url()
                    );
                }
                Some(PullContractError::AccessDenied { email }) => {
                    eprintln!("{email} does not have access to this account.");
                    let switch_account = Confirm::new()
                        .with_prompt("Switch account and sign in again?")
                        .default(false)
                        .interact()
                        .context("could not read the account-switch selection")?;
                    if !switch_account {
                        anyhow::bail!(
                            "Contract pull cancelled. No private contract was downloaded."
                        );
                    }
                    login_for_contract_pull().await?;
                }
                Some(PullContractError::NotFound) => {
                    if authenticated.is_none() {
                        if !profile_matches_reference {
                            anyhow::bail!(
						"This reference belongs to a different control-plane profile. Use `laxiom` for a loopback private project or `axiom` for a production private project."
					);
                        }
                        anyhow::bail!(
                            "Contract project or requested version was not found. It may be private; run `axiom login` and retry if you expect access."
                        );
                    }
                    if reference.version.is_none() {
                        anyhow::bail!(
                            "The project is unavailable, or it has no contract promoted to latest yet. A release reaches latest only after its required verification, semantic, test, and delivery-policy checks pass."
                        );
                    }
                    anyhow::bail!("Contract project or requested version was not found.");
                }
                Some(PullContractError::RateLimited) => {
                    anyhow::bail!("Contract pull is rate limited. Please wait briefly and retry.");
                }
                _ => return Err(error),
            },
        }
    }
}

async fn login_for_contract_pull() -> Result<()> {
    println!("Starting AxiomCore device login for the selected account...");
    let serialized = CloudClient::login().await?;
    let token: Value = serde_json::from_str(&serialized)
        .context("AxiomCore returned an invalid device-login session")?;
    let access_token = token["access_token"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .context("AxiomCore returned an incomplete device-login session")?;
    let refresh_token = token["refresh_token"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .context("AxiomCore returned an incomplete device-login session")?;
    let expires_in = token["expires_in"]
        .as_u64()
        .context("AxiomCore returned an invalid device-login expiry")?;
    crate::auth_store::save_tokens(access_token, refresh_token, expires_in)?;
    println!("✅ Signed in. Rechecking project access...");
    Ok(())
}

fn write_downloaded_contract(destination: &Path, data: &[u8]) -> Result<()> {
    let parent = destination
        .parent()
        .context("download destination has no parent directory")?;
    let temporary = parent.join(format!(".contract.{}.tmp", std::process::id()));
    fs::write(&temporary, data).context("failed to write downloaded contract")?;
    fs::rename(&temporary, destination).context("failed to install downloaded contract")?;
    Ok(())
}

fn contract_proof_path(contract_path: &Path) -> PathBuf {
    let file_name = contract_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("contract.axiom");
    contract_path.with_file_name(format!("{file_name}.proof.json"))
}

fn remove_contract_proof(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to remove stale proof: {}", path.display()))
        }
    }
}

fn contract_install_dir(name: &str) -> Result<PathBuf> {
    let home = dirs::home_dir()
        .with_context(|| "Cannot determine home directory for ~/.axiom/contracts")?;
    Ok(home.join(".axiom").join("contracts").join(name))
}

// ==========================================
// CODEGEN DISPATCH
// ==========================================

async fn run_codegen(
    project_root: &Path,
    framework: &Framework,
    installed_contract: &Path,
    entry: &ContractEntry,
    out_dir: &str,
) -> Result<()> {
    println!(
        "\n⚙️  Generating '{}' SDK for {}...",
        entry.name,
        framework.as_str()
    );

    match framework {
        Framework::Flutter => {
            run_codegen_flutter(project_root, installed_contract, entry, out_dir).await?
        }
        Framework::Dart => {
            run_codegen_dart(project_root, installed_contract, entry, out_dir).await?
        }
        Framework::AtmxWeb => {
            run_codegen_atmx(project_root, installed_contract, entry, out_dir, false).await?
        }
        Framework::AtmxReact => {
            run_codegen_atmx(project_root, installed_contract, entry, out_dir, true).await?
        }
    }
    Ok(())
}

async fn run_codegen_flutter(
    project_root: &Path,
    installed_contract: &Path,
    entry: &ContractEntry,
    out_dir: &str,
) -> Result<()> {
    let asset_dir = project_root.join("assets").join("axiom");
    fs::create_dir_all(&asset_dir)?;
    let asset_file = asset_dir.join(format!("{}.axiom", entry.name));
    fs::copy(installed_contract, &asset_file)?;

    let asset_relative = format!("assets/axiom/{}.axiom", entry.name);
    ensure_deps(project_root, &asset_relative)?;

    let frontend_cfg = axiom_lib::config::FrontendConfig {
        framework: "flutter".to_string(),
        output_dir: Some(out_dir.to_string()),
    };

    let deps_toml_path = project_root.join("AxiomDeps.toml");
    generate_from_fbs(
        project_root,
        &frontend_cfg,
        &[],
        &deps_toml_path.to_string_lossy(),
    )
    .await?;
    Ok(())
}

async fn run_codegen_dart(
    project_root: &Path,
    installed_contract: &Path,
    entry: &ContractEntry,
    out_dir: &str,
) -> Result<()> {
    let frontend_cfg = axiom_lib::config::FrontendConfig {
        framework: "dart".to_string(),
        output_dir: Some(out_dir.to_string()),
    };
    generate_from_fbs(
        project_root,
        &frontend_cfg,
        &[],
        &installed_contract.to_string_lossy(),
    )
    .await?;

    println!("📦 Dart SDK generated → {}", out_dir);
    Ok(())
}

async fn run_codegen_atmx(
    project_root: &Path,
    installed_contract: &Path,
    entry: &ContractEntry,
    out_dir: &str,
    is_react: bool,
) -> Result<()> {
    // 1. Copy to public/
    let public_dir = project_root.join("public");
    fs::create_dir_all(&public_dir)?;
    let public_contract = public_dir.join(format!("{}.axiom", entry.name));
    fs::copy(installed_contract, &public_contract).with_context(|| {
        format!(
            "Failed to copy contract to public/: {}",
            public_contract.display()
        )
    })?;
    println!("📄 Static asset written → public/{}.axiom", entry.name);

    remove_contract_proof(&contract_proof_path(&public_contract))?;

    // 2. Copy to static/axiom/ for Go servers if static folder exists
    let static_axiom_dir = project_root.join("static").join("axiom");
    if project_root.join("static").exists() {
        fs::create_dir_all(&static_axiom_dir)?;
        let _ = fs::copy(installed_contract, static_axiom_dir.join(".axiom"));
        let _ = fs::copy(
            installed_contract,
            static_axiom_dir.join(format!("{}.axiom", entry.name)),
        );
        println!("📄 Static asset written → static/axiom/.axiom");
    }

    // 3. Trigger the project-pinned generator when present. Otherwise use the
    // exact companion CLI version instead of inheriting an arbitrary global
    // `atmx` executable from PATH. This keeps generated SDK syntax compatible
    // with the Axiom CLI that invoked it.
    #[cfg(target_os = "windows")]
    let local_atmx = project_root
        .join("node_modules")
        .join(".bin")
        .join("atmx.cmd");
    #[cfg(not(target_os = "windows"))]
    let local_atmx = project_root.join("node_modules").join(".bin").join("atmx");

    let mut cmd = if local_atmx.is_file() {
        tokio::process::Command::new(&local_atmx)
    } else {
        #[cfg(target_os = "windows")]
        let npx_cmd = "npx.cmd";
        #[cfg(not(target_os = "windows"))]
        let npx_cmd = "npx";
        let mut command = tokio::process::Command::new(npx_cmd);
        command
            .arg("--yes")
            .arg("--package")
            .arg(format!("atmx-cli@{}", env!("CARGO_PKG_VERSION")))
            .arg("atmx");
        command
    };
    cmd.current_dir(project_root)
        .arg("generate")
        .arg("-c")
        .arg("AxiomDeps.toml")
        .arg("-o")
        .arg(out_dir);

    if is_react {
        cmd.arg("--react");
    }

    println!("📦 Running atmx-cli generation...");
    let output = cmd.output().await?;

    if !output.status.success() {
        println!(
            "--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        anyhow::bail!("atmx-cli generation failed");
    }

    println!("📦 ATMX SDK generated → {}", out_dir);
    Ok(())
}

// ==========================================
// AXIOM DEPS TOML I/O
// ==========================================

pub fn write_axiom_deps(path: &Path, deps: &AxiomDeps) -> Result<()> {
    let mut doc: DocumentMut = if path.exists() {
        fs::read_to_string(path)?
            .parse::<DocumentMut>()
            .unwrap_or_default()
    } else {
        DocumentMut::new()
    };

    doc["framework"] = toml_edit::value(deps.framework.as_str());

    if doc.get("contracts").is_none() {
        doc["contracts"] = Item::Table(Table::new());
    }

    let contracts_table = doc["contracts"]
        .as_table_mut()
        .with_context(|| "AxiomDeps.toml: 'contracts' must be a table")?;

    for entry in &deps.contracts {
        let sub = contracts_table
            .entry(&entry.name)
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .with_context(|| {
                format!("AxiomDeps.toml: 'contracts.{}' must be a table", entry.name)
            })?;
        if let Some(ref source) = entry.source {
            sub["source"] = toml_edit::value(source.as_str());
        }
        if let Some(ref version) = entry.version {
            sub["version"] = toml_edit::value(version.as_str());
        } else {
            // A versionless reference intentionally follows the active release
            // channel. Do not leave a previous immutable pin behind when a
            // developer changes `pull org/project/vX` to `pull org/project`.
            sub.remove("version");
        }
        match (&entry.signature, &entry.public_key) {
            (Some(signature), Some(public_key)) => {
                sub["signature"] = toml_edit::value(signature.as_str());
                sub["public_key"] = toml_edit::value(public_key.as_str());
            }
            _ => {
                sub.remove("signature");
                sub.remove("public_key");
            }
        }
    }

    fs::write(path, doc.to_string())?;
    Ok(())
}

fn read_framework_from_deps(path: &Path) -> Result<Framework> {
    let content = fs::read_to_string(path)?;
    let doc = content.parse::<DocumentMut>()?;
    let fw_str = doc["framework"]
        .as_str()
        .with_context(|| "AxiomDeps.toml missing 'framework' key")?;
    Framework::from_str(fw_str)
        .with_context(|| format!("Unknown framework '{}' in AxiomDeps.toml", fw_str))
}

fn read_contracts_from_deps(path: &Path) -> Result<Vec<ContractEntry>> {
    let content = fs::read_to_string(path)?;
    let doc = content.parse::<DocumentMut>()?;
    let contracts = match doc.get("contracts").and_then(|c| c.as_table()) {
        Some(t) => t,
        None => return Ok(vec![]),
    };
    let mut entries = Vec::new();
    for (name, item) in contracts.iter() {
        if let Some(sub) = item.as_table() {
            let source = sub.get("source").and_then(|v| v.as_str()).map(String::from);
            let version = sub
                .get("version")
                .and_then(|v| v.as_str())
                .map(String::from);
            let signature = sub
                .get("signature")
                .and_then(|v| v.as_str())
                .map(String::from);
            let public_key = sub
                .get("public_key")
                .and_then(|v| v.as_str())
                .map(String::from);
            entries.push(ContractEntry {
                name: name.to_string(),
                source,
                version,
                signature,
                public_key,
            });
        }
    }
    Ok(entries)
}

fn slugify(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
