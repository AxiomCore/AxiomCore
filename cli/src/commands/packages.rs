use std::collections::BTreeSet;
use std::path::PathBuf;

use acore::ui_contract::analyze_package_imports;
use anyhow::{bail, Context, Result};
use axiom_lib::package::{
    decode_axiom_package, encode_axiom_package, AxiomPackageEnvelope, DecodedAxiomPackage,
};
use axiom_lib::package_diff::{diff_packages, require_package_approvals, PackageApproval};
use axiom_lib::package_resolver::{
    load_locked_package, read_package_lock, resolve_package_manifest, verify_package_lock,
};
use axiom_ui::{compile_ui_source_with_package_lock_at_path, UiCompileOptions, UiTarget};

fn adaptation_result(result: Result<serde_json::Value>) -> Result<()> {
    match result {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({"format":"acore-adaptation-check/v1","passed":false,"diagnostic":format!("{error:#}")})
            );
            Err(error)
        }
    }
}
pub async fn handle_adaptation_inputs(inputs: PathBuf) -> Result<()> {
    adaptation_result(axiom_ui::adaptation::inspect_inputs(&inputs))
}
pub async fn handle_check_adaptation(inputs: PathBuf, report: PathBuf) -> Result<()> {
    adaptation_result(axiom_ui::adaptation::check_report(&inputs, &report))
}
pub async fn handle_diff_adaptation(
    before: PathBuf,
    after: PathBuf,
    before_inputs: PathBuf,
    after_inputs: PathBuf,
) -> Result<()> {
    adaptation_result((|| {
        axiom_ui::adaptation::check_report(&before_inputs, &before)?;
        axiom_ui::adaptation::check_report(&after_inputs, &after)?;
        axiom_ui::adaptation::diff(
            &axiom_ui::adaptation::read_json(&before)?,
            &axiom_ui::adaptation::read_json(&after)?,
        )
    })())
}
pub async fn handle_adapt_library(inputs: PathBuf, recipe: PathBuf, out: PathBuf) -> Result<()> {
    adaptation_result((|| {
        anyhow::ensure!(!out.exists(),"AD_OUTPUT_EXISTS: use a new candidate directory; existing source and forks are never overwritten");
        let output = axiom_ui::adaptation::assemble(&inputs, &recipe)?;
        let parent = out
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        std::fs::create_dir_all(parent)?;
        let stage = tempfile::tempdir_in(parent)?;
        for (path, bytes) in &output.files {
            let destination = stage.path().join(path);
            std::fs::create_dir_all(destination.parent().unwrap())?;
            std::fs::write(destination, bytes)?;
        }
        let proof = serde_json::json!({"format":"acore-adaptation-build/v1","passed":true,"artifactSha256":output.build.sha256,"reportSha256":axiom_lib::component_library::hash(&axiom_lib::component_library::canonical(&output.report)?),"build":output.build.report,"runtimeQualified":false,"locksChanged":false});
        std::fs::write(stage.path().join("library.axiom"), &output.build.bytes)?;
        std::fs::write(
            stage.path().join("BUILD.json"),
            serde_json::to_vec_pretty(&proof)?,
        )?;
        std::fs::rename(stage.path(), &out)?;
        Ok(proof)
    })())
}
pub async fn handle_check_library_api(inputs: PathBuf, api: PathBuf) -> Result<()> {
    adaptation_result((|| {
        let pins = axiom_ui::adaptation::verify_inputs(&inputs)?;
        axiom_ui::adaptation_api::check_blueprint(&axiom_ui::adaptation::read_json(&api)?, &pins)
    })())
}
pub async fn handle_plan_adaptation(inputs: PathBuf, api: PathBuf, out: PathBuf) -> Result<()> {
    adaptation_result((|| {
        anyhow::ensure!(
            !out.exists(),
            "AD_OUTPUT_EXISTS: preserve the reviewed report and choose a new candidate path"
        );
        let pins = axiom_ui::adaptation::verify_inputs(&inputs)?;
        let report =
            axiom_ui::adaptation::plan_blueprint(&axiom_ui::adaptation::read_json(&api)?, &pins)?;
        write_atomic(&out, &axiom_lib::component_library::canonical(&report)?)?;
        axiom_ui::adaptation::validate_report(&report, &pins)
    })())
}
pub async fn handle_fork_library(
    source: PathBuf,
    name: String,
    version: String,
    out: PathBuf,
) -> Result<()> {
    adaptation_result((|| {
        anyhow::ensure!(
            !out.exists(),
            "AD_OUTPUT_EXISTS: choose a new source-fork directory"
        );
        let result = axiom_ui::adaptation::fork_source(&source, &name, &version)?;
        let parent = out
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        std::fs::create_dir_all(parent)?;
        let stage = tempfile::tempdir_in(parent)?;
        for (path, bytes) in &result.files {
            let destination = stage.path().join(path);
            std::fs::create_dir_all(destination.parent().unwrap())?;
            std::fs::write(destination, bytes)?;
        }
        std::fs::write(stage.path().join("library.axiom"), &result.build.bytes)?;
        std::fs::write(
            stage.path().join("FORK.json"),
            serde_json::to_vec_pretty(&result.provenance)?,
        )?;
        std::fs::rename(stage.path(), &out)?;
        Ok(result.provenance)
    })())
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub async fn handle_init_library(directory: PathBuf, name: String, json: bool) -> Result<()> {
    let files = axiom_ui::source_library::starter(&name)?;
    if directory.exists() && std::fs::read_dir(&directory)?.next().is_some() {
        bail!("PK_SOURCE_EXISTS: starter directory must be empty");
    }
    let parent = directory
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let stage = tempfile::tempdir_in(parent)?;
    for (path, source) in &files {
        std::fs::write(stage.path().join(path), source)?;
    }
    std::fs::rename(stage.path(), &directory)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"format":"acore-library-init/v1","directory":directory,"package":name,"files":files.keys().collect::<Vec<_>>()})
            )?
        );
    } else {
        println!("Created {}. Check it with `axiom packages check-library --source-manifest {}/AxiomLibrary.toml`.",directory.display(),directory.display());
    }
    Ok(())
}
pub async fn handle_check_library(source: PathBuf, json: bool) -> Result<()> {
    match axiom_ui::source_library::build(&source) {
        Ok(output) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&output.report)?);
            } else {
                println!(
                    "Checked {} ({}) on every declared target and the verified consumer path.",
                    source.display(),
                    output.sha256
                );
            }
            Ok(())
        }
        Err(error) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"format":"acore-source-library-build/v1","passed":false,"diagnostic":format!("{error:#}")})
                );
            }
            Err(error)
        }
    }
}
pub async fn handle_build_input(
    source: Option<PathBuf>,
    manifest: Option<PathBuf>,
    out: PathBuf,
    json: bool,
) -> Result<()> {
    if let Some(manifest) = manifest {
        match axiom_ui::source_library::build(&manifest) {
            Ok(output) => {
                write_atomic(&out, &output.bytes)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&output.report)?);
                } else {
                    println!("Built {} ({})", out.display(), output.sha256);
                }
                return Ok(());
            }
            Err(error) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"format":"acore-source-library-build/v1","passed":false,"diagnostic":format!("{error:#}")})
                    );
                }
                return Err(error);
            }
        }
    }
    let source = source.context("provide a JSON envelope or --source-manifest")?;
    if !json {
        return handle_build(source, out).await;
    }
    let package: AxiomPackageEnvelope = serde_json::from_slice(&std::fs::read(source)?)?;
    let encoded = encode_axiom_package(&package)?;
    write_atomic(&out, &encoded.bytes)?;
    println!(
        "{}",
        serde_json::json!({"format":"axiom-package-build/v1","sha256":encoded.sha256,"out":out})
    );
    Ok(())
}

pub async fn handle_resolve(deps: PathBuf, lock: PathBuf) -> Result<()> {
    let resolved = resolve_package_manifest(&deps)?;
    // Resolution validates the complete lock before any output is replaced.
    // Default project state may not exist yet in a freshly authored project.
    write_atomic(&lock, &serde_jcs::to_vec(&resolved)?)?;
    println!(
        "Resolved {} package alias(es) into {}.",
        resolved.packages.len(),
        lock.display()
    );
    Ok(())
}

pub async fn handle_build(source: PathBuf, out: PathBuf) -> Result<()> {
    let package: AxiomPackageEnvelope = serde_json::from_slice(&std::fs::read(&source)?)?;
    let encoded = encode_axiom_package(&package)?;
    write_atomic(&out, &encoded.bytes)?;
    println!("Built {} ({})", out.display(), encoded.sha256);
    Ok(())
}

pub async fn handle_verify(deps: PathBuf, lock: PathBuf) -> Result<()> {
    let locked = read_package_lock(&lock)?;
    verify_package_lock(&deps, &locked)?;
    for alias in locked.packages.keys() {
        load_locked_package(&lock, alias)
            .with_context(|| format!("locked package `{alias}` failed verification"))?;
    }
    println!(
        "Verified {} locked package alias(es).",
        locked.packages.len()
    );
    Ok(())
}

pub async fn handle_inspect(lock: PathBuf, alias: String) -> Result<()> {
    let package = load_locked_package(&lock, &alias)?;
    println!("{}", serde_json::to_string_pretty(&package)?);
    Ok(())
}

pub async fn handle_diff(before: PathBuf, after: PathBuf, approvals: Vec<String>) -> Result<()> {
    let before = read_envelope(&before)?;
    let after = read_envelope(&after)?;
    let diff = diff_packages(&before, &after)?;
    println!("{}", serde_json::to_string_pretty(&diff)?);
    let approvals = approvals
        .iter()
        .map(|value| parse_approval(value))
        .collect::<Result<BTreeSet<_>>>()?;
    require_package_approvals(&diff, &approvals)?;
    Ok(())
}

pub async fn handle_check_source(
    source: PathBuf,
    package_lock: PathBuf,
    ui_lock: PathBuf,
    target: String,
    json: bool,
) -> Result<()> {
    let text = std::fs::read_to_string(&source)?;
    let analysis = analyze_package_imports(&text, &package_lock);
    if !analysis.diagnostics.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::json!({"format":"acore-package-source-check/v1","passed":false,"diagnostics":analysis.diagnostics.iter().map(|d|serde_json::json!({"start":d.range.start,"end":d.range.end,"message":d.message})).collect::<Vec<_>>()})
            );
        }
        for diagnostic in &analysis.diagnostics {
            eprintln!(
                "{}:{}: {}",
                source.display(),
                diagnostic.range.start,
                diagnostic.message
            );
        }
        bail!(
            "frontend package source validation failed with {} diagnostic(s)",
            analysis.diagnostics.len()
        );
    }
    let target = match target.as_str() {
        "android" => UiTarget::Android,
        "ios" => UiTarget::Ios,
        "web" => UiTarget::Web,
        _ => bail!("target must be android, ios, or web"),
    };
    let compilation = compile_ui_source_with_package_lock_at_path(
        &text,
        &source,
        &UiCompileOptions {
            target,
            lock_path: ui_lock,
            asset_root: source.parent().map(PathBuf::from),
        },
        &package_lock,
    );
    if !compilation.is_valid() {
        if json {
            println!(
                "{}",
                serde_json::json!({"format":"acore-package-source-check/v1","passed":false,"diagnostics":compilation.diagnostics})
            );
        }
        for diagnostic in &compilation.diagnostics {
            eprintln!(
                "{}:{}: {} {}",
                source.display(),
                diagnostic.span.start,
                diagnostic.code,
                diagnostic.message
            );
        }
        bail!("frontend package source did not compile for {target:?}");
    }
    if json {
        let ir = compilation.ir.as_ref().context("PK_SOURCE_IR")?;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"format":"acore-package-source-check/v1","passed":true,"target":target.as_str(),"graphRevision":compilation.context.graph_revision,"irFormat":ir.format,"bindingExplanation":ir.component_contracts,"styleContracts":ir.style_contracts,"sourceEvidence":ir.source_evidence})
            )?
        );
        return Ok(());
    }
    println!(
        "Validated {} locked frontend import(s) and compiled {} for {target:?}.",
        analysis.definitions.len(),
        source.display()
    );
    Ok(())
}

fn read_envelope(path: &PathBuf) -> Result<axiom_lib::package::AxiomPackageEnvelope> {
    match decode_axiom_package(&std::fs::read(path)?)? {
        DecodedAxiomPackage::Package(package) => Ok(package),
        DecodedAxiomPackage::LegacyService(_) => {
            bail!(
                "{} is a legacy service artifact, not a package envelope",
                path.display()
            )
        }
    }
}

fn parse_approval(value: &str) -> Result<PackageApproval> {
    match value {
        "breaking-change" => Ok(PackageApproval::BreakingChange),
        "target-reduction" => Ok(PackageApproval::TargetReduction),
        "new-required-theme-role" => Ok(PackageApproval::NewRequiredThemeRole),
        "permission-increase" => Ok(PackageApproval::PermissionIncrease),
        _ => bail!(
            "unknown approval `{value}`; expected breaking-change, target-reduction, new-required-theme-role, or permission-increase"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn adaptation_candidates_and_source_forks_preserve_prior_artifacts_and_dependency_locks()
    {
        let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../axiom-ui/examples/source-adaptation/react-action")
            .canonicalize()
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let candidate = temp.path().join("candidate");
        handle_adapt_library(
            example.join("UPSTREAM.lock.json"),
            example.join("RECIPE.json"),
            candidate.clone(),
        )
        .await
        .unwrap();
        let original = std::fs::read(candidate.join("library.axiom")).unwrap();
        assert!(handle_adapt_library(
            example.join("UPSTREAM.lock.json"),
            example.join("RECIPE.json"),
            candidate.clone()
        )
        .await
        .is_err());
        assert_eq!(
            std::fs::read(candidate.join("library.axiom")).unwrap(),
            original
        );
        let fork = temp.path().join("fork");
        handle_fork_library(
            candidate.join("AxiomLibrary.toml"),
            "local.action".into(),
            "1.0.0".into(),
            fork.clone(),
        )
        .await
        .unwrap();
        std::fs::create_dir(fork.join("deps")).unwrap();
        std::fs::write(fork.join("deps/original.axiom"), &original).unwrap();
        std::fs::write(fork.join("AxiomDeps.toml"),"format='axiom-package-deps/v1'\ntargets=['android','ios','web']\n[packages.original]\nartifact='deps/original.axiom'\nname='example.react-action'\nversion='1.0.0'\nkind='component-library'\n").unwrap();
        let lock = fork.join(".axiom/AxiomPackages.lock");
        handle_resolve(fork.join("AxiomDeps.toml"), lock.clone())
            .await
            .unwrap();
        let before = std::fs::read(&lock).unwrap();
        let second = temp.path().join("second");
        handle_fork_library(
            fork.join("AxiomLibrary.toml"),
            "local.second".into(),
            "1.0.0".into(),
            second.clone(),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&lock).unwrap(), before);
        assert_eq!(
            std::fs::read(second.join(".axiom/AxiomPackages.lock")).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read(second.join("deps/original.axiom")).unwrap(),
            original
        );
        load_locked_package(&second.join(".axiom/AxiomPackages.lock"), "original").unwrap();
        assert!(second.join("vendor/action.tsx").is_file());
        let mut invalid: serde_json::Value =
            serde_json::from_slice(&std::fs::read(example.join("RECIPE.json")).unwrap()).unwrap();
        invalid["transforms"][0]["component"]["props"]["label"]["type"] =
            serde_json::json!("Window");
        let bad = example.join("RECIPE.json");
        let changed = temp.path().join("bad.json");
        std::fs::write(&changed, serde_json::to_vec(&invalid).unwrap()).unwrap(); // manually owned theme paths are deliberately absent too
        let absent = temp.path().join("invalid-candidate");
        assert!(
            handle_adapt_library(example.join("UPSTREAM.lock.json"), changed, absent.clone())
                .await
                .is_err()
        );
        assert!(!absent.exists());
        assert_eq!(
            std::fs::read(candidate.join("library.axiom")).unwrap(),
            original
        );
        assert!(bad.is_file());
    }

    #[tokio::test]
    async fn first_resolution_creates_hidden_state_and_invalid_update_keeps_exact_lock() {
        use axiom_lib::package::*;
        use axiom_lib::theme::*;
        use std::collections::BTreeMap;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let package = AxiomPackageEnvelope {
            format: AXIOM_PACKAGE_FORMAT.into(),
            package: PackageIdentity {
                name: "test.theme".into(),
                version: "1.0.0".into(),
            },
            targets: vec![PackageTarget::Web],
            contents: PackageContents::Theme(ThemePackagePayload {
                schema: THEME_PACKAGE_SCHEMA.into(),
                exports: BTreeMap::from([(
                    "primary".into(),
                    ThemeExport {
                        tokens: BTreeMap::from([(
                            "color.primary".into(),
                            ThemeToken::Color(ThemeValue::Literal {
                                value: ColorValue("#112233".into()),
                            }),
                        )]),
                        required_roles: BTreeMap::from([(
                            "color.primary".into(),
                            ThemeTokenKind::Color,
                        )]),
                        appearances: BTreeMap::new(),
                        platforms: BTreeMap::new(),
                    },
                )]),
            }),
        };
        std::fs::write(
            root.join("theme.axiom"),
            encode_axiom_package(&package).unwrap().bytes,
        )
        .unwrap();
        let deps = root.join("AxiomDeps.toml");
        std::fs::write(
            &deps,
            r#"format="axiom-package-deps/v1"
targets=["web"]
[packages.theme]
artifact="theme.axiom"
name="test.theme"
version="1.0.0"
kind="theme"
"#,
        )
        .unwrap();
        let lock = root.join(".axiom/AxiomPackages.lock");
        assert!(!lock.parent().unwrap().exists());
        handle_resolve(deps.clone(), lock.clone()).await.unwrap();
        handle_verify(deps.clone(), lock.clone()).await.unwrap();
        assert_eq!(load_locked_package(&lock, "theme").unwrap(), package);
        let original = std::fs::read(&lock).unwrap();
        std::fs::write(&deps, "invalid dependency candidate").unwrap();
        assert!(handle_resolve(deps, lock.clone()).await.is_err());
        assert_eq!(std::fs::read(&lock).unwrap(), original);
    }

    #[test]
    fn approval_names_are_explicit_and_closed() {
        assert_eq!(
            parse_approval("target-reduction").unwrap(),
            PackageApproval::TargetReduction
        );
        assert!(parse_approval("all").is_err());
    }
}
