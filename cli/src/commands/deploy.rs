use acore::evaluator::Evaluator;
use acore::security::SecurityManager;
use anyhow::{anyhow, Result};
use axiom_cloud::CloudClient;
use console::style;
use std::path::PathBuf;

pub async fn handle_deploy_mock_server(file: PathBuf) -> Result<()> {
    println!(
        "{}",
        style("🚀 Deploying Mock Server Configuration...")
            .cyan()
            .bold()
    );

    // 1. Fetch Credentials and Project ID
    let auth_data = crate::auth_store::load_auth_data()
        .map_err(|_| anyhow!("Not logged in. Run 'axiom login' first."))?;

    let current_dir = std::env::current_dir()?;
    let project_slug = crate::auth_store::get_project_id(&current_dir)?.ok_or_else(|| {
        anyhow!("Directory not linked to an Axiom project. Run 'axiom project link'.")
    })?;

    // 2. Evaluate Local .acore file
    println!(
        "⚙️  Compiling local contract definition from {}...",
        file.display()
    );

    let mut evaluator = Evaluator::new(SecurityManager::allow_all());
    evaluator.is_axiom_project = true;

    let abs_path = file
        .canonicalize()
        .map_err(|e| anyhow!("File not found: {}", e))?;
    let module_uri = format!("file://{}", abs_path.display());

    let val = evaluator
        .evaluate_module(&module_uri)
        .map_err(|e| anyhow!("{}", e))?;

    let json_payload =
        acore::render::render_value(&mut evaluator, &val, acore::render::OutputFormat::Json)
            .map_err(|e| anyhow!("Failed to render to JSON: {}", e))?;

    // 3. Upload to Axiom Cloud
    println!(
        "☁️  Uploading mock configuration for project {}...",
        style(&project_slug).cyan()
    );

    let client = CloudClient::new(auth_data.access_token);
    client
        .upload_mock_config(&project_slug, &json_payload)
        .await?;

    println!(
        "✅ {}",
        style("Mock Configuration Deployed Successfully!")
            .green()
            .bold()
    );
    println!("   Frontend developers can now run 'axiom serve' to consume this API.");

    Ok(())
}
