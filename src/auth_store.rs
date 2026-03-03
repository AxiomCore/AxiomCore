use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const TOKEN_FILE: &str = "auth.json";

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AuthData {
    pub access_token: String,
    pub refresh_token: String,
    // Map of Project Root Path -> Project ID
    #[serde(default)]
    pub projects: HashMap<PathBuf, String>,
}

fn get_config_dir() -> Result<PathBuf> {
    let mut path = dirs::config_dir().context("Could not find config directory")?;
    path.push("axiom");
    fs::create_dir_all(&path)?;
    Ok(path)
}

pub fn get_auth_file_path() -> Result<PathBuf> {
    let mut path = get_config_dir()?;
    path.push(TOKEN_FILE);
    Ok(path)
}

pub fn save_tokens(access_token: &str, refresh_token: &str) -> Result<()> {
    let mut data = load_auth_data().unwrap_or_default();
    data.access_token = access_token.to_string();
    data.refresh_token = refresh_token.to_string();
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
    let content = serde_json::to_string_pretty(data)?;
    fs::write(path, content).context("Failed to write auth file")?;
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
