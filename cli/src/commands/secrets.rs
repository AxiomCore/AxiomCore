//! Secret commands deliberately bypass generic CLI error payloads and value
//! serialization. Only the set command accepts material, through prompt/stdin.
use anyhow::{ensure, Context, Result};
use axiom_server::{
    secret_cloud::{CloudSession, LeaseRequirements, LeaseTicket, ProofKey},
    secret_runtime::RuntimeSecrets,
};
use clap::Subcommand;
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

#[derive(Clone, Debug, Subcommand)]
pub enum Action {
    Init {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Check {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        resolve: bool,
    },
    Inspect {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        environment: String,
    },
    /// Create an isolated logical scope; production scopes must be protected.
    CreateEnvironment {
        #[arg(long)]
        project: String,
        #[arg(long)]
        slug: String,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        protected: bool,
        #[arg(long)]
        expires_at: Option<String>,
    },
    /// Define a name and purpose. No secret value is accepted here.
    Define {
        name: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        purpose: String,
        #[arg(long, default_value = "")]
        description: String,
    },
    Bind {
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        file: PathBuf,
    },
    Workloads {
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
    },
    RegisterWorkload {
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        file: PathBuf,
    },
    DisableWorkload {
        id: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
    },
    Grants {
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
    },
    Grant {
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        file: PathBuf,
    },
    RevokeGrant {
        id: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
    },
    UpdateEnvironment {
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        file: PathBuf,
    },
    Versions {
        secret: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
    },
    Destroy {
        secret: String,
        version: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        revision: u64,
    },
    Requirements {
        plan: PathBuf,
    },
    Set {
        secret: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        stdin: bool,
    },
    List {
        #[arg(long)]
        project: String,
    },
    Environments {
        #[arg(long)]
        project: String,
    },
    Audit {
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        before: Option<u64>,
    },
    Activate {
        secret: String,
        version: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        revision: u64,
    },
    /// Rewrap an encrypted version with the configured KMS primary key.
    Rewrap {
        secret: String,
        version: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        revision: u64,
    },
    Revoke {
        secret: String,
        version: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        #[arg(long)]
        revision: u64,
    },
}
pub async fn runtime(
    root: &Path,
    environment: &str,
    plan: &axiom_lib::backend::ServerPlan,
) -> Result<RuntimeSecrets> {
    ensure!(cfg!(all(target_os="macos",target_arch="aarch64")),"AXSECRET_UNSUPPORTED_TARGET: select a separately qualified protected runtime before authenticating a secret source");
    let selected = RuntimeSecrets::inspect(root, environment)?;
    let cloud = if let Some(config) = &selected.cloud {
        let requirements = LeaseRequirements {
            project_id: config.project_id.clone(),
            environment_id: config.environment_id.clone(),
            plan_sha256: plan.sha256()?,
            bindings: config.bindings.values().cloned().collect(),
        };
        let base = axiom_cloud::cloud_base_url()?;
        if let Ok(identity) = std::env::var("AXIOM_SECRET_WORKLOAD_TOKEN") {
            let identity = Zeroizing::new(identity);
            Some(CloudSession::workload(&base, &identity, &requirements)?)
        } else if config.identity == axiom_lib::secrets::CloudIdentity::GoogleMetadata {
            Some(CloudSession::workload_metadata(&base, &requirements)?)
        } else {
            let client = crate::auth_store::authenticated_cloud_client()?;
            let key = ProofKey::default();
            let input = axiom_cloud::secrets::DevelopmentRequest {
                project_id: &requirements.project_id,
                environment_id: &requirements.environment_id,
                plan_sha256: &requirements.plan_sha256,
                bindings: &requirements.bindings,
            };
            let mut ticket = client
                .secret_development_session(&config.project_slug, input, |uri, token| {
                    Ok(key.proof("POST", uri, token)?)
                })
                .await?;
            Some(
                CloudSession::from_ticket(
                    &base,
                    key,
                    LeaseTicket {
                        id: std::mem::take(&mut ticket.id),
                        access_token: std::mem::take(&mut ticket.access_token),
                        token_type: std::mem::take(&mut ticket.token_type),
                        expires_at: std::mem::take(&mut ticket.expires_at),
                    },
                )?
                .with_identity_provider(std::sync::Arc::new(development_identity))?,
            )
        }
    } else {
        None
    };
    Ok(RuntimeSecrets::from_bindings(
        root,
        &selected,
        cloud
            .as_ref()
            .map(|c| c as &dyn axiom_server::secret_runtime::CloudSecretSource),
    )?)
}
fn development_identity() -> axiom_server::Result<Zeroizing<String>> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let client = crate::auth_store::authenticated_cloud_client().map_err(|_| {
                    axiom_server::BoundaryError {
                        cleanup: None,
                        validation: None,
                        typed_failure: None,
                        status: 403,
                        code: "AXSECRET_IDENTITY",
                        path: "secrets".into(),
                        message: "native development identity is unavailable".into(),
                    }
                })?;
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| identity_unavailable())?;
                runtime
                    .block_on(client.secret_identity_token())
                    .map_err(|_| identity_unavailable())
            })
            .join()
            .map_err(|_| identity_unavailable())?
    })
}
fn identity_unavailable() -> axiom_server::BoundaryError {
    axiom_server::BoundaryError {
        cleanup: None,
        validation: None,
        typed_failure: None,
        status: 503,
        code: "AXSECRET_IDENTITY",
        path: "secrets".into(),
        message: "native development identity renewal is unavailable".into(),
    }
}

async fn metadata(
    project: &str,
    segments: &[&str],
    method: reqwest::Method,
    body: Option<serde_json::Value>,
) -> Result<()> {
    let client = crate::auth_store::authenticated_cloud_client()?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &client
                .secret_metadata(project, segments, method, body)
                .await?
        )?
    );
    Ok(())
}
pub async fn handle(action: &Action) -> Result<()> {
    match action {
        Action::Requirements { plan } => {
            let plan: axiom_lib::backend::ServerPlan =
                axiom_server::serve::read_json(plan, 16_777_216)?;
            let consumers=plan.resources.iter().map(|(name,resource)|serde_json::json!({"consumer":format!("resource:{name}"),"secrets":resource.secrets})).collect::<Vec<_>>();
            println!(
                "{}",
                serde_json::json!({"planSha256":plan.sha256()?,"requirements":plan.secrets,"consumers":consumers})
            );
        }
        Action::CreateEnvironment {
            project,
            slug,
            kind,
            protected,
            expires_at,
        } => {
            metadata(project,&["configuration-environments"],reqwest::Method::POST,Some(serde_json::json!({"slug":slug,"kind":kind,"protected":protected,"expires_at":expires_at}))).await?;
        }
        Action::Define {
            name,
            project,
            purpose,
            description,
        } => {
            metadata(
                project,
                &["secrets"],
                reqwest::Method::POST,
                Some(serde_json::json!({"name":name,"purpose":purpose,"description":description})),
            )
            .await?;
        }
        Action::Bind {
            project,
            environment,
            file,
        }
        | Action::RegisterWorkload {
            project,
            environment,
            file,
        }
        | Action::Grant {
            project,
            environment,
            file,
        }
        | Action::UpdateEnvironment {
            project,
            environment,
            file,
        } => {
            let bytes = axiom_server::secret_runtime::read_private(
                file.parent()
                    .filter(|path| !path.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new(".")),
                Path::new(file.file_name().context("metadata file required")?),
                false,
            )?;
            ensure!(
                bytes.len() <= 32768,
                "AXSECRET_INVALID: metadata input exceeds bounds"
            );
            let body = serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("AXSECRET_INVALID: invalid reference metadata"))?;
            let (part, method) = match action {
                Action::Bind { .. } => ("secret-bindings", reqwest::Method::PUT),
                Action::RegisterWorkload { .. } => ("secret-workloads", reqwest::Method::POST),
                Action::Grant { .. } => ("secret-grants", reqwest::Method::POST),
                _ => ("", reqwest::Method::PATCH),
            };
            let mut parts = vec!["configuration-environments", environment.as_str()];
            if !part.is_empty() {
                parts.push(part);
            } else {
                parts.push("");
            }
            metadata(project, &parts, method, Some(body)).await?;
        }
        Action::Workloads {
            project,
            environment,
        }
        | Action::Grants {
            project,
            environment,
        } => {
            let part = if matches!(action, Action::Workloads { .. }) {
                "secret-workloads"
            } else {
                "secret-grants"
            };
            metadata(
                project,
                &["configuration-environments", environment, part],
                reqwest::Method::GET,
                None,
            )
            .await?;
        }
        Action::DisableWorkload {
            id,
            project,
            environment,
        }
        | Action::RevokeGrant {
            id,
            project,
            environment,
        } => {
            let (part, operation) = if matches!(action, Action::DisableWorkload { .. }) {
                ("secret-workloads", "disable")
            } else {
                ("secret-grants", "revoke")
            };
            metadata(
                project,
                &[
                    "configuration-environments",
                    environment,
                    part,
                    id,
                    operation,
                ],
                reqwest::Method::POST,
                None,
            )
            .await?;
        }
        Action::Versions {
            secret,
            project,
            environment,
        } => {
            metadata(
                project,
                &[
                    "configuration-environments",
                    environment,
                    "secrets",
                    secret,
                    "versions",
                ],
                reqwest::Method::GET,
                None,
            )
            .await?;
        }
        Action::Destroy {
            secret,
            version,
            project,
            environment,
            revision,
        } => {
            metadata(
                project,
                &[
                    "configuration-environments",
                    environment,
                    "secrets",
                    secret,
                    "versions",
                    version,
                    "destroy",
                ],
                reqwest::Method::POST,
                Some(serde_json::json!({"expected_revision":revision})),
            )
            .await?;
        }
        Action::Init { root } => {
            ensure!(root.is_dir(), "project root is unavailable");
            let path = root.join("AxiomSecrets.toml");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .context("AxiomSecrets.toml already exists or cannot be created")?;
            use std::io::Write;
            file.write_all(b"format = \"axiom-secret-bindings/v1\"\n\n[environments.development.secrets]\n\n[environments.development.configuration]\n")?;
            println!("Created reference-only AxiomSecrets.toml. Add explicit sources and keep value files owner-only.");
        }
        Action::Inspect { root, environment } => {
            let selected = RuntimeSecrets::inspect(root, environment)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"environment":environment,"secrets":selected.secrets,"configuration":selected.configuration,"sandboxes":selected.sandboxes,"cloud":selected.cloud})
                )?
            );
        }
        Action::Check {
            root,
            environment,
            resolve,
        } => {
            let selected = RuntimeSecrets::inspect(root, environment)?;
            if *resolve {
                ensure!(selected.cloud.is_none(),"Cloud material validation requires the exact executable private plan; use serve --environment");
                let source = RuntimeSecrets::from_bindings(root, &selected, None)?;
                println!(
                    "{}",
                    serde_json::json!({"valid":true,"resolved":true,"bindings":source.binding_names().collect::<Vec<_>>()})
                );
            } else {
                println!(
                    "{}",
                    serde_json::json!({"valid":true,"resolved":false,"secretRequirements":selected.secrets.len()})
                );
            }
        }
        Action::Set {
            secret,
            project,
            environment,
            stdin,
        } => {
            let mut material = if *stdin {
                let mut value = Zeroizing::new(Vec::new());
                std::io::stdin().take(65_537).read_to_end(&mut value)?;
                value
            } else {
                Zeroizing::new(
                    dialoguer::Password::new()
                        .with_prompt("New secret value")
                        .interact()?
                        .into_bytes(),
                )
            };
            ensure!(
                !material.is_empty() && material.len() <= 65_536,
                "AXSECRET_INVALID: value must contain 1..65536 bytes"
            );
            let client = crate::auth_store::authenticated_cloud_client()?;
            let result = client
                .write_secret_version(
                    project,
                    environment,
                    secret,
                    &material,
                    &uuid::Uuid::new_v4().to_string(),
                )
                .await;
            material.fill(0);
            println!("{}", serde_json::to_string_pretty(&result?)?);
        }
        Action::List { project } | Action::Environments { project } => {
            let client = crate::auth_store::authenticated_cloud_client()?;
            let part = if matches!(action, Action::List { .. }) {
                "secrets"
            } else {
                "configuration-environments"
            };
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &client
                        .secret_metadata(project, &[part], reqwest::Method::GET, None)
                        .await?
                )?
            );
        }
        Action::Audit {
            project,
            environment,
            before,
        } => {
            let client = crate::auth_store::authenticated_cloud_client()?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &client
                        .secret_audit_page(project, environment, *before)
                        .await?
                )?
            );
        }
        Action::Activate {
            secret,
            version,
            project,
            environment,
            revision,
        }
        | Action::Rewrap {
            secret,
            version,
            project,
            environment,
            revision,
        }
        | Action::Revoke {
            secret,
            version,
            project,
            environment,
            revision,
        } => {
            let operation = if matches!(action, Action::Activate { .. }) {
                "activate"
            } else if matches!(action, Action::Rewrap { .. }) {
                "rewrap"
            } else {
                "disable"
            };
            let client = crate::auth_store::authenticated_cloud_client()?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &client
                        .secret_metadata(
                            project,
                            &[
                                "configuration-environments",
                                environment,
                                "secrets",
                                secret,
                                "versions",
                                version,
                                operation
                            ],
                            reqwest::Method::POST,
                            Some(serde_json::json!({"expected_revision":revision}))
                        )
                        .await?
                )?
            );
        }
    }
    Ok(())
}
