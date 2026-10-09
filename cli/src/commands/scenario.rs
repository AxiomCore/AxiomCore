//! Explicit development-only execution over exact compiled artifacts.
use anyhow::{ensure, Result};
use axiom_lib::{backend::{Deployment, ServerPlan}, contract::AxiomFile, development_scenario as scenario};
use clap::Subcommand;
use std::path::{Path, PathBuf};

#[derive(Subcommand, Debug)]
pub enum ScenarioAction {
    /// Describe typed operations and exact artifact identities without execution
    Catalog {
        contract: PathBuf,
        #[arg(long)] plan: PathBuf,
        #[arg(long)] environment: Option<PathBuf>,
    },
    /// Validate every step without acquiring resources or invoking application code
    Check {
        contract: PathBuf,
        #[arg(long)] plan: PathBuf,
        #[arg(long)] scenario: PathBuf,
        #[arg(long)] environment: Option<PathBuf>,
    },
    /// Run the native managed query fixture with logical time and strict scripts
    Run {
        contract: PathBuf,
        #[arg(long)] plan: PathBuf,
        #[arg(long)] deployment: PathBuf,
        #[arg(long)] scenario: PathBuf,
        /// Reviewed development-only synthetic expiry environment; no live credentials
        #[arg(long, conflicts_with_all=["epoch_ms","seed"])] environment: Option<PathBuf>,
        /// Explicit synthetic wall-clock epoch, in milliseconds
        #[arg(long, default_value_t = 0)] epoch_ms: u64,
        /// Test-only policy jitter seed; never replaces secure randomness
        #[arg(long, default_value_t = 7)] seed: u64,
        /// Save the bounded, payload-free report, including on assertion failure
        #[arg(long)] report: Option<PathBuf>,
        /// Save payload-free actual host observations, labeled scripted-provider
        #[arg(long)] observations: Option<PathBuf>,
    },
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(axiom_server::serve::read_json(path, 16_777_216)?)
}
fn inputs(contract: &Path, plan: &Path) -> Result<(AxiomFile, ServerPlan)> {
    let contract = read(contract)?;
    let plan: ServerPlan = read(plan)?;
    plan.validate(&contract)?;
    Ok((contract, plan))
}
fn source(path: &Path) -> Result<scenario::Scenario> {
    scenario::decode(&axiom_server::serve::read(path, scenario::MAX_BYTES as u64)?)
}
fn catalog(contract:&AxiomFile,plan:&ServerPlan,environment:&Option<PathBuf>)->Result<scenario::Catalog> {
    if let Some(path)=environment {scenario::backend_environment_catalog(contract,plan,&read(path)?)}else{scenario::backend_catalog(contract,plan)}
}
pub async fn handle(action: &ScenarioAction) -> Result<()> {
    match action {
        ScenarioAction::Catalog { contract, plan, environment } => {
            let (contract, plan) = inputs(contract, plan)?;
            println!("{}", serde_json::to_string_pretty(&catalog(&contract, &plan, environment)?.inspection()?)?);
        },
        ScenarioAction::Check { contract, plan, scenario: path, environment } => {
            let (contract, plan) = inputs(contract, plan)?;
            let catalog = catalog(&contract, &plan, environment)?;
            let qualified = scenario::qualify_backend(source(path)?, &catalog)?;
            println!("{}", serde_json::json!({"format":"axiom-development-scenario-check/v1","validated":true,"executed":false,"scenarioSha256":qualified.sha256(),"artifacts":catalog.artifacts,"level":catalog.level}));
        },
        ScenarioAction::Run { contract, plan, deployment, scenario: path, environment, epoch_ms, seed, report, observations } => {
            // Capture all bytes before any fixture is activated. No listener,
            // external provider, credential resolver or live capture is used.
            let (contract, plan) = inputs(contract, plan)?;
            let deployment: Deployment = read(deployment)?;
            let source = source(path)?;
            let epoch_ms = *epoch_ms; let seed = *seed;
            let (qualified, mut adapter) = if let Some(path)=environment {
                axiom_server::scenarios::prepare_with_environment(contract,plan,deployment,source,Default::default(),read(path)?)?
            } else {axiom_server::scenarios::prepare(contract, plan, deployment, source, Default::default(), epoch_ms, seed)?};
            let cancellation = axiom_server::invocation::InvocationCancellation::default();
            adapter.runner_cancellation(cancellation.clone());
            let mut worker = tokio::task::spawn_blocking(move || {
                let report=scenario::run(&qualified, &mut adapter);
                (report,adapter.observations())
            });
            let result = tokio::select! {
                result = &mut worker => result?,
                signal = tokio::signal::ctrl_c() => {
                    signal?; cancellation.cancel(); worker.await?
                },
            };
            let (result,snapshot)=result;
            if let Some(path)=observations {super::contract_build::atomic_write(path,&serde_json::to_vec_pretty(&snapshot)?)?;}
            let bytes = serde_json::to_vec_pretty(&result)?;
            if let Some(path) = report { super::contract_build::atomic_write(path, &bytes)?; }
            println!("{}", std::str::from_utf8(&bytes)?);
            ensure!(result.passed, "scenario failed; inspect failure, failedStep and cleanupFailure in the report");
        },
    }
    Ok(())
}
