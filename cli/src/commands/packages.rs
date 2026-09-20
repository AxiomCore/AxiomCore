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
    write_package_lock,
};
use axiom_ui::{compile_ui_source_with_package_lock, UiCompileOptions, UiTarget};

pub async fn handle_resolve(deps: PathBuf, lock: PathBuf) -> Result<()> {
    let resolved = resolve_package_manifest(&deps)?;
    write_package_lock(&lock, &resolved)?;
    println!(
        "Resolved {} frontend package alias(es) into {}.",
        resolved.packages.len(),
        lock.display()
    );
    Ok(())
}

pub async fn handle_build(source: PathBuf, out: PathBuf) -> Result<()> {
    let package: AxiomPackageEnvelope = serde_json::from_slice(&std::fs::read(&source)?)?;
    let encoded = encode_axiom_package(&package)?;
    std::fs::write(&out, &encoded.bytes)?;
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
        "Verified {} locked frontend package alias(es).",
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
) -> Result<()> {
    let text = std::fs::read_to_string(&source)?;
    let analysis = analyze_package_imports(&text, &package_lock);
    if !analysis.diagnostics.is_empty() {
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
    let compilation = compile_ui_source_with_package_lock(
        &text,
        &UiCompileOptions {
            target,
            lock_path: ui_lock,
            asset_root: source.parent().map(PathBuf::from),
        },
        &package_lock,
    );
    if !compilation.is_valid() {
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

    #[test]
    fn approval_names_are_explicit_and_closed() {
        assert_eq!(
            parse_approval("target-reduction").unwrap(),
            PackageApproval::TargetReduction
        );
        assert!(parse_approval("all").is_err());
    }
}
