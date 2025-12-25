use anyhow::{anyhow, Context, Result};
use axiom_build::core::client_sdk::flutter::{ensure_deps, generate_from_fbs, install_runtime};
use axiom_build::core::unpackager::unpack_axiom_file;
use axiom_lib::config::load_config;
use std::path::PathBuf;

/// Entry for `axiom pull --path <file>`
pub async fn handle_pull_path(path: &str, runtime_source: Option<&str>) -> Result<()> {
    let project_root = std::env::current_dir().context("Failed to get current directory")?;
    let axiom_src_path = PathBuf::from(path);

    if !axiom_src_path.exists() {
        anyhow::bail!(
            "Provided .axiom path does not exist: {}",
            axiom_src_path.display()
        );
    }

    let axiom_filename = axiom_src_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("Invalid .axiom path: could not extract filename"))?;

    let axiom_dst_path = project_root.join(axiom_filename);

    println!("   Verifying and unpacking contract...");

    // FIX: Unpack the SOURCE path (the one passed in CLI), not the destination
    let contract =
        unpack_axiom_file(&axiom_src_path).context("Failed to verify the source .axiom file")?;

    // Now that verification passed, copy it to the project root
    std::fs::copy(&axiom_src_path, &axiom_dst_path)
        .context("Failed to copy .axiom file to project root")?;

    println!("   Contract imported to: {}", axiom_dst_path.display());

    // 5. Validate Client Configuration (axiom.yaml)
    let config = load_config()?;
    let frontend_config = config.frontend.ok_or_else(|| {
        anyhow::anyhow!("Missing 'frontend' section in axiom.yaml. Run 'axiom init'?")
    })?;

    if frontend_config.framework.to_lowercase() != "flutter" {
        anyhow::bail!(
            "axiom pull currently only supports 'flutter'. Found: {}",
            frontend_config.framework
        );
    }

    // 6. Install Runtime
    println!(
        "⬇️  Installing runtime (v{})...",
        contract.header.min_runtime_version
    );
    install_runtime::install_runtime(
        &project_root,
        contract.header.min_runtime_version,
        runtime_source,
    )
    .await?;

    // 7. Ensure Flutter Dependencies
    ensure_deps::ensure_deps(&project_root, axiom_filename)?;

    // 8. Generate Client Code
    println!("⚙️  Generating SDK code...");

    generate_from_fbs::generate_from_fbs(
        &project_root,
        &frontend_config,
        &contract.schema_fbs,
        path,
    )
    .await?;

    println!("✅ axiom pull finished successfully.");
    Ok(())
}
