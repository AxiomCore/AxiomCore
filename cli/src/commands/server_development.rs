//! Snapshot-based local reload. Reviewed grants and provider interfaces are retained.
use anyhow::{ensure, Context, Result};
use axiom_lib::{backend::*, contract::AxiomFile};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    Ok(axiom_server::serve::read(path, 16_777_216)?)
}
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, ChildStdout, Command},
};

pub(super) struct Snapshot {
    pub directory: tempfile::TempDir,
    pub launch_arguments: Option<Vec<std::ffi::OsString>>,
    pub mode: &'static str,
}
pub(super) struct Running {
    pub child: Child,
    pub snapshot: Snapshot,
    _stdout: BufReader<ChildStdout>,
}

fn files(root: &Path, directory: &Path, out: &mut Vec<PathBuf>, depth: usize) -> Result<()> {
    ensure!(
        depth <= 16 && out.len() < 1024,
        "development project exceeds file/depth limits"
    );
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if !name.starts_with('.')
                && !matches!(
                    name.as_ref(),
                    "target" | "node_modules" | "build" | "dist" | "venv" | "__pycache__"
                )
            {
                files(root, &entry.path(), out, depth + 1)?;
            }
        } else if kind.is_file() {
            let path = entry.path();
            let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !name.ends_with(".server.json")
                && !name.ends_with("release.json")
                && (matches!(
                    extension,
                    "acore"
                        | "toml"
                        | "json"
                        | "py"
                        | "go"
                        | "rs"
                        | "ts"
                        | "lock"
                        | "lockfile"
                        | "sum"
                ) || name.starts_with("AxiomDeps")
                    || name == "requirements.txt")
            {
                ensure!(path.starts_with(root), "development file escaped project");
                ensure!(
                    out.len() < 1024,
                    "development project exceeds 1024 source files"
                );
                out.push(path);
            }
        }
    }
    Ok(())
}
fn collect(
    root: &Path,
    deployment: &Path,
    bindings: Option<&Path>,
    delivery: Option<&Path>,
) -> Result<(Vec<(PathBuf, Vec<u8>)>, Vec<Vec<u8>>, String)> {
    let auxiliary = [Some(deployment), bindings, delivery]
        .into_iter()
        .flatten()
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    collect_inputs(root, &auxiliary)
}
/// Capture source and auxiliary bytes together; candidates never reopen a
/// mutable fixture/configuration after their fingerprint has been selected.
pub(super) fn collect_inputs(
    root: &Path,
    auxiliary: &[PathBuf],
) -> Result<(Vec<(PathBuf, Vec<u8>)>, Vec<Vec<u8>>, String)> {
    let mut paths = vec![];
    files(root, root, &mut paths, 0)?;
    paths.sort();
    let mut collected = vec![];
    let mut hash = Sha256::new();
    let mut size = 0usize;
    for path in paths {
        let data = read_bounded(&path)?;
        size = size.saturating_add(data.len());
        ensure!(
            size <= 16_777_216,
            "development source snapshot exceeds 16 MiB"
        );
        let relative = path.strip_prefix(root)?.to_path_buf();
        hash.update(relative.to_string_lossy().as_bytes());
        hash.update([0]);
        hash.update(&data);
        hash.update([0]);
        collected.push((relative, data));
    }
    let mut captured = vec![];
    for path in auxiliary {
        // Reuse the captured file when it belongs to the source snapshot.
        let data = match path
            .strip_prefix(root)
            .ok()
            .and_then(|relative| collected.iter().find(|(p, _)| p == relative))
        {
            Some((_, data)) => data.clone(),
            None => read_bounded(path)?,
        };
        size = size.saturating_add(data.len());
        ensure!(
            size <= 33_554_432,
            "development source/configuration snapshot exceeds 32 MiB"
        );
        hash.update(path.to_string_lossy().as_bytes());
        hash.update([0]);
        hash.update(&data);
        hash.update([0]);
        captured.push(data);
    }
    Ok((collected, captured, hex::encode(hash.finalize())))
}
async fn compile(
    source_name: &Path,
    _deployment: &Path,
    bindings: Option<&Path>,
    delivery: Option<&Path>,
    files: Vec<(PathBuf, Vec<u8>)>,
    configuration: Vec<Vec<u8>>,
    variant: Option<&str>,
    rebind: bool,
) -> Result<Snapshot> {
    let directory = tempfile::Builder::new()
        .prefix("axiom-server-dev-")
        .tempdir()?;
    for (relative, data) in files {
        let path = directory.path().join(relative);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, data)?;
    }
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new(std::env::current_exe()?)
            .current_dir(directory.path())
            .arg("build")
            .arg(source_name)
            .args(["--out", "axiom.axiom"])
            .args(variant.map(|v| vec!["--variant", v]).unwrap_or_default())
            .env("CI", "true")
            .kill_on_drop(true)
            .status(),
    )
    .await
    .context("development build exceeded 30 seconds")??;
    ensure!(
        result.success(),
        "development source does not compile; the previous service remains active"
    );
    let c: AxiomFile =
        axiom_server::serve::read_json(&directory.path().join("axiom.axiom"), 16_777_216)?;
    let p: ServerPlan =
        axiom_server::serve::read_json(&directory.path().join("axiom.server.json"), 16_777_216)?;
    p.validate(&c)?;
    let mut d: Deployment = axiom_server::serve::parse_json(&configuration[0], 16_777_216)?;
    // This is a temporary development binding, never a modification of reviewed
    // configuration. Existing interfaces and grants must satisfy the new plan.
    if rebind {
        d.plan_sha256 = p.sha256()?;
    } else {
        ensure!(d.plan_sha256 == p.sha256()?, "reviewed deployment must match the initial private plan exactly; rebuild and review configuration before serving");
    }
    d.validate(&p, &axiom_server::host_capabilities(true, delivery.is_some()))?;
    ensure!(
        p.actions
            .values()
            .all(|a| !matches!(a.implementation, Implementation::Native { .. })),
        "native actions require a registered Rust host; use its development runner"
    );
    fs::write(
        directory.path().join("deployment.json"),
        serde_jcs::to_vec(&d)?,
    )?;
    if bindings.is_some() {
        let mut value: serde_json::Value =
            axiom_server::serve::parse_json(&configuration[1], 16_777_216)?;
        if rebind {
            value["planSha256"] = json!(p.sha256()?);
        } else {
            ensure!(
                value["planSha256"] == json!(p.sha256()?),
                "initial implementation bindings must match the exact private plan"
            );
        }
        fs::write(
            directory.path().join("bindings.json"),
            serde_jcs::to_vec(&value)?,
        )?;
    }
    if delivery.is_some() {
        let index=1+usize::from(bindings.is_some());
        let text=std::str::from_utf8(&configuration[index])?;
        let mut config=axiom_server::delivery::DeliveryDeployment::from_json(text)?;
        if rebind {config.plan_sha256=p.sha256()?;} else {ensure!(config.plan_sha256==p.sha256()?,"initial delivery configuration must match the exact private plan");}
        fs::write(directory.path().join("delivery.json"),serde_jcs::to_vec(&config)?)?;
    }
    Ok(Snapshot {
        directory,
        launch_arguments: None,
        mode: "real",
    })
}
fn arguments(snapshot: &Snapshot, operation: &str) -> Vec<std::ffi::OsString> {
    if let Some(arguments) = &snapshot.launch_arguments {
        return arguments.clone();
    }
    let root = snapshot.directory.path();
    let mut args = vec![
        "server".into(),
        operation.into(),
        root.join("axiom.axiom").into_os_string(),
        root.join("axiom.server.json").into_os_string(),
        "--deployment".into(),
        root.join("deployment.json").into_os_string(),
    ];
    if root.join("bindings.json").exists() {
        args.extend([
            "--bindings".into(),
            root.join("bindings.json").into_os_string(),
        ]);
    }
    if root.join("delivery.json").exists() {args.extend(["--delivery".into(),root.join("delivery.json").into_os_string()]);}
    args
}
async fn preflight(snapshot: &Snapshot) -> Result<()> {
    let valid = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new(std::env::current_exe()?)
            .args(arguments(&snapshot, "validate"))
            .kill_on_drop(true)
            .status(),
    )
    .await
    .context("development preflight exceeded 30 seconds")??;
    ensure!(
        valid.success(),
        "reload preflight failed: check credentials, provider configuration, grants and handler bindings"
    );
    Ok(())
}
pub(super) async fn start(snapshot: Snapshot, listen: std::net::SocketAddr) -> Result<Running> {
    let mock = snapshot.launch_arguments.is_some();
    let mut args = arguments(&snapshot, "run");
    if mock {
        args.extend([
            "--host".into(),
            listen.ip().to_string().into(),
            "--port".into(),
            listen.port().to_string().into(),
        ]);
    } else {
        args.extend(["--listen".into(), listen.to_string().into()]);
    }
    let mut child = Command::new(std::env::current_exe()?)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdout = BufReader::new(child.stdout.take().context("missing service stdout")?);
    let ready = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut line = String::new();
            if stdout.read_line(&mut line).await? == 0 || line.len() > 65_536 {
                return Ok::<_, std::io::Error>(None);
            }
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                if value["ready"] == true {
                    return Ok(Some(value));
                }
            }
        }
    })
    .await;
    let mut ready = if let Ok(Ok(Some(value))) = ready {
        value
    } else {
        stop(&mut child).await?;
        anyhow::bail!("service failed before readiness; inspect its startup diagnostic");
    };
    if ready.get("mode").is_some_and(|m| m != snapshot.mode) {
        stop(&mut child).await?;
        anyhow::bail!("reload changed the selected execution mode");
    }
    ready["mode"] = json!(snapshot.mode);
    ready["dev"] = json!("ready");
    ready["listen"] = json!(listen);
    ready["reload"] = json!("snapshot");
    ready["reviewedGrants"] = json!("retained");
    let plan:ServerPlan=axiom_server::serve::read_json(&snapshot.directory.path().join("axiom.server.json"),16_777_216)?;
    ready["planSha256"] = json!(plan.sha256()?);
    println!("{ready}");
    Ok(Running {
        child,
        snapshot,
        _stdout: stdout,
    })
}
pub(super) async fn stop(child: &mut Child) -> Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        if let Some(id) = child.id() {
            Command::new("/bin/kill")
                .args(["-TERM", &id.to_string()])
                .status()
                .await?;
        }
    }
    #[cfg(not(unix))]
    child.start_kill()?;
    if tokio::time::timeout(Duration::from_secs(6), child.wait())
        .await
        .is_err()
    {
        child.kill().await?;
    }
    Ok(())
}
pub(super) async fn cancelled(token: &axiom_server::invocation::InvocationCancellation) {
    while !token.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
pub(super) async fn supervise(
    source: &Path,
    deployment: &Path,
    bindings: Option<&Path>,
    delivery: Option<&Path>,
    listen: std::net::SocketAddr,
    poll_ms: u64,
    variant: Option<&str>,
    cancellation: axiom_server::invocation::InvocationCancellation,
) -> Result<()> {
    ensure!((50..=2000).contains(&poll_ms), "poll-ms must be 50..2000");
    ensure!(
        listen.ip().is_loopback() && listen.port() != 0,
        "development requires a fixed loopback listen address"
    );
    let source = fs::canonicalize(source)?;
    let root = source.parent().context("source has no project directory")?;
    let source_name = source.file_name().context("source has no filename")?;
    let deployment = fs::canonicalize(deployment)?;
    let bindings = bindings.map(fs::canonicalize).transpose()?;
    let delivery = delivery.map(fs::canonicalize).transpose()?;
    let (initial, configuration, mut fingerprint) =
        collect(root, &deployment, bindings.as_deref(), delivery.as_deref())?;
    let snapshot = tokio::select! {
        result=compile(Path::new(source_name), &deployment, bindings.as_deref(), delivery.as_deref(), initial, configuration, variant, false)=>result?,
        _=cancelled(&cancellation)=>return Ok(()),
    };
    tokio::select! {
        result=preflight(&snapshot)=>result?,
        _=cancelled(&cancellation)=>return Ok(()),
    }
    if cancellation.is_cancelled() {
        return Ok(());
    }
    let mut running = start(snapshot, listen).await?;
    loop {
        if cancellation.is_cancelled() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
        if cancellation.is_cancelled() {
            break;
        }
        if let Some(status) = running.child.try_wait()? {
            anyhow::bail!(
                "development service exited unexpectedly ({status}); inspect the startup/runtime diagnostic"
            );
        }
        let (files, configuration, next) = match collect(root, &deployment, bindings.as_deref(), delivery.as_deref()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Development snapshot failed: {e:#}");
                continue;
            }
        };
        if next == fingerprint {
            continue;
        }
        fingerprint = next;
        let candidate = match tokio::select! {
            result=compile(Path::new(source_name), &deployment, bindings.as_deref(), delivery.as_deref(), files, configuration, variant, true)=>result,
            _=cancelled(&cancellation)=>break,
        } {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Reload rejected: {e:#}");
                continue;
            }
        };
        if cancellation.is_cancelled() {
            break;
        }
        let preflight = tokio::select! { result=preflight(&candidate)=>result, _=cancelled(&cancellation)=>break };
        if let Err(error) = preflight {
            eprintln!("Reload preflight rejected: {error:#}");
            continue;
        }
        if cancellation.is_cancelled() {
            break;
        }
        stop(&mut running.child).await?;
        match start(candidate, listen).await {
            Ok(replacement) => running = replacement,
            Err(error) => {
                eprintln!(
                    "Reload startup failed: {error:#}; restarting the last validated snapshot"
                );
                running=start(running.snapshot,listen).await.context("previous snapshot could not restart; check external provider/credential availability")?;
            }
        }
    }
    stop(&mut running.child).await?;
    println!("{}", json!({"dev":"stopped","drained":true}));
    Ok(())
}
pub async fn run(
    source: &Path,
    deployment: &Path,
    bindings: Option<&Path>,
    delivery: Option<&Path>,
    listen: std::net::SocketAddr,
    poll_ms: u64,
) -> Result<()> {
    let cancellation = axiom_server::invocation::InvocationCancellation::default();
    let token = cancellation.clone();
    let signal = tokio::spawn(async move {
        axiom_server::development::shutdown_signal().await;
        token.cancel();
    });
    let result = supervise(
        source,
        deployment,
        bindings,
        delivery,
        listen,
        poll_ms,
        None,
        cancellation,
    )
    .await;
    signal.abort();
    result
}
