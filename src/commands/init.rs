use anyhow::Context;
use axiom_lib::config::default_config;
use std::path::Path;

pub async fn handle_init() -> anyhow::Result<()> {
    let config_path = Path::new("axiom.yaml");
    if config_path.exists() {
        println!("`axiom.yaml` already exists.");
        return Ok(());
    }

    let default_config = default_config();
    let yaml_string =
        serde_yaml::to_string(&default_config).context("Failed to serialize default config.")?;

    std::fs::write(config_path, yaml_string).context("Failed to write axiom.yaml file.")?;

    println!("✅ Initialized AxiomCore project in `axiom.yaml`.");
    Ok(())
}
