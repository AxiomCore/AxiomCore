use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::{
    io::IsTerminal,
    path::{Path, PathBuf},
};

/// Releases are scoped to an Axiom Cloud project, never to the `project_id`
/// string embedded in an `.axiom` artifact alone. A directory link is reused
/// silently; a first-time interactive release explicitly creates or selects a
/// project and saves that choice for all future release commands.
pub async fn handle_release(
    file_path: &str,
    project_override: Option<&str>,
    version_override: Option<&str>,
    source_path: Option<&Path>,
    variant: &str,
) -> anyhow::Result<()> {
    if !Path::new(file_path).exists() {
        anyhow::bail!("Artifact file not found at '{}'", file_path);
    }

    let mut artifact_path = PathBuf::from(file_path);
    let mut file_bytes = std::fs::read(&artifact_path)?;
    let contract = axiom_lib::unpackager::unpack_axiom_bytes(&file_bytes)?;
    let contract_version = contract.project.version.clone();

    let client = crate::auth_store::authenticated_cloud_client().map_err(|error| {
        anyhow::anyhow!(
            "You are not logged in to Axiom Cloud. Run `axiom login`, then rerun `axiom release` or `axiom build --release`. Details: {error}"
        )
    })?;
    let project_slug = crate::commands::project::resolve_release_project(
        &client,
        project_override,
        contract.project.project_id.as_str(),
    )
    .await?;
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .expect("Failed to parse progress bar template"),
    );
    pb.set_message(format!("Uploading '{}' to Axiom Cloud...", file_path));
    let mut artifact_hash = hex_sha256(&file_bytes);
    let mut version = requested_version(version_override, &contract_version)?;

    if version != contract_version {
        let source = source_path.ok_or_else(|| anyhow::anyhow!(
            "`--version {version}` differs from the compiled contract version `{}`. Re-run from the contract directory so Axiom can update and rebuild `axiom.acore`.",
            contract_version
        ))?;
        let rebuilt_version;
        (artifact_path, file_bytes, rebuilt_version) =
            rebuild_with_version(source, &version, variant).await?;
        if rebuilt_version != version {
            anyhow::bail!(
                "Axiom rebuilt `{}` but it still declares version `{rebuilt_version}` instead of `{version}`. No release was sent.",
                source.display()
            );
        }
        artifact_hash = hex_sha256(&file_bytes);
    }

    loop {
        // A release may have succeeded remotely after the terminal lost its
        // response. Find an identical stored artifact before attempting a new
        // immutable version. This also remembers a prior interactive version
        // selection without developers editing project metadata.
        if let Some(existing) =
            find_existing_artifact(&client, &project_slug, &artifact_hash).await?
        {
            if existing.signature.trim().is_empty() {
                client.sign_contract(&existing.id).await.map_err(|error| anyhow::anyhow!(
                    "The exact artifact is already stored as `{}`, but it is not signed yet: {error}. Retry after the signing-key issue is resolved; do not change the project ID or version.",
                    existing.version
                ))?;
            }
            pb.finish_with_message(format!(
                "✅ '{}' is already released to '{}' as {} (idempotent retry).",
                artifact_path.display(),
                project_slug,
                existing.version
            ));
            return Ok(());
        }

        match client
            .upload_contract(&project_slug, &version, &artifact_path)
            .await
        {
            Ok(_) => break,
            Err(error) if is_duplicate_version_error(&error) => {
                if recover_duplicate_release(&client, &project_slug, &version, &artifact_hash)
                    .await?
                {
                    break;
                }
                pb.finish_and_clear();
                version = prompt_next_version(&client, &project_slug, &version).await?;
                let source = source_path.ok_or_else(|| anyhow::anyhow!(
                    "Version `{version}` was selected, but no `.acore` source file is available to update. Re-run from the contract directory so Axiom can rebuild the artifact."
                ))?;
                let rebuilt_version;
                (artifact_path, file_bytes, rebuilt_version) =
                    rebuild_with_version(source, &version, variant).await?;
                artifact_hash = hex_sha256(&file_bytes);
                if rebuilt_version != version {
                    anyhow::bail!(
                        "Axiom rebuilt `{}` but it still declares version `{}` instead of `{version}`. No release was sent.",
                        source.display(),
                        rebuilt_version
                    );
                }
                pb.set_message(format!(
                    "Uploading '{}' to Axiom Cloud...",
                    artifact_path.display()
                ));
            }
            Err(error) => return Err(error),
        }
    }

    pb.finish_with_message(format!(
        "🚀 Successfully released '{}' to '{}' as {}!",
        artifact_path.display(),
        project_slug,
        version
    ));
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn requested_version(
    override_value: Option<&str>,
    artifact_version: &str,
) -> anyhow::Result<String> {
    let version = override_value.unwrap_or(artifact_version).trim();
    if version.is_empty() {
        anyhow::bail!(
            "This artifact has no release version. Pass `--version v0.1.0` once, or declare a version in the contract."
        );
    }
    Ok(version.to_string())
}

fn is_duplicate_version_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("contracts_project_id_version_key")
        || message.contains("sqlstate 23505")
        || message.contains("duplicate key value")
        || message.contains("already uses this project version")
        || message.contains("already uses this immutable project version")
        || message.contains("already uses this immutable release version")
}

async fn rebuild_with_version(
    source_path: &Path,
    version: &str,
    variant: &str,
) -> anyhow::Result<(PathBuf, Vec<u8>, String)> {
    update_source_version(source_path, version)?;
    println!(
        "✅ Updated {} to version `{version}`. Rebuilding release artifact...",
        source_path.display()
    );

    let source = source_path.to_str().ok_or_else(|| {
        anyhow::anyhow!(
            "Contract path is not valid UTF-8: {}",
            source_path.display()
        )
    })?;
    let artifact = axiom_build::core::build::handle_build(source, variant, "", "", None)
        .await
        .map_err(|error| anyhow::anyhow!("Could not rebuild {}: {error}", source_path.display()))?;
    let artifact_path = PathBuf::from(artifact);
    let artifact_bytes = std::fs::read(&artifact_path)?;
    let contract = axiom_lib::unpackager::unpack_axiom_bytes(&artifact_bytes)?;
    Ok((artifact_path, artifact_bytes, contract.project.version))
}

fn update_source_version(source_path: &Path, version: &str) -> anyhow::Result<()> {
    if version.trim().is_empty()
        || !version.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '+')
        })
    {
        anyhow::bail!("Release versions may only contain letters, numbers, `.`, `_`, `-`, or `+`.");
    }

    let source = std::fs::read_to_string(source_path)
        .map_err(|error| anyhow::anyhow!("Could not read {}: {error}", source_path.display()))?;
    let project_version = Regex::new(r#"(?s)(\bproject\s*\{.*?\bversion\s*=\s*)\"[^\"\r\n]*\""#)
        .expect("the project version expression is valid");
    if !project_version.is_match(&source) {
        anyhow::bail!(
            "Could not find `project {{ ... version = \"...\" }}` in {}. No source file was changed.",
            source_path.display()
        );
    }
    let updated = project_version
        .replace(&source, |captures: &regex::Captures| {
            format!(r#"{}"{}""#, &captures[1], version)
        })
        .into_owned();
    write_atomic(source_path, updated.as_bytes())
}

fn write_atomic(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("axiom.acore");
    let temporary = parent.join(format!(".{filename}.{}.tmp", std::process::id()));
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

async fn find_existing_artifact(
    client: &axiom_cloud::CloudClient,
    project_slug: &str,
    artifact_hash: &str,
) -> anyhow::Result<Option<axiom_cloud::contract::ContractMetadata>> {
    let contracts = client.list_contracts(project_slug).await.map_err(|error| {
        anyhow::anyhow!(
            "Could not inspect existing releases for `{project_slug}` before uploading: {error}"
        )
    })?;
    Ok(contracts
        .into_iter()
        .find(|contract| contract.sha256.eq_ignore_ascii_case(artifact_hash)))
}

async fn recover_duplicate_release(
    client: &axiom_cloud::CloudClient,
    project_slug: &str,
    version: &str,
    artifact_hash: &str,
) -> anyhow::Result<bool> {
    let existing = client
        .get_contract_metadata(project_slug, version)
        .await
        .map_err(|error| anyhow::anyhow!(
            "A release version collision was reported for `{project_slug}` / `{version}`, but its existing release could not be inspected: {error}. Rebuild and retry; do not change a project ID manually."
        ))?;

    if !existing.sha256.eq_ignore_ascii_case(artifact_hash) {
        return Ok(false);
    }

    if existing.signature.trim().is_empty() {
        client.sign_contract(&existing.id).await.map_err(|error| anyhow::anyhow!(
            "The exact artifact is already stored as `{version}`, but it is not signed yet: {error}. Retry after the signing-key issue is resolved; do not change the project ID or version."
        ))?;
    }

    Ok(true)
}

async fn prompt_next_version(
    client: &axiom_cloud::CloudClient,
    project_slug: &str,
    occupied_version: &str,
) -> anyhow::Result<String> {
    let existing_versions = client
        .list_contracts(project_slug)
        .await
        .map_err(|error| anyhow::anyhow!(
            "Release version `{occupied_version}` already exists, but Axiom Cloud could not load existing versions to suggest the next one: {error}"
        ))?;
    let used_versions: Vec<String> = existing_versions
        .iter()
        .map(|contract| contract.version.clone())
        .collect();
    let recommended = next_available_version(occupied_version, &used_versions);

    if !std::io::stdin().is_terminal() || std::env::var_os("CI").is_some() {
        anyhow::bail!(
            "Release version `{occupied_version}` already exists in `{project_slug}` with different contract bytes. Recommended next version: `{recommended}`. Re-run with `--version {recommended}`. Do not change the project ID."
        );
    }

    println!(
        "\nRelease version `{occupied_version}` is already used by a different contract in `{project_slug}`."
    );
    println!("Suggested next version: `{recommended}`.");
    let selected: String =
        dialoguer::Input::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt("Release version")
            .default(recommended)
            .interact_text()
            .map_err(|error| anyhow::anyhow!("Could not read release version: {error}"))?;
    let selected = selected.trim();
    if selected.is_empty() {
        anyhow::bail!("Release cancelled: version cannot be empty.");
    }
    Ok(selected.to_string())
}

fn next_available_version(current: &str, used_versions: &[String]) -> String {
    let is_used = |version: &str| used_versions.iter().any(|existing| existing == version);
    let (prefix, numeric) = current
        .strip_prefix('v')
        .map(|value| ("v", value))
        .unwrap_or(("", current));
    let mut parts = numeric.split('.');
    if let (Some(major), Some(minor), Some(patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    {
        if let (Ok(major), Ok(minor), Ok(mut patch)) = (
            major.parse::<u64>(),
            minor.parse::<u64>(),
            patch.parse::<u64>(),
        ) {
            loop {
                patch = patch.saturating_add(1);
                let candidate = format!("{prefix}{major}.{minor}.{patch}");
                if !is_used(&candidate) {
                    return candidate;
                }
            }
        }
    }

    let mut suffix = 2_u64;
    loop {
        let candidate = format!("{current}-{suffix}");
        if !is_used(&candidate) {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{is_duplicate_version_error, next_available_version, update_source_version};
    use std::fs;

    #[test]
    fn recommends_the_next_unused_semver_patch() {
        assert_eq!(
            next_available_version("v0.0.1", &["v0.0.1".to_string(), "v0.0.2".to_string()]),
            "v0.0.3"
        );
        assert_eq!(
            next_available_version("nightly", &["nightly".to_string(), "nightly-2".to_string()]),
            "nightly-3"
        );
    }

    #[test]
    fn recognises_the_backend_immutable_version_conflict() {
        let error = anyhow::anyhow!(
            "a different contract already uses this immutable release version; choose the suggested version to rebuild and deploy"
        );
        assert!(is_duplicate_version_error(&error));
    }

    #[test]
    fn updates_only_the_project_version_in_source() {
        let path = std::env::temp_dir().join(format!(
            "axiom-release-version-{}-{}.acore",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::write(
            &path,
            "amends \"axiom-fastapi:main.py:app\"\n\nproject {\n  id = \"demo-app\"\n  version = \"v0.0.1\"\n}\n",
        )
        .expect("write test source");

        update_source_version(&path, "v0.0.2").expect("update source version");
        let updated = fs::read_to_string(&path).expect("read updated source");
        assert!(updated.contains("version = \"v0.0.2\""));
        assert!(updated.contains("id = \"demo-app\""));
        let _ = fs::remove_file(path);
    }
}
