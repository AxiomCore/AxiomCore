//! Server-plan inspection and real managed/sandbox execution. Never starts mocks.
use anyhow::{Context, Result};
use axiom_lib::{backend::*, contract::AxiomFile};
use clap::{Subcommand, ValueEnum};
use std::{fs, io::Write, path::PathBuf};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Language {
    Python,
    Go,
}

#[derive(Clone, Debug, Default, clap::Args)]
pub struct SecretProfileArgs {
    #[arg(long)]
    pub environment: Option<String>,
    /// Project directory containing the reference-only AxiomSecrets.toml.
    #[arg(long, requires = "environment")]
    pub secret_root: Option<PathBuf>,
}
impl SecretProfileArgs {
    /// The selected environment as a sealable container profile (no values are read).
    pub(super) fn sealed(
        &self,
        contract: &std::path::Path,
    ) -> Result<Option<axiom_server::release::SealedProfile>> {
        let Some(environment) = &self.environment else {
            return Ok(None);
        };
        let root = self.secret_root.as_deref().unwrap_or_else(|| {
            contract
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
        });
        let bindings = axiom_server::secret_runtime::RuntimeSecrets::inspect(root, environment)?;
        Ok(Some(axiom_server::release::SealedProfile::new(environment, bindings)?))
    }
    pub(super) fn resolve(
        &self,
        contract: &std::path::Path,
        plan: &axiom_lib::backend::ServerPlan,
    ) -> Result<Option<std::sync::Arc<axiom_server::secret_runtime::RuntimeSecrets>>> {
        let Some(environment) = &self.environment else {
            return Ok(None);
        };
        let root = self.secret_root.as_deref().unwrap_or_else(|| {
            contract
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
        });
        // Some server commands are synchronous adapters. Isolate native login
        // HTTP from any caller's Tokio executor instead of nesting block_on.
        let source = std::thread::scope(|scope| {
            scope
                .spawn(|| -> Result<_> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(super::secrets::runtime(root, environment, plan))
                })
                .join()
                .map_err(|_| anyhow::anyhow!("AXSECRET_UNAVAILABLE: runtime preparation failed"))?
        })?;
        Ok(Some(std::sync::Arc::new(source)))
    }
}

#[derive(Debug, Subcommand)]
pub enum ServerAction {
    /// Validate providers, credentials, handlers and cleanup before opening a listener
    Validate {
        #[command(flatten)]
        secrets: SecretProfileArgs,
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        bindings: Option<PathBuf>,
        #[arg(long)]
        delivery: Option<PathBuf>,
        #[arg(long, default_value = "native-server")]
        target: String,
    },
    /// List supported profiles; uncertified targets fail closed
    Targets,
    /// Create a reproducible, hash-bound conventional-server release directory
    Package {
        /// Container targets: seal this AxiomSecrets.toml environment (value-free; Cloud
        /// sources only) into the release, so run-bundle resolves it through the workload.
        #[command(flatten)]
        secrets: SecretProfileArgs,
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        bindings: Option<PathBuf>,
        #[arg(long, default_value = "native-server")]
        target: String,
        /// Reviewed workspace source root used to build a container or Worker runtime
        #[arg(long)]
        runtime_source: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Verify all release files, platform, contracts and capability requirements
    VerifyBundle { directory: PathBuf },
    /// Run a verified release using its exact packaged executable and configuration
    RunBundle {
        #[command(flatten)]
        secrets: SecretProfileArgs,
        directory: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: std::net::SocketAddr,
    },
    /// Compare immutable releases for public compatibility and review private behavior
    RollbackCheck { current: PathBuf, previous: PathBuf },
    /// Build and reload local source within the reviewed provider/grant boundary
    Dev {
        source: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        bindings: Option<PathBuf>,
        #[arg(long)]
        delivery: Option<PathBuf>,
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: std::net::SocketAddr,
        #[arg(long, default_value = "250")]
        poll_ms: u64,
    },
    /// Execute durable jobs, consumers, schedules and signed webhook attempts
    Worker {
        #[command(flatten)]
        secrets: SecretProfileArgs,
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        delivery: PathBuf,
        #[arg(long)]
        bindings: Option<PathBuf>,
        #[arg(long)]
        once: bool,
        #[arg(long, default_value = "100")]
        poll_ms: u64,
    },
    /// Submit a typed job or publish to explicitly granted event bindings
    Enqueue {
        #[command(flatten)]
        secrets: SecretProfileArgs,
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        delivery: PathBuf,
        #[arg(long)]
        request: PathBuf,
    },
    /// Inspect redacted provider state, attempts, failures and recovery history
    DeliveryInspect {
        #[command(flatten)]
        secrets: SecretProfileArgs,
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        delivery: PathBuf,
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        dead: bool,
        #[arg(long, default_value = "32")]
        limit: usize,
    },
    /// Cancel pending work or signal a leased attempt
    DeliveryCancel {
        #[command(flatten)]
        secrets: SecretProfileArgs,
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        delivery: PathBuf,
        #[arg(long)]
        id: String,
    },
    /// Explicitly redrive a dead letter; external effects can repeat
    DeliveryRedrive {
        #[command(flatten)]
        secrets: SecretProfileArgs,
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        delivery: PathBuf,
        #[arg(long)]
        id: String,
    },
    /// Scaffold private delivery references with empty operation grants
    DeliveryDeployment {
        contract: PathBuf,
        plan: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Execute managed/sandbox actions as an HTTP service; never starts mocks
    Run {
        #[command(flatten)]
        secrets: SecretProfileArgs,
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        bindings: Option<PathBuf>,
        /// Reviewed durable controller; producer input cannot select its grants.
        #[arg(long)]
        delivery: Option<PathBuf>,
        /// Export bounded payload-free native observations after owned shutdown.
        /// The destination must be new; the report is written privately.
        #[arg(long)]
        observations: Option<PathBuf>,
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: std::net::SocketAddr,
    },
    /// Invoke an authored action directly through its mandatory guards
    Invoke {
        #[command(flatten)]
        secrets: SecretProfileArgs,
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        bindings: Option<PathBuf>,
        /// Reviewed durable controller; producer input cannot select its grants.
        #[arg(long)]
        delivery: Option<PathBuf>,
        #[arg(long)]
        request: PathBuf,
    },
    /// Run the private JSON-lines enforcement bridge owned by a native adapter
    Boundary {
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        deployment: Option<PathBuf>,
    },
    /// Scaffold native action bindings from a compiled contract; review status/body mappings
    Plan {
        contract: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Scaffold private provider references; review providers and explicit grants
    Deployment {
        contract: PathBuf,
        plan: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Validate exact contract identity and inspect action guards/effect evidence
    Inspect {
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long)]
        deployment: Option<PathBuf>,
        #[arg(long)]
        bindings: Option<PathBuf>,
        #[arg(long, default_value = "native-server")]
        target: String,
    },
    /// Compare private behavior without changing the public JSON-Atom diff
    Diff {
        before_contract: PathBuf,
        before_plan: PathBuf,
        after_contract: PathBuf,
        after_plan: PathBuf,
    },
    /// Generate OpenAPI 3.1 from the effective contract and server mappings
    Openapi {
        contract: PathBuf,
        plan: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Generate Python or Go handler input/output types
    Types {
        contract: PathBuf,
        plan: PathBuf,
        #[arg(long, value_enum)]
        language: Language,
        #[arg(short, long)]
        output: PathBuf,
    },
}

/// Await long-lived execution on the CLI runtime without a blocking bridge.
pub async fn handle_async(action: &ServerAction) -> Result<()> {
    match action {
        ServerAction::Run {
            contract,
            plan,
            deployment,
            bindings,
            delivery,
            observations,
            listen,
            secrets,
        } => {
            let secrets = secrets.clone();
            let (contract, plan, deployment, bindings, delivery) = (
                contract.clone(),
                plan.clone(),
                deployment.clone(),
                bindings.clone(),
                delivery.clone(),
            );
            let cancellation = axiom_server::invocation::InvocationCancellation::default();
            let observation_output = observations
                .as_deref()
                .map(prepare_observation_output)
                .transpose()?;
            let token = cancellation.clone();
            let signal = tokio::spawn(async move {
                axiom_server::development::shutdown_signal().await;
                token.cancel();
            });
            let result = async {
                let host = tokio::task::spawn_blocking(move || {
                    let private_plan: ServerPlan = read(&plan)?;
                    let source = secrets.resolve(&contract, &private_plan)?;
                    super::serve::load_host_with_resolver(
                        &contract,
                        &plan,
                        &deployment,
                        bindings.as_deref(),
                        delivery.as_deref(),
                        source,
                    )
                })
                .await??;
                let observed = host.clone();
                let result = axiom_server::development::real_session(host, *listen, false)
                    .run(cancellation)
                    .await;
                if let Some((mut file, destination)) = observation_output {
                    let bytes = serde_json::to_vec_pretty(&observed.observations())?;
                    anyhow::ensure!(
                        bytes.len() <= 25 * 1024 * 1024,
                        "observation report exceeds its bound"
                    );
                    file.write_all(&bytes)?;
                    file.write_all(b"\n")?;
                    file.as_file().sync_all()?;
                    file.persist_noclobber(&destination)
                        .with_context(|| format!("save observations {}", destination.display()))?;
                }
                result.map_err(Into::into)
            }
            .await;
            signal.abort();
            result
        }
        ServerAction::RunBundle {
            directory,
            listen,
            secrets,
        } => {
            let directory = directory.clone();
            let secrets = secrets.clone();
            let cancellation = axiom_server::invocation::InvocationCancellation::default();
            let token = cancellation.clone();
            let signal = tokio::spawn(async move {
                axiom_server::development::shutdown_signal().await;
                token.cancel();
            });
            let result = async {
                let host = tokio::task::spawn_blocking(move || {
                    super::server_workflow::prepare_bundle(&directory, &secrets)
                })
                .await??;
                axiom_server::development::real_session(host, *listen, false)
                    .run(cancellation)
                    .await?;
                Ok(())
            }
            .await;
            signal.abort();
            result
        }
        ServerAction::Dev {
            source,
            deployment,
            bindings,
            delivery,
            listen,
            poll_ms,
        } => {
            super::server_development::run(
                source,
                deployment,
                bindings.as_deref(),
                delivery.as_deref(),
                *listen,
                *poll_ms,
            )
            .await
        }
        _ => handle(action),
    }
}

fn prepare_observation_output(
    path: &std::path::Path,
) -> Result<(tempfile::NamedTempFile, PathBuf)> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        Ok(_) => anyhow::bail!("observation destination already exists; choose a new report path"),
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let file = tempfile::NamedTempFile::new_in(parent)
        .context("prepare private observation output before host activation")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok((file, path.into()))
}

fn read<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T> {
    axiom_server::serve::read_json(path, 16_777_216)
        .with_context(|| format!("decode {}", path.display()))
}

fn write(path: &PathBuf, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "create {}; choose a new output path to preserve an existing reviewed file",
                path.display()
            )
        })?;
    file.write_all(bytes)?;
    file.sync_all()?;
    println!("Created {}", path.display());
    Ok(())
}
fn read_delivery(path: &PathBuf) -> Result<axiom_server::delivery::DeliveryDeployment> {
    anyhow::ensure!(
        fs::metadata(path)?.len() <= 1_048_576,
        "delivery manifest exceeds 1 MiB"
    );
    Ok(axiom_server::delivery::DeliveryDeployment::from_json(
        &fs::read_to_string(path)?,
    )?)
}

pub fn handle(action: &ServerAction) -> Result<()> {
    if super::server_workflow::handle(action)? {
        return Ok(());
    }
    match action {
        ServerAction::Worker {
            contract,
            plan,
            delivery,
            secrets,
            ..
        }
        | ServerAction::Enqueue {
            contract,
            plan,
            delivery,
            secrets,
            ..
        }
        | ServerAction::DeliveryInspect {
            contract,
            plan,
            delivery,
            secrets,
            ..
        }
        | ServerAction::DeliveryCancel {
            contract,
            plan,
            delivery,
            secrets,
            ..
        }
        | ServerAction::DeliveryRedrive {
            contract,
            plan,
            delivery,
            secrets,
            ..
        } => {
            let plan: ServerPlan = read(plan)?;
            let source = secrets.resolve(contract, &plan)?;
            let resolver: std::sync::Arc<
                dyn axiom_server::providers::ConfigurationResolver + Send + Sync,
            > = match source {
                Some(source) => source,
                None => std::sync::Arc::new(|key: &str| std::env::var(key).ok()),
            };
            let contract: AxiomFile = read(contract)?;
            let controller = axiom_server::delivery::DeliveryController::connect(
                contract.clone(),
                plan.clone(),
                read_delivery(delivery)?,
                resolver.clone(),
            )?;
            match action {
                ServerAction::Worker {
                    deployment,
                    bindings,
                    once,
                    poll_ms,
                    ..
                } => {
                    anyhow::ensure!((10..=60_000).contains(poll_ms), "poll-ms must be 10..60000");
                    let registry = axiom_server::serve::registry_from_file(
                        &contract,
                        &plan,
                        bindings.as_deref(),
                    )?;
                    let worker = std::sync::Arc::new(axiom_server::delivery::DeliveryWorker::new(
                        controller,
                        read(deployment)?,
                        registry,
                    )?);
                    if *once {
                        return tokio::task::block_in_place(|| {
                            worker.tick_schedules()?;
                            println!("{}", serde_json::to_string(&worker.run_one()?)?);
                            worker.shutdown(std::time::Duration::from_secs(1))?;
                            Ok(())
                        });
                    }
                    return tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                        let task_worker=worker.clone();let interval=*poll_ms;
                        let mut task=tokio::task::spawn_blocking(move ||->Result<()> {loop {task_worker.tick_schedules()?;match task_worker.run_one() {Ok(Some(record))=>println!("{}",serde_json::to_string(&record)?),Ok(None)=>std::thread::sleep(std::time::Duration::from_millis(interval)),Err(e) if e.code=="draining"=>break,Err(e) if matches!(e.code,"delivery_unavailable"|"delivery_lease_lost")=>{eprintln!("delivery provider: {}",e.code);std::thread::sleep(std::time::Duration::from_millis(interval));},Err(e)=>return Err(e.into())}}Ok(())});
                        #[cfg(unix)] let signal=async {let mut term=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}Ok::<(),std::io::Error>(())};
                        #[cfg(not(unix))] let signal=tokio::signal::ctrl_c();
                        tokio::select! {result=&mut task=>result??,_=signal=>{worker.begin_drain();task.await??;}}
                        worker.shutdown(std::time::Duration::from_secs(1))?;Ok(())
                    })
                    });
                }
                ServerAction::Enqueue { request, .. } => {
                    anyhow::ensure!(
                        std::fs::metadata(request)?.len() <= 70_000,
                        "delivery submission exceeds bounds"
                    );
                    let request = axiom_server::delivery::Submission::from_json(
                        &std::fs::read_to_string(request)?,
                    )?;
                    println!("{}", serde_json::to_string(&controller.submit(&request)?)?);
                }
                ServerAction::DeliveryInspect {
                    id, dead, limit, ..
                } => {
                    let value = if let Some(id) = id {
                        serde_json::to_value(controller.queue().inspect(id)?)?
                    } else {
                        serde_json::to_value(controller.queue().list(*dead, *limit)?)?
                    };
                    println!("{}", serde_json::to_string_pretty(&value)?);
                }
                ServerAction::DeliveryCancel { id, .. } => controller.cancel(id)?,
                ServerAction::DeliveryRedrive { id, .. } => controller.redrive(id)?,
                _ => unreachable!(),
            }
            return Ok(());
        }
        ServerAction::DeliveryDeployment {
            contract,
            plan,
            output,
        } => {
            let contract: AxiomFile = read(contract)?;
            let plan: ServerPlan = read(plan)?;
            plan.validate(&contract)?;
            let credentials = plan
                .delivery
                .jobs
                .keys()
                .map(|n| format!("job:{n}"))
                .chain(
                    plan.delivery
                        .consumers
                        .keys()
                        .map(|n| format!("consumer:{n}")),
                )
                .map(|n| {
                    let env = format!(
                        "AXIOM_DELIVERY_{}_TOKEN",
                        n.replace(':', "_").to_uppercase()
                    );
                    (
                        n,
                        axiom_server::delivery::DeliveryIdentity {
                            identity: axiom_server::invocation::InvocationIdentity::Service,
                            headers: std::collections::BTreeMap::from([(
                                "authorization".into(),
                                env,
                            )]),
                        },
                    )
                })
                .collect();
            let destinations = plan
                .delivery
                .webhooks
                .keys()
                .map(|n| {
                    (
                        n.clone(),
                        axiom_server::delivery::WebhookDestination {
                            url_env: format!("AXIOM_WEBHOOK_{}_URL", n.to_uppercase()),
                            signing_key_env: format!("AXIOM_WEBHOOK_{}_KEY", n.to_uppercase()),
                        },
                    )
                })
                .collect();
            let config = axiom_server::delivery::DeliveryDeployment {
                format: "axiom-delivery-deployment/v1".into(),
                plan_sha256: plan.sha256()?,
                redis_url_env: "AXIOM_DELIVERY_REDIS_URL".into(),
                namespace: "delivery".into(),
                lease_ms: 30_000,
                retention_ms: 86_400_000,
                grants: Default::default(),
                credentials,
                destinations,
                allow_loopback_http: false,
            };
            return write(output, &serde_json::to_vec_pretty(&config)?);
        }
        _ => {}
    }
    if let ServerAction::Run {
        contract,
        plan,
        deployment,
        bindings,
        delivery,
        secrets,
        ..
    }
    | ServerAction::Invoke {
        contract,
        plan,
        deployment,
        bindings,
        delivery,
        secrets,
        ..
    } = action
    {
        let private_plan: ServerPlan = read(plan)?;
        let source = secrets.resolve(contract, &private_plan)?;
        let host = super::serve::load_host_with_resolver(
            contract,
            plan,
            deployment,
            bindings.as_deref(),
            delivery.as_deref(),
            source,
        )?;
        if matches!(action, ServerAction::Run { .. }) {
            anyhow::bail!("HTTP execution requires async command routing");
        }
        let ServerAction::Invoke { request, .. } = action else {
            unreachable!()
        };
        anyhow::ensure!(
            std::fs::metadata(request)?.len() <= 1_114_112,
            "invocation request exceeds bounds"
        );
        let request = axiom_server::invocation::InvocationRequest::from_json(
            &std::fs::read_to_string(request)?,
        )?;
        let result = host.invoke(&request)?;
        host.shutdown(std::time::Duration::from_secs(1))?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    if let ServerAction::Diff {
        before_contract,
        before_plan,
        after_contract,
        after_plan,
    } = action
    {
        let diff = diff_plans(
            &read(before_contract)?,
            &read(before_plan)?,
            &read(after_contract)?,
            &read(after_plan)?,
        )?;
        println!("{}", serde_json::to_string_pretty(&diff)?);
        return Ok(());
    }
    if let ServerAction::Boundary {
        contract,
        plan,
        deployment,
    } = action
    {
        return axiom_server::protocol::run_with_deployment(contract, plan, deployment.as_deref())
            .map_err(anyhow::Error::from_boxed);
    }
    if let ServerAction::Plan { contract, output } = action {
        let contract: AxiomFile = read(contract)?;
        return write(output, &ServerPlan::native(&contract)?.canonical_bytes()?);
    }
    let (contract, plan) = match action {
        ServerAction::Inspect { contract, plan, .. }
        | ServerAction::Deployment { contract, plan, .. }
        | ServerAction::Openapi { contract, plan, .. }
        | ServerAction::Types { contract, plan, .. } => (contract, plan),
        _ => {
            unreachable!()
        }
    };
    let contract: AxiomFile = read(contract)?;
    let plan: ServerPlan = read(plan)?;
    plan.validate(&contract)?;
    match action {
        ServerAction::Deployment { output, .. } => {
            let bindings = plan
                .resources
                .iter()
                .map(|(name, resource)| {
                    Ok((
                        name.clone(),
                        ProviderBinding {
                            configuration_sha256: resource
                                .configuration
                                .as_ref()
                                .map(|n| plan.configurations[n].digest())
                                .transpose()?,
                            secrets: resource
                                .secrets
                                .iter()
                                .map(|n| (n.clone(), format!("secret:{n}")))
                                .collect(),
                            provider: if resource.transaction_profile.as_deref() == Some(axiom_lib::database::runtime::HOSTED_LIBSQL_PROFILE) {
                                // Hosted libSQL: endpoint, credential and head come from the
                                // managed credential's lease (runtime-data/v1).
                                "libsql".into()
                            } else if resource.transaction_profile.is_some() {
                                "postgres".into()
                            } else if let Some(provider) = plan.integrations.provider(name) {
                                provider.into()
                            } else if resource.interface_schema_sha256.is_some() {
                                "reviewRequired".into()
                            } else {
                                "jsonSnapshot".into()
                            },
                            interface_sha256: resource.interface_hash()?,
                            configuration: if let Some(schema) = resource
                                .configuration
                                .as_ref()
                                .map(|n| &plan.configurations[n])
                            {
                                schema
                                    .fields
                                    .iter()
                                    .filter(|(_, f)| !f.optional && f.default.is_none())
                                    .map(|(key, _)| {
                                        (
                                            key.clone(),
                                            format!(
                                                "env:AXIOM_{}_{}",
                                                name.to_ascii_uppercase(),
                                                key.to_ascii_uppercase()
                                            ),
                                        )
                                    })
                                    .collect()
                            } else if plan.integrations.provider(name) == Some("memoryCache")
                                || resource.transaction_profile.as_deref() == Some(axiom_lib::database::runtime::HOSTED_LIBSQL_PROFILE)
                            {
                                Default::default()
                            } else if plan.integrations.provider(name) == Some("httpClient") {
                                [
                                    "origin",
                                    "credential",
                                    "allowLoopback",
                                    "idempotencySupported",
                                ]
                                .into_iter()
                                .filter(|key| *key != "credential" || resource.secrets.is_empty())
                                .map(|key| {
                                    (
                                        key.into(),
                                        format!(
                                            "AXIOM_HTTP_{}_{}",
                                            name.to_ascii_uppercase(),
                                            key.to_ascii_uppercase()
                                        ),
                                    )
                                })
                                .collect()
                            } else {
                                std::collections::BTreeMap::from([(
                                    (if resource.transaction_profile.is_some() {
                                        "binding"
                                    } else if plan.integrations.provider(name)
                                        == Some("localUpload")
                                    {
                                        "root"
                                    } else {
                                        "path"
                                    })
                                    .into(),
                                    format!(
                                        "AXIOM_RESOURCE_{}_FILE",
                                        name.chars()
                                            .map(|c| if c.is_ascii_alphanumeric() {
                                                c.to_ascii_uppercase()
                                            } else {
                                                '_'
                                            })
                                            .collect::<String>()
                                    ),
                                )])
                            },
                        },
                    ))
                })
                .collect::<std::result::Result<std::collections::BTreeMap<_, _>, PlanError>>()?;
            let shared = plan
                .required_capabilities
                .contains(&Capability::DistributedRateLimit)
                || plan
                    .required_capabilities
                    .contains(&Capability::DurableIdempotency);
            let coordination = if shared {
                Some(CoordinationProvider::Redis {
                    url_env: "AXIOM_REDIS_URL".into(),
                    namespace: "backend".into(),
                })
            } else if plan
                .actions
                .values()
                .any(|a| a.admission != AdmissionPolicy::default())
            {
                Some(CoordinationProvider::Memory)
            } else {
                None
            };
            let security = SecurityRequirements::compile(&contract)?;
            let deployment = Deployment {
                format: DEPLOYMENT_FORMAT.into(),
                plan_sha256: plan.sha256()?,
                capabilities: plan.required_capabilities.clone(),
                bindings,
                grants: Default::default(),
                coordination,
                authentication: if let Some(profile) = security.authentication.values().next() {
                    Some(AuthenticationProvider::Jwt {
                        issuer: profile.issuer.clone(),
                        audience: profile.audience.clone(),
                        algorithm: JwtAlgorithm::RS256,
                        key_env: "AXIOM_JWT_PUBLIC_KEY".into(),
                    })
                } else if !plan.guards.is_empty()
                    || contract.ir.endpoints.values().any(|e| e.auth.is_some())
                {
                    Some(AuthenticationProvider::Jwt {
                        issuer: "https://issuer.example".into(),
                        audience: "server".into(),
                        algorithm: JwtAlgorithm::RS256,
                        key_env: "AXIOM_JWT_PUBLIC_KEY".into(),
                    })
                } else {
                    None
                },
            };
            security.validate_provider(deployment.authentication.as_ref())?;
            write(output, &serde_json::to_vec_pretty(&deployment)?)
        }
        ServerAction::Inspect {
            deployment,
            bindings,
            target,
            ..
        } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&super::server_workflow::inspection(
                    &contract,
                    &plan,
                    deployment.as_deref(),
                    bindings.as_deref(),
                    target
                )?)?
            );
            Ok(())
        }
        ServerAction::Openapi { output, .. } => write(
            output,
            &serde_json::to_vec_pretty(&axiom_server::openapi::generate(&contract, &plan)?)?,
        ),
        ServerAction::Types {
            language, output, ..
        } => write(
            output,
            axiom_server::interfaces::generate(
                &contract,
                &plan,
                match language {
                    Language::Python => axiom_server::interfaces::Language::Python,
                    Language::Go => axiom_server::interfaces::Language::Go,
                },
            )?
            .as_bytes(),
        ),
        _ => unreachable!(),
    }
}
