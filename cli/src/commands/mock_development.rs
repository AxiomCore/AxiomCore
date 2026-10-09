//! Mode-locked, source/fixture snapshot reload using the real supervisor's
//! bounded capture, listener readiness, process ownership and drain primitives.
use super::{
    server_command_contract::{PreparationInput, ResolvedInput},
    server_development::{self, Snapshot},
};
use anyhow::{bail, ensure, Context, Result};
use axiom_lib::{backend::*, contract::AxiomFile};
use axiom_server::{
    development::ValidatedMockFixtures, host::ActionHost, invocation::InvocationCancellation,
    simulation::MockIdentities,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::Command;

#[derive(Clone)]
struct Binding {
    contract: AxiomFile,
    plan: ServerPlan,
}
struct Candidate {
    snapshot: Snapshot,
    binding: Option<Binding>,
}

async fn compile(
    source: &Path,
    variant: Option<&str>,
    resolved: &ResolvedInput,
    files: Vec<(PathBuf, Vec<u8>)>,
    auxiliary: Vec<Vec<u8>>,
    anchor: Option<&Binding>,
    selected: Option<ServingMode>,
) -> Result<Candidate> {
    let directory = tempfile::Builder::new()
        .prefix("axiom-mock-dev-")
        .tempdir()?;
    for (relative, bytes) in files {
        let path = directory.path().join(relative);
        fs::create_dir_all(path.parent().context("invalid snapshot path")?)?;
        fs::write(path, bytes)?;
    }
    let status = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new(std::env::current_exe()?)
            .current_dir(directory.path())
            .arg("build")
            .arg(source)
            .args(["--out", "axiom.axiom"])
            .args(variant.map(|v| vec!["--variant", v]).unwrap_or_default())
            .env("CI", "true")
            .kill_on_drop(true)
            .status(),
    )
    .await
    .context("mock development build exceeded 30 seconds")??;
    ensure!(
        status.success(),
        "mock source does not compile; previous session remains active"
    );
    let contract_path = directory.path().join("axiom.axiom");
    let plan_path = directory.path().join("axiom.server.json");
    let mode = resolved.mode(plan_path.is_file())?;
    ensure!(
        selected.is_none_or(|m| m == mode),
        "mock reload cannot change the selected server/contract profile; restart explicitly"
    );
    let private = directory.path().join(".axiom-dev");
    fs::create_dir(&private)?;
    let mut args: Vec<std::ffi::OsString> = vec!["mock".into()];
    let binding = if mode == ServingMode::ServerMock {
        let contract: AxiomFile = axiom_server::serve::read_json(&contract_path, 16_777_216)?;
        let plan: ServerPlan = axiom_server::serve::read_json(&plan_path, 16_777_216)?;
        let mut fixture: MockFixtures =
            axiom_server::serve::parse_json(&auxiliary[0], MAX_FIXTURE_BYTES)?;
        if let Some(anchor) = anchor {
            validate_mock_reload(&anchor.contract, &anchor.plan, &contract, &plan)?;
            let bound = (&fixture.contract_sha256, &fixture.plan_sha256);
            ensure!(bound == (&anchor.plan.contract_sha256,&anchor.plan.sha256()?) || bound == (&plan.contract_sha256,&plan.sha256()?),
                "reload fixture must bind the reviewed session baseline or exact candidate; arbitrary hash drift is rejected");
            fixture.contract_sha256 = plan.contract_sha256.clone();
            fixture.plan_sha256 = plan.sha256()?;
        }
        let bytes = serde_json::to_vec(&fixture)?;
        let fixtures = ValidatedMockFixtures::from_json(&bytes, &contract, &plan)?;
        let identities = if resolved.identities.is_some() {
            MockIdentities::from_json(&auxiliary[1])?
        } else {
            MockIdentities::default()
        };
        // Preflight has no listener and no real implementation/provider registry.
        let host = ActionHost::simulated(contract.clone(), plan.clone(), fixtures, identities)?;
        tokio::task::spawn_blocking(move || host.shutdown(Duration::from_secs(5))).await??;
        let fixture_path = private.join("mock.json");
        fs::write(&fixture_path, bytes)?;
        args.extend([
            contract_path.into_os_string(),
            "--plan".into(),
            plan_path.into_os_string(),
            "--profile".into(),
            "server".into(),
            "--fixtures".into(),
            fixture_path.into_os_string(),
        ]);
        if resolved.identities.is_some() {
            let path = private.join("identities.json");
            fs::write(&path, &auxiliary[1])?;
            args.extend(["--identities".into(), path.into_os_string()]);
        }
        Some(Binding { contract, plan })
    } else {
        let source = directory.path().join(source);
        let evaluated = axiom_extractor::evaluate_acore_config_with_json(
            source.to_str().context("source path must be UTF-8")?,
            variant,
        )?;
        let config: axiom_mock::models::AxiomConfig = serde_json::from_str(&evaluated.json)?;
        axiom_mock::validation::validate_mock_config(&config).map_err(|e| anyhow::anyhow!(e))?;
        args.extend([
            source.into_os_string(),
            "--profile".into(),
            "contract".into(),
        ]);
        if let Some(variant) = variant {
            args.extend(["--variant".into(), variant.into()]);
        }
        None
    };
    if resolved.debug {
        args.push("--debug".into());
    }
    Ok(Candidate {
        snapshot: Snapshot {
            directory,
            launch_arguments: Some(args),
            mode: if mode == ServingMode::ServerMock {
                "server-mock"
            } else {
                "contract-mock"
            },
        },
        binding,
    })
}

pub(super) async fn supervise(
    resolved: ResolvedInput,
    cancellation: InvocationCancellation,
) -> Result<()> {
    let PreparationInput::Source { path, variant } = &resolved.input else {
        bail!("mock watch requires local source input");
    };
    let listen = std::net::SocketAddr::new(resolved.host, resolved.port);
    ensure!(
        listen.ip().is_loopback() && listen.port() != 0,
        "mock development requires a fixed loopback listen address"
    );
    let root = path.parent().context("source has no project directory")?;
    let source = Path::new(path.file_name().context("source has no filename")?);
    let auxiliary = resolved
        .fixtures
        .iter()
        .chain(resolved.identities.iter())
        .cloned()
        .collect::<Vec<_>>();
    let (files, configs, mut fingerprint) = server_development::collect_inputs(root, &auxiliary)?;
    let initial = tokio::select! {result=compile(source,variant.as_deref(),&resolved,files,configs,None,None)=>result?, _=server_development::cancelled(&cancellation)=>return Ok(())};
    let selected = if initial.binding.is_some() {
        ServingMode::ServerMock
    } else {
        ServingMode::ContractMock
    };
    let anchor = initial.binding.clone();
    if cancellation.is_cancelled() {
        return Ok(());
    }
    let mut running = server_development::start(initial.snapshot, listen).await?;
    loop {
        tokio::select! {_=server_development::cancelled(&cancellation)=>break, _=tokio::time::sleep(Duration::from_millis(250))=>{}}
        if let Some(status) = running.child.try_wait()? {
            bail!("mock development service exited unexpectedly ({status})");
        }
        let (files, configs, next) = match server_development::collect_inputs(root, &auxiliary) {
            Ok(v) => v,
            Err(_) => {
                eprintln!("Mock reload snapshot rejected; previous session remains active");
                continue;
            }
        };
        if next == fingerprint {
            continue;
        }
        fingerprint = next;
        let candidate = match tokio::select! {result=compile(source,variant.as_deref(),&resolved,files,configs,anchor.as_ref(),Some(selected))=>result, _=server_development::cancelled(&cancellation)=>break}
        {
            Ok(candidate) => candidate,
            Err(error) => {
                eprintln!("Mock reload rejected: {error:#}; previous session remains active");
                continue;
            }
        };
        if cancellation.is_cancelled() {
            break;
        }
        server_development::stop(&mut running.child).await?;
        match server_development::start(candidate.snapshot, listen).await {
            Ok(replacement) => running = replacement,
            Err(error) => {
                eprintln!(
                    "Mock reload startup failed: {error:#}; restarting last validated snapshot"
                );
                running = server_development::start(running.snapshot, listen)
                    .await
                    .context("last validated mock snapshot could not restart")?;
            }
        }
    }
    server_development::stop(&mut running.child).await?;
    println!(
        "{}",
        serde_json::json!({"dev":"stopped","mode":if selected==ServingMode::ServerMock{"server-mock"}else{"contract-mock"},"drained":true})
    );
    Ok(())
}
