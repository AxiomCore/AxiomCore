use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::PathBuf;

pub async fn handle_diff(
    file1: PathBuf,
    file2: Option<PathBuf>,
    format: String,
    variant: Option<String>,
) -> Result<()> {
    let variant_name = variant.as_deref().unwrap_or("default");
    // 1. Resolve Old vs New state files
    let (old_file, new_file) = match file2 {
        Some(f2) => (file1, f2),
        None => {
            let lockfile = super::backend::baseline_path(&file1, variant_name);

            if !lockfile.exists() {
                return Err(anyhow!(
                    "No lockfile found at '{}'.\nPlease run 'axiom build {} --variant {}' first to establish a baseline.",
                    lockfile.display(),
                    file1.display(),
                    variant_name
                ));
            }
            (lockfile, file1)
        }
    };

    if format.to_lowercase() == "text" {
        let mut src1 = fs::read_to_string(&old_file)?;
        let src2 = if old_file.to_string_lossy().ends_with(".lockfile") {
            let expanded = axiom_extractor::evaluate_acore_json(
                new_file.to_str().context("Acore path is not valid UTF-8")?,
                Some(variant_name),
            )?;
            src1 =
                serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(&src1)?)?;
            serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(&expanded)?)?
        } else {
            fs::read_to_string(&new_file)?
        };
        let changes = acore_diff::diff_text(&src1, &src2);
        println!("{}", acore_diff::renderers::render_text_diff(&changes));
        return Ok(());
    }

    // Helper to evaluate .acore files
    let evaluate_acore = |file: &PathBuf| -> Result<acore::render::MaterializedValue> {
        let json = axiom_extractor::evaluate_acore_json(
            file.to_str().context("Acore path is not valid UTF-8")?,
            Some(variant_name),
        )?;
        Ok(acore_diff::value_utils::from_json(&serde_json::from_str(
            &json,
        )?))
    };

    // --- FIXED: Load Lockfile directly from JSON ---
    let mat_old = if old_file.to_string_lossy().ends_with(".lockfile") {
        let content = fs::read_to_string(&old_file)?;
        let json_val: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("Invalid JSON baseline '{}'. Rebuild with 'axiom build {} --variant {}' to replace legacy source-text baselines.", old_file.display(), new_file.display(), variant_name))?;
        acore_diff::value_utils::from_json(&json_val)
    } else {
        evaluate_acore(&old_file)?
    };

    let mat_new = evaluate_acore(&new_file)?;

    // Calculate the structural JSON-Atom diff!
    let patch = acore_diff::diff(&mat_old, &mat_new);

    let output = match format.to_lowercase().as_str() {
        "atom" | "json-atom" => acore_diff::renderers::render_atom(&patch)?,
        "semantic" => acore_diff::renderers::render_semantic_diff(&patch),
        "changelog" | _ => acore_diff::renderers::render_changelog(&patch),
    };

    println!("{}", output);
    Ok(())
}
