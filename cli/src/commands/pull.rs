// axiom-cli/src/commands/pull.rs

use anyhow::{Context, Result};
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
    contract: Option<PathBuf>,
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
        contract.as_deref(),
        contract_config.as_deref(),
        name_flag.as_deref(),
        &deps_path,
    )?;

    // 4. Write AxiomDeps.toml
    let axiom_deps = AxiomDeps {
        framework: framework.clone(),
        contracts: contracts.clone(),
    };
    write_axiom_deps(&deps_path, &axiom_deps)?;
    println!("📝 AxiomDeps.toml written → {}", deps_path.display());

    // 5. Install each contract to ~/.axiom/contracts/ then run codegen
    for entry in &contracts {
        let installed_path = install_contract(entry)?;
        run_codegen(&project_root, &framework, &installed_path, entry, &out_dir).await?;
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
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        Ok(default_dir.to_string())
    } else {
        Ok(input.to_string())
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
    let detected_str = detected.as_ref().map(|f| f.as_str()).unwrap_or("unknown");
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
                    .any(|d| d == "atmx-react" || d == "@axiomcore/react")
                {
                    return Some(Framework::AtmxReact);
                }
                if all_deps
                    .iter()
                    .any(|d| d == "atmx" || d == "@axiomcore/web")
                    || project_root
                        .join("public")
                        .join("axiom_runtime.wasm")
                        .exists()
                    || project_root.join("public").join(".axiom").exists()
                {
                    return Some(Framework::AtmxWeb);
                }
                if all_deps.iter().any(|d| d == "react")
                    && project_root.join("vite.config.ts").exists()
                {
                    return Some(Framework::AtmxReact);
                }
            }
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
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        Framework::from_str(detected).with_context(|| {
            format!(
                "Could not recognise '{}'. Please pass --framework explicitly.",
                detected
            )
        })
    } else {
        Framework::from_str(input).with_context(|| {
            format!(
                "Unknown framework '{}'. Valid: flutter, dart, atmx-web, atmx-react",
                input
            )
        })
    }
}

// ==========================================
// CONTRACT RESOLUTION
// ==========================================

fn resolve_contracts(
    contract: Option<&Path>,
    contract_config: Option<&Path>,
    name_flag: Option<&str>,
    deps_path: &Path,
) -> Result<Vec<ContractEntry>> {
    if contract.is_none() && contract_config.is_none() {
        if deps_path.exists() {
            let entries = read_contracts_from_deps(deps_path)?;
            if !entries.is_empty() {
                println!("📖 Re-pulling contracts from existing AxiomDeps.toml");
                return Ok(entries);
            }
        }
        anyhow::bail!("No contract specified and no AxiomDeps.toml found. Use --contract <path> or --contract-config <path>.");
    }

    let mut entries = Vec::new();

    if let Some(cfg_path) = contract_config {
        entries.extend(load_contracts_from_json_config(cfg_path)?);
    } else if let Some(c_path) = contract {
        let abs = canonicalize_or_absolute(c_path)?;
        let name = resolve_single_contract_name(&abs, name_flag)?;
        entries.push(ContractEntry {
            name,
            source: Some(abs.to_string_lossy().to_string()),
            version: None,
        });
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
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    if input.is_empty() {
        Ok(inferred)
    } else {
        Ok(slugify(input))
    }
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
        });
    }
    Ok(entries)
}

fn canonicalize_or_absolute(p: &Path) -> Result<PathBuf> {
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(p))
    }
}

// ==========================================
// CONTRACT INSTALL → ~/.axiom/contracts/
// ==========================================

fn install_contract(entry: &ContractEntry) -> Result<PathBuf> {
    let install_dir = contract_install_dir(&entry.name)?;
    fs::create_dir_all(&install_dir)?;
    let dest = install_dir.join("contract.axiom");

    if let Some(ref src_str) = entry.source {
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
        println!("📥 Installed '{}' → {}", entry.name, dest.display());
    } else if dest.exists() {
        println!(
            "✓  Contract '{}' already installed at {}",
            entry.name,
            dest.display()
        );
    } else {
        anyhow::bail!("Contract '{}' has no source and is not installed at {}. Re-run with --contract <path>.", entry.name, dest.display());
    }

    Ok(dest)
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

/// A dedicated execution pipeline for ATMX frameworks (bypasses `generate_from_fbs` and `dart pub get`)
async fn run_codegen_atmx(
    project_root: &Path,
    installed_contract: &Path,
    entry: &ContractEntry,
    out_dir: &str,
    is_react: bool,
) -> Result<()> {
    let public_dir = project_root.join("public");
    fs::create_dir_all(&public_dir)?;
    let public_contract = public_dir.join(format!("{}.axiom", entry.name));
    fs::copy(installed_contract, &public_contract).with_context(|| {
        format!(
            "Failed to copy contract to public/: {}",
            public_contract.display()
        )
    })?;
    println!("📄 Vite static asset → public/{}.axiom", entry.name);

    // ✨ FIX: Removed atmx.config.json generation entirely!

    // 2. Trigger `npx atmx generate` using AxiomDeps.toml
    #[cfg(target_os = "windows")]
    let npx_cmd = "npx.cmd";
    #[cfg(not(target_os = "windows"))]
    let npx_cmd = "npx";

    let mut cmd = tokio::process::Command::new(npx_cmd);
    cmd.current_dir(project_root)
        .arg("atmx")
        .arg("generate")
        .arg("-c")
        .arg("AxiomDeps.toml") // ✨ FIX: Read TOML directly
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
            entries.push(ContractEntry {
                name: name.to_string(),
                source,
                version,
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
