//! CLI routing for shared release verification and packaging.
use super::server::ServerAction;
use anyhow::Result;
use axiom_lib::{backend::*, contract::AxiomFile};
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(axiom_server::serve::read_json(path, 16_777_216)?)
}
fn artifacts(
    contract: &Path,
    plan: &Path,
    deployment: &Path,
    bindings: Option<&Path>,
) -> Result<(AxiomFile, ServerPlan, Deployment, Option<ActionBindings>)> {
    Ok((
        read(contract)?,
        read(plan)?,
        read(deployment)?,
        bindings.map(read).transpose()?,
    ))
}
pub(super) fn selected_target(id: &str) -> Result<TargetProfile> {
    axiom_server::release::selected_target(id, env!("CARGO_PKG_VERSION"))
}
pub(super) fn prepare_bundle(directory: &Path) -> Result<Arc<axiom_server::host::ActionHost>> {
    axiom_server::release::prepare_bundle(directory, env!("CARGO_PKG_VERSION"))
}
pub(super) fn inspection(
    contract: &AxiomFile,
    plan: &ServerPlan,
    deployment: Option<&Path>,
    bindings: Option<&Path>,
    target: &str,
) -> Result<serde_json::Value> {
    let d: Option<Deployment> = deployment.map(read).transpose()?;
    let b: Option<ActionBindings> = bindings.map(read).transpose()?;
    let mut view = plan.inspection(contract)?;
    view["targetAssessment"] = serde_json::to_value(assess_target(
        contract,
        plan,
        d.as_ref(),
        b.as_ref(),
        &selected_target(target)?,
        None,
    )?)?;
    Ok(view)
}
pub fn handle(action: &ServerAction) -> Result<bool> {
    match action {
        ServerAction::Validate {
            contract,
            plan,
            deployment,
            bindings,
            delivery,
            target,
        } => {
            let (c, p, d, b) = artifacts(contract, plan, deployment, bindings.as_deref())?;
            let mut selected=selected_target(target)?;
            if delivery.is_some() {
                anyhow::ensure!(target=="native-server","an explicit delivery controller requires the native local host; other targets need a qualified delivery packaging adapter");
                selected.capabilities.insert(Capability::Jobs);
            }
            let assessment = assess_target(
                &c,
                &p,
                Some(&d),
                b.as_ref(),
                &selected,
                None,
            )?;
            assessment.require_compatible()?;
            if target == "cloudflare-workers-local" {
                let plan_sha256 = p.sha256()?;
                axiom_server::worker_package::preflight(c,p,d)?;
                println!("{}",json!({"validated":true,"targetAssessment":assessment,"listenerOpened":false,"cleanup":"request-local; no remote dispatch","planSha256":plan_sha256}));
                return Ok(true);
            }
            let mut registry = axiom_server::serve::registry_from_file(&c, &p, bindings.as_deref())?;
            super::serve::attach_delivery(&c,&p,&mut registry,delivery.as_deref())?;
            let plan_sha256 = p.sha256()?;
            let ready =
                axiom_server::host::ActionHost::configured(c, p, d, registry, &|key: &str| {
                    std::env::var(key).ok()
                })?;
            ready.shutdown(Duration::from_secs(5))?;
            println!(
                "{}",
                json!({"validated":true,"targetAssessment":assessment,"listenerOpened":false,"cleanup":"passed","planSha256":plan_sha256})
            );
        }
        ServerAction::Targets => println!(
            "{}",
            serde_json::to_string(&target_catalog(
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            ))?
        ),
        ServerAction::Package {
            contract,
            plan,
            deployment,
            bindings,
            target,
            runtime_source,
            output,
        } => {
            if matches!(target.as_str(), "container-amd64" | "container-arm64") {
                axiom_server::container::package_context(contract, plan, deployment, bindings.as_deref(), target, runtime_source.as_deref().ok_or_else(|| anyhow::anyhow!("container packaging requires --runtime-source pointing to the reviewed workspace root"))?, output)?;
            } else if target.starts_with("cloudflare-workers") {
                axiom_server::worker_package::package(contract,plan,deployment,bindings.as_deref(),target,runtime_source.as_deref().ok_or_else(|| anyhow::anyhow!("Worker packaging requires --runtime-source pointing to the reviewed workspace root"))?,output)?;
            } else {
                anyhow::ensure!(
                    runtime_source.is_none(),
                    "--runtime-source applies only to container and Worker targets"
                );
                axiom_server::release::package(
                    contract,
                    plan,
                    deployment,
                    bindings.as_deref(),
                    target,
                    output,
                    env!("CARGO_PKG_VERSION"),
                )?;
            }
        }
        ServerAction::VerifyBundle { directory } => println!(
            "{}",
            if directory.join("worker-release.json").exists() { axiom_server::worker_package::verify_bundle(directory)? } else { axiom_server::release::verify_bundle(directory, env!("CARGO_PKG_VERSION"))? }
        ),
        ServerAction::RollbackCheck { current, previous } => println!(
            "{}",
            axiom_server::release::rollback_check(current, previous, env!("CARGO_PKG_VERSION"))?
        ),
        ServerAction::RunBundle { .. } | ServerAction::Dev { .. } => {
            anyhow::bail!("execution requires async command routing")
        }
        _ => return Ok(false),
    }
    Ok(true)
}
