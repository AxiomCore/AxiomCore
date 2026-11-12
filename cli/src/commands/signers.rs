//! `axiom project signers`: the project's hosted migration signer registry.
use anyhow::{bail, Result};
use axiom_cloud::migrations::{self as api, Signer};
use axiom_cloud::CloudClient;
use axiom_database::developer as db;
use clap::Subcommand;
use std::{io::Write, path::PathBuf};

#[derive(Subcommand, Debug)]
pub enum Action {
    /// List the Ed25519 keys allowed to sign database releases for the project, revoked keys included
    List {
        /// AxiomCore Cloud project slug
        #[arg(long)]
        project: String,
        /// Print machine-readable JSON instead of text
        #[arg(long)]
        json: bool,
    },
    /// Enroll this machine's database release signing key for the project
    ///
    /// Enrolls the key that `axiom database migrate` and `rollback` sign
    /// releases with: AXIOM_DATABASE_SIGNING_KEY when it is set, otherwise the
    /// private per-contract key under ~/.axiom/keys/database (or
    /// AXIOM_DATABASE_KEY_DIR), created on first use. The key signs the proof
    /// `axiom-migration-signer/v1:<project ID>:<public key>`; only the public
    /// key and that signature leave this machine. Enrolling a key that is
    /// already enrolled changes nothing. A revoked key cannot be enrolled again.
    Add {
        /// AxiomCore Cloud project slug
        #[arg(long)]
        project: String,
        /// Name shown in the signer list, 1 to 64 printable characters. Defaults to `axiom-cli@<host name>`
        #[arg(long)]
        label: Option<String>,
        /// Database contract whose local signing key to enroll; keys are per contract ID. Not needed when AXIOM_DATABASE_SIGNING_KEY is set
        #[arg(long, value_name = "CONTRACT")]
        schema: Option<PathBuf>,
    },
    /// Permanently revoke a signing key
    ///
    /// The hosted executor refuses new releases signed by a revoked key and
    /// keeps accepting history it signed earlier. A revoked key cannot be
    /// enrolled again; enroll a new key instead.
    Revoke {
        /// AxiomCore Cloud project slug
        #[arg(long)]
        project: String,
        /// The key's 64-character lowercase hex public key, as `signers list` prints it
        #[arg(long)]
        public_key: String,
    },
}

/// Which local key `add` enrolls, resolved before any network call.
pub(crate) struct LocalKey {
    contract: String,
}

pub async fn handle(action: &Action) -> Result<()> {
    let key = prepare(action)?;
    let client = crate::auth_store::authenticated_cloud_client()?;
    run(&client, action, key, &mut std::io::stdout()).await
}

/// Validates every local input so mistakes fail before authentication.
pub(crate) fn prepare(action: &Action) -> Result<Option<LocalKey>> {
    match action {
        Action::List { .. } => Ok(None),
        Action::Revoke { public_key, .. } => {
            if !api::is_lower_hex(public_key, 32) {
                bail!("--public-key must be 64 lowercase hex characters");
            }
            Ok(None)
        }
        Action::Add { label, schema, .. } => {
            if let Some(label) = label {
                api::check_label(label)?;
            }
            let contract = match schema {
                Some(schema) => {
                    super::database_dev::load_existing(schema)?
                        .0
                        .compiled
                        .identity
                        .name
                }
                None if std::env::var_os("AXIOM_DATABASE_SIGNING_KEY").is_some() => String::new(),
                None => bail!(
                    "ADB1109: the local signing key is per database contract; pass --schema <CONTRACT> (or set AXIOM_DATABASE_SIGNING_KEY)"
                ),
            };
            Ok(Some(LocalKey { contract }))
        }
    }
}

pub(crate) async fn run(
    client: &CloudClient,
    action: &Action,
    key: Option<LocalKey>,
    out: &mut dyn Write,
) -> Result<()> {
    match action {
        Action::List { project, json } => {
            let signers = client.migration_signers(project).await?;
            if *json {
                writeln!(out, "{}", serde_json::json!({ "signers": signers }))?;
            } else if signers.is_empty() {
                writeln!(out, "No signing keys are enrolled for {project}.")?;
            } else {
                for signer in &signers {
                    writeln!(out, "{}", line(signer))?;
                }
            }
        }
        Action::Add { project, label, .. } => {
            let key = key.expect("prepared key");
            let label = label.clone().unwrap_or_else(default_label);
            let project_id = client.project_id(project).await?;
            let public_key = db::local_public_key(&key.contract).map_err(engine_error)?;
            let message = api::signer_proof_message(&project_id, &public_key);
            let (signed_by, proof) =
                db::sign_message_with_local_key(&key.contract, message.as_bytes())
                    .map_err(engine_error)?;
            if signed_by != public_key {
                bail!("ADB1109: the local signing key changed while enrolling; retry");
            }
            let signer = client
                .add_migration_signer(
                    project,
                    &public_key,
                    &label,
                    &proof,
                    &uuid::Uuid::new_v4().to_string(),
                )
                .await?;
            if signer.label == label {
                writeln!(
                    out,
                    "Enrolled signing key {} as \"{}\" for {project}.",
                    signer.public_key,
                    printable(&signer.label)
                )?;
            } else {
                writeln!(
                    out,
                    "Signing key {} is already enrolled for {project} as \"{}\".",
                    signer.public_key,
                    printable(&signer.label)
                )?;
            }
        }
        Action::Revoke {
            project,
            public_key,
        } => {
            let signer = client
                .revoke_migration_signer(project, public_key, &uuid::Uuid::new_v4().to_string())
                .await?;
            writeln!(
                out,
                "Revoked signing key {} (\"{}\") for {project}. History it signed stays valid; new releases signed by it are refused.",
                signer.public_key,
                printable(&signer.label)
            )?;
        }
    }
    Ok(())
}

fn engine_error(error: axiom_database::Error) -> anyhow::Error {
    anyhow::anyhow!(error.to_string())
}
fn printable(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).collect()
}
fn line(signer: &Signer) -> String {
    let mut text = format!(
        "{}  {}  added {}",
        signer.public_key,
        printable(&signer.label),
        printable(&signer.added_at)
    );
    if let Some(revoked) = &signer.revoked_at {
        text.push_str(&format!("  revoked {}", printable(revoked)));
    }
    text
}

/// `axiom-cli@<host>`: the first host name label, reduced to ASCII letters,
/// digits, `-` and `_`. It never includes a user name or a path.
pub(crate) fn default_label() -> String {
    label_for_host(&hostname().unwrap_or_default())
}
fn label_for_host(host: &str) -> String {
    let host = host
        .split('.')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(48)
        .collect::<String>()
        .to_ascii_lowercase();
    if host.is_empty() {
        "axiom-cli".into()
    } else {
        format!("axiom-cli@{host}")
    }
}
/// The machine's host name from the `hostname` utility, which exists on
/// macOS, Linux and Windows, falling back to the usual environment variables.
fn hostname() -> Option<String> {
    std::process::Command::new("hostname")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_labels_are_safe_and_bounded() {
        assert_eq!(label_for_host("Yash-Mac-mini.local"), "axiom-cli@yash-mac-mini");
        assert_eq!(label_for_host("../../etc\u{1b}[31m"), "axiom-cli");
        assert_eq!(label_for_host(""), "axiom-cli");
        assert!(label_for_host(&"h".repeat(300)).len() <= 64);
        api::check_label(&default_label()).unwrap();
    }

    #[test]
    fn local_input_errors_precede_the_network() {
        assert!(prepare(&Action::Revoke {
            project: "acme".into(),
            public_key: "ABC".into()
        })
        .is_err());
        assert!(prepare(&Action::Add {
            project: "acme".into(),
            label: Some(String::new()),
            schema: None
        })
        .is_err());
    }
}
