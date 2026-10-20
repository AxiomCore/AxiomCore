//! Protected development reload: compile without sources, resolve a candidate,
//! atomically publish it, then drain the previous immutable generation.
use super::server_development::{self, Snapshot};
use anyhow::{Context, Result};
use axiom_lib::backend::ServerPlan;
use axiom_server::{host::ActionHost, invocation::InvocationCancellation, serve::HostGenerations};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(PartialEq, Eq)]
struct PrivateRevision(Vec<(PathBuf, Option<(u64, u64, u64, u64, u64, u64)>)>);
fn private_revision(root: &Path) -> Result<PrivateRevision> {
    let mut paths = axiom_lib::compiler_sandbox::private_source_paths(root)?;
    paths.push(root.join("AxiomSecrets.toml"));
    paths.sort();
    paths.dedup();
    Ok(PrivateRevision(
        paths
            .into_iter()
            .map(|path| {
                #[cfg(unix)]
                let revision = std::fs::symlink_metadata(&path).ok().map(|meta| {
                    use std::os::unix::fs::MetadataExt;
                    (
                        meta.ino(),
                        meta.mtime() as u64,
                        meta.mtime_nsec() as u64,
                        meta.ctime() as u64,
                        meta.ctime_nsec() as u64,
                        meta.mode() as u64,
                    )
                });
                #[cfg(not(unix))]
                let revision = None;
                (path, revision)
            })
            .collect(),
    ))
}
async fn prepare(snapshot: &Snapshot, root: &Path, environment: &str) -> Result<Arc<ActionHost>> {
    let directory = snapshot.directory.path().to_path_buf();
    let profile = super::server::SecretProfileArgs {
        environment: Some(environment.into()),
        secret_root: Some(root.into()),
    };
    tokio::task::spawn_blocking(move || {
        let contract = directory.join("axiom.axiom");
        let plan_path = directory.join("axiom.server.json");
        let plan: ServerPlan = axiom_server::serve::read_json(&plan_path, 16_777_216)?;
        let secrets = profile.resolve(&contract, &plan)?;
        let bindings = directory.join("bindings.json");
        let delivery = directory.join("delivery.json");
        super::serve::load_host_with_resolver(
            &contract,
            &plan_path,
            &directory.join("deployment.json"),
            bindings.exists().then_some(bindings.as_path()),
            delivery.exists().then_some(delivery.as_path()),
            secrets,
        )
    })
    .await
    .context("protected generation preparation failed")?
}
pub(super) async fn supervise(
    source: &Path,
    deployment: &Path,
    bindings: Option<&Path>,
    delivery: Option<&Path>,
    listen: std::net::SocketAddr,
    debug: bool,
    variant: Option<&str>,
    environment: &str,
    cancellation: InvocationCancellation,
) -> Result<()> {
    anyhow::ensure!(
        listen.ip().is_loopback() && listen.port() != 0,
        "protected development requires a fixed loopback listener"
    );
    let source = source.canonicalize()?;
    let root = source.parent().context("source root unavailable")?;
    let source_name = Path::new(source.file_name().context("source name unavailable")?);
    let deployment = deployment.canonicalize()?;
    let bindings = bindings.map(Path::canonicalize).transpose()?;
    let delivery = delivery.map(Path::canonicalize).transpose()?;
    let auxiliary = [Some(deployment.clone()), bindings.clone(), delivery.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let (files, configuration, mut fingerprint) =
        server_development::collect_inputs(root, &auxiliary)?;
    let mut revision = private_revision(root)?;
    let mut snapshot = server_development::compile(
        root,
        source_name,
        &deployment,
        bindings.as_deref(),
        delivery.as_deref(),
        files,
        configuration,
        variant,
        false,
    )
    .await?;
    let host = prepare(&snapshot, root, environment).await?;
    let generations = Arc::new(HostGenerations::new(host));
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let signal = cancellation.clone();
    let serving_generations = generations.clone();
    let mut serving = tokio::spawn(async move {
        axiom_server::serve::serve_generations(serving_generations, listener, async move {
            server_development::cancelled(&signal).await
        })
        .await
        .map_err(|_| anyhow::anyhow!("protected development transport stopped"))
    });
    println!(
        "{}",
        serde_json::json!({"ready":true,"mode":"real","listen":listen,"debug":debug,"environment":environment,"reload":"protected-generations"})
    );
    let mut retired: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    loop {
        tokio::select! {
            _=server_development::cancelled(&cancellation)=>break,
            result=&mut serving=>{cancellation.cancel();result??;return Ok(());},
            _=tokio::time::sleep(Duration::from_millis(250))=>{},
        }
        retired.retain(|task| !task.is_finished());
        let inputs = server_development::collect_inputs(root, &auxiliary)
            .and_then(|inputs| Ok((inputs, private_revision(root)?)));
        let ((files, configuration, next), next_revision) = match inputs {
            Ok(value) => value,
            Err(_) => {
                eprintln!(
                    "AXSECRET_RELOAD: candidate inputs unavailable; previous generation retained"
                );
                continue;
            }
        };
        if next == fingerprint && next_revision == revision {
            continue;
        }
        if retired.len() >= 16 {
            eprintln!("AXSECRET_RELOAD: draining generation capacity reached; previous generation retained");
            continue;
        }
        fingerprint = next;
        revision = next_revision;
        let candidate = match server_development::compile(
            root,
            source_name,
            &deployment,
            bindings.as_deref(),
            delivery.as_deref(),
            files,
            configuration,
            variant,
            true,
        )
        .await
        {
            Ok(value) => value,
            Err(_) => {
                eprintln!("AXSECRET_RELOAD: compilation rejected; previous generation retained");
                continue;
            }
        };
        let host = match prepare(&candidate, root, environment).await {
            Ok(value) => value,
            Err(_) => {
                eprintln!("AXSECRET_RELOAD: preflight rejected; previous generation retained");
                continue;
            }
        };
        if cancellation.is_cancelled() {
            let _ =
                tokio::task::spawn_blocking(move || host.shutdown(Duration::from_secs(5))).await;
            break;
        }
        let old = generations.replace(host)?;
        let old_snapshot = std::mem::replace(&mut snapshot, candidate);
        retired.push(tokio::task::spawn_blocking(move || {
            if old.shutdown(Duration::from_secs(5)).is_err() {
                old.cancel_active();
                eprintln!("AXSECRET_RELOAD: retired generation exceeded its drain deadline");
            }
            drop(old);
            drop(old_snapshot);
        }));
        println!(
            "{}",
            serde_json::json!({"reloaded":true,"environment":environment,"reviewedGrants":"retained"})
        );
    }
    cancellation.cancel();
    serving.await??;
    for task in retired {
        let _ = task.await;
    }
    drop(snapshot);
    Ok(())
}
