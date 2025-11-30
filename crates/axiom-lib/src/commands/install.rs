// This code is mostly the same, but imports are local to the crate
use anyhow::{Context, Result};
use axiom_cloud::CloudClient;
use indicatif::ProgressBar;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub async fn handle_install(extractor_name: &str) -> Result<()> {
    let client = CloudClient::new("dummy_token".to_string());
    let home = dirs::home_dir().context("Could not determine home directory.")?;
    let plugins_dir = home.join(".axiom/plugins");
    fs::create_dir_all(&plugins_dir)?;

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let target_os_arch = format!("{}-{}", os, arch);

    let pb = ProgressBar::new_spinner();
    pb.set_message(format!(
        "📦 Downloading extractor '{}' for {}...",
        extractor_name, target_os_arch
    ));
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    let bytes = client
        .download_plugin(extractor_name, "latest", &target_os_arch)
        .await
        .context(format!("Failed to download '{}' extractor", extractor_name))?;

    let plugin_path = plugins_dir.join(extractor_name);
    fs::write(&plugin_path, bytes).context(format!(
        "Failed to save plugin to {}",
        plugin_path.display()
    ))?;

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&plugin_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&plugin_path, perms)?;
    }

    pb.finish_with_message(format!(
        "✅ Extractor '{}' installed successfully.",
        extractor_name
    ));
    Ok(())
}
