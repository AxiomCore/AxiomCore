use crate::api::upload_artifact;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;

pub async fn handle_release(file_path: &str) -> anyhow::Result<()> {
    if !Path::new(file_path).exists() {
        anyhow::bail!("Artifact file not found at '{}'", file_path);
    }

    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .expect("Failed to parse progress bar template"),
    );
    pb.set_message(format!("Uploading '{}' to Axiom Cloud...", file_path));

    upload_artifact(file_path).await?;

    pb.finish_with_message(format!("🚀 Successfully released '{}'!", file_path));
    Ok(())
}
