use acore::evaluator::Evaluator;
use acore::render::materialize;
use acore::security::SecurityManager;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub async fn handle_diff(
    file1: PathBuf,
    file2: Option<PathBuf>,
    format: String,
    variant: Option<String>,
) -> Result<()> {
    // 1. Resolve Old vs New state files
    let (old_file, new_file) = match file2 {
        Some(f2) => (file1, f2),
        None => {
            let lockfile_name = format!("{}.lockfile", file1.display());
            let lockfile = PathBuf::from(&lockfile_name);

            if !lockfile.exists() {
                return Err(anyhow!(
                    "No lockfile found at '{}'.\nPlease run 'axiom build {}' first to establish a baseline.",
                    lockfile.display(),
                    file1.display()
                ));
            }
            (lockfile, file1)
        }
    };

    if format.to_lowercase() == "text" {
        let src1 = fs::read_to_string(&old_file)?;
        let src2 = fs::read_to_string(&new_file)?;
        let changes = acore_diff::diff_text(&src1, &src2);
        println!("{}", acore_diff::renderers::render_text_diff(&changes));
        return Ok(());
    }

    // Helper to evaluate .acore files
    let evaluate_acore = |file: &PathBuf| -> Result<acore::render::MaterializedValue> {
        let mut evaluator = Evaluator::new(SecurityManager::allow_all());
        evaluator.active_variant = variant.clone();

        let uri = format!("file://{}", file.canonicalize()?.display());
        let val = evaluator
            .evaluate_module(&uri)
            .map_err(|e| anyhow!("{}", e.kind))?;

        let mut mat_val = materialize(&mut evaluator, &val, None, &HashMap::new())
            .map_err(|e| anyhow!("{}", e.kind))?;

        if evaluator.is_axiom_project {
            acore::render::prune_empty_fields(&mut mat_val);
            let active_var = variant.clone().unwrap_or_else(|| "default".to_string());
            acore::render::apply_variant_filtering(&mut mat_val, &active_var);
        }
        Ok(mat_val)
    };

    // --- FIXED: Load Lockfile directly from JSON ---
    let mat_old = if old_file.to_string_lossy().ends_with(".lockfile") {
        let content = fs::read_to_string(&old_file)?;
        let json_val: serde_json::Value = serde_json::from_str(&content)?;
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
