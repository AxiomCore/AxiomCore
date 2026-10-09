//! One offline entry point for the three application contract compilers.
use anyhow::{bail, Context, Result};
use axiom_ui::{UiCompileOptions, UiDiagnostic, UiIr, UiTarget};
use logos::Logos;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

pub const FRONTEND_FORMAT: &str = "axiom-frontend-contract/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Backend,
    Frontend,
    Database,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Frontend => "frontend",
            Self::Database => "database",
        }
    }
}

pub struct Source {
    pub path: PathBuf,
    pub root: PathBuf,
    pub kind: Kind,
    manifest: Option<axiom_lib::deps_manifest::AxiomDepsManifestV2>,
}

/// The nearest manifest owns the source. Legacy headers are consulted only
/// when that manifest does not declare a type. Strings/comments are never
/// inspected as declarations.
pub fn source(path: &Path) -> Result<Source> {
    if path.extension().is_none_or(|value| value != "acore") {
        bail!("Expected an .acore contract, got {}", path.display());
    }
    let path = path
        .canonicalize()
        .with_context(|| format!("Read Acore source {}", path.display()))?;
    let text = std::fs::read_to_string(&path)?;
    let mut root = path
        .parent()
        .context("source has no parent directory")?
        .to_path_buf();
    let mut manifest = None;
    for directory in path.parent().unwrap().ancestors() {
        let candidate = directory.join("AxiomDeps.toml");
        if candidate.try_exists()? {
            let text = std::fs::read_to_string(&candidate)
                .with_context(|| format!("Read {}", candidate.display()))?;
            // Legacy manifests can contain extractor-owned fields. A typed
            // manifest, however, must pass the entire shared schema.
            if axiom_lib::deps_manifest::authored_contract_type(&text)?.is_some() {
                manifest = Some(axiom_lib::deps_manifest::parse_axiom_deps_v2(&text)?);
            }
            root = directory.to_path_buf();
            break;
        }
    }
    let kind = match manifest.as_ref().and_then(|m| m.contract_type.as_deref()) {
        Some("backend") => Kind::Backend,
        Some("frontend") => Kind::Frontend,
        Some("database") => Kind::Database,
        Some(other) => bail!("Project type `{other}` is not an application contract; use `axiom packages` or `axiom extensions` for those artifacts"),
        None => legacy_kind(&text),
    };
    Ok(Source {
        path,
        root,
        kind,
        manifest,
    })
}

fn legacy_kind(text: &str) -> Kind {
    use acore::lexer::Token;
    let tokens: Vec<_> = Token::lexer(text.trim_start_matches('\u{feff}'))
        .filter_map(|t| t.ok())
        .collect();
    if let [Token::Ident(profile), Token::Ident(kind), ..] = tokens.as_slice() {
        if profile == "profile" {
            return match kind.as_str() {
                "database" => Kind::Database,
                "frontend" => Kind::Frontend,
                _ => Kind::Backend,
            };
        }
    }
    let has_module = tokens.iter().any(|token| matches!(token, Token::Module));
    let ui_module = if let [Token::Module, Token::Ident(first), ..] = tokens.as_slice() {
        let mut last = first.as_str();
        let mut index = 2;
        while let Some([Token::Dot, Token::Ident(next)]) = tokens.get(index..index + 2) {
            last = next;
            index += 2;
        }
        last == "ui"
    } else {
        false
    };
    let has_ui = tokens.windows(2).any(|pair| matches!(pair, [Token::Ident(kind), Token::Ident(_)] if matches!(kind.as_str(), "app" | "page" | "component")));
    if ui_module || (has_module && has_ui) {
        Kind::Frontend
    } else {
        Kind::Backend
    }
}

#[derive(Clone, Copy, Default)]
pub struct Options<'a> {
    pub variant: Option<&'a str>,
    pub target: Option<&'a str>,
    pub lock: Option<&'a Path>,
    pub compatibility_baseline: Option<&'a Path>,
    pub release: bool,
}

pub fn validate_options(kind: Kind, options: Options<'_>) -> Result<()> {
    if kind != Kind::Backend
        && (options.variant.is_some()
            || options.compatibility_baseline.is_some()
            || options.release)
    {
        bail!("--variant, --compatibility-baseline and --release are backend-only options; this is a {} contract", kind.name());
    }
    if kind != Kind::Frontend && (options.target.is_some() || options.lock.is_some()) {
        bail!(
            "--target and --lock are frontend-only options; this is a {} contract",
            kind.name()
        );
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrontendContract {
    pub format: String,
    pub profile: String,
    pub module: String,
    pub targets: Vec<UiIr>,
}

fn target(value: &str) -> Result<UiTarget> {
    match value {
        "web" => Ok(UiTarget::Web),
        "ios" => Ok(UiTarget::Ios),
        "android" => Ok(UiTarget::Android),
        _ => bail!("Frontend target must be web, ios, or android (got `{value}`)"),
    }
}

fn compile_frontend(
    source: &Source,
    options: Options<'_>,
) -> Result<(FrontendContract, Vec<UiDiagnostic>)> {
    let mut targets = match options.target {
        Some(value) => vec![target(value)?],
        None => {
            let configuration = source.root.join("AcoreUI.json");
            if configuration.try_exists()? {
                let manifest: axiom_ui::project::Manifest =
                    serde_json::from_slice(&std::fs::read(&configuration)?)
                        .context("Read AcoreUI.json")?;
                if manifest.profile != axiom_ui::frontend_program::PROFILE_FORMAT {
                    bail!("Unsupported AcoreUI.json profile: {}", manifest.profile);
                }
                manifest.targets
            } else if let Some(manifest) = &source.manifest {
                manifest
                    .targets
                    .iter()
                    .map(|t| target(t.as_str()))
                    .collect::<Result<Vec<_>>>()?
            } else {
                vec![UiTarget::Web]
            }
        }
    };
    if targets.is_empty() {
        targets.push(UiTarget::Web);
    }
    targets.sort_by_key(|target| target.as_str());
    targets.dedup();
    let lock = options.lock.map(Path::to_path_buf).unwrap_or_else(|| {
        let committed = source.root.join("axiom.ui.lock.json");
        let development = source.root.join(".axiom/dev/axiom.ui.lock.json");
        if !committed.is_file() && development.is_file() {
            development
        } else {
            committed
        }
    });
    let text = std::fs::read_to_string(&source.path)?;
    let mut irs = Vec::new();
    let mut diagnostics = Vec::new();
    for target in targets {
        let compilation = axiom_ui::compile_ui_source_at_path(
            &text,
            &source.path,
            &UiCompileOptions {
                target,
                lock_path: lock.clone(),
                asset_root: Some(source.root.clone()),
            },
        );
        diagnostics.extend(compilation.diagnostics.clone());
        if !compilation.is_valid() {
            bail!(
                "Frontend check failed ({}): {}",
                target.as_str(),
                serde_json::to_string(&compilation.diagnostics)?
            );
        }
        irs.push(
            compilation
                .ir
                .context("Frontend compiler did not produce an IR")?,
        );
    }
    diagnostics.dedup();
    let module = irs[0].module.clone();
    Ok((
        FrontendContract {
            format: FRONTEND_FORMAT.into(),
            profile: axiom_ui::frontend_program::PROFILE_FORMAT.into(),
            module,
            targets: irs,
        },
        diagnostics,
    ))
}

/// Canonical output is replaced only after all targets have compiled.
pub fn write_artifact(path: &Path, bytes: &[u8]) -> Result<()> {
    validate_output(path)?;
    atomic_write(path, bytes)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.is_symlink() || path.is_dir() {
        bail!("Output must be a regular file: {}", path.display());
    }
    let parent = parent(path);
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .with_context(|| format!("Write artifact {}", path.display()))?;
    Ok(())
}

pub fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

pub fn validate_output(path: &Path) -> Result<()> {
    if path.extension().is_none_or(|e| e != "axiom") {
        bail!("Contract output must use .axiom: {}", path.display());
    }
    if path.is_symlink() {
        bail!("Contract output cannot be a symlink: {}", path.display());
    }
    if path.is_dir() {
        bail!("Contract output is a directory: {}", path.display());
    }
    Ok(())
}

pub async fn build(file: &Path, output: Option<&Path>, options: Options<'_>) -> Result<PathBuf> {
    let source = source(file)?;
    validate_options(source.kind, options)?;
    let output = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| parent(file).join(format!("{}.axiom", source.kind.name())));
    validate_output(&output)?;
    match source.kind {
        Kind::Backend => {
            super::backend::build_to(
                file,
                options.variant.unwrap_or("default"),
                None,
                options.compatibility_baseline,
                &output,
            )
            .await?;
        }
        Kind::Database => {
            super::database::build_artifact(file, Some(&output))?;
        }
        Kind::Frontend => {
            let (contract, diagnostics) = compile_frontend(&source, options)?;
            for diagnostic in diagnostics {
                eprintln!("{}: {}", diagnostic.code, diagnostic.message);
            }
            write_artifact(&output, &serde_jcs::to_vec(&contract)?)?;
        }
    }
    println!(
        "Built {} contract: {}",
        source.kind.name(),
        output.display()
    );
    Ok(output)
}

pub fn check(file: &Path, options: Options<'_>, json: bool) -> Result<()> {
    let mut kind = None;
    let result = (|| -> Result<serde_json::Value> {
        let source = source(file)?;
        kind = Some(source.kind);
        validate_options(source.kind, options)?;
        match source.kind {
            Kind::Backend => {
                super::backend::validate(file, options.variant)?;
                Ok(serde_json::json!([]))
            }
            Kind::Database => {
                super::database::source(file)?;
                Ok(serde_json::json!([]))
            }
            Kind::Frontend => Ok(serde_json::to_value(compile_frontend(&source, options)?.1)?),
        }
    })();
    let diagnostics = match &result {
        Ok(diagnostics) => diagnostics.clone(),
        Err(error) => serde_json::json!([format!("{error:#}")]),
    };
    if json {
        println!(
            "{}",
            serde_json::json!({"format":"axiom-contract-check/v1","valid":result.is_ok(),"file":file,"profile":kind,"variant":(kind == Some(Kind::Backend)).then_some(options.variant.unwrap_or("default")),"diagnostics":diagnostics})
        );
    } else if result.is_ok() {
        println!(
            "Valid {} contract: {}",
            kind.unwrap().name(),
            file.display()
        );
        if let Some(warnings) = diagnostics.as_array() {
            for diagnostic in warnings {
                eprintln!("{diagnostic}");
            }
        }
    }
    result.map(|_| ())
}

/// Return true for the new frontend envelope or an existing database schema.
pub fn inspect(path: &Path) -> Result<bool> {
    let bytes = std::fs::read(path)?;
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        if value.get("format").and_then(|v| v.as_str()) == Some(FRONTEND_FORMAT) {
            let contract: FrontendContract = serde_json::from_value(value)?;
            if contract.profile != axiom_ui::frontend_program::PROFILE_FORMAT
                || contract.targets.is_empty()
                || contract
                    .targets
                    .iter()
                    .any(|ir| ir.format != axiom_ui::UI_IR_FORMAT || ir.module != contract.module)
            {
                bail!("Unsupported or inconsistent frontend contract snapshot");
            }
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"format":contract.format,"profile":contract.profile,"module":contract.module,"targets":contract.targets.iter().map(|ir| ir.target).collect::<Vec<_>>(),"pages":contract.targets[0].pages.len(),"components":contract.targets[0].components.len(),"artifact":path})
                )?
            );
            return Ok(true);
        }
    }
    if matches!(axiom_lib::package::decode_axiom_package(&bytes), Ok(axiom_lib::package::DecodedAxiomPackage::Package(package)) if matches!(package.contents, axiom_lib::package::PackageContents::DatabaseSchema(_)))
    {
        super::database::inspect(path)?;
        return Ok(true);
    }
    Ok(false)
}

pub fn validate_release(path: &Path) -> Result<()> {
    axiom_lib::unpackager::unpack_axiom_bytes(&std::fs::read(path)?).context(
        "Cloud release requires a compiled backend contract; frontend snapshots use application packaging and database schemas use database release tooling"
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routing_ignores_strings_and_comments() {
        assert_eq!(legacy_kind("// module demo.ui\n// page Fake {}\nproject { description = \"\"\"module demo.ui\npage Fake {}\"\"\" }"), Kind::Backend);
        assert_eq!(
            legacy_kind("module helpers.ui\npure fn trim(value: String): String = value.trim()"),
            Kind::Frontend
        );
        assert_eq!(
            legacy_kind("// heading\nprofile database 1\nmodule storage"),
            Kind::Database
        );
        assert_eq!(
            legacy_kind("module demo.ui\npage Home { view { Text(\"Hi\") } }"),
            Kind::Frontend
        );
    }
}
