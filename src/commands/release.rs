use axiom_cloud::CloudClient;
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

    let auth_data = crate::auth_store::load_auth_data()?;
    let token = auth_data.access_token;

    // We need project_id and version.
    // BUT release_artifact (which was in old lib.rs) is GONE.
    // I replaced it with `upload_contract`.
    // The old `release_artifact` took `path`.
    // `upload_contract` takes `project_id, version, path`.
    // I need to parse project_id and version from the filename or axiom.acore?
    // cli release command takes file_path.
    // I should probably read the header of the .axiom file to get project_id and version!
    // `axiom_build::core::unpackager::unpack_axiom_bytes` can read it.

    let file_bytes = std::fs::read(Path::new(file_path))?;
    let contract = axiom_build::core::unpackager::unpack_axiom_bytes(&file_bytes)?;

    let project_id = contract.project.project_id;
    let version = contract.project.version;

    let client = CloudClient::new(token);
    client
        .upload_contract(&project_id, &version, Path::new(file_path))
        .await?;

    pb.finish_with_message(format!("🚀 Successfully released '{}'!", file_path));
    Ok(())
}
