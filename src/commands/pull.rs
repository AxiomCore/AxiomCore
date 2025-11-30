use anyhow::{Context, Result};
use axiom_build::core::client_sdk::flutter::{ensure_deps, generate_from_fbs, install_runtime};
use axiom_build::core::unpackager::unpack_axiom_file;
use axiom_lib::config::load_frontend_config;
use std::fs;
use std::path::PathBuf;

/// Entry for `axiom pull --path <file>`
pub async fn handle_pull_path(path: &str) -> Result<()> {
    let project_root = std::env::current_dir().context("Failed to get current directory")?;
    let axiom_src_path = PathBuf::from(path);

    if !axiom_src_path.exists() {
        anyhow::bail!(
            "Provided .axiom path does not exist: {}",
            axiom_src_path.display()
        );
    }

    // Copy into project root (keep same file name)
    let file_name = axiom_src_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid .axiom path (no file name)"))?;
    let axiom_dst_path = project_root.join(file_name);
    if axiom_src_path != axiom_dst_path {
        fs::copy(&axiom_src_path, &axiom_dst_path).with_context(|| {
            format!(
                "Failed to copy .axiom file to project root: {}",
                axiom_dst_path.display()
            )
        })?;
    }

    // Load frontend config
    let frontend_cfg = load_frontend_config(&project_root)?;
    let framework = frontend_cfg.framework.to_lowercase();
    if framework != "flutter" {
        anyhow::bail!(
            "axiom pull (flutter) expected framework: flutter in axiom.yaml, found: {}",
            frontend_cfg.framework
        );
    }

    // Decode the .axiom file
    let (ir, runtime_bytes, fbs_bytes) = unpack_axiom_file(&axiom_dst_path).await?;

    install_runtime::install_runtime(&project_root, &ir, &runtime_bytes)?;

    generate_from_fbs::generate_from_fbs(&project_root, &frontend_cfg, &fbs_bytes).await?;

    ensure_deps::ensure_deps(&project_root)?;

    println!("✅ axiom pull (path) finished successfully.");
    Ok(())
}
