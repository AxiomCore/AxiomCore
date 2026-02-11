use anyhow::{anyhow, Context, Result};
use axiom_build::core::client_sdk::flutter::{ensure_deps, generate_from_fbs};
use axiom_build::core::unpackager::unpack_axiom_file;
use axiom_cloud::CloudClient;
use axiom_lib::config::load_config;
use std::fs;
use std::path::{Path, PathBuf};

pub async fn handle_pull_path(path: &str) -> Result<()> {
    internal_handle_pull_local(path).await
}

async fn internal_handle_pull_local(path: &str) -> Result<()> {
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
        .ok_or_else(|| anyhow!("Invalid .axiom path"))?;

    let axiom_dst_path = project_root.join(axiom_filename);

    println!("   Verifying and unpacking contract...");
    let contract = unpack_axiom_file(&axiom_src_path)?;

    fs::copy(&axiom_src_path, &axiom_dst_path).context("Failed to copy .axiom file")?;
    println!("   Contract imported to: {}", axiom_dst_path.display());

    post_pull_steps(&project_root, &axiom_dst_path, &contract).await
}

pub async fn handle_pull_package(package_ptr: &str, version: Option<&str>) -> Result<()> {
    let project_root = std::env::current_dir().context("Failed to get current directory")?;

    let auth_data = crate::auth_store::load_auth_data()?;
    let client = CloudClient::new(
        std::env::var("AXIOM_CLOUD_URL").unwrap_or_else(|_| "https://api.axiom.xyz".to_string()),
        auth_data.access_token,
    );

    let version = version.unwrap_or("latest");
    println!("⬇️  Fetching contract '{}' (@{})...", package_ptr, version);

    let metadata = client.get_contract_metadata(package_ptr, version).await?;
    let content = client.pull_contract_content(package_ptr, version).await?;

    let filename = format!("{}.axiom", package_ptr);
    let axiom_dst_path = project_root.join(&filename);

    fs::write(&axiom_dst_path, &content).context("Failed to write .axiom file")?;

    // Create trust file
    let keys = client.get_project_keys(package_ptr).await?;
    let public_key = keys
        .iter()
        .find(|k| k.id == metadata.key_id)
        .map(|k| k.public_key.clone())
        .ok_or_else(|| anyhow!("Public key not found"))?;

    let trust_data = serde_json::json!({
        "project_id": metadata.project_id,
        "version": metadata.version,
        "signature": metadata.signature,
        "public_key": public_key,
    });
    fs::write(
        project_root.join(".trust-axiom.json"),
        serde_json::to_string_pretty(&trust_data)?,
    )?;

    let contract = unpack_axiom_file(&axiom_dst_path)?;
    post_pull_steps(&project_root, &axiom_dst_path, &contract).await
}

async fn post_pull_steps(
    project_root: &Path,
    axiom_path: &Path,
    contract: &axiom_lib::contract::AxiomFile,
) -> Result<()> {
    let config = load_config()?;
    let frontend_config = config
        .frontend
        .ok_or_else(|| anyhow!("Missing 'frontend' in axiom.yaml"))?;

    // 1. Ensure axiom_flutter is in pubspec.yaml
    // This now implicitly "installs" the runtime because the plugin contains the XCFramework
    println!("📦 Checking dependencies...");
    ensure_deps::ensure_deps(
        project_root,
        axiom_path.file_name().unwrap().to_str().unwrap(),
    )?;

    // 2. Generate Client Code
    println!("⚙️  Generating SDK code...");
    let fbs_bytes = contract.schema_fbs.as_deref().unwrap_or(&[]);
    generate_from_fbs::generate_from_fbs(
        project_root,
        &frontend_config,
        fbs_bytes,
        axiom_path.to_str().unwrap(),
    )
    .await?;

    println!("✅ axiom pull finished successfully.");
    Ok(())
}
