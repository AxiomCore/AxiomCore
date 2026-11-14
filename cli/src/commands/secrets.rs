//! Secret commands deliberately bypass generic CLI error payloads and value
//! serialization. Only the set command accepts material, through prompt/stdin.
use anyhow::{ensure, Context, Result};
use axiom_server::{
    secret_cloud::{CloudSession, LeaseRequirements, LeaseTicket, ProofKey},
    secret_runtime::RuntimeSecrets,
};
use clap::Subcommand;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Help for `--project` on Cloud subcommands.
const PROJECT_HELP: &str = "Project slug, such as `payments-api`";
/// Help for a Cloud `--environment`: a name or UUID, resolved before the request.
const ENVIRONMENT_HELP: &str = "Cloud configuration environment: a name such as `staging` or its UUID";

#[derive(Clone, Debug, Subcommand)]
pub enum Action {
    /// Create a reference-only AxiomSecrets.toml scaffold.
    Init {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    /// Validate a local profile; `--resolve` also checks local sources.
    Check {
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Profile name in AxiomSecrets.toml, such as `development`.
        #[arg(long)]
        environment: String,
        #[arg(long)]
        resolve: bool,
    },
    /// Show a local profile's references (never values).
    Inspect {
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Profile name in AxiomSecrets.toml, such as `development`.
        #[arg(long)]
        environment: String,
    },
    /// Create an isolated logical scope; production scopes must be protected.
    CreateEnvironment {
        #[arg(long, help = PROJECT_HELP)]
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
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long)]
        purpose: String,
        #[arg(long, default_value = "")]
        description: String,
    },
    /// Create or update a binding from a private JSON file. Updating an existing binding
    /// (a non-zero `expected_revision`) invalidates older leases' authority, so it asks first.
    Bind {
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        #[arg(long)]
        file: PathBuf,
        /// Confirm updating an existing binding (a non-zero `expected_revision`) without a
        /// prompt (required without a terminal). A new binding does not ask.
        #[arg(long)]
        yes: bool,
    },
    /// List workload registrations.
    Workloads {
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
    },
    /// Register an exact workload identity and plan from a private JSON file.
    RegisterWorkload {
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        #[arg(long)]
        file: PathBuf,
    },
    /// Disable a workload registration and invalidate its leases.
    DisableWorkload {
        id: String,
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        /// Confirm without a prompt (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// List grants.
    Grants {
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
    },
    /// Grant a member or workload access from a private JSON file.
    Grant {
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        #[arg(long)]
        file: PathBuf,
    },
    /// Revoke a grant and its affected leases.
    RevokeGrant {
        id: String,
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        /// Confirm without a prompt (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// Update an environment's status, protection or expiry from a private JSON file.
    /// Every update carries a status that may enable or disable the environment, so it asks first.
    UpdateEnvironment {
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        #[arg(long)]
        file: PathBuf,
        /// Confirm without a prompt (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// List a secret's versions with the current revision and active version.
    Versions {
        /// Secret name, such as `payments/api-key`, or its UUID.
        secret: String,
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        /// Print JSON: secret, environment, revision, active_version_id and versions.
        #[arg(long)]
        json: bool,
    },
    /// Permanently destroy an eligible disabled version after retention.
    Destroy {
        /// Secret name or UUID.
        secret: String,
        /// Version: `v3`, `3` or a version UUID.
        version: String,
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        /// Expected environment-secret revision. Defaults to the current one.
        #[arg(long)]
        revision: Option<u64>,
        /// Confirm without a prompt (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// Show a compiled private plan's secret requirements.
    Requirements {
        plan: PathBuf,
    },
    /// Save a candidate value from a masked prompt or stdin. Does not activate it.
    Set {
        /// Secret name or UUID.
        secret: String,
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        /// Read one raw value (1..65536 bytes, every byte kept) from stdin.
        #[arg(long)]
        stdin: bool,
    },
    /// Save a new value and activate it (set, then activate).
    Rotate {
        /// Secret name or UUID.
        secret: String,
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        /// Read one raw value (1..65536 bytes, every byte kept) from stdin.
        #[arg(long)]
        stdin: bool,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
        /// Confirm without a prompt (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// List project-wide secret definitions.
    List {
        #[arg(long, help = PROJECT_HELP)]
        project: String,
    },
    /// List configuration environments with IDs, names and revisions.
    Environments {
        #[arg(long, help = PROJECT_HELP)]
        project: String,
    },
    /// List up to 100 audit events.
    Audit {
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        #[arg(long)]
        before: Option<u64>,
    },
    /// Make a candidate version the active one.
    Activate {
        /// Secret name or UUID.
        secret: String,
        /// Version: `v3`, `3` or a version UUID.
        version: String,
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        /// Expected environment-secret revision. Defaults to the current one.
        #[arg(long)]
        revision: Option<u64>,
        /// Confirm without a prompt (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// Rewrap an encrypted version with the configured KMS primary key.
    Rewrap {
        /// Secret name or UUID.
        secret: String,
        /// Version: `v3`, `3` or a version UUID.
        version: String,
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        /// Expected environment-secret revision. Defaults to the current one.
        #[arg(long)]
        revision: Option<u64>,
        /// Confirm without a prompt (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// Disable a version. Different from revoking a grant.
    Revoke {
        /// Secret name or UUID.
        secret: String,
        /// Version: `v3`, `3` or a version UUID.
        version: String,
        #[arg(long, help = PROJECT_HELP)]
        project: String,
        #[arg(long, help = ENVIRONMENT_HELP)]
        environment: String,
        /// Expected environment-secret revision. Defaults to the current one.
        #[arg(long)]
        revision: Option<u64>,
        /// Confirm without a prompt (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
}
pub async fn runtime(
    root: &Path,
    environment: &str,
    plan: &axiom_lib::backend::ServerPlan,
) -> Result<RuntimeSecrets> {
    ensure!(cfg!(all(target_os="macos",target_arch="aarch64")),"AXSECRET_UNSUPPORTED_TARGET: select a separately qualified protected runtime before authenticating a secret source");
    let selected = RuntimeSecrets::inspect(root, environment)?;
    use axiom_lib::secrets::SecretSource;
    // Cloud secrets open a secret session; managed databases lease through the gateway.
    let has_cloud_secrets = selected.secrets.values().any(|s| matches!(s, SecretSource::AxiomCloud { .. }));
    let database = axiom_server::database_lease::Gateway::for_profile(&selected, &plan.sha256()?)?;
    let cloud = if let Some(config) = selected.cloud.as_ref().filter(|_| has_cloud_secrets) {
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
    Ok(RuntimeSecrets::from_bindings_with(
        root,
        &selected,
        cloud
            .as_ref()
            .map(|c| c as &dyn axiom_server::secret_runtime::CloudSecretSource),
        database.as_ref(),
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
/// Resolve a Cloud `--environment` name to the UUID the routes need.
async fn environment_id(project: &str, environment: &str) -> Result<String> {
    let client = crate::auth_store::authenticated_cloud_client()?;
    super::secrets_cloud::environment_id(&client, project, environment).await
}
/// A binding file with a non-zero `expected_revision` updates an existing binding. The server
/// decodes field names case-insensitively, so any spelling of the key counts.
fn updates_binding(body: &serde_json::Value) -> bool {
    let Some(fields) = body.as_object() else { return false };
    fields.iter().filter(|(key, _)| key.eq_ignore_ascii_case("expected_revision")).any(|(_, value)| match value {
        serde_json::Value::Null => false,
        serde_json::Value::Number(revision) => revision.as_f64() != Some(0.0),
        // Not a number: the server refuses it, but treat it as an update and ask.
        _ => true,
    })
}

fn binding_prompt(environment: &str, body: &serde_json::Value) -> String {
    let field = |key: &str| body[key].as_str().unwrap_or("?").to_string();
    format!(
        "Update the binding of {} for {} in {environment}? Saving invalidates authority held by older leases.",
        field("consumer"),
        field("requirement")
    )
}

/// The dashboard's enable and disable wording; any other status names the change.
fn environment_prompt(environment: &str, body: &serde_json::Value) -> String {
    match body["status"].as_str() {
        Some("disabled") => format!("Disable {environment}? Runtime consumption will be denied and affected leases invalidated. Values and audit history remain."),
        Some("active") => format!("Update {environment} with status active? If it is disabled, this enables it: explicit grants and version pins still apply, and enabling does not restore revoked access."),
        Some(status) => format!("Set the status of {environment} to {status}?"),
        None => format!("Update {environment}?"),
    }
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
            ..
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
            ..
        } => {
            // Every environment update carries a status (the API refuses one without), and
            // whether it changes the stored status is known only to the server, so every update
            // asks, as the dashboard's enable and disable do. Without a terminal and --yes it is
            // refused here, before the file is read or anything is sent.
            if let Action::UpdateEnvironment { yes, .. } = action {
                super::auth::require_yes_or_terminal(*yes)?;
            }
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
            let body: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("AXSECRET_INVALID: invalid reference metadata"))?;
            match action {
                Action::UpdateEnvironment { yes, .. } => {
                    super::auth::confirm_yes(*yes, &environment_prompt(environment, &body))?;
                }
                // A binding with an expected revision updates an existing one; the server then
                // bumps its revision, which invalidates authority held by older leases. Asked
                // first, as the dashboard's "Update this binding?". A new binding does not ask.
                Action::Bind { yes, .. } if updates_binding(&body) => {
                    super::auth::confirm_yes(*yes, &binding_prompt(environment, &body))?;
                }
                _ => {}
            }
            let environment = environment_id(project, environment).await?;
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
            let environment = environment_id(project, environment).await?;
            metadata(
                project,
                &["configuration-environments", &environment, part],
                reqwest::Method::GET,
                None,
            )
            .await?;
        }
        Action::DisableWorkload {
            id,
            project,
            environment,
            yes,
        }
        | Action::RevokeGrant {
            id,
            project,
            environment,
            yes,
        } => {
            // The dashboard asks before both; a terminal prompts, otherwise --yes is required.
            let (part, operation, prompt) = if matches!(action, Action::DisableWorkload { .. }) {
                ("secret-workloads", "disable", format!("Disable workload {id} in {environment}? It gets no new secrets or database credentials and its Cloud leases are invalidated. A database credential it already holds keeps working for the engine's bound (SQLite: at most 15 minutes; PostgreSQL: open connections end within about 10 minutes)."))
            } else {
                ("secret-grants", "revoke", format!("Revoke grant {id} in {environment}? Affected Cloud leases are invalidated."))
            };
            super::auth::confirm_yes(*yes, &prompt)?;
            let environment = environment_id(project, environment).await?;
            metadata(
                project,
                &[
                    "configuration-environments",
                    &environment,
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
            json,
        } => super::secrets_cloud::show_versions(secret, project, environment, *json).await?,
        Action::Destroy {
            secret,
            version,
            project,
            environment,
            revision,
            yes,
        } => {
            super::secrets_cloud::version_action("destroy", secret, version, project, environment, *revision, *yes)
                .await?
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
                ensure!(selected.cloud.is_none(),"Cloud material and managed database validation require the exact executable private plan; use serve --environment");
                let source = RuntimeSecrets::from_bindings(root, &selected, None)?;
                println!(
                    "{}",
                    serde_json::json!({"valid":true,"resolved":true,"bindings":source.binding_names().collect::<Vec<_>>()})
                );
            } else {
                let databases = selected.secrets.values().filter(|s| matches!(s, axiom_lib::secrets::SecretSource::AxiomCloudDatabase { .. })).count();
                println!(
                    "{}",
                    serde_json::json!({"valid":true,"resolved":false,"secretRequirements":selected.secrets.len(),"managedDatabases":databases})
                );
            }
        }
        Action::Set {
            secret,
            project,
            environment,
            stdin,
        } => {
            let material = super::secrets_cloud::read_material(*stdin)?;
            super::secrets_cloud::set(secret, project, environment, material).await?;
        }
        Action::Rotate {
            secret,
            project,
            environment,
            stdin,
            json,
            yes,
        } => {
            super::secrets_cloud::rotate(
                secret,
                project,
                environment,
                || super::secrets_cloud::read_material(*stdin),
                *json,
                *yes,
            )
            .await?
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
            let environment =
                super::secrets_cloud::environment_id(&client, project, environment).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &client
                        .secret_audit_page(project, &environment, *before)
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
            yes,
        }
        | Action::Rewrap {
            secret,
            version,
            project,
            environment,
            revision,
            yes,
        }
        | Action::Revoke {
            secret,
            version,
            project,
            environment,
            revision,
            yes,
        } => {
            let operation = if matches!(action, Action::Activate { .. }) {
                "activate"
            } else if matches!(action, Action::Rewrap { .. }) {
                "rewrap"
            } else {
                "disable"
            };
            super::secrets_cloud::version_action(operation, secret, version, project, environment, *revision, *yes)
                .await?;
        }
    }
    Ok(())
}
