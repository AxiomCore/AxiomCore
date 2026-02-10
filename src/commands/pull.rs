use anyhow::{anyhow, Context, Result};
use axiom_build::core::client_sdk::flutter::{ensure_deps, generate_from_fbs, install_runtime};
use axiom_build::core::unpackager::unpack_axiom_file;
use axiom_cloud::CloudClient;
use axiom_lib::config::load_config;
use std::fs;
use std::path::{Path, PathBuf};

// Entry for `axiom pull --path <file>`
pub async fn handle_pull_path(path: &str, runtime_source: Option<&str>) -> Result<()> {
    // ... existing implementation ...
    // I will just copy the existing logic here but I need to be careful not to overwrite it if I can reuse.
    // Wait, `write_to_file` overwrites.
    // I should implement `handle_pull_package` and include `handle_pull_path` (or keep it if I use replace).
    // Since I'm using `write_to_file`, I must provide FULL content of the file.
    // I'll grab the existing content of `handle_pull_path` from the `view_file` output (Step 2686)
    // and add `handle_pull_package` and necessary imports.

    internal_handle_pull_local(path, runtime_source).await
}

async fn internal_handle_pull_local(path: &str, runtime_source: Option<&str>) -> Result<()> {
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
    let contract =
        unpack_axiom_file(&axiom_src_path).context("Failed to verify the source .axiom file")?;

    std::fs::copy(&axiom_src_path, &axiom_dst_path)
        .context("Failed to copy .axiom file to project root")?;

    println!("   Contract imported to: {}", axiom_dst_path.display());

    post_pull_steps(
        &project_root,
        &axiom_dst_path,
        &contract,
        runtime_source,
        None,
    )
    .await
}

pub async fn handle_pull_package(
    package_ptr: &str,
    version: Option<&str>,
    runtime_source: Option<&str>,
) -> Result<()> {
    let project_root = std::env::current_dir().context("Failed to get current directory")?;

    // 1. Authenticate
    let auth_data = crate::auth_store::load_auth_data()?;
    let token = auth_data.access_token;
    let base_url =
        std::env::var("AXIOM_CLOUD_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let client = CloudClient::new(base_url, token);

    // 2. Resolve Package (assume UUID for now, or fetch latest)
    let project_id = package_ptr;
    let version = version.unwrap_or("latest");

    println!(
        "⬇️  Fetching latest contract for project '{}'...",
        project_id
    );

    // 3. Get Metadata (to get signature and version)
    let metadata = client.get_contract_metadata(project_id, version).await?;
    println!("   Resolved version: {}", metadata.version);

    // 4. Pull Content
    let content = client.pull_contract_content(project_id, version).await?;

    let filename = format!("{}.axiom", project_id); // Or use metadata to name it?
    let axiom_dst_path = project_root.join(&filename);

    fs::write(&axiom_dst_path, &content).context("Failed to write .axiom file")?;
    println!("   Saved contract to: {}", axiom_dst_path.display());

    // 5a. Fetch Public Key
    let keys = client.get_project_keys(project_id).await?;
    let public_key = keys
        .iter()
        .find(|k| k.id == metadata.key_id)
        .map(|k| k.public_key.clone())
        .ok_or_else(|| anyhow!("Public key not found for key_id: {}", metadata.key_id))?;

    // 5b. Create .trust-axiom.json
    let trust_file = project_root.join(".trust-axiom.json");
    let trust_data = serde_json::json!({
        "project_id": metadata.project_id,
        "version": metadata.version,
        "signature": metadata.signature,
        "key_id": metadata.key_id,
        "public_key": public_key,
        "algorithm": "ed25519"
    });
    fs::write(&trust_file, serde_json::to_string_pretty(&trust_data)?)
        .context("Failed to write .trust-axiom.json")?;
    println!("   Created trust file: {}", trust_file.display());

    // 6. Unpack/Verify (Local Unpack)
    let contract = unpack_axiom_file(&axiom_dst_path)?;

    // 7. Post Pull Steps (Deps, Runtime, Gen)
    post_pull_steps(
        &project_root,
        &axiom_dst_path,
        &contract,
        runtime_source,
        Some(&metadata.signature),
    )
    .await
}

async fn post_pull_steps(
    project_root: &Path,
    axiom_path: &Path, // path to .axiom file
    contract: &axiom_lib::contract::AxiomFile,
    runtime_source: Option<&str>,
    signature: Option<&str>,
) -> Result<()> {
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
        project_root,
        contract.header.min_runtime_version,
        runtime_source,
    )
    .await?;

    // 7. Ensure Flutter Dependencies
    ensure_deps::ensure_deps(
        project_root,
        axiom_path.file_name().unwrap().to_str().unwrap(),
    )?;

    // 8. Generate Client Code
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
