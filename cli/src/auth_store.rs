use anyhow::{Context, Result};
use axiom_cloud::CloudClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const TOKEN_FILE: &str = "auth.json";

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AuthData {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix time used for diagnostics and future proactive renewal. The server
    /// remains the authority for expiry and refresh-token rotation.
    #[serde(default)]
    pub access_token_expires_at: Option<u64>,
    // Map of Project Root Path -> Project ID
    #[serde(default)]
    pub projects: HashMap<PathBuf, String>,
}

fn get_config_dir() -> Result<PathBuf> {
    let mut path = dirs::config_dir().context("Could not find config directory")?;
    path.push("axiom");
    fs::create_dir_all(&path)?;
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .context("Could not secure Axiom CLI configuration directory")?;
    Ok(path)
}

pub fn get_auth_file_path() -> Result<PathBuf> {
    let mut path = get_config_dir()?;
    path.push(TOKEN_FILE);
    Ok(path)
}

pub fn save_tokens(access_token: &str, refresh_token: &str, expires_in: u64) -> Result<()> {
    if access_token.trim().is_empty() || refresh_token.trim().is_empty() {
        anyhow::bail!("AxiomCore returned an incomplete CLI session");
    }

    let mut data = load_auth_data().unwrap_or_default();
    data.access_token = access_token.to_string();
    data.refresh_token = refresh_token.to_string();
    data.access_token_expires_at = Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_add(expires_in),
    );
    write_auth_data(&data)
}

pub fn load_auth_data() -> Result<AuthData> {
    let path = get_auth_file_path()?;

    for _ in 0..3 {
        match fs::read_to_string(&path) {
            Ok(content) => {
                let data: AuthData = serde_json::from_str(&content)?;
                return Ok(data);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }

    anyhow::bail!("Failed to read auth file after retries");
}

fn write_auth_data(data: &AuthData) -> Result<()> {
    let path = get_auth_file_path()?;
    let content = serde_json::to_vec_pretty(data)?;
    write_private_file(&path, &content)?;
    Ok(())
}

/// Hydrates a CloudClient with both halves of the native CLI session. The
/// client persists rotated tokens back to this private file, while preserving
/// project-link metadata.
pub fn authenticated_cloud_client() -> Result<CloudClient> {
    let data = load_auth_data().context("Not logged in. Run 'axiom login' first.")?;
    if data.access_token.trim().is_empty() || data.refresh_token.trim().is_empty() {
        anyhow::bail!("Not logged in. Run 'axiom login' first.");
    }

    Ok(CloudClient::with_session(
        data.access_token,
        data.refresh_token,
        get_auth_file_path()?,
    ))
}

fn write_private_file(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("Axiom CLI auth path has no parent directory")?;
    fs::create_dir_all(parent)?;
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(TOKEN_FILE);
    let temporary = parent.join(format!(".{filename}.{}.tmp", std::process::id()));

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .context("Failed to create private auth file")?;
    if let Err(error) = file.write_all(content).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(error).context("Failed to write private auth file");
    }
    fs::rename(&temporary, path).context("Failed to update auth file")?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .context("Failed to secure auth file permissions")?;
    Ok(())
}

pub fn get_project_id(path: &Path) -> Result<Option<String>> {
    let data = load_auth_data().unwrap_or_default();
    // Normalize path to absolute
    let abs_path = fs::canonicalize(path).unwrap_or(path.to_path_buf());
    Ok(data.projects.get(&abs_path).cloned())
}

pub fn link_project(path: &Path, project_id: &str) -> Result<()> {
    let mut data = load_auth_data().unwrap_or_default();
    let abs_path = fs::canonicalize(path).unwrap_or(path.to_path_buf());
    data.projects.insert(abs_path, project_id.to_string());
    write_auth_data(&data)
}
