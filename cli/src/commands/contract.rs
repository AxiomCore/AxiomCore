use acore::ui_contract::analyze_ui_contract_imports;
use anyhow::Result;
use axiom_lib::ui_contract::{
    diff_locks, read_lock, resolve_manifest, verify_lock, virtual_facade, write_lock,
};
use std::path::PathBuf;

pub async fn handle_resolve(deps: PathBuf, lock: PathBuf) -> Result<()> {
    let next = resolve_manifest(&deps)?;
    if lock.is_file() {
        let previous = read_lock(&lock)?;
        let diff = diff_locks(&previous, &next);
        if !diff.changes.is_empty() {
            println!("{}", serde_json::to_string_pretty(&diff)?);
        }
    }
    write_lock(&lock, &next)?;
    print_unsigned_warnings(&next);
    println!(
        "Resolved {} UI contract alias(es) into {}",
        next.contracts.len(),
        lock.display()
    );
    Ok(())
}

pub async fn handle_verify(deps: PathBuf, lock: PathBuf) -> Result<()> {
    let lockfile = read_lock(&lock)?;
    verify_lock(&deps, &lockfile)?;
    print_unsigned_warnings(&lockfile);
    let signed = lockfile
        .contracts
        .values()
        .filter(|contract| contract.signature.is_some())
        .count();
    println!(
        "Checked {} locked UI contract alias(es): {} signed, {} unsigned.",
        lockfile.contracts.len(),
        signed,
        lockfile.contracts.len() - signed
    );
    Ok(())
}

fn print_unsigned_warnings(lock: &axiom_lib::ui_contract::UiContractLock) {
    for (alias, contract) in &lock.contracts {
        if contract.signature.is_none() {
            eprintln!(
                "Warning: contract `{alias}` uses unsigned local artifact {}. Allowed for development; release it through Axiom Cloud before production use.",
                contract.artifact.display()
            );
        }
    }
}

pub async fn handle_diff(before: PathBuf, after: PathBuf) -> Result<()> {
    let before = read_lock(&before)?;
    let after = read_lock(&after)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&diff_locks(&before, &after))?
    );
    Ok(())
}

pub async fn handle_facade(lock: PathBuf, alias: String) -> Result<()> {
    let lockfile = read_lock(&lock)?;
    let facade = virtual_facade(&lockfile, &alias)?;
    println!("{}", serde_json::to_string_pretty(&facade)?);
    Ok(())
}

/// Validate `use contract` declarations against a committed lock.
/// No generated facade file is created; the next compiler phase consumes the
/// same virtual metadata directly in memory.
pub async fn handle_check_source(source: PathBuf, lock: PathBuf) -> Result<()> {
    let text = std::fs::read_to_string(&source)?;
    let analysis = analyze_ui_contract_imports(&text, &lock);
    if analysis.diagnostics.is_empty() {
        println!(
            "Validated {} locked UI contract import(s) in {}.",
            analysis.definitions.len(),
            source.display()
        );
        return Ok(());
    }
    for diagnostic in &analysis.diagnostics {
        println!(
            "{}:{}: {}",
            source.display(),
            diagnostic.range.start,
            diagnostic.message
        );
    }
    anyhow::bail!(
        "UI contract import validation failed with {} diagnostic(s)",
        analysis.diagnostics.len()
    )
}
