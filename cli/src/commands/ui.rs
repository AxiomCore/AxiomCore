use anyhow::{bail, Context, Result};
use axiom_build::core::extension_source::{build_rust_source_extension, SourceBuildOptions};
use axiom_extension_host::HostProfile;
use axiom_lib::ui_contract::{read_lock, verify_locked_artifact_bytes, UiOperationKind};
use axiom_lib::{extension_workflow::load_verified_extension_target, package::PackageTarget};
use axiom_ui::{
    capability_registry::{
        phase2_capability_registry, AcoreSupportStatus, CapabilityKind,
        PHASE2_CAPABILITY_REGISTRY_SHA256, PHASE2_CAPABILITY_REGISTRY_VERSION,
        PHASE2_LYNX_UI_NPM_INTEGRITY, PHASE2_LYNX_UI_PACKAGE, PHASE2_LYNX_UI_SOURCE_COMMIT,
        PHASE2_LYNX_UI_VERSION, PHASE2_PINNED_LYNX_COMMIT,
    },
    compact_semantic_context, compile_ui_source, native_reload_directive, HotReloadEvent,
    HotReloadOutcome, NativeReloadDirective, SafeEditStatus, UiActionStep, UiCompilation,
    UiCompileOptions, UiDevelopmentSession, UiTarget, VirtualReactLynxBuild,
};
use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{header, Response, StatusCode},
    response::{sse::Event as SseEvent, IntoResponse, Sse},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use clap::ValueEnum;
use dialoguer::{theme::ColorfulTheme, Confirm};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::IsTerminal;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};

const UI_HOST_GITHUB_REPOSITORY: &str = "AxiomCore/axiom-ui-host";
const LYNX_UI_VERSION: &str = "3.138.0";
const LYNX_UI_SOURCE_COMMIT: &str = "b9b3fd7a34d7cde6ef4dddfb2fb95de4f5457d73";
const LYNX_UI_NPM_INTEGRITY: &str =
    "sha512-7j1au6sOIHY+lHnM1iR5UcevSPxOzwGM7mbB1H8s3cZeTgWcrnbPgCoPToetjc2vJ6jymk/+hlduXcOlrDHL/A==";
// These are the exact companion versions used by the pinned lynx-ui source
// revision. The older engine-demo ReactLynx (0.107.0) satisfies lynx-ui's broad
// peer range syntactically, but lacks setEomShouldFlushElementTree and cannot
// link the component package.
const LYNX_REACT_VERSION: &str = "0.123.1";
const LYNX_REACT_NPM_INTEGRITY: &str =
    "sha512-wgkSly8Nv3O6tdWJ5tc+dgUHp+XpPzJdOy36dPA1np9LXUEWfwkiWs6BBkluPv620c6tyev2Vj3cEjnGHs0orw==";
const LYNX_REACT_RSBUILD_PLUGIN_VERSION: &str = "0.18.1";
const LYNX_REACT_RSBUILD_PLUGIN_NPM_INTEGRITY: &str =
    "sha512-c+ekAagAocry6NjQMIyn67CN++fU8pStM+9khWfM2ioL/j4zL0jePiuV49gBvLBCErayOrMRB2+/IrokhnIBkw==";
const LYNX_RSPEEDY_VERSION: &str = "0.16.1";
const LYNX_RSPEEDY_NPM_INTEGRITY: &str =
    "sha512-E8FVmglBIfpyZ7AuBlB4Nl+H99uO5F+Hx35HHYW7ySuB/gFxh2+JNx9z5ll1CvfK76ePQmVdsB99ZnVobBPV1w==";
const LYNX_TYPES_VERSION: &str = "4.1.0";
const LYNX_TYPES_NPM_INTEGRITY: &str =
    "sha512-WLWnMpGZMrjffSR62PYesSfjD0RWumPyT21iMTi7GySGkOSOGdv4q4wWkarg8zNI7SxcCFM6kpXFhyIERADBPQ==";
const TYPESCRIPT_VERSION: &str = "5.8.3";
const TYPES_REACT_VERSION: &str = "18.3.28";
// This is a public Ed25519 verification key, not a signing secret. Releases
// from the fixed Axiom-owned repository must verify with this key before an
// archive reaches the local host cache. An environment override supports a
// deliberate future key rotation in controlled environments.
const UI_HOST_RELEASE_PUBLIC_KEY_HEX: &str =
    "85f0729ca36ada0ef648b08e0284085bf08815b1a291e926e2df555b1bf3a721";

#[derive(Debug, Clone, ValueEnum)]
pub enum UiInspectView {
    Ir,
    Lowered,
    Facade,
    Context,
    /// Trace source-level extension imports through their exact action
    /// bindings without executing a guest module.
    Extensions,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum UiCapabilityKind {
    Element,
    Component,
    Property,
    AtRule,
    Datatype,
    Selector,
}

impl From<UiCapabilityKind> for CapabilityKind {
    fn from(value: UiCapabilityKind) -> Self {
        match value {
            UiCapabilityKind::Element => Self::Element,
            UiCapabilityKind::Component => Self::Component,
            UiCapabilityKind::Property => Self::Property,
            UiCapabilityKind::AtRule => Self::AtRule,
            UiCapabilityKind::Datatype => Self::Datatype,
            UiCapabilityKind::Selector => Self::Selector,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiDoctorReport {
    format: &'static str,
    target: String,
    compiler: &'static str,
    virtual_graph: &'static str,
    native_host: &'static str,
    ready_for_native_run: bool,
    diagnostic_code: &'static str,
    remediation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_abi_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_module_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_facade_protocol_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiHostInstallation {
    format: String,
    target: String,
    host_root: PathBuf,
    #[serde(default)]
    artifact: Option<UiHostArtifact>,
    delivery_adapter_ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiHostArtifact {
    version: String,
    variant: String,
    file: String,
    sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IosDeliveryMode {
    StateResetTemplate,
}

impl IosDeliveryMode {
    fn as_protocol_value(self) -> &'static str {
        match self {
            Self::StateResetTemplate => "state_reset_template",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IosDeliveryAcknowledgement {
    AppliedStatePreserved,
    AppliedStateReset { reason: String },
}

#[derive(Debug, Clone)]
enum NativeDiagnosticSource {
    Ios(PathBuf),
    Android(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeUiDiagnostic {
    id: u64,
    sequence: u64,
    graph_revision: String,
    severity: String,
    code: String,
    message: String,
    #[serde(default)]
    suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NativeUiDiagnosticEnvelope {
    format: String,
    diagnostics: Vec<NativeUiDiagnostic>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiHostReleaseManifest {
    format: String,
    version: String,
    assets: Vec<UiHostReleaseAsset>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UiHostReleaseAsset {
    target: String,
    variant: String,
    file: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubReleaseAsset {
    name: String,
    browser_download_url: String,
}

fn ui_host_spinner(message: impl Into<String>) -> ProgressBar {
    let progress = if std::io::stderr().is_terminal() {
        ProgressBar::new_spinner()
    } else {
        ProgressBar::hidden()
    };
    progress.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .expect("valid UI Host spinner template"),
    );
    progress.set_message(message.into());
    progress.enable_steady_tick(Duration::from_millis(100));
    progress
}

fn ui_host_download_progress(asset_name: &str, content_length: Option<u64>) -> ProgressBar {
    let progress = match content_length {
        Some(total) if std::io::stderr().is_terminal() => ProgressBar::new(total),
        Some(_) => ProgressBar::hidden(),
        None => ui_host_spinner(format!("Downloading {asset_name}...")),
    };
    if content_length.is_some() && std::io::stderr().is_terminal() {
        progress.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
            )
            .expect("valid UI Host download progress template")
            .progress_chars("=> "),
        );
        progress.set_message(format!("Downloading {asset_name}"));
        progress.enable_steady_tick(Duration::from_millis(100));
    }
    progress
}

/// Create only authored application files. Compiler/lowering output remains
/// virtual and is never part of this starter directory.
pub async fn handle_init(directory: PathBuf, target: String) -> Result<()> {
    let target = parse_target(&target)?;
    if directory.exists()
        && std::fs::read_dir(&directory)
            .with_context(|| format!("cannot inspect {}", directory.display()))?
            .next()
            .is_some()
    {
        bail!(
            "refusing to initialize non-empty directory {}; choose an empty directory",
            directory.display()
        );
    }
    std::fs::create_dir_all(directory.join("src"))?;
    std::fs::create_dir_all(directory.join("tests"))?;
    write_new(
        &directory.join("axiom.app.toml"),
        &format!(
            "[app]\nid = \"starter\"\ntarget = \"{}\"\n\n[source]\nentry = \"src/main.acore\"\n",
            target.as_str()
        ),
    )?;
    write_new(&directory.join("src/main.acore"), STARTER_SOURCE)?;
    write_new(&directory.join("tests/smoke.acore"), STARTER_TEST)?;
    write_new(&directory.join("README.md"), STARTER_README)?;
    println!(
        "Created authored Acore UI starter at {}. Next: axiom run src/main.acore --target {}",
        directory.display(),
        target.as_str()
    );
    Ok(())
}

const IOS_HOST_BUNDLE_ID: &str = "dev.axiomcore.uihost";
const ANDROID_HOST_BUNDLE_ID: &str = "dev.axiomcore.uihost";
const ANDROID_HOST_ACTIVITY_COMPONENT: &str =
    "dev.axiomcore.uihost/com.axiom.uihost.AxiomHostActivity";
const HOST_ARTIFACT_MARKER: &str = "axiom.host.artifact.sha256";
const NATIVE_RUNTIME_CONFIG_MARKER: &str = "axiom.runtime.config.sha256";
// Host protocol v3 retains the v2 delivery acknowledgement and adds a bounded,
// graph-bound Lynx diagnostic channel. Older hosts can render bundles but
// cannot provide the error visibility required by `axiom run`.
// 0.6.5 is the first host release containing the complete Phase 2 renderer
// surface, including Web stylesheet delivery and component scope boundaries.
// Treating an older host as current is especially deceptive on Web:
// interaction still works while presentation is incomplete or absent.
const IOS_HOST_PROTOCOL_MINIMUM: &str = "0.6.5";
// Android delivery uses an Axiom-owned development APK. It is separate from a
// future end-user application build because adb app-private transport requires
// a debuggable host.
// 0.6.5 also retains the supported ReadableMap/JavaOnlyMap reflection boundary
// and emulator loopback routing introduced by the earlier Android host.
const ANDROID_HOST_PROTOCOL_MINIMUM: &str = "0.6.5";
const WEB_HOST_PROTOCOL_MINIMUM: &str = "0.6.5";
const LYNX_ENGINE_SOURCE: &str = "https://github.com/lynx-family/lynx.git";
const LYNX_ENGINE_COMMIT: &str = "73bf89185547d0caf725bab4f3a46fa1e1f9616d";

fn ui_debug_enabled() -> bool {
    ["AXIOM_UI_DEBUG", "DEBUG"].into_iter().any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|value| ui_debug_value(&value))
    })
}

fn ui_debug_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Run the compiler core without materializing generated files or invoking a
/// native host. The continuous session command below owns watching/revisions.
pub async fn handle_check(source: PathBuf, lock: PathBuf, target: String) -> Result<()> {
    let target = parse_target(&target)?;
    let result = compile_path(&source, lock.clone(), target)?;
    let virtual_build = result
        .virtual_build
        .expect("valid compilation lowers virtually");
    println!(
        "Checked UI module {} ({} virtual read-only file(s), graph {}).",
        result.ir.expect("valid compilation has IR").module,
        virtual_build.files.len(),
        virtual_build.graph_revision
    );
    Ok(())
}

/// Own a long-lived file-watch session. Authored Acore remains the only source
/// in the workspace; the adapter uses a permission-private cache because the
/// pinned ReactLynx compiler requires filesystem inputs.
pub async fn handle_run(source: PathBuf, lock: PathBuf, target: String, once: bool) -> Result<()> {
    handle_run_internal(source, lock, target, once, None).await
}

pub async fn handle_run_with_packages(
    source: PathBuf,
    lock: PathBuf,
    package_lock: PathBuf,
    target: String,
    once: bool,
) -> Result<()> {
    handle_run_internal(source, lock, target, once, Some(package_lock)).await
}

async fn handle_run_internal(
    source: PathBuf,
    lock: PathBuf,
    target: String,
    once: bool,
    package_lock: Option<PathBuf>,
) -> Result<()> {
    let target = parse_target(&target)?;
    if target == UiTarget::Web && !once {
        return handle_run_web(source, lock, package_lock).await;
    }
    // `--once` is the non-interactive compiler/CI form. A normal development
    // session owns the friendly UI Host installation prompt.
    let host = if once {
        None
    } else {
        Some(ensure_ui_host(target).await?)
    };
    // `Path::parent("main.acore")` is an empty path rather than `None`.
    // Passing that empty path to notify causes its opaque "No path was found"
    // error after an otherwise useful compiler diagnostic. Normalize relative
    // single-file invocations to the current directory before constructing the
    // recursive authored-input watch root.
    let asset_root = source
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let options = UiCompileOptions {
        target,
        lock_path: lock.clone(),
        asset_root: Some(asset_root.clone()),
    };
    let mut session = match package_lock {
        Some(package_lock) => UiDevelopmentSession::new_with_package_lock(options, package_lock),
        None => UiDevelopmentSession::new(options),
    };
    let initial = apply_path(&mut session, &source)?;
    report_reload(&initial);
    if let Some(host) = host.as_ref() {
        deliver_last_good(&session, host, &initial, &source, &lock, &asset_root)?;
    }
    if once {
        return Ok(());
    }

    let source = canonical_or_original(&source);
    let lock = canonical_or_original(&lock);
    // macOS FSEvents commonly emits more than one notification for a single
    // editor save. A graph revision is the authoritative debouncing key: do
    // not compile, report, or deliver an identical graph twice.
    let mut last_observed_graph_revision = initial.graph_revision.clone();
    let diagnostic_source = host.as_ref().map(native_diagnostic_source).transpose()?;
    let mut seen_native_diagnostics = HashSet::new();
    let mut diagnostic_tick = tokio::time::interval(Duration::from_secs(1));
    diagnostic_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |event| {
            let _ = event_tx.send(event);
        },
        notify::Config::default(),
    )?;
    // Acore and declared assets are the only authored UI inputs. Recursive
    // watching keeps nested `images/` folders responsive without watching the
    // opaque compiler cache or generated toolchain files.
    watch_ui_source_root(&mut watcher, &asset_root)?;
    if !same_path(nonempty_parent(&lock), &asset_root) {
        watch_parent(&mut watcher, &lock)?;
    }
    let dependency_paths = if crate::commands::run::is_managed_development_lock(&lock) {
        crate::commands::run::frontend_dependency_paths(&source)?
    } else {
        Vec::new()
    };
    watch_frontend_dependency_paths(&mut watcher, &dependency_paths, &asset_root)?;

    if ui_debug_enabled() {
        println!(
            "Watching {} for {} UI updates. Native delivery uses an opaque Axiom cache; no generated UI source is written to this workspace. Press Ctrl-C to stop.",
            source.display(), target.as_str()
        );
    } else {
        println!(
            "Watching {} for {} changes. Press Ctrl-C to stop.",
            source.display(),
            target.as_str()
        );
    }
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = diagnostic_tick.tick(), if diagnostic_source.is_some() => {
                for diagnostic in read_native_diagnostics(
                    diagnostic_source.as_ref().expect("guarded diagnostic source"),
                ) {
                    let identity = format!(
                        "{}:{}:{}",
                        diagnostic.sequence, diagnostic.graph_revision, diagnostic.id
                    );
                    if seen_native_diagnostics.insert(identity) {
                        report_native_diagnostic(&diagnostic);
                    }
                }
            }
            event = event_rx.recv() => match event {
                Some(Ok(event)) if event.paths.iter().any(|path| same_path(path, &source) || same_path(path, &lock) || frontend_dependency_changed(path, &dependency_paths) || declared_asset_path(&session, &asset_root, path)) => {
                    let dependency_changed = event.paths.iter().any(|path| frontend_dependency_changed(path, &dependency_paths));
                    if dependency_changed {
                        if let Err(error) = crate::commands::run::prepare_frontend(&source, None, false).await {
                            eprintln!("Could not prepare the changed frontend dependency: {error}");
                            continue;
                        }
                    }
                    match apply_path(&mut session, &source) {
                        Ok(update) => {
                            let graph_changed = observe_graph_revision(
                                &mut last_observed_graph_revision,
                                &update.graph_revision,
                            );
                            if !graph_changed && !dependency_changed {
                                continue;
                            }
                            if graph_changed {
                                report_reload(&update);
                            }
                            if let Some(host) = host.as_ref() {
                                if let Err(error) = deliver_last_good(&session, host, &update, &source, &lock, &asset_root) {
                                    if ui_debug_enabled() {
                                        eprintln!("AXIOM_UI_DELIVERY: {error:#}");
                                    } else {
                                        eprintln!("UI update failed: {error}");
                                    }
                                }
                            }
                        }
                        Err(error) if ui_debug_enabled() => {
                            eprintln!("AXIOM_UI_WATCH_READ: {error:#}")
                        }
                        Err(error) => eprintln!("Could not apply the UI edit: {error}"),
                    }
                }
                Some(Err(error)) if ui_debug_enabled() => {
                    eprintln!("AXIOM_UI_WATCH: {error:#}")
                }
                Some(Err(error)) => eprintln!("UI file watcher error: {error}"),
                Some(Ok(_)) => {},
                None => bail!("Acore UI file watcher stopped unexpectedly"),
            }
        }
    }
}

#[derive(Clone)]
struct WebHostState {
    app: Arc<RwLock<Vec<u8>>>,
    files: Arc<HashMap<String, Vec<u8>>>,
    extension_files: Arc<RwLock<BTreeMap<String, Vec<u8>>>>,
    asset_root: PathBuf,
    declared_assets: Arc<RwLock<HashSet<String>>>,
    reload: broadcast::Sender<String>,
    diagnostics: mpsc::UnboundedSender<WebDiagnostic>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebDiagnostic {
    severity: String,
    code: String,
    message: String,
    #[serde(default)]
    graph_revision: String,
}

async fn web_index(State(state): State<WebHostState>) -> impl IntoResponse {
    web_static_response(&state, "index.html")
}

async fn web_static(
    State(state): State<WebHostState>,
    AxumPath(path): AxumPath<String>,
) -> impl IntoResponse {
    web_static_response(&state, &path)
}

async fn web_extension(
    State(state): State<WebHostState>,
    AxumPath(path): AxumPath<String>,
) -> Response<Body> {
    if path.is_empty()
        || !Path::new(&path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap();
    }
    let full_path = format!("runtime/extensions/{path}");
    let bytes = state
        .extension_files
        .read()
        .expect("extension artifact lock poisoned")
        .get(&full_path)
        .cloned();
    let Some(bytes) = bytes else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap();
    };
    Response::builder()
        .header(
            header::CONTENT_TYPE,
            mime_guess::from_path(&full_path)
                .first_or_octet_stream()
                .as_ref(),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .unwrap()
}

fn web_static_response(state: &WebHostState, path: &str) -> Response<Body> {
    let file = state
        .files
        .get(path)
        .map(|bytes| (path, bytes))
        .or_else(|| {
            (!path.contains('.'))
                .then(|| {
                    state
                        .files
                        .get("index.html")
                        .map(|bytes| ("index.html", bytes))
                })
                .flatten()
        });
    let Some((served_path, bytes)) = file else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap();
    };
    Response::builder()
        .header(
            header::CONTENT_TYPE,
            mime_guess::from_path(served_path)
                .first_or_octet_stream()
                .as_ref(),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes.clone()))
        .unwrap()
}

async fn web_app(State(state): State<WebHostState>) -> Response<Body> {
    let bytes = state.app.read().expect("web app lock poisoned").clone();
    Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .unwrap()
}

async fn web_asset(
    State(state): State<WebHostState>,
    AxumPath(path): AxumPath<String>,
) -> Response<Body> {
    if path.is_empty()
        || !Path::new(&path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        || !state
            .declared_assets
            .read()
            .expect("asset lock poisoned")
            .contains(&path)
    {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap();
    }
    let file = state.asset_root.join(&path);
    match std::fs::read(&file) {
        Ok(bytes) => Response::builder()
            .header(
                header::CONTENT_TYPE,
                mime_guess::from_path(&file)
                    .first_or_octet_stream()
                    .as_ref(),
            )
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(bytes))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap(),
    }
}

async fn web_events(
    State(state): State<WebHostState>,
) -> Sse<impl futures::Stream<Item = std::result::Result<SseEvent, std::convert::Infallible>>> {
    let receiver = state.reload.subscribe();
    let stream = futures::stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(value) => {
                    return Some((
                        Ok(SseEvent::default().event("reload").data(value)),
                        receiver,
                    ))
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

async fn web_diagnostic(
    State(state): State<WebHostState>,
    Json(diagnostic): Json<WebDiagnostic>,
) -> StatusCode {
    if diagnostic.code == "WEB_RUNTIME"
        && matches!(
            diagnostic.message.as_str(),
            "ResizeObserver loop limit exceeded"
                | "ResizeObserver loop completed with undelivered notifications."
        )
    {
        return StatusCode::NO_CONTENT;
    }
    let _ = state.diagnostics.send(diagnostic);
    StatusCode::NO_CONTENT
}

fn extract_web_host(host: &UiHostInstallation) -> Result<HashMap<String, Vec<u8>>> {
    let artifact = host
        .artifact
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("web UI Host has no release artifact"))?;
    let archive_path = host.host_root.join(&artifact.file);
    let bytes = std::fs::read(&archive_path)
        .with_context(|| format!("cannot open web UI Host archive {}", archive_path.display()))?;
    if sha256_bytes(&bytes) != artifact.sha256 {
        bail!("AXIOM_UI_HOST_TAMPERED: installed web host artifact no longer matches its verified digest");
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .context("web UI Host archive is not a valid zip")?;
    let mut files = HashMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_string();
        if !matches!(
            name.as_str(),
            "index.html"
                | "host.css"
                | "host.js"
                | "foreign-island.js"
                | "axiom-extension-browser-kernel.mjs"
                | "axiom-extension-worker.mjs"
                | "wasm-policy.mjs"
                | "axiom_runtime.js"
                | "axiom_runtime_bg.wasm"
        ) {
            continue;
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes)?;
        files.insert(name, bytes);
    }
    for required in web_host_required_files() {
        if !files.contains_key(*required) {
            bail!(
                "AXIOM_UI_HOST_WEB_INCOMPATIBLE: verified web UI Host archive is missing {required}; install a host release built with the current browser extension runtime"
            );
        }
    }
    Ok(files)
}

fn web_host_required_files() -> &'static [&'static str] {
    &[
        "index.html",
        "host.css",
        "host.js",
        "foreign-island.js",
        "axiom-extension-browser-kernel.mjs",
        "axiom-extension-worker.mjs",
        "wasm-policy.mjs",
        "axiom_runtime.js",
        "axiom_runtime_bg.wasm",
    ]
}

/// Confirm that a signed browser host archive actually carries the runtime
/// files claimed by its protocol version. Version metadata alone cannot prove
/// that an older release asset was built after a host-file addition.
fn web_host_archive_is_current(host: &UiHostInstallation) -> Result<bool> {
    if host.target != "web" {
        return Ok(true);
    }
    let artifact = match &host.artifact {
        Some(artifact) => artifact,
        None => return Ok(false),
    };
    let archive_path = host.host_root.join(&artifact.file);
    let bytes = match std::fs::read(&archive_path) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(false),
    };
    if sha256_bytes(&bytes) != artifact.sha256 {
        return Ok(false);
    }
    let mut archive = match zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
        Ok(archive) => archive,
        Err(_) => return Ok(false),
    };
    for required in web_host_required_files() {
        let Ok(entry) = archive.by_name(required) else {
            return Ok(false);
        };
        if entry.size() == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn web_model(compilation: &UiCompilation, lock_path: &Path, hot_reload: bool) -> Result<Vec<u8>> {
    web_model_with_runtime_config(
        compilation,
        hot_reload,
        build_verified_runtime_config_value(compilation, lock_path)?,
    )
}

fn web_model_with_runtime_config(
    compilation: &UiCompilation,
    hot_reload: bool,
    runtime_config: serde_json::Value,
) -> Result<Vec<u8>> {
    let ir = compilation
        .ir
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("valid web compilation has no UI IR"))?;
    let stylesheet = compilation
        .virtual_build
        .as_ref()
        .and_then(|build| build.files.get("virtual/axiom-ui.css"))
        .map(|file| file.content.as_str())
        .unwrap_or("");
    Ok(serde_json::to_vec(&serde_json::json!({
        "format": if hot_reload { "axiom-web-development/v1" } else { "axiom-web-application/v1" },
        "hotReload": hot_reload,
        "graphRevision": compilation.context.graph_revision,
        "ir": ir,
        "stylesheet": stylesheet,
        "runtimeConfig": runtime_config,
    }))?)
}

fn web_declared_assets(compilation: &UiCompilation) -> HashSet<String> {
    compilation
        .ir
        .as_ref()
        .into_iter()
        .flat_map(|ir| ir.assets.iter().map(|asset| asset.path.clone()))
        .collect()
}

fn open_default_browser(url: &str) -> Result<()> {
    if std::env::var("AXIOM_UI_WEB_NO_OPEN")
        .ok()
        .is_some_and(|value| ui_debug_value(&value))
    {
        return Ok(());
    }
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", "", url]).status()
    } else {
        Command::new("xdg-open").arg(url).status()
    }
    .context("cannot launch the default browser")?;
    if !status.success() {
        bail!("default browser launcher exited with {status}");
    }
    Ok(())
}

pub(crate) fn open_application_browser(url: &str) -> Result<()> {
    open_default_browser(url)
}

async fn handle_run_web(
    source: PathBuf,
    lock: PathBuf,
    package_lock: Option<PathBuf>,
) -> Result<()> {
    let host = ensure_ui_host(UiTarget::Web).await?;
    let asset_root = source
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let options = UiCompileOptions {
        target: UiTarget::Web,
        lock_path: lock.clone(),
        asset_root: Some(asset_root.clone()),
    };
    let mut session = match package_lock {
        Some(package_lock) => UiDevelopmentSession::new_with_package_lock(options, package_lock),
        None => UiDevelopmentSession::new(options),
    };
    let initial = apply_path(&mut session, &source)?;
    report_reload(&initial);
    let compilation = session.last_good().ok_or_else(|| {
        anyhow::anyhow!("web UI was not launched because the initial source did not compile")
    })?;
    let initial_extensions = assemble_verified_extensions(&source, compilation, UiTarget::Web)?;
    let initial_runtime_config =
        build_application_runtime_config_value(compilation, &lock, &initial_extensions)?;
    let (reload, _) = broadcast::channel(16);
    let (diagnostic_tx, mut diagnostic_rx) = mpsc::unbounded_channel();
    let state = WebHostState {
        app: Arc::new(RwLock::new(web_model_with_runtime_config(
            compilation,
            true,
            initial_runtime_config,
        )?)),
        files: Arc::new(extract_web_host(&host)?),
        extension_files: Arc::new(RwLock::new(initial_extensions.files)),
        asset_root: asset_root.clone(),
        declared_assets: Arc::new(RwLock::new(web_declared_assets(compilation))),
        reload,
        diagnostics: diagnostic_tx,
    };
    let router = Router::new()
        .route("/", get(web_index))
        .route("/__axiom/app.json", get(web_app))
        .route("/__axiom/events", get(web_events))
        .route("/__axiom/diagnostics", post(web_diagnostic))
        .route("/__axiom/assets/*path", get(web_asset))
        .route("/runtime/extensions/*path", get(web_extension))
        .route("/*path", get(web_static))
        .with_state(state.clone());
    let requested_port = std::env::var("AXIOM_UI_WEB_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", requested_port)).await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("Web UI server stopped: {error}");
        }
    });
    let url = format!("http://{address}/");
    open_default_browser(&url)?;
    println!("Opened Axiom UI at {url}");

    let source = canonical_or_original(&source);
    let lock = canonical_or_original(&lock);
    let extension_workflow = canonical_or_original(&asset_root.join("AxiomExtensions.toml"));
    let mut last_observed_graph_revision = initial.graph_revision;
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |event| {
            let _ = event_tx.send(event);
        },
        notify::Config::default(),
    )?;
    watch_ui_source_root(&mut watcher, &asset_root)?;
    if !same_path(nonempty_parent(&lock), &asset_root) {
        watch_parent(&mut watcher, &lock)?;
    }
    let dependency_paths = if crate::commands::run::is_managed_development_lock(&lock) {
        crate::commands::run::frontend_dependency_paths(&source)?
    } else {
        Vec::new()
    };
    watch_frontend_dependency_paths(&mut watcher, &dependency_paths, &asset_root)?;
    println!(
        "Watching {} for web changes. Press Ctrl-C to stop.",
        source.display()
    );
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            Some(diagnostic) = diagnostic_rx.recv() => {
                if ui_debug_enabled() { eprintln!("Web UI {} {} graph {}: {}", diagnostic.severity, diagnostic.code, diagnostic.graph_revision, diagnostic.message); }
                else { eprintln!("UI {} {}: {}", diagnostic.severity, diagnostic.code, diagnostic.message); }
            }
            event = event_rx.recv() => match event {
                Some(Ok(event)) if event.paths.iter().any(|path| same_path(path, &source) || same_path(path, &lock) || same_path(path, &extension_workflow) || frontend_dependency_changed(path, &dependency_paths) || declared_asset_path(&session, &asset_root, path)) => {
                    let dependency_changed = event.paths.iter().any(|path| frontend_dependency_changed(path, &dependency_paths));
                    if dependency_changed {
                        if let Err(error) = crate::commands::run::prepare_frontend(&source, None, false).await {
                            eprintln!("Could not prepare the changed frontend dependency: {error}");
                            continue;
                        }
                    }
                    let extension_changed = dependency_changed || event.paths.iter().any(|path| same_path(path, &extension_workflow));
                    match apply_path(&mut session, &source) {
                        Ok(update) => {
                            let graph_changed = observe_graph_revision(&mut last_observed_graph_revision, &update.graph_revision);
                            if graph_changed {
                                report_reload(&update);
                            }
                            if !matches!(update.outcome, HotReloadOutcome::RejectedLastGood { .. }) {
                                if graph_changed || extension_changed {
                                    if let Some(compilation) = session.last_good() {
                                    match assemble_verified_extensions(&source, compilation, UiTarget::Web)
                                        .and_then(|assembly| {
                                            let config = build_application_runtime_config_value(compilation, &lock, &assembly)?;
                                            let app = web_model_with_runtime_config(compilation, true, config)?;
                                            Ok((assembly, app))
                                        }) {
                                        Ok((assembly, app)) => {
                                            *state.app.write().expect("web app lock poisoned") = app;
                                            *state.extension_files.write().expect("extension artifact lock poisoned") = assembly.files;
                                            *state.declared_assets.write().expect("asset lock poisoned") = web_declared_assets(compilation);
                                            let preserve_state = !extension_changed && matches!(update.outcome, HotReloadOutcome::AppliedStatePreserved);
                                            let _ = state.reload.send(serde_json::json!({"graphRevision": update.graph_revision, "preserveState": preserve_state}).to_string());
                                            println!("Updated browser.");
                                        }
                                        Err(error) => eprintln!("Verified extension update rejected; the prior last-good extension set remains active: {error}"),
                                    }
                                    }
                                }
                            }
                        }
                        Err(error) => eprintln!("Could not apply the UI edit: {error}"),
                    }
                }
                Some(Err(error)) => eprintln!("UI file watcher error: {error}"),
                Some(Ok(_)) => {}
                None => bail!("Acore UI file watcher stopped unexpectedly"),
            }
        }
    }
}

fn native_diagnostic_source(host: &UiHostInstallation) -> Result<NativeDiagnosticSource> {
    match host.target.as_str() {
        "ios" => {
            let device = select_ios_simulator()?;
            let container = ios_host_data_container(&device)?.ok_or_else(|| {
                anyhow::anyhow!("Axiom UI Host Simulator diagnostics container is unavailable")
            })?;
            Ok(NativeDiagnosticSource::Ios(
                PathBuf::from(container.trim())
                    .join("Library/Application Support/AxiomUIHost/axiom.app.diagnostic.json"),
            ))
        }
        "android" => Ok(NativeDiagnosticSource::Android(select_android_emulator()?)),
        other => bail!("AXIOM_UI_TARGET_UNSUPPORTED: no native diagnostics adapter for {other}"),
    }
}

fn read_native_diagnostics(source: &NativeDiagnosticSource) -> Vec<NativeUiDiagnostic> {
    let text = match source {
        NativeDiagnosticSource::Ios(path) => std::fs::read_to_string(path).ok(),
        NativeDiagnosticSource::Android(device) => {
            android_private_text(device, "axiom.app.diagnostic.json")
        }
    };
    let Some(text) = text else { return Vec::new() };
    let Ok(envelope) = serde_json::from_str::<NativeUiDiagnosticEnvelope>(&text) else {
        return Vec::new();
    };
    if envelope.format != "axiom-ui-host-diagnostics/v1" {
        return Vec::new();
    }
    envelope.diagnostics
}

fn report_native_diagnostic(diagnostic: &NativeUiDiagnostic) {
    if ui_debug_enabled() {
        eprintln!(
            "Native UI {} {} at delivery #{} graph {}: {}{}",
            diagnostic.severity,
            diagnostic.code,
            diagnostic.sequence,
            diagnostic.graph_revision,
            diagnostic.message,
            if diagnostic.suggestion.is_empty() {
                String::new()
            } else {
                format!(" Suggestion: {}", diagnostic.suggestion)
            }
        );
    } else {
        eprintln!(
            "UI {} {}: {}",
            diagnostic.severity, diagnostic.code, diagnostic.message
        );
    }
}

pub async fn handle_host_install(
    target: String,
    release_manifest: Option<PathBuf>,
    variant: Option<String>,
    host_root: Option<PathBuf>,
    non_interactive: bool,
) -> Result<()> {
    let target = parse_target(&target)?;
    install_ui_host(
        target,
        release_manifest,
        variant,
        host_root,
        non_interactive,
    )
    .await
}

pub async fn handle_host_status(target: String) -> Result<()> {
    let target = parse_target(&target)?;
    match read_ui_host(target)? {
        Some(host) => {
            println!(
                "UI Host for {} is set up at {}.",
                target.as_str(),
                host.host_root.display()
            );
            if let Some(artifact) = &host.artifact {
                println!(
                    "Installed release: {} ({})",
                    artifact.version, artifact.variant
                );
                if artifact.version == "0.0.0-validation" {
                    println!("Installed release is a legacy validation fixture. Run `axiom ui host install --target {}` to replace it with the latest verified release.", target.as_str());
                }
            } else {
                println!("Installed host: local development source");
            }
            if host.delivery_adapter_ready {
                println!("UI bundle delivery: ready (state-preserving patches are capability-negotiated; the host reports any state-reset fallback)");
            } else {
                println!(
                    "UI bundle delivery: unavailable — install Axiom UI Host {} or later",
                    host_protocol_minimum(target)
                );
            }
        }
        None => println!("UI Host for {} is not installed.", target.as_str()),
    }
    Ok(())
}

/// Recover only the host's disposable delivery control records. This never
/// deletes a verified archive, Acore source, the opaque compiler cache, or a
/// last-good bundle. It is the deterministic escape hatch for a stale host
/// acknowledgement after an interrupted development session.
pub async fn handle_host_recover(target: String) -> Result<()> {
    let target = parse_target(&target)?;
    let host = read_ui_host(target)?.ok_or_else(|| {
        anyhow::anyhow!("AXIOM_UI_HOST_MISSING: install the UI Host before running recovery")
    })?;
    if !host_supports_delivery(&host) {
        bail!("AXIOM_UI_HOST_UPGRADE_REQUIRED: install a current Axiom UI Host before recovery");
    }
    match target {
        UiTarget::Ios => recover_ios_host()?,
        UiTarget::Android => recover_android_host()?,
        UiTarget::Web => println!("Recovered the web UI Host. Restart `axiom run --target web` to create a fresh local browser session."),
    }
    Ok(())
}

fn recover_ios_host() -> Result<()> {
    let device = select_ios_simulator()?;
    let container = ios_host_data_container(&device)?.ok_or_else(|| anyhow::anyhow!(
        "AXIOM_UI_RECOVER_HOST_NOT_INSTALLED: install or run the Axiom UI Host on the selected Simulator first"
    ))?;
    let support = PathBuf::from(container.trim()).join("Library/Application Support/AxiomUIHost");
    for file in ["axiom.app.revision.json", "axiom.app.ack.json"] {
        let path = support.join(file);
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| {
                format!("cannot clear stale host control record {}", path.display())
            })?;
        }
    }
    let _ = Command::new("xcrun")
        .args(["simctl", "terminate", &device, IOS_HOST_BUNDLE_ID])
        .output()?;
    require_success(
        "Axiom UI Host recovery launch",
        &Command::new("xcrun")
            .args(["simctl", "launch", &device, IOS_HOST_BUNDLE_ID])
            .output()?,
    )?;
    println!(
        "Recovered the iOS UI Host: restarted the app and cleared only stale delivery control records. The next `axiom run` will publish the current last-good bundle."
    );
    Ok(())
}

pub async fn handle_inspect(
    view: UiInspectView,
    source: PathBuf,
    lock: PathBuf,
    target: String,
    max_symbols: usize,
) -> Result<()> {
    let result = compile_path(&source, lock, parse_target(&target)?)?;
    match view {
        UiInspectView::Ir => print_json(&result.ir),
        UiInspectView::Lowered => print_json(&result.virtual_build),
        UiInspectView::Facade => {
            let build = result
                .virtual_build
                .expect("valid compilation lowers virtually");
            let facade = build
                .files
                .get("virtual/axiom-facade.ts")
                .expect("lowerer always emits facade");
            print_json(facade)
        }
        UiInspectView::Context => {
            print_json(&compact_semantic_context(&result.context, max_symbols))
        }
        UiInspectView::Extensions => print_json(&extension_binding_inspection(&result)),
    }
    Ok(())
}

/// A compact, source-first inspection view for executable extension bindings.
/// It deliberately contains only authored spans, static import metadata, and
/// compiler-derived action identities. Runtime evidence is collected by
/// `axiom extensions run --audit-out` after package verification.
fn extension_binding_inspection(result: &UiCompilation) -> serde_json::Value {
    let Some(ir) = result.ir.as_ref() else {
        return serde_json::json!({
            "format": "axiom-ui-extension-bindings/v1",
            "status": "invalid-source",
        });
    };
    let mut invocations = Vec::new();
    for page in &ir.pages {
        for action in &page.actions {
            for step in &action.steps {
                let UiActionStep::ExtensionInvoke {
                    local_name,
                    alias,
                    export,
                    interface_sha256,
                    abi_symbol,
                    state_scope,
                    input,
                    span,
                } = step
                else {
                    continue;
                };
                invocations.push(serde_json::json!({
                    "page": page.name,
                    "pageSemanticId": page.semantic_id,
                    "action": action.name,
                    "actionSemanticId": action.semantic_id,
                    "localName": local_name,
                    "alias": alias,
                    "export": export,
                    "interfaceSha256": interface_sha256,
                    "abiSymbol": abi_symbol,
                    "stateScope": state_scope,
                    "inputExpression": input,
                    "sourceSpan": span,
                }));
            }
        }
    }
    serde_json::json!({
        "format": "axiom-ui-extension-bindings/v1",
        "status": "compiled",
        "module": ir.module,
        "target": ir.target,
        "graphRevision": result.virtual_build.as_ref().map(|build| &build.graph_revision),
        "imports": ir.extension_imports,
        "invocations": invocations,
    })
}

pub async fn handle_context(
    source: PathBuf,
    lock: PathBuf,
    target: String,
    max_symbols: usize,
) -> Result<()> {
    let result = compile_path(&source, lock, parse_target(&target)?)?;
    print_json(&compact_semantic_context(&result.context, max_symbols));
    Ok(())
}

pub async fn handle_ai_check(
    source: PathBuf,
    proposed: PathBuf,
    base: String,
    lock: PathBuf,
    target: String,
) -> Result<()> {
    let target = parse_target(&target)?;
    let source_text = std::fs::read_to_string(&source)?;
    let proposed_text = std::fs::read_to_string(&proposed)?;
    let mut session = UiDevelopmentSession::new(UiCompileOptions {
        target,
        lock_path: lock,
        asset_root: source.parent().map(Path::to_path_buf),
    });
    let initial = session.apply_source(&source_text);
    if !matches!(initial.outcome, HotReloadOutcome::InitialLoad) {
        bail!("AI validation requires a valid base UI source");
    }
    let validation = session.validate_safe_edit(&base, &proposed_text);
    print_json(&validation);
    match validation.status {
        SafeEditStatus::Accepted => Ok(()),
        SafeEditStatus::RejectedInvalid => bail!("AI proposal failed compiler postconditions"),
        SafeEditStatus::StaleRevision => bail!("AI proposal was based on a stale graph revision"),
    }
}

pub async fn handle_doctor(target: String, json: bool) -> Result<()> {
    let target = parse_target(&target)?;
    let host = read_ui_host(target)?;
    let host_installed = host.is_some();
    let runtime_info = host.as_ref().and_then(read_installed_ios_runtime_info);
    let (native_host, ready_for_native_run, remediation) = match host {
        None => (
            "missing",
            false,
            "Run `axiom ui host install --target <target>` to set up the UI Host.",
        ),
        Some(host) if !host_supports_delivery(&host) => (
            "installed-outdated",
            false,
            "UI Host is verified but predates the current delivery and rendering contract. Install the latest Axiom UI Host.",
        ),
        Some(_) if target == UiTarget::Web && host.as_ref().is_some_and(host_supports_delivery) => (
            "ready", true,
            "The verified browser host and embedded Axiom Runtime WASM are ready.",
        ),
        Some(_) if target == UiTarget::Android && host.as_ref().is_some_and(host_supports_delivery) => (
            "delivery-ready", false,
            "Android native bundle delivery is ready, but contract-runtime event-channel conformance is still pending.",
        ),
        Some(_) if runtime_info.as_ref().is_some_and(|info| info.abi == 1 && info.module == 1 && info.facade == 1) => ("ready", true, "UI Host is installed and its ABI-v1 contract bridge is ready."),
        Some(_) => ("installed", false, "UI Host is installed but lacks the ABI-v1 verified contract bridge. Install the latest Axiom UI Host."),
    };
    let report = UiDoctorReport {
        format: "axiom-ui-doctor/v1",
        target: target.as_str().to_string(),
        compiler: "ready",
        virtual_graph: "ready",
        native_host,
        ready_for_native_run,
        diagnostic_code: if ready_for_native_run {
            "AXIOM_UI_HOST_READY"
        } else if host_installed {
            "AXIOM_UI_HOST_UPGRADE_REQUIRED"
        } else {
            "AXIOM_UI_HOST_MISSING"
        },
        remediation,
        runtime_abi_version: runtime_info.as_ref().map(|info| info.abi),
        runtime_module_version: runtime_info.as_ref().map(|info| info.module),
        runtime_facade_protocol_version: runtime_info.as_ref().map(|info| info.facade),
    };
    if json {
        print_json(&report);
    } else {
        println!("Acore UI compiler: ready");
        println!("Virtual graph: ready (workspace source generation disabled)");
        if let Some(runtime_info) = runtime_info {
            println!(
                "Native runtime: ABI {}, module {}, facade protocol {}",
                runtime_info.abi, runtime_info.module, runtime_info.facade
            );
        }
        println!(
            "{}: {} — {}",
            target.as_str(),
            report.diagnostic_code,
            report.remediation
        );
    }
    Ok(())
}

pub async fn handle_capabilities(
    target: String,
    kind: Option<UiCapabilityKind>,
    query: Option<String>,
    json: bool,
) -> Result<()> {
    let target = parse_target(&target)?;
    let registry = phase2_capability_registry();
    let kind = kind.map(CapabilityKind::from);
    let query = query.map(|value| value.to_lowercase());
    let selected = registry
        .capabilities
        .iter()
        .filter(|capability| kind.is_none_or(|expected| capability.kind == expected))
        .filter(|capability| {
            query.as_ref().is_none_or(|needle| {
                capability.id.to_lowercase().contains(needle)
                    || capability.name.to_lowercase().contains(needle)
                    || capability.acore_name.to_lowercase().contains(needle)
            })
        })
        .collect::<Vec<_>>();

    let mut status_counts = BTreeMap::<&str, usize>::new();
    for capability in &registry.capabilities {
        let status = capability
            .acore_support(target)
            .map(|support| support.status)
            .unwrap_or(AcoreSupportStatus::Unsupported);
        *status_counts.entry(status.as_str()).or_default() += 1;
    }

    if json {
        let capabilities = selected
            .iter()
            .map(|capability| {
                let acore = capability
                    .acore_support(target)
                    .expect("registry validation requires every Acore target");
                let upstream = capability
                    .upstream_support(target)
                    .expect("registry validation requires every upstream target");
                serde_json::json!({
                    "id": capability.id,
                    "kind": capability.kind,
                    "name": capability.name,
                    "acoreName": capability.acore_name,
                    "batch": capability.batch,
                    "stability": capability.stability,
                    "acoreStatus": acore.status,
                    "upstreamStatus": upstream.status,
                    "upstreamSince": upstream.since,
                    "source": capability.source,
                })
            })
            .collect::<Vec<_>>();
        print_json(&serde_json::json!({
            "format": registry.format,
            "registryVersion": registry.registry_version,
            "registrySha256": PHASE2_CAPABILITY_REGISTRY_SHA256,
            "renderer": registry.renderer,
            "documentation": registry.documentation,
            "target": target.as_str(),
            "counts": registry.counts,
            "targetStatusCounts": status_counts,
            "capabilities": capabilities,
        }));
        return Ok(());
    }

    println!(
        "Acore frontend registry {} · {} entries · sha256:{}",
        registry.registry_version,
        registry.capabilities.len(),
        &PHASE2_CAPABILITY_REGISTRY_SHA256[..12]
    );
    println!("Pinned Lynx renderer: {}", registry.renderer.commit);
    println!(
        "{}: {}",
        target.as_str(),
        status_counts
            .iter()
            .map(|(status, count)| format!("{count} {status}"))
            .collect::<Vec<_>>()
            .join(" · ")
    );
    if kind.is_some() || query.is_some() {
        if selected.is_empty() {
            bail!("no Phase 2 capabilities match the requested filter");
        }
        for capability in selected {
            let acore = capability
                .acore_support(target)
                .expect("registry validation requires every Acore target");
            let upstream = capability
                .upstream_support(target)
                .expect("registry validation requires every upstream target");
            let since = upstream
                .since
                .as_deref()
                .map(|value| format!(" since {value}"))
                .unwrap_or_default();
            println!(
                "{} · {} · Acore {} · upstream {:?}{} · {}",
                capability.id,
                capability.batch,
                acore.status.as_str(),
                upstream.status,
                since,
                capability.source.url
            );
        }
    } else {
        println!(
            "Filter entries with `axiom ui capabilities --target {} --kind <kind>` or `--query <name>`. Use `--json` for tooling.",
            target.as_str()
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct InstalledIosRuntimeInfo {
    abi: u32,
    module: u32,
    facade: u32,
}

fn read_installed_ios_runtime_info(host: &UiHostInstallation) -> Option<InstalledIosRuntimeInfo> {
    let artifact = host.artifact.as_ref()?;
    if artifact.variant != "simulator" {
        return None;
    }
    let app = extract_ios_host_application(&host.host_root.join(&artifact.file), &artifact.version)
        .ok()?;
    let output = Command::new("plutil")
        .args([
            "-convert",
            "json",
            "-o",
            "-",
            app.join("Info.plist").to_string_lossy().as_ref(),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let plist: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    Some(InstalledIosRuntimeInfo {
        abi: plist["AxiomRuntimeABIVersion"].as_u64()? as u32,
        module: plist["AxiomRuntimeModuleVersion"].as_u64()? as u32,
        facade: plist["AxiomRuntimeFacadeProtocolVersion"].as_u64()? as u32,
    })
}

pub async fn handle_test(source: PathBuf, lock: PathBuf, target: String) -> Result<()> {
    let target = parse_target(&target)?;
    let source_text = std::fs::read_to_string(&source)?;
    let mut session = UiDevelopmentSession::new(UiCompileOptions {
        target,
        lock_path: lock,
        asset_root: source.parent().map(Path::to_path_buf),
    });
    let first = session.apply_source(&source_text);
    if !matches!(first.outcome, HotReloadOutcome::InitialLoad) {
        print_diagnostics(&source, &first.diagnostics);
        bail!("UI test setup failed");
    }
    let compatible = session.apply_source(&format!("{}\n", source_text));
    let invalid = session.apply_source(
        "module invalid.ui\npage Broken { view { List(items: [],) { Text(\"x\") } } }\n",
    );
    if !matches!(compatible.outcome, HotReloadOutcome::AppliedStatePreserved)
        || !matches!(invalid.outcome, HotReloadOutcome::RejectedLastGood { .. })
    {
        bail!("UI session behavior did not satisfy hot-reload safety checks");
    }
    println!("UI session checks passed: valid update preserved state; invalid update retained last-good graph.");
    Ok(())
}

/// Compile and assemble one deterministic application payload. `.axiomapp`
/// is the reviewed Axiom application boundary consumed by platform packaging;
/// it is not a renamed host fixture or an editable generated project.
pub async fn handle_build(
    source: PathBuf,
    lock: PathBuf,
    target: String,
    out: Option<PathBuf>,
    development: bool,
) -> Result<()> {
    let target = parse_target(&target)?;
    let result = compile_path(&source, lock.clone(), target)?;
    let host = read_ui_host(target)?
        .filter(host_supports_delivery)
        .ok_or_else(|| anyhow::anyhow!(
            "AXIOM_UI_HOST_MISSING: install the current verified {} UI Host before building this application",
            target.as_str()
        ))?;
    let output = out.unwrap_or_else(|| default_application_artifact_path(&result, target));
    assemble_application_artifact(&source, &lock, target, &result, &host, &output, development)?;
    println!(
        "Built {} application artifact: {} (graph {}).",
        target.as_str(),
        output.display(),
        result.context.graph_revision
    );
    Ok(())
}

fn default_application_artifact_path(compilation: &UiCompilation, target: UiTarget) -> PathBuf {
    let module = compilation
        .ir
        .as_ref()
        .map(|ir| sanitize_artifact_name(&ir.module))
        .unwrap_or_else(|| "app".to_string());
    PathBuf::from("dist").join(format!("{module}-{}.axiomapp", target.as_str()))
}

fn sanitize_artifact_name(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let value = value.trim_matches('-');
    if value.is_empty() {
        "app".to_string()
    } else {
        value.to_string()
    }
}

fn deterministic_json(value: &serde_json::Value) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn add_artifact_file(
    files: &mut BTreeMap<String, Vec<u8>>,
    path: &str,
    bytes: Vec<u8>,
) -> Result<()> {
    let relative = Path::new(path);
    if relative.is_absolute()
        || !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        bail!("AXIOM_UI_ARTIFACT_PATH: unsafe application artifact path `{path}`");
    }
    if files.insert(path.to_string(), bytes).is_some() {
        bail!("AXIOM_UI_ARTIFACT_PATH: duplicate application artifact path `{path}`");
    }
    Ok(())
}

fn assemble_application_artifact(
    source: &Path,
    lock_path: &Path,
    target: UiTarget,
    compilation: &UiCompilation,
    host: &UiHostInstallation,
    output: &Path,
    development: bool,
) -> Result<()> {
    let ir = compilation
        .ir
        .as_ref()
        .context("valid application build has no UI IR")?;
    let build = compilation
        .virtual_build
        .as_ref()
        .context("valid application build has no virtual Lynx input")?;
    let extension_assembly = assemble_verified_extensions(source, compilation, target)?;
    let runtime_config =
        build_application_runtime_config_value(compilation, lock_path, &extension_assembly)?;
    let contracts = runtime_config["contracts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let unsigned = contracts
        .iter()
        .filter_map(|contract| {
            (!contract["verified"].as_bool().unwrap_or(false)).then(|| {
                contract["localName"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string()
            })
        })
        .collect::<Vec<_>>();
    if !unsigned.is_empty() && !development {
        bail!(
            "AXIOM_UI_UNSIGNED_RELEASE_INPUT: unsigned local contract(s) {} cannot enter a release artifact; publish them or rebuild with --development",
            unsigned.join(", ")
        );
    }
    let host_artifact = host
        .artifact
        .as_ref()
        .context("verified UI Host record has no artifact identity")?;
    let source_bytes = std::fs::read(source)
        .with_context(|| format!("cannot read application source {}", source.display()))?;
    let lock_sha256 = if ir.imports.is_empty() {
        None
    } else {
        Some(
            sha256_file(lock_path)
                .with_context(|| format!("cannot hash UI lock {}", lock_path.display()))?,
        )
    };
    let mut files = BTreeMap::new();
    add_artifact_file(
        &mut files,
        "runtime/config.json",
        deterministic_json(&runtime_config)?,
    )?;
    for (path, bytes) in &extension_assembly.files {
        add_artifact_file(&mut files, path, bytes.clone())?;
    }
    add_artifact_file(
        &mut files,
        "runtime/extensions/extension-lock.json",
        deterministic_json(&extension_assembly.lock)?,
    )?;
    let capabilities = serde_json::json!({
        "format": "axiom-app-capabilities/v1",
        "networkOrigins": contracts.iter().filter_map(|value| value["baseUrl"].as_str()).collect::<Vec<_>>(),
        "contracts": contracts.iter().map(|value| serde_json::json!({
            "name": value["localName"], "verified": value["verified"], "operations": value["operations"]
        })).collect::<Vec<_>>(),
        "extensions": extension_assembly.runtime_entries.iter().map(|value| serde_json::json!({
            "name": value["extension"], "exports": value["exports"], "target": value["target"],
            "verified": value["verified"], "moduleSha256": value["moduleSha256"],
            "effectiveAuthority": value["effectiveAuthority"],
        })).collect::<Vec<_>>(),
        "frontendProfile": {
            "registryVersion": PHASE2_CAPABILITY_REGISTRY_VERSION,
            "registrySha256": PHASE2_CAPABILITY_REGISTRY_SHA256,
            "lynxEngineCommit": PHASE2_PINNED_LYNX_COMMIT,
            "componentPackage": {
                "package": PHASE2_LYNX_UI_PACKAGE,
                "version": PHASE2_LYNX_UI_VERSION,
                "sourceCommit": PHASE2_LYNX_UI_SOURCE_COMMIT,
                "npmIntegrity": PHASE2_LYNX_UI_NPM_INTEGRITY,
            }
        }
    });
    add_artifact_file(
        &mut files,
        "reports/capabilities.json",
        deterministic_json(&capabilities)?,
    )?;
    let frontend_support = serde_json::json!({
        "format": "axiom-acore-frontend-support/v1",
        "registryVersion": PHASE2_CAPABILITY_REGISTRY_VERSION,
        "registrySha256": PHASE2_CAPABILITY_REGISTRY_SHA256,
        "target": target.as_str(),
        "capabilities": phase2_capability_registry().capabilities.iter().map(|capability| {
            let support = capability.acore_support(target)
                .expect("validated Phase 2 registry contains each application target");
            serde_json::json!({
                "id": capability.id,
                "kind": capability.kind,
                "name": capability.name,
                "acoreName": capability.acore_name,
                "status": support.status,
            })
        }).collect::<Vec<_>>()
    });
    add_artifact_file(
        &mut files,
        "reports/frontend-support.json",
        deterministic_json(&frontend_support)?,
    )?;

    let asset_root = source
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for asset in &ir.assets {
        let path = Path::new(&asset.path);
        if path.is_absolute()
            || !path
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            bail!(
                "AXIOM_UI_ASSET_PATH: unsafe declared asset path {}",
                asset.path
            );
        }
        let bytes = std::fs::read(asset_root.join(path))?;
        if sha256_bytes(&bytes) != asset.sha256 {
            bail!(
                "AXIOM_UI_ASSET_CHANGED: declared asset {} changed after compilation",
                asset.path
            );
        }
        add_artifact_file(&mut files, &format!("assets/{}", asset.path), bytes)?;
    }

    if target == UiTarget::Web {
        for (name, bytes) in extract_web_host(host)? {
            add_artifact_file(&mut files, &name, bytes)?;
        }
        add_artifact_file(
            &mut files,
            "__axiom/app.json",
            web_model_with_runtime_config(compilation, false, runtime_config.clone())?,
        )?;
        for asset in &ir.assets {
            let bytes = std::fs::read(asset_root.join(&asset.path))?;
            add_artifact_file(&mut files, &format!("__axiom/assets/{}", asset.path), bytes)?;
        }
    } else {
        let host_path = host.host_root.join(&host_artifact.file);
        let host_bytes = std::fs::read(&host_path).with_context(|| {
            format!(
                "cannot read verified UI Host artifact {}",
                host_path.display()
            )
        })?;
        if sha256_bytes(&host_bytes) != host_artifact.sha256 {
            bail!("AXIOM_UI_HOST_TAMPERED: installed {} host artifact no longer matches its verified digest", target.as_str());
        }
        add_artifact_file(
            &mut files,
            if target == UiTarget::Ios {
                "platform/AxiomUIHost.app.zip"
            } else {
                "platform/AxiomUIHost.apk"
            },
            host_bytes,
        )?;
        let runtime_config_source = runtime_config_source(&runtime_config)?;
        let bundle =
            compile_virtual_lynx_bundle(build, &runtime_config_source, asset_root, target)?;
        add_artifact_file(
            &mut files,
            "payload/main.lynx.bundle",
            std::fs::read(bundle)?,
        )?;
    }

    let manifest = serde_json::json!({
        "format": "axiom-application/v1",
        "applicationId": ir.module,
        "target": target.as_str(),
        "mode": if development { "development" } else { "release" },
        "graphRevision": compilation.context.graph_revision,
        "entry": if target == UiTarget::Web { "index.html" } else { "payload/main.lynx.bundle" },
        "platformHost": if target == UiTarget::Web { serde_json::Value::Null } else if target == UiTarget::Ios { serde_json::json!("platform/AxiomUIHost.app.zip") } else { serde_json::json!("platform/AxiomUIHost.apk") },
        "runtimeAbiVersion": 1,
        "runtimeFacadeProtocolVersion": 1,
        "uiIrFormat": ir.format,
        "host": { "version": host_artifact.version, "variant": host_artifact.variant, "sha256": host_artifact.sha256 },
        "unsignedContracts": unsigned,
        "extensionLock": "runtime/extensions/extension-lock.json",
        "extensions": extension_assembly.runtime_entries.clone(),
    });
    add_artifact_file(&mut files, "manifest.json", deterministic_json(&manifest)?)?;
    let provenance = serde_json::json!({
        "format": "axiom-application-provenance/v1",
        "sourceSha256": sha256_bytes(&source_bytes),
        "lockSha256": lock_sha256,
        "graphRevision": compilation.context.graph_revision,
        "cliVersion": env!("CARGO_PKG_VERSION"),
        "lynxEngineCommit": LYNX_ENGINE_COMMIT,
        "frontendProfile": {
            "registryVersion": PHASE2_CAPABILITY_REGISTRY_VERSION,
            "registrySha256": PHASE2_CAPABILITY_REGISTRY_SHA256,
            "lynxEngineCommit": PHASE2_PINNED_LYNX_COMMIT,
            "componentPackage": {
                "package": PHASE2_LYNX_UI_PACKAGE,
                "version": PHASE2_LYNX_UI_VERSION,
                "sourceCommit": PHASE2_LYNX_UI_SOURCE_COMMIT,
                "npmIntegrity": PHASE2_LYNX_UI_NPM_INTEGRITY,
            }
        },
        "hostArtifactSha256": host_artifact.sha256,
        "extensionLockSha256": sha256_bytes(&deterministic_json(&extension_assembly.lock)?),
    });
    add_artifact_file(
        &mut files,
        "provenance.json",
        deterministic_json(&provenance)?,
    )?;
    let checksums = files
        .iter()
        .map(|(path, bytes)| format!("{}  {}", sha256_bytes(bytes), path))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    add_artifact_file(&mut files, "checksums.sha256", checksums.into_bytes())?;
    write_deterministic_zip(output, &files)
}

fn write_deterministic_zip(output: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = output.with_extension("axiomapp.tmp");
    let file = std::fs::File::create(&temporary)?;
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for (path, bytes) in files {
        archive.start_file(path, options)?;
        archive.write_all(bytes)?;
    }
    archive.finish()?.sync_all()?;
    std::fs::rename(&temporary, output)?;
    Ok(())
}

fn compile_path(source: &Path, lock: PathBuf, target: UiTarget) -> Result<UiCompilation> {
    let text = read_ui_source(source)?;
    let result = compile_ui_source(
        &text,
        &UiCompileOptions {
            target,
            lock_path: lock,
            asset_root: source.parent().map(Path::to_path_buf),
        },
    );
    if !result.is_valid() {
        print_diagnostics(source, &result.diagnostics);
        bail!(
            "Acore UI check failed with {} diagnostic(s)",
            result.diagnostics.len()
        );
    }
    print_diagnostics(
        source,
        &result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == axiom_ui::UiDiagnosticSeverity::Warning)
            .cloned()
            .collect::<Vec<_>>(),
    );
    Ok(result)
}

fn apply_path(session: &mut UiDevelopmentSession, source: &Path) -> Result<HotReloadEvent> {
    let text = read_ui_source(source)?;
    Ok(session.apply_source(&text))
}

fn read_ui_source(source: &Path) -> Result<String> {
    match std::fs::read_to_string(source) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = source
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let available = std::fs::read_dir(parent)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "acore")
                })
                .filter_map(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                })
                .collect::<Vec<_>>();
            if available.is_empty() {
                bail!("UI source {} does not exist", source.display());
            }
            let mut available = available;
            available.sort();
            bail!(
                "UI source {} does not exist. Available Acore file(s): {}",
                source.display(),
                available.join(", ")
            );
        }
        Err(error) => {
            Err(error).with_context(|| format!("cannot read UI source {}", source.display()))
        }
    }
}

fn parse_target(target: &str) -> Result<UiTarget> {
    match target {
        "ios" => Ok(UiTarget::Ios),
        "android" => Ok(UiTarget::Android),
        "web" => Ok(UiTarget::Web),
        _ => bail!("UI target must be `ios`, `android`, or `web`"),
    }
}

fn print_diagnostics(source: &Path, diagnostics: &[axiom_ui::UiDiagnostic]) {
    for diagnostic in diagnostics {
        println!(
            "{}:{}:{}: {} {}",
            source.display(),
            diagnostic.span.start,
            diagnostic.code,
            match diagnostic.severity {
                axiom_ui::UiDiagnosticSeverity::Error => "error",
                axiom_ui::UiDiagnosticSeverity::Warning => "warning",
            },
            diagnostic.message
        );
    }
}

fn report_reload(event: &HotReloadEvent) {
    let outcome = match &event.outcome {
        HotReloadOutcome::InitialLoad => "initial virtual graph ready".to_string(),
        // This is a graph-level candidate for state preservation. The native
        // delivery policy currently converts it to an explicit reset because
        // the pinned renderer's patch API has not passed native conformance.
        HotReloadOutcome::AppliedStatePreserved => {
            "applied; graph is patch-compatible (web preserves state; native targets explicitly reset)".to_string()
        }
        HotReloadOutcome::AppliedStateReset { reason } => format!("applied; state reset: {reason}"),
        HotReloadOutcome::FullReloadRequired { reason } => {
            format!("full reload required: {reason}")
        }
        HotReloadOutcome::RejectedLastGood { last_good_revision } => format!(
            "rejected; last-good graph retained ({})",
            last_good_revision.as_deref().unwrap_or("none")
        ),
    };
    if ui_debug_enabled() {
        println!(
            "UI reload #{} {} · {}",
            event.sequence, event.graph_revision, outcome
        );
    } else {
        let label = match event.outcome {
            HotReloadOutcome::InitialLoad => "UI compiled",
            HotReloadOutcome::RejectedLastGood { .. } => {
                "UI edit rejected; showing last good version"
            }
            _ => "UI updated",
        };
        println!("{label} (#{}).", event.sequence);
    }
    for diagnostic in &event.diagnostics {
        println!("{} {}", diagnostic.code, diagnostic.message);
    }
}

/// Filesystem notifications are advisory. A graph revision is authoritative,
/// so duplicate editor events must not result in duplicate native delivery.
fn observe_graph_revision(last_observed: &mut String, next: &str) -> bool {
    if last_observed == next {
        return false;
    }
    *last_observed = next.to_owned();
    true
}

fn print_json<T: Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("serializable CLI response")
    );
}

fn write_new(path: &Path, contents: &str) -> Result<()> {
    if path.exists() {
        bail!("refusing to overwrite {}", path.display());
    }
    std::fs::write(path, contents).with_context(|| format!("cannot create {}", path.display()))
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn watch_parent(watcher: &mut RecommendedWatcher, path: &Path) -> Result<()> {
    let parent = nonempty_parent(path);
    watcher.watch(parent, RecursiveMode::NonRecursive)?;
    Ok(())
}

fn watch_frontend_dependency_paths(
    watcher: &mut RecommendedWatcher,
    dependencies: &[PathBuf],
    asset_root: &Path,
) -> Result<()> {
    let asset_root = canonical_or_original(asset_root);
    let mut watched = HashSet::new();
    for dependency in dependencies {
        let dependency = canonical_or_original(dependency);
        if dependency.starts_with(&asset_root) {
            continue;
        }
        let (path, mode) = if dependency.is_dir() {
            (dependency, RecursiveMode::Recursive)
        } else {
            (
                nonempty_parent(&dependency).to_path_buf(),
                RecursiveMode::NonRecursive,
            )
        };
        if watched.insert(path.clone()) {
            watcher.watch(&path, mode)?;
        }
    }
    Ok(())
}

fn frontend_dependency_changed(path: &Path, dependencies: &[PathBuf]) -> bool {
    dependencies.iter().any(|dependency| {
        same_path(path, dependency) || (dependency.is_dir() && path.starts_with(dependency))
    })
}

fn nonempty_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn watch_ui_source_root(watcher: &mut RecommendedWatcher, root: &Path) -> Result<()> {
    watcher.watch(root, RecursiveMode::Recursive)?;
    Ok(())
}

fn declared_asset_path(session: &UiDevelopmentSession, asset_root: &Path, path: &Path) -> bool {
    let path = canonical_or_original(path);
    let root = canonical_or_original(asset_root);
    let Ok(relative) = path.strip_prefix(&root) else {
        return false;
    };
    let Some(ir) = session
        .last_good()
        .and_then(|compilation| compilation.ir.as_ref())
    else {
        return false;
    };
    ir.assets
        .iter()
        .any(|asset| Path::new(&asset.path) == relative)
}

fn same_path(left: &Path, right: &Path) -> bool {
    canonical_or_original(left) == canonical_or_original(right)
}

fn version_at_least(actual: &str, minimum: &str) -> bool {
    fn parts(value: &str) -> Option<[u64; 3]> {
        let value = value.trim_start_matches('v');
        let mut parts = value.split('.').map(|part| part.parse::<u64>().ok());
        Some([
            parts.next()??,
            parts.next().unwrap_or(Some(0))?,
            parts.next().unwrap_or(Some(0))?,
        ])
    }
    match (parts(actual), parts(minimum)) {
        (Some(actual), Some(minimum)) => actual >= minimum,
        _ => false,
    }
}

fn host_protocol_minimum(target: UiTarget) -> &'static str {
    match target {
        UiTarget::Ios => IOS_HOST_PROTOCOL_MINIMUM,
        UiTarget::Android => ANDROID_HOST_PROTOCOL_MINIMUM,
        UiTarget::Web => WEB_HOST_PROTOCOL_MINIMUM,
    }
}

fn host_supports_delivery(host: &UiHostInstallation) -> bool {
    host.delivery_adapter_ready
        && host
            .artifact
            .as_ref()
            .is_some_and(|artifact| match host.target.as_str() {
                "ios" => {
                    artifact.variant == "simulator"
                        && version_at_least(&artifact.version, IOS_HOST_PROTOCOL_MINIMUM)
                }
                "android" => {
                    artifact.variant == "emulator"
                        && version_at_least(&artifact.version, ANDROID_HOST_PROTOCOL_MINIMUM)
                }
                "web" => {
                    artifact.variant == "browser"
                        && version_at_least(&artifact.version, WEB_HOST_PROTOCOL_MINIMUM)
                }
                _ => false,
            })
}

fn deliver_last_good(
    session: &UiDevelopmentSession,
    host: &UiHostInstallation,
    event: &HotReloadEvent,
    source: &Path,
    lock_path: &Path,
    asset_root: &Path,
) -> Result<()> {
    if let HotReloadOutcome::RejectedLastGood { last_good_revision } = &event.outcome {
        if last_good_revision.is_some() {
            println!(
                "Native UI retained the prior last-good bundle; the invalid edit was not delivered."
            );
        } else {
            println!(
                "Native UI was not launched because the initial source did not compile. Fix the diagnostics above and save to retry."
            );
        }
        return Ok(());
    }
    if !host_supports_delivery(host) {
        bail!(
            "AXIOM_UI_HOST_UPGRADE_REQUIRED: install a current Axiom UI Host for native delivery"
        );
    }
    let compilation = session.last_good().ok_or_else(|| {
        anyhow::anyhow!("no valid virtual UI build is available for native delivery")
    })?;
    let build = compilation.virtual_build.as_ref().ok_or_else(|| {
        anyhow::anyhow!("the last-good UI compilation has no virtual ReactLynx build")
    })?;
    let target = compilation
        .ir
        .as_ref()
        .expect("valid native delivery has UI IR")
        .target;
    let extension_assembly = assemble_verified_extensions(source, compilation, target)?;
    let runtime_config_value =
        build_application_runtime_config_value(compilation, lock_path, &extension_assembly)?;
    let runtime_config_fingerprint = sha256_bytes(&deterministic_json(&runtime_config_value)?);
    let runtime_config = runtime_config_source(&runtime_config_value)?;
    let bundle = compile_virtual_lynx_bundle(build, &runtime_config, asset_root, target)?;
    let acknowledgement = match host.target.as_str() {
        "ios" => deliver_ios_simulator_bundle(
            host,
            &bundle,
            event,
            matches!(event.outcome, HotReloadOutcome::InitialLoad),
            &runtime_config_fingerprint,
            &extension_assembly.files,
        )?,
        "android" => deliver_android_emulator_bundle(
            host,
            &bundle,
            event,
            matches!(event.outcome, HotReloadOutcome::InitialLoad),
            &runtime_config_fingerprint,
            &android_loopback_base_urls(&runtime_config_value),
            &extension_assembly.files,
        )?,
        other => bail!("AXIOM_UI_TARGET_UNSUPPORTED: no native delivery adapter for {other}"),
    };
    let target_name = if host.target == "android" {
        "Android Emulator"
    } else {
        "iOS Simulator"
    };
    match acknowledgement {
        IosDeliveryAcknowledgement::AppliedStatePreserved if ui_debug_enabled() => println!(
            "Native UI delivery #{} acknowledged by the {}. The running host stayed open and preserved compatible page state.", event.sequence, target_name
        ),
        IosDeliveryAcknowledgement::AppliedStateReset { ref reason } if ui_debug_enabled() => println!(
            "Native UI delivery #{} acknowledged by the {}. The running host stayed open; page state reset: {}.", event.sequence, target_name, reason
        ),
        IosDeliveryAcknowledgement::AppliedStatePreserved => {
            println!("Updated {target_name}; page state preserved.")
        }
        IosDeliveryAcknowledgement::AppliedStateReset { .. } => {
            println!("Updated {target_name}.")
        }
    }
    Ok(())
}

/// Run a verified target member directly from an `.axiomapp`. The archive is
/// never expanded into the developer workspace. Only the two files required
/// by native platform tools are materialized in a content-addressed opaque
/// cache because `simctl install` and `adb install` require filesystem paths.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_packaged_native_application(
    target: UiTarget,
    archive_sha256: &str,
    application_id: &str,
    graph_revision: &str,
    host_version: &str,
    host_variant: &str,
    host_sha256: &str,
    files: &HashMap<String, Vec<u8>>,
) -> Result<()> {
    if !matches!(target, UiTarget::Ios | UiTarget::Android) {
        bail!("AXIOM_APP_TARGET: packaged native runner requires ios or android");
    }
    let host_entry = if target == UiTarget::Ios {
        "platform/AxiomUIHost.app.zip"
    } else {
        "platform/AxiomUIHost.apk"
    };
    let host_bytes = files
        .get(host_entry)
        .with_context(|| format!("AXIOM_APP_CONTENT: missing `{host_entry}`"))?;
    if sha256_bytes(host_bytes) != host_sha256 {
        bail!("AXIOM_APP_HOST_TAMPERED: embedded host does not match manifest identity");
    }
    let bundle_bytes = files
        .get("payload/main.lynx.bundle")
        .context("AXIOM_APP_CONTENT: native artifact has no payload/main.lynx.bundle")?;
    let cache = application_cache_root()?
        .join("runtime")
        .join(archive_sha256)
        .join(target.as_str());
    std::fs::create_dir_all(&cache)?;
    let host_file_name = if target == UiTarget::Ios {
        "AxiomUIHost.app.zip"
    } else {
        "AxiomUIHost.apk"
    };
    let host_path = cache.join(host_file_name);
    let bundle_path = cache.join("main.lynx.bundle");
    std::fs::write(&host_path, host_bytes)?;
    std::fs::write(&bundle_path, bundle_bytes)?;
    let extension_files = files
        .iter()
        .filter(|(path, _)| path.starts_with("runtime/extensions/"))
        .map(|(path, bytes)| (path.clone(), bytes.clone()))
        .collect::<BTreeMap<_, _>>();
    let host = UiHostInstallation {
        format: "axiom-ui-host/v1".into(),
        target: target.as_str().into(),
        host_root: cache,
        artifact: Some(UiHostArtifact {
            version: host_version.into(),
            variant: host_variant.into(),
            file: host_file_name.into(),
            sha256: host_sha256.into(),
        }),
        delivery_adapter_ready: true,
    };
    if !host_supports_delivery(&host) {
        bail!(
            "AXIOM_APP_HOST_INCOMPATIBLE: packaged {} host {} ({}) does not satisfy protocol minimum {}",
            target.as_str(),
            host_version,
            host_variant,
            host_protocol_minimum(target)
        );
    }
    let event = HotReloadEvent {
        sequence: 1,
        graph_revision: graph_revision.into(),
        outcome: HotReloadOutcome::InitialLoad,
        changed_semantic_ids: Vec::new(),
        diagnostics: Vec::new(),
    };
    let acknowledgement = if target == UiTarget::Ios {
        let runtime_config: serde_json::Value = serde_json::from_slice(
            files
                .get("runtime/config.json")
                .context("AXIOM_APP_CONTENT: missing runtime/config.json")?,
        )?;
        let runtime_config_fingerprint = sha256_bytes(&deterministic_json(&runtime_config)?);
        deliver_ios_simulator_bundle(
            &host,
            &bundle_path,
            &event,
            true,
            &runtime_config_fingerprint,
            &extension_files,
        )?
    } else {
        let runtime_config: serde_json::Value = serde_json::from_slice(
            files
                .get("runtime/config.json")
                .context("AXIOM_APP_CONTENT: missing runtime/config.json")?,
        )?;
        let base_urls = runtime_config["contracts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|contract| contract["baseUrl"].as_str().map(str::to_string))
            .collect::<Vec<_>>();
        let runtime_config_fingerprint = sha256_bytes(&deterministic_json(&runtime_config)?);
        deliver_android_emulator_bundle(
            &host,
            &bundle_path,
            &event,
            true,
            &runtime_config_fingerprint,
            &base_urls,
            &extension_files,
        )?
    };
    let reset = matches!(
        acknowledgement,
        IosDeliveryAcknowledgement::AppliedStateReset { .. }
    );
    println!(
        "Running packaged application `{application_id}` on {}{}.",
        target.as_str(),
        if reset { " (initial state loaded)" } else { "" }
    );
    Ok(())
}

fn axiom_ui_cache_base() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("AXIOM_UI_CACHE_ROOT") {
        let root = PathBuf::from(root);
        if !root.is_absolute() {
            bail!("AXIOM_UI_CACHE_ROOT must be an absolute path");
        }
        return Ok(root);
    }
    Ok(dirs::cache_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine local cache directory"))?
        .join("axiom"))
}

pub(crate) fn application_cache_root() -> Result<PathBuf> {
    Ok(axiom_ui_cache_base()?.join("applications"))
}

fn ui_cache_root() -> Result<PathBuf> {
    Ok(axiom_ui_cache_base()?.join("ui"))
}

/// Turn selected lock entries into compiler-owned runtime configuration.
/// Every local artifact is pinned by hash. Cloud-released artifacts additionally
/// carry and verify their signature/public key; unsigned local artifacts remain
/// available for development and are visibly marked as unverified.
fn build_verified_runtime_config_value(
    compilation: &UiCompilation,
    lock_path: &Path,
) -> Result<serde_json::Value> {
    let ir = compilation.ir.as_ref().ok_or_else(|| {
        anyhow::anyhow!("AXIOM_UI_RUNTIME_IR: valid native delivery requires compiled UI IR")
    })?;
    let mut contracts = Vec::new();
    if !ir.imports.is_empty() {
        let lock = read_lock(lock_path)?;
        let root = lock_path.parent().unwrap_or_else(|| Path::new("."));
        for import in &ir.imports {
            let locked = lock.contracts.get(&import.alias).with_context(|| {
                format!(
                    "AXIOM_UI_RUNTIME_LOCK: imported contract `{}` is not present in {}",
                    import.alias,
                    lock_path.display()
                )
            })?;
            if locked.min_runtime_version > 1 {
                bail!("AXIOM_UI_RUNTIME_ABI: contract `{}` requires runtime ABI {}, but this host provides ABI 1", import.alias, locked.min_runtime_version);
            }
            let artifact_path = locked_ui_input(root, &locked.artifact, "artifact")?;
            let artifact = std::fs::read(&artifact_path).with_context(|| {
                format!(
                    "AXIOM_UI_RUNTIME_LOCK: read locked artifact {}",
                    artifact_path.display()
                )
            })?;
            if sha256_bytes(&artifact) != locked.artifact_sha256 {
                bail!("AXIOM_UI_RUNTIME_LOCK: locked artifact digest mismatch for contract `{}`; resolve and review the lock again", import.alias);
            }
            let (signature, public_key, verified) = match (
                &locked.signature,
                &locked.public_key,
                &locked.signature_sha256,
                &locked.public_key_sha256,
            ) {
                (Some(signature), Some(public_key), Some(signature_hash), Some(public_key_hash)) => {
                    if sha256_bytes(signature.as_bytes()) != *signature_hash
                        || sha256_bytes(public_key.as_bytes()) != *public_key_hash
                    {
                        bail!("AXIOM_UI_RUNTIME_LOCK: signed proof digest mismatch for contract `{}`; resolve and review the lock again", import.alias);
                    }
                    verify_locked_artifact_bytes(&artifact, &locked.artifact_sha256, &signature, &public_key)
                        .with_context(|| format!("AXIOM_UI_RUNTIME_VERIFY: verify contract `{}`", import.alias))?;
                    (signature.clone(), public_key.clone(), true)
                }
                (None, None, None, None) => (String::new(), String::new(), false),
                _ => bail!("AXIOM_UI_RUNTIME_LOCK: contract `{}` has incomplete signature metadata; resolve its manifest again", import.alias),
            };
            let operations = locked.operations.iter().map(|operation| serde_json::json!({
                "name": operation.name,
                "endpointId": operation.endpoint_id,
                "method": operation.method,
                "path": operation.path,
                "kind": match &operation.kind { UiOperationKind::Query => "query", UiOperationKind::Mutation => "mutation", UiOperationKind::Stream => "stream" },
                "cacheIdentity": operation.cache_identity,
                "invalidates": operation.invalidates,
            })).collect::<Vec<_>>();
            contracts.push(serde_json::json!({
                "localName": import.local_name,
                "namespace": import.alias,
                "baseUrl": locked.base_url,
                "artifactBase64": BASE64.encode(artifact),
                "signature": signature,
                "publicKey": public_key,
                "verified": verified,
                "expectedSha256": locked.artifact_sha256,
                "operations": operations,
            }));
        }
    }
    // The virtual compiler output has no artifact bytes or authority lock.
    // Application assembly replaces this empty list with verified, embedded
    // extension facts. Keeping the source-only default empty means a direct
    // virtual graph can never accidentally grant a sandbox invocation.
    Ok(serde_json::json!({ "contracts": contracts, "extensions": [] }))
}

#[derive(Clone)]
struct VerifiedExtensionAssembly {
    runtime_entries: Vec<serde_json::Value>,
    files: BTreeMap<String, Vec<u8>>,
    lock: serde_json::Value,
}

fn package_target(target: UiTarget) -> PackageTarget {
    match target {
        UiTarget::Android => PackageTarget::Android,
        UiTarget::Ios => PackageTarget::Ios,
        UiTarget::Web => PackageTarget::Web,
    }
}

fn frontend_profile(target: UiTarget) -> axiom_extension_host::HostProfile {
    match target {
        UiTarget::Android => HostProfile::android(),
        UiTarget::Ios => HostProfile::ios(),
        UiTarget::Web => HostProfile::web(),
    }
}

/// Resolve every imported extension only from a signed release workflow and
/// stage content-addressed bytes for one application target. A local Rust
/// source build is permitted only as a reproducibility check: its module hash
/// must equal the separately signed release selected by the workflow. Building
/// source never creates authority, a release signature, or an executable
/// fallback.
fn assemble_verified_extensions(
    source: &Path,
    compilation: &UiCompilation,
    target: UiTarget,
) -> Result<VerifiedExtensionAssembly> {
    let ir = compilation
        .ir
        .as_ref()
        .context("valid extension assembly requires compiled UI IR")?;
    if ir.extension_imports.is_empty() {
        return Ok(VerifiedExtensionAssembly {
            runtime_entries: Vec::new(),
            files: BTreeMap::new(),
            lock: serde_json::json!({
                "format": "axiom-application-extension-lock/v1",
                "target": target.as_str(),
                "extensions": [],
            }),
        });
    }
    let root = source
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let workflow = root.join("AxiomExtensions.toml");
    if !workflow.is_file() {
        let aliases = ir
            .extension_imports
            .iter()
            .map(|value| value.alias.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "AXIOM_EXTENSION_WORKFLOW_MISSING: {} imports extension(s) {aliases}, but {} is absent. Build/sign/resolve the release workflow before target assembly",
            source.display(),
            workflow.display(),
        );
    }
    let target_package = package_target(target);
    let profile = frontend_profile(target);
    let mut selected = BTreeMap::<String, Vec<&axiom_ui::UiExtensionImport>>::new();
    for import in &ir.extension_imports {
        selected
            .entry(import.alias.clone())
            .or_default()
            .push(import);
    }
    let mut runtime_entries = Vec::new();
    let mut files = BTreeMap::new();
    let mut lock_entries = Vec::new();
    for (alias, imports) in selected {
        // AxiomDeps.toml is the only source registry. Rebuild/reuse its
        // content-addressed development cache first, then require it to match
        // the signed release workflow exactly. This gives `axiom run` a normal
        // source-edit loop without allowing an unsigned cache artifact to
        // cross the target boundary.
        let source_build = build_rust_source_extension(&SourceBuildOptions {
            deps: root.join("AxiomDeps.toml"),
            alias: alias.clone(),
            output_root: PathBuf::from(".axiom/extensions"),
            target: target_package,
            clean: false,
        })
        .with_context(|| {
            format!("AXIOM_EXTENSION_SOURCE_BUILD: build or reuse registered source for `{alias}`")
        })?;
        let loaded = load_verified_extension_target(&workflow, &alias, target_package)
            .with_context(|| {
                format!(
                    "AXIOM_EXTENSION_ASSEMBLY: resolve `{alias}` for {}",
                    target.as_str()
                )
            })?;
        if source_build.module_sha256 != loaded.report.provenance.module_sha256 {
            bail!(
                "AXIOM_EXTENSION_SOURCE_RELEASE_MISMATCH: `{alias}` source builds to {}, but the signed release workflow selects {}. Sign/resolve a release for the current source; the unsigned development cache is never embedded",
                source_build.module_sha256,
                loaded.report.provenance.module_sha256,
            );
        }
        let effective_authority = axiom_lib::extension_authority::EffectiveTargetAuthority {
            permissions: loaded.effective.permissions.clone(),
            budgets: loaded.effective.budgets.clone(),
        };
        profile
            .validate_authority(&effective_authority)
            .with_context(|| {
                format!(
                    "AXIOM_EXTENSION_PROFILE: `{alias}` is not permitted in the {} frontend host",
                    target.as_str()
                )
            })?;
        for import in &imports {
            if import.interface_sha256 != loaded.report.provenance.interface_sha256 {
                bail!(
                    "AXIOM_EXTENSION_INTERFACE_MISMATCH: `{alias}` source binding {} does not match the signed package interface",
                    import.local_name
                );
            }
        }
        let exports = imports
            .iter()
            .map(|import| import.export.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut bindings = Vec::new();
        let mut seen_bindings = BTreeSet::new();
        for page in &ir.pages {
            for action in &page.actions {
                for step in &action.steps {
                    let axiom_ui::UiActionStep::ExtensionInvoke {
                        alias: step_alias,
                        export,
                        interface_sha256,
                        abi_symbol,
                        state_scope,
                        ..
                    } = step
                    else {
                        continue;
                    };
                    if step_alias != &alias {
                        continue;
                    }
                    if !exports.contains(export)
                        || interface_sha256 != &loaded.report.provenance.interface_sha256
                    {
                        bail!("AXIOM_EXTENSION_BINDING_MISMATCH: `{alias}` action binding disagrees with its verified export interface");
                    }
                    if !seen_bindings.insert(action.semantic_id.value.clone()) {
                        bail!(
                            "AXIOM_EXTENSION_BINDING_DUPLICATE: action `{}` invokes `{alias}` more than once; split it into separately named actions so each sandbox event has one exact-once identity",
                            action.name,
                        );
                    }
                    bindings.push(serde_json::json!({
                        "actionSemanticId": action.semantic_id.value,
                        "export": export,
                        "interfaceSha256": interface_sha256,
                        "abiSymbol": abi_symbol,
                        "stateScope": state_scope,
                    }));
                }
            }
        }
        bindings.sort_by(|left, right| {
            left["actionSemanticId"]
                .as_str()
                .cmp(&right["actionSemanticId"].as_str())
        });
        let module_sha256 = loaded.report.provenance.module_sha256.clone();
        let package_sha256 = sha256_bytes(&loaded.package_bytes);
        let authority_sha256 = sha256_bytes(&loaded.authority_lock_bytes);
        let package_lock_sha256 = sha256_bytes(&loaded.package_lock_bytes);
        let module_path = format!("runtime/extensions/modules/{module_sha256}/module.wasm");
        let package_path = format!("runtime/extensions/packages/{package_sha256}.axiom");
        let authority_path = format!("runtime/extensions/authority/{authority_sha256}.json");
        let package_lock_path = format!("runtime/extensions/locks/{package_lock_sha256}.json");
        for (path, bytes) in [
            (module_path.clone(), loaded.module_bytes),
            (package_path.clone(), loaded.package_bytes),
            (authority_path.clone(), loaded.authority_lock_bytes),
            (package_lock_path.clone(), loaded.package_lock_bytes),
        ] {
            match files.get(&path) {
                Some(existing) if existing == &bytes => {}
                Some(_) => bail!("AXIOM_EXTENSION_ASSEMBLY: conflicting content-addressed extension artifact `{path}`"),
                None => {
                    files.insert(path, bytes);
                }
            }
        }
        let entry = serde_json::json!({
            "extension": alias,
            "application": loaded.application,
            "exports": exports,
            "interfaceSha256": loaded.report.provenance.interface_sha256,
            "target": target.as_str(),
            "verified": true,
            "moduleSha256": module_sha256,
            "packageSha256": package_sha256,
            "authoritySha256": authority_sha256,
            "packageLockSha256": package_lock_sha256,
            "packageSignerSha256": loaded.report.provenance.package_signer_sha256,
            "limitsSha256": loaded.report.provenance.limits_sha256,
            "modulePath": module_path,
            "packagePath": package_path,
            "authorityLockPath": authority_path,
            "packageLockPath": package_lock_path,
            "effectiveAuthority": loaded.effective,
            "bindings": bindings,
            "runtime": {
                "engine": loaded.report.provenance.engine,
                "broker": loaded.report.provenance.broker,
                "host": loaded.report.provenance.host,
            },
        });
        lock_entries.push(entry.clone());
        runtime_entries.push(entry);
    }
    Ok(VerifiedExtensionAssembly {
        runtime_entries,
        files,
        lock: serde_json::json!({
            "format": "axiom-application-extension-lock/v1",
            "target": target.as_str(),
            "extensions": lock_entries,
        }),
    })
}

fn build_application_runtime_config_value(
    compilation: &UiCompilation,
    lock_path: &Path,
    assembly: &VerifiedExtensionAssembly,
) -> Result<serde_json::Value> {
    let mut config = build_verified_runtime_config_value(compilation, lock_path)?;
    config["extensions"] = serde_json::Value::Array(assembly.runtime_entries.clone());
    Ok(config)
}

fn build_verified_runtime_config(compilation: &UiCompilation, lock_path: &Path) -> Result<String> {
    runtime_config_source(&build_verified_runtime_config_value(
        compilation,
        lock_path,
    )?)
}

fn runtime_config_source(runtime_config: &serde_json::Value) -> Result<String> {
    Ok(format!(
        "// Compiler-owned opaque runtime configuration. Never materialized in the workspace.\nexport const runtimeConfig = {} as const;\n",
        serde_json::to_string(runtime_config)?
    ))
}

fn locked_ui_input(root: &Path, relative: &Path, kind: &str) -> Result<PathBuf> {
    if relative.is_absolute()
        || !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        bail!(
            "AXIOM_UI_RUNTIME_LOCK: {} path must be a safe lock-relative path",
            kind
        )
    }
    Ok(root.join(relative))
}

fn sha256_bytes(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn materialize_runtime_facade_package(project: &Path) -> Result<()> {
    let destination = project.join("virtual/axiom-runtime");
    std::fs::create_dir_all(&destination)?;
    // These files are part of the CLI protocol, not application source. Embed
    // them in the executable so a released CLI never depends on the absolute
    // CARGO_MANIFEST_DIR of the machine that compiled it or on a monorepo
    // checkout being present on the developer's machine.
    for (name, contents) in [
        (
            "index.js",
            include_bytes!("../../../../packages/axiom-lynx-runtime/src/index.js").as_slice(),
        ),
        (
            "lynx-adapter.js",
            include_bytes!("../../../../packages/axiom-lynx-runtime/src/lynx-adapter.js")
                .as_slice(),
        ),
    ] {
        std::fs::write(destination.join(name), contents)?;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpaqueUiAsset {
    path: String,
    sha256: String,
}

/// Embed compiler-declared, content-addressed assets into the private TSX
/// input as data URLs. Native delivery currently transports one Lynx bundle,
/// so a sibling asset file would never reach either host. A changed file is
/// rejected until the Acore graph has been recompiled with its new digest.
fn embed_declared_assets(
    build: &VirtualReactLynxBuild,
    asset_root: &Path,
    project: &Path,
) -> Result<()> {
    let manifest = build
        .files
        .get("virtual/axiom-assets.json")
        .ok_or_else(|| {
            anyhow::anyhow!("AXIOM_UI_ASSET_MANIFEST: virtual asset manifest is missing")
        })?;
    let assets: Vec<OpaqueUiAsset> = serde_json::from_str(&manifest.content)
        .context("AXIOM_UI_ASSET_MANIFEST: parse compiler-owned asset manifest")?;
    for asset in assets {
        let relative = Path::new(&asset.path);
        if relative.is_absolute()
            || !relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
            || !matches!(
                relative.extension().and_then(|value| value.to_str()),
                Some("png" | "jpg" | "jpeg" | "webp" | "svg")
            )
        {
            bail!(
                "AXIOM_UI_ASSET_PATH: asset path must be a safe relative image path: {}",
                asset.path
            );
        }
        if asset.sha256.len() != 64 || !asset.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!(
                "AXIOM_UI_ASSET_MANIFEST: asset digest is invalid for {}",
                asset.path
            );
        }
        let source = asset_root.join(relative);
        let bytes = std::fs::read(&source).with_context(|| {
            format!(
                "AXIOM_UI_ASSET_READ: read declared asset {}",
                source.display()
            )
        })?;
        if sha256_bytes(&bytes) != asset.sha256 {
            bail!("AXIOM_UI_ASSET_CHANGED: declared asset {} changed after Acore compilation; save/recompile before native delivery", asset.path);
        }
        let mime = match relative.extension().and_then(|value| value.to_str()) {
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("svg") => "image/svg+xml",
            _ => unreachable!("asset extension was validated above"),
        };
        let encoded = BASE64.encode(&bytes);
        let escaped_path = asset.path.replace('&', "&amp;").replace('"', "&quot;");
        let mut replacements = 0usize;
        for attribute in ["src", "placeholder"] {
            let reference = format!("{attribute}=\"./assets/{escaped_path}\"");
            let embedded = if mime == "image/svg+xml" && attribute == "src" {
                let content = std::str::from_utf8(&bytes)
                    .context("AXIOM_UI_ASSET_EMBED: declared SVG is not UTF-8")?;
                format!("content={{{}}}", serde_json::to_string(content)?)
            } else {
                format!("{attribute}=\"data:{mime};base64,{encoded}\"")
            };
            for file in build
                .files
                .values()
                .filter(|file| file.path.ends_with(".tsx"))
            {
                let generated = project.join(&file.path);
                let source = std::fs::read_to_string(&generated).with_context(|| {
                    format!("AXIOM_UI_ASSET_EMBED: read {}", generated.display())
                })?;
                let count = source.matches(&reference).count();
                if count > 0 {
                    std::fs::write(&generated, source.replace(&reference, &embedded))
                        .with_context(|| {
                            format!("AXIOM_UI_ASSET_EMBED: write {}", generated.display())
                        })?;
                    replacements += count;
                }
            }
        }
        if replacements == 0 {
            bail!(
                "AXIOM_UI_ASSET_REFERENCE: declared asset {} has no generated image reference",
                asset.path
            );
        }
    }
    Ok(())
}

/// Materialize compiler-owned virtual files only under a private cache. This
/// exists because Rspeedy is a filesystem compiler; no caller-supplied path is
/// used and no generated source enters the Acore workspace or release input.
fn compile_virtual_lynx_bundle(
    build: &VirtualReactLynxBuild,
    runtime_config: &str,
    asset_root: &Path,
    target: UiTarget,
) -> Result<PathBuf> {
    if build
        .files
        .values()
        .any(|file| !file.read_only || !file.path.starts_with("virtual/"))
    {
        bail!(
            "AXIOM_UI_VIRTUAL_INPUT: native delivery accepts only read-only virtual compiler files"
        );
    }
    let engine = resolve_lynx_toolchain()?;
    // A graph revision is shared by equivalent iOS and Android compilations.
    // Each watcher needs its own disposable Rspeedy workspace: deleting a
    // graph-global directory lets one target destroy the other target's build
    // midway through compilation.
    let project = ui_build_workspace(
        &ui_cache_root()?,
        &build.graph_revision,
        target,
        std::process::id(),
    );
    if project.exists() {
        std::fs::remove_dir_all(&project)
            .with_context(|| format!("cannot reset opaque UI build cache {}", project.display()))?;
    }
    if ui_debug_enabled() {
        eprintln!(
            "AXIOM_UI_BUILD: compiling {} graph {} in {}",
            target.as_str(),
            build.graph_revision,
            project.display()
        );
    }
    let virtual_root = project.join("virtual");
    std::fs::create_dir_all(&virtual_root)?;
    for file in build.files.values() {
        let relative = Path::new(&file.path);
        if relative.components().count() != 2 || relative.parent() != Some(Path::new("virtual")) {
            bail!("AXIOM_UI_VIRTUAL_INPUT: unsafe virtual path {}", file.path);
        }
        let destination = project.join(relative);
        let content = if file.path == "virtual/axiom-runtime-config.ts" {
            runtime_config
        } else {
            &file.content
        };
        std::fs::write(&destination, content).with_context(|| {
            format!("cannot write opaque virtual file {}", destination.display())
        })?;
    }
    materialize_runtime_facade_package(&project)?;
    embed_declared_assets(build, asset_root, &project)?;
    if !project.join("virtual/main.tsx").is_file() {
        bail!("AXIOM_UI_LOWERING: virtual ReactLynx entrypoint is missing");
    }
    std::fs::write(
        project.join("package.json"),
        "{\"private\":true,\"type\":\"module\"}\n",
    )?;
    std::fs::write(project.join("tsconfig.json"), LYNX_TSCONFIG)?;
    std::fs::write(project.join("lynx.config.mjs"), LYNX_RSPEEDY_CONFIG)?;
    let component_modules = ensure_lynx_ui_toolchain()?;
    link_compiler_dependencies(&engine, &component_modules, &project)?;

    let node = resolve_rspack_node(&engine)?;
    let rspeedy = component_modules.join("@lynx-js/rspeedy/bin/rspeedy.js");
    if !rspeedy.is_file() {
        bail!("AXIOM_UI_TOOLCHAIN: pinned ReactLynx compiler is incomplete at {}; provision it with `axiom ui toolchain provision`", engine.display());
    }
    let output = Command::new(&node)
        .arg(&rspeedy)
        .arg("build")
        .arg("--config")
        .arg(project.join("lynx.config.mjs"))
        .current_dir(&project)
        // ReactLynx transitively invokes Browserslist. Its database-age notice
        // is unrelated to a native bundle and must not make an exact pinned,
        // opaque compiler cache nondeterministic or require developer-local
        // package mutation. Dependency upgrades happen only by updating the
        // reviewed toolchain pin.
        .env("BROWSERSLIST_IGNORE_OLD_DATA", "true")
        .env("NODE_DISABLE_COMPILE_CACHE", "1")
        .output()
        .with_context(|| {
            format!(
                "cannot start the pinned ReactLynx compiler at {}",
                node.display()
            )
        })?;
    if ui_debug_enabled() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stdout.trim().is_empty() {
            eprintln!("AXIOM_UI_COMPILER_STDOUT:\n{}", stdout.trim());
        }
        if !stderr.trim().is_empty() {
            eprintln!("AXIOM_UI_COMPILER_STDERR:\n{}", stderr.trim());
        }
    }
    require_success("ReactLynx bundle compilation", &output)?;
    let bundle = project.join("dist/main.lynx.bundle");
    if !bundle.is_file() || std::fs::metadata(&bundle)?.len() == 0 {
        bail!("AXIOM_UI_BUNDLE: the pinned compiler completed without dist/main.lynx.bundle");
    }
    Ok(bundle)
}

fn resolve_rspack_node(engine: &Path) -> Result<PathBuf> {
    let bundled = engine.join("buildtools/node/bin/node");
    let candidates = [bundled, PathBuf::from("node")];
    for candidate in candidates {
        if candidate.is_absolute() && !candidate.is_file() {
            continue;
        }
        let Ok(output) = Command::new(&candidate).arg("--version").output() else {
            continue;
        };
        if output.status.success()
            && rspack_supports_node_version(String::from_utf8_lossy(&output.stdout).trim())
        {
            return Ok(candidate);
        }
    }
    bail!("AXIOM_UI_NODE_VERSION: ReactLynx compilation requires Node.js 20.19+ or 22.12+ on PATH")
}

fn rspack_supports_node_version(version: &str) -> bool {
    let mut parts = version
        .strip_prefix('v')
        .unwrap_or(version)
        .split('.')
        .filter_map(|part| part.parse::<u64>().ok());
    match (parts.next(), parts.next()) {
        (Some(20), Some(minor)) => minor >= 19,
        (Some(major), Some(minor)) if major >= 22 => major > 22 || minor >= 12,
        _ => false,
    }
}

fn ui_build_workspace(root: &Path, graph_revision: &str, target: UiTarget, pid: u32) -> PathBuf {
    root.join("builds")
        .join(graph_revision)
        .join(format!("{}-{pid}", target.as_str()))
}

fn resolve_lynx_toolchain() -> Result<PathBuf> {
    let configured = std::env::var_os("AXIOM_UI_LYNX_ENGINE_ROOT").map(PathBuf::from);
    let maintained =
        dirs::home_dir().map(|home| home.join(".cache/axiom-ui-host/stage/ios-simulator/engine"));
    let owned = ui_cache_root()?
        .join("toolchain/lynx")
        .join(LYNX_ENGINE_COMMIT);
    for candidate in configured
        .into_iter()
        .chain(maintained)
        .chain(std::iter::once(owned.clone()))
    {
        if candidate
            .join("explorer/homepage/node_modules/@lynx-js/rspeedy/bin/rspeedy.js")
            .is_file()
        {
            verify_lynx_engine_revision(&candidate)?;
            return Ok(candidate);
        }
    }
    provision_lynx_toolchain(&owned)?;
    verify_lynx_engine_revision(&owned)?;
    Ok(owned)
}

fn verify_lynx_engine_revision(engine: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["-C", engine.to_string_lossy().as_ref(), "rev-parse", "HEAD"])
        .output()
        .with_context(|| format!("cannot verify pinned UI toolchain at {}", engine.display()))?;
    require_success("pinned UI toolchain revision check", &output)?;
    let revision = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if revision != LYNX_ENGINE_COMMIT {
        bail!(
            "AXIOM_UI_TOOLCHAIN_PIN: expected Lynx {}, found {}",
            LYNX_ENGINE_COMMIT,
            revision
        );
    }
    Ok(())
}

fn provision_lynx_toolchain(engine: &Path) -> Result<()> {
    let parent = engine.parent().expect("engine cache has parent");
    std::fs::create_dir_all(parent)?;
    if !engine.exists() {
        require_success(
            "pinned UI toolchain clone",
            &Command::new("git")
                .args([
                    "clone",
                    "--filter=blob:none",
                    LYNX_ENGINE_SOURCE,
                    engine.to_string_lossy().as_ref(),
                ])
                .output()?,
        )?;
    }
    require_success(
        "pinned UI toolchain checkout",
        &Command::new("git")
            .args([
                "-C",
                engine.to_string_lossy().as_ref(),
                "checkout",
                "--detach",
                LYNX_ENGINE_COMMIT,
            ])
            .output()?,
    )?;
    require_success(
        "pinned ReactLynx dependency install",
        &Command::new("corepack")
            .args(["pnpm", "install", "--frozen-lockfile"])
            .current_dir(engine)
            .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
            .output()?,
    )?;
    Ok(())
}

fn ensure_lynx_ui_toolchain() -> Result<PathBuf> {
    let root = ui_cache_root()?
        .join("toolchain/lynx-ui")
        .join(format!("{LYNX_UI_VERSION}-react-{LYNX_REACT_VERSION}"));
    let modules = root.join("node_modules");
    let package = modules.join("@lynx-js/lynx-ui/package.json");
    if package.is_file() {
        if verify_lynx_ui_toolchain(&root, &package).is_ok() {
            return Ok(modules);
        }
        bail!("AXIOM_UI_COMPONENT_PIN: cached Lynx UI package does not match the pinned version and integrity; remove the versioned cache directory and retry");
    }
    std::fs::create_dir_all(&root)?;
    std::fs::write(
        root.join("package.json"),
        format!(
            "{{\"private\":true,\"dependencies\":{{\"@lynx-js/lynx-ui\":\"={LYNX_UI_VERSION}\",\"@lynx-js/react\":\"={LYNX_REACT_VERSION}\",\"@lynx-js/react-rsbuild-plugin\":\"={LYNX_REACT_RSBUILD_PLUGIN_VERSION}\",\"@lynx-js/rspeedy\":\"={LYNX_RSPEEDY_VERSION}\",\"@lynx-js/types\":\"={LYNX_TYPES_VERSION}\",\"@types/react\":\"={TYPES_REACT_VERSION}\",\"typescript\":\"={TYPESCRIPT_VERSION}\"}}}}\n"
        ),
    )?;
    let output = Command::new("npm")
        .args([
            "install",
            "--ignore-scripts",
            "--legacy-peer-deps",
            "--no-audit",
            "--no-fund",
            "--package-lock=true",
        ])
        .current_dir(&root)
        .output()
        .with_context(|| "cannot provision the pinned official Lynx UI component package")?;
    require_success("pinned Lynx UI component install", &output)?;
    verify_lynx_ui_toolchain(&root, &package)?;
    Ok(modules)
}

fn verify_lynx_ui_toolchain(root: &Path, package: &Path) -> Result<()> {
    let payload: serde_json::Value = serde_json::from_slice(&std::fs::read(package)?)?;
    let lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("package-lock.json"))?)?;
    let packages = &lock["packages"];
    let locked = &packages["node_modules/@lynx-js/lynx-ui"];
    if payload["version"].as_str() != Some(LYNX_UI_VERSION)
        || locked["version"].as_str() != Some(LYNX_UI_VERSION)
        || locked["integrity"].as_str() != Some(LYNX_UI_NPM_INTEGRITY)
    {
        bail!(
            "AXIOM_UI_COMPONENT_PIN: @lynx-js/lynx-ui did not match version {}, integrity {}, source {}",
            LYNX_UI_VERSION,
            LYNX_UI_NPM_INTEGRITY,
            LYNX_UI_SOURCE_COMMIT
        );
    }
    for (name, version, integrity) in [
        (
            "@lynx-js/react",
            LYNX_REACT_VERSION,
            LYNX_REACT_NPM_INTEGRITY,
        ),
        (
            "@lynx-js/react-rsbuild-plugin",
            LYNX_REACT_RSBUILD_PLUGIN_VERSION,
            LYNX_REACT_RSBUILD_PLUGIN_NPM_INTEGRITY,
        ),
        (
            "@lynx-js/rspeedy",
            LYNX_RSPEEDY_VERSION,
            LYNX_RSPEEDY_NPM_INTEGRITY,
        ),
        (
            "@lynx-js/types",
            LYNX_TYPES_VERSION,
            LYNX_TYPES_NPM_INTEGRITY,
        ),
    ] {
        let locked = &packages[format!("node_modules/{name}")];
        if locked["version"].as_str() != Some(version)
            || locked["integrity"].as_str() != Some(integrity)
        {
            bail!(
                "AXIOM_UI_COMPONENT_PIN: {name} did not match version {version} and its reviewed npm integrity"
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn link_compiler_dependencies(engine: &Path, components: &Path, project: &Path) -> Result<()> {
    let engine_modules = engine.join("explorer/homepage/node_modules");
    if !engine_modules.is_dir() {
        bail!(
            "AXIOM_UI_TOOLCHAIN: ReactLynx dependencies are missing at {}",
            engine_modules.display()
        );
    }
    let destination = project.join("node_modules");
    std::fs::create_dir_all(&destination)?;
    // Companion packages from the reviewed lynx-ui source revision must win
    // over the older versions in the pinned engine's explorer demo.
    for source in [components, engine_modules.as_path()] {
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('@') && entry.path().is_dir() {
                let scope = destination.join(&name);
                std::fs::create_dir_all(&scope)?;
                for package in std::fs::read_dir(entry.path())? {
                    let package = package?;
                    let target = scope.join(package.file_name());
                    if !target.exists() {
                        std::os::unix::fs::symlink(package.path(), target)?;
                    }
                }
            } else {
                let target = destination.join(&name);
                if !target.exists() {
                    std::os::unix::fs::symlink(entry.path(), target)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn link_compiler_dependencies(_engine: &Path, _components: &Path, _project: &Path) -> Result<()> {
    bail!("AXIOM_UI_PLATFORM: native UI bundle compilation currently requires a Unix host")
}

fn deliver_ios_simulator_bundle(
    host: &UiHostInstallation,
    bundle: &Path,
    event: &HotReloadEvent,
    initial_delivery: bool,
    runtime_config_fingerprint: &str,
    extension_files: &BTreeMap<String, Vec<u8>>,
) -> Result<IosDeliveryAcknowledgement> {
    let artifact = host.artifact.as_ref().ok_or_else(|| {
        anyhow::anyhow!("native iOS delivery requires an installed signed UI Host release")
    })?;
    let preparation_progress =
        ui_host_spinner(format!("Preparing iOS UI Host {}...", artifact.version));
    let app = extract_ios_host_application(&host.host_root.join(&artifact.file), &artifact.version);
    preparation_progress.finish_and_clear();
    let app = app?;
    let device = select_ios_simulator()?;
    if initial_delivery {
        let boot_progress = ui_host_spinner("Preparing the iOS Simulator...");
        let boot = (|| {
            let _ = Command::new("xcrun")
                .args(["simctl", "boot", &device])
                .output()?;
            require_success(
                "iOS Simulator boot",
                &Command::new("xcrun")
                    .args(["simctl", "bootstatus", &device, "-b"])
                    .output()?,
            )?;
            // `simctl` can boot and launch a device without making Simulator.app
            // visible. Development mode must show the running native application.
            require_success(
                "iOS Simulator window",
                &Command::new("open").args(["-a", "Simulator"]).output()?,
            )
        })();
        boot_progress.finish_and_clear();
        boot?;
    }
    let existing_container = ios_host_data_container(&device)?;
    let installed_artifact_matches = existing_container.as_ref().is_some_and(|container| {
        let marker = PathBuf::from(container.trim())
            .join("Library/Application Support/AxiomUIHost")
            .join(HOST_ARTIFACT_MARKER);
        std::fs::read_to_string(marker)
            .ok()
            .is_some_and(|value| value.trim() == artifact.sha256)
    });
    let installed_now = existing_container.is_none() || !installed_artifact_matches;
    let container = if installed_now {
        let install_progress = ui_host_spinner(format!(
            "Installing Axiom UI Host {} on the iOS Simulator...",
            artifact.version
        ));
        // The bundle identifier is stable across host releases. Replace an
        // older installed app when its artifact marker is absent or differs;
        // otherwise a newly verified archive can be paired with a stale host
        // binary that still speaks an older delivery protocol.
        let installation = (|| {
            let _ = Command::new("xcrun")
                .args(["simctl", "terminate", &device, IOS_HOST_BUNDLE_ID])
                .output()?;
            require_success(
                "Axiom UI Host install",
                &Command::new("xcrun")
                    .args(["simctl", "install", &device, app.to_string_lossy().as_ref()])
                    .output()?,
            )?;
            ios_host_data_container(&device)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "Axiom UI Host was installed but its Simulator container is unavailable"
                )
            })
        })();
        install_progress.finish_and_clear();
        installation?
    } else {
        existing_container.expect("matching artifact has an installed data container")
    };
    let support = PathBuf::from(container.trim()).join("Library/Application Support/AxiomUIHost");
    std::fs::create_dir_all(&support)?;
    if installed_now {
        std::fs::write(
            support.join(HOST_ARTIFACT_MARKER),
            format!("{}\n", artifact.sha256),
        )?;
    }
    let prior_runtime_config =
        std::fs::read_to_string(support.join(NATIVE_RUNTIME_CONFIG_MARKER)).ok();
    let restart_required = native_runtime_restart_required(
        initial_delivery,
        installed_now,
        prior_runtime_config.as_deref().map(str::trim),
        runtime_config_fingerprint,
    );
    // The native runtime deliberately refuses to replace verified contracts in
    // a live process. Stop it before staging a changed revision so the old host
    // cannot acknowledge the new files with its stale runtime configuration.
    if restart_required && !installed_now {
        let _ = Command::new("xcrun")
            .args(["simctl", "terminate", &device, IOS_HOST_BUNDLE_ID])
            .output()?;
    }
    let (delivery_mode, fallback_reason) = ios_delivery_plan(event);
    let base_graph_revision = previous_ios_graph_revision(&support);
    // UiDevelopmentSession sequence numbers restart with every CLI process.
    // The host, however, is long-lived and uses the sequence/graph pair to
    // decide whether a template must reload. Allocate a delivery sequence from
    // host-owned state so a fresh `axiom run` cannot falsely accept an earlier
    // acknowledgement for the same graph revision.
    let delivery_sequence = next_ios_delivery_sequence(&support, event.sequence);
    let _ = std::fs::remove_file(support.join("axiom.app.diagnostic.json"));
    stage_extension_files(&support, extension_files)?;
    atomic_copy(bundle, &support.join("axiom.app.lynx.bundle"))?;
    atomic_json(
        &support.join("axiom.app.revision.json"),
        &serde_json::json!({
            "format": "axiom-ui-host-revision/v2", "sequence": delivery_sequence, "graphRevision": event.graph_revision,
            "bundleSha256": sha256_file(bundle)?,
            "deliveryMode": delivery_mode.as_protocol_value(),
            "baseGraphRevision": base_graph_revision,
            "fallbackReason": fallback_reason,
        }),
    )?;
    if restart_required {
        require_success(
            "Axiom UI Host launch",
            &Command::new("xcrun")
                .args(["simctl", "launch", &device, IOS_HOST_BUNDLE_ID])
                .output()?,
        )?;
    }
    let acknowledgement =
        wait_for_ios_acknowledgement(&support, delivery_sequence, &event.graph_revision)?;
    std::fs::write(
        support.join(NATIVE_RUNTIME_CONFIG_MARKER),
        format!("{runtime_config_fingerprint}\n"),
    )?;
    Ok(acknowledgement)
}

fn native_runtime_restart_required(
    initial_delivery: bool,
    installed_now: bool,
    previous_fingerprint: Option<&str>,
    next_fingerprint: &str,
) -> bool {
    initial_delivery || installed_now || previous_fingerprint != Some(next_fingerprint)
}

/// Android mirrors the iOS fixed-file protocol, but transport goes through
/// `adb run-as` so bundle/revision/ack records remain app-private. The shipped
/// Android artifact is intentionally a debuggable development host; a future
/// product APK must not inherit this transport capability.
fn deliver_android_emulator_bundle(
    host: &UiHostInstallation,
    bundle: &Path,
    event: &HotReloadEvent,
    initial_delivery: bool,
    runtime_config_fingerprint: &str,
    loopback_base_urls: &[String],
    extension_files: &BTreeMap<String, Vec<u8>>,
) -> Result<IosDeliveryAcknowledgement> {
    let artifact = host.artifact.as_ref().ok_or_else(|| {
        anyhow::anyhow!("native Android delivery requires an installed signed UI Host release")
    })?;
    let apk = host.host_root.join(&artifact.file);
    if !apk.is_file() {
        bail!(
            "installed Android UI Host APK is missing: {}",
            apk.display()
        );
    }
    let device = select_android_emulator()?;
    configure_android_loopback(&device, loopback_base_urls)?;
    let installed = android_host_installed(&device)?;
    let installed_artifact_matches = installed
        && android_private_text(&device, HOST_ARTIFACT_MARKER)
            .is_some_and(|value| value.trim() == artifact.sha256);
    let installed_now = !installed || !installed_artifact_matches;
    if installed_now {
        let install_progress = ui_host_spinner(format!(
            "Installing Axiom UI Host {} on the Android Emulator...",
            artifact.version
        ));
        let installation = install_android_host(&device, &apk);
        install_progress.finish_and_clear();
        installation?;
    }
    android_run_as(&device, "mkdir -p files/axiom-ui-host")?;
    let support = android_delivery_cache(&device)?;
    let _ = android_run_as(
        &device,
        "rm -f files/axiom-ui-host/axiom.app.diagnostic.json",
    );
    if installed_now {
        let marker = support.join(HOST_ARTIFACT_MARKER);
        std::fs::write(&marker, format!("{}\n", artifact.sha256))?;
        android_push_private_file(&device, &marker, HOST_ARTIFACT_MARKER)?;
    }
    let prior_runtime_config = android_private_text(&device, NATIVE_RUNTIME_CONFIG_MARKER);
    let restart_required = native_runtime_restart_required(
        initial_delivery,
        installed_now,
        prior_runtime_config.as_deref().map(str::trim),
        runtime_config_fingerprint,
    );
    // Like iOS, Android's verified bridge refuses to replace contract inputs
    // inside a live process. Stop it before staging the next revision so a
    // stale Java/native runtime cannot acknowledge the new template first.
    if restart_required && !installed_now {
        require_success(
            "Axiom UI Host Android Emulator stop",
            &adb_command(
                &device,
                ["shell", "am", "force-stop", ANDROID_HOST_BUNDLE_ID],
            )
            .output()?,
        )?;
    }
    let (delivery_mode, fallback_reason) = ios_delivery_plan(event);
    let base_graph_revision = previous_android_graph_revision(&device);
    let delivery_sequence = next_android_delivery_sequence(&device, event.sequence);
    let revision = serde_json::json!({
        "format": "axiom-ui-host-revision/v2", "sequence": delivery_sequence,
        "graphRevision": event.graph_revision, "bundleSha256": sha256_file(bundle)?,
        "deliveryMode": delivery_mode.as_protocol_value(), "baseGraphRevision": base_graph_revision,
        "fallbackReason": fallback_reason,
    });
    let local_bundle = support.join("axiom.app.lynx.bundle");
    let local_revision = support.join("axiom.app.revision.json");
    stage_extension_files(&support, extension_files)?;
    atomic_copy(bundle, &local_bundle)?;
    atomic_json(&local_revision, &revision)?;
    android_push_private_file(&device, &local_bundle, "axiom.app.lynx.bundle")?;
    android_push_private_file(&device, &local_revision, "axiom.app.revision.json")?;
    android_push_private_extension_files(&device, &support, extension_files)?;
    if restart_required {
        require_success(
            "Axiom UI Host Android Emulator launch",
            &adb_command(
                &device,
                [
                    "shell",
                    "am",
                    "start",
                    "-n",
                    ANDROID_HOST_ACTIVITY_COMPONENT,
                ],
            )
            .output()?,
        )?;
    }
    let acknowledgement =
        wait_for_android_acknowledgement(&device, delivery_sequence, &event.graph_revision)?;
    let marker = support.join(NATIVE_RUNTIME_CONFIG_MARKER);
    std::fs::write(&marker, format!("{runtime_config_fingerprint}\n"))?;
    android_push_private_file(&device, &marker, NATIVE_RUNTIME_CONFIG_MARKER)?;
    Ok(acknowledgement)
}

/// Development host builds can be signed with a fresh local key. Android
/// correctly refuses an in-place update when that key changes. This host uses
/// a fixed Axiom-owned development package ID, so it is safe to remove only
/// that package and retry; no user application package is touched.
fn install_android_host(device: &str, apk: &Path) -> Result<()> {
    let output = adb_command(device, ["install", "-r", apk.to_string_lossy().as_ref()]).output()?;
    if output.status.success() {
        return Ok(());
    }
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !android_signature_conflict(&diagnostic) {
        return require_success("Axiom UI Host Android Emulator install", &output);
    }
    println!(
        "Replacing the stale Axiom development UI Host on the Android Emulator because its signing identity changed."
    );
    require_success(
        "Axiom UI Host Android Emulator stale-host removal",
        &adb_command(device, ["uninstall", ANDROID_HOST_BUNDLE_ID]).output()?,
    )?;
    require_success(
        "Axiom UI Host Android Emulator reinstall",
        &adb_command(device, ["install", apk.to_string_lossy().as_ref()]).output()?,
    )
}

fn android_signature_conflict(output: &str) -> bool {
    output.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE")
        || output.contains("signatures do not match")
}

/// Keep localhost contract URLs target-neutral. On Android, adb reverse makes
/// device loopback reach the developer machine just as Simulator loopback does.
/// Derive these URLs from the already verified runtime configuration so an app
/// without contract imports never needs a UI lock merely to launch.
fn android_loopback_base_urls(runtime_config: &serde_json::Value) -> Vec<String> {
    runtime_config["contracts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|contract| contract["baseUrl"].as_str().map(str::to_string))
        .collect()
}

fn configure_android_loopback(device: &str, base_urls: &[String]) -> Result<()> {
    let mut ports = base_urls
        .iter()
        .filter_map(|base_url| loopback_port(base_url))
        .collect::<Vec<_>>();
    ports.sort_unstable();
    ports.dedup();
    for port in ports {
        let socket = format!("tcp:{port}");
        require_success(
            "Android local contract port forwarding",
            &adb_command(device, ["reverse", &socket, &socket]).output()?,
        )?;
    }
    Ok(())
}

fn loopback_port(base_url: &str) -> Option<u16> {
    let (default_port, authority) = if let Some(value) = base_url.strip_prefix("http://") {
        (80, value)
    } else if let Some(value) = base_url.strip_prefix("https://") {
        (443, value)
    } else {
        return None;
    };
    let authority = authority.split('/').next()?;
    let (host, port) = authority
        .rsplit_once(':')
        .map(|(host, port)| (host, port.parse().ok()))
        .unwrap_or((authority, Some(default_port)));
    if matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        port
    } else {
        None
    }
}

fn adb_command<'a>(device: &'a str, args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut command = Command::new("adb");
    command.args(["-s", device]);
    command.args(args);
    command
}

fn select_android_emulator() -> Result<String> {
    let requested = std::env::var("AXIOM_UI_ANDROID_DEVICE_SERIAL").ok();
    let output = command_stdout(
        "Android device discovery",
        Command::new("adb").arg("devices"),
    )?;
    let mut devices = output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            let state = fields.next()?;
            (state == "device").then(|| serial.to_string())
        })
        .collect::<Vec<_>>();
    if let Some(serial) = requested {
        if devices.iter().any(|candidate| candidate == &serial) {
            return Ok(serial);
        }
        bail!("AXIOM_UI_ANDROID_DEVICE: AXIOM_UI_ANDROID_DEVICE_SERIAL `{serial}` is not an authorized adb device");
    }
    devices.sort();
    devices.into_iter().find(|serial| serial.starts_with("emulator-"))
        .ok_or_else(|| anyhow::anyhow!("AXIOM_UI_ANDROID_EMULATOR_MISSING: start an Android Emulator (or set AXIOM_UI_ANDROID_DEVICE_SERIAL to an authorized development device), then rerun `axiom run`"))
}

fn android_host_installed(device: &str) -> Result<bool> {
    Ok(
        adb_command(device, ["shell", "pm", "path", ANDROID_HOST_BUNDLE_ID])
            .output()?
            .status
            .success(),
    )
}

fn android_delivery_cache(device: &str) -> Result<PathBuf> {
    let safe = device
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() {
                value
            } else {
                '_'
            }
        })
        .collect::<String>();
    let root = ui_cache_root()?.join("android-delivery").join(safe);
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

fn android_run_as(device: &str, script: &str) -> Result<String> {
    // `adb shell` joins its local argv into one remote command line. Without
    // explicit quoting, `sh -c mkdir -p ...` gives `sh` only `mkdir` as the
    // script and treats the remaining words as positional parameters.
    let quoted_script = quote_remote_shell_argument(script);
    command_stdout(
        "Android UI Host private storage access",
        &mut adb_command(
            device,
            [
                "shell",
                "run-as",
                ANDROID_HOST_BUNDLE_ID,
                "sh",
                "-c",
                quoted_script.as_str(),
            ],
        ),
    )
}

fn quote_remote_shell_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn android_push_private_file(device: &str, source: &Path, name: &str) -> Result<()> {
    let remote = format!("/data/local/tmp/axiom-ui-host-{name}");
    require_success(
        "Axiom UI Host Android bundle transfer",
        &adb_command(device, ["push", source.to_string_lossy().as_ref(), &remote]).output()?,
    )?;
    let copy = format!("cp {remote} files/axiom-ui-host/{name}");
    android_run_as(device, &copy)?;
    require_success(
        "Axiom UI Host Android temporary-file cleanup",
        &adb_command(device, ["shell", "rm", "-f", &remote]).output()?,
    )?;
    Ok(())
}

fn android_push_private_extension_files(
    device: &str,
    root: &Path,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    for (relative, bytes) in files {
        validate_extension_artifact_path(relative)?;
        let source = root.join(relative);
        if !source.is_file() || sha256_file(&source)? != sha256_bytes(bytes) {
            bail!("AXIOM_UI_EXTENSION_DELIVERY: staged artifact `{relative}` changed before Android delivery");
        }
        let parent = Path::new(relative)
            .parent()
            .expect("validated extension artifact has a parent");
        let remote_name = format!("axiom-ui-extension-{}", sha256_bytes(bytes));
        let remote = format!("/data/local/tmp/{remote_name}");
        require_success(
            "Axiom UI Host Android extension transfer",
            &adb_command(device, ["push", source.to_string_lossy().as_ref(), &remote]).output()?,
        )?;
        android_run_as(
            device,
            &format!("mkdir -p files/axiom-ui-host/{}", parent.display()),
        )?;
        android_run_as(
            device,
            &format!("cp {remote} files/axiom-ui-host/{relative}"),
        )?;
        require_success(
            "Axiom UI Host Android extension temporary-file cleanup",
            &adb_command(device, ["shell", "rm", "-f", &remote]).output()?,
        )?;
    }
    Ok(())
}

fn android_private_text(device: &str, name: &str) -> Option<String> {
    android_run_as(device, &format!("cat files/axiom-ui-host/{name}")).ok()
}

fn previous_android_graph_revision(device: &str) -> Option<String> {
    android_private_text(device, "axiom.app.ack.json")
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|ack| ack["graphRevision"].as_str().map(str::to_string))
}

fn next_android_delivery_sequence(device: &str, fallback: u64) -> u64 {
    ["axiom.app.revision.json", "axiom.app.ack.json"]
        .into_iter()
        .filter_map(|name| android_private_text(device, name))
        .filter_map(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .filter_map(|value| value["sequence"].as_u64())
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .max(fallback)
}

fn wait_for_android_acknowledgement(
    device: &str,
    delivery_sequence: u64,
    graph_revision: &str,
) -> Result<IosDeliveryAcknowledgement> {
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if let Some(text) = android_private_text(device, "axiom.app.ack.json") {
            let ack = serde_json::from_str::<serde_json::Value>(&text).unwrap_or_default();
            if let Some(result) =
                parse_ios_acknowledgement(&ack, delivery_sequence, graph_revision)?
            {
                return Ok(result);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("AXIOM_UI_HOST_ACK_TIMEOUT: the Android Emulator host did not acknowledge native UI revision {delivery_sequence}. Run `axiom ui host recover --target android`, keep the Emulator open, and retry.")
}

fn recover_android_host() -> Result<()> {
    let device = select_android_emulator()?;
    if !android_host_installed(&device)? {
        bail!("AXIOM_UI_RECOVER_HOST_NOT_INSTALLED: install or run the Axiom UI Host on the selected Android Emulator first");
    }
    android_run_as(
        &device,
        "rm -f files/axiom-ui-host/axiom.app.revision.json files/axiom-ui-host/axiom.app.ack.json",
    )?;
    let _ = adb_command(
        &device,
        ["shell", "am", "force-stop", ANDROID_HOST_BUNDLE_ID],
    )
    .output()?;
    require_success(
        "Axiom UI Host Android recovery launch",
        &adb_command(
            &device,
            [
                "shell",
                "am",
                "start",
                "-n",
                ANDROID_HOST_ACTIVITY_COMPONENT,
            ],
        )
        .output()?,
    )?;
    println!("Recovered the Android UI Host: restarted the app and cleared only stale delivery control records. The next `axiom run` will publish the current last-good bundle.");
    Ok(())
}

fn ios_delivery_plan(event: &HotReloadEvent) -> (IosDeliveryMode, Option<String>) {
    match native_reload_directive(event) {
        NativeReloadDirective::StatePreservingPatch => (
            IosDeliveryMode::StateResetTemplate,
            Some("native state preservation is not certified for the pinned Lynx renderer; full template reload applied".to_string()),
        ),
        NativeReloadDirective::InitialTemplate => (
            IosDeliveryMode::StateResetTemplate,
            Some("initial native template load".to_string()),
        ),
        NativeReloadDirective::StateResetTemplate { reason } => {
            (IosDeliveryMode::StateResetTemplate, Some(reason))
        }
        NativeReloadDirective::RetainLastGood => (
            IosDeliveryMode::StateResetTemplate,
            Some("invalid edit retained the prior last-good graph".to_string()),
        ),
    }
}

fn previous_ios_graph_revision(support: &Path) -> Option<String> {
    std::fs::read_to_string(support.join("axiom.app.ack.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|ack| ack["graphRevision"].as_str().map(str::to_string))
}

fn next_ios_delivery_sequence(support: &Path, fallback: u64) -> u64 {
    let previous = ["axiom.app.revision.json", "axiom.app.ack.json"]
        .into_iter()
        .filter_map(|name| std::fs::read_to_string(support.join(name)).ok())
        .filter_map(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .filter_map(|value| value["sequence"].as_u64())
        .max()
        .unwrap_or(0);
    previous.saturating_add(1).max(fallback)
}

fn ios_host_data_container(device: &str) -> Result<Option<String>> {
    let output = Command::new("xcrun")
        .args([
            "simctl",
            "get_app_container",
            device,
            IOS_HOST_BUNDLE_ID,
            "data",
        ])
        .output()
        .context("cannot inspect the Axiom UI Host Simulator container")?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8(output.stdout)
                .context("Axiom UI Host Simulator container path is not UTF-8")?,
        ));
    }
    Ok(None)
}

fn extract_ios_host_application(archive: &Path, version: &str) -> Result<PathBuf> {
    if !archive.is_file() {
        bail!(
            "installed iOS UI Host archive is missing: {}",
            archive.display()
        );
    }
    let root = ui_cache_root()?.join("installed-hosts/ios").join(version);
    let app = root.join("AxiomUIHost.app");
    if app.is_dir() {
        return Ok(app);
    }
    std::fs::create_dir_all(&root)?;
    require_success(
        "Axiom UI Host archive extraction",
        &Command::new("ditto")
            .args([
                "-x",
                "-k",
                archive.to_string_lossy().as_ref(),
                root.to_string_lossy().as_ref(),
            ])
            .output()?,
    )?;
    if !app.is_dir() {
        bail!("Axiom UI Host archive did not contain AxiomUIHost.app");
    }
    Ok(app)
}

fn select_ios_simulator() -> Result<String> {
    let output = command_stdout(
        "iOS Simulator device discovery",
        Command::new("xcrun").args(["simctl", "list", "devices", "available", "-j"]),
    )?;
    let devices: serde_json::Value =
        serde_json::from_str(&output).context("iOS Simulator returned invalid device metadata")?;
    let mut available = devices["devices"]
        .as_object()
        .into_iter()
        .flat_map(|runtimes| runtimes.values())
        .flat_map(|value| value.as_array().into_iter().flatten())
        .filter(|device| {
            device["isAvailable"].as_bool().unwrap_or(false)
                && device["name"].as_str().unwrap_or("").contains("iPhone")
        })
        .collect::<Vec<_>>();
    available.sort_by_key(|device| {
        (
            !matches!(device["state"].as_str(), Some("Booted")),
            device["name"].as_str().unwrap_or("").to_string(),
        )
    });
    available.first().and_then(|device| device["udid"].as_str()).map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("AXIOM_UI_SIMULATOR_MISSING: install an iOS Simulator runtime, then rerun `axiom run`"))
}

fn wait_for_ios_acknowledgement(
    support: &Path,
    delivery_sequence: u64,
    graph_revision: &str,
) -> Result<IosDeliveryAcknowledgement> {
    let deadline = Instant::now() + Duration::from_secs(12);
    let path = support.join("axiom.app.ack.json");
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let ack: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            if let Some(result) =
                parse_ios_acknowledgement(&ack, delivery_sequence, graph_revision)?
            {
                return Ok(result);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("AXIOM_UI_HOST_ACK_TIMEOUT: the host launched but did not acknowledge native UI revision {delivery_sequence}. Run `axiom ui host recover --target ios`, keep the Simulator open, and retry.")
}

fn parse_ios_acknowledgement(
    ack: &serde_json::Value,
    delivery_sequence: u64,
    graph_revision: &str,
) -> Result<Option<IosDeliveryAcknowledgement>> {
    if ack["format"] != "axiom-ui-host-ack/v2"
        || ack["sequence"].as_u64() != Some(delivery_sequence)
        || ack["graphRevision"].as_str() != Some(graph_revision)
    {
        return Ok(None);
    }
    match ack["status"].as_str() {
        Some("applied_state_preserved") => Ok(Some(IosDeliveryAcknowledgement::AppliedStatePreserved)),
        Some("applied_state_reset") => {
            let reason = ack["reason"]
                .as_str()
                .unwrap_or("the host applied a full template reload");
            Ok(Some(IosDeliveryAcknowledgement::AppliedStateReset {
                reason: reason.to_string(),
            }))
        }
        Some("rejected_last_good") => bail!(
            "AXIOM_UI_HOST_REJECTED_LAST_GOOD: the native host rejected revision {delivery_sequence} and retained its prior bundle: {}",
            ack["reason"].as_str().unwrap_or("no reason provided")
        ),
        Some(status) => bail!(
            "AXIOM_UI_HOST_ACK_INVALID: native host returned unsupported acknowledgement status `{status}`"
        ),
        None => Ok(None),
    }
}

fn atomic_copy(source: &Path, destination: &Path) -> Result<()> {
    let temporary = destination.with_extension("bundle.tmp");
    std::fs::copy(source, &temporary)?;
    std::fs::rename(&temporary, destination)?;
    Ok(())
}

/// Stage only compiler-owned, content-addressed extension facts below the
/// private host directory. A module update receives a new digest path, so an
/// old running bundle can never observe partially replaced bytes.
fn stage_extension_files(root: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    for (relative, bytes) in files {
        validate_extension_artifact_path(relative)?;
        let destination = root.join(relative);
        let parent = destination
            .parent()
            .expect("validated extension artifact has a parent");
        std::fs::create_dir_all(parent)?;
        let temporary = destination.with_extension("extension.tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(&temporary, &destination)?;
        if sha256_file(&destination)? != sha256_bytes(bytes) {
            bail!("AXIOM_UI_EXTENSION_DELIVERY: staged artifact `{relative}` did not retain its verified digest");
        }
    }
    Ok(())
}

fn validate_extension_artifact_path(path: &str) -> Result<()> {
    let relative = Path::new(path);
    if !path.starts_with("runtime/extensions/")
        || relative.is_absolute()
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        bail!("AXIOM_UI_EXTENSION_DELIVERY: unsafe extension artifact path `{path}`");
    }
    Ok(())
}

fn atomic_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec(value)?)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn require_success(label: &str, output: &Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    bail!(
        "{label} failed: {}",
        if stderr.is_empty() { stdout } else { stderr }
    );
}

fn command_stdout(label: &str, command: &mut Command) -> Result<String> {
    let output = command.output()?;
    require_success(label, &output)?;
    String::from_utf8(output.stdout).with_context(|| format!("{label} returned non-UTF-8 output"))
}

const LYNX_TSCONFIG: &str = r#"{
  "compilerOptions": {
    "jsx": "react-jsx",
    "jsxImportSource": "@lynx-js/react",
    "module": "esnext",
    "moduleResolution": "bundler",
    "strict": true,
    "skipLibCheck": true
  },
  "exclude": ["dist"]
}
"#;

const LYNX_RSPEEDY_CONFIG: &str = r#"import { defineConfig } from '@lynx-js/rspeedy';
import { pluginReactLynx } from '@lynx-js/react-rsbuild-plugin';
export default defineConfig({
  source: { entry: './virtual/main.tsx' },
  output: { distPath: { root: './dist' } },
  plugins: [pluginReactLynx({ enableCSSInheritance: true })],
});
"#;

async fn ensure_ui_host(target: UiTarget) -> Result<UiHostInstallation> {
    let existing = read_ui_host(target)?;
    let web_archive_is_current = existing
        .as_ref()
        .map(web_host_archive_is_current)
        .transpose()?
        .unwrap_or(false);
    let needs_replacement = matches!(
        existing.as_ref().and_then(|host| host.artifact.as_ref()),
        Some(artifact) if artifact.version == "0.0.0-validation"
    ) || existing
        .as_ref()
        .is_some_and(|host| !host_supports_delivery(host))
        || (target == UiTarget::Web && !web_archive_is_current);
    if let Some(host) = existing.as_ref().filter(|_| !needs_replacement) {
        return Ok(host.clone());
    }
    if needs_replacement {
        let version = existing
            .as_ref()
            .and_then(|host| host.artifact.as_ref())
            .map(|artifact| artifact.version.as_str())
            .unwrap_or("local development host");
        println!(
            "The installed UI Host ({version}) cannot render current development bundles for {}.",
            target.as_str()
        );
    } else {
        println!("UI Host for {} is not installed.", target.as_str());
    }
    let install = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Install the latest verified UI Host now?")
        .default(true)
        .interact()?;
    if !install {
        bail!(
            "UI Host installation was skipped. Run `axiom ui host install --target {}` when ready.",
            target.as_str()
        );
    }
    install_ui_host(target, None, None, None, false).await?;
    let installed = read_ui_host(target)?
        .ok_or_else(|| anyhow::anyhow!("UI Host installation did not create a host record"))?;
    if !host_supports_delivery(&installed) {
        let version = installed
            .artifact
            .as_ref()
            .map(|value| value.version.as_str())
            .unwrap_or("local development host");
        bail!("AXIOM_UI_HOST_UPGRADE_REQUIRED: installed UI Host {version} does not implement the current delivery and rendering contract. Publish and install Axiom UI Host {} or later.", host_protocol_minimum(target));
    }
    if target == UiTarget::Web && !web_host_archive_is_current(&installed)? {
        bail!(
            "AXIOM_UI_HOST_WEB_INCOMPATIBLE: the installed web UI Host release does not contain the current browser extension runtime. Install a newly built signed web host release before running this application."
        );
    }
    Ok(installed)
}

async fn install_ui_host(
    target: UiTarget,
    release_manifest: Option<PathBuf>,
    variant: Option<String>,
    host_root: Option<PathBuf>,
    non_interactive: bool,
) -> Result<()> {
    if let Some(manifest) = release_manifest {
        return install_released_ui_host(target, &manifest, variant, non_interactive);
    }
    if host_root.is_none() {
        return install_latest_released_ui_host(target, variant).await;
    }
    let host_root = host_root.expect("checked above");
    if !host_root.is_dir() {
        bail!(
            "UI Host source directory does not exist: {}",
            host_root.display()
        );
    }
    let record = UiHostInstallation {
        format: "axiom-ui-host/v1".to_string(),
        target: target.as_str().to_string(),
        host_root: canonical_or_original(&host_root),
        artifact: None,
        // Local source registrations are intentionally not trusted as a
        // development transport. A signed release must identify its protocol.
        delivery_adapter_ready: false,
    };
    let path = ui_host_record_path(target)?;
    let parent = path.parent().expect("host record has parent");
    std::fs::create_dir_all(parent)?;
    std::fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&record)?),
    )?;
    println!("UI Host setup saved for {}.", target.as_str());
    println!("Host source: {}", record.host_root.display());
    println!("The host is registered for compiler development. Install a signed release to use native bundle delivery.");
    Ok(())
}

async fn install_latest_released_ui_host(
    target: UiTarget,
    requested_variant: Option<String>,
) -> Result<()> {
    if target == UiTarget::Ios && std::env::consts::ARCH != "aarch64" {
        bail!("the current Axiom iOS Simulator host release supports Apple Silicon Macs only; an x86_64 simulator host release is required for this machine");
    }
    let client = reqwest::Client::builder()
        .user_agent(format!("axiom-cli/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("cannot create the UI Host release client")?;
    let release_progress = ui_host_spinner(format!(
        "Checking the latest verified {} UI Host release...",
        target.as_str()
    ));
    let release_response = client
        .get(format!(
            "https://api.github.com/repos/{UI_HOST_GITHUB_REPOSITORY}/releases/latest"
        ))
        .send()
        .await
        .context("cannot reach the Axiom UI Host release service")?;
    if release_response.status() == reqwest::StatusCode::NOT_FOUND {
        release_progress.finish_and_clear();
        bail!(
            "the Axiom UI Host release is not publicly downloadable. Publish `https://github.com/{UI_HOST_GITHUB_REPOSITORY}` and its release assets publicly, or configure a public Axiom distribution endpoint before asking end users to install the host"
        );
    }
    let release: GitHubRelease = release_response
        .error_for_status()
        .context("cannot read the latest Axiom UI Host release")?
        .json()
        .await
        .context("latest Axiom UI Host release metadata is invalid")?;
    release_progress.finish_and_clear();
    let manifest_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == "host-manifest.json")
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Axiom UI Host release {} has no host-manifest.json asset",
                release.tag_name
            )
        })?;
    let signature_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == "host-manifest.json.sig")
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Axiom UI Host release {} has no host-manifest.json.sig asset",
                release.tag_name
            )
        })?;

    let download_dir = ui_host_record_path(target)?
        .parent()
        .expect("host record has parent")
        .join("downloads")
        .join("latest");
    std::fs::create_dir_all(&download_dir)?;
    let manifest_path = download_dir.join("host-manifest.json");
    std::fs::write(
        &manifest_path,
        download_release_asset(&client, manifest_asset).await?,
    )?;
    std::fs::write(
        PathBuf::from(format!("{}.sig", manifest_path.display())),
        download_release_asset(&client, signature_asset).await?,
    )?;

    // Authenticate manifest metadata before using its asset name or URL.
    let verification_progress = ui_host_spinner("Verifying the signed UI Host manifest...");
    let manifest = (|| {
        verify_release_manifest_signature(&manifest_path)?;
        serde_json::from_str::<UiHostReleaseManifest>(&std::fs::read_to_string(&manifest_path)?)
            .context("verified Axiom UI Host manifest is not valid JSON")
    })();
    verification_progress.finish_and_clear();
    let manifest = manifest?;
    // Do this before downloading the target artifact or replacing an installed
    // record. Otherwise a developer is prompted to "install the latest" and
    // ends up repeatedly downloading the same legacy release that the active
    // CLI deliberately cannot use for native development delivery.
    if !version_at_least(&manifest.version, host_protocol_minimum(target)) {
        bail!(
            "AXIOM_UI_HOST_RELEASE_TOO_OLD: the latest verified {} UI Host release ({}) predates the current delivery and rendering contract. Publish Axiom UI Host {} or later, then rerun this command. The installed host was not changed.",
            target.as_str(),
            manifest.version,
            host_protocol_minimum(target),
        );
    }
    let selected = select_release_asset(&manifest, target, requested_variant.as_deref(), true)?;
    let release_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == selected.file)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Axiom UI Host release {} is missing declared asset {}",
                release.tag_name,
                selected.file
            )
        })?;
    std::fs::write(
        download_dir.join(&selected.file),
        download_release_asset(&client, release_asset).await?,
    )?;

    install_released_ui_host(target, &manifest_path, requested_variant, true)
}

async fn download_release_asset(
    client: &reqwest::Client,
    asset: &GitHubReleaseAsset,
) -> Result<Vec<u8>> {
    let response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .with_context(|| format!("cannot download Axiom UI Host asset {}", asset.name))?
        .error_for_status()
        .with_context(|| format!("cannot download Axiom UI Host asset {}", asset.name))?;
    let capacity = response
        .content_length()
        .and_then(|size| usize::try_from(size).ok())
        .unwrap_or_default();
    let progress = ui_host_download_progress(&asset.name, response.content_length());
    let mut bytes = Vec::with_capacity(capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                progress.finish_and_clear();
                return Err(error)
                    .with_context(|| format!("cannot read Axiom UI Host asset {}", asset.name));
            }
        };
        progress.inc(chunk.len() as u64);
        bytes.extend_from_slice(&chunk);
    }
    progress.finish_and_clear();
    Ok(bytes)
}

fn install_released_ui_host(
    target: UiTarget,
    manifest_path: &Path,
    requested_variant: Option<String>,
    non_interactive: bool,
) -> Result<()> {
    let progress = ui_host_spinner(format!(
        "Verifying signed {} UI Host release...",
        target.as_str()
    ));
    let manifest_path = canonical_or_original(manifest_path);
    verify_release_manifest_signature(&manifest_path)?;
    let manifest = serde_json::from_str::<UiHostReleaseManifest>(
        &std::fs::read_to_string(&manifest_path).with_context(|| {
            format!(
                "cannot read UI Host release manifest {}",
                manifest_path.display()
            )
        })?,
    )?;
    if manifest.format != "axiom-ui-host-release/v1" || manifest.version.trim().is_empty() {
        bail!(
            "unsupported or incomplete UI Host release manifest: {}",
            manifest_path.display()
        );
    }
    let version = manifest.version.clone();
    let delivery_adapter_ready = version_at_least(&version, host_protocol_minimum(target));
    let asset = select_release_asset(
        &manifest,
        target,
        requested_variant.as_deref(),
        non_interactive,
    )?;
    let source = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&asset.file);
    if !source.is_file() {
        progress.finish_and_clear();
        bail!("UI Host release asset is missing: {}", source.display());
    }
    progress.set_message(format!("Verifying {} checksum...", asset.file));
    let actual = sha256_file(&source)?;
    if actual != asset.sha256 {
        progress.finish_and_clear();
        bail!(
            "UI Host release asset checksum mismatch for {}",
            source.display()
        );
    }
    let host_root = ui_host_record_path(target)?
        .parent()
        .expect("host record has parent")
        .join("artifacts")
        .join(&version);
    std::fs::create_dir_all(&host_root)?;
    let installed_asset = host_root.join(&asset.file);
    progress.set_message(format!("Installing {} UI Host archive...", target.as_str()));
    std::fs::copy(&source, &installed_asset).with_context(|| {
        format!(
            "cannot install UI Host asset to {}",
            installed_asset.display()
        )
    })?;
    let record = UiHostInstallation {
        format: "axiom-ui-host/v1".to_string(),
        target: target.as_str().to_string(),
        host_root: canonical_or_original(&host_root),
        artifact: Some(UiHostArtifact {
            version,
            variant: asset.variant.clone(),
            file: asset.file.clone(),
            sha256: asset.sha256.clone(),
        }),
        delivery_adapter_ready,
    };
    let path = ui_host_record_path(target)?;
    std::fs::create_dir_all(path.parent().expect("host record has parent"))?;
    std::fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&record)?),
    )?;
    progress.finish_and_clear();
    println!(
        "Verified and installed Axiom UI Host {} ({}) for {}.",
        record.artifact.as_ref().expect("set above").version,
        record.artifact.as_ref().expect("set above").variant,
        target.as_str()
    );
    println!("Host archive: {}", installed_asset.display());
    if record.delivery_adapter_ready {
        if target == UiTarget::Web {
            println!("Browser delivery and the embedded Axiom Runtime WASM are ready for this host release.");
        } else {
            println!("Native bundle delivery is ready for this host release.");
        }
    } else {
        println!(
            "This host release predates native bundle delivery. Publish {} or later.",
            host_protocol_minimum(target)
        );
    }
    Ok(())
}

fn select_release_asset<'a>(
    manifest: &'a UiHostReleaseManifest,
    target: UiTarget,
    requested_variant: Option<&str>,
    non_interactive: bool,
) -> Result<&'a UiHostReleaseAsset> {
    let candidates: Vec<&UiHostReleaseAsset> = manifest
        .assets
        .iter()
        .filter(|asset| asset.target == target.as_str())
        .collect();
    if candidates.is_empty() {
        bail!(
            "UI Host release {} has no {} asset",
            manifest.version,
            target.as_str()
        );
    }
    let asset = if let Some(variant) = requested_variant {
        candidates
            .into_iter()
            .find(|asset| asset.variant == variant)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "UI Host release {} has no {} `{}` variant",
                    manifest.version,
                    target.as_str(),
                    variant
                )
            })?
    } else if candidates.len() == 1 {
        candidates[0]
    } else if non_interactive {
        bail!(
            "UI Host release {} has multiple {} variants; pass `--variant <name>`",
            manifest.version,
            target.as_str()
        );
    } else {
        let options: Vec<String> = candidates
            .iter()
            .map(|asset| asset.variant.clone())
            .collect();
        let selection = dialoguer::Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Select {} UI Host variant", target.as_str()))
            .items(&options)
            .default(0)
            .interact()?;
        candidates[selection]
    };
    if !is_safe_asset_name(&asset.file)
        || !is_sha256(&asset.sha256)
        || !is_safe_variant(&asset.variant)
    {
        bail!(
            "UI Host release manifest contains an unsafe {} asset entry",
            target.as_str()
        );
    }
    Ok(asset)
}

fn verify_release_manifest_signature(manifest: &Path) -> Result<()> {
    let public_key = std::env::var("AXIOM_UI_HOST_SIGNING_PUBLIC_KEY_HEX")
        .unwrap_or_else(|_| UI_HOST_RELEASE_PUBLIC_KEY_HEX.to_string());
    let public_key: [u8; 32] = decode_hex(&public_key)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("UI Host public key must be 32 bytes"))?;
    let signature_path = PathBuf::from(format!("{}.sig", manifest.display()));
    let signature: [u8; 64] = decode_hex(
        std::fs::read_to_string(&signature_path)
            .with_context(|| {
                format!(
                    "UI Host manifest signature is missing: {}",
                    signature_path.display()
                )
            })?
            .trim(),
    )?
    .try_into()
    .map_err(|_| anyhow::anyhow!("UI Host manifest signature must be 64 bytes"))?;
    VerifyingKey::from_bytes(&public_key)?
        .verify(
            &std::fs::read(manifest)?,
            &Signature::from_bytes(&signature),
        )
        .map_err(|_| anyhow::anyhow!("UI Host manifest signature verification failed"))
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if value.len() % 2 != 0 {
        bail!("hex value must have an even length");
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| anyhow::anyhow!("invalid hexadecimal value"))
        })
        .collect()
}

fn is_safe_asset_name(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('/')
        && !value.contains('\\')
        && value != "."
        && value != ".."
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_safe_variant(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn read_ui_host(target: UiTarget) -> Result<Option<UiHostInstallation>> {
    let path = ui_host_record_path(target)?;
    if !path.is_file() {
        return Ok(None);
    }
    let host = serde_json::from_str::<UiHostInstallation>(&std::fs::read_to_string(&path)?)?;
    if host.format != "axiom-ui-host/v1"
        || host.target != target.as_str()
        || !host.host_root.is_dir()
    {
        return Ok(None);
    }
    Ok(Some(host))
}

fn ui_host_record_path(target: UiTarget) -> Result<PathBuf> {
    Ok(axiom_ui_cache_base()?
        .join("ui-hosts")
        .join(target.as_str())
        .join("host.json"))
}

const STARTER_SOURCE: &str = r#"module starter.app.ui

app Starter { route "/" => Home }

page Home {
  state greeting: String = "Welcome to Axiom"

  action reset() {
    greeting = "Welcome to Axiom"
  }

  view {
    SafeArea {
      View {
        Text(greeting)
        Text("Add a locked contract import when your backend is ready.")
        Button(label: "Reset", on_press: reset)
      }
    }
  }
}
"#;

const STARTER_TEST: &str = "// Phase 5 starter smoke scenario: `axiom ui test src/main.acore` validates this app's session behavior.\n";

const STARTER_README: &str = r#"# Axiom UI starter

This directory contains only authored Acore source, manifest, and tests. Axiom
keeps UI IR, contract facade, ReactLynx source, CSS, and source maps in memory.

```sh
axiom run src/main.acore --target ios
axiom ui inspect ir src/main.acore --target ios
axiom ui context src/main.acore --target ios
```

`axiom ui doctor` reports native-host readiness. The first `axiom run` offers
to install the latest verified Axiom UI Host automatically; this starter keeps
all compiler output in memory.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_runtime_restart_tracks_verified_configuration_lifecycle() {
        assert!(native_runtime_restart_required(
            true,
            false,
            Some("same"),
            "same"
        ));
        assert!(native_runtime_restart_required(
            false,
            true,
            Some("same"),
            "same"
        ));
        assert!(native_runtime_restart_required(false, false, None, "next"));
        assert!(native_runtime_restart_required(
            false,
            false,
            Some("previous"),
            "next"
        ));
        assert!(!native_runtime_restart_required(
            false,
            false,
            Some("same"),
            "same"
        ));
    }

    fn asset(target: &str, variant: &str, file: &str) -> UiHostReleaseAsset {
        UiHostReleaseAsset {
            target: target.to_string(),
            variant: variant.to_string(),
            file: file.to_string(),
            sha256: "a".repeat(64),
        }
    }

    fn manifest(assets: Vec<UiHostReleaseAsset>) -> UiHostReleaseManifest {
        UiHostReleaseManifest {
            format: "axiom-ui-host-release/v1".to_string(),
            version: "0.1.0".to_string(),
            assets,
        }
    }

    #[test]
    fn delivery_requires_the_phase2_host_release() {
        assert!(!version_at_least("0.1.0", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.1.1", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.2.0", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.3.0", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.5.3", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.5.4", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.5.5", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.5.6", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.6.0", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.6.1", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.6.3", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.6.4", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(version_at_least("0.6.5", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(version_at_least("1.0.0", IOS_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least(
            "not-a-release",
            IOS_HOST_PROTOCOL_MINIMUM
        ));
        assert!(!version_at_least("0.5.6", ANDROID_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.5.7", ANDROID_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.5.8", ANDROID_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.6.1", ANDROID_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.6.3", ANDROID_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.6.4", ANDROID_HOST_PROTOCOL_MINIMUM));
        assert!(version_at_least("0.6.5", ANDROID_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.6.0", WEB_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.6.3", WEB_HOST_PROTOCOL_MINIMUM));
        assert!(!version_at_least("0.6.4", WEB_HOST_PROTOCOL_MINIMUM));
        assert!(version_at_least("0.6.5", WEB_HOST_PROTOCOL_MINIMUM));
    }

    #[test]
    fn native_delivery_materializes_the_embedded_lynx_facade() {
        let project = std::env::temp_dir().join(format!(
            "axiom-ui-embedded-runtime-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        materialize_runtime_facade_package(&project).expect("materialize embedded facade");
        let index = std::fs::read_to_string(project.join("virtual/axiom-runtime/index.js"))
            .expect("read embedded facade");
        let adapter =
            std::fs::read_to_string(project.join("virtual/axiom-runtime/lynx-adapter.js"))
                .expect("read embedded Lynx adapter");
        assert!(index.contains("export function encodeJsonBase64"));
        assert!(!index.contains("btoa("));
        assert!(adapter.contains("nativeModule.poll"));
        std::fs::remove_dir_all(project).expect("remove embedded facade fixture");
    }

    #[test]
    fn native_bundle_compiler_enables_documented_css_inheritance() {
        assert!(LYNX_RSPEEDY_CONFIG.contains("enableCSSInheritance: true"));
    }

    #[test]
    fn android_forwards_only_loopback_contract_ports() {
        assert_eq!(loopback_port("http://127.0.0.1:8080"), Some(8080));
        assert_eq!(loopback_port("http://localhost/tasks"), Some(80));
        assert_eq!(loopback_port("https://localhost"), Some(443));
        assert_eq!(loopback_port("https://api.example.com"), None);
    }

    #[test]
    fn android_signature_conflicts_are_the_only_install_failures_that_replace_the_dev_host() {
        assert!(android_signature_conflict(
            "Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Existing package signatures do not match]"
        ));
        assert!(android_signature_conflict(
            "signatures do not match newer version"
        ));
        assert!(!android_signature_conflict(
            "Failure [INSTALL_FAILED_INSUFFICIENT_STORAGE]"
        ));
    }

    #[test]
    fn contract_free_native_delivery_does_not_require_a_lock_file() {
        let missing_lock = std::env::temp_dir().join(format!(
            "axiom-ui-missing-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        let compilation = compile_ui_source(
            "module test.local.ui\napp Test { route \"/\" => Home }\npage Home { view { Text(\"Hello\") } }\n",
            &UiCompileOptions {
                target: UiTarget::Ios,
                lock_path: missing_lock,
                asset_root: None,
            },
        );
        assert!(compilation.is_valid());

        let config =
            build_verified_runtime_config(&compilation, Path::new("missing-axiom.ui.lock.json"))
                .expect("contract-free delivery must use an empty runtime configuration");
        assert!(config.contains(r#""contracts":[]"#));

        let config_value = build_verified_runtime_config_value(
            &compilation,
            Path::new("missing-axiom.ui.lock.json"),
        )
        .expect("contract-free delivery must not read a missing lock");
        assert!(android_loopback_base_urls(&config_value).is_empty());
    }

    #[test]
    fn relative_optional_lock_watches_the_current_directory() {
        assert_eq!(
            nonempty_parent(Path::new("axiom.ui.lock.json")),
            Path::new(".")
        );
    }

    #[test]
    fn debug_environment_values_are_explicit() {
        for enabled in ["1", "true", "TRUE", "yes", "on"] {
            assert!(ui_debug_value(enabled));
        }
        for disabled in ["", "0", "false", "debug"] {
            assert!(!ui_debug_value(disabled));
        }
    }

    #[test]
    fn native_build_workspaces_are_isolated_by_target_and_process() {
        let root = Path::new("/opaque/ui-cache");
        let ios = ui_build_workspace(root, "same-graph", UiTarget::Ios, 10);
        let android = ui_build_workspace(root, "same-graph", UiTarget::Android, 11);
        assert_ne!(ios, android);
        assert_eq!(ios, root.join("builds/same-graph/ios-10"));
        assert_eq!(android, root.join("builds/same-graph/android-11"));
    }

    #[test]
    fn application_archives_are_byte_reproducible_and_ordered() {
        let directory = std::env::temp_dir().join(format!(
            "axiom-app-artifact-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let files = BTreeMap::from([
            ("manifest.json".to_string(), b"{}\n".to_vec()),
            ("payload/main.lynx.bundle".to_string(), b"bundle".to_vec()),
        ]);
        let first = directory.join("first.axiomapp");
        let second = directory.join("second.axiomapp");
        write_deterministic_zip(&first, &files).unwrap();
        write_deterministic_zip(&second, &files).unwrap();
        assert_eq!(
            std::fs::read(&first).unwrap(),
            std::fs::read(&second).unwrap()
        );
        let archive_file = std::fs::File::open(&first).unwrap();
        let mut archive = zip::ZipArchive::new(archive_file).unwrap();
        assert_eq!(archive.by_index(0).unwrap().name(), "manifest.json");
        assert_eq!(
            archive.by_index(1).unwrap().name(),
            "payload/main.lynx.bundle"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn web_application_assembly_contains_a_static_host_runtime_and_model() {
        let directory = std::env::temp_dir().join(format!(
            "axiom-web-artifact-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let host_root = directory.join("host");
        std::fs::create_dir_all(&host_root).unwrap();
        let host_files = BTreeMap::from([
            ("index.html".to_string(), b"<main id=app></main>".to_vec()),
            ("host.css".to_string(), b"body{}".to_vec()),
            ("host.js".to_string(), b"// static host".to_vec()),
            ("foreign-island.js".to_string(), b"export {}".to_vec()),
            (
                "axiom-extension-browser-kernel.mjs".to_string(),
                b"export {}".to_vec(),
            ),
            ("axiom-extension-worker.mjs".to_string(), b"void 0".to_vec()),
            ("wasm-policy.mjs".to_string(), b"export {}".to_vec()),
            ("axiom_runtime.js".to_string(), b"// wasm glue".to_vec()),
            ("axiom_runtime_bg.wasm".to_string(), b"\0asm".to_vec()),
        ]);
        let host_archive = host_root.join("axiom-ui-host-web-browser.zip");
        write_deterministic_zip(&host_archive, &host_files).unwrap();
        let host = UiHostInstallation {
            format: "axiom-ui-host/v1".to_string(),
            target: "web".to_string(),
            host_root,
            artifact: Some(UiHostArtifact {
                version: WEB_HOST_PROTOCOL_MINIMUM.to_string(),
                variant: "browser".to_string(),
                file: "axiom-ui-host-web-browser.zip".to_string(),
                sha256: sha256_file(&host_archive).unwrap(),
            }),
            delivery_adapter_ready: true,
        };
        let source = directory.join("main.acore");
        std::fs::write(&source, STARTER_SOURCE).unwrap();
        let lock = directory.join("missing-lock.json");
        let compilation = compile_ui_source(
            STARTER_SOURCE,
            &UiCompileOptions {
                target: UiTarget::Web,
                lock_path: lock.clone(),
                asset_root: Some(directory.clone()),
            },
        );
        let output = directory.join("app.axiomapp");
        assemble_application_artifact(
            &source,
            &lock,
            UiTarget::Web,
            &compilation,
            &host,
            &output,
            false,
        )
        .unwrap();

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
        for required in [
            "manifest.json",
            "provenance.json",
            "checksums.sha256",
            "runtime/config.json",
            "reports/capabilities.json",
            "reports/frontend-support.json",
            "index.html",
            "host.js",
            "foreign-island.js",
            "axiom-extension-browser-kernel.mjs",
            "axiom-extension-worker.mjs",
            "wasm-policy.mjs",
            "axiom_runtime_bg.wasm",
            "__axiom/app.json",
        ] {
            archive
                .by_name(required)
                .unwrap_or_else(|_| panic!("missing {required}"));
        }
        let mut model = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("__axiom/app.json").unwrap(),
            &mut model,
        )
        .unwrap();
        let model: serde_json::Value = serde_json::from_str(&model).unwrap();
        assert_eq!(model["format"], "axiom-web-application/v1");
        assert_eq!(model["hotReload"], false);
        let mut provenance = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("provenance.json").unwrap(),
            &mut provenance,
        )
        .unwrap();
        let provenance: serde_json::Value = serde_json::from_str(&provenance).unwrap();
        assert_eq!(
            provenance["frontendProfile"]["registrySha256"],
            PHASE2_CAPABILITY_REGISTRY_SHA256
        );
        assert_eq!(
            provenance["frontendProfile"]["componentPackage"]["version"],
            LYNX_UI_VERSION
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn application_artifact_names_are_portable() {
        assert_eq!(
            sanitize_artifact_name("Example.Tasks UI"),
            "example-tasks-ui"
        );
        assert_eq!(sanitize_artifact_name("..."), "app");
    }

    #[test]
    fn native_diagnostics_parse_as_versioned_records() {
        let envelope: NativeUiDiagnosticEnvelope = serde_json::from_str(
            r#"{"format":"axiom-ui-host-diagnostics/v1","diagnostics":[{"id":2,"sequence":7,"graphRevision":"graph","severity":"error","code":"LYNX_102","message":"layout failed"}]}"#,
        )
        .expect("parse host diagnostic");
        assert_eq!(envelope.format, "axiom-ui-host-diagnostics/v1");
        let diagnostic = &envelope.diagnostics[0];
        assert_eq!(diagnostic.code, "LYNX_102");
        assert!(diagnostic.suggestion.is_empty());
    }

    #[test]
    fn adb_remote_shell_script_is_passed_as_one_argument() {
        assert_eq!(
            quote_remote_shell_argument("mkdir -p files/axiom-ui-host"),
            "'mkdir -p files/axiom-ui-host'"
        );
        assert_eq!(
            quote_remote_shell_argument("printf '%s' value"),
            "'printf '\"'\"'%s'\"'\"' value'"
        );
    }

    #[test]
    fn duplicate_graph_revisions_are_not_delivered_twice() {
        let mut observed = "first".to_string();
        assert!(!observe_graph_revision(&mut observed, "first"));
        assert_eq!(observed, "first");
        assert!(observe_graph_revision(&mut observed, "second"));
        assert_eq!(observed, "second");
        assert!(!observe_graph_revision(&mut observed, "second"));
    }

    #[test]
    fn native_delivery_sequence_advances_across_cli_sessions() {
        let directory = std::env::temp_dir().join(format!(
            "axiom-ui-delivery-sequence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("create temporary support directory");
        std::fs::write(
            directory.join("axiom.app.ack.json"),
            r#"{"format":"axiom-ui-host-ack/v2","sequence":41,"graphRevision":"same","status":"applied_state_reset"}"#,
        )
        .expect("write acknowledgement");
        assert_eq!(next_ios_delivery_sequence(&directory, 1), 42);
        std::fs::remove_dir_all(directory).expect("remove temporary support directory");
    }

    #[test]
    fn only_signed_simulator_releases_are_delivery_capable() {
        let host = UiHostInstallation {
            format: "axiom-ui-host/v1".to_string(),
            target: "ios".to_string(),
            host_root: PathBuf::from("/opaque/cache"),
            artifact: Some(UiHostArtifact {
                version: IOS_HOST_PROTOCOL_MINIMUM.to_string(),
                variant: "simulator".to_string(),
                file: "axiom-ui-host-ios-simulator.app.zip".to_string(),
                sha256: "a".repeat(64),
            }),
            delivery_adapter_ready: true,
        };
        assert!(host_supports_delivery(&host));
        let mut old = host.clone();
        old.artifact.as_mut().unwrap().version = "0.1.0".to_string();
        assert!(!host_supports_delivery(&old));
    }

    #[test]
    fn only_signed_android_emulator_development_releases_are_delivery_capable() {
        let host = UiHostInstallation {
            format: "axiom-ui-host/v1".to_string(),
            target: "android".to_string(),
            host_root: PathBuf::from("/opaque/cache"),
            artifact: Some(UiHostArtifact {
                version: ANDROID_HOST_PROTOCOL_MINIMUM.to_string(),
                variant: "emulator".to_string(),
                file: "axiom-ui-host-android-emulator.apk".to_string(),
                sha256: "a".repeat(64),
            }),
            delivery_adapter_ready: true,
        };
        assert!(host_supports_delivery(&host));
        let mut device = host.clone();
        device.artifact.as_mut().unwrap().variant = "device".to_string();
        assert!(
            !host_supports_delivery(&device),
            "a device APK cannot silently receive adb run-as development transport"
        );
        let mut old = host;
        old.artifact.as_mut().unwrap().version = "0.3.0".to_string();
        assert!(!host_supports_delivery(&old));
    }

    #[test]
    fn delivery_plan_never_promises_state_preservation_for_a_reset_or_rejection() {
        let initial = HotReloadEvent {
            sequence: 1,
            graph_revision: "first".to_string(),
            outcome: HotReloadOutcome::InitialLoad,
            changed_semantic_ids: Vec::new(),
            diagnostics: Vec::new(),
        };
        assert_eq!(
            ios_delivery_plan(&initial),
            (
                IosDeliveryMode::StateResetTemplate,
                Some("initial native template load".to_string())
            )
        );
        let patch = HotReloadEvent {
            outcome: HotReloadOutcome::AppliedStatePreserved,
            ..initial.clone()
        };
        assert_eq!(
            ios_delivery_plan(&patch),
            (
                IosDeliveryMode::StateResetTemplate,
                Some("native state preservation is not certified for the pinned Lynx renderer; full template reload applied".to_string())
            )
        );
        let reset = HotReloadEvent {
            outcome: HotReloadOutcome::AppliedStateReset {
                reason: "view shape changed".to_string(),
            },
            ..initial
        };
        assert_eq!(
            ios_delivery_plan(&reset),
            (
                IosDeliveryMode::StateResetTemplate,
                Some("view shape changed".to_string())
            )
        );
    }

    #[test]
    fn native_acknowledgement_binds_sequence_and_reports_actual_state_result() {
        let reset = serde_json::json!({
            "format": "axiom-ui-host-ack/v2",
            "sequence": 7,
            "graphRevision": "graph-b",
            "status": "applied_state_reset",
            "reason": "declared local asset changed",
        });
        assert_eq!(
            parse_ios_acknowledgement(&reset, 7, "graph-b").unwrap(),
            Some(IosDeliveryAcknowledgement::AppliedStateReset {
                reason: "declared local asset changed".to_string(),
            })
        );
        assert_eq!(
            parse_ios_acknowledgement(&reset, 6, "graph-b").unwrap(),
            None,
            "a stale acknowledgement must not satisfy a newer delivery"
        );
        let rejected = serde_json::json!({
            "format": "axiom-ui-host-ack/v2",
            "sequence": 7,
            "graphRevision": "graph-b",
            "status": "rejected_last_good",
            "reason": "unsupported mode",
        });
        assert!(parse_ios_acknowledgement(&rejected, 7, "graph-b")
            .expect_err("last-good rejection must be actionable")
            .to_string()
            .contains("AXIOM_UI_HOST_REJECTED_LAST_GOOD"));
    }

    #[test]
    fn selects_the_only_target_asset_without_prompting() {
        let manifest = manifest(vec![asset(
            "ios",
            "simulator",
            "axiom-ui-host-ios-simulator.app.zip",
        )]);
        let selected =
            select_release_asset(&manifest, UiTarget::Ios, None, true).expect("select asset");
        assert_eq!(selected.variant, "simulator");
    }

    #[test]
    fn selects_signed_web_browser_release() {
        let manifest = manifest(vec![asset(
            "web",
            "browser",
            "axiom-ui-host-web-browser.zip",
        )]);
        let selected =
            select_release_asset(&manifest, UiTarget::Web, None, true).expect("select web asset");
        assert_eq!(selected.variant, "browser");

        let host = UiHostInstallation {
            format: "axiom-ui-host/v1".to_string(),
            target: "web".to_string(),
            host_root: PathBuf::from("/verified-host"),
            artifact: Some(UiHostArtifact {
                version: WEB_HOST_PROTOCOL_MINIMUM.to_string(),
                variant: "browser".to_string(),
                file: selected.file.clone(),
                sha256: selected.sha256.clone(),
            }),
            delivery_adapter_ready: true,
        };
        assert!(host_supports_delivery(&host));
    }

    #[test]
    fn packaged_web_model_disables_the_development_event_stream() {
        let directory = std::env::temp_dir().join(format!(
            "axiom-web-model-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let lock = directory.join("missing-lock.json");
        let compilation = compile_ui_source(
            STARTER_SOURCE,
            &UiCompileOptions {
                target: UiTarget::Web,
                lock_path: lock.clone(),
                asset_root: Some(directory.clone()),
            },
        );
        assert!(compilation.is_valid(), "{:?}", compilation.diagnostics);
        let development: serde_json::Value =
            serde_json::from_slice(&web_model(&compilation, &lock, true).unwrap()).unwrap();
        let packaged: serde_json::Value =
            serde_json::from_slice(&web_model(&compilation, &lock, false).unwrap()).unwrap();
        assert_eq!(development["format"], "axiom-web-development/v1");
        assert_eq!(development["hotReload"], true);
        assert!(development["stylesheet"]
            .as_str()
            .expect("development stylesheet")
            .contains("axiom-motion-root"));
        assert_eq!(packaged["format"], "axiom-web-application/v1");
        assert_eq!(packaged["hotReload"], false);
        assert_eq!(packaged["stylesheet"], development["stylesheet"]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_ambiguous_noninteractive_release() {
        let manifest = manifest(vec![
            asset("ios", "simulator", "axiom-ui-host-ios-simulator.app.zip"),
            asset("ios", "device", "axiom-ui-host-ios-device.app.zip"),
        ]);
        let error = select_release_asset(&manifest, UiTarget::Ios, None, true)
            .expect_err("must require an explicit variant");
        assert!(error.to_string().contains("multiple ios variants"));
    }

    #[test]
    fn rejects_unsafe_release_asset_name() {
        let manifest = manifest(vec![asset("ios", "simulator", "../host.zip")]);
        let error = select_release_asset(&manifest, UiTarget::Ios, None, true)
            .expect_err("unsafe name must fail");
        assert!(error.to_string().contains("unsafe ios asset entry"));
    }

    #[test]
    fn missing_source_lists_sibling_acore_files() {
        let directory = std::env::temp_dir().join(format!(
            "axiom-ui-source-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("temporary directory");
        std::fs::write(directory.join("main.acore"), "module app.ui").expect("write source");
        let error =
            read_ui_source(&directory.join("app.acore")).expect_err("missing source must fail");
        assert!(error.to_string().contains("main.acore"));
        std::fs::remove_dir_all(directory).expect("remove temporary directory");
    }

    #[test]
    fn declared_assets_are_embedded_only_when_their_digest_matches() {
        let directory = std::env::temp_dir().join(format!(
            "axiom-ui-asset-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source_root = directory.join("source");
        let project = directory.join("opaque-project");
        std::fs::create_dir_all(source_root.join("images")).unwrap();
        let bytes = b"fixture-image";
        std::fs::write(source_root.join("images/logo.png"), bytes).unwrap();
        let digest = sha256_bytes(bytes);
        let manifest = format!(r#"[{{"path":"images/logo.png","sha256":"{digest}"}}]"#);
        let file = axiom_ui::VirtualFile {
            path: "virtual/axiom-assets.json".to_string(),
            content: manifest,
            sha256: "unused".to_string(),
            read_only: true,
        };
        let page = axiom_ui::VirtualFile {
            path: "virtual/Home.tsx".to_string(),
            content: "export const page = <image src=\"./assets/images/logo.png\" placeholder=\"./assets/images/logo.png\" auto-size={true} />;\n"
                .to_string(),
            sha256: "unused".to_string(),
            read_only: true,
        };
        let build = VirtualReactLynxBuild {
            format: "test".to_string(),
            graph_revision: "test".to_string(),
            files: std::collections::BTreeMap::from([
                (file.path.clone(), file),
                (page.path.clone(), page.clone()),
            ]),
            source_map: vec![],
        };
        std::fs::create_dir_all(project.join("virtual")).unwrap();
        std::fs::write(project.join(&page.path), &page.content).unwrap();
        embed_declared_assets(&build, &source_root, &project).unwrap();
        let embedded = std::fs::read_to_string(project.join(&page.path)).unwrap();
        assert!(embedded.contains(&format!(
            "src=\"data:image/png;base64,{}\"",
            BASE64.encode(bytes)
        )));
        assert!(embedded.contains(&format!(
            "placeholder=\"data:image/png;base64,{}\"",
            BASE64.encode(bytes)
        )));
        assert!(!embedded.contains("./assets/images/logo.png"));
        std::fs::write(source_root.join("images/logo.png"), b"changed").unwrap();
        let error = embed_declared_assets(&build, &source_root, &project)
            .expect_err("changed input must be rejected");
        assert!(error.to_string().contains("AXIOM_UI_ASSET_CHANGED"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn declared_svg_assets_are_embedded_for_native_delivery() {
        let directory = std::env::temp_dir().join(format!(
            "axiom-ui-svg-asset-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source_root = directory.join("source");
        let project = directory.join("opaque-project");
        std::fs::create_dir_all(source_root.join("images")).unwrap();
        let bytes = br#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0h1v1z"/></svg>"#;
        std::fs::write(source_root.join("images/mark.svg"), bytes).unwrap();
        let manifest = format!(
            r#"[{{"path":"images/mark.svg","sha256":"{}"}}]"#,
            sha256_bytes(bytes)
        );
        let asset_manifest = axiom_ui::VirtualFile {
            path: "virtual/axiom-assets.json".to_string(),
            content: manifest,
            sha256: "unused".to_string(),
            read_only: true,
        };
        let page = axiom_ui::VirtualFile {
            path: "virtual/Home.tsx".to_string(),
            content: "export const page = <image src=\"./assets/images/mark.svg\" />;\n"
                .to_string(),
            sha256: "unused".to_string(),
            read_only: true,
        };
        let build = VirtualReactLynxBuild {
            format: "test".to_string(),
            graph_revision: "test".to_string(),
            files: std::collections::BTreeMap::from([
                (asset_manifest.path.clone(), asset_manifest),
                (page.path.clone(), page.clone()),
            ]),
            source_map: vec![],
        };
        std::fs::create_dir_all(project.join("virtual")).unwrap();
        std::fs::write(project.join(&page.path), &page.content).unwrap();

        embed_declared_assets(&build, &source_root, &project).unwrap();

        let embedded = std::fs::read_to_string(project.join(&page.path)).unwrap();
        assert!(embedded.contains(&format!(
            "content={{{}}}",
            serde_json::to_string(std::str::from_utf8(bytes).unwrap()).unwrap()
        )));
        assert!(!embedded.contains("src=\"./assets/images/mark.svg\""));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rspack_node_version_gate_matches_supported_runtimes() {
        assert!(!rspack_supports_node_version("v18.19.1"));
        assert!(!rspack_supports_node_version("20.18.9"));
        assert!(rspack_supports_node_version("v20.19.0"));
        assert!(!rspack_supports_node_version("v22.11.0"));
        assert!(rspack_supports_node_version("v22.12.0"));
        assert!(rspack_supports_node_version("v26.3.0"));
        assert!(!rspack_supports_node_version("not-a-version"));
    }
}
