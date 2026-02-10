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
    println!("DEBUG: Loading auth data from: {}", path.display());

    if !path.exists() {
        // Fallback: Check for old "auth_token" file and migrate
        let mut old_path = get_config_dir()?;
        old_path.push("auth_token");
        println!("DEBUG: Checking for migration at: {}", old_path.display());

        if old_path.exists() {
            println!("Migrating legacy auth token...");
            let token = fs::read_to_string(&old_path).context("Failed to read old auth token")?;
            // Create new structure with just access token (user will need to re-login eventually for refresh)
            let data = AuthData {
                access_token: token.trim().to_string(),
                refresh_token: String::new(),
                projects: HashMap::new(),
            };
            write_auth_data(&data)?;
            fs::remove_file(old_path).ok(); // Clean up old file
            return Ok(data);
        }
        anyhow::bail!("No auth data found. Please run 'axiom login'.");
    }

    let content = fs::read_to_string(&path).context("Failed to read auth file")?;
    let data: AuthData = serde_json::from_str(&content).context("Failed to parse auth file")?;
    Ok(data)
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
