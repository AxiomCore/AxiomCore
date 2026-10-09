//! Local real serving and explicitly selected full server simulation.
use super::server_command_contract::{PreparationInput, ServeInput};
use anyhow::{bail, ensure, Context, Result};
use axiom_lib::{backend::ServerPlan, contract::AxiomFile};
use axiom_server::{host::ActionHost, invocation::InvocationCancellation};
use std::{net::SocketAddr, sync::Arc};

pub async fn handle(input: ServeInput) -> Result<()> {
    if input.mock {
        return super::mock::handle_resolved(input.resolve(&std::env::current_dir()?)?).await;
    }
    let resolved = input.resolve(&std::env::current_dir()?)?;
    let cancellation = InvocationCancellation::default();
    let token = cancellation.clone();
    let signal = tokio::spawn(async move {
        axiom_server::development::shutdown_signal().await;
        token.cancel();
    });
    let result = async {
        let listen = SocketAddr::new(resolved.host, resolved.port);
        if resolved.watch {
            let PreparationInput::Source { path, variant } = &resolved.input else {
                bail!("watch requires source input");
            };
            let deployment = resolved.deployment.as_deref().context("watch requires a source-owned server plan and reviewed --deployment (or adjacent deployment.json); external Python/Go services use their own reload tooling")?;
            return super::server_development::supervise(path, deployment, resolved.bindings.as_deref(), resolved.delivery.as_deref(), listen, 250, variant.as_deref(), cancellation).await;
        }
        // Source compilation and provider preflight can block; use the existing
        // compiler on a worker thread and write only to an owned temporary dir.
        let prepared = tokio::task::spawn_blocking(move || -> Result<_> {
            let mut temporary = None;
            let (contract, plan) = match resolved.input {
                PreparationInput::Source { path, variant } => {
                    let variant = variant.as_deref().unwrap_or("default");
                    let config = axiom_extractor::evaluate_acore_config(path.to_str().context("source path must be UTF-8")?, Some(variant))?;
                    let backend = config.backend.clone();
                    let directory = tempfile::Builder::new().prefix("axiom-serve-").tempdir()?;
                    let artifact = directory.path().join("axiom.axiom");
                    axiom_build::core::build::build_evaluated_config(config, variant, None, &artifact)?;
                    let plan = artifact.with_extension("server.json");
                    if !plan.is_file() {
                        ensure!(resolved.deployment.is_none() && resolved.bindings.is_none() && resolved.delivery.is_none(), "external services cannot consume ActionHost deployment/bindings without a private server plan; configure their application adapter explicitly");
                        let backend = backend.context("no executable server plan or declared Python/Go backend; real serving never starts a mock or retrieves cloud mock data. Declare action implementations or use `axiom run <source.acore> --mode mock` explicitly")?;
                        return Ok(Prepared::External { source: path, backend, listen, debug: resolved.debug });
                    }
                    temporary = Some(directory);
                    (artifact, plan)
                }
                PreparationInput::Artifact { contract, plan } => (contract, plan.context("real serving requires the exact private plan")?),
                PreparationInput::Cloud => bail!("real serving never retrieves cloud mock data"),
            };
            let deployment = resolved.deployment.context("real action serving requires reviewed --deployment or adjacent deployment.json; run `axiom server deployment` to scaffold and explicitly review providers, credentials and grants")?;
            let host = load_host(&contract, &plan, &deployment, resolved.bindings.as_deref(), resolved.delivery.as_deref())?;
            Ok(Prepared::Host { host, listen, debug: resolved.debug, _artifacts: temporary })
        }).await.context("real server preparation task failed")??;
        match prepared {
            Prepared::Host { host, listen, debug, _artifacts } => {
                let _artifacts = _artifacts;
                axiom_server::development::real_session(host, listen, debug).run(cancellation).await?;
            }
            Prepared::External { source, backend, listen, debug } => {
                axiom_server::development::PreparedSession::new(axiom_lib::backend::ServingMode::Real, Box::new(ExternalRunner { source, backend, listen, debug })).run(cancellation).await?;
            }
        }
        Ok(())
    }.await;
    signal.abort();
    result
}

struct ExternalRunner {
    source: std::path::PathBuf,
    backend: axiom_lib::config::BackendConfig,
    listen: SocketAddr,
    debug: bool,
}
impl axiom_server::development::DevelopmentRunner for ExternalRunner {
    fn run(
        self: Box<Self>,
        cancellation: InvocationCancellation,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = axiom_server::Result<()>> + Send>> {
        Box::pin(async move {
            super::run::run_external_service(&self.source, self.backend, self.listen, self.debug, cancellation).await.map_err(|_| axiom_server::BoundaryError { typed_failure: None, validation: None, cleanup: None,
                status: 500, code: "external_service_failed", path: "backend".into(),
                message: "external backend failed; check its entrypoint, installed runtime/dependencies and application startup diagnostic".into(),
            })
        })
    }
}

enum Prepared {
    Host {
        host: Arc<ActionHost>,
        listen: SocketAddr,
        debug: bool,
        _artifacts: Option<tempfile::TempDir>,
    },
    External {
        source: std::path::PathBuf,
        backend: axiom_lib::config::BackendConfig,
        listen: SocketAddr,
        debug: bool,
    },
}

pub(super) fn load_host(
    contract: &std::path::Path,
    plan: &std::path::Path,
    deployment: &std::path::Path,
    bindings: Option<&std::path::Path>,
    delivery: Option<&std::path::Path>,
) -> Result<Arc<ActionHost>> {
    let contract: AxiomFile = axiom_server::serve::read_json(contract, 16_777_216)?;
    let plan: ServerPlan = axiom_server::serve::read_json(plan, 16_777_216)?;
    plan.validate(&contract)?;
    ensure!(plan.actions.values().all(|action| !matches!(action.implementation, axiom_lib::backend::Implementation::Native { .. })), "native actions require a registered trusted Rust application host; generic `axiom serve` cannot resolve arbitrary native symbols. Use that application's runner and `axiom_server::development::real_session`");
    let mut registry = axiom_server::serve::registry_from_file(&contract, &plan, bindings)?;
    attach_delivery(&contract,&plan,&mut registry,delivery)?;
    Ok(Arc::new(ActionHost::new(
        contract,
        plan,
        axiom_server::serve::read_json(deployment, 16_777_216)?,
        registry,
    )?))
}

/// Explicit private configuration; never discovered from an action input.
pub(super) fn attach_delivery(contract:&AxiomFile,plan:&ServerPlan,registry:&mut axiom_server::host::HostRegistry,path:Option<&std::path::Path>)->Result<()> {
    if let Some(path)=path {
        let bytes=axiom_server::serve::read(path,1_048_576)?;
        let config=axiom_server::delivery::DeliveryDeployment::from_json(std::str::from_utf8(&bytes)?)?;
        let controller=axiom_server::delivery::DeliveryController::connect(contract.clone(),plan.clone(),config,Arc::new(|key:&str|std::env::var(key).ok()))?;
        registry.delivery(Arc::new(controller))?;
    }
    Ok(())
}
