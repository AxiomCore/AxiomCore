//! Cloud secret management for `axiom secrets`: names for environments,
//! secrets and versions, the versions view with revisions, version actions
//! with an optional revision, and `rotate`. Backend routes take UUIDs, so
//! every name is resolved to the UUID the dashboard sends. Responses here are
//! metadata only; no secret material is read back or printed.
use anyhow::{bail, Context, Result};
use axiom_cloud::CloudClient;
use reqwest::Method;
use serde_json::{json, Value};
use zeroize::Zeroizing;

/// A canonical 8-4-4-4-12 hexadecimal UUID. Such a value is sent as given.
pub(crate) fn uuid_shaped(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

async fn rows(client: &CloudClient, project: &str, segments: &[&str]) -> Result<Vec<Value>> {
    match client.secret_metadata(project, segments, Method::GET, None).await? {
        Value::Array(rows) => Ok(rows),
        _ => bail!("AXSECRET_CLOUD: invalid response"),
    }
}

/// Resolve `--environment` (a configured environment name or UUID) to its UUID.
pub(crate) async fn environment_id(client: &CloudClient, project: &str, value: &str) -> Result<String> {
    if uuid_shaped(value) {
        // The backend lists IDs in lower case; matching them below needs the same form.
        return Ok(value.to_ascii_lowercase());
    }
    rows(client, project, &["configuration-environments"])
        .await?
        .iter()
        .find(|row| row["slug"].as_str() == Some(value))
        .and_then(|row| row["id"].as_str())
        .filter(|id| uuid_shaped(id))
        .map(str::to_string)
        .with_context(|| format!("unknown environment \"{value}\"; pass a configured environment name or UUID"))
}

/// Resolve a secret (a defined secret name or UUID) to its UUID.
pub(crate) async fn secret_id(client: &CloudClient, project: &str, value: &str) -> Result<String> {
    if uuid_shaped(value) {
        // The backend lists IDs in lower case; matching them below needs the same form.
        return Ok(value.to_ascii_lowercase());
    }
    rows(client, project, &["secrets"])
        .await?
        .iter()
        .find(|row| row["name"].as_str() == Some(value))
        .and_then(|row| row["id"].as_str())
        .filter(|id| uuid_shaped(id))
        .map(str::to_string)
        .with_context(|| format!("unknown secret \"{value}\"; pass a defined secret name or UUID"))
}

/// The secret's row in the environment inventory, which carries its revision.
async fn environment_secret(client: &CloudClient, project: &str, environment: &str, secret: &str) -> Result<Option<Value>> {
    Ok(rows(client, project, &["configuration-environments", environment, "secrets"])
        .await?
        .into_iter()
        .find(|row| row["id"].as_str() == Some(secret)))
}

/// The current environment-secret revision, as the dashboard reads it: the
/// listed revision, or 1 when the secret has no value in the environment yet.
async fn current_revision(client: &CloudClient, project: &str, environment: &str, secret: &str) -> Result<u64> {
    let row = environment_secret(client, project, environment, secret).await?;
    Ok(row.as_ref().and_then(|row| row["revision"].as_u64()).filter(|r| *r > 0).unwrap_or(1))
}

async fn versions(client: &CloudClient, project: &str, environment: &str, secret: &str) -> Result<Vec<Value>> {
    let mut versions = rows(client, project, &["configuration-environments", environment, "secrets", secret, "versions"]).await?;
    versions.sort_by_key(|v| std::cmp::Reverse(v["version"].as_u64().unwrap_or(0)));
    Ok(versions)
}

/// `v3`, `V3` or `3` as a version number; `None` for anything else.
fn version_number(value: &str) -> Option<u64> {
    value.strip_prefix(['v', 'V']).unwrap_or(value).parse().ok()
}

/// Resolve a version (`v<N>`, `<N>` or a version UUID) to its UUID.
async fn version_id(client: &CloudClient, project: &str, environment: &str, secret: &str, value: &str) -> Result<String> {
    if uuid_shaped(value) {
        // The backend lists IDs in lower case; matching them below needs the same form.
        return Ok(value.to_ascii_lowercase());
    }
    let number = version_number(value).with_context(|| format!("invalid version \"{value}\"; pass v<N>, <N> or a version UUID"))?;
    versions(client, project, environment, secret)
        .await?
        .iter()
        .find(|v| v["version"].as_u64() == Some(number))
        .and_then(|v| v["id"].as_str())
        .map(str::to_string)
        .with_context(|| format!("unknown version \"{value}\" in this environment; run `axiom secrets versions` to list versions"))
}

/// `axiom secrets versions`: both the versions list and the environment
/// inventory, so the revision that version actions need is shown.
pub(crate) async fn show_versions(secret: &str, project: &str, environment: &str, as_json: bool) -> Result<()> {
    let client = crate::auth_store::authenticated_cloud_client()?;
    let environment_id = environment_id(&client, project, environment).await?;
    let secret_id = secret_id(&client, project, secret).await?;
    let versions = versions(&client, project, &environment_id, &secret_id).await?;
    let row = environment_secret(&client, project, &environment_id, &secret_id).await?;
    let row = row.unwrap_or(Value::Null);
    let name = row["name"].as_str().unwrap_or(secret).to_string();
    let revision = row["revision"].as_u64();
    if as_json {
        let output = json!({
            "secret": name,
            "secret_id": secret_id,
            "environment": environment,
            "environment_id": environment_id,
            "revision": revision,
            "active_version_id": row["active_version_id"],
            "active_version": row["active_version"],
            "versions": versions,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    let text = |value: &Value| value.as_str().unwrap_or("-").to_string();
    println!("{name} in {environment}");
    println!("  Secret ID:       {secret_id}");
    println!("  Environment ID:  {environment_id}");
    println!("  Revision:        {}", revision.map_or_else(|| "none yet".to_string(), |r| r.to_string()));
    let active = match (row["active_version"].as_u64(), row["active_version_id"].as_str()) {
        (Some(number), Some(id)) => format!("v{number} ({id})"),
        (None, Some(id)) => id.to_string(),
        _ => "none".to_string(),
    };
    println!("  Active version:  {active}");
    println!();
    let scope = format!("--project {project} --environment {environment}");
    if versions.is_empty() {
        println!("No versions in this environment yet.");
        println!("Save and activate a value with: axiom secrets rotate {secret} {scope}");
        return Ok(());
    }
    let labels: Vec<String> = versions.iter().map(|v| format!("v{}", v["version"].as_u64().unwrap_or(0))).collect();
    let label_width = labels.iter().map(String::len).max().unwrap_or(2);
    let status_width = versions.iter().map(|v| text(&v["status"]).len()).max().unwrap_or(6);
    for (version, label) in versions.iter().zip(&labels) {
        let mut line = format!(
            "  {label:<label_width$}  {}  {:<status_width$}  created {}",
            text(&version["id"]),
            text(&version["status"]),
            text(&version["created_at"]),
        );
        if let Some(after) = version["destroy_after"].as_str() {
            line.push_str(&format!("  destroy after {after}"));
        }
        println!("{}", line.trim_end());
    }
    println!();
    let revision = revision.unwrap_or(1);
    match versions.iter().position(|v| v["status"].as_str() == Some("candidate")) {
        Some(index) => println!(
            "Activate with: axiom secrets activate {secret} {} {scope} --revision {revision}",
            labels[index]
        ),
        None => println!("Save and activate a new value with: axiom secrets rotate {secret} {scope}"),
    }
    Ok(())
}

/// activate, disable (`revoke`), rewrap and destroy. Without `--revision`
/// the current revision is read from the environment inventory.
pub(crate) async fn version_action(
    operation: &str,
    secret: &str,
    version: &str,
    project: &str,
    environment: &str,
    revision: Option<u64>,
) -> Result<()> {
    let client = crate::auth_store::authenticated_cloud_client()?;
    let environment_id = environment_id(&client, project, environment).await?;
    let secret_id = secret_id(&client, project, secret).await?;
    let version_id = version_id(&client, project, &environment_id, &secret_id, version).await?;
    let revision = match revision {
        Some(revision) => revision,
        None => {
            let revision = current_revision(&client, project, &environment_id, &secret_id).await?;
            eprintln!("Using current revision {revision}.");
            revision
        }
    };
    let response = client
        .secret_metadata(
            project,
            &["configuration-environments", &environment_id, "secrets", &secret_id, "versions", &version_id, operation],
            Method::POST,
            Some(json!({"expected_revision": revision})),
        )
        .await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

/// Read one value from a masked prompt or raw stdin (every byte kept),
/// bounded to 1..65536 bytes. The buffer is zeroized on drop.
pub(crate) fn read_material(stdin: bool) -> Result<Zeroizing<Vec<u8>>> {
    use std::io::Read;
    let material = if stdin {
        // Sized up front so `read_to_end` never reallocates and leaves an unzeroized copy.
        let mut value = Zeroizing::new(Vec::with_capacity(65_538));
        std::io::stdin().take(65_537).read_to_end(&mut value)?;
        value
    } else {
        Zeroizing::new(dialoguer::Password::new().with_prompt("New secret value").interact()?.into_bytes())
    };
    anyhow::ensure!(
        !material.is_empty() && material.len() <= 65_536,
        "AXSECRET_INVALID: value must contain 1..65536 bytes"
    );
    Ok(material)
}

/// `axiom secrets set`: one candidate version; activation is separate.
pub(crate) async fn set(secret: &str, project: &str, environment: &str, material: Zeroizing<Vec<u8>>) -> Result<()> {
    let client = crate::auth_store::authenticated_cloud_client()?;
    let environment_id = environment_id(&client, project, environment).await?;
    let secret_id = secret_id(&client, project, secret).await?;
    let result = client
        .write_secret_version(project, &environment_id, &secret_id, &material, &uuid::Uuid::new_v4().to_string())
        .await;
    drop(material);
    println!("{}", serde_json::to_string_pretty(&result?)?);
    Ok(())
}

/// `axiom secrets rotate`: save a candidate, then activate it.
pub(crate) async fn rotate(
    secret: &str,
    project: &str,
    environment: &str,
    read: impl FnOnce() -> Result<Zeroizing<Vec<u8>>>,
    as_json: bool,
) -> Result<()> {
    let client = crate::auth_store::authenticated_cloud_client()?;
    // Resolve names before asking for a value, so a typo costs no input.
    let environment_id = environment_id(&client, project, environment).await?;
    let secret_id = secret_id(&client, project, secret).await?;
    let material = read()?;
    let created = client
        .write_secret_version(project, &environment_id, &secret_id, &material, &uuid::Uuid::new_v4().to_string())
        .await;
    drop(material);
    let created = created?;
    let candidate = created["id"].as_str().filter(|id| uuid_shaped(id)).map(str::to_string);
    let Some(candidate) = candidate else {
        bail!(
            "The candidate was saved, but its version ID was not returned, so it was not activated. \
             Find it with: axiom secrets versions {secret} --project {project} --environment {environment}"
        );
    };
    let mut number = None;
    let activated: Result<u64> = async {
        number = versions(&client, project, &environment_id, &secret_id)
            .await?
            .iter()
            .find(|v| v["id"].as_str() == Some(candidate.as_str()))
            .and_then(|v| v["version"].as_u64());
        let revision = current_revision(&client, project, &environment_id, &secret_id).await?;
        client
            .secret_metadata(
                project,
                &["configuration-environments", &environment_id, "secrets", &secret_id, "versions", &candidate, "activate"],
                Method::POST,
                Some(json!({"expected_revision": revision})),
            )
            .await?;
        Ok::<u64, anyhow::Error>(revision + 1)
    }
    .await;
    let label = number.map_or_else(|| candidate.clone(), |n| format!("v{n}"));
    let revision = match activated {
        Ok(revision) => revision,
        Err(error) => bail!(
            "The candidate was saved as {label} ({candidate}), but it could not be activated: {error}\n\
             Finish with: axiom secrets activate {secret} {label} --project {project} --environment {environment}"
        ),
    };
    if as_json {
        let output = json!({
            "secret": secret,
            "secret_id": secret_id,
            "environment": environment,
            "environment_id": environment_id,
            "version": number,
            "version_id": candidate,
            "status": "active",
            "revision": revision,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Saved {label} ({candidate}) and activated it in {environment} (revision now {revision}).");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_uuids_are_recognised() {
        assert_eq!(version_number("v3"), Some(3));
        assert_eq!(version_number("V12"), Some(12));
        assert_eq!(version_number("4"), Some(4));
        assert_eq!(version_number("v"), None);
        assert_eq!(version_number("latest"), None);
        assert!(uuid_shaped("3a4b5c6d-7e8f-4a0b-8c1d-2e3f4a5b6c7d"));
        assert!(uuid_shaped("3A4B5C6D-7E8F-4A0B-8C1D-2E3F4A5B6C7D"));
        assert!(!uuid_shaped("production"));
        assert!(!uuid_shaped("3a4b5c6d7e8f4a0b8c1d2e3f4a5b6c7d"));
    }
}
