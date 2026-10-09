//! Explicit SQLite lifecycle entry points. Pure authoring uses ordinary `database`
//! commands; these native operations require a private deployment binding.
use anyhow::Result;
use clap::Subcommand;
use std::path::PathBuf;
#[derive(Subcommand, Debug)]
pub enum Action {
    /// Validate fields without resolving a provider or loading SQLite
    BindingCheck { deployment: PathBuf },
    /// Observe an explicitly bound local SQLite file without adopting objects
    InspectLive {
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        source: Option<PathBuf>,
    },
    /// Validate a private typed candidate on a full prefix and populated snapshot
    Plan {
        source: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        prefix: Option<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Sign a complete SQLite candidate prefix using an owner-only private key
    Sign {
        bundle: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        allow_destructive: bool,
        #[arg(long)]
        allow_data_dependent: bool,
    },
    /// Verify source, complete prefix and the explicitly trusted signature offline
    ReleaseVerify {
        bundle: PathBuf,
        #[arg(long)]
        trusted_key: String,
    },
    /// Enroll a new empty file; prints the incarnation for the execution binding
    Register {
        bundle: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
    },
    /// Apply the signed prefix with transactional DDL and revision checkpoints
    Apply {
        bundle: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
    },
    /// Observe the enrolled checkpoint without executing DDL
    Status {
        bundle: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
    },
    /// Verify that the requested complete prefix is applied
    Verify {
        bundle: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
    },
    /// Permit native journal recovery and verify the committed schema/ledger checkpoint
    Reconcile {
        bundle: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
    },
}
#[cfg(unix)]
fn private_read(path: &std::path::Path) -> Result<Vec<u8>> {
    use std::{io::Read, os::unix::fs::PermissionsExt};
    let m = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        m.is_file()
            && !m.file_type().is_symlink()
            && m.len() <= 65_536
            && m.permissions().mode() & 0o077 == 0,
        "ADB730: deployment must be an owner-only bounded regular file"
    );
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(65_537)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 65_536, "ADB730: deployment exceeds budget");
    Ok(bytes)
}
#[cfg(unix)]
fn binding(
    path: &std::path::Path,
) -> Result<(
    axiom_database::sqlite::Binding,
    axiom_database::inspection::bindings::NativeProvider,
)> {
    Ok((
        axiom_database::sqlite::Binding::decode(&private_read(path)?)?,
        axiom_database::inspection::bindings::NativeProvider {
            root: path.parent().unwrap_or(std::path::Path::new(".")).into(),
        },
    ))
}
#[cfg(unix)]
fn default_parent(root: &std::path::Path) -> Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let mut path = root.to_path_buf();
    for (index, part) in [".axiom", "database", "v1", "sqlite", "candidates"]
        .iter()
        .enumerate()
    {
        path.push(part);
        if !path.exists() {
            std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        }
        let meta = std::fs::symlink_metadata(&path)?;
        anyhow::ensure!(
            meta.is_dir()
                && !meta.file_type().is_symlink()
                && (index == 0 || meta.permissions().mode() & 0o077 == 0),
            "ADB734: private SQLite candidate parents must be real owner-only directories"
        );
    }
    Ok(path)
}
#[cfg(unix)]
pub fn run(action: &Action) -> Result<()> {
    use axiom_database::sqlite::{self, Fault};
    let value = match action {
        Action::BindingCheck { deployment } => {
            let b = sqlite::Binding::decode(&private_read(deployment)?)?;
            serde_json::json!({"format":"axiom-sqlite-binding-check/v1","profile":b.profile,"valid":true,"effects":"offline-only","migrationAuthority":false})
        }
        Action::InspectLive { deployment, source } => {
            let (b, p) = binding(deployment)?;
            let desired = source
                .as_ref()
                .map(|path| {
                    acore::database::project::load(path, &std::collections::HashMap::new())
                        .map_err(anyhow::Error::msg)
                        .and_then(|(_, input)| {
                            acore::database::compile(&input).map_err(|e| {
                                anyhow::anyhow!(serde_json::to_string(&e).unwrap_or_default())
                            })
                        })
                })
                .transpose()?;
            sqlite::inspect(&b, &p, desired.as_ref().map(|c| &c.contract))?
        }
        Action::Plan {
            source,
            deployment,
            prefix,
            out,
        } => {
            let (root, input) =
                acore::database::project::load(source, &std::collections::HashMap::new())
                    .map_err(anyhow::Error::msg)?;
            let (b, p) = binding(deployment)?;
            let path = if let Some(path) = out {
                path.clone()
            } else {
                let parent = default_parent(&root)?;
                parent.join(format!("candidate-{}", uuid::Uuid::new_v4()))
            };
            let report = sqlite::plan(&b, &p, &input, prefix.as_deref(), &path)?;
            serde_json::json!({"candidate":path,"report":report})
        }
        Action::Sign {
            bundle,
            key_file,
            allow_destructive,
            allow_data_dependent,
        } => sqlite::sign(bundle, key_file, *allow_destructive, *allow_data_dependent)?,
        Action::ReleaseVerify {
            bundle,
            trusted_key,
        } => sqlite::verify_release(bundle, trusted_key)?,
        Action::Register { bundle, deployment }
        | Action::Apply { bundle, deployment }
        | Action::Status { bundle, deployment }
        | Action::Verify { bundle, deployment }
        | Action::Reconcile { bundle, deployment } => {
            let (b, p) = binding(deployment)?;
            let kind = match action {
                Action::Register { .. } => sqlite::Action::Register,
                Action::Apply { .. } => sqlite::Action::Apply,
                Action::Status { .. } => sqlite::Action::Status,
                Action::Verify { .. } => sqlite::Action::Verify,
                _ => sqlite::Action::Reconcile,
            };
            sqlite::operate(&b, &p, bundle, kind, Fault::None)?
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
#[cfg(not(unix))]
pub fn run(_action: &Action) -> Result<()> {
    anyhow::bail!("ADB711: SQLite native lifecycle has no selected profile on this platform")
}
