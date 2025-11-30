use crate::config::load_config;
use crate::core::{compiler, extractor, packager};
use anyhow::{Context, Result};
use axiom_client::{generate_sdk_from_ir, GeneratedSdk};
use axiom_extractor::IR;
use indicatif::{ProgressBar, ProgressStyle};

pub async fn handle_build(variant: &str, entrypoint: &str, target: &str) -> Result<()> {
    let config =
        load_config().context("Failed to load `axiom.yaml`. Have you run `axiom init`?")?;
    if !config.variants.contains_key(variant) {
        anyhow::bail!("Variant '{}' not found in `axiom.yaml`", variant);
    }

    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.blue} {msg}")
            .unwrap(),
    );

    // --- Step 1: Run Extractor (in-memory) ---
    pb.set_message("Step 1/4: Extracting schema...");
    let mut ir: IR = extractor::run_extractor(entrypoint).await?;

    if ir.target.is_none() {
        ir.target = Some(target.to_string());
    }

    // --- Step 2: Generate Client Code (in-memory) ---
    pb.set_message("Step 2/4: Generating Rust client code...");
    let GeneratedSdk {
        rust_code,
        fbs_schema,
    } = generate_sdk_from_ir(&ir)?;

    // --- Step 3: Compile Runtime (in-memory) ---
    pb.set_message(format!(
        "Step 3/4: Compiling native runtime for '{}'...",
        target
    ));
    let binary_bytes = compiler::compile_runtime(&rust_code, target, &fbs_schema).await?;

    // --- Step 4: Package Artifact ---
    let output_filename = format!(
        "{}_{}_{}.axiom",
        config.project_id.replace('/', "_"),
        variant,
        config.version
    );
    pb.set_message(format!(
        "Step 4/4: Packaging artifact into '{}'...",
        output_filename
    ));
    packager::package_axiom_file(&ir, &binary_bytes, fbs_schema.as_bytes(), &output_filename)
        .await?;

    pb.finish_with_message(format!("✅ Build complete: Created '{}'", output_filename));

    Ok(())
}
