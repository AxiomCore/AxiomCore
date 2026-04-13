use anyhow::Result;
use std::fs;
use std::path::PathBuf;

// Import ensure_deps from the build crate
use axiom_build::core::client_sdk::flutter::ensure_deps::ensure_deps;

pub async fn handle_pull_auto(
    contract: Option<PathBuf>,
    contract_config: Option<PathBuf>,
) -> Result<()> {
    let project_root = std::env::current_dir()?;

    // 1. Build the Generator Config Manifest
    let mut codegen_cfg = serde_json::json!({
        "single": true,
        "contracts": {}
    });

    if let Some(cfg_path) = contract_config {
        // MULTI-CONTRACT MODE
        println!(
            "⚙️  Loading multi-contract config from {}...",
            cfg_path.display()
        );
        let content = fs::read_to_string(&cfg_path)?;
        let parsed: serde_json::Value = serde_json::from_str(&content)?;

        codegen_cfg["single"] = serde_json::Value::Bool(false);
        codegen_cfg["contracts"] = parsed["contracts"].clone();
    } else {
        // SINGLE CONTRACT MODE
        let path = contract.unwrap_or_else(|| PathBuf::from(".axiom"));
        if !path.exists() {
            anyhow::bail!("Contract file not found at: {}", path.display());
        }

        codegen_cfg["single"] = serde_json::Value::Bool(true);
        codegen_cfg["contracts"] = serde_json::json!({
            "default": {
                "file": path.to_string_lossy().to_string(),
                "baseUrl": "http://localhost:8000"
            }
        });
    }

    let codegen_cfg_path = project_root.join(".axiom_codegen.json");
    fs::write(
        &codegen_cfg_path,
        serde_json::to_string_pretty(&codegen_cfg)?,
    )?;

    // 2. Run Code Generation
    println!("📦 Ensuring Flutter dependencies and assets...");
    // We just register the files as assets
    if let Some(contracts) = codegen_cfg["contracts"].as_object() {
        for val in contracts.values() {
            if let Some(file_path) = val["file"].as_str() {
                ensure_deps(&project_root, file_path)?;
            }
        }
    }

    println!("⚙️  Generating Flutter SDK code...");
    let frontend_cfg = axiom_lib::config::FrontendConfig {
        framework: "flutter".to_string(),
        output_dir: Some("lib/axiom_generated".to_string()),
    };

    axiom_build::core::client_sdk::flutter::generate_from_fbs::generate_from_fbs(
        &project_root,
        &frontend_cfg,
        &[], // No FlatBuffers for now
        &codegen_cfg_path.to_string_lossy(),
    )
    .await?;

    // Cleanup manifest
    let _ = fs::remove_file(&codegen_cfg_path);

    println!("✅ axiom pull finished successfully.");
    Ok(())
}
