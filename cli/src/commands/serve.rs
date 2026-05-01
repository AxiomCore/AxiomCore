use acore::evaluator::Evaluator;
use acore::security::SecurityManager;
use anyhow::{anyhow, Result};
use axiom_cloud::CloudClient;
use console::style;
use std::fs;
use std::path::PathBuf;

fn get_cache_paths(project_id: &str) -> Result<(PathBuf, PathBuf)> {
    let mut cache_dir = dirs::config_dir().ok_or_else(|| anyhow!("No config dir"))?;
    cache_dir.push("axiom");
    cache_dir.push("cache");
    cache_dir.push("mocks");

    fs::create_dir_all(&cache_dir)?;

    let json_path = cache_dir.join(format!("{}.json", project_id));
    let etag_path = cache_dir.join(format!("{}.etag", project_id));

    Ok((json_path, etag_path))
}

pub async fn handle_serve(file: Option<PathBuf>, port: u16) -> Result<()> {
    println!(
        "{}",
        style("🚀 Starting Axiom Mock Server...").cyan().bold()
    );

    let config_json = if let Some(path) = file {
        println!("⚙️  Loading local definition from {}...", path.display());

        let mut evaluator = Evaluator::new(SecurityManager::allow_all());
        evaluator.is_axiom_project = true;

        let abs_path = path
            .canonicalize()
            .map_err(|e| anyhow!("File not found: {}", e))?;
        let module_uri = format!("file://{}", abs_path.display());

        let val = evaluator
            .evaluate_module(&module_uri)
            .map_err(|e| anyhow!("{}", e))?;

        acore::render::render_value(&mut evaluator, &val, acore::render::OutputFormat::Json)
            .map_err(|e| anyhow!("Failed to render to JSON: {}", e))?
    } else {
        println!("☁️  No local .acore file provided. Authenticating with Axiom Cloud...");

        let auth_data = crate::auth_store::load_auth_data()
            .map_err(|_| anyhow!("Not logged in. Run 'axiom login' first."))?;

        let current_dir = std::env::current_dir()?;
        let project_slug = crate::auth_store::get_project_id(&current_dir)?.ok_or_else(|| {
            anyhow!("Directory not linked to an Axiom project. Run 'axiom project link'.")
        })?;

        let (json_path, etag_path) = get_cache_paths(&project_slug)?;
        let cached_etag = fs::read_to_string(&etag_path).ok();

        println!(
            "Downloading mock configuration for project {}...",
            style(&project_slug).cyan()
        );

        let client = CloudClient::new(auth_data.access_token);

        match client
            .get_mock_config(&project_slug, cached_etag.as_deref())
            .await?
        {
            axiom_cloud::project::MockResponse::NotModified => {
                println!("⚡ Using cached mock configuration (Not Modified).");
                fs::read_to_string(&json_path)
                    .map_err(|e| anyhow!("Failed to read cache: {}", e))?
            }
            axiom_cloud::project::MockResponse::Data { json, etag } => {
                println!("🔄 Downloaded fresh mock configuration.");
                // Update Cache
                let _ = fs::write(&json_path, &json);
                let _ = fs::write(&etag_path, &etag);
                json
            }
        }
    };

    println!("✅ Mock Configuration Loaded Successfully.\n");

    // Boot the Mock Server via Axum
    if let Err(e) = axiom_mock::start_mock_server(&config_json, port).await {
        return Err(anyhow!("Mock Server Error: {}", e));
    }

    Ok(())
}
