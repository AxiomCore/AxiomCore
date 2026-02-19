use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AccessConfig {
    pub machine_id: String,
    pub referral_code: String,
    pub registered_at: String,
}

impl AccessConfig {
    fn get_path() -> Result<PathBuf> {
        let mut path = directories::BaseDirs::new()
            .context("Could not find home directory")?
            .config_dir()
            .to_path_buf();
        path.push("axiom");
        if !path.exists() {
            std::fs::create_dir_all(&path)?;
        }
        path.push("access.json");
        Ok(path)
    }

    pub async fn load() -> Result<Option<Self>> {
        let path = Self::get_path()?;
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path).await?;
        let config: AccessConfig = serde_json::from_str(&content)?;
        Ok(Some(config))
    }

    pub async fn save(referral_code: &str) -> Result<Self> {
        let path = Self::get_path()?;

        // Generate a new Machine ID if saving for the first time
        // Or preserve existing if we are just updating code (unlikely in this flow, but safe)
        let machine_id = if let Ok(Some(existing)) = Self::load().await {
            existing.machine_id
        } else {
            Uuid::new_v4().to_string()
        };

        let config = AccessConfig {
            machine_id,
            referral_code: referral_code.to_string(),
            registered_at: chrono::Utc::now().to_rfc3339(),
        };

        let content = serde_json::to_string_pretty(&config)?;
        fs::write(path, content).await?;

        Ok(config)
    }

    pub async fn wipe() -> Result<()> {
        let path = Self::get_path()?;
        if path.exists() {
            fs::remove_file(path).await?;
        }
        Ok(())
    }
}
