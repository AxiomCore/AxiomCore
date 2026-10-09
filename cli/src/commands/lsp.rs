//! Offline stdio server and a portable, self-contained user-level installation.
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub fn metadata() -> serde_json::Value {
    serde_json::json!({
        "serverVersion": format!("acore/{} (CLI {})", acore::VERSION, env!("CARGO_PKG_VERSION")),
        "protocolVersion": axiom_lib::editor_protocol::PROTOCOL_VERSION,
        "compilerVersion": acore::editor::project::compiler_identity(),
        "editorFeatures": { "virtualDocumentNavigationOptOut": true },
    })
}

pub fn print_version(json: bool) {
    if json {
        println!("{}", metadata());
    } else {
        println!("acore-lsp {} (axiom-editor/v1)", acore::VERSION);
    }
}

/// A copied CLI named acore-lsp is a native stdio LSP executable. This dispatch
/// runs before CLI hooks, cloud access, or telemetry, including metadata probes.
pub async fn native_entry() -> Result<bool> {
    let arguments: Vec<_> = std::env::args_os().collect();
    let name = Path::new(&arguments[0])
        .file_stem()
        .and_then(|s| s.to_str());
    let native = name == Some("acore-lsp");
    let metadata_probe = arguments.len() == 2 && arguments[1] == "--version-json";
    if !native && !metadata_probe {
        return Ok(false);
    }
    let flags = &arguments[1..];
    if flags.is_empty() {
        acore::server::run_server().await;
    } else if flags.len() == 1 && flags[0] == "--version-json" {
        print_version(true);
    } else if flags.len() == 1 && flags[0] == "--version" {
        print_version(false);
    } else if flags.len() == 1 && matches!(flags[0].to_str(), Some("--help" | "-h")) {
        println!(
            "Usage: acore-lsp [--version | --version-json]\nRuns Acore LSP over stdin/stdout."
        );
    } else {
        bail!("Usage: acore-lsp [--version | --version-json]");
    }
    Ok(true)
}

fn digest(path: &Path) -> Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().to_vec())
}

pub fn install() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Cannot locate your home directory")?;
    let mut directory = home;
    for component in [".axiom", "bin"] {
        directory.push(component);
        if directory.is_symlink() {
            bail!(
                "LSP installation directory cannot be a symlink: {}",
                directory.display()
            );
        }
        if !directory.try_exists()? {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&directory)?;
        }
        if !directory.is_dir() {
            bail!(
                "LSP installation requires a directory: {}",
                directory.display()
            );
        }
    }
    let name = if cfg!(windows) {
        "acore-lsp.exe"
    } else {
        "acore-lsp"
    };
    let destination = directory.join(name);
    if destination.is_symlink() || destination.is_dir() {
        bail!(
            "LSP installation output must be a regular file: {}",
            destination.display()
        );
    }
    let executable = std::env::current_exe()?.canonicalize()?;
    if !destination.is_file() || digest(&destination)? != digest(&executable)? {
        // Use the actual server basename even during the compatibility probe.
        let staging = tempfile::Builder::new()
            .prefix(".lsp-install-")
            .tempdir_in(&directory)?;
        let staged = staging.path().join(name);
        std::fs::copy(&executable, &staged).context("Copy the bundled LSP server")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o700))?;
        }
        std::fs::File::open(&staged)?.sync_all()?;
        let probe = std::process::Command::new(&staged)
            .arg("--version-json")
            .output()
            .context("Probe the installed LSP executable")?;
        if !probe.status.success()
            || serde_json::from_slice::<serde_json::Value>(&probe.stdout).ok() != Some(metadata())
        {
            bail!("Bundled LSP compatibility check failed; existing installation was preserved");
        }
        std::fs::rename(&staged, &destination).context("Replace installed LSP server (close editors using it and retry if the OS locks the executable)")?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // An otherwise intact installation may have lost its executable bit.
        if std::fs::metadata(&destination)?.permissions().mode() & 0o100 == 0 {
            std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    println!("Installed Acore LSP: {}", destination.display());
    println!("Use this absolute path in your editor's language-server binary setting.");
    println!("Version metadata: {} --version-json", destination.display());
    println!("Run `axiom install lsp-server` again after updating the CLI to update this server.");
    Ok(destination)
}
