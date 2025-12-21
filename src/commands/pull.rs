use anyhow::{anyhow, Context, Result};
use axiom_build::core::client_sdk::flutter::{ensure_deps, generate_from_fbs, install_runtime};
use axiom_lib::config::load_config;
use axiom_lib::contract::AxiomFile;
use ed25519_dalek::{Signature, Verifier, VerifyingKey}; // NEW: For signature verification
use std::fs;
use std::path::PathBuf;

// This public key MUST correspond to the private key used in `axiom build`.
// In a real application, you might embed this during the build of the CLI itself.
pub const AXIOM_PUBLIC_KEY: [u8; 32] = [
    0x1b, 0x20, 0x54, 0x69, 0xc6, 0x7f, 0x3a, 0x1b, 0x73, 0x36, 0x5c, 0xc5, 0x9d, 0xb8, 0x1e, 0x5e,
    0xf2, 0x23, 0x72, 0xc5, 0x10, 0x49, 0x99, 0xf4, 0x5b, 0x9e, 0xa0, 0x27, 0xc4, 0x54, 0x1f, 0x95,
];

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
    if axiom_src_path != axiom_dst_path {
        println!("   Copying '{}' to project root...", axiom_filename);
        fs::copy(&axiom_src_path, &axiom_dst_path)?;
    }

    // --- THIS IS THE NEW LOGIC ---

    // 1. Read the entire signed .axiom file.
    let file_content =
        fs::read(&axiom_dst_path).context("Failed to read .axiom file from project root")?;

    // 2. Separate the payload and signature.
    if file_content.len() < 64 {
        return Err(anyhow!(
            "Invalid .axiom file: too small to contain a signature."
        ));
    }
    let split_index = file_content.len() - 64;
    let payload_bytes = &file_content[..split_index];
    let signature_bytes = &file_content[split_index..];

    // 3. Verify the signature.
    let verifying_key = VerifyingKey::from_bytes(&AXIOM_PUBLIC_KEY)?;
    let signature = Signature::from_bytes(signature_bytes.try_into()?);
    verifying_key.verify(payload_bytes, &signature)
        .context("Contract verification failed! The .axiom file is either corrupted or not signed by a trusted authority.")?;
    println!("✅ Contract signature verified.");

    // 4. Parse the verified payload as JSON.
    let contract: AxiomFile = serde_json::from_slice(payload_bytes)
        .context("Failed to parse the contract payload. The .axiom file may be from an incompatible version.")?;

    // --- END OF NEW LOGIC ---

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
