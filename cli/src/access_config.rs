use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
        let mut path = crate::auth_store::get_config_dir()?;
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

        Self::write(&path, &config)?;

        Ok(config)
    }

    /// access.json holds the referral code and machine id, so it is owner-only
    /// like auth.json (W7-J3). The write replaces the file through a rename, so
    /// a copy left 0644 by an older CLI becomes 0600 on the next write.
    fn write(path: &Path, config: &AccessConfig) -> Result<()> {
        let content = serde_json::to_string_pretty(config)?;
        crate::auth_store::write_private_file(path, content.as_bytes())
    }

    pub async fn wipe() -> Result<()> {
        let path = Self::get_path()?;
        if path.exists() {
            fs::remove_file(path).await?;
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn config() -> AccessConfig {
        AccessConfig {
            machine_id: Uuid::new_v4().to_string(),
            referral_code: "CODE".into(),
            registered_at: "2026-10-31T16:34:00Z".into(),
        }
    }

    #[test]
    fn a_new_access_file_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("access.json");
        AccessConfig::write(&path, &config()).unwrap();
        assert_eq!(mode(&path), 0o600);
        let saved: AccessConfig = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.referral_code, "CODE");
    }

    #[test]
    fn an_older_world_readable_access_file_is_fixed_on_the_next_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("access.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(mode(&path), 0o644);
        let config = config();
        AccessConfig::write(&path, &config).unwrap();
        assert_eq!(mode(&path), 0o600);
        let saved: AccessConfig = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.machine_id, config.machine_id);
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("access.json")], "no temporary file is left");
    }
}
