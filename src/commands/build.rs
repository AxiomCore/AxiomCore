use anyhow::{Context, Result};
use axiom_build::core::build::handle_build;
use axiom_lib::config::load_config;

pub async fn build(variant: &str, entrypoint: &str, target: &str) -> Result<()> {
    let config =
        load_config().context("Failed to load `axiom.yaml`. Have you run `axiom init`?")?;
    if !config.variants.contains_key(variant) {
        anyhow::bail!("Variant '{}' not found in `axiom.yaml`", variant);
    }

    handle_build(&config, variant, entrypoint, target).await
}
